use super::{
    Ui,
    collection::{comment_button, like_button},
    icon_button, label,
};
use crate::{domain::*, media};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

const PLAYBACK_CONTROLS_HIDE_DELAY: Duration = Duration::from_millis(1800);

fn show_playback_controls(
    controls: &gtk::Box,
    pointer_inside: &Rc<Cell<bool>>,
    activity: &Rc<Cell<u64>>,
) {
    controls.set_visible(true);
    let ticket = activity.get().wrapping_add(1);
    activity.set(ticket);
    glib::timeout_add_local_once(
        PLAYBACK_CONTROLS_HIDE_DELAY,
        glib::clone!(
            #[weak]
            controls,
            #[strong]
            pointer_inside,
            #[strong]
            activity,
            move || {
                if activity.get() == ticket && !pointer_inside.get() {
                    controls.set_visible(false);
                }
            }
        ),
    );
}

/// One visible reel and one decoder, independent of the number of loaded posts.
pub(super) struct Reels {
    pub root: gtk::Box,
    ui: Weak<Ui>,
    store: gio::ListStore,
    index: Cell<u32>,
    stage: gtk::Box,
    previous: gtk::Button,
    next: gtk::Button,
    position: gtk::Label,
    last_scroll: RefCell<Option<Instant>>,
    more: Box<dyn Fn()>,
    exhausted: Cell<bool>,
    pending: Cell<bool>,
    autoplay: Rc<Cell<bool>>,
    play: RefCell<Option<gtk::Button>>,
}
impl Reels {
    pub fn new(ui: &Rc<Ui>, store: &gio::ListStore, more: impl Fn() + 'static) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        root.set_margin_start(12);
        root.set_margin_end(12);
        root.set_focusable(true);
        root.set_widget_name("reels-browser");
        let stage = gtk::Box::new(gtk::Orientation::Vertical, 8);
        stage.set_vexpand(true);
        root.append(&stage);
        let previous = icon_button("go-up-symbolic", "Previous reel");
        let next = icon_button("go-down-symbolic", "Next reel");
        previous.add_css_class("circular");
        next.add_css_class("circular");
        let position = gtk::Label::new(None);
        position.add_css_class("caption");
        position.add_css_class("dim-label");
        let r = Rc::new(Self {
            root,
            ui: Rc::downgrade(ui),
            store: store.clone(),
            index: Cell::new(0),
            stage,
            previous,
            next,
            position,
            last_scroll: RefCell::new(None),
            more: Box::new(more),
            exhausted: Cell::new(false),
            pending: Cell::new(false),
            autoplay: Rc::new(Cell::new(true)),
            play: RefCell::new(None),
        });
        r.previous.connect_clicked(glib::clone!(
            #[weak]
            r,
            move |_| r.step(false)
        ));
        r.next.connect_clicked(glib::clone!(
            #[weak]
            r,
            move |_| r.step(true)
        ));
        store.connect_items_changed(glib::clone!(
            #[weak]
            r,
            move |_, pos, removed, _| {
                if removed > 0 || pos == 0 {
                    r.index.set(0);
                    r.stage.set_widget_name("");
                    r.pending.set(false);
                } else if r.pending.replace(false) && r.index.get() + 1 < r.store.n_items() {
                    r.index.set(r.index.get() + 1);
                }
                r.render();
            }
        ));
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
        );
        scroll.connect_scroll(glib::clone!(
            #[weak]
            r,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _, dy| {
                if dy != 0.0
                    && r.last_scroll
                        .borrow()
                        .is_none_or(|t| t.elapsed() > Duration::from_millis(350))
                {
                    r.last_scroll.replace(Some(Instant::now()));
                    r.step(dy > 0.0);
                }
                glib::Propagation::Stop
            }
        ));
        r.root.add_controller(scroll);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak]
            r,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                match key {
                    gdk::Key::Down | gdk::Key::Page_Down => r.step(true),
                    gdk::Key::Up | gdk::Key::Page_Up => r.step(false),
                    gdk::Key::space => {
                        if let Some(play) = r.play.borrow().as_ref() {
                            play.emit_clicked();
                        }
                    }
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            }
        ));
        r.root.add_controller(keys);
        r.root.connect_map(glib::clone!(
            #[weak]
            r,
            move |_| {
                r.autoplay.set(true);
                r.stage.set_widget_name("");
                r.render();
                r.root.grab_focus();
            }
        ));
        r.root.connect_unmap(glib::clone!(
            #[weak]
            r,
            move |_| {
                r.autoplay.set(false);
                if let Some(ui) = r.ui.upgrade() {
                    ui.playback.stop();
                }
            }
        ));
        r
    }
    pub fn set_exhausted(&self, exhausted: bool) {
        self.exhausted.set(exhausted);
        self.next
            .set_sensitive(!exhausted || self.index.get() + 1 < self.store.n_items());
        if exhausted {
            self.pending.set(false);
        }
    }
    fn step(&self, forward: bool) {
        let count = self.store.n_items();
        let next = if forward {
            self.index.get().saturating_add(1)
        } else {
            self.index.get().saturating_sub(1)
        };
        if next < count && next != self.index.get() {
            self.index.set(next);
            self.render();
        }
        if forward && !self.exhausted.get() && next.saturating_add(2) >= count {
            self.pending.set(next >= count);
            (self.more)();
        }
    }
    fn render(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        // Loading an additional batch should not interrupt the current reel.
        let count = self.store.n_items();
        self.previous.set_sensitive(self.index.get() > 0);
        self.next
            .set_sensitive(!self.exhausted.get() || self.index.get() + 1 < count);
        self.position
            .set_label(&format!("Reel {}", self.index.get() + 1));
        let Some(object) = self
            .store
            .item(self.index.get())
            .and_downcast::<glib::BoxedAnyObject>()
        else {
            while let Some(child) = self.stage.first_child() {
                self.stage.remove(&child);
            }
            return;
        };
        let Item::Post(post) = &*object.borrow::<Item>() else {
            return;
        };
        if self.stage.widget_name().as_str() == post.id {
            self.preload_next(&ui);
            return;
        }
        self.stage.set_widget_name(&post.id);
        self.play.borrow_mut().take();
        if self.root.is_mapped() {
            ui.playback.stop();
        }
        while let Some(child) = self.stage.first_child() {
            self.stage.remove(&child);
        }
        let author = gtk::Button::new();
        author.add_css_class("flat");
        author.set_halign(gtk::Align::Start);
        let identity = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        identity.append(&media::avatar(ui.images.clone(), &post.author, 32));
        let name = label(&post.author.username);
        name.add_css_class("heading");
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_wrap(false);
        identity.append(&name);
        author.set_child(Some(&identity));
        author.connect_clicked(glib::clone!(
            #[weak]
            ui,
            #[strong]
            post,
            move |_| ui.push(Route::Profile(post.author.clone()))
        ));
        let details = gtk::Box::new(gtk::Orientation::Vertical, 6);
        details.add_css_class("reel-details");
        details.set_hexpand(true);
        details.append(&author);
        // Pango's line limit applies per paragraph; keep the preview in one.
        let preview_caption = post
            .caption
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let caption = label(&preview_caption);
        caption.set_lines(2);
        caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
        caption.set_tooltip_text(Some(&post.caption));
        details.append(&caption);
        if post.caption.chars().count() > 80 || post.caption.lines().count() > 2 {
            let more = gtk::MenuButton::builder().label("More").build();
            more.add_css_class("flat");
            more.add_css_class("caption-expander");
            more.set_halign(gtk::Align::Start);
            more.set_tooltip_text(Some("Expand caption"));
            let full_caption = label(&post.caption);
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                // With horizontal scrolling disabled, min-content-width does
                // not constrain the wrapped label's minimum width.
                .width_request(280)
                .min_content_height(120)
                .max_content_height(320)
                .propagate_natural_height(true)
                .child(&full_caption)
                .build();
            let popover = gtk::Popover::builder().child(&scroll).build();
            more.set_popover(Some(&popover));
            details.append(&more);
        }
        if let Some(m) = post.media.first() {
            let surface = gtk::Overlay::new();
            surface.add_css_class("reel-surface");
            surface.set_overflow(gtk::Overflow::Hidden);
            surface.set_vexpand(true);
            surface.set_hexpand(true);
            let preview = media::preview(ui.images.clone(), m, 1080, false);
            surface.set_child(Some(&preview));
            let picture = gtk::Picture::builder()
                .can_shrink(true)
                .content_fit(gtk::ContentFit::Contain)
                .build();
            let buffering = adw::Spinner::new();
            buffering.set_size_request(24, 24);
            buffering.set_halign(gtk::Align::Center);
            buffering.set_valign(gtk::Align::Center);
            buffering.set_tooltip_text(Some("Loading reel"));
            buffering.set_visible(false);
            let status = gtk::Label::new(None);
            status.set_wrap(true);
            status.add_css_class("error");
            status.connect_label_notify(glib::clone!(
                #[weak]
                buffering,
                move |s| {
                    let is_buffering = s.label() == "Buffering…";
                    buffering.set_visible(is_buffering);
                    s.set_visible(!is_buffering && !s.label().is_empty());
                }
            ));
            status.set_visible(false);
            surface.add_overlay(&buffering);
            let ratio = if m.width > 0 && m.height > 0 {
                m.width as f32 / m.height as f32
            } else {
                9.0 / 16.0
            };
            let media_frame = gtk::AspectFrame::new(0.5, 0.5, ratio, false);
            media_frame.set_child(Some(&surface));
            media_frame.set_hexpand(true);
            media_frame.set_vexpand(true);
            let actions = gtk::Box::new(gtk::Orientation::Vertical, 12);
            actions.add_css_class("reel-actions");
            actions.set_halign(gtk::Align::Center);
            actions.set_valign(gtk::Align::End);
            let playback_controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            playback_controls.add_css_class("osd");
            playback_controls.add_css_class("toolbar");
            playback_controls.set_halign(gtk::Align::Start);
            playback_controls.set_valign(gtk::Align::Start);
            playback_controls.set_margin_start(12);
            playback_controls.set_margin_top(12);
            surface.add_overlay(&playback_controls);
            let pointer_inside = Rc::new(Cell::new(false));
            let playback_activity = Rc::new(Cell::new(0));
            let hover = gtk::EventControllerMotion::new();
            hover.connect_enter(glib::clone!(
                #[weak]
                playback_controls,
                #[strong]
                pointer_inside,
                #[strong]
                playback_activity,
                move |_, _, _| {
                    pointer_inside.set(true);
                    show_playback_controls(&playback_controls, &pointer_inside, &playback_activity);
                }
            ));
            hover.connect_leave(glib::clone!(
                #[weak]
                playback_controls,
                #[strong]
                pointer_inside,
                #[strong]
                playback_activity,
                move |_| {
                    pointer_inside.set(false);
                    show_playback_controls(&playback_controls, &pointer_inside, &playback_activity);
                }
            ));
            surface.add_controller(hover);
            if let Some(url) = m.video.clone() {
                let play = icon_button("media-playback-start-symbolic", "Play or pause reel");
                let started = Rc::new(Cell::new(false));
                let autoplay = self.autoplay.clone();
                let paused = Cell::new(false);
                let next_video = self.next_video();
                play.connect_clicked(glib::clone!(
                    #[weak]
                    ui,
                    #[weak]
                    surface,
                    #[strong]
                    picture,
                    #[strong]
                    status,
                    #[weak]
                    playback_controls,
                    #[strong]
                    pointer_inside,
                    #[strong]
                    playback_activity,
                    move |b| {
                        show_playback_controls(
                            &playback_controls,
                            &pointer_inside,
                            &playback_activity,
                        );
                        if !started.get()
                            || ui.playback.failed.get()
                            || picture.paintable().is_none()
                        {
                            match ui.playback.start(&url, &picture, &status) {
                                Ok(()) => {
                                    ui.playback.preload_next(next_video.clone());
                                    surface.set_child(Some(&picture));
                                    started.set(true);
                                    autoplay.set(true);
                                    paused.set(false);
                                }
                                Err(e) => {
                                    status.set_label(&e);
                                    return;
                                }
                            }
                        } else {
                            paused.set(!paused.get());
                            ui.playback.pause(paused.get());
                            autoplay.set(!paused.get());
                        }
                        b.set_icon_name(if paused.get() {
                            "media-playback-start-symbolic"
                        } else {
                            "media-playback-pause-symbolic"
                        });
                    }
                ));
                playback_controls.append(&play);
                let click = gtk::GestureClick::new();
                click.connect_released(glib::clone!(
                    #[weak]
                    play,
                    move |_, _, _, _| play.emit_clicked()
                ));
                preview.add_controller(click);
                let click = gtk::GestureClick::new();
                click.connect_released(glib::clone!(
                    #[weak]
                    play,
                    move |_, _, _, _| play.emit_clicked()
                ));
                picture.add_controller(click);
                self.play.replace(Some(play.clone()));
                if self.autoplay.get() && self.root.is_mapped() {
                    play.emit_clicked();
                }
                let mute = gtk::ToggleButton::builder()
                    .icon_name(if ui.playback.muted.get() {
                        "audio-volume-muted-symbolic"
                    } else {
                        "audio-volume-high-symbolic"
                    })
                    .tooltip_text("Mute audio")
                    .active(ui.playback.muted.get())
                    .build();
                mute.connect_toggled(glib::clone!(
                    #[weak]
                    ui,
                    #[weak]
                    playback_controls,
                    #[strong]
                    pointer_inside,
                    #[strong]
                    playback_activity,
                    move |b| {
                        show_playback_controls(
                            &playback_controls,
                            &pointer_inside,
                            &playback_activity,
                        );
                        ui.playback.mute(b.is_active());
                        b.set_icon_name(if b.is_active() {
                            "audio-volume-muted-symbolic"
                        } else {
                            "audio-volume-high-symbolic"
                        });
                    }
                ));
                playback_controls.append(&mute);
            }
            let like = like_button(&ui, post);
            let comments = comment_button(&ui, post);
            for button in [&like, &comments] {
                button.add_css_class("flat");
                button.set_halign(gtk::Align::Fill);
                button.set_hexpand(false);
                if let Some(content) = button.child().and_downcast::<gtk::Box>() {
                    content.set_orientation(gtk::Orientation::Vertical);
                    content.set_spacing(6);
                }
                actions.append(button);
            }
            details.set_valign(gtk::Align::End);
            details.add_css_class("reel-caption-overlay");
            surface.add_overlay(&details);
            surface.set_clip_overlay(&details, true);
            // The caption background belongs to details, so all video chrome
            // follows the same visibility timer and pointer interactions.
            playback_controls
                .bind_property("visible", &details, "visible")
                .sync_create()
                .build();
            status.set_halign(gtk::Align::Center);
            status.set_valign(gtk::Align::Center);
            status.add_css_class("osd");
            surface.add_overlay(&status);

            // The navigation widgets survive reel changes; detach them from
            // the retired rail before attaching them to the new one.
            for widget in [
                self.previous.clone().upcast::<gtk::Widget>(),
                self.position.clone().upcast(),
                self.next.clone().upcast(),
            ] {
                if let Some(parent) = widget.parent().and_downcast::<gtk::Box>() {
                    parent.remove(&widget);
                }
            }
            let navigation = gtk::Box::new(gtk::Orientation::Vertical, 8);
            navigation.add_css_class("reel-navigation");
            navigation.append(&self.previous);
            navigation.append(&self.position);
            navigation.append(&self.next);
            actions.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            actions.append(&navigation);
            let rail = gtk::Box::new(gtk::Orientation::Vertical, 0);
            rail.set_valign(gtk::Align::Fill);
            let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
            spacer.set_vexpand(true);
            rail.append(&spacer);
            rail.append(&actions);
            let layout = ReelLayout::new(&media_frame, &rail, ratio as f64);
            self.stage.append(&layout);
        }
        self.preload_next(&ui);
    }
    fn preload_next(&self, ui: &Rc<Ui>) {
        if !self.root.is_mapped() {
            return;
        }
        ui.playback.preload_next(self.next_video());
    }
    fn next_video(&self) -> Option<String> {
        self.store
            .item(self.index.get().saturating_add(1))
            .and_downcast::<glib::BoxedAnyObject>()
            .and_then(|object| {
                let item = object.borrow::<Item>();
                match &*item {
                    Item::Post(post) => post.media.first().and_then(|m| m.video.clone()),
                    _ => None,
                }
            })
    }
}

