//! Message bubbles and their attachments: media, shared posts, links, voice,
//! stickers and GIFs, reply quotes and emoji reactions.
use super::{Collection, Ui, format, icon_button, label, media_frame::MediaFrame, viewer};
use crate::{domain::*, media};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::rc::Rc;

/// Quick reactions offered on the context menu; matches the web default set.
const QUICK_REACTIONS: [&str; 6] = ["❤️", "😂", "😮", "😢", "😡", "🙏"];

/// The bubble widget carrying the rounded background, for the grouping tick
/// callback in the list factory.
pub(super) fn bubble_of(row: &gtk::Widget) -> Option<gtk::Widget> {
    let mut child = row.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("message-bubble") {
            return Some(widget);
        }
        if let Some(inner) = widget.first_child()
            && inner.has_css_class("message-bubble")
        {
            return Some(inner);
        }
        child = widget.next_sibling();
    }
    None
}

fn participant<'a>(route: &'a Route, id: &str) -> Option<&'a User> {
    if let Route::Thread(t) = route {
        t.participants.iter().find(|u| u.id == id)
    } else {
        None
    }
}

/// True when the widget picked at (x, y) inside `bubble` owns the click, so a
/// double tap over a button does not also fire the heart reaction.
fn interactive_at(bubble: &gtk::Widget, x: f64, y: f64) -> bool {
    let Some(mut picked) = bubble.pick(x, y, gtk::PickFlags::DEFAULT) else {
        return false;
    };
    loop {
        if picked == *bubble {
            return false;
        }
        if picked.is::<gtk::Button>()
            || picked.is::<gtk::LinkButton>()
            || picked.is::<gtk::MenuButton>()
        {
            return true;
        }
        let Some(parent) = picked.parent() else {
            return false;
        };
        picked = parent;
    }
}

pub(super) fn row(
    ui: &Rc<Ui>,
    message: &Message,
    route: &Route,
    collection: &Rc<Collection>,
) -> gtk::Widget {
    let message = message.clone();
    let route = route.clone();
    let own = ui
        .client
        .borrow()
        .as_ref()
        .is_some_and(|c| c.account_id == message.sender);
    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 6);
    bubble.add_css_class("message-bubble");
    if own {
        bubble.add_css_class("outgoing");
    }
    let group_chat = matches!(&route, Route::Thread(t) if t.participants.len() > 1);
    if !own && group_chat {
        let sender = label(&sender_name(&message, &route, false));
        sender.add_css_class("caption-heading");
        bubble.append(&sender);
    }
    if let Some(quoted) = &message.reply_to {
        bubble.append(&quote(ui, quoted, &route, collection));
    }
    if !message.text.is_empty() {
        bubble.append(&label(&message.text));
    }
    // A message that is only media, a shared post or stickers renders as the
    // surface itself — no tinted bubble around it, matching Direct.
    let surface_only = message.text.is_empty()
        && message.reply_to.is_none()
        && !message.attachments.is_empty()
        && message.attachments.iter().all(|a| {
            matches!(
                a,
                Attachment::Animated { .. } | Attachment::Media(_) | Attachment::Post(_)
            )
        });
    if surface_only {
        bubble.add_css_class("transparent");
    }
    for attachment in &message.attachments {
        bubble.append(&attachment_widget(ui, attachment));
    }
    bubble.set_tooltip_text(format::message_timestamp(message.timestamp).as_deref());
    if !message.seen_by.is_empty() {
        let seen = label(&format!("Seen by {}", message.seen_by.join(", ")));
        seen.add_css_class("caption");
        bubble.append(&seen);
    }
    // The bubble keeps its rounded shape while the chip hangs off the lower
    // outer corner, like other Direct clients draw it.
    let holder = gtk::Overlay::new();
    holder.set_child(Some(&bubble));
    if let Some(chip) = reactions_chip(ui, &message, &route) {
        chip.set_halign(if own {
            gtk::Align::End
        } else {
            gtk::Align::Start
        });
        chip.set_valign(gtk::Align::End);
        chip.set_margin_bottom(-14);
        holder.add_overlay(&chip);
        bubble.add_css_class("has-reactions");
    }
    // Gestures live on the bubble: a double tap toggles a heart, secondary
    // click and long press open the message menu.
    let double = gtk::GestureClick::new();
    double.set_button(1);
    double.connect_pressed(glib::clone!(
        #[weak]
        bubble,
        #[weak]
        collection,
        #[strong]
        message,
        move |gesture, n_press, x, y| {
            if n_press == 2 && !interactive_at(bubble.upcast_ref(), x, y) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                collection.react_to(&message, "♥");
            }
        }
    ));
    bubble.add_controller(double);
    let menu = gtk::GestureClick::new();
    menu.set_button(3);
    menu.connect_pressed(glib::clone!(
        #[weak]
        ui,
        #[weak]
        bubble,
        #[weak]
        collection,
        #[strong]
        message,
        move |_, _, x, y| {
            context_popover(&ui, &message, &collection, bubble.upcast_ref(), x, y);
        }
    ));
    bubble.add_controller(menu);
    let press = gtk::GestureLongPress::new();
    press.connect_pressed(glib::clone!(
        #[weak]
        ui,
        #[weak]
        bubble,
        #[weak]
        collection,
        #[strong]
        message,
        move |_, x, y| {
            context_popover(&ui, &message, &collection, bubble.upcast_ref(), x, y);
        }
    ));
    bubble.add_controller(press);
    bubble.set_halign(if own {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });
    // Align bubbles within the same column as the composer. A second centered
    // clamp here pulls both sides of the conversation into the middle.
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_margin_start(20);
    row.set_margin_end(20);
    row.set_margin_top(1);
    row.set_margin_bottom(if message.reactions.is_empty() { 1 } else { 15 });
    let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    space.set_hexpand(true);
    space.set_size_request(40, -1);
    if own {
        row.append(&space);
        row.append(&holder);
    } else {
        row.append(&holder);
        row.append(&space);
    }
    row.upcast()
}

