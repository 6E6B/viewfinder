use super::{Ui, format, icon_button, label, notifications_button, viewer};
use crate::{
    app::{self, Pagination, Task},
    domain::*,
    media,
};
use adw::prelude::*;
use gtk::{gio, glib};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

pub struct Collection {
    pub root: gtk::Box,
    pub page: RefCell<Option<adw::NavigationPage>>,
    route: RefCell<Route>,
    own_profile: bool,
    ui: Weak<Ui>,
    store: gio::ListStore,
    list: RefCell<Option<gtk::ListView>>,
    pagination: RefCell<Pagination>,
    task: RefCell<Option<Task>>,
    profile_task: RefCell<Option<Task>>,
    action_task: RefCell<Option<Task>>,
    read_task: RefCell<Option<Task>>,
    read_watermark: RefCell<String>,
    status: adw::StatusPage,
    retry: gtk::Button,
    replacing: Cell<bool>,
    stack: gtk::Stack,
    more: adw::Spinner,
    scrolled: gtk::ScrolledWindow,
    auto_failed: Cell<bool>,
    stick_bottom: Cell<bool>,
    banner: adw::Banner,
    header: gtk::Box,
    split: RefCell<Option<adw::NavigationSplitView>>,
    thread: RefCell<Option<Rc<Collection>>>,
    reels: RefCell<Option<Rc<super::reels::Reels>>>,
    pub(super) stories: Option<Rc<super::stories::Tray>>,
    // Popover hosts set this so activating a row closes the surface.
    pub(super) dismiss: RefCell<Option<Box<dyn Fn()>>>,
    // Thread composer state for replies, reactions and the sticker tray.
    reply_to: RefCell<Option<Message>>,
    reply_bar: RefCell<Option<gtk::Box>>,
    composer_entry: RefCell<Option<gtk::Entry>>,
    message_task: RefCell<Option<Task>>,
    sticker_task: RefCell<Option<Task>>,
    sticker_packs: RefCell<Option<Rc<Vec<StickerPack>>>>,
}
impl Collection {
    pub fn new(ui: &Rc<Ui>, route: Route) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let banner = adw::Banner::new("");
        root.append(&banner);
        let loading = gtk::Box::new(gtk::Orientation::Vertical, 0);
        loading.set_halign(gtk::Align::Center);
        loading.set_valign(gtk::Align::Center);
        loading.set_accessible_role(gtk::AccessibleRole::Status);
        let loading_spinner = adw::Spinner::new();
        loading_spinner.set_size_request(24, 24);
        loading_spinner.set_tooltip_text(Some("Loading"));
        loading.append(&loading_spinner);
        let status = adw::StatusPage::new();
        let stack = gtk::Stack::builder().vexpand(true).build();
        stack.add_named(&loading, Some("loading"));
        stack.add_named(&status, Some("status"));
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection: gtk::SelectionModel = if matches!(route, Route::Inbox) {
            gtk::SingleSelection::builder()
                .model(&store)
                .autoselect(false)
                .can_unselect(true)
                .build()
                .upcast()
        } else {
            gtk::NoSelection::new(Some(store.clone())).upcast()
        };
        let factory = gtk::SignalListItemFactory::new();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        if matches!(route, Route::Notifications) {
            // In a popover the list hugs its rows instead of filling a page.
            scrolled.set_propagate_natural_height(true);
            scrolled.set_max_content_height(420);
        }
        let more = adw::Spinner::new();
        more.set_size_request(18, 18);
        more.set_tooltip_text(Some("Loading more posts"));
        more.set_halign(gtk::Align::Center);
        more.set_margin_top(8);
        more.set_margin_bottom(8);
        more.set_visible(false);
        let retry = gtk::Button::with_label("Try Again");
        retry.set_visible(false);
        let c = Rc::new(Self {
            root: root.clone(),
            page: RefCell::new(None),
            own_profile: matches!(&route,Route::Profile(u) if u.id.is_empty()),
            route: RefCell::new(route.clone()),
            ui: Rc::downgrade(ui),
            store: store.clone(),
            list: RefCell::new(None),
            pagination: RefCell::new(Pagination::default()),
            task: RefCell::new(None),
            profile_task: RefCell::new(None),
            action_task: RefCell::new(None),
            read_task: RefCell::new(None),
            read_watermark: RefCell::new(String::new()),
            status: status.clone(),
            retry: retry.clone(),
            replacing: Cell::new(false),
            stack: stack.clone(),
            more: more.clone(),
            scrolled: scrolled.clone(),
            auto_failed: Cell::new(false),
            stick_bottom: Cell::new(false),
            banner,
            header: header.clone(),
            split: RefCell::new(None),
            thread: RefCell::new(None),
            reels: RefCell::new(None),
            stories: matches!(route, Route::Home).then(|| super::stories::Tray::new(ui)),
            dismiss: RefCell::new(None),
            reply_to: RefCell::new(None),
            reply_bar: RefCell::new(None),
            composer_entry: RefCell::new(None),
            message_task: RefCell::new(None),
            sticker_task: RefCell::new(None),
            sticker_packs: RefCell::new(None),
        });
        factory.connect_bind(glib::clone!(
            #[weak]
            c,
            move |_, object| {
                let Some(li) = object.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let Some(object) = li.item().and_downcast::<glib::BoxedAnyObject>() else {
                    return;
                };
                let item = object.borrow::<Item>().clone();
                if matches!(item, Item::FeedHeader) {
                    li.set_activatable(false);
                    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                    heading.set_margin_top(12);
                    heading.set_margin_bottom(12);
                    heading.set_margin_start(16);
                    heading.set_margin_end(16);
                    let title = label("Your feed");
                    title.add_css_class("title-2");
                    title.set_selectable(false);
                    title.set_hexpand(true);
                    heading.append(&title);
                    column.append(&heading);
                    if let Some(stories) = &c.stories {
                        if stories.root.parent().is_some() {
                            stories.root.unparent();
                        }
                        column.append(&stories.root);
                    }
                    li.set_child(Some(&column));
                    return;
                }
                li.set_activatable(!matches!(
                    item,
                    Item::Message(_) | Item::Notification(Notification { user: None, .. })
                ));
                if let Some(ui) = c.ui.upgrade() {
                    let mut child = render_item(&ui, &item, &c.route.borrow(), &c);
                    if let Item::Message(message) = &item {
                        let sender = message.sender.clone();
                        let timestamp = message.timestamp;
                        let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
                        let divider = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                        divider.add_css_class("message-divider");
                        for is_label in [false, true, false] {
                            if is_label {
                                let time = gtk::Label::new(format::message_timestamp(timestamp).as_deref());
                                divider.append(&time);
                            } else {
                                let line = gtk::Separator::new(gtk::Orientation::Horizontal);
                                line.set_hexpand(true);
                                line.set_valign(gtk::Align::Center);
                                divider.append(&line);
                            }
                        }
                        group.append(&divider);
                        group.append(&child);
                        child.add_tick_callback(glib::clone!(
                            #[weak] c,
                            #[weak] li,
                            #[weak] divider,
                            #[upgrade_or] glib::ControlFlow::Break,
                            move |row, _| {
                                let neighbor = |position: Option<u32>| {
                                    position
                                        .and_then(|p| c.store.item(p))
                                        .and_downcast::<glib::BoxedAnyObject>()
                                };
                                let joins = |o: &glib::BoxedAnyObject| {
                                    matches!(&*o.borrow::<Item>(), Item::Message(m) if m.sender == sender && format::same_message_group(m.timestamp.min(timestamp), m.timestamp.max(timestamp)))
                                };
                                let previous = neighbor(li.position().checked_sub(1));
                                let same_group = previous.as_ref().is_some_and(|o| matches!(&*o.borrow::<Item>(), Item::Message(m) if format::same_message_group(m.timestamp, timestamp)));
                                let joins_previous = previous.as_ref().is_some_and(&joins);
                                let joins_next = neighbor(li.position().checked_add(1))
                                    .as_ref()
                                    .is_some_and(joins);
                                divider.set_visible(timestamp > 0 && !same_group);
                                row.set_margin_top(if same_group && !joins_previous { 8 } else { 1 });
                                if let Some(bubble) = super::messages::bubble_of(row) {
                                    for (joined, class) in [
                                        (joins_previous, "join-previous"),
                                        (joins_next, "join-next"),
                                    ] {
                                        if joined { bubble.add_css_class(class); }
                                        else { bubble.remove_css_class(class); }
                                    }
                                }
                                glib::ControlFlow::Continue
                            }
                        ));
                        child = group.upcast();
                    }
                    if matches!(item, Item::Conversation(_)) {
                        let click = gtk::GestureClick::new();
                        click.set_button(1);
                        click.connect_released(glib::clone!(
                            #[weak]
                            c,
                            #[weak]
                            li,
                            move |_, _, _, _| c.activate(li.position())
                        ));
                        child.add_controller(click);
                    }
                    li.set_child(Some(&child));
                }
            }
        ));
        factory.connect_unbind(|_, object| {
            if let Some(li) = object.downcast_ref::<gtk::ListItem>() {
                li.set_child(None::<&gtk::Widget>);
            }
        });
        let content: gtk::Widget = if matches!(route, Route::Reels) {
            let reels = super::reels::Reels::new(
                ui,
                &store,
                glib::clone!(
                    #[weak]
                    c,
                    move || c.load()
                ),
            );
            let widget = reels.root.clone().upcast();
            c.reels.replace(Some(reels));
            widget
        } else if route.grid() {
            let grid = gtk::GridView::builder()
                .model(&selection)
                .factory(&factory)
                .min_columns(2)
                .max_columns(4)
                .single_click_activate(true)
                .build();
            grid.connect_activate(glib::clone!(
                #[weak]
                c,
                move |_, position| c.activate(position)
            ));
            if matches!(route, Route::Profile(_) | Route::Keyword(_)) {
                grid.set_margin_start(12);
                grid.set_margin_end(12);
                grid.set_margin_bottom(12);
                grid.add_css_class("discovery-grid");
            }
            grid.upcast()
        } else {
            let list = gtk::ListView::builder()
                .model(&selection)
                .factory(&factory)
                .single_click_activate(!matches!(route, Route::Inbox))
                .build();

            list.connect_activate(glib::clone!(
                #[weak]
                c,
                move |_, position| c.activate(position)
            ));
            if matches!(route, Route::Home | Route::Thread(_)) {
                list.add_css_class("social-feed");
            }
            if matches!(route, Route::Inbox) {
                list.add_css_class("navigation-sidebar");
                list.add_css_class("conversation-list");
            }
            if matches!(route, Route::Thread(_)) {
                list.add_css_class("message-list");
            }
            if matches!(
                route,
                Route::Notifications | Route::Search(_) | Route::Comments(_) | Route::Followers(..)
            ) {
                list.add_css_class("discovery-list");
                list.set_margin_start(12);
                list.set_margin_end(12);
                list.set_margin_bottom(24);
            }
            c.list.replace(Some(list.clone()));
            list.upcast()
        };
        if matches!(route, Route::Reels) {
            stack.add_named(&content, Some("content"));
        } else {
            if matches!(route, Route::Inbox) {
                // A navigation sidebar should fill its pane. ClampScrollable is useful
                // for feeds, but makes an inbox look like centered page content.
                scrolled.set_child(Some(&content));
            } else {
                let clamp = adw::ClampScrollable::builder()
                    .maximum_size(if matches!(route, Route::Home) {
                        600
                    } else {
                        760
                    })
                    .child(&content)
                    .build();
                scrolled.set_child(Some(&clamp));
            }
            stack.add_named(&scrolled, Some("content"));
        }
        if matches!(route, Route::Profile(_)) {
            root.append(
                &adw::Clamp::builder()
                    .maximum_size(760)
                    .child(&header)
                    .build(),
            );
        }
        if let Route::Comments(post) = &route {
            let summary = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            summary.set_margin_start(16);
            summary.set_margin_end(16);
            summary.set_margin_top(16);
            summary.set_margin_bottom(16);
            summary.append(&media::avatar(ui.images.clone(), &post.author, 40));
            let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
            text.set_hexpand(true);
            let title = label(&format!("Comments on @{}’s post", post.author.username));
            title.add_css_class("heading");
            text.append(&title);
            if !post.caption.is_empty() {
                let caption = label(&post.caption);
                caption.set_lines(2);
                caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
                caption.add_css_class("dim-label");
                text.append(&caption);
            }
            summary.append(&text);
            summary.append(&like_button(ui, post));
            root.append(
                &adw::Clamp::builder()
                    .maximum_size(760)
                    .child(&summary)
                    .build(),
            );
        }
        if matches!(
            route,
            Route::Search(_) | Route::Keyword(_) | Route::Followers(..)
        ) {
            root.append(&super::discovery::heading(&c, &route));
        }
        if matches!(route, Route::Inbox) {
            let bar = adw::HeaderBar::new();
            bar.set_title_widget(Some(&adw::WindowTitle::new("Messages", "")));
            bar.pack_start(&ui.sidebar_toggle());
            bar.pack_end(&notifications_button(ui));
            let sidebar_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            sidebar_content.append(&stack);
            sidebar_content.append(&more);
            let sidebar = adw::ToolbarView::new();
            sidebar.add_top_bar(&bar);
            sidebar.set_content(Some(&sidebar_content));
            let empty = adw::StatusPage::builder()
                .icon_name("mail-unread-symbolic")
                .title("Your conversations")
                .description("Choose a conversation to see your messages and shared posts.")
                .build();
            let empty_toolbar = adw::ToolbarView::new();
            empty_toolbar.add_top_bar(&adw::HeaderBar::new());
            empty_toolbar.set_content(Some(&empty));
            let split = adw::NavigationSplitView::builder()
                .sidebar(&adw::NavigationPage::new(&sidebar, "Messages"))
                .content(&adw::NavigationPage::new(&empty_toolbar, "Conversation"))
                .min_sidebar_width(280.0)
                .max_sidebar_width(320.0)
                .sidebar_width_fraction(0.28)
                .build();
            let bin = adw::BreakpointBin::new();
            bin.set_size_request(320, 300);
            bin.set_vexpand(true);
            bin.set_child(Some(&split));
            let breakpoint =
                adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 700sp").unwrap());
            breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
            bin.add_breakpoint(breakpoint);
            root.append(&bin);
            c.split.replace(Some(split));
        } else {
            root.append(&stack);
            if !matches!(route, Route::Reels) {
                root.append(&more);
            }
        }
        if matches!(route, Route::Thread(_) | Route::Comments(_)) {
            c.composer();
        }
        retry.add_css_class("suggested-action");
        retry.set_halign(gtk::Align::Center);
        retry.connect_clicked(glib::clone!(
            #[weak]
            c,
            move |_| c.load()
        ));
        status.set_child(Some(&retry));

