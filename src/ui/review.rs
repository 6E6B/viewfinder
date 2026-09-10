//! Opt-in rendered review of real widgets with synthetic data and no network.
use super::*;
use gtk::gdk;
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};
thread_local! { static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) }; }
pub(super) fn remember(ui: &Rc<Ui>) {
    UI.with(|slot| slot.replace(Some(ui.clone())));
}
fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(350);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
pub(super) fn capture(window: &adw::ApplicationWindow, name: &str) {
    settle();
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
    let node = snapshot.to_node().expect("rendered widget");
    let texture = window.renderer().unwrap().render_texture(&node, None);
    let directory =
        std::env::var("VIEWFINDER_REVIEW_DIR").unwrap_or_else(|_| "/tmp/viewfinder-ui-review".into());
    std::fs::create_dir_all(&directory).unwrap();
    texture
        .save_to_png(format!("{directory}/{name}.png"))
        .unwrap();
}
fn user() -> User {
    User { id: "42".into(), username: "alex_photography".into(), name: "Alex Morgan".into(), avatar: Some("fixture://image".into()), biography: "Photos from walks around the city. A long biography to check wrapping, content width, and the space left for posts on a small screen.".into(), followers: 1234567, following_count: 3456, posts: 245, ..User::default() }
}
fn post() -> Post {
    Post { id: "1".into(), code: "fixture".into(), author: user(), caption: "A quiet afternoon by the water. Testing a longer caption that should stay readable without dominating the photograph.".into(), media: vec![Media { thumbnail: Some("fixture://image".into()), image: Some("fixture://image".into()), video: None, width: 1080, height: 800 }], liked: false, likes: 26200, comments: 1234567, timestamp: 1788800000 }
}
#[test]
#[ignore = "opens a real GTK window; run explicitly on a desktop"]
fn stories_review() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    adw::init().unwrap();
    let app = adw::Application::builder()
        .application_id("io.github._6E6B.viewfinder.StoriesReview")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    super::activate(&app);
    let ui = UI.with(|slot| slot.borrow().as_ref().unwrap().clone());
    ui.task.borrow_mut().take();
    ui.images.review_seed();
    ui.login_stack.set_visible_child_name("app");
    ui.collections.borrow()[0].review_finish(Ok(Page::complete(vec![Item::Post(post())])));
    super::stories::review(&ui);
    ui.window.upgrade().unwrap().close();
}