fn sender_name(message: &Message, route: &Route, own: bool) -> String {
    if own {
        "You".into()
    } else {
        participant(route, &message.sender)
            .map(|u| u.username.clone())
            .unwrap_or_else(|| "Participant".into())
    }
}

/// The quoted block at the top of a reply. Activating scrolls to the original
/// message when it is loaded.
fn quote(
    ui: &Rc<Ui>,
    quoted: &QuotedMessage,
    route: &Route,
    collection: &Rc<Collection>,
) -> gtk::Widget {
    let button = gtk::Button::new();
    button.add_css_class("message-quote");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if let Some(thumbnail) = &quoted.thumbnail {
        let preview = media::preview(ui.images.clone(), thumbnail, 160, true);
        preview.set_size_request(28, 28);
        preview.add_css_class("quote-thumbnail");
        preview.set_overflow(gtk::Overflow::Hidden);
        preview.set_valign(gtk::Align::Center);
        content.append(&preview);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let own_sender = ui
        .client
        .borrow()
        .as_ref()
        .is_some_and(|c| c.account_id == quoted.sender);
    let name = if own_sender {
        "You".into()
    } else {
        participant(route, &quoted.sender)
            .map(|u| u.username.clone())
            .unwrap_or_else(|| "Participant".into())
    };
    let author = label(&name);
    author.add_css_class("caption-heading");
    author.set_selectable(false);
    text.append(&author);
    let summary = label(&quoted.summary);
    summary.add_css_class("caption");
    summary.set_lines(1);
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    summary.set_selectable(false);
    text.append(&summary);
    content.append(&text);
    button.set_child(Some(&content));
    button.set_tooltip_text(Some("Go to the original message"));
    let id = quoted.id.clone();
    button.connect_clicked(glib::clone!(
        #[weak]
        collection,
        move |_| collection.scroll_to_message(&id)
    ));
    button.upcast()
}

fn attachment_widget(ui: &Rc<Ui>, attachment: &Attachment) -> gtk::Widget {
    match attachment {
        Attachment::Media(media) => message_media(ui, media),
        Attachment::Post(post) => post_card(ui, post),
        Attachment::Animated { media, alt } => animated(ui, media, alt),
        Attachment::Link {
            title,
            url,
            image_url,
        } if url.starts_with("https://") => link_card(ui, title, url, image_url.as_deref()),
        Attachment::Link { .. } => {
            let text = label("Link unavailable");
            text.add_css_class("dim-label");
            text.upcast()
        }
        Attachment::Voice {
            url,
            duration_ms,
            waveform,
        } => voice_card(ui, url, *duration_ms, waveform),
        Attachment::Unavailable => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&gtk::Image::from_icon_name("image-missing-symbolic"));
            let text = label("This attachment is unavailable");
            text.add_css_class("dim-label");
            row.append(&text);
            row.upcast()
        }
    }
}

