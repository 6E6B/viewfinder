use super::{Ui, icon_button, label};
use crate::{
    app::{self, Task},
    domain::*,
    media,
};
use adw::prelude::*;
use gtk::{gdk, glib};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

pub struct Tray {
    pub root: gtk::ScrolledWindow,
    row: gtk::Box,
    ui: Weak<Ui>,
    task: RefCell<Option<Task>>,
    loaded: Cell<bool>,
}

impl Tray {
    pub fn new(ui: &Rc<Ui>) -> Rc<Self> {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("story-tray");
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&row)
            .build();
        Rc::new(Self {
            root,
            row,
            ui: Rc::downgrade(ui),
            task: RefCell::new(None),
            loaded: Cell::new(false),
        })
    }

    pub fn reset(&self) {
        self.task.borrow_mut().take();
        self.loaded.set(false);
        while let Some(child) = self.row.first_child() {
            self.row.remove(&child);
        }
    }

    pub fn load(self: &Rc<Self>) {
        if self.loaded.get() {
            return;
        }
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some(client) = ui.client.borrow().clone() else {
            return;
        };
        self.loaded.set(true);
        while let Some(child) = self.row.first_child() {
            self.row.remove(&child);
        }
        self.show_loading();
        *self.task.borrow_mut() = Some(app::background(
            app::load(client, Route::Stories, None),
            glib::clone!(
                #[weak(rename_to=tray)]
                self,
                move |result| {
                    while let Some(child) = tray.row.first_child() {
                        tray.row.remove(&child);
                    }
                    let Some(ui) = tray.ui.upgrade() else {
                        return;
                    };
                    match result {
                        Ok(page) => {
                            tray.populate(&ui, page);
                        }
                        Err(_) => {
                            tray.row.append(&label("Couldn’t load stories"));
                            let retry = gtk::Button::with_label("Retry");
                            retry.connect_clicked(glib::clone!(
                                #[weak]
                                tray,
                                move |_| {
                                    tray.loaded.set(false);
                                    tray.load();
                                }
                            ));
                            tray.row.append(&retry);
                        }
                    }
                }
            ),
        ));
    }

    fn populate(&self, ui: &Rc<Ui>, page: Page) {
        let stories: Rc<Vec<Story>> = Rc::new(
            page.items
                .into_iter()
                .filter_map(|item| match item {
                    Item::Story(s) => Some(s),
                    _ => None,
                })
                .collect(),
        );
        if stories.is_empty() {
            self.row.append(&label("No stories right now"));
        }
        for (index, story) in stories.iter().enumerate() {
            let button = gtk::Button::new();
            button.add_css_class("flat");
            button.add_css_class("story-person");
            button.set_tooltip_text(Some(&format!(
                "{} · {}",
                story.author.username,
                if story.seen {
                    "Seen stories"
                } else {
                    "New stories"
                }
            )));
            let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let ring = gtk::Box::new(gtk::Orientation::Vertical, 0);
            ring.set_halign(gtk::Align::Center);
            ring.add_css_class("story-ring");
            ring.add_css_class(if story.seen { "seen" } else { "unseen" });
            ring.append(&media::avatar(ui.images.clone(), &story.author, 56));
            column.append(&ring);
            let name = label(&story.author.username);
            name.set_selectable(false);
            name.set_xalign(0.5);
            name.set_max_width_chars(11);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            column.append(&name);
            button.set_child(Some(&column));
            button.connect_clicked(glib::clone!(
                #[weak]
                ui,
                #[strong]
                stories,
                move |_| {
                    StoryViewer::present(&ui, stories.clone(), index);
                }
            ));
            self.row.append(&button);
        }
    }

    fn show_loading(&self) {
        let spinner = adw::Spinner::new();
        spinner.set_size_request(20, 20);
        spinner.set_margin_top(28);
        spinner.set_margin_bottom(28);
        spinner.set_tooltip_text(Some("Loading stories"));
        self.row.append(&spinner);
    }
}

struct StoryViewer {
    ui: Weak<Ui>,
    stories: Rc<Vec<Story>>,
    friend: Cell<usize>,
    index: Cell<usize>,
    posts: RefCell<Vec<Post>>,
    task: RefCell<Option<Task>>,
    dialog: glib::WeakRef<adw::Dialog>,
    title: adw::WindowTitle,
    content: gtk::Box,
    progress: gtk::Box,
    actions: gtk::Box,
    status: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    pause: gtk::Button,
    paused: Cell<bool>,
    loading: Cell<bool>,
    last_scroll: Cell<Option<Instant>>,
}

