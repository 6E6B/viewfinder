use super::{Ui, format, icon_button, label, viewer};
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
    banner: adw::Banner,
    header: gtk::Box,
    split: RefCell<Option<adw::NavigationSplitView>>,
    thread: RefCell<Option<Rc<Collection>>>,
    reels: RefCell<Option<Rc<super::reels::Reels>>>,
    pub(super) stories: Option<Rc<super::stories::Tray>>,
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
            banner,
            header: header.clone(),
            split: RefCell::new(None),
            thread: RefCell::new(None),
            reels: RefCell::new(None),
            stories: matches!(route, Route::Home).then(|| super::stories::Tray::new(ui)),
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
                li.set_activatable(!matches!(
                    item,
                    Item::Message(_) | Item::Notification(Notification { user: None, .. })
                ));
                if let Some(ui) = c.ui.upgrade() {
                    let mut child = render_item(&ui, &item, &c.route.borrow());
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
                                let previous = li.position().checked_sub(1).and_then(|p| c.store.item(p))
                                    .and_downcast::<glib::BoxedAnyObject>();
                                let same_group = previous.is_some_and(|o| matches!(&*o.borrow::<Item>(), Item::Message(m) if format::same_message_group(m.timestamp, timestamp)));
                                divider.set_visible(timestamp > 0 && !same_group);
                                let mut widget = row.first_child();
                                while let Some(bubble) = widget {
                                    if bubble.has_css_class("message-bubble") {
                                        for (position, class) in [
                                            (li.position().checked_sub(1), "join-previous"),
                                            (li.position().checked_add(1), "join-next"),
                                        ] {
                                            let joins = position.and_then(|p| c.store.item(p))
                                                .and_downcast::<glib::BoxedAnyObject>()
                                                .is_some_and(|o| matches!(&*o.borrow::<Item>(), Item::Message(m) if m.sender == sender && format::same_message_group(m.timestamp.min(timestamp), m.timestamp.max(timestamp))));
                                            if joins { bubble.add_css_class(class); }
                                            else { bubble.remove_css_class(class); }
                                        }
                                        break;
                                    }
                                    widget = bubble.next_sibling();
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
                Route::Notifications | Route::Search(_) | Route::Comments(_)
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
            Route::Notifications | Route::Search(_) | Route::Keyword(_)
        ) {
            root.append(&super::discovery::heading(&c, &route));
        }
        if matches!(route, Route::Home) {
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
            root.append(
                &adw::Clamp::builder()
                    .maximum_size(600)
                    .child(&heading)
                    .build(),
            );
            if let Some(stories) = &c.stories {
                root.append(
                    &adw::Clamp::builder()
                        .maximum_size(600)
                        .child(&stories.root)
                        .build(),
                );
            }
        }
        if matches!(route, Route::Inbox) {
            let bar = adw::HeaderBar::new();
            bar.set_title_widget(Some(&adw::WindowTitle::new("Messages", "")));
            bar.pack_start(&ui.sidebar_toggle());
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
                    ui.playback.stop();
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
            likes: 0,
            comments: 0,
            timestamp: 0,
        };
        let items = vec![
            Item::Message(Message {
                id: "a".into(),
                sender: "42".into(),
                text: "Here are the photos".into(),
                timestamp: 1788800000000000,
                ..Message::default()
            }),
            Item::Message(Message {
                id: "b".into(),
                sender: "42".into(),
                attachments: vec![Attachment::Media(image)],
                timestamp: 1788800060000000,
                ..Message::default()
            }),
            Item::Message(Message {
                id: "c".into(),
                sender: "42".into(),
                attachments: vec![Attachment::Post(Box::new(post))],
                timestamp: 1788801860000000,
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
        assert_eq!(
            self.store.n_items(),
            0,
            "successful refresh replaces content"
        );
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
        if self.own_profile {
            *self.route.borrow_mut() = Route::Profile(User::default());
        }
        self.task.borrow_mut().take();
        self.profile_task.borrow_mut().take();
        self.action_task.borrow_mut().take();
        self.read_task.borrow_mut().take();
        self.read_watermark.borrow_mut().clear();
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
            let client = client.clone();
            *self.task.borrow_mut() = Some(app::background(
                async move {
                    let user = client.profile(&id).await?;
                    let page = client.load(Route::Profile(user.clone()), cursor).await;
                    Ok::<_, crate::viewfinder::Error>((user, page))
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
                let initial = self.store.n_items() == 0
                    || adjustment.upper() - adjustment.value() - adjustment.page_size() < 40.0;
                if !self.pagination.borrow_mut().finish(generation, &mut page) {
                    return;
                }
                let additions: Vec<_> = page
                    .items
                    .into_iter()
                    .map(glib::BoxedAnyObject::new)
                    .collect();
                if self.replacing.replace(false) {
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
                    && let Some(list) = self.list.borrow().as_ref()
                {
                    list.scroll_to(self.store.n_items() - 1, gtk::ListScrollFlags::NONE, None);
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
        match item {
            Item::Post(_) => {
                let posts = (0..self.store.n_items())
                    .filter_map(|i| self.store.item(i).and_downcast::<glib::BoxedAnyObject>())
                    .filter_map(|o| {
                        if let Item::Post(p) = &*o.borrow::<Item>() {
                            Some(p.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                viewer::present(
                    &ui,
                    posts,
                    position as usize,
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
            Item::Message(_) => (),
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
        box_.append(&entry);
        box_.append(&send);
        let composer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        composer.add_css_class("message-composer");
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
            send_status.set_label("Sending…");send_status.set_visible(true);
            // Retain draft for any uncertain delivery; never automatically resend.
            let context=offline_id();
            *c.action_task.borrow_mut()=Some(app::background(async move {match route {
                Route::Thread(t)=>client.send_message(&t,&text,&context).await.map(|page|(None,page)),
                Route::Comments(p)=>client.comment(&p,&text).await.map(|c|(Some(Item::Comment(c)),None)),
                _=>Err(crate::viewfinder::Error::Unsupported),
            }},glib::clone!(#[weak] c,#[weak] ui,#[weak] entry,#[weak] send,#[weak] send_status,move |r|{
                entry.set_editable(true);send.set_sensitive(!entry.text().trim().is_empty());
                send.set_tooltip_text(Some(if is_dm { "Send" } else { "Post comment" }));
                match r {
                    Ok((Some(item),_))=>{send_status.set_visible(false);c.store.append(&glib::BoxedAnyObject::new(item));c.stack.set_visible_child_name("content");entry.set_text("");},
                    Ok((_,Some(page)))=>{
                        c.task.borrow_mut().take();
                        c.pagination.borrow_mut().reset();
                        c.replacing.set(true);
                        let generation=c.pagination.borrow().generation;
                        c.finish(generation,Ok(page));
                        entry.set_text("");
                        send_status.set_label("Sent");
                        entry.grab_focus();
                    },
                    Ok((None,None))=>{c.refresh();send_status.set_label("Delivery is unconfirmed. Check the conversation before retrying; your draft is saved.");},
                    Err(e)=>{tracing::warn!(kind=?e,"Send failed");if is_dm{send_status.set_label("Delivery could not be confirmed. Refresh before retrying; your draft is saved.");}else{send_status.set_visible(false);ui.notify(&e.to_string());}},
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

fn render_item(ui: &Rc<Ui>, item: &Item, route: &Route) -> gtk::Widget {
    if matches!(route, Route::Notifications | Route::Search(_))
        && let Some(row) = super::discovery::result_row(ui, item)
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
        Item::Message(m) => {
            let own = ui
                .client
                .borrow()
                .as_ref()
                .is_some_and(|c| c.account_id == m.sender);
            (
                if own {
                    "You".into()
                } else if let Route::Thread(t) = route {
                    t.participants
                        .iter()
                        .find(|u| u.id == m.sender)
                        .map(|u| u.username.clone())
                        .unwrap_or_else(|| "Participant".into())
                } else {
                    "Participant".into()
                },
                m.text.clone(),
                None,
            )
        }
        Item::Post(_) => unreachable!(),
    };
    if let Item::Message(m) = item {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let own = ui
            .client
            .borrow()
            .as_ref()
            .is_some_and(|c| c.account_id == m.sender);
        if !own && matches!(route, Route::Thread(t) if t.participants.len() > 1) {
            let sender = label(&title);
            sender.add_css_class("caption-heading");
            body.append(&sender);
        }
        if !subtitle.is_empty() {
            body.append(&label(&subtitle));
        }
        for attachment in &m.attachments {
            match attachment {
                Attachment::Media(m) => {
                    body.append(&message_media(ui, m));
                }
                Attachment::Post(post) => {
                    let b = gtk::Button::new();
                    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
                    card.append(&label(&format!("Post by {}", post.author.username)));
                    if let Some(media) = post.media.first() {
                        let preview = media::preview(ui.images.clone(), media, 400, false);
                        preview.set_size_request(220, 220);
                        card.append(&preview);
                    }
                    if !post.caption.is_empty() {
                        let caption = label(&post.caption);
                        caption.set_lines(3);
                        caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        card.append(&caption);
                    }
                    b.set_child(Some(&card));
                    b.connect_clicked(glib::clone!(
                        #[weak]
                        ui,
                        #[strong]
                        post,
                        move |_| viewer::present(&ui, vec![*post.clone()], 0, false)
                    ));
                    body.append(&b);
                }
                Attachment::Link { title, url } => {
                    if url.starts_with("https://") {
                        body.append(&gtk::LinkButton::with_label(url, title));
                    } else {
                        body.append(&label("Link unavailable"));
                    }
                }
                Attachment::Unavailable => body.append(&label("This attachment is unavailable")),
            }
        }
        body.add_css_class("message-bubble");
        body.set_tooltip_text(format::message_timestamp(m.timestamp).as_deref());
        if !m.seen_by.is_empty() {
            let seen = label(&format!("Seen by {}", m.seen_by.join(", ")));
            seen.add_css_class("caption");
            body.append(&seen);
        }
        let own = ui
            .client
            .borrow()
            .as_ref()
            .is_some_and(|c| c.account_id == m.sender);
        body.set_halign(if own {
            gtk::Align::End
        } else {
            gtk::Align::Start
        });
        if own {
            body.add_css_class("outgoing");
        }
        // Align bubbles within the same column as the composer. A second centered
        // clamp here pulls both sides of the conversation into the middle.
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.set_margin_start(20);
        row.set_margin_end(20);
        row.set_margin_top(1);
        row.set_margin_bottom(1);
        let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        space.set_hexpand(true);
        space.set_size_request(40, -1);
        if own {
            row.append(&space);
            row.append(&body);
        } else {
            row.append(&body);
            row.append(&space);
        }
        return row.upcast();
    }
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

fn message_media(ui: &Rc<Ui>, media: &Media) -> gtk::Widget {
    let widget = if media.video.is_some() {
        super::inline_video::new(ui, media)
    } else {
        let button = gtk::Button::new();
        let preview = media::preview(ui.images.clone(), media, 400, false);
        preview.set_size_request(220, 220);
        button.set_child(Some(&preview));
        button.add_css_class("flat");
        button.set_tooltip_text(Some("Open image"));
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
            move |_| viewer::present(&ui, vec![post.clone()], 0, false)
        ));
        button.upcast()
    };
    let clamp = adw::Clamp::builder()
        .maximum_size(300)
        .tightening_threshold(240)
        .child(&widget)
        .build();
    clamp.upcast()
}
