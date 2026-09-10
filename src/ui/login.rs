use super::Ui;
use crate::{app, viewfinder::session::Session};
use adw::prelude::*;
use gtk::glib;
use std::{collections::BTreeMap, rc::Rc, time::Duration};
use webkit6::prelude::*;

const ORIGIN: &str = "https://www.instagram.com/";

// Only finish on a Viewfinder content page, after login and account challenges.
fn can_capture(uri: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(uri) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("www.instagram.com")
        && url.port_or_known_default() == Some(443)
        && ![
            "/accounts",
            "/challenge",
            "/checkpoint",
            "/two_factor",
            "/oauth",
        ]
        .iter()
        .any(|prefix| url.path().starts_with(prefix))
}

pub(super) fn open(ui: &Rc<Ui>) -> adw::Dialog {
    let dialog = adw::Dialog::builder()
        .title("Sign In to Viewfinder")
        .content_width(520)
        .content_height(720)
        .build();
    // A fresh, memory-only browser session never reads another browser's profile.
    let network = webkit6::NetworkSession::new_ephemeral();
    let view = webkit6::WebView::builder()
        .network_session(&network)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let status = gtk::Label::new(Some(
        "Sign in below, including any verification Viewfinder requests.",
    ));
    status.set_wrap(true);
    status.set_margin_top(8);
    status.set_margin_bottom(8);
    let retry = gtk::Button::with_label("Reload Login Page");
    retry.set_visible(false);
    retry.connect_clicked(glib::clone!(
        #[weak]
        view,
        #[weak]
        retry,
        move |_| {
            retry.set_visible(false);
            view.load_uri("https://www.instagram.com/accounts/login/");
        }
    ));
    toolbar.add_top_bar(&status);
    toolbar.add_top_bar(&retry);
    toolbar.set_content(Some(&view));
    dialog.set_child(Some(&toolbar));
    view.connect_load_failed(glib::clone!(
        #[weak]
        status,
        #[weak]
        retry,
        #[upgrade_or]
        false,
        move |_, _, _, _| {
            status.set_text("Could not load Viewfinder. Check your connection and reload.");
            retry.set_visible(true);
            false
        }
    ));
    view.connect_web_process_terminated(glib::clone!(
        #[weak]
        status,
        #[weak]
        retry,
        move |_, _| {
            status.set_text("The login page stopped. Reload to try again.");
            retry.set_visible(true);
        }
    ));
    view.load_uri("https://www.instagram.com/accounts/login/");
    let task = glib::spawn_future_local(glib::clone!(
        #[weak]
        ui,
        #[weak]
        dialog,
        #[strong]
        view,
        async move {
            let manager = network.cookie_manager().expect("WebKit cookie manager");
            loop {
                glib::timeout_future(Duration::from_secs(1)).await;
                if view.is_loading() || !view.uri().is_some_and(|uri| can_capture(&uri)) {
                    continue;
                }
                // CookieManager includes HttpOnly cookies, unlike document.cookie.
                let Ok(cookies) = manager.cookies_future(ORIGIN).await else {
                    status.set_text("Could not read the login session. Reload to try again.");
                    retry.set_visible(true);
                    continue;
                };
                if view.is_loading() || !view.uri().is_some_and(|uri| can_capture(&uri)) {
                    continue;
                }
                let mut values = BTreeMap::new();
                for mut cookie in cookies {
                    if let (Some(name), Some(value)) = (cookie.name(), cookie.value()) {
                        values.insert(name.to_string(), value.to_string());
                    }
                }
                if let Some(agent) =
                    WebViewExt::settings(&view).and_then(|settings| settings.user_agent())
                {
                    values.insert("user_agent".into(), agent.to_string());
                }
                let Ok(bytes) = serde_json::to_vec(&values) else {
                    continue;
                };
                let Ok(session) = Session::parse(&bytes) else {
                    continue;
                };
                ui.install(session.clone());
                if ui.client.borrow().is_none() {
                    continue;
                }
                *ui.task.borrow_mut() = Some(app::background(
                    async move { session.save().await },
                    glib::clone!(
                        #[weak]
                        ui,
                        move |result| {
                            if result.is_err() {
                                ui.notify("Signed in for this window. The session could not be saved in the keyring.");
                            }
                        }
                    ),
                ));
                dialog.close();
                break;
            }
        }
    ));
    dialog.connect_closed(move |_| {
        task.abort();
        view.stop_loading();
        toolbar.set_content(None::<&gtk::Widget>);
    });
    dialog.present(ui.window.upgrade().as_ref());
    dialog
}

#[cfg(test)]
mod tests {
    use super::can_capture;

    #[test]
    fn capture_waits_for_instagram_content() {
        for uri in [
            "https://www.instagram.com/",
            "https://www.instagram.com/direct/inbox/",
        ] {
            assert!(can_capture(uri));
        }
        for uri in [
            "https://www.instagram.com/accounts/login/",
            "https://www.instagram.com/accounts/onetap/",
            "https://www.instagram.com/challenge/123/",
            "https://www.instagram.com/checkpoint/",
            "https://www.instagram.com.evil.example/",
            "https://www.instagram.com@evil.example/",
            "http://www.instagram.com/",
            "about:blank",
        ] {
            assert!(!can_capture(uri));
        }
    }
}