// Allocate the video and its rail as one centered group. Width can constrain
// the aspect ratio on small windows; tall windows never impose a height cap.
mod layout_imp {
    use gtk::{glib, prelude::*, subclass::prelude::*};
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    pub struct ReelLayout {
        pub children: RefCell<Vec<gtk::Widget>>,
        pub ratio: Cell<f64>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for ReelLayout {
        const NAME: &'static str = "ViewfinderReelLayout";
        type Type = super::ReelLayout;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for ReelLayout {
        fn dispose(&self) {
            for child in self.children.borrow_mut().drain(..) {
                child.unparent();
            }
        }
    }
    impl WidgetImpl for ReelLayout {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            (
                0,
                if orientation == gtk::Orientation::Horizontal {
                    600
                } else {
                    800
                },
                -1,
                -1,
            )
        }
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let children = self.children.borrow();
            if children.len() != 2 {
                return;
            }
            let rail_width = children[1]
                .measure(gtk::Orientation::Horizontal, -1)
                .1
                .max(52);
            let gap = 12;
            let video_width = ((height as f64 * self.ratio.get()).round() as i32)
                .min((width - rail_width - gap).max(1));
            let video_height = ((video_width as f64 / self.ratio.get()).round() as i32).min(height);
            let x = ((width - video_width - rail_width - gap) / 2).max(0);
            let y = (height - video_height) / 2;
            children[0].size_allocate(
                &gtk::Allocation::new(x, y, video_width, video_height),
                baseline,
            );
            children[1].size_allocate(
                &gtk::Allocation::new(x + video_width + gap, y, rail_width, video_height),
                baseline,
            );
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            for child in self.children.borrow().iter() {
                self.obj().snapshot_child(child, snapshot);
            }
        }
    }
}
glib::wrapper! {
    pub struct ReelLayout(ObjectSubclass<layout_imp::ReelLayout>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
impl ReelLayout {
    fn new(media: &impl IsA<gtk::Widget>, rail: &impl IsA<gtk::Widget>, ratio: f64) -> Self {
        use gtk::subclass::prelude::*;
        let layout: Self = glib::Object::new();
        layout.set_hexpand(true);
        layout.set_vexpand(true);
        layout.imp().ratio.set(ratio.max(0.1));
        for child in [media.clone().upcast::<gtk::Widget>(), rail.clone().upcast()] {
            child.set_parent(&layout);
            layout.imp().children.borrow_mut().push(child);
        }
        layout
    }
}