#[test]
#[ignore = "opens a real GTK window; run explicitly on a desktop"]
fn rendered_review() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    adw::init().unwrap();
    let app = adw::Application::builder()
        .application_id("io.github._6E6B.viewfinder.Review")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    super::activate(&app);
    let ui = UI.with(|slot| slot.borrow().as_ref().unwrap().clone());
    let window = app
        .active_window()
        .unwrap()
        .downcast::<adw::ApplicationWindow>()
        .unwrap();
    ui.images.review_seed();
    capture(&window, "login-wide");
    ui.login_stack.set_visible_child_name("app");
    let home = ui.collections.borrow()[0].clone();
    let mut portrait = post();
    portrait.id = "portrait".into();
    portrait.media[0].width = 1080;
    portrait.media[0].height = 1920;
    home.review_finish(Ok(Page::complete(vec![
        Item::Post(post()),
        Item::Post(portrait),
    ])));
    let home_widgets = descendants(home.root.upcast_ref());
    assert!(home_widgets.iter().any(|widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.label() == "26.2K")
    }));
    assert!(home_widgets.iter().any(|widget| {
        widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.label() == "1.2M")
    }));
    capture(&window, "home-wide");
    window.set_default_size(390, 700);
    capture(&window, "home-narrow");
    let mut inline = post();
    inline.media[0].video = Some("fixture://video".into());
    home.refresh();
    home.review_finish(Ok(Page::complete(vec![Item::Post(inline)])));
    capture(&window, "home-inline-video");
    let widgets = descendants(home.root.upcast_ref());
    assert!(!widgets.iter().any(|w| {
        w.downcast_ref::<gtk::Button>()
            .is_some_and(|b| b.label().as_deref() == Some("Load More"))
    }));
    let play = widgets
        .iter()
        .find_map(|w| {
            w.downcast_ref::<gtk::Button>()
                .filter(|b| b.tooltip_text().as_deref() == Some("Play video"))
        })
        .unwrap();
    play.emit_clicked();
    settle();
    assert!(
        window.visible_dialog().is_none(),
        "feed video plays in place"
    );
    assert!(descendants(home.root.upcast_ref()).iter().any(|w| {
        w.downcast_ref::<gtk::Label>()
            .is_some_and(|l| l.label() == "This video is unavailable.")
    }));
    ui.destinations.set_visible_child_name("profile");
    let profile = ui.collections.borrow()[4].clone();
    profile.profile_header(user());
    profile.review_finish(Ok(Page::complete(
        (0..24)
            .map(|i| {
                let mut p = post();
                p.id = format!("profile-{i}");
                Item::Post(p)
            })
            .collect(),
    )));
    capture(&window, "profile-narrow");
    window.set_default_size(1100, 820);
    capture(&window, "profile-wide");
    window.set_default_size(360, 700);
    capture(&window, "profile-minimum");
    let profile_widgets = descendants(profile.root.upcast_ref());
    let about = profile_widgets
        .iter()
        .find_map(|w| w.downcast_ref::<adw::ExpanderRow>())
        .unwrap();
    about.set_expanded(true);
    capture(&window, "profile-about-expanded");
    assert!(window.width() <= 360);
    about.set_expanded(false);
    ui.destinations.set_visible_child_name("reels");
    ui.collections.borrow()[2].review_finish(Ok(Page::complete(
        (0..12)
            .map(|i| {
                let mut p = post();
                p.id = format!("reel-{i}");
                p.media[0].video = Some("fixture://video".into());
                p.media[0].width = 1080;
                p.media[0].height = 1920;
                Item::Post(p)
            })
            .collect(),
    )));
    capture(&window, "reels-narrow");
    window.set_default_size(1100, 820);
    capture(&window, "reels-wide");
    let reels = ui.collections.borrow()[2].clone();
    let widgets = descendants(reels.root.upcast_ref());
    let details = widgets
        .iter()
        .find(|w| w.has_css_class("reel-details"))
        .unwrap();
    assert!(
        details
            .parent()
            .is_some_and(|p| p.has_css_class("reel-surface")),
        "caption stays attached to the video surface"
    );
    let actions = widgets
        .iter()
        .find(|w| w.has_css_class("reel-actions"))
        .unwrap();
    assert!(
        !actions
            .parent()
            .is_some_and(|p| p.has_css_class("reel-surface")),
        "engagement controls stay outside the video"
    );
    let playback_controls = widgets
        .iter()
        .find(|widget| widget.has_css_class("toolbar"))
        .unwrap();
    assert!(
        playback_controls
            .parent()
            .is_some_and(|parent| parent.is::<gtk::Overlay>()),
        "playback controls do not participate in engagement-button layout"
    );
    let caption_expander = widgets
        .iter()
        .find_map(|widget| {
            widget
                .downcast_ref::<gtk::MenuButton>()
                .filter(|button| button.has_css_class("caption-expander"))
        })
        .unwrap();
    let caption = caption_expander
        .prev_sibling()
        .and_downcast::<gtk::Label>()
        .unwrap();
    assert_eq!(caption.lines(), 2);
    caption_expander.popup();
    settle();
    assert!(caption_expander.popover().unwrap().is_visible());
    assert_eq!(caption.lines(), 2);
    caption_expander.popdown();
    let next = widgets
        .iter()
        .find_map(|w| {
            w.downcast_ref::<gtk::Button>()
                .filter(|b| b.tooltip_text().as_deref() == Some("Next reel"))
        })
        .unwrap();
    next.emit_clicked();
    settle();
    assert!(descendants(reels.root.upcast_ref()).iter().any(|w| {
        w.downcast_ref::<gtk::Label>()
            .is_some_and(|l| l.label().starts_with("Reel 2"))
    }));
    assert!(!descendants(reels.root.upcast_ref()).iter().any(|w| {
        w.downcast_ref::<gtk::Label>()
            .is_some_and(|l| l.label().contains("Scroll to browse"))
    }));
    capture(&window, "reels-next-wide");
    let scroll = descendants(reels.root.upcast_ref())
        .iter()
        .filter(|w| w.widget_name() == "reels-browser")
        .find_map(|w| {
            let controllers = w.observe_controllers();
            (0..controllers.n_items()).find_map(|i| {
                controllers
                    .item(i)
                    .and_downcast::<gtk::EventControllerScroll>()
            })
        })
        .unwrap();
    scroll.emit_by_name::<bool>("scroll", &[&0.0f64, &1.0f64]);
    assert!(descendants(reels.root.upcast_ref()).iter().any(|w| {
        w.downcast_ref::<gtk::Label>()
            .is_some_and(|l| l.label().starts_with("Reel 3"))
    }));
    let keys = descendants(reels.root.upcast_ref())
        .iter()
        .filter(|w| w.widget_name() == "reels-browser")
        .find_map(|w| {
            let controllers = w.observe_controllers();
            (0..controllers.n_items()).find_map(|i| {
                controllers
                    .item(i)
                    .and_downcast::<gtk::EventControllerKey>()
            })
        })
        .unwrap();
    keys.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::Up, &0u32, &gdk::ModifierType::empty()],
    );
    keys.emit_by_name::<bool>(
        "key-pressed",
        &[&gdk::Key::space, &0u32, &gdk::ModifierType::empty()],
    );
    assert!(descendants(reels.root.upcast_ref()).iter().any(|w| {
        w.downcast_ref::<gtk::Label>()
            .is_some_and(|l| l.label() == "This video is unavailable.")
    }));
    window.set_default_size(360, 700);
    ui.destinations.set_visible_child_name("messages");
    ui.collections.borrow()[3].review_finish(Ok(Page::complete(vec![Item::Conversation(
        Conversation {
            id: "5".into(),
            title: "Alex Morgan".into(),
            participants: vec![user()],
            preview: "See you soon!".into(),
            preview_sender: "42".into(),
            preview_timestamp: 1788800000000000,
            ..Conversation::default()
        },
    )])));
    capture(&window, "inbox-narrow");
    window.set_default_size(1100, 820);
    capture(&window, "inbox-wide");
    let inbox = ui.collections.borrow()[3].clone();
    inbox.review_open_thread();
    capture(&window, "inbox-selected-wide");
    window.set_default_size(360, 700);
    capture(&window, "inbox-selected-narrow");
    assert!(inbox.back());
    settle();
    assert!(!inbox.back());
    assert!(window.width() <= 360);
    ui.push(Route::Thread(Conversation {
        id: "5".into(),
        title: "Alex Morgan".into(),
        participants: vec![user()],
        preview: String::new(),
        preview_sender: String::new(),
        preview_timestamp: 0,
        ..Conversation::default()
    }));
    let thread = ui.collections.borrow().last().unwrap().clone();
    thread.review_finish(Ok(Page::complete(vec![Item::Message(Message {id:"1".into(), sender:"42".into(), text:"Hello! This message is intentionally long enough to need more than three lines on a small screen. The full message should be readable in the conversation without opening a separate page or losing the end of the text.".into(), timestamp:1788800000000000, attachments:vec![], ..Message::default()}), Item::Message(Message {id:"2".into(), sender:"42".into(), text:"See you soon.".into(), timestamp:1788800001000000, attachments:vec![], ..Message::default()})])));
    capture(&window, "thread-narrow");
    window.set_default_size(1100, 820);
    capture(&window, "thread-wide");
    let widgets = descendants(thread.root.upcast_ref());
    let composer = widgets
        .iter()
        .find_map(|w| w.downcast_ref::<gtk::Entry>())
        .unwrap();
    let send = widgets
        .iter()
        .find_map(|w| {
            w.downcast_ref::<gtk::Button>()
                .filter(|b| b.tooltip_text().as_deref() == Some("Send"))
        })
        .unwrap();
    assert!(!send.is_sensitive());
    composer.set_text("   ");
    assert!(!send.is_sensitive());
    composer.set_text("A draft");
    assert!(send.is_sensitive());
    window.set_default_size(360, 700);
    for (route, items, name) in [
        (Route::Comments(post()), vec![Item::Comment(Comment { id: "comment".into(), author: user(), liked: true, likes: 12, text: "A longer comment that should wrap naturally and remain fully readable at the minimum window size.".into() })], "comments-minimum"),
        (Route::Followers(user(), false), vec![Item::User(user())], "followers-minimum"),
        (Route::Stories, vec![Item::Story(Story { id: "story".into(), author: user(), seen: false })], "stories-minimum"),
        (Route::Notifications, vec![Item::Notification(Notification { id: "notification".into(), text: "Alex Morgan and other people liked your photo.".into(), user: Some(user()) })], "notifications-minimum"),
    ] {
        ui.push(route);
        ui.collections.borrow().last().unwrap().review_finish(Ok(Page::complete(items)));
        capture(&window, name);
    }
    let settings = gtk::Settings::default().unwrap();
    let old_font = settings.gtk_font_name();
    settings.set_gtk_font_name(Some("Sans 15"));
    capture(&window, "notifications-large-text");
    assert!(
        window.width() <= 360,
        "large text must not force a wider window"
    );
    settings.set_gtk_font_name(old_font.as_deref());
    let activity = ui.collections.borrow().last().unwrap().clone();
    activity.review_finish(Ok(Page::complete(vec![Item::Notification(Notification {
        id: "system".into(),
        text: "Welcome to your activity. Updates from Viewfinder appear here too.".into(),
        user: None,
    })])));
    window.set_default_size(1100, 820);
    capture(&window, "notifications-wide");
    for (route, items, name) in [
        (
            Route::Search("alex".into()),
            vec![Item::User(user())],
            "search-accounts",
        ),
        (
            Route::Keyword("city walks".into()),
            (0..8)
                .map(|i| {
                    let mut p = post();
                    p.id = format!("search-{i}");
                    Item::Post(p)
                })
                .collect(),
            "search-posts",
        ),
    ] {
        ui.push(route);
        ui.collections
            .borrow()
            .last()
            .unwrap()
            .review_finish(Ok(Page::complete(items)));
        window.set_default_size(1100, 820);
        capture(&window, &format!("{name}-wide"));
        window.set_default_size(360, 700);
        capture(&window, &format!("{name}-narrow"));
        assert!(window.width() <= 360, "search must fit a narrow window");
    }
    // Exercise switching result models and editing the query in place.
    let results = ui.collections.borrow().last().unwrap().clone();
    let widgets = descendants(results.root.upcast_ref());
    let kind = widgets
        .iter()
        .find_map(|w| w.downcast_ref::<adw::ToggleGroup>())
        .unwrap();
    let entry = widgets
        .iter()
        .find_map(|w| w.downcast_ref::<gtk::SearchEntry>())
        .unwrap();
    let submit = widgets
        .iter()
        .find_map(|w| {
            w.downcast_ref::<gtk::Button>()
                .filter(|b| b.label().as_deref() == Some("Search"))
        })
        .unwrap();
    entry.set_text("  ");
    assert!(!submit.is_sensitive());
    entry.set_text("Alex");
    kind.set_active(0);
    let count = ui.collections.borrow().len();
    entry.emit_activate();
    settle();
    assert_eq!(
        ui.collections.borrow().len(),
        count,
        "changing type replaces the results page"
    );
    let accounts = ui.collections.borrow().last().unwrap().clone();
    accounts.review_finish(Ok(Page::complete(vec![Item::User(user())])));
    assert!(
        descendants(accounts.root.upcast_ref())
            .iter()
            .any(|w| w.is::<gtk::ListView>())
    );
    let widgets = descendants(accounts.root.upcast_ref());
    let entry = widgets
        .iter()
        .find_map(|w| w.downcast_ref::<gtk::SearchEntry>())
        .unwrap();
    entry.set_text("Morgan");
    entry.emit_activate();
    assert_eq!(accounts.page.borrow().as_ref().unwrap().title(), "Morgan");
    let style = adw::StyleManager::default();
    style.set_color_scheme(adw::ColorScheme::ForceLight);
    accounts.review_finish(Ok(Page::complete(vec![Item::User(user())])));
    capture(&window, "search-accounts-light");
    style.set_color_scheme(adw::ColorScheme::Default);
    ui.push(Route::Search("a search with no matches".into()));
    let results = ui.collections.borrow().last().unwrap().clone();
    capture(&window, "loading");
    results.review_finish(Ok(Page::complete(vec![])));
    capture(&window, "empty");
    results.review_finish(Err(crate::viewfinder::Error::Network));
    capture(&window, "error");
    super::present_search(&ui);
    capture(&window, "search");
    let dialog = window.visible_dialog().unwrap();
    let entries = descendants(dialog.upcast_ref());
    let entry = entries
        .iter()
        .find_map(|w| w.downcast_ref::<gtk::SearchEntry>())
        .unwrap();
    assert!(entry.has_focus() || entry.focus_child().is_some());
    entry.set_text("Alex");
    let search = entries
        .iter()
        .find_map(|w| {
            w.downcast_ref::<gtk::Button>()
                .filter(|b| b.label().as_deref() == Some("Search"))
        })
        .unwrap();
    assert!(search.is_sensitive());
    entry.emit_activate();
    settle();
    assert!(window.visible_dialog().is_none());
    home.review_refresh_states();
    viewer::present(&ui, vec![post()], 0, false);
    capture(&window, "viewer");
    window.visible_dialog().unwrap().close();
    settle();
    let mut missing = post();
    missing.media[0].image = None;
    missing.media[0].thumbnail = None;
    viewer::present(&ui, vec![missing], 0, false);
    capture(&window, "viewer-unavailable");
    window.visible_dialog().unwrap().close();
    settle();
    let mut carousel = post();
    let mut video = carousel.media[0].clone();
    video.video = Some("fixture://video".into());
    carousel.media.push(video);
    viewer::present(&ui, vec![carousel], 0, false);
    settle();
    let dialog = window.visible_dialog().unwrap();
    let widgets = descendants(dialog.upcast_ref());
    let previous = widgets
        .iter()
        .find_map(|w| {
            w.downcast_ref::<gtk::Button>()
                .filter(|b| b.tooltip_text().as_deref() == Some("Previous media"))
        })
        .unwrap();
    let next = widgets
        .iter()
        .find_map(|w| {
            w.downcast_ref::<gtk::Button>()
                .filter(|b| b.tooltip_text().as_deref() == Some("Next media"))
        })
        .unwrap();
    assert!(!previous.is_sensitive());
    assert!(next.is_sensitive());
    let controllers = dialog.observe_controllers();
    let keys = (0..controllers.n_items())
        .find_map(|i| {
            controllers
                .item(i)
                .unwrap()
                .downcast::<gtk::EventControllerKey>()
                .ok()
        })
        .unwrap();
    let key = |value: gdk::Key| {
        keys.emit_by_name::<bool>("key-pressed", &[&value, &0u32, &gdk::ModifierType::empty()]);
    };
    key(gdk::Key::Right);
    assert!(previous.is_sensitive());
    assert!(!next.is_sensitive());
    key(gdk::Key::space);
    capture(&window, "video-unavailable");
    assert!(
        widgets
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::Label>())
            .any(|l| l.label() == "This video is unavailable.")
    );
    key(gdk::Key::Left);
    assert!(!previous.is_sensitive());
    key(gdk::Key::Escape);
    settle();
    assert!(window.visible_dialog().is_none());
    window.close();
    UI.with(|slot| slot.borrow_mut().take());
    settle();
}