        c.banner.connect_button_clicked(glib::clone!(
            #[weak]
            c,
            move |_| {
                c.auto_failed.set(false);
                c.load();
            }
        ));
        let adjustment = scrolled.vadjustment();
        adjustment.connect_value_changed(glib::clone!(
            #[weak]
            c,
            move |_| {
                c.load_near_end();
                c.mark_visible_read();
            }
        ));
        adjustment.connect_changed(glib::clone!(
            #[weak]
            c,
            move |_| {
                c.load_near_end();
                c.mark_visible_read();
                c.scroll_to_bottom();
            }
        ));
        root.connect_map(glib::clone!(
            #[weak]
            c,
            move |_| {
                if !c.pagination.borrow().started {
                    c.load();
                } else {
                    c.load_near_end();
                }
            }
        ));
        root.connect_unmap(glib::clone!(
            #[weak]
            c,
            move |_| {
                c.task.borrow_mut().take();
                c.profile_task.borrow_mut().take();
                let mut p = c.pagination.borrow_mut();
                p.loading = false;
                c.more.set_visible(false);
                p.generation += 1;
                if let Some(ui) = c.ui.upgrade() {
                    ui.playback.stop_owned(&c.root);
                }
            }
        ));
        if matches!(route, Route::Thread(_) | Route::Inbox) {
            glib::timeout_add_seconds_local(
                20,
                glib::clone!(
                    #[weak]
                    c,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        if c.is_active_chat()
                            && !c.pagination.borrow().loading
                            && !c.auto_failed.get()
                        {
                            c.refresh();
                        }
                        glib::ControlFlow::Continue
                    }
                ),
            );
        }
        c
    }
    fn is_active_chat(&self) -> bool {
        self.root.is_mapped()
            && self
                .ui
                .upgrade()
                .and_then(|ui| ui.window.upgrade())
                .is_some_and(|w| w.is_active())
    }
    fn mark_visible_read(self: &Rc<Self>) {
        if !self.is_active_chat() || self.read_task.borrow().is_some() {
            return;
        }
        let adjustment = self.scrolled.vadjustment();
        if adjustment.page_size() <= 0.0
            || adjustment.upper() - adjustment.value() - adjustment.page_size() > 40.0
        {
            return;
        }
        let Route::Thread(thread) = self.route.borrow().clone() else {
            return;
        };
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some(client) = ui.client.borrow().clone() else {
            return;
        };
        let Some(object) = self
            .store
            .n_items()
            .checked_sub(1)
            .and_then(|p| self.store.item(p))
            .and_downcast::<glib::BoxedAnyObject>()
        else {
            return;
        };
        let Item::Message(message) = object.borrow::<Item>().clone() else {
            return;
        };
        if *self.read_watermark.borrow() == message.id {
            return;
        }
        // One attempt per visible item, including failures, to avoid receipt storms.
        self.read_watermark.replace(message.id.clone());
        *self.read_task.borrow_mut() = Some(app::background(
            async move { client.mark_thread_read(&thread, &message).await },
            glib::clone!(
                #[weak(rename_to=c)]
                self,
                move |result| {
                    c.read_task.borrow_mut().take();
                    if let Err(error) = result {
                        tracing::warn!(kind=?error, "Read receipt failed");
                    }
                }
            ),
        ));
    }
    fn scroll_to_bottom(self: &Rc<Self>) {
        let adjustment = self.scrolled.vadjustment();
        if !self.stick_bottom.get() || adjustment.page_size() <= 0.0 || adjustment.upper() <= 0.0 {
            return;
        }
        self.stick_bottom.set(false);
        if self.store.n_items() > 0
            && let Some(list) = self.list.borrow().as_ref()
        {
            list.scroll_to(self.store.n_items() - 1, gtk::ListScrollFlags::NONE, None);
        }
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to=c)]
            self,
            move || c.scrolled.set_opacity(1.0)
        ));
    }
    /// Adopt a freshly loaded history after a send, sticker or reaction so the
    /// store reflects the server rather than trusting the mutation response.
    fn adopt_page(self: &Rc<Self>, page: Page) {
        self.task.borrow_mut().take();
        self.pagination.borrow_mut().reset();
        self.replacing.set(true);
        let generation = self.pagination.borrow().generation;
        self.finish(generation, Ok(page));
    }
    /// Choose or clear the reply target shown above the composer.
    pub(super) fn set_reply_to(self: &Rc<Self>, message: Option<Message>) {
        *self.reply_to.borrow_mut() = message.clone();
        let Some(bar) = self.reply_bar.borrow().clone() else {
            return;
        };
        while let Some(child) = bar.first_child() {
            bar.remove(&child);
        }
        let Some(target) = message else {
            bar.set_visible(false);
            return;
        };
        let icon = gtk::Image::from_icon_name("mail-reply-sender-symbolic");
        icon.set_valign(gtk::Align::Center);
        icon.add_css_class("dim-label");
        bar.append(&icon);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let ui = self.ui.upgrade();
        let own = ui
            .as_ref()
            .and_then(|ui| ui.client.borrow().clone())
            .is_some_and(|c| c.account_id == target.sender);
        let name = if own {
            "yourself".to_owned()
        } else if let Route::Thread(thread) = &*self.route.borrow() {
            thread
                .participants
                .iter()
                .find(|u| u.id == target.sender)
                .map(|u| format!("@{}", u.username))
                .unwrap_or_else(|| "this message".into())
        } else {
            "this message".into()
        };
        let heading = label(&format!("Replying to {name}"));
        heading.add_css_class("caption-heading");
        heading.set_selectable(false);
        text.append(&heading);
        let snippet = if target.text.is_empty() {
            message_snippet(&target.attachments).to_owned()
        } else {
            target.text.clone()
        };
        let snippet = label(&snippet);
        snippet.add_css_class("caption");
        snippet.add_css_class("dim-label");
        snippet.set_lines(1);
        snippet.set_ellipsize(gtk::pango::EllipsizeMode::End);
        snippet.set_selectable(false);
        text.append(&snippet);
        bar.append(&text);
        let cancel = icon_button("window-close-symbolic", "Cancel reply");
        cancel.add_css_class("flat");
        cancel.set_valign(gtk::Align::Center);
        cancel.connect_clicked(glib::clone!(
            #[weak(rename_to=c)]
            self,
            move |_| c.set_reply_to(None)
        ));
        bar.append(&cancel);
        bar.set_visible(true);
        if let Some(entry) = self.composer_entry.borrow().as_ref() {
            entry.grab_focus();
        }
    }
    /// Toggle an emoji reaction on a message and reconcile history after.
    pub(super) fn react_to(self: &Rc<Self>, message: &Message, emoji: &str) {
        let Route::Thread(thread) = self.route.borrow().clone() else {
            return;
        };
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some(client) = ui.client.borrow().clone() else {
            return;
        };
        let message = message.clone();
        let emoji = emoji.to_owned();
        *self.message_task.borrow_mut() = Some(app::background(
            async move { client.react_message(&thread, &message, &emoji).await },
            glib::clone!(
                #[weak(rename_to=c)]
                self,
                #[weak]
                ui,
                move |result| {
                    c.message_task.borrow_mut().take();
                    match result {
                        Ok(page) => c.adopt_page(page),
                        Err(error) => {
                            tracing::warn!(kind=?error, "Reaction send failed");
                            ui.notify("Couldn't send the reaction");
                        }
                    }
                }
            ),
        ));
    }
    /// Scroll a reply quote's original message into view when it is loaded.
    pub(super) fn scroll_to_message(&self, id: &str) {
        let Some(list) = self.list.borrow().clone() else {
            return;
        };
        for index in 0..self.store.n_items() {
            let Some(object) = self
                .store
                .item(index)
                .and_downcast::<glib::BoxedAnyObject>()
            else {
                continue;
            };
            if matches!(&*object.borrow::<Item>(), Item::Message(m) if m.id == id) {
                list.scroll_to(index, gtk::ListScrollFlags::NONE, None);
                return;
            }
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.notify("The original message isn't loaded yet");
        }
    }
    /// Send a tray sticker and reconcile delivery from the refreshed history.
    fn send_sticker(self: &Rc<Self>, sticker: &Sticker) {
        let Route::Thread(thread) = self.route.borrow().clone() else {
            return;
        };
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some(client) = ui.client.borrow().clone() else {
            return;
        };
        let sticker = sticker.clone();
        let context = offline_id();
        *self.message_task.borrow_mut() = Some(app::background(
            async move { client.send_sticker(&thread, &sticker, &context).await },
            glib::clone!(
                #[weak(rename_to=c)]
                self,
                #[weak]
                ui,
                move |result| {
                    c.message_task.borrow_mut().take();
                    match result {
                        Ok(Some(page)) => c.adopt_page(page),
                        Ok(None) => {
                            ui.notify("Sticker delivery is unconfirmed. Refresh before retrying.")
                        }
                        Err(error) => {
                            tracing::warn!(kind=?error, "Sticker send failed");
                            ui.notify("Couldn't send the sticker");
                        }
                    }
                }
            ),
        ));
    }
    /// The smiley button left of the entry opens the sticker tray popover.
    fn sticker_button(self: &Rc<Self>, ui: &Rc<Ui>) -> gtk::MenuButton {
        let button = gtk::MenuButton::new();
        button.set_icon_name("face-smile-symbolic");
        button.add_css_class("flat");
        button.set_valign(gtk::Align::Center);
        button.set_tooltip_text(Some("Send a sticker"));
        button.update_property(&[gtk::accessible::Property::Label("Send a sticker")]);
        let popover = gtk::Popover::new();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(300)
            .max_content_height(360)
            .propagate_natural_height(true)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(8);
        content.set_margin_end(8);
        scroll.set_child(Some(&content));
        popover.set_child(Some(&scroll));
        button.set_popover(Some(&popover));
        popover.connect_map(glib::clone!(
            #[weak(rename_to=c)]
            self,
            #[weak]
            ui,
            #[weak]
            content,
            move |popover| {
                c.populate_sticker_tray(&ui, &content, popover);
                if c.sticker_packs.borrow().is_some() {
                    return;
                }
                let Some(client) = ui.client.borrow().clone() else {
                    return;
                };
                *c.sticker_task.borrow_mut() = Some(app::background(
                    async move { client.stickers().await },
                    glib::clone!(
                        #[weak]
                        c,
                        #[weak]
                        ui,
                        #[weak]
                        content,
                        #[weak]
                        popover,
                        move |result| {
                            c.sticker_task.borrow_mut().take();
                            match result {
                                Ok(packs) => {
                                    c.sticker_packs.borrow_mut().replace(Rc::new(packs));
                                }
                                Err(error) => {
                                    tracing::warn!(kind=?error, "Sticker tray failed");
                                    c.sticker_packs.borrow_mut().replace(Rc::new(Vec::new()));
                                }
                            }
                            c.populate_sticker_tray(&ui, &content, &popover);
                        }
                    ),
                ));
            }
        ));
        button
    }
    fn populate_sticker_tray(
        self: &Rc<Self>,
        ui: &Rc<Ui>,
        content: &gtk::Box,
        popover: &gtk::Popover,
    ) {
        while let Some(child) = content.first_child() {
            content.remove(&child);
        }
        match self.sticker_packs.borrow().clone() {
            Some(packs) if !packs.is_empty() => {
                for pack in packs.iter() {
                    if !pack.title.is_empty() {
                        let heading = label(&pack.title);
                        heading.add_css_class("caption-heading");
                        heading.set_halign(gtk::Align::Start);
                        content.append(&heading);
                    }
                    let grid = gtk::FlowBox::new();
                    grid.set_selection_mode(gtk::SelectionMode::None);
                    grid.set_min_children_per_line(4);
                    grid.set_max_children_per_line(4);
                    grid.set_homogeneous(true);
                    grid.set_row_spacing(6);
                    grid.set_column_spacing(6);
                    for sticker in &pack.stickers {
                        let cell = gtk::Button::new();
                        cell.add_css_class("flat");
                        cell.add_css_class("sticker-cell");
                        let preview = media::preview(ui.images.clone(), &sticker.media, 400, false);
                        preview.set_size_request(64, 64);
                        cell.set_child(Some(&preview));
                        if !sticker.alt.is_empty() {
                            cell.set_tooltip_text(Some(&sticker.alt));
                        }
                        cell.connect_clicked(glib::clone!(
                            #[weak]
                            popover,
                            #[weak(rename_to=c)]
                            self,
                            #[strong]
                            sticker,
                            move |_| {
                                popover.popdown();
                                c.send_sticker(&sticker);
                            }
                        ));
                        grid.insert(&cell, -1);
                    }
                    content.append(&grid);
                }
            }
            Some(_) => {
                let empty = label("No stickers available");
                empty.add_css_class("dim-label");
                content.append(&empty);
            }
            None => {
                let spinner = adw::Spinner::new();
                spinner.set_size_request(24, 24);
                spinner.set_halign(gtk::Align::Center);
                spinner.set_margin_top(24);
                spinner.set_margin_bottom(24);
                content.append(&spinner);
            }
        }
    }
    fn load_near_end(self: &Rc<Self>) {
        if !self.scrolled.is_mapped() || self.auto_failed.get() {
            return;
        }
        let adjustment = self.scrolled.vadjustment();
        if adjustment.page_size() > 0.0
            && adjustment.upper() - adjustment.value() - adjustment.page_size()
                < adjustment.page_size().max(400.0) * 0.75
        {
            self.load();
        }
    }
    /// Rows only start fetching when they map; warm the cache now so media is
    /// already arriving before the list scrolls to it. Sizes must match the
    /// renderers in `render_item` or the keys miss the shared request locks.
    fn prefetch(&self, ui: &Rc<Ui>, additions: &[glib::BoxedAnyObject]) {
        let grid = self.route.borrow().grid();
        for object in additions {
            match &*object.borrow::<Item>() {
                Item::Post(post) => {
                    if let Some(media) = post.media.first() {
                        ui.images
                            .prefetch_media(media, if grid { 400 } else { 1080 });
                    }
                    ui.images.prefetch_avatar(&post.author, 32);
                }
                Item::User(user) => ui.images.prefetch_avatar(user, 40),
                Item::Conversation(thread) => {
                    if let Some(user) = thread.participants.first() {
                        ui.images.prefetch_avatar(user, 40);
                    }
                }
                Item::Comment(comment) => ui.images.prefetch_avatar(&comment.author, 36),
                Item::Story(story) => ui.images.prefetch_avatar(&story.author, 40),
                Item::Notification(notification) => {
                    if let Some(user) = &notification.user {
                        ui.images.prefetch_avatar(user, 40);
                    }
                }
                Item::Message(message) => {
                    for attachment in &message.attachments {
                        match attachment {
                            Attachment::Media(media) => ui.images.prefetch_media(
                                media,
                                if media.video.is_some() { 1080 } else { 400 },
                            ),
                            Attachment::Animated { media, .. } => {
                                ui.images.prefetch_media(media, 400);
                            }
                            Attachment::Post(post) => {
                                if let Some(media) = post.media.first() {
                                    ui.images.prefetch_media(media, 400);
                                }
                            }
                            Attachment::Link {
                                image_url: Some(image),
                                ..
                            } => ui.images.prefetch(Some(image.clone()), 160),
                            Attachment::Link { .. }
                            | Attachment::Voice { .. }
                            | Attachment::Unavailable => {}
                        }
                    }
                    if let Some(quoted) = &message.reply_to
                        && let Some(media) = &quoted.thumbnail
                    {
                        ui.images.prefetch_media(media, 160);
                    }
                }
                Item::FeedHeader => {}
            }
        }
    }
    pub fn back(&self) -> bool {
        if let Some(split) = self.split.borrow().as_ref()
            && split.is_collapsed()
            && split.shows_content()
        {
            split.set_show_content(false);
            return true;
        }
        false
    }
    /// Transient surfaces call this when they reappear; the first map-time
    /// load already covers never-loaded collections.
    pub(super) fn poll(self: &Rc<Self>) {
        if self.pagination.borrow().started {
            self.refresh();
        }
    }
    pub fn refresh(self: &Rc<Self>) {
        if let Some(thread) = self.thread.borrow().as_ref() {
            thread.refresh();
        }
        self.task.borrow_mut().take();
        self.profile_task.borrow_mut().take();
        self.pagination.borrow_mut().reset();
        self.auto_failed.set(false);
        self.replacing.set(true);
        self.load();
    }
    #[cfg(test)]
    pub(super) fn review_open_thread(self: &Rc<Self>) {
        self.activate(0);
        let thread = self.thread.borrow().as_ref().unwrap().clone();
        thread.review_finish(Ok(Page::complete(vec![Item::Message(Message {
            id: "review-message".into(),
            sender: "42".into(),
            text: "See you soon! I'll send the photos here.".into(),
            timestamp: 1788800000000000,
            attachments: vec![],
            seen_by: vec!["alex".into()],
            ..Message::default()
        })])));
    }
    #[cfg(test)]
    pub(super) fn review_dm_media(self: &Rc<Self>) {
        let thread = self.thread.borrow().as_ref().unwrap().clone();
        let image = Media {
            thumbnail: Some("fixture://image".into()),
            image: Some("fixture://image".into()),
            video: None,
            width: 800,
            height: 600,
        };
        let post = Post {
            id: "fixture".into(),
            code: "fixture".into(),
            author: User {
                username: "alex".into(),
                ..User::default()
            },
            caption: "A photo from today".into(),
            media: vec![image.clone()],
            liked: false,
            likes: 1200,
            comments: 34,
            timestamp: 1788800000,
        };
        let sticker = Media {
            thumbnail: Some("fixture://image".into()),
            image: Some("fixture://image".into()),
            video: None,
            width: 240,
            height: 240,
        };
        let video = Media {
            thumbnail: Some("fixture://image".into()),
            image: Some("fixture://image".into()),
            video: Some("fixture://video".into()),
            width: 1280,
            height: 720,
        };
        let mut video_post = post.clone();
        video_post.media = vec![video.clone()];
        let items = vec![
            Item::Message(Message {
                id: "a".into(),
                sender: "42".into(),
                text: "Here are the photos".into(),
                timestamp: 1788800000000000,
                reactions: vec![Reaction {
                    emoji: "❤️".into(),
                    sender: "me".into(),
                    timestamp: 1788800010,
                }],
                ..Message::default()
            }),
            Item::Message(Message {
                id: "b".into(),
                sender: "42".into(),
                attachments: vec![Attachment::Media(image)],
                timestamp: 1788800060000000,
                reactions: vec![
                    Reaction {
                        emoji: "❤️".into(),
                        sender: "me".into(),
                        timestamp: 1788800070,
                    },
                    Reaction {
                        emoji: "😂".into(),
                        sender: "42".into(),
                        timestamp: 1788800080,
                    },
                ],
                ..Message::default()
            }),
            Item::Message(Message {
                id: "c".into(),
                sender: "42".into(),
                attachments: vec![Attachment::Post(Box::new(post))],
                timestamp: 1788801860000000,
                ..Message::default()
            }),
            Item::Message(Message {
                id: "d".into(),
                sender: "me".into(),
                text: "Love it, where was this?".into(),
                timestamp: 1788802000000000,
                reply_to: Some(QuotedMessage {
                    id: "a".into(),
                    sender: "42".into(),
                    summary: "Here are the photos".into(),
                    thumbnail: None,
                }),
                ..Message::default()
            }),
            Item::Message(Message {
                id: "e".into(),
                sender: "42".into(),
                attachments: vec![Attachment::Animated {
                    media: sticker,
                    alt: "Cat waving".into(),
                }],
                timestamp: 1788802100000000,
                ..Message::default()
            }),
            Item::Message(Message {
                id: "f".into(),
                sender: "42".into(),
                attachments: vec![Attachment::Voice {
                    url: "fixture://voice".into(),
                    duration_ms: 32000,
                    waveform: (0..48).map(|i| ((i % 9) as f32 + 1.0) / 10.0).collect(),
                }],
                timestamp: 1788802200000000,
                ..Message::default()
            }),
            Item::Message(Message {
                id: "g".into(),
                sender: "me".into(),
                attachments: vec![Attachment::Link {
                    title: "The trail we talked about".into(),
                    url: "https://example.com/trail".into(),
                    image_url: Some("fixture://image".into()),
                }],
                timestamp: 1788802300000000,
                ..Message::default()
            }),
            Item::Message(Message {
                id: "v".into(),
                sender: "me".into(),
                attachments: vec![Attachment::Media(video)],
                timestamp: 1788802400000000,
                ..Message::default()
            }),
            Item::Message(Message {
                id: "w".into(),
                sender: "42".into(),
                attachments: vec![Attachment::Post(Box::new(video_post))],
                timestamp: 1788802500000000,
                ..Message::default()
            }),
            Item::Message(Message {
                id: "h".into(),
                sender: "42".into(),
                attachments: vec![Attachment::Unavailable],
                // A >15m gap starts a new group so its divider lands in the
                // bottom of the viewport where the mapped-only check sees it.
                timestamp: 1788803500000000,
                reactions: vec![
                    Reaction {
                        emoji: "❤️".into(),
                        sender: "me".into(),
                        timestamp: 1788802450,
                    },
                    Reaction {
                        emoji: "😂".into(),
                        sender: "42".into(),
                        timestamp: 1788802460,
                    },
                ],
                ..Message::default()
            }),
        ];
        thread.refresh();
        thread.review_finish(Ok(Page::complete(items.clone())));
        let first = thread.store.item(0).unwrap();
        thread.refresh();
        assert!(!thread.more.is_visible());
        thread.review_finish(Ok(Page::complete(items)));
        assert_eq!(
            first,
            thread.store.item(0).unwrap(),
            "unchanged refresh retains message widgets"
        );
        thread.set_reply_to(Some(Message {
            id: "a".into(),
            sender: "42".into(),
            text: "Here are the photos".into(),
            ..Message::default()
        }));
        assert!(
            thread.reply_bar.borrow().as_ref().unwrap().is_visible(),
            "choosing a reply shows the bar"
        );
    }
    #[cfg(test)]
    pub(super) fn review_finish(self: &Rc<Self>, result: Result<Page, crate::viewfinder::Error>) {
        let generation = self.pagination.borrow().generation;
        self.finish(generation, result);
    }
    #[cfg(test)]
    pub(super) fn review_refresh_states(self: &Rc<Self>) {
        let count = self.store.n_items();
        assert!(count > 0);
        let old = self.pagination.borrow().generation;
        self.refresh();
        assert_eq!(self.store.n_items(), count, "refresh preserves content");
        self.finish(old, Ok(Page::complete(vec![])));
        assert_eq!(self.store.n_items(), count, "stale response ignored");
        self.review_finish(Err(crate::viewfinder::Error::Network));
        assert_eq!(
            self.store.n_items(),
            count,
            "failed refresh preserves content"
        );
        assert!(self.banner.is_revealed());
        self.review_finish(Ok(Page::complete(vec![])));
        let remaining = (0..self.store.n_items())
            .filter_map(|i| self.store.item(i).and_downcast::<glib::BoxedAnyObject>())
            .filter(|o| !matches!(&*o.borrow::<Item>(), Item::FeedHeader))
            .count();
        assert_eq!(remaining, 0, "successful refresh replaces content");
        assert!(!self.retry.is_visible(), "empty state has no retry action");
    }
    pub fn reset(&self) {
        if let Some(stories) = &self.stories {
            stories.reset();
        }
        if let Some(thread) = self.thread.borrow_mut().take() {
            thread.reset();
        }
        if let Some(split) = self.split.borrow().as_ref() {
            split.set_show_content(false);
            let empty = adw::StatusPage::builder()
                .icon_name("mail-unread-symbolic")
                .title("Your conversations")
                .description("Choose a conversation to see your messages and shared posts.")
                .build();
            split.set_content(Some(&adw::NavigationPage::new(&empty, "Conversation")));
        }
        self.replacing.set(false);
        self.stick_bottom.set(false);
        self.scrolled.set_opacity(1.0);
        if self.own_profile {
            *self.route.borrow_mut() = Route::Profile(User::default());
        }
        self.task.borrow_mut().take();
        self.profile_task.borrow_mut().take();
        self.action_task.borrow_mut().take();
        self.read_task.borrow_mut().take();
        self.message_task.borrow_mut().take();
        self.read_watermark.borrow_mut().clear();
        self.reply_to.borrow_mut().take();
        if let Some(bar) = self.reply_bar.borrow().as_ref() {
            bar.set_visible(false);
        }
        self.sticker_packs.borrow_mut().take();
        self.pagination.borrow_mut().reset();
        self.auto_failed.set(false);
        self.store.remove_all();
        self.retry.set_visible(false);
        self.stack.set_visible_child_name("loading");
        self.banner.set_revealed(false);
        self.more.set_visible(false);
        while let Some(child) = self.header.first_child() {
            self.header.remove(&child);
        }
    }
    pub(super) fn search(self: &Rc<Self>, query: String, posts: bool) {
        let route = if posts {
            Route::Keyword(query)
        } else {
            Route::Search(query)
        };
        if route.grid() == self.route.borrow().grid() {
            self.reset();
            *self.route.borrow_mut() = route.clone();
            if let Some(page) = self.page.borrow().as_ref() {
                page.set_title(&route.title());
            }
            self.load();
        } else if let Some(ui) = self.ui.upgrade() {
            // Changing result type requires the other recycled view (grid/list).
            ui.navigation.pop();
            ui.push(route);
        }
    }
    pub fn load(self: &Rc<Self>) {
        if let Some(stories) = &self.stories {
            stories.load();
        }
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some(client) = ui.client.borrow().clone() else {
            return;
        };
        let Some((generation, cursor)) = self.pagination.borrow_mut().begin() else {
            return;
        };
        self.retry.set_visible(false);
        if self.store.n_items() == 0 && !self.replacing.get() {
            self.stack.set_visible_child_name("loading");
        }
        self.more.set_visible(
            !matches!(
                *self.route.borrow(),
                Route::Reels | Route::Thread(_) | Route::Inbox
            ) && self.store.n_items() > 0,
        );
        self.banner.set_revealed(false);
        let route = self.route.borrow().clone();
        if let Route::Profile(user) = &route {
            let id = if user.id.is_empty() {
                client.account_id.clone()
            } else {
                user.id.clone()
            };
            let user = user.clone();
            let client = client.clone();
            *self.task.borrow_mut() = Some(app::background(
                async move {
                    // The timeline only needs the username; without it the
                    // profile fetch must finish first to learn it.
                    if user.username.is_empty() {
                        let user = client.profile(&id).await?;
                        let page = client.load(Route::Profile(user.clone()), cursor).await;
                        Ok::<_, crate::viewfinder::Error>((user, page))
                    } else {
                        let (user, page) = tokio::join!(
                            client.profile(&id),
                            client.load(Route::Profile(user.clone()), cursor)
                        );
                        Ok((user?, page))
                    }
                },
                glib::clone!(
                    #[weak(rename_to=c)]
                    self,
                    move |result| {
                        if c.pagination.borrow().generation != generation {
                            return;
                        }
                        match result {
                            Ok((user, page)) => {
                                *c.route.borrow_mut() = Route::Profile(user.clone());
                                c.profile_header(user);
                                c.finish(generation, page);
                            }
                            Err(e) => c.finish(generation, Err(e)),
                        }
                    }
                ),
            ));
        } else {
            *self.task.borrow_mut() = Some(app::background(
                app::load(client, route, cursor),
                glib::clone!(
                    #[weak(rename_to=c)]
                    self,
                    move |result| c.finish(generation, result)
                ),
            ));
        }
    }
    pub(super) fn finish(
        self: &Rc<Self>,
        generation: u64,
        result: Result<Page, crate::viewfinder::Error>,
    ) {
        if self.pagination.borrow().generation != generation {
            return;
        }
        self.more.set_visible(false);
        match result {
            Ok(mut page) => {
                self.auto_failed.set(false);
                self.banner.set_revealed(false);
                self.banner.set_button_label(None);
                let adjustment = self.scrolled.vadjustment();
                let fresh = self.store.n_items() == 0;
                let initial = fresh
                    || adjustment.upper() - adjustment.value() - adjustment.page_size() < 40.0;
                if !self.pagination.borrow_mut().finish(generation, &mut page) {
                    return;
                }
                let replacing = self.replacing.replace(false);
                let mut additions: Vec<_> = page
                    .items
                    .into_iter()
                    .map(glib::BoxedAnyObject::new)
                    .collect();
                if let Some(ui) = self.ui.upgrade() {
                    self.prefetch(&ui, &additions);
                }
                if matches!(*self.route.borrow(), Route::Home)
                    && (replacing || self.store.n_items() == 0)
                {
                    additions.insert(0, glib::BoxedAnyObject::new(Item::FeedHeader));
                }
                if replacing {
                    if matches!(*self.route.borrow(), Route::Thread(_) | Route::Inbox) {
                        // Keep equal objects alive so polling doesn't reload their media.
                        let old_len = self.store.n_items() as usize;
                        let equal = |old: usize, new: usize| {
                            self.store
                                .item(old as u32)
                                .and_downcast::<glib::BoxedAnyObject>()
                                .is_some_and(|o| {
                                    *o.borrow::<Item>() == *additions[new].borrow::<Item>()
                                })
                        };
                        let mut start = 0;
                        while start < old_len.min(additions.len()) && equal(start, start) {
                            start += 1;
                        }
                        let mut end = 0;
                        while end < old_len.min(additions.len()) - start
                            && equal(old_len - end - 1, additions.len() - end - 1)
                        {
                            end += 1;
                        }
                        if start + end != old_len || start + end != additions.len() {
                            self.store.splice(
                                start as u32,
                                (old_len - start - end) as u32,
                                &additions[start..additions.len() - end],
                            );
                        }
                    } else {
                        self.store.splice(0, self.store.n_items(), &additions);
                    }
                } else {
                    self.store.splice(self.store.n_items(), 0, &additions);
                }
                if let Some(reels) = self.reels.borrow().as_ref() {
                    reels.set_exhausted(self.pagination.borrow().exhausted);
                }
                if initial
                    && self.store.n_items() > 0
                    && matches!(*self.route.borrow(), Route::Thread(_))
                {
                    self.stick_bottom.set(true);
                    if fresh {
                        self.scrolled.set_opacity(0.0);
                    }
                    self.scroll_to_bottom();
                }
                glib::idle_add_local_once(glib::clone!(
                    #[weak(rename_to=c)]
                    self,
                    move || c.mark_visible_read()
                ));
                if self.store.n_items() == 0 {
                    self.retry.set_visible(false);
                    self.status.set_title(match &*self.route.borrow() {
                        Route::Thread(_) => "No messages yet",
                        Route::Inbox => "No conversations yet",
                        Route::Comments(_) => "No comments yet",
                        Route::Search(_) | Route::Keyword(_) => "No results",
                        Route::Stories | Route::Story(_) => "No stories to show",
                        Route::Followers(_, false) => "No followers to show",
                        Route::Followers(_, true) => "No accounts to show",
                        Route::Notifications => "No notifications",
                        _ => "No posts to show",
                    });
                    self.status.set_description(None);
                    self.status.set_icon_name(Some("folder-symbolic"));
                    match &*self.route.borrow() {
                        Route::Notifications => {
                            self.status.set_title("All quiet for now");
                            self.status.set_description(Some(
                                "Likes, comments, and new connections will appear here.",
                            ));
                            self.status
                                .set_icon_name(Some("preferences-system-notifications-symbolic"));
                        }
                        Route::Search(_) | Route::Keyword(_) => {
                            self.status.set_description(Some(
                                "Try a different spelling or a broader search above.",
                            ));
                            self.status.set_icon_name(Some("system-search-symbolic"));
                        }
                        Route::Profile(user) if user.private => {
                            self.status.set_title("This account is private");
                            self.status.set_description(Some(
                                "Posts are only visible to approved followers.",
                            ));
                            self.status.set_icon_name(Some("channel-secure-symbolic"));
                        }
                        _ => {}
                    }
                    self.stack.set_visible_child_name("status");
                } else {
                    self.stack.set_visible_child_name("content");
                }
                glib::idle_add_local_once(glib::clone!(
                    #[weak(rename_to=c)]
                    self,
                    move || c.load_near_end()
                ));
                if matches!(*self.route.borrow(), Route::Story(_)) && self.store.n_items() > 0 {
                    self.activate(0);
                }
            }
            Err(error) => {
                self.auto_failed.set(true);
                self.pagination.borrow_mut().loading = false;
                tracing::warn!(kind=?error,"Collection could not load");
                if self.store.n_items() == 0 {
                    self.retry.set_visible(true);
                    self.status.set_title("Could not load content");
                    self.status.set_description(Some(&error.to_string()));
                    self.status.set_icon_name(Some("dialog-warning-symbolic"));
                    self.stack.set_visible_child_name("status");
                } else {
                    self.banner.set_button_label(Some("Try Again"));
                    self.banner.set_title(&error.to_string());
                    self.banner.set_revealed(true);
                }
            }
        }
    }
    fn activate(self: &Rc<Self>, position: u32) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let Some(object) = self
            .store
            .item(position)
            .and_downcast::<glib::BoxedAnyObject>()
        else {
            return;
        };
        let item = object.borrow::<Item>().clone();
        if let Some(dismiss) = self.dismiss.borrow().as_ref() {
            dismiss();
        }
        match item {
            Item::Post(post) => {
                let posts: Vec<Post> = (0..self.store.n_items())
                    .filter_map(|i| self.store.item(i).and_downcast::<glib::BoxedAnyObject>())
                    .filter_map(|o| {
                        if let Item::Post(p) = &*o.borrow::<Item>() {
                            Some(p.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                let index = posts
                    .iter()
                    .position(|p| p.id == post.id)
                    .unwrap_or_default();
                viewer::present(
                    &ui,
                    posts,
                    index,
                    matches!(*self.route.borrow(), Route::Story(_)),
                );
            }
            Item::User(user) => ui.push(Route::Profile(user)),
            Item::Conversation(thread) => {
                if let Some(list) = self.list.borrow().as_ref()
                    && let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>()
                {
                    selection.set_selected(position);
                }
                if let Some(split) = self.split.borrow().as_ref() {
                    let collection = Collection::new(&ui, Route::Thread(thread.clone()));
                    let toolbar = adw::ToolbarView::new();
                    let header = adw::HeaderBar::new();
                    header.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
                    toolbar.add_top_bar(&header);
                    toolbar.set_content(Some(&collection.root));
                    self.thread.replace(Some(collection));
                    split.set_content(Some(&adw::NavigationPage::new(&toolbar, &thread.title)));
                    split.set_show_content(true);
                } else {
                    ui.push(Route::Thread(thread));
                }
            }
            Item::Story(story) => ui.push(Route::Story(story)),
            Item::Notification(n) => {
                if let Some(user) = n.user {
                    ui.push(Route::Profile(user));
                }
            }
            Item::Comment(c) => ui.push(Route::Profile(c.author)),
            Item::Message(_) | Item::FeedHeader => (),
        }
    }
    pub(super) fn profile_header(self: &Rc<Self>, user: User) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        while let Some(child) = self.header.first_child() {
            self.header.remove(&child);
        }
        self.header.set_margin_start(12);
        self.header.set_margin_end(12);
        self.header.set_margin_top(12);
        self.header.set_margin_bottom(12);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.add_css_class("card");
        card.add_css_class("profile-card");
        self.header.append(&card);
        let identity = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let avatar = media::avatar(ui.images.clone(), &user, 64);
        identity.append(&avatar);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let name = label(if user.name.is_empty() {
            &user.username
        } else {
            &user.name
        });
        name.add_css_class("title-2");
        name.set_lines(2);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&name);
        let handle = label(&format!("@{}", user.username));
        handle.add_css_class("dim-label");
        handle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        handle.set_lines(1);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&handle);
        identity.append(&text);
        card.append(&identity);
        if !user.biography.is_empty() {
            let about = adw::ExpanderRow::builder()
                .title("About")
                .subtitle(if user.private {
                    "Private account"
                } else {
                    "Biography"
                })
                .build();
            let bio = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .max_content_height(100)
                .child(&label(&user.biography))
                .build();
            bio.set_margin_start(12);
            bio.set_margin_end(12);
            bio.set_margin_bottom(12);
            about.add_row(&bio);
            let about_list = gtk::ListBox::new();
            about_list.set_selection_mode(gtk::SelectionMode::None);
            about_list.add_css_class("boxed-list");
            about_list.append(&about);
            self.header.append(&about_list);
        } else if user.private {
            let privacy = label("Private account");
            privacy.add_css_class("dim-label");
            card.append(&privacy);
        }
        let counts = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        counts.set_homogeneous(true);
        let count = |number: u64, title: &str| {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let value = gtk::Label::new(Some(&format::compact_count(number)));
            value.add_css_class("heading");
            content.append(&value);
            let caption = gtk::Label::new(Some(title));
            caption.add_css_class("dim-label");
            content.append(&caption);
            content
        };
        let followers = gtk::Button::builder()
            .child(&count(user.followers, "Followers"))
            .build();
        followers.add_css_class("flat");
        followers.set_tooltip_text(Some(&format!("{} followers", user.followers)));
        let following = gtk::Button::builder()
            .child(&count(user.following_count, "Following"))
            .build();
        following.add_css_class("flat");
        following.set_tooltip_text(Some(&format!("{} following", user.following_count)));
        let posts = count(user.posts, "Posts");
        posts.set_valign(gtk::Align::Center);
        counts.append(&posts);
        counts.append(&followers);
        counts.append(&following);
        card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        card.append(&counts);
        let posts_heading = label("Posts");
        posts_heading.add_css_class("heading");
        posts_heading.set_margin_start(4);
        self.header.append(&posts_heading);
        followers.connect_clicked(glib::clone!(
            #[weak]
            ui,
            #[strong]
            user,
            move |_| ui.push(Route::Followers(user.clone(), false))
        ));
        following.connect_clicked(glib::clone!(
            #[weak]
            ui,
            #[strong]
            user,
            move |_| ui.push(Route::Followers(user.clone(), true))
        ));
        let Some(client) = ui.client.borrow().clone() else {
            return;
        };
        if user.id == client.account_id {
            return;
        }
        let follow = gtk::Button::with_label("Loading…");
        follow.set_tooltip_text(Some("Loading relationship"));
        follow.set_sensitive(false);
        follow.set_halign(gtk::Align::End);
        follow.set_valign(gtk::Align::Center);
        identity.append(&follow);
        let state = Rc::new(RefCell::new(user.relationship));
        let id = user.id.clone();
        *self.profile_task.borrow_mut() = Some(app::background(
            async move { client.relationship(&id).await },
            glib::clone!(
                #[weak]
                follow,
                #[strong]
                state,
                move |r| {
                    if let Ok(r) = r {
                        *state.borrow_mut() = r;
                        follow.set_label(relationship_label(r));
                        follow.set_tooltip_text(None);
                        follow.set_sensitive(r != Relationship::Unknown);
                        if r == Relationship::NotFollowing {
                            follow.add_css_class("suggested-action");
                        }
                    } else {
                        follow.set_label("Unavailable");
                        follow.set_tooltip_text(Some("Relationship unavailable"));
                    }
                }
            ),
        ));
        follow.connect_clicked(glib::clone!(
            #[weak(rename_to=c)]
            self,
            #[weak]
            ui,
            #[strong]
            user,
            move |button| {
                let Some(client) = ui.client.borrow().clone() else {
                    return;
                };
                let target = *state.borrow() != Relationship::Following
                    && *state.borrow() != Relationship::Requested;
                button.set_sensitive(false);
                let user = user.clone();
                *c.action_task.borrow_mut() = Some(app::background(
                    async move { client.follow(&user, target).await },
                    glib::clone!(
                        #[weak]
                        button,
                        #[weak]
                        ui,
                        #[strong]
                        state,
                        move |r| {
                            button.set_sensitive(true);
                            match r {
                                Ok(s) => {
                                    *state.borrow_mut() = s;
                                    button.set_label(relationship_label(s));
                                    if s == Relationship::NotFollowing {
                                        button.add_css_class("suggested-action");
                                    } else {
                                        button.remove_css_class("suggested-action");
                                    }
                                }
                                Err(e) => ui.notify(&e.to_string()),
                            }
                        }
                    ),
                ));
            }
        ));
    }
    fn composer(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let key = match &*self.route.borrow() {
            Route::Thread(t) => format!("thread:{}", t.id),
            Route::Comments(p) => format!("post:{}", p.id),
            _ => return,
        };
        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        box_.set_margin_start(20);
        box_.set_margin_end(20);
        box_.set_margin_top(12);
        box_.set_margin_bottom(16);
        let entry = gtk::Entry::builder()
            .placeholder_text("Write a message…")
            .hexpand(true)
            .build();
        if matches!(*self.route.borrow(), Route::Comments(_)) {
            entry.set_placeholder_text(Some("Write a comment…"));
        }
        if let Some(draft) = ui.drafts.borrow().get(&key) {
            entry.set_text(draft);
        }
        entry.connect_changed(glib::clone!(
            #[weak]
            ui,
            #[strong]
            key,
            move |entry| {
                ui.drafts
                    .borrow_mut()
                    .insert(key.clone(), entry.text().to_string());
            }
        ));
        let send = icon_button("mail-send-symbolic", "Send");
        send.add_css_class("suggested-action");
        if matches!(*self.route.borrow(), Route::Comments(_)) {
            send.set_label("Post");
            send.set_tooltip_text(Some("Post comment"));
            send.update_property(&[gtk::accessible::Property::Label("Post comment")]);
        } else {
            send.add_css_class("circular");
            entry.add_css_class("message-entry");
            entry.set_enable_emoji_completion(true);
        }
        send.set_valign(gtk::Align::Center);
        send.set_sensitive(!entry.text().trim().is_empty());
        entry.update_property(&[gtk::accessible::Property::Label(
            if matches!(*self.route.borrow(), Route::Comments(_)) {
                "Comment"
            } else {
                "Message"
            },
        )]);
        entry.connect_changed(glib::clone!(
            #[weak]
            send,
            move |entry| send.set_sensitive(entry.is_editable() && !entry.text().trim().is_empty())
        ));
        if matches!(*self.route.borrow(), Route::Thread(_)) {
            box_.append(&self.sticker_button(&ui));
        }
        box_.append(&entry);
        box_.append(&send);
        let composer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        composer.add_css_class("message-composer");
        let reply_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        reply_bar.add_css_class("reply-bar");
        reply_bar.set_margin_start(20);
        reply_bar.set_margin_end(20);
        reply_bar.set_visible(false);
        self.reply_bar.replace(Some(reply_bar.clone()));
        self.composer_entry.replace(Some(entry.clone()));
        composer.append(&reply_bar);
        let send_status = label("");
        send_status.add_css_class("caption");
        send_status.set_margin_start(20);
        send_status.set_margin_end(20);
        send_status.set_accessible_role(gtk::AccessibleRole::Status);
        send_status.set_visible(false);
        composer.append(&send_status);
        composer.append(&adw::Clamp::builder().maximum_size(760).child(&box_).build());
        self.root.append(&composer);
        send.connect_clicked(glib::clone!(#[weak(rename_to=c)] self,#[weak] ui,#[weak] entry,#[weak] send_status,move |send| {
            let text=entry.text().trim().to_owned();if text.is_empty(){return;}
            let Some(client)=ui.client.borrow().clone() else {return;};let route=c.route.borrow().clone();
            let is_dm=matches!(route,Route::Thread(_));
            send.set_sensitive(false);entry.set_editable(false);
            send.set_tooltip_text(Some("Sending…"));
            let spinner=adw::Spinner::new();
            spinner.set_size_request(16,16);
            send.set_child(Some(&spinner));
            // Retain draft for any uncertain delivery; never automatically resend.
            let context=offline_id();
            let reply=c.reply_to.borrow().clone();
            c.set_reply_to(None);
            let reply_restore=reply.clone();
            *c.action_task.borrow_mut()=Some(app::background(async move {match route {
                Route::Thread(t)=>client.send_message(&t,&text,&context,reply.as_ref()).await.map(|page|(None,page)),
                Route::Comments(p)=>client.comment(&p,&text).await.map(|c|(Some(Item::Comment(c)),None)),
                _=>Err(crate::viewfinder::Error::Unsupported),
            }},glib::clone!(#[weak] c,#[weak] ui,#[weak] entry,#[weak] send,#[weak] send_status,#[strong] reply_restore,move |r|{
                entry.set_editable(true);send.set_sensitive(!entry.text().trim().is_empty());
                send.set_tooltip_text(Some(if is_dm { "Send" } else { "Post comment" }));
                if is_dm {
                    send.set_child(Some(&gtk::Image::from_icon_name("mail-send-symbolic")));
                } else {
                    send.set_child(Some(&gtk::Label::new(Some("Post"))));
                }
                match r {
                    Ok((Some(item),_))=>{send_status.set_visible(false);c.store.append(&glib::BoxedAnyObject::new(item));c.stack.set_visible_child_name("content");entry.set_text("");},
                    Ok((_,Some(page)))=>{
                        c.task.borrow_mut().take();
                        c.pagination.borrow_mut().reset();
                        c.replacing.set(true);
                        let generation=c.pagination.borrow().generation;
                        c.finish(generation,Ok(page));
                        entry.set_text("");
                        send_status.set_visible(false);
                        entry.grab_focus();
                    },
                    Ok((None,None))=>{c.set_reply_to(reply_restore);c.refresh();send_status.set_label("Delivery is unconfirmed. Check the conversation before retrying; your draft is saved.");send_status.set_visible(true);},
                    Err(e)=>{tracing::warn!(kind=?e,"Send failed");if is_dm{c.set_reply_to(reply_restore);send_status.set_label("Delivery could not be confirmed. Refresh before retrying; your draft is saved.");send_status.set_visible(true);}else{send_status.set_visible(false);ui.notify(&e.to_string());}},
                }
            })));
        }));
        entry.connect_activate(glib::clone!(
            #[weak]
            send,
            move |_| if send.is_sensitive() {
                send.emit_clicked();
            }
        ));
    }
}
fn relationship_label(r: Relationship) -> &'static str {
    match r {
        Relationship::Following => "Following",
        Relationship::Requested => "Requested",
        Relationship::NotFollowing => "Follow",
        Relationship::Unknown => "Relationship unavailable",
    }
}
fn offline_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let uuid = glib::uuid_string_random();
    let bits = u32::from_str_radix(&uuid[..8], 16).unwrap_or(0) & 0x3fffff;
    ((millis << 22) | bits as u64).to_string()
}

