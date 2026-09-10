use super::{Ui, icon_button, label};
use crate::{domain::Post, media};
use adw::prelude::*;
use gtk::{gdk, glib};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

struct Viewer {
    ui: Weak<Ui>,
    posts: Vec<Post>,
    post: Cell<usize>,
    slide: Cell<usize>,
    paused: Cell<bool>,
    playing: Cell<bool>,
    content: gtk::Box,
    position: gtk::Label,
    title: adw::WindowTitle,
    play: gtk::Button,
    status: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    mute: gtk::ToggleButton,
    copy: gtk::Button,
    picture: gtk::Picture,
}

pub fn present(ui: &Rc<Ui>, posts: Vec<Post>, position: usize, story: bool) {
    if posts.is_empty() || position >= posts.len() {
        return;
    }
    let dialog = adw::Dialog::builder()
        .title(if story { "Story" } else { "Post" })
        .content_width(800)
        .content_height(760)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new("", "");
    header.set_title_widget(Some(&title));
    toolbar.add_top_bar(&header);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let position_label = gtk::Label::new(None);
    let status = label("");
    let picture = gtk::Picture::builder()
        .vexpand(true)
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .build();
    picture.update_property(&[gtk::accessible::Property::Label("Video")]);
    let play = icon_button("media-playback-start-symbolic", "Play or pause");
    let previous = icon_button("go-previous-symbolic", "Previous media");
    let next = icon_button("go-next-symbolic", "Next media");
    let mute = gtk::ToggleButton::builder()
        .icon_name(if ui.playback.muted.get() {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        })
        .tooltip_text("Mute audio")
        .active(ui.playback.muted.get())
        .build();
    status.set_visible(false);
    status.connect_label_notify(|status| status.set_visible(!status.label().is_empty()));
    let copy = icon_button("edit-copy-symbolic", "Copy post link");
    let viewer = Rc::new(Viewer {
        ui: Rc::downgrade(ui),
        posts,
        post: Cell::new(position),
        slide: Cell::new(0),
        paused: Cell::new(false),
        playing: Cell::new(false),
        content: content.clone(),
        position: position_label.clone(),
        title,
        play: play.clone(),
        status: status.clone(),
        previous: previous.clone(),
        next: next.clone(),
        mute: mute.clone(),
        copy: copy.clone(),
        picture,
    });
    copy.connect_clicked(glib::clone!(
        #[weak]
        viewer,
        move |button| {
            let code = &viewer.posts[viewer.post.get()].code;
            if !code.is_empty()
                && code
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                button
                    .clipboard()
                    .set_text(&format!("https://www.instagram.com/p/{code}/"));
            }
        }
    ));
    header.pack_end(&copy);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    controls.set_halign(gtk::Align::Center);
    controls.set_margin_top(8);
    controls.set_margin_bottom(8);
    mute.update_property(&[gtk::accessible::Property::Label("Mute audio")]);
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
    for widget in [
        previous.clone().upcast::<gtk::Widget>(),
        position_label.upcast(),
        next.clone().upcast(),
        play.clone().upcast(),
        mute.upcast(),
    ] {
        controls.append(&widget);
    }
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom.append(&status);
    bottom.append(&controls);
    toolbar.set_content(Some(&content));
    toolbar.add_bottom_bar(&bottom);
    dialog.set_child(Some(&toolbar));
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
    play.connect_clicked(glib::clone!(
        #[weak]
        viewer,
        move |_| viewer.toggle()
    ));
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        viewer,
        #[weak]
        dialog,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            match key {
                gdk::Key::Left => viewer.step(false),
                gdk::Key::Right => viewer.step(true),
                gdk::Key::space => viewer.toggle(),
                gdk::Key::Escape => {
                    dialog.close();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    ));
    dialog.add_controller(keys);
    viewer.render();
    dialog.connect_closed(move |_| {
        if let Some(ui) = viewer.ui.upgrade() {
            ui.playback.stop();
        }
    });
    dialog.present(ui.window.upgrade().as_ref());
}