#[test]
#[ignore = "opens a real GTK window; run explicitly on a desktop"]
fn comments_review() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    adw::init().unwrap();
    let app = adw::Application::builder()
        .application_id("io.github._6E6B.viewfinder.CommentsReview")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    super::activate(&app);
    let ui = UI.with(|slot| slot.borrow().as_ref().unwrap().clone());
    let window = ui.window.upgrade().unwrap();
    ui.images.review_seed();
    ui.login_stack.set_visible_child_name("app");
    super::engagement::review(&ui);
    ui.push(Route::Comments(post()));
    let comments = ui.collections.borrow().last().unwrap().clone();
    comments.review_finish(Ok(Page::complete((0..8).map(|i| Item::Comment(Comment {
        id: format!("comment-{i}"), author: user(),
        text: if i == 0 { "A longer comment that wraps naturally and remains readable on a narrow window. Love the light in this photo!".into() }
            else { "Such a lovely view.".into() }, liked: i == 0, likes: 12 + i,
    })).collect())));
    for (width, name) in [(1100, "comments-wide"), (360, "comments-narrow")] {
        window.set_default_size(width, 800);
        capture(&window, name);
        assert!(window.width() <= width);
        assert!(ui.client.borrow().is_none());
        assert!(
            descendants(comments.root.upcast_ref()).iter().any(|w| w
                .downcast_ref::<gtk::Label>()
                .is_some_and(|l| l.is_mapped() && l.label() == "Such a lovely view.")),
            "comment rows must be rendered"
        );
    }
    let widgets = descendants(comments.root.upcast_ref());
    let entry = widgets
        .iter()
        .find_map(|w| w.downcast_ref::<gtk::Entry>())
        .unwrap();
    let send = widgets
        .iter()
        .filter_map(|w| w.downcast_ref::<gtk::Button>())
        .find(|b| b.label().as_deref() == Some("Post"))
        .unwrap();
    entry.set_text("   ");
    assert!(!send.is_sensitive());
    entry.set_text("A comment draft");
    assert!(send.is_sensitive());
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    capture(&window, "comments-dark");
    window.close();
    UI.with(|slot| slot.borrow_mut().take());
    settle();
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        widgets.extend(descendants(&current));
        child = current.next_sibling();
    }
    widgets
}

