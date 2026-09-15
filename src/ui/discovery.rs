//! Shared visual language for account discovery and activity.
use super::{Ui, collection::Collection, label};
use crate::{domain::*, media};
use adw::prelude::*;
use gtk::glib;
use std::rc::Rc;

pub(super) fn heading(collection: &Rc<Collection>, route: &Route) -> adw::Clamp {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.set_margin_top(20);
    body.set_margin_bottom(16);
    if let Route::Followers(user, following) = route {
        let title = label(if *following { "Following" } else { "Followers" });
        title.add_css_class("title-1");
        body.append(&title);
        if !user.username.is_empty() {
            let context = label(&format!("@{}", user.username));
            context.add_css_class("dim-label");
            body.append(&context);
        }
    }
    if let Route::Search(query) | Route::Keyword(query) = route {
        let title = label("Find something new");
        title.add_css_class("title-1");
        body.append(&title);
        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Search Viewfinder")
            .text(query)
            .hexpand(true)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label("Search Viewfinder")]);
        body.append(&entry);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let kind = adw::ToggleGroup::new();
        kind.set_hexpand(true);
        kind.set_homogeneous(true);
        kind.add(
            adw::Toggle::builder()
                .name("accounts")
                .label("Accounts")
                .build(),
        );
        kind.add(adw::Toggle::builder().name("posts").label("Posts").build());
        kind.set_active(u32::from(matches!(route, Route::Keyword(_))));
        kind.upcast_ref::<gtk::Widget>()
            .update_property(&[gtk::accessible::Property::Label("Result type")]);
        let submit = gtk::Button::with_label("Search");
        submit.add_css_class("suggested-action");
        controls.append(&kind);
        controls.append(&submit);
        body.append(&controls);
        entry.connect_changed(glib::clone!(
            #[weak]
            submit,
            move |entry| {
                submit.set_sensitive(!entry.text().trim().is_empty());
            }
        ));
        submit.connect_clicked(glib::clone!(
            #[weak]
            collection,
            #[weak]
            entry,
            #[weak]
            kind,
            move |_| {
                let query = entry.text().trim().to_owned();
                if !query.is_empty() {
                    collection.search(query, kind.active() == 1);
                }
            }
        ));
        entry.connect_activate(glib::clone!(
            #[weak]
            submit,
            move |_| {
                if submit.is_sensitive() {
                    submit.emit_clicked();
                }
            }
        ));
        let section = label(if matches!(route, Route::Search(_)) {
            "Accounts"
        } else {
            "Posts"
        });
        section.add_css_class("heading");
        body.append(&section);
    }
    adw::Clamp::builder().maximum_size(760).child(&body).build()
}

pub(super) fn result_row(ui: &Rc<Ui>, item: &Item) -> Option<gtk::Widget> {
    let (user, title, detail, navigable) = match item {
        Item::User(user) => (
            Some(user),
            if user.name.is_empty() {
                user.username.clone()
            } else {
                user.name.clone()
            },
            format!("@{}", user.username),
            true,
        ),
        Item::Notification(n) => (
            n.user.as_ref(),
            n.text.clone(),
            n.user
                .as_ref()
                .map(|u| format!("@{}", u.username))
                .unwrap_or_else(|| "Viewfinder".into()),
            n.user.is_some(),
        ),
        _ => return None,
    };
    // ListView owns keyboard focus and activation; never nest an actionable row.
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("card");
    if navigable {
        row.add_css_class("activatable");
    }
    row.add_css_class("discovery-row");
    if let Some(user) = user {
        let avatar = media::avatar(ui.images.clone(), user, 40);
        avatar.set_valign(gtk::Align::Center);
        row.append(&avatar);
    } else {
        let avatar = adw::Avatar::new(40, Some("Viewfinder"), false);
        avatar.set_icon_name(Some("preferences-system-notifications-symbolic"));
        avatar.set_valign(gtk::Align::Center);
        row.append(&avatar);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let primary = label(&title);
    primary.set_selectable(false);
    text.append(&primary);
    let secondary = label(&detail);
    secondary.set_selectable(false);
    secondary.add_css_class("dim-label");
    secondary.add_css_class("caption");
    text.append(&secondary);
    row.append(&text);
    if navigable {
        let next = gtk::Image::from_icon_name("go-next-symbolic");
        next.add_css_class("dim-label");
        row.append(&next);
    }
    Some(row.upcast())
}
