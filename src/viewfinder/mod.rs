mod diagnostics;
pub mod normalize;
mod operations;
pub mod session;
mod web_context;

use crate::domain::*;
use reqwest::{
    cookie::{CookieStore, Jar},
    header,
};
use serde_json::Value;
use session::Session;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tracing::Instrument;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Cannot connect to Viewfinder. Check your connection and try again.")]
    Network,
    #[error("Viewfinder took too long to respond. Try again.")]
    Timeout,
    #[error("Your session has expired. Sign in to Viewfinder again.")]
    Authentication,
    #[error("Viewfinder needs you to review your account in your browser.")]
    Challenge,
    #[error("This content is private or unavailable to your account.")]
    Permission,
    #[error("Viewfinder is limiting requests. Please wait before trying again.")]
    RateLimit,
    #[error("Viewfinder is temporarily unavailable. Try again later.")]
    Server,
    #[error("Viewfinder rejected this request. Try again later or refresh your sign-in.")]
    Rejected,
    #[error("Viewfinder returned an unsupported response. This feature needs an update.")]
    Protocol,
    #[error("This feature is not available with the documented Viewfinder protocol yet.")]
    Unsupported,
    #[error("The Viewfinder session is incomplete or invalid. Please sign in again.")]
    SessionFile,
    #[error("The session could not be accessed in the system keyring.")]
    Keyring,
}
impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            Self::Timeout
        } else {
            Self::Network
        }
    }
}

