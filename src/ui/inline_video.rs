use super::{Ui, icon_button, media_frame::MediaFrame};
use crate::{domain::Media, media};
use gtk::{glib, prelude::*};
use std::{cell::Cell, rc::Rc};

/// Feed videos share the application's single decoder with the viewer and reels.
pub(super) fn new(ui: &Rc<Ui>, media: &Media) -> gtk::Widget {
    let surface = gtk::Overlay::new();
    surface.add_css_class("reel-surface");
    surface.set_overflow(gtk::Overflow::Hidden);
    let preview = media::preview(ui.images.clone(), media, 1080, false);
    surface.set_child(Some(&preview));
    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .build();
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_halign(gtk::Align::Center);
    status.set_valign(gtk::Align::Center);
    status.add_css_class("osd");
    status.set_visible(false);
    status.connect_label_notify(|s| s.set_visible(!s.label().is_empty()));
    surface.add_overlay(&status);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    controls.set_halign(gtk::Align::End);
    controls.set_valign(gtk::Align::End);
    controls.set_margin_end(12);
    controls.set_margin_bottom(12);
    controls.add_css_class("osd");
    controls.add_css_class("toolbar");
    let play = icon_button("media-playback-start-symbolic", "Play video");
    let paused = Rc::new(Cell::new(true));
    let url = media.video.clone().unwrap_or_default();
    play.connect_clicked(glib::clone!(
        #[weak]
        ui,
        #[weak]
        surface,
        #[strong]
        picture,
        #[strong]
        status,
        #[strong]
        paused,
        move |button| {
            if !ui.playback.owns(&picture) || ui.playback.failed.get() {
                status.set_label("");
                if let Err(error) = ui.playback.start(&url, &picture, &status) {
                    status.set_label(&error);
                    return;
                }
                surface.set_child(Some(&picture));
                paused.set(false);
            } else {
                paused.set(!paused.get());
                ui.playback.pause(paused.get());
            }
            button.set_icon_name(if paused.get() {
                "media-playback-start-symbolic"
            } else {
                "media-playback-pause-symbolic"
            });
            button.set_tooltip_text(Some(if paused.get() {
                "Play video"
            } else {
                "Pause video"
            }));
        }
    ));
    controls.append(&play);
    let mute = icon_button("audio-volume-muted-symbolic", "Toggle audio");
    mute.connect_clicked(glib::clone!(
        #[weak]
        ui,
        move |button| {
            ui.playback.mute(!ui.playback.muted.get());
            button.set_icon_name(if ui.playback.muted.get() {
                "audio-volume-muted-symbolic"
            } else {
                "audio-volume-high-symbolic"
            });
        }
    ));
    controls.append(&mute);
    surface.add_overlay(&controls);
    let click = gtk::GestureClick::new();
    click.connect_released(glib::clone!(
        #[weak]
        play,
        move |_, _, _, _| play.emit_clicked()
    ));
    // Attach only to the media child so the audio button doesn't also toggle playback.
    preview.add_controller(click);
    let click = gtk::GestureClick::new();
    click.connect_released(glib::clone!(
        #[weak]
        play,
        move |_, _, _, _| play.emit_clicked()
    ));
    picture.add_controller(click);
    surface.connect_unmap(glib::clone!(
        #[weak]
        ui,
        #[strong]
        picture,
        #[weak]
        play,
        move |_| {
            if ui.playback.owns(&picture) {
                ui.playback.stop();
            }
            play.set_icon_name("media-playback-start-symbolic");
            play.set_tooltip_text(Some("Play video"));
        }
    ));
    picture.connect_paintable_notify(glib::clone!(
        #[weak]
        surface,
        #[strong]
        preview,
        #[weak]
        play,
        move |picture| {
            if picture.paintable().is_none() {
                surface.set_child(Some(&preview));
                play.set_icon_name("media-playback-start-symbolic");
                play.set_tooltip_text(Some("Play video"));
            }
        }
    ));
    MediaFrame::new(
        &surface,
        (media.width.max(1) as f64 / media.height.max(1) as f64).clamp(0.5, 2.0),
    )
    .upcast()
}