/// Message media bounds, wider than feed media so screenshots and banners
/// keep their shape instead of letterboxing.
const RATIO_MIN: f64 = 0.4;
const RATIO_MAX: f64 = 2.5;

fn media_ratio(media: &Media) -> f64 {
    (media.width.max(1) as f64 / media.height.max(1) as f64).clamp(RATIO_MIN, RATIO_MAX)
}

/// Once the real image decodes, trust its ratio over the transport metadata so
/// the frame stops reserving space for the wrong shape.
fn track_ratio(frame: &MediaFrame, widget: &gtk::Widget) {
    let Some(picture) = widget
        .clone()
        .downcast::<gtk::Picture>()
        .ok()
        .or_else(|| widget.first_child().and_downcast::<gtk::Picture>())
    else {
        return;
    };
    picture.connect_paintable_notify(glib::clone!(
        #[weak]
        frame,
        move |picture| {
            let Some(paintable) = picture.paintable() else {
                return;
            };
            let (w, h) = (paintable.intrinsic_width(), paintable.intrinsic_height());
            if w > 0 && h > 0 {
                frame.set_ratio((w as f64 / h as f64).clamp(RATIO_MIN, RATIO_MAX));
            }
        }
    ));
}

/// Photo and video attachments keep their aspect ratio inside a bounded,
/// rounded surface instead of a fixed square.
fn message_media(ui: &Rc<Ui>, media: &Media) -> gtk::Widget {
    let ratio = media_ratio(media);
    if media.video.is_some() {
        let surface = super::inline_video::message(ui, media);
        surface.add_css_class("message-media");
        if let Ok(frame) = surface.clone().downcast::<MediaFrame>() {
            frame.set_max_width(300);
        }
        return surface;
    }
    let button = gtk::Button::new();
    button.add_css_class("message-media");
    button.add_css_class("flat");
    button.set_overflow(gtk::Overflow::Hidden);
    button.set_tooltip_text(Some("Open image"));
    let preview = media::preview(ui.images.clone(), media, 400, true);
    let frame = MediaFrame::new(&preview, ratio);
    frame.set_max_width(300);
    track_ratio(&frame, preview.upcast_ref());
    button.set_child(Some(&frame));
    let post = Post {
        id: String::new(),
        code: String::new(),
        author: User::default(),
        caption: String::new(),
        media: vec![media.clone()],
        liked: false,
        likes: 0,
        comments: 0,
        timestamp: 0,
    };
    button.connect_clicked(glib::clone!(
        #[weak]
        ui,
        #[strong]
        post,
        move |_| viewer::present(&ui, vec![post.clone()], 0, false)
    ));
    button.upcast()
}

/// Shared posts and stories render as a compact card: author header,
/// aspect-correct media, clipped caption and engagement counts.
fn post_card(ui: &Rc<Ui>, post: &Post) -> gtk::Widget {
    let post = post.clone();
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("post-share");
    card.add_css_class("card");
    card.set_overflow(gtk::Overflow::Hidden);
    let header = gtk::Button::new();
    header.add_css_class("flat");
    header.add_css_class("post-share-author");
    let identity = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    identity.append(&media::avatar(ui.images.clone(), &post.author, 24));
    let name = label(&post.author.username);
    name.add_css_class("heading");
    name.set_lines(1);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_selectable(false);
    identity.append(&name);
    header.set_child(Some(&identity));
    header.connect_clicked(glib::clone!(
        #[weak]
        ui,
        #[strong]
        post,
        move |_| ui.push(Route::Profile(post.author.clone()))
    ));
    card.append(&header);
    if let Some(item) = post.media.first() {
        let preview = media::preview(ui.images.clone(), item, 400, true);
        let frame = MediaFrame::new(&preview, media_ratio(item));
        frame.set_max_width(300);
        track_ratio(&frame, preview.upcast_ref());
        let open = gtk::Button::new();
        open.add_css_class("flat");
        open.add_css_class("post-share-media");
        open.set_tooltip_text(Some("Open post"));
        if item.video.is_some() {
            let surface = gtk::Overlay::new();
            surface.set_child(Some(&frame));
            let play = gtk::Image::from_icon_name("media-playback-start-symbolic");
            play.add_css_class("video-badge");
            play.set_halign(gtk::Align::Center);
            play.set_valign(gtk::Align::Center);
            play.set_pixel_size(22);
            surface.add_overlay(&play);
            open.set_child(Some(&surface));
        } else {
            open.set_child(Some(&frame));
        }
        open.connect_clicked(glib::clone!(
            #[weak]
            ui,
            #[strong]
            post,
            move |_| viewer::present(&ui, vec![post.clone()], 0, false)
        ));
        card.append(&open);
    }
    let footer = gtk::Box::new(gtk::Orientation::Vertical, 4);
    footer.add_css_class("post-share-footer");
    if !post.caption.is_empty() {
        let caption = label(&post.caption);
        caption.set_lines(2);
        caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
        footer.append(&caption);
    }
    let mut stats = Vec::new();
    if post.media.len() > 1 {
        stats.push(format!("{} media", post.media.len()));
    }
    if post.likes > 0 {
        stats.push(format!("♥ {}", format::compact_count(post.likes)));
    }
    if post.comments > 0 {
        stats.push(format!("{} comments", format::compact_count(post.comments)));
    }
    if let Some(date) = format::relative_time(post.timestamp) {
        stats.push(date.trim_end_matches(" ago").to_owned());
    }
    if !stats.is_empty() {
        let details = label(&stats.join(" · "));
        details.add_css_class("caption");
        details.add_css_class("dim-label");
        details.set_lines(1);
        details.set_ellipsize(gtk::pango::EllipsizeMode::End);
        footer.append(&details);
    }
    card.append(&footer);
    card.upcast()
}

