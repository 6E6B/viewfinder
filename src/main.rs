mod app;
mod domain;
mod viewfinder;
mod media;
mod ui;

use adw::prelude::*;
use gtk::{gio, glib};

fn main() -> glib::ExitCode {
    let smoke_test = std::env::args().any(|arg| arg == "--smoke-test");
    glib::set_application_name("Viewfinder");
    tracing_subscriber::fmt().with_target(false).init();
    if std::env::var("VIEWFINDER_HTTP_DEBUG").as_deref() == Ok("1") {
        tracing::info!(
            "Viewfinder HTTP diagnostics enabled; request bodies, credentials, and response text are withheld"
        );
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(3)
        .enable_all()
        .build()
        .expect("background runtime");
    let _guard = runtime.enter();
    if let Err(error) = gstreamer::init() {
        tracing::warn!(%error, "Video playback unavailable");
    }
    let app = adw::Application::builder()
        .application_id("io.github._6E6B.viewfinder")
        .flags(if smoke_test {
            gio::ApplicationFlags::NON_UNIQUE
        } else {
            gio::ApplicationFlags::FLAGS_NONE
        })
        .build();
    app.add_main_option(
        "smoke-test",
        glib::Char::from(b'\0'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open an isolated logged-out window and close it without network activity",
        None,
    );
    app.connect_activate(move |app| {
        ui::activate(app);
        if smoke_test && let Some(window) = app.active_window() {
            window.set_sensitive(false);
            glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                window.close();
                tracing::info!("Logged-out startup smoke test completed");
            });
        }
    });
    let quit = gio::SimpleAction::new("quit", None);
    quit.connect_activate(glib::clone!(
        #[weak]
        app,
        move |_, _| app.quit()
    ));
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("win.search", &["<Control>f"]);
    app.set_accels_for_action("win.refresh", &["<Control>r"]);
    app.set_accels_for_action("win.back", &["<Alt>Left"]);
    app.run()
}