#[test]
#[ignore = "opens a real GTK window; run explicitly on a desktop"]
fn messages_review() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _guard = runtime.enter();
    adw::init().unwrap();
    let app = adw::Application::builder()
        .application_id("io.github._6E6B.viewfinder.MessagesReview")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    super::activate(&app);
    let ui = UI.with(|slot| slot.borrow().as_ref().unwrap().clone());
    let window = ui.window.upgrade().unwrap();
    ui.images.review_seed();
    ui.login_stack.set_visible_child_name("app");
    ui.destinations.set_visible_child_name("messages");
    let inbox = ui.collections.borrow()[2].clone();
    inbox.review_finish(Ok(Page::complete(
        (0..8)
            .map(|i| {
                Item::Conversation(Conversation {
                    id: i.to_string(),
                    title: if i == 0 {
                        "Alex Morgan".into()
                    } else {
                        format!("Conversation {i}")
                    },
                    participants: vec![user()],
                    preview: "See you soon! I'll send the photos here.".into(),
                    preview_sender: "42".into(),
                    preview_timestamp: 1788800000000000,
                    unread: i == 0,
                    ..Conversation::default()
                })
            })
            .collect(),
    )));
    inbox.review_open_thread();
    window.set_default_size(1100, 820);
    capture(&window, "messages-redesign-wide");
    let widgets = descendants(inbox.root.upcast_ref());
    assert!(widgets.iter().any(|w| {
        w.downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.label() == "Seen by alex")
    }));
    assert!(widgets.iter().any(|w| {
        w.downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.label() == "●")
    }));
    let list = widgets
        .iter()
        .find_map(|w| {
            w.downcast_ref::<gtk::ListView>()
                .filter(|l| l.has_css_class("conversation-list"))
        })
        .unwrap();
    assert_eq!(
        list.model()
            .and_downcast::<gtk::SingleSelection>()
            .unwrap()
            .selected(),
        0
    );
    window.set_default_size(360, 700);
    capture(&window, "messages-redesign-narrow");
    assert!(window.width() <= 360);
    inbox.review_dm_media();
    capture(&window, "messages-media-narrow");
    assert!(window.width() <= 360);
    window.set_default_size(1100, 820);
    capture(&window, "messages-media-wide");
    let widgets = descendants(inbox.root.upcast_ref());
    assert_eq!(
        widgets
            .iter()
            .filter(|w| w.has_css_class("message-divider") && w.is_visible())
            .count(),
        2
    );
    assert!(
        widgets
            .iter()
            .any(|w| w.has_css_class("message-bubble") && w.has_css_class("join-previous"))
    );
    window.set_default_size(360, 700);
    settle();
    assert!(inbox.back());
    capture(&window, "messages-redesign-sidebar");
    assert!(ui.shell.is_collapsed());
    ui.shell.set_show_sidebar(true);
    capture(&window, "navigation-drawer");
    let widgets = descendants(ui.shell.upcast_ref());
    let navigation = widgets
        .iter()
        .find_map(|w| {
            w.downcast_ref::<gtk::ListBox>()
                .filter(|list| list.widget_name() == "app-navigation")
        })
        .unwrap();
    assert_eq!(navigation.selected_row().unwrap().widget_name(), "messages");
    let home_row = navigation.row_at_index(0).unwrap();
    navigation.emit_by_name::<()>("row-activated", &[&home_row]);
    settle();
    assert_eq!(
        ui.destinations.visible_child_name().as_deref(),
        Some("home")
    );
    assert!(!ui.shell.shows_sidebar());
    ui.destinations.set_visible_child_name("messages");
    assert_eq!(navigation.selected_row().unwrap().widget_name(), "messages");
    window.set_default_size(1200, 820);
    settle();
    assert!(!ui.shell.is_collapsed());
    assert!(ui.shell.shows_sidebar());
    let style = adw::StyleManager::default();
    let old_scheme = style.color_scheme();
    style.set_color_scheme(adw::ColorScheme::ForceLight);
    capture(&window, "navigation-light");
    style.set_color_scheme(old_scheme);
    window.close();
    UI.with(|slot| slot.borrow_mut().take());
    settle();
}