const BASE: &str = "https://www.instagram.com";
pub struct Client {
    http: reqwest::Client,
    jar: Arc<Jar>,
    pub account_id: String,
    web_context: tokio::sync::OnceCell<web_context::WebContext>,
    web_session_id: String,
    comet_request: AtomicU64,
    www_claim: Mutex<String>,
    device_id: Option<String>,
    blocked_until: Mutex<Option<Instant>>,
    auth_blocked: Mutex<bool>,
}
impl Client {
    pub fn new(session: Session) -> Result<Arc<Self>, Error> {
        let base = reqwest::Url::parse(BASE).expect("constant URL");
        let jar = Arc::new(Jar::default());
        for key in [
            "sessionid",
            "csrftoken",
            "ds_user_id",
            "mid",
            "ig_did",
            "datr",
            "rur",
        ] {
            if let Some(value) = session.values.get(key).filter(|s| !s.is_empty()) {
                if value.contains(['\r', '\n', ';']) {
                    return Err(Error::SessionFile);
                }
                jar.add_cookie_str(&format!("{key}={value}; Path=/; Secure"), &base);
            }
        }
        let mut headers = header::HeaderMap::new();
        for (key, value) in [
            ("x-ig-app-id", "936619743392459"),
            ("x-asbd-id", "359341"),
            ("origin", BASE),
            ("referer", "https://www.instagram.com/"),
            ("x-requested-with", "XMLHttpRequest"),
            ("accept", "*/*"),
        ] {
            headers.insert(
                header::HeaderName::from_static(key),
                header::HeaderValue::from_static(value),
            );
        }
        let ua = session.values.get("user_agent").map(String::as_str).unwrap_or("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36");
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(ua)
            .cookie_provider(jar.clone())
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Arc::new(Self {
            http,
            jar,
            account_id: session.values["ds_user_id"].clone(),
            web_context: tokio::sync::OnceCell::new(),
            web_session_id: gtk::glib::uuid_string_random().to_string(),
            comet_request: AtomicU64::new(1),
            www_claim: Mutex::new("0".into()),
            device_id: session.values.get("ig_did").cloned(),
            blocked_until: Mutex::new(None),
            auth_blocked: Mutex::new(false),
        }))
    }
    fn csrf(&self) -> Result<String, Error> {
        let base = reqwest::Url::parse(BASE).expect("constant URL");
        let cookies = self.jar.cookies(&base).ok_or(Error::Authentication)?;
        cookies
            .to_str()
            .map_err(|_| Error::Authentication)?
            .split(';')
            .find_map(|s| s.trim().strip_prefix("csrftoken=").map(str::to_owned))
            .ok_or(Error::Authentication)
    }
    async fn execute(&self, request: reqwest::RequestBuilder) -> Result<Value, Error> {
        static NEXT_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let request = request.header("x-csrftoken", self.csrf()?).build()?;
        let request_id = NEXT_REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = diagnostics::path(request.url().path());
        let span =
            tracing::info_span!("viewfinder_http", request_id, method = %request.method(), path);
        async {
            let started = Instant::now();
            if diagnostics::enabled() {
                tracing::info!("Sending Viewfinder request (headers and body withheld)");
            }
            let result = self.execute_request(request).await;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            match &result {
                Err(error) => tracing::warn!(elapsed_ms, %error, "Viewfinder request failed"),
                Ok(value) if diagnostics::enabled() => tracing::info!(elapsed_ms, response = %diagnostics::shape(value), "Viewfinder request completed"),
                _ => {}
            }
            result
        }.instrument(span).await
    }
    async fn execute_request(&self, request: reqwest::Request) -> Result<Value, Error> {
        if *self.auth_blocked.lock().unwrap() {
            return Err(Error::Authentication);
        }
        if self
            .blocked_until
            .lock()
            .unwrap()
            .is_some_and(|t| t > Instant::now())
        {
            return Err(Error::RateLimit);
        }
        let mut response = self.http.execute(request).await.map_err(|error| {
            tracing::warn!(
                timeout = error.is_timeout(),
                connect = error.is_connect(),
                request = error.is_request(),
                "Viewfinder HTTP transport failed"
            );
            Error::from(error)
        })?;
        let status = response.status();
        if let Some(claim) = response
            .headers()
            .get("x-ig-set-www-claim")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
        {
            *self.www_claim.lock().unwrap() = claim.to_owned();
            if diagnostics::enabled() {
                tracing::info!("Viewfinder rotated the WWW claim (value withheld)");
            }
        }
        if diagnostics::enabled() || !status.is_success() {
            tracing::info!(status = status.as_u16(), content_type = ?response.headers().get("content-type"), "Viewfinder HTTP response");
        }
        if status.as_u16() == 429 {
            let seconds = response
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60)
                .max(60);
            *self.blocked_until.lock().unwrap() =
                Some(Instant::now() + Duration::from_secs(seconds.min(86400)));
            return Err(Error::RateLimit);
        }
        if status.is_redirection() || status.as_u16() == 401 {
            *self.auth_blocked.lock().unwrap() = true;
            return Err(Error::Authentication);
        }
        if status.is_server_error() {
            return Err(Error::Server);
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !content_type.contains("json") && !content_type.contains("javascript") {
            tracing::warn!(
                status = status.as_u16(),
                content_type,
                "Viewfinder returned non-JSON content"
            );
            return Err(Error::Protocol);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                return Err(Error::Protocol);
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = parse_response(&bytes).inspect_err(|_| {
            tracing::warn!(
                bytes = bytes.len(),
                "Viewfinder response was not valid JSON (body withheld)"
            );
        })?;
        if value["message"]
            .as_str()
            .is_some_and(|m| m.contains("challenge") || m.contains("checkpoint"))
            || value.get("challenge").is_some()
        {
            *self.auth_blocked.lock().unwrap() = true;
            return Err(Error::Challenge);
        }
        if value["message"] == "login_required" {
            *self.auth_blocked.lock().unwrap() = true;
            return Err(Error::Authentication);
        }
        if status.as_u16() == 403 {
            return Err(Error::Permission);
        }
        if !status.is_success()
            || diagnostics::gateway_error(&value)
            || value["status"] == "fail"
            || value["errors"].as_array().is_some_and(|v| !v.is_empty())
        {
            tracing::warn!(
                status = status.as_u16(),
                gateway_error_code = diagnostics::gateway_error_code(&value),
                response = %diagnostics::shape(&value),
                "Viewfinder rejected the request"
            );
            // 1357054 is Viewfinder's generic request-processing failure. It is
            // returned with HTTP 200, so treating it as an unknown response made
            // ordinary server rejection look like an unsupported app feature.
            return Err(
                if diagnostics::gateway_error_code(&value) == Some(1357054) {
                    Error::Server
                } else {
                    Error::Rejected
                },
            );
        }
        Ok(value)
    }
    async fn get(&self, path: &str, params: &[(&str, String)]) -> Result<Value, Error> {
        self.execute(self.http.get(format!("{BASE}/api/v1/{path}")).query(params))
            .await
    }
    async fn graphql(&self, id: &str, variables: Value, mutation: bool) -> Result<Value, Error> {
        let path = if mutation {
            "/api/graphql"
        } else {
            "/graphql/query"
        };
        let mut fields = vec![
            ("doc_id".to_owned(), id.to_owned()),
            ("variables".to_owned(), variables.to_string()),
        ];
        let mut request = self.http.post(format!("{BASE}{path}"));
        if mutation {
            let needs_claim = self.www_claim.lock().unwrap().as_str() == "0";
            if needs_claim {
                // Read-only REST calls issue the session-bound HMAC claim used by
                // Viewfinder's write gateway. GraphQL feed responses often do not.
                self.get(&format!("friendships/show/{}/", self.account_id), &[])
                    .await?;
            }
            if diagnostics::enabled() {
                tracing::info!(
                    has_server_www_claim = self.www_claim.lock().unwrap().as_str() != "0",
                    "Prepared Viewfinder mutation session"
                );
            }
            let context = self.web_context().await?;
            fields.extend(context.fields.iter().cloned());
            fields.push((
                "__req".into(),
                base36(self.comet_request.fetch_add(1, Ordering::Relaxed)),
            ));
            fields.push(("dpr".into(), "1".into()));
            fields.push(("__ccg".into(), "GOOD".into()));
            fields.push(("__csr".into(), String::new()));
            fields.push(("fb_api_caller_class".into(), "RelayModern".into()));
            fields.push(("server_timestamps".into(), "true".into()));
            let name = match id {
                "28794932076791671" => Some("PolarisDirectInboxQuery"),
                "26911679871773184" => Some("IGDirectTextSendMutation"),
                "27399783383056109" => Some("useIGDMarkThreadAsReadMutation"),
                "35211594988486314" => Some("useIGDMarkThreadAsReadValidationMutation"),
                "27182485238052618" => Some("usePolarisLikeMediaXIGLikeMutation"),
                "27345296031770102" => Some("usePolarisLikeMediaXIGUnlikeMutation"),
                "27358573637160660" => Some("PolarisAPILikePostMutation"),
                "37071567952434061" => Some("PolarisAPIUnlikePostMutation"),
                "27184292767848867" => Some("PolarisCommentActionsLikeMutation"),
                "27318337671093716" => Some("PolarisCommentActionsUnlikeMutation"),
                "26938887309082050" => Some("usePolarisStoriesV4LikeMutationLikeMutation"),
                "26510485515280697" => Some("usePolarisStoriesV4LikeMutationUnlikeMutation"),
                _ => None,
            };
            if let Some(name) = name {
                fields.push(("fb_api_req_friendly_name".into(), name.into()));
                request = request.header("x-fb-friendly-name", name);
            }
            if let Some((_, lsd)) = context.fields.iter().find(|(key, _)| key == "lsd") {
                request = request.header("x-fb-lsd", lsd);
            }
            request = request.header("x-web-session-id", &self.web_session_id);
            request = request.header("x-ig-www-claim", self.www_claim.lock().unwrap().clone());
        }
        self.execute(request.form(&fields))
            .instrument(tracing::info_span!(
                "viewfinder_graphql",
                doc_id = id,
                mutation
            ))
            .await
    }
    pub async fn profile(&self, id: &str) -> Result<User, Error> {
        validate_id(id)?;
        let v = self.graphql("28036671149327607", serde_json::json!({
            "enable_integrity_filters": true,
            "id": id,
            "__relay_internal__pv__PolarisCannesGuardianExperienceEnabledrelayprovider": true,
            "__relay_internal__pv__PolarisCASB976ProfileEnabledrelayprovider": false,
            "__relay_internal__pv__PolarisWebSchoolsEnabledrelayprovider": false,
            "__relay_internal__pv__PolarisRepostsConsumptionEnabledrelayprovider": false,
            "__relay_internal__pv__PolarisShortDramaEnabledrelayprovider": false
        }), false).await?;
        normalize::user(&v["data"]["user"])
    }
    pub async fn relationship(&self, id: &str) -> Result<Relationship, Error> {
        validate_id(id)?;
        let v = self.get(&format!("friendships/show/{id}/"), &[]).await?;
        Ok(normalize::relationship(&v))
    }
}
fn parse_response(bytes: &[u8]) -> Result<Value, Error> {
    let bytes = bytes.strip_prefix(b"for (;;);").unwrap_or(bytes);
    serde_json::from_slice(bytes).map_err(|_| Error::Protocol)
}
fn base36(mut value: u64) -> String {
    let mut output = [0u8; 13];
    let mut position = output.len();
    loop {
        position -= 1;
        let digit = (value % 36) as u8;
        output[position] = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
        value /= 36;
        if value == 0 {
            return String::from_utf8(output[position..].to_vec()).expect("base36 is ASCII");
        }
    }
}
fn validate_id(id: &str) -> Result<(), Error> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit() || b == b'_') {
        Err(Error::Protocol)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    fn parses_guarded_json_but_never_executes_javascript() {
        assert_eq!(
            parse_response(br#"for (;;);{"data":{"ok":true}}"#).unwrap()["data"]["ok"],
            true
        );
        assert!(parse_response(b"alert('not JSON')").is_err());
        assert!(parse_response(b"<html>Login</html>").is_err());
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
    }

    #[test]
    #[ignore = "contacts Viewfinder using the local session; run explicitly"]
    fn read_only_pages() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .try_init();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let session = Session::restore().await.unwrap_or_else(|_| {
                Session::parse(&std::fs::read("session_cookies.json").unwrap()).unwrap()
            });
            let client = Client::new(session).unwrap();
            let user = client
                .profile(&client.account_id)
                .await
                .expect("profile metadata");
            let page = client
                .load(Route::Profile(user), None)
                .await
                .expect("profile posts");
            eprintln!("profile: {} items", page.items.len());
            for (name, route) in [
                ("home", Route::Home),
                ("reels", Route::Reels),
                ("stories", Route::Stories),
                ("notifications", Route::Notifications),
                ("inbox", Route::Inbox),
            ] {
                match client.load(route, None).await {
                    Ok(page) => eprintln!("{name}: {} items", page.items.len()),
                    Err(error) => panic!("{name}: {error}"),
                }
            }
        });
    }

    #[test]
    #[ignore = "contacts Viewfinder and submits an idempotent like-state mutation"]
    fn mutation_envelope() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .try_init();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let session = Session::restore().await.unwrap_or_else(|_| {
                Session::parse(&std::fs::read("session_cookies.json").unwrap()).unwrap()
            });
            let client = Client::new(session).unwrap();
            let page = client.load(Route::Home, None).await.expect("home feed");
            let post = page
                .items
                .into_iter()
                .find_map(|item| match item {
                    Item::Post(post) => Some(post),
                    _ => None,
                })
                .expect("a post in the home feed");
            eprintln!(
                "WWW claim after feed request: {}",
                if client.www_claim.lock().unwrap().as_str() == "0" {
                    "default"
                } else {
                    "server-issued"
                }
            );
            let id = post.id.split('_').next().unwrap_or(&post.id).to_owned();
            assert!(!post.liked, "probe requires an already-unliked post");
            let actor_id = client
                .web_context()
                .await
                .expect("authenticated web context")
                .actor_id()
                .expect("viewer mutation ID");
            let response = client
                .graphql(
                    "27345296031770102",
                    serde_json::json!({"input": {
                        "actor_id": actor_id,
                        "client_mutation_id": gtk::glib::uuid_string_random().to_string(),
                        "media_id": id,
                        "tracking_token": null
                    }}),
                    true,
                )
                .await
                .expect("idempotent mutation");
            assert_eq!(
                response["data"]["xig_media_unlike"]["media"]["has_liked"],
                false
            );
        });
    }
}