fn render_item(
    ui: &Rc<Ui>,
    item: &Item,
    route: &Route,
    collection: &Rc<Collection>,
) -> gtk::Widget {
    if let Item::Message(message) = item {
        return super::messages::row(ui, message, route, collection);
    }
    if matches!(
        route,
        Route::Notifications | Route::Search(_) | Route::Followers(..)
    ) && let Some(row) = super::discovery::result_row(ui, item)
    {
        return row;
    }
    if let Item::Comment(comment) = item {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("card");
        row.add_css_class("discovery-row");
        let avatar = media::avatar(ui.images.clone(), &comment.author, 36);
        avatar.set_valign(gtk::Align::Start);
        row.append(&avatar);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.set_hexpand(true);
        let author = label(&format!("@{}", comment.author.username));
        author.add_css_class("heading");
        author.set_selectable(false);
        body.append(&author);
        body.append(&label(&comment.text));
        row.append(&body);
        row.append(&super::engagement::button(
            ui,
            LikeTarget::Comment(comment.id.clone()),
            comment.liked,
            comment.likes,
        ));
        return row.upcast();
    }
    if let Item::Post(post) = item {
        if route.grid() {
            let overlay = gtk::Overlay::new();
            if matches!(route, Route::Profile(_) | Route::Keyword(_)) {
                overlay.set_overflow(gtk::Overflow::Hidden);
            }
            let media = post.media.first();
            if let Some(media) = media {
                let preview = media::preview(ui.images.clone(), media, 400, true);
                preview.set_size_request(
                    140,
                    if matches!(route, Route::Reels) {
                        240
                    } else {
                        180
                    },
                );
                overlay.set_child(Some(&preview));
            } else {
                let missing = gtk::Image::from_icon_name("image-missing-symbolic");
                missing.set_size_request(140, 180);
                missing.update_property(&[gtk::accessible::Property::Label("Media unavailable")]);
                overlay.set_child(Some(&missing));
            }
            if post.media.len() > 1 || media.is_some_and(|m| m.video.is_some()) {
                let icon = gtk::Image::from_icon_name(if post.media.len() > 1 {
                    "view-paged-symbolic"
                } else {
                    "media-playback-start-symbolic"
                });
                icon.set_halign(gtk::Align::End);
                icon.set_valign(gtk::Align::Start);
                icon.set_margin_top(8);
                icon.set_margin_end(8);
                overlay.add_overlay(&icon);
            }
            overlay.update_property(&[gtk::accessible::Property::Label(&format!(
                "Post by {}",
                post.author.username
            ))]);
            return overlay.upcast();
        }
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.add_css_class("post-card");
        body.add_css_class("card");
        body.set_valign(gtk::Align::Start);
        body.set_margin_top(8);
        body.set_margin_bottom(16);
        body.set_margin_start(12);
        body.set_margin_end(12);
        let author = gtk::Button::with_label(&post.author.username);
        author.add_css_class("flat");
        author.set_halign(gtk::Align::Start);
        let identity = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        identity.append(&media::avatar(ui.images.clone(), &post.author, 32));
        identity.append(&gtk::Label::new(Some(&post.author.username)));
        author.set_child(Some(&identity));
        author.connect_clicked(glib::clone!(
            #[weak]
            ui,
            #[strong]
            post,
            move |_| ui.push(Route::Profile(post.author.clone()))
        ));
        body.append(&author);
        if let Some(m) = post.media.first() {
            if m.video.is_some() {
                body.append(&super::inline_video::new(ui, m));
            } else {
                let preview = media::preview(ui.images.clone(), m, 1080, false);
                let open = gtk::Button::new();
                open.add_css_class("post-media");
                open.set_child(Some(&super::media_frame::MediaFrame::new(
                    &preview,
                    (m.width.max(1) as f64 / m.height.max(1) as f64).clamp(0.5, 2.0),
                )));
                open.connect_clicked(glib::clone!(
                    #[weak]
                    ui,
                    #[strong]
                    post,
                    move |_| viewer::present(&ui, vec![post.clone()], 0, false)
                ));
                body.append(&open);
            }
            if post.media.len() > 1 {
                let carousel =
                    gtk::Button::with_label(&format!("1 / {} · View carousel", post.media.len()));
                carousel.add_css_class("flat");
                carousel.set_halign(gtk::Align::End);
                carousel.connect_clicked(glib::clone!(
                    #[weak]
                    ui,
                    #[strong]
                    post,
                    move |_| viewer::present(&ui, vec![post.clone()], 0, false)
                ));
                body.append(&carousel);
            }
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("post-actions");
        actions.append(&like_button(ui, post));
        let comments = comment_button(ui, post);
        actions.append(&comments);
        body.append(&actions);
        if !post.caption.is_empty() {
            let caption = label(&post.caption);
            caption.set_lines(4);
            caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
            body.append(&caption);
        }
        if let Some(date) = format::relative_time(post.timestamp) {
            let time = label(&date);
            time.add_css_class("dim-label");
            body.append(&time);
        }
        let clamp = adw::Clamp::builder()
            .valign(gtk::Align::Start)
            .maximum_size(600)
            .tightening_threshold(500)
            .child(&body)
            .build();
        return clamp.upcast();
    }
    let (title, subtitle, avatar) = match item {
        Item::User(u) => (u.username.clone(), u.name.clone(), Some(u.clone())),
        Item::Conversation(t) => (
            t.title.clone(),
            if !t.preview.is_empty()
                && ui
                    .client
                    .borrow()
                    .as_ref()
                    .is_some_and(|c| c.account_id == t.preview_sender)
            {
                format!("You: {}", t.preview)
            } else {
                t.preview.clone()
            },
            t.participants.first().cloned(),
        ),
        Item::Comment(c) => (
            c.author.username.clone(),
            c.text.clone(),
            Some(c.author.clone()),
        ),
        Item::Story(s) => (
            s.author.username.clone(),
            if s.seen { "Seen" } else { "New story" }.into(),
            Some(s.author.clone()),
        ),
        Item::Notification(n) => (n.text.clone(), String::new(), n.user.clone()),
        Item::Message(_) | Item::Post(_) | Item::FeedHeader => unreachable!(),
    };
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_start(12);
    row.set_margin_end(12);
    row.set_margin_top(10);
    row.set_margin_bottom(10);
    if matches!(item, Item::Conversation(_)) {
        row.set_margin_start(8);
        row.set_margin_end(8);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
    }
    if let Some(u) = avatar {
        row.append(&media::avatar(ui.images.clone(), &u, 40));
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let title = label(&title);
    title.set_selectable(false);
    if matches!(item, Item::Conversation(_)) {
        title.set_wrap(false);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_max_width_chars(22);
    }
    if !matches!(item, Item::Notification(_)) {
        title.add_css_class("heading");
    }
    if let Item::Conversation(thread) = item {
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        title.set_hexpand(true);
        heading.append(&title);
        if thread.unread {
            let unread = label("●");
            unread.add_css_class("accent");
            unread.set_tooltip_text(Some("Unread messages"));
            heading.append(&unread);
        }
        let time = gtk::Label::new(None);
        time.add_css_class("dim-label");
        time.add_css_class("caption");
        time.set_halign(gtk::Align::End);
        time.set_visible(false);
        heading.append(&time);
        let timestamp = thread.preview_timestamp;
        heading.add_tick_callback(glib::clone!(
            #[weak]
            title,
            #[weak]
            time,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move |heading, _| {
                let compact = format::relative_time(timestamp)
                    .map(|value| value.trim_end_matches(" ago").to_owned())
                    .unwrap_or_default();
                if time.label() != compact {
                    time.set_label(&compact);
                }
                let name_width = title
                    .create_pango_layout(Some(&title.label()))
                    .pixel_size()
                    .0;
                let time_width = time.create_pango_layout(Some(&compact)).pixel_size().0;
                time.set_visible(
                    !compact.is_empty() && heading.width() >= name_width + time_width + 8,
                );
                glib::ControlFlow::Continue
            }
        ));
        text.append(&heading);
    } else {
        text.append(&title);
    }
    if !subtitle.is_empty() {
        let subtitle = label(&subtitle);
        if !matches!(item, Item::Comment(_)) {
            subtitle.set_selectable(false);
            subtitle.set_lines(2);
            if matches!(item, Item::Conversation(_)) {
                subtitle.set_lines(1);
                subtitle.set_max_width_chars(24);
            }
            subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
            subtitle.add_css_class("dim-label");
        }
        text.append(&subtitle);
    }
    row.append(&text);
    row.upcast()
}

pub fn like_button(ui: &Rc<Ui>, post: &Post) -> gtk::Button {
    super::engagement::button(
        ui,
        LikeTarget::Post(post.id.split('_').next().unwrap_or(&post.id).to_owned()),
        post.liked,
        post.likes,
    )
}

pub(super) fn comment_button(ui: &Rc<Ui>, post: &Post) -> gtk::Button {
    let button = icon_button("mail-unread-symbolic", "View comments");
    button.add_css_class("flat");
    button.add_css_class("engagement-button");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Image::from_icon_name("viewfinder-comment-symbolic"));
    content.append(&gtk::Label::new(Some(&format::compact_count(
        post.comments,
    ))));
    button.set_child(Some(&content));
    button.connect_clicked(glib::clone!(
        #[weak]
        ui,
        #[strong]
        post,
        move |_| ui.push(Route::Comments(post.clone()))
    ));
    button
}

/// A short attachment description for the reply bar's quoted snippet.
fn message_snippet(attachments: &[Attachment]) -> &'static str {
    match attachments.first() {
        Some(Attachment::Media(media)) if media.video.is_some() => "Video",
        Some(Attachment::Media(_)) => "Photo",
        Some(Attachment::Post(_)) => "Post",
        Some(Attachment::Animated { .. }) => "Sticker",
        Some(Attachment::Link { .. }) => "Link",
        Some(Attachment::Voice { .. }) => "Voice message",
        _ => "Message",
    }
}