impl StoryViewer {
    fn present(ui: &Rc<Ui>, stories: Rc<Vec<Story>>, index: usize) -> Rc<Self> {
        let dialog = adw::Dialog::builder()
            .title("Stories")
            .content_width(480)
            .content_height(800)
            .build();
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let title = adw::WindowTitle::new("", "");
        header.set_title_widget(Some(&title));
        toolbar.add_top_bar(&header);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let progress = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        progress.set_homogeneous(true);
        progress.add_css_class("story-progress");
        body.append(&progress);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_vexpand(true);
        let surface = gtk::Overlay::new();
        surface.set_child(Some(&content));
        surface.set_vexpand(true);
        let buffering = adw::Spinner::new();
        buffering.set_size_request(24, 24);
        buffering.set_halign(gtk::Align::Center);
        buffering.set_valign(gtk::Align::Center);
        buffering.set_tooltip_text(Some("Loading story"));
        buffering.set_visible(false);
        surface.add_overlay(&buffering);
        body.append(&surface);
        let status = label("");
        status.set_visible(false);
        status.connect_label_notify(glib::clone!(
            #[weak]
            buffering,
            move |label| {
                let is_buffering = label.label() == "Buffering…";
                buffering.set_visible(is_buffering);
                label.set_visible(!is_buffering && !label.label().is_empty());
            }
        ));
        body.append(&status);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        controls.set_halign(gtk::Align::Center);
        controls.set_margin_bottom(12);
        let previous = icon_button("go-previous-symbolic", "Previous story");
        let next = icon_button("go-next-symbolic", "Next story");
        let pause = icon_button("media-playback-pause-symbolic", "Pause or resume video");
        let mute = gtk::ToggleButton::builder()
            .icon_name("audio-volume-muted-symbolic")
            .tooltip_text("Mute audio")
            .active(ui.playback.muted.get())
            .build();
        mute.set_icon_name(if mute.is_active() {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });
        mute.connect_toggled(glib::clone!(
            #[weak]
            ui,
            move |button| {
                ui.playback.mute(button.is_active());
                button.set_icon_name(if button.is_active() {
                    "audio-volume-muted-symbolic"
                } else {
                    "audio-volume-high-symbolic"
                });
            }
        ));
        for widget in [previous.clone(), pause.clone(), next.clone()] {
            controls.append(&widget);
        }
        controls.append(&mute);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        controls.append(&actions);
        body.append(&controls);
        toolbar.set_content(Some(&body));
        dialog.set_child(Some(&toolbar));
        let viewer = Rc::new(Self {
            ui: Rc::downgrade(ui),
            stories,
            friend: Cell::new(index),
            index: Cell::new(0),
            posts: RefCell::new(vec![]),
            task: RefCell::new(None),
            dialog: dialog.downgrade(),
            title,
            content,
            progress,
            actions,
            status,
            previous: previous.clone(),
            next: next.clone(),
            pause: pause.clone(),
            paused: Cell::new(false),
            loading: Cell::new(false),
            last_scroll: Cell::new(None),
        });
        previous.connect_clicked(glib::clone!(
            #[weak]
            viewer,
            move |_| viewer.step(false)
        ));
        next.connect_clicked(glib::clone!(
            #[weak]
            viewer,
            move |_| viewer.step(true)
        ));
        pause.connect_clicked(glib::clone!(
            #[weak]
            viewer,
            move |_| viewer.toggle()
        ));
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak]
            viewer,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                match key {
                    gdk::Key::Left | gdk::Key::Up => viewer.step(false),
                    gdk::Key::Right | gdk::Key::Down => viewer.step(true),
                    gdk::Key::space => viewer.toggle(),
                    gdk::Key::Escape => {
                        if let Some(dialog) = viewer.dialog.upgrade() {
                            dialog.close();
                        }
                    }
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            }
        ));
        dialog.add_controller(keys);
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::DISCRETE,
        );
        scroll.connect_scroll(glib::clone!(
            #[weak]
            viewer,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, dx, dy| {
                let delta = if dy.abs() > dx.abs() { dy } else { dx };
                if delta != 0.0
                    && viewer
                        .last_scroll
                        .get()
                        .is_none_or(|last| last.elapsed() > Duration::from_millis(350))
                {
                    viewer.last_scroll.set(Some(Instant::now()));
                    viewer.step(delta > 0.0);
                }
                glib::Propagation::Stop
            }
        ));
        viewer.content.add_controller(scroll);
        viewer.load(false);
        let keep_alive = viewer.clone();
        dialog.connect_closed(move |_| {
            let viewer = &keep_alive;
            viewer.task.borrow_mut().take();
            if let Some(ui) = viewer.ui.upgrade() {
                ui.playback.stop();
            }
        });
        dialog.present(ui.window.upgrade().as_ref());
        viewer
    }

    fn clear(&self) {
        while let Some(child) = self.actions.first_child() {
            self.actions.remove(&child);
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.playback.stop();
        }
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        while let Some(child) = self.progress.first_child() {
            self.progress.remove(&child);
        }
        self.status.set_label("");
        self.pause.set_visible(false);
    }

    fn load(self: &Rc<Self>, last: bool) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some(client) = ui.client.borrow().clone() else {
            return;
        };
        self.clear();
        self.posts.borrow_mut().clear();
        self.loading.set(true);
        self.previous.set_sensitive(false);
        self.next.set_sensitive(false);
        let story = self.stories[self.friend.get()].clone();
        self.title.set_title(&story.author.username);
        self.title.set_subtitle(&format!(
            "Friend {} of {}",
            self.friend.get() + 1,
            self.stories.len()
        ));
        let spinner = adw::Spinner::new();
        spinner.set_valign(gtk::Align::Center);
        spinner.set_vexpand(true);
        self.content.append(&spinner);
        *self.task.borrow_mut() = Some(app::background(
            app::load(client, Route::Story(story), None),
            glib::clone!(
                #[weak(rename_to=v)]
                self,
                move |result| {
                    v.loading.set(false);
                    match result {
                        Ok(page) => {
                            *v.posts.borrow_mut() = page
                                .items
                                .into_iter()
                                .filter_map(|item| match item {
                                    Item::Post(p) => Some(p),
                                    _ => None,
                                })
                                .collect();
                            v.index.set(if last {
                                v.posts.borrow().len().saturating_sub(1)
                            } else {
                                0
                            });
                            v.render();
                        }
                        Err(_) => {
                            v.clear();
                            v.status.set_label(
                                "Couldn’t load these stories. Retry or move to the next friend.",
                            );
                            let retry = gtk::Button::with_label("Retry");
                            retry.set_halign(gtk::Align::Center);
                            retry.connect_clicked(glib::clone!(
                                #[weak]
                                v,
                                move |_| v.load(last)
                            ));
                            v.content.append(&retry);
                            v.previous.set_sensitive(v.friend.get() > 0);
                            v.next.set_sensitive(true);
                        }
                    }
                }
            ),
        ));
    }

    fn render(&self) {
        self.clear();
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let posts = self.posts.borrow();
        self.previous
            .set_sensitive(self.index.get() > 0 || self.friend.get() > 0);
        self.next.set_sensitive(true);
        self.next.set_tooltip_text(Some(
            if self.friend.get() + 1 == self.stories.len() && self.index.get() + 1 >= posts.len() {
                "Finish stories"
            } else {
                "Next story"
            },
        ));
        let first_segment = (self.index.get() / 30) * 30;
        for i in first_segment..posts.len().min(first_segment + 30) {
            let segment = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            segment.add_css_class("story-segment");
            if i <= self.index.get() {
                segment.add_css_class("complete");
            }
            segment.set_hexpand(true);
            self.progress.append(&segment);
        }
        let Some(post) = posts.get(self.index.get()) else {
            self.status
                .set_label("No stories available. Continue to the next friend.");
            return;
        };
        self.title.set_subtitle(&format!(
            "Story {} of {} · Friend {} of {}",
            self.index.get() + 1,
            posts.len(),
            self.friend.get() + 1,
            self.stories.len()
        ));
        if ui
            .client
            .borrow()
            .as_ref()
            .is_none_or(|c| c.account_id != post.author.id)
        {
            self.actions.append(&super::engagement::button(
                &ui,
                LikeTarget::Story(post.id.clone()),
                post.liked,
                0,
            ));
        }
        if let Some(item) = post.media.first() {
            if let Some(url) = &item.video {
                let picture = gtk::Picture::builder()
                    .vexpand(true)
                    .can_shrink(true)
                    .content_fit(gtk::ContentFit::Contain)
                    .build();
                self.content.append(&picture);
                match ui.playback.start(url, &picture, &self.status) {
                    Ok(()) => {
                        let next_video = posts
                            .get(self.index.get() + 1)
                            .and_then(|post| post.media.first())
                            .and_then(|media| media.video.clone());
                        ui.playback.preload_next(next_video);
                    }
                    Err(error) => self.status.set_label(&error),
                }
                self.paused.set(false);
                self.pause.set_icon_name("media-playback-pause-symbolic");
                self.pause.set_visible(true);
            } else {
                let picture = media::preview(ui.images.clone(), item, 1600, false);
                picture.set_vexpand(true);
                self.content.append(&picture);
            }
        } else {
            self.status.set_label("Story media unavailable");
        }
    }

    fn step(self: &Rc<Self>, forward: bool) {
        if self.loading.get() {
            return;
        }
        let count = self.posts.borrow().len();
        if forward {
            if self.index.get() + 1 < count {
                self.index.set(self.index.get() + 1);
                self.render();
            } else if self.friend.get() + 1 < self.stories.len() {
                self.friend.set(self.friend.get() + 1);
                self.load(false);
            } else if let Some(dialog) = self.dialog.upgrade() {
                dialog.close();
            }
        } else if self.index.get() > 0 {
            self.index.set(self.index.get() - 1);
            self.render();
        } else if self.friend.get() > 0 {
            self.friend.set(self.friend.get() - 1);
            self.load(true);
        }
    }

    fn toggle(&self) {
        if !self.pause.is_visible() {
            return;
        }
        if let Some(ui) = self.ui.upgrade() {
            self.paused.set(!self.paused.get());
            ui.playback.pause(self.paused.get());
            self.pause.set_icon_name(if self.paused.get() {
                "media-playback-start-symbolic"
            } else {
                "media-playback-pause-symbolic"
            });
        }
    }
}

