use super::{Ui, format};
use crate::{
    app::{self, Task},
    domain::LikeTarget,
};
use adw::prelude::*;
use gtk::glib;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) struct LikeState {
    liked: Cell<bool>,
    count: Cell<u64>,
    pending: Cell<bool>,
    task: RefCell<Option<Task>>,
    views: RefCell<Vec<View>>,
}

struct View {
    button: glib::WeakRef<gtk::Button>,
    icon: glib::WeakRef<gtk::Image>,
    count: glib::WeakRef<gtk::Label>,
    noun: &'static str,
}

impl LikeState {
    fn update(&self) {
        self.views.borrow_mut().retain(|view| {
            let Some(button) = view.button.upgrade() else {
                return false;
            };
            let text = format!(
                "{} {}",
                if self.liked.get() { "Unlike" } else { "Like" },
                view.noun
            );
            button.set_sensitive(!self.pending.get());
            button.set_tooltip_text(Some(&text));
            button.update_property(&[gtk::accessible::Property::Label(&text)]);
            if self.liked.get() {
                button.add_css_class("liked");
            } else {
                button.remove_css_class("liked");
            }
            if let Some(icon) = view.icon.upgrade() {
                icon.set_icon_name(Some(if self.liked.get() {
                    "viewfinder-heart-filled-symbolic"
                } else {
                    "viewfinder-heart-symbolic"
                }));
            }
            if let Some(count) = view.count.upgrade() {
                count.set_label(&format::compact_count(self.count.get()));
            }
            true
        });
    }
}

pub(super) fn button(ui: &Rc<Ui>, target: LikeTarget, liked: bool, count: u64) -> gtk::Button {
    let noun = match &target {
        LikeTarget::Post(_) => "post",
        LikeTarget::Comment(_) => "comment",
        LikeTarget::Story(_) => "story",
    };
    let state = ui
        .likes
        .borrow_mut()
        .entry(target.clone())
        .or_insert_with(|| {
            Rc::new(LikeState {
                liked: Cell::new(liked),
                count: Cell::new(count),
                pending: Cell::new(false),
                task: RefCell::new(None),
                views: RefCell::new(vec![]),
            })
        })
        .clone();
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("engagement-button");
    button.set_valign(gtk::Align::Center);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let icon = gtk::Image::new();
    let count = gtk::Label::new(None);
    count.set_visible(noun != "story");
    content.append(&icon);
    content.append(&count);
    button.set_child(Some(&content));
    state.views.borrow_mut().push(View {
        button: button.downgrade(),
        icon: icon.downgrade(),
        count: count.downgrade(),
        noun,
    });
    state.update();
    button.connect_clicked(glib::clone!(
        #[weak]
        ui,
        #[weak]
        state,
        move |_| {
            if state.pending.get() {
                return;
            }
            let Some(client) = ui.client.borrow().clone() else {
                return;
            };
            let liked = !state.liked.get();
            let target = target.clone();
            state.pending.set(true);
            state.update();
            *state.task.borrow_mut() = Some(app::background(
                async move { client.like(&target, liked).await },
                glib::clone!(
                    #[weak]
                    ui,
                    #[weak]
                    state,
                    move |result| {
                        state.pending.set(false);
                        match result {
                            Ok(liked) => {
                                state.count.set(if liked {
                                    state.count.get().saturating_add(1)
                                } else {
                                    state.count.get().saturating_sub(1)
                                });
                                state.liked.set(liked);
                            }
                            Err(error) => ui.notify(&error.to_string()),
                        }
                        state.update();
                        state.task.borrow_mut().take();
                    }
                ),
            ));
        }
    ));
    button
}

#[cfg(test)]
pub(super) fn review(ui: &Rc<Ui>) {
    let target = LikeTarget::Post("shared-fixture".into());
    let a = button(ui, target.clone(), false, 5);
    let b = button(ui, target.clone(), false, 5);
    let state = ui.likes.borrow()[&target].clone();
    state.pending.set(true);
    state.update();
    assert!(!a.is_sensitive() && !b.is_sensitive());
    state.pending.set(false);
    state.liked.set(true);
    state.count.set(6);
    state.update();
    assert!(a.has_css_class("liked") && b.has_css_class("liked"));
    let recycled = button(ui, target.clone(), false, 5);
    assert!(recycled.has_css_class("liked"));
    state.liked.set(false);
    state.update();
    assert!(!a.has_css_class("liked") && !recycled.has_css_class("liked"));
    drop(b);
    state.update();
    assert_eq!(state.views.borrow().len(), 2);
    ui.likes.borrow_mut().clear();
}