/// Stickers and GIFs draw without a bubble. Animated renditions get a compact
/// play surface instead of the full video controls.
fn animated(ui: &Rc<Ui>, media: &Media, alt: &str) -> gtk::Widget {
    let surface = gtk::Overlay::new();
    surface.add_css_class("message-sticker");
    let preview = media::preview(ui.images.clone(), media, 400, false);
    let ratio = media_ratio(media);
    let preview_frame = MediaFrame::new(&preview, ratio);
    preview_frame.set_max_width(180);
    track_ratio(&preview_frame, preview.upcast_ref());
    surface.set_child(Some(&preview_frame));
    if let Some(video) = media.video.clone() {
        let picture = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Contain)
            .build();
        let picture_frame = MediaFrame::new(&picture, ratio);
        picture_frame.set_max_width(180);
        track_ratio(&picture_frame, picture.upcast_ref());
        let status = label("");
        status.add_css_class("osd");
        status.set_visible(false);
        status.connect_label_notify(|s| s.set_visible(!s.label().is_empty()));
        surface.add_overlay(&status);
        let play = icon_button("media-playback-start-symbolic", "Play");
        play.add_css_class("video-badge");
        play.set_halign(gtk::Align::Center);
        play.set_valign(gtk::Align::Center);
        let paused = std::rc::Rc::new(std::cell::Cell::new(true));
        play.connect_clicked(glib::clone!(
            #[weak]
            ui,
            #[weak]
            surface,
            #[strong]
            picture,
            #[strong]
            picture_frame,
            #[strong]
            status,
            #[strong]
            paused,
            #[strong]
            video,
            move |button| {
                if !ui.playback.owns(&picture) || ui.playback.failed.get() {
                    status.set_label("");
                    if let Err(error) = ui.playback.start(&video, &picture, &status) {
                        status.set_label(&error);
                        return;
                    }
                    surface.set_child(Some(&picture_frame));
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
            }
        ));
        surface.add_overlay(&play);
        picture.connect_paintable_notify(glib::clone!(
            #[weak]
            surface,
            #[strong]
            preview_frame,
            #[weak]
            play,
            move |picture| {
                if picture.paintable().is_none() {
                    surface.set_child(Some(&preview_frame));
                    play.set_icon_name("media-playback-start-symbolic");
                }
            }
        ));
    }
    if !alt.is_empty() {
        surface.set_tooltip_text(Some(alt));
        surface.update_property(&[gtk::accessible::Property::Label(alt)]);
    }
    surface.upcast()
}