impl Viewer {
    fn render(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        ui.playback.stop();
        self.playing.set(false);
        self.paused.set(false);
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let post = &self.posts[self.post.get()];
        self.title.set_title(&post.author.username);
        let total: usize = self.posts.iter().map(|p| p.media.len().max(1)).sum();
        let before: usize = self.posts[..self.post.get()]
            .iter()
            .map(|p| p.media.len().max(1))
            .sum();
        self.position.set_label(&format!(
            "Media {} of {total}",
            before + self.slide.get() + 1
        ));
        self.copy.set_sensitive(
            !post.code.is_empty()
                && post
                    .code
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        );
        self.previous
            .set_sensitive(self.post.get() > 0 || self.slide.get() > 0);
        self.next.set_sensitive(
            self.post.get() + 1 < self.posts.len() || self.slide.get() + 1 < post.media.len(),
        );
        let multiple = self.posts.len() > 1 || post.media.len() > 1;
        self.previous.set_visible(multiple);
        self.next.set_visible(multiple);
        self.position.set_visible(multiple);
        self.mute.set_visible(
            post.media
                .get(self.slide.get())
                .is_some_and(|m| m.video.is_some()),
        );
        self.status.set_label("");
        self.play.set_icon_name("media-playback-start-symbolic");
        self.play.set_visible(false);
        if let Some(item) = post.media.get(self.slide.get()) {
            let image = media::preview(ui.images.clone(), item, 1600, false);
            image.set_vexpand(true);
            self.content.append(&image);
            self.play.set_visible(item.video.is_some());
        } else {
            self.content.append(
                &adw::StatusPage::builder()
                    .title("Media unavailable")
                    .icon_name("image-missing-symbolic")
                    .build(),
            );
        }
        if !post.caption.is_empty() {
            let caption = label(&post.caption);

            caption.set_margin_start(12);
            caption.set_margin_end(12);
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .max_content_height(120)
                .propagate_natural_height(true)
                .child(&caption)
                .build();
            self.content.append(&scroll);
        }
        if post
            .media
            .get(self.slide.get())
            .is_some_and(|media| media.video.is_some())
        {
            self.toggle();
        }
    }
    fn step(&self, forward: bool) {
        let count = self.posts[self.post.get()].media.len();
        if forward {
            if self.slide.get() + 1 < count {
                self.slide.set(self.slide.get() + 1);
            } else if self.post.get() + 1 < self.posts.len() {
                self.post.set(self.post.get() + 1);
                self.slide.set(0);
            } else {
                return;
            }
        } else if self.slide.get() > 0 {
            self.slide.set(self.slide.get() - 1);
        } else if self.post.get() > 0 {
            self.post.set(self.post.get() - 1);
            self.slide
                .set(self.posts[self.post.get()].media.len().saturating_sub(1));
        } else {
            return;
        }
        self.render();
    }
    fn toggle(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some(url) = self.posts[self.post.get()]
            .media
            .get(self.slide.get())
            .and_then(|m| m.video.as_ref())
        else {
            return;
        };
        if !self.playing.get() || ui.playback.failed.get() {
            if let Some(child) = self.content.first_child() {
                self.content.remove(&child);
            }
            self.content.prepend(&self.picture);
            match ui.playback.start(url, &self.picture, &self.status) {
                Ok(()) => {
                    self.playing.set(true);
                    self.paused.set(false);
                }
                Err(message) => {
                    self.status.set_label(&message);
                    return;
                }
            }
        } else {
            self.paused.set(!self.paused.get());
            ui.playback.pause(self.paused.get());
        }
        self.play.set_icon_name(if self.paused.get() {
            "media-playback-start-symbolic"
        } else {
            "media-playback-pause-symbolic"
        });
    }
}
