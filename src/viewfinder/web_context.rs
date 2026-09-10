use super::{BASE, Client, Error};
use serde_json::Value;

// Deliberately not Debug: these are authenticated web tokens.
pub(super) struct WebContext {
    pub fields: Vec<(String, String)>,
}

fn module(html: &str, name: &str) -> Option<Value> {
    let marker = format!("[\"{name}\",");
    for (offset, _) in html.match_indices(&marker) {
        let mut values = serde_json::Deserializer::from_str(&html[offset..]).into_iter::<Value>();
        if let Some(Ok(value)) = values.next()
            && let Some(object) = value.as_array()?.iter().find(|v| v.is_object())
        {
            return Some(object.clone());
        }
    }
    None
}

impl WebContext {
    pub(super) fn actor_id(&self) -> Result<&str, Error> {
        self.fields
            .iter()
            .find(|(key, _)| key == "av")
            .map(|(_, value)| value.as_str())
            .filter(|id| {
                !id.is_empty()
                    && id.bytes().all(|b| b.is_ascii_digit())
                    && id.bytes().any(|b| b != b'0')
            })
            .ok_or(Error::SessionFile)
    }

    fn parse(html: &str) -> Result<Self, Error> {
        let dtsg = module(html, "DTSGInitialData").ok_or(Error::SessionFile)?;
        let user = module(html, "CurrentUserInitialData").unwrap_or(Value::Null);
        let lsd = module(html, "LSD").ok_or(Error::SessionFile)?;
        let token = dtsg["token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Error::SessionFile)?;
        let lsd = lsd["token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Error::SessionFile)?;
        let mut fields = vec![
            ("__a".into(), "1".into()),
            ("__d".into(), "www".into()),
            ("av".into(), super::normalize::string(&user["IG_USER_EIMU"])),
            ("__comet_req".into(), "7".into()),
            (
                "__user".into(),
                user["USER_ID"].as_str().unwrap_or("0").into(),
            ),
            ("fb_dtsg".into(), token.into()),
            (
                "jazoest".into(),
                format!("2{}", token.encode_utf16().map(u64::from).sum::<u64>()),
            ),
            ("lsd".into(), lsd.into()),
        ];
        if let Some(site) = module(html, "SiteData") {
            for (source, target) in [
                ("haste_session", "__hs"),
                ("client_revision", "__rev"),
                ("hsi", "__hsi"),
                ("spin_r", "__spin_r"),
                ("spin_b", "__spin_b"),
                ("spin_t", "__spin_t"),
            ] {
                if let Some(value) = site.get(source).filter(|v| v.is_string() || v.is_number()) {
                    fields.push((
                        target.into(),
                        value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string()),
                    ));
                }
            }
        }
        Ok(Self { fields })
    }
}

impl Client {
    pub(super) async fn web_context(&self) -> Result<&WebContext, Error> {
        self.web_context
            .get_or_try_init(|| async {
                let started = std::time::Instant::now();
                if super::diagnostics::enabled() {
                    tracing::info!(
                        method = "GET",
                        path = "/",
                        "Loading authenticated Viewfinder web context"
                    );
                }
                let mut response = self.http.get(format!("{BASE}/")).send().await?;
                if let Some(claim) = response
                    .headers()
                    .get("x-ig-set-www-claim")
                    .and_then(|value| value.to_str().ok())
                    .filter(|value| !value.is_empty())
                {
                    *self.www_claim.lock().unwrap() = claim.to_owned();
                }
                if super::diagnostics::enabled() || !response.status().is_success() {
                    tracing::info!(
                        status = response.status().as_u16(),
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "Viewfinder web-context HTTP response"
                    );
                }
                if response.status().is_redirection() || response.status().as_u16() == 401 {
                    return Err(Error::Authentication);
                }
                if !response.status().is_success() {
                    return Err(Error::Server);
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                        return Err(Error::Protocol);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let html = std::str::from_utf8(&bytes).map_err(|_| Error::Protocol)?;
                WebContext::parse(html).inspect_err(|_| {
                    tracing::warn!(
                        has_dtsg_module = module(html, "DTSGInitialData").is_some(),
                        has_lsd_module = module(html, "LSD").is_some(),
                        has_user_module = module(html, "CurrentUserInitialData").is_some(),
                        "Unable to parse authenticated web context (HTML and tokens withheld)"
                    );
                })
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_web_tokens_without_executing_page_scripts() {
        let context = WebContext::parse(r#"<script>{"define":[["DTSGInitialData",[],{"token":"abc"},1],["LSD",[],{"token":"xyz"},2],["SiteData",[],{"client_revision":123,"haste_session":"test"},3]]}</script>"#).unwrap();
        assert!(context.fields.contains(&("jazoest".into(), "2294".into())));
        assert!(context.fields.contains(&("__rev".into(), "123".into())));
        assert!(WebContext::parse("<html>Login</html>").is_err());
    }
    #[test]
    fn mutation_actor_uses_interop_id_not_instagram_or_facebook_user_id() {
        for id in [
            serde_json::json!("17841400000000001"),
            serde_json::json!(17841400000000001u64),
        ] {
            let html = format!(
                r#"<script>{{"define":[["DTSGInitialData",[],{{"token":"abc"}},1],["LSD",[],{{"token":"xyz"}},2],["CurrentUserInitialData",[],{{"IG_USER_EIMU":{id},"IG_USER_ID":"123","USER_ID":"456"}},3]]}}</script>"#
            );
            let context = WebContext::parse(&html).unwrap();
            assert_eq!(context.actor_id().unwrap(), "17841400000000001");
        }
    }

    #[test]
    fn missing_or_invalid_mutation_actor_requires_a_valid_session() {
        for value in ["", "0", "000", "12_34", "invalid"] {
            let context = WebContext {
                fields: vec![("av".into(), value.into())],
            };
            assert!(matches!(context.actor_id(), Err(Error::SessionFile)));
        }
        assert!(WebContext { fields: vec![] }.actor_id().is_err());
    }

    #[test]
    #[ignore = "read-only Viewfinder web-context check using local session"]
    fn live_web_context() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .try_init();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let session = crate::viewfinder::session::Session::restore()
                .await
                .unwrap_or_else(|_| {
                    crate::viewfinder::session::Session::parse(
                        &std::fs::read("session_cookies.json").unwrap(),
                    )
                    .unwrap()
                });
            let client = Client::new(session).unwrap();
            client
                .web_context()
                .await
                .expect("authenticated web context");
            eprintln!("Authenticated web context parsed (tokens withheld)");
            client
                .web_context()
                .await
                .unwrap()
                .actor_id()
                .expect("viewer mutation ID");
            eprintln!(
                "Viewer mutation ID resolved without an inbox request (account data withheld)"
            );
        });
    }
}