fn link_card(ui: &Rc<Ui>, title: &str, url: &str, image_url: Option<&str>) -> gtk::Widget {
    let card = gtk::Button::new();
    card.add_css_class("link-card");
    card.add_css_class("flat");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    if let Some(image) = image_url {
        let preview = media::picture(ui.images.clone(), Some(image.to_owned()), 160);
        preview.set_size_request(44, 44);
        preview.add_css_class("link-card-image");
        preview.set_overflow(gtk::Overflow::Hidden);
        row.append(&preview);
    } else {
        let icon = gtk::Image::from_icon_name("web-browser-symbolic");
        icon.set_pixel_size(24);
        icon.set_valign(gtk::Align::Center);
        row.append(&icon);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let heading = label(if title.is_empty() { url } else { title });
    heading.add_css_class("heading");
    heading.set_lines(1);
    heading.set_ellipsize(gtk::pango::EllipsizeMode::End);
    heading.set_selectable(false);
    text.append(&heading);
    if !title.is_empty() {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| url.to_owned());
        let host = label(&host);
        host.add_css_class("caption");
        host.add_css_class("dim-label");
        host.set_lines(1);
        host.set_ellipsize(gtk::pango::EllipsizeMode::End);
        host.set_selectable(false);
        text.append(&host);
    }
    row.append(&text);
    card.set_child(Some(&row));
    let url = url.to_owned();
    card.connect_clicked(move |_| {
        let _ = gio::AppInfo::launch_default_for_uri(&url, gio::AppLaunchContext::NONE);
    });
    card.upcast()
}

fn voice_card(ui: &Rc<Ui>, url: &str, duration_ms: u64, waveform: &[f32]) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("voice-card");
    let play = icon_button("media-playback-start-symbolic", "Play voice message");
    play.add_css_class("circular");
    row.append(&play);
    let samples: Vec<f32> = if waveform.is_empty() {
        vec![0.3; 40]
    } else {
        waveform.to_vec()
    };
    let bars = gtk::DrawingArea::new();
    bars.set_content_height(28);
    bars.set_content_width(160.min(samples.len() as i32 * 4));
    bars.set_hexpand(true);
    bars.set_valign(gtk::Align::Center);
    bars.set_draw_func(move |bars, cr, width, height| {
        let color = bars.color();
        cr.set_source_rgba(
            color.red() as f64,
            color.green() as f64,
            color.blue() as f64,
            0.85,
        );
        let count = samples.len().max(1) as f64;
        let slot = width as f64 / count;
        let bar = (slot * 0.55).clamp(1.0, 4.0);
        let middle = height as f64 / 2.0;
        for (i, sample) in samples.iter().enumerate() {
            let h = (height as f64 * sample.clamp(0.05, 1.0) as f64).max(2.0);
            cr.rectangle(i as f64 * slot, middle - h / 2.0, bar, h);
        }
        cr.fill().expect("waveform fill");
    });
    row.append(&bars);
    let seconds = duration_ms / 1000;
    let duration = label(&format!("{}:{:02}", seconds / 60, seconds % 60));
    duration.add_css_class("caption");
    duration.set_selectable(false);
    duration.set_valign(gtk::Align::Center);
    row.append(&duration);
    // Voice plays through the shared single-pipeline controller; the paintable
    // sink needs a Picture even though nothing is shown.
    let picture = gtk::Picture::new();
    picture.set_visible(false);
    picture.set_size_request(1, 1);
    row.append(&picture);
    let status = label("");
    status.set_visible(false);
    status.connect_label_notify(|s| s.set_visible(!s.label().is_empty()));
    row.append(&status);
    let url = url.to_owned();
    let paused = std::rc::Rc::new(std::cell::Cell::new(true));
    play.connect_clicked(glib::clone!(
        #[weak]
        ui,
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
                "Play voice message"
            } else {
                "Pause voice message"
            }));
        }
    ));
    row.connect_unmap(glib::clone!(
        #[weak]
        ui,
        #[weak]
        play,
        move |_| {
            if ui.playback.owns(&picture) {
                ui.playback.stop();
            }
            play.set_icon_name("media-playback-start-symbolic");
        }
    ));
    row.upcast()
}

/// The emoji chip hanging off a reacted-to bubble. Opens the list of reactors.
fn reactions_chip(ui: &Rc<Ui>, message: &Message, route: &Route) -> Option<gtk::Button> {
    if message.reactions.is_empty() {
        return None;
    }
    let mut emojis: Vec<String> = Vec::new();
    for reaction in &message.reactions {
        if !emojis.contains(&reaction.emoji) {
            emojis.push(reaction.emoji.clone());
        }
    }
    let shown: String = emojis.iter().take(2).cloned().collect();
    let text = if message.reactions.len() > 1 {
        format!("{shown} {}", message.reactions.len())
    } else {
        shown
    };
    let chip = gtk::Button::with_label(&text);
    chip.add_css_class("message-reactions");
    let account = ui.client.borrow().as_ref().map(|c| c.account_id.clone());
    if message
        .reactions
        .iter()
        .any(|r| Some(&r.sender) == account.as_ref())
    {
        chip.add_css_class("own");
    }
    let name = |sender: &str| {
        if Some(sender) == account.as_deref() {
            "You".to_owned()
        } else {
            participant(route, sender)
                .map(|u| u.username.clone())
                .unwrap_or_else(|| "Participant".into())
        }
    };
    let names: Vec<String> = message
        .reactions
        .iter()
        .map(|r| format!("{}: {}", name(&r.sender), r.emoji))
        .collect();
    chip.set_tooltip_text(Some(&names.join("\n")));
    chip.connect_clicked(glib::clone!(
        #[weak]
        ui,
        #[strong]
        message,
        #[strong]
        route,
        move |chip| reactors_popover(&ui, &message, &route, chip.upcast_ref())
    ));
    Some(chip)
}