#[cfg(test)]
pub(super) fn review(ui: &Rc<Ui>) {
    let window = ui.window.upgrade().unwrap();
    let tray = ui.collections.borrow()[0].stories.as_ref().unwrap().clone();
    tray.show_loading();
    super::review::capture(&window, "stories-loading");
    tray.reset();
    tray.populate(
        ui,
        Page::complete(
            (0..12)
                .map(|i| {
                    Item::Story(Story {
                        id: i.to_string(),
                        author: User {
                            username: format!("friend_{i}"),
                            ..User::default()
                        },
                        seen: i > 2,
                    })
                })
                .collect(),
        ),
    );
    for (width, height, name) in [
        (1200, 850, "story-tray-wide"),
        (390, 700, "story-tray-narrow"),
    ] {
        window.set_default_size(width, height);
        super::review::capture(&window, name);
        assert!(tray.root.width() <= 600);
        assert!(window.width() <= width);
    }
    let stories = Rc::new(vec![Story {
        id: "fixture".into(),
        author: User {
            username: "a_friend_with_a_long_username".into(),
            ..User::default()
        },
        seen: false,
    }]);
    let viewer = StoryViewer::present(ui, stories, 0);
    viewer.title.set_title("a_friend_with_a_long_username");
    *viewer.posts.borrow_mut() = (0..15)
        .map(|index| Post {
            id: index.to_string(),
            code: String::new(),
            author: User::default(),
            caption: String::new(),
            media: vec![Media {
                thumbnail: None,
                image: Some("fixture://portrait".into()),
                video: None,
                width: 1080,
                height: 1920,
            }],
            liked: false,
            likes: 0,
            comments: 0,
            timestamp: 0,
        })
        .collect();
    viewer.render();
    for (width, height, name) in [(1200, 850, "stories-wide"), (390, 700, "stories-narrow")] {
        window.set_default_size(width, height);
        super::review::capture(&window, name);
        let dialog = viewer.dialog.upgrade().unwrap();
        assert!(window.width() <= width, "window must fit requested width");
        assert!(
            dialog.width() <= window.width(),
            "story dialog must fit window"
        );
        assert!(
            viewer.progress.width() <= dialog.width(),
            "segments must fit dialog"
        );
        assert!(
            viewer.content.width() <= dialog.width(),
            "media must fit dialog"
        );
        for button in [&viewer.previous, &viewer.next] {
            let bounds = button.compute_bounds(&dialog).unwrap();
            assert!(bounds.x() >= 0.0 && bounds.x() + bounds.width() <= dialog.width() as f32);
        }
    }
    viewer.step(true);
    assert_eq!(viewer.index.get(), 1);
    viewer.step(false);
    assert_eq!(viewer.index.get(), 0);
    viewer.dialog.upgrade().unwrap().close();
}