fn reactors_popover(ui: &Rc<Ui>, message: &Message, route: &Route, anchor: &gtk::Widget) {
    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    list.set_margin_top(8);
    list.set_margin_bottom(8);
    list.set_margin_start(8);
    list.set_margin_end(8);
    for reaction in &message.reactions {
        let own_sender = ui
            .client
            .borrow()
            .as_ref()
            .is_some_and(|c| c.account_id == reaction.sender);
        let user = participant(route, &reaction.sender)
            .cloned()
            .unwrap_or(User {
                id: reaction.sender.clone(),
                username: if own_sender {
                    "You".into()
                } else {
                    "Participant".into()
                },
                ..User::default()
            });
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&media::avatar(ui.images.clone(), &user, 28));
        let name = label(if own_sender { "You" } else { &user.username });
        name.set_selectable(false);
        name.set_hexpand(true);
        row.append(&name);
        row.append(&label(&reaction.emoji));
        list.append(&row);
    }
    popover.set_child(Some(&list));
    popover.set_parent(anchor);
    popover.connect_closed(|p| p.unparent());
    popover.popup();
}

/// Message menu: quick reactions, Reply and Copy.
fn context_popover(
    ui: &Rc<Ui>,
    message: &Message,
    collection: &Rc<Collection>,
    anchor: &gtk::Widget,
    x: f64,
    y: f64,
) {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(true);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(8);
    content.set_margin_end(8);
    let reactions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    reactions.add_css_class("quick-reactions");
    let account = ui.client.borrow().as_ref().map(|c| c.account_id.clone());
    for emoji in QUICK_REACTIONS {
        let button = gtk::Button::with_label(emoji);
        button.add_css_class("flat");
        button.add_css_class("reaction-option");
        if message
            .reactions
            .iter()
            .any(|r| Some(&r.sender) == account.as_ref() && r.emoji == emoji)
        {
            button.add_css_class("active");
        }
        button.set_tooltip_text(Some(&format!("React {emoji}")));
        button.connect_clicked(glib::clone!(
            #[weak]
            popover,
            #[weak]
            collection,
            #[strong]
            message,
            move |_| {
                popover.popdown();
                collection.react_to(&message, emoji);
            }
        ));
        reactions.append(&button);
    }
    content.append(&reactions);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let reply = gtk::Button::new();
    reply.add_css_class("flat");
    reply.add_css_class("message-menu-action");
    let reply_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    reply_row.append(&gtk::Image::from_icon_name("mail-reply-sender-symbolic"));
    let reply_label = label("Reply");
    reply_label.set_selectable(false);
    reply_row.append(&reply_label);
    reply.set_child(Some(&reply_row));
    reply.connect_clicked(glib::clone!(
        #[weak]
        popover,
        #[weak]
        collection,
        #[strong]
        message,
        move |_| {
            popover.popdown();
            collection.set_reply_to(Some(message.clone()));
        }
    ));
    content.append(&reply);
    if !message.text.is_empty() {
        let copy = gtk::Button::new();
        copy.add_css_class("flat");
        copy.add_css_class("message-menu-action");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&gtk::Image::from_icon_name("edit-copy-symbolic"));
        let title = label("Copy");
        title.set_selectable(false);
        row.append(&title);
        copy.set_child(Some(&row));
        let text = message.text.clone();
        copy.connect_clicked(glib::clone!(
            #[weak]
            popover,
            move |_| {
                popover.popdown();
                popover.clipboard().set_text(&text);
            }
        ));
        content.append(&copy);
    }
    popover.set_child(Some(&content));
    popover.set_parent(anchor);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|p| p.unparent());
    popover.popup();
}
