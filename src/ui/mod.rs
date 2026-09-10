mod collection;
mod discovery;
mod engagement;
mod format;
mod inline_video;
mod login;
mod media_frame;
mod reels;
#[cfg(test)]
mod review;
mod stories;
mod viewer;

use crate::{
    app::{self, Task},
    domain::*,
    viewfinder::{Client, session::Session},
    media::{Images, Playback},
};
use adw::prelude::*;
use collection::Collection;
use gtk::{gio, glib};
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub struct Ui {
    pub window: glib::WeakRef<adw::ApplicationWindow>,
    pub navigation: adw::NavigationView,
    pub toast: adw::ToastOverlay,
    pub client: RefCell<Option<Arc<Client>>>,
    pub images: Arc<Images>,
    pub playback: Rc<Playback>,
    pub collections: RefCell<Vec<Rc<Collection>>>,
    pub task: RefCell<Option<Task>>,
    pub login_stack: gtk::Stack,
    pub login_status: adw::StatusPage,
    pub destinations: adw::ViewStack,
    pub shell: adw::OverlaySplitView,
    pub drafts: RefCell<std::collections::HashMap<String, String>>,
    likes: RefCell<std::collections::HashMap<LikeTarget, Rc<engagement::LikeState>>>,
}
impl Ui {
    pub fn sidebar_toggle(&self) -> gtk::ToggleButton {
        let button = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Show navigation")
            .build();
        button.update_property(&[gtk::accessible::Property::Label("Show navigation")]);
        self.shell
            .bind_property("show-sidebar", &button, "active")
            .bidirectional()
            .sync_create()
            .build();
        self.shell
            .bind_property("collapsed", &button, "visible")
            .sync_create()
            .build();
        button
    }
    pub fn notify(&self, text: &str) {
        self.toast.add_toast(adw::Toast::new(text));
    }
    pub fn push(self: &Rc<Self>, route: Route) {
        self.playback.stop();
        let collection = Collection::new(self, route.clone());
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let refresh = icon_button("view-refresh-symbolic", "Refresh");
        refresh.set_action_name(Some("win.refresh"));
        header.pack_end(&refresh);
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&collection.root));
        let page = adw::NavigationPage::new(&toolbar, &route.title());
        collection.page.replace(Some(page.clone()));
        self.collections.borrow_mut().push(collection);
        self.navigation.push(&page);
    }
    fn install(self: &Rc<Self>, session: Session) {
        match Client::new(session) {
            Ok(client) => self.install_client(client),
            Err(e) => {
                self.login_status.set_description(Some(&e.to_string()));
            }
        }
    }
    fn install_client(self: &Rc<Self>, client: Arc<Client>) {
        self.likes.borrow_mut().clear();
        *self.client.borrow_mut() = Some(client);
        for collection in self.collections.borrow().iter() {
            collection.reset();
        }
        self.destinations.set_visible_child_name("home");
        self.login_stack.set_visible_child_name("app");
    }
    fn logout(self: &Rc<Self>) {
        self.task.borrow_mut().take();
        self.playback.stop();
        self.playback.clear_cache();
        self.client.borrow_mut().take();
        self.images.clear();
        self.drafts.borrow_mut().clear();
        self.likes.borrow_mut().clear();
        self.login_stack.set_visible_child_name("login");
        while self.navigation.pop() {}
        for c in self.collections.borrow().iter() {
            c.reset();
        }
        *self.task.borrow_mut() = Some(app::background(
            Session::forget(),
            glib::clone!(
                #[weak(rename_to=ui)]
                self,
                move |result| {
                    if result.is_err() {
                        ui.notify(
                            "Signed out here, but the saved keyring session could not be removed.",
                        );
                    }
                }
            ),
        ));
    }
}

#[cfg(not(test))]
async fn restore_client() -> Result<Arc<Client>, crate::viewfinder::Error> {
    let session = Session::restore().await?;
    let client = Client::new(session)?;
    client.profile(&client.account_id).await?;
    Ok(client)
}

pub fn activate(application: &adw::Application) {
    gtk::Window::set_default_icon_name("io.github._6E6B.viewfinder");
    let icons = gio::Resource::from_data(&glib::Bytes::from_static(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/icons.gresource"
    ))))
    .expect("bundled icons");
    gio::resources_register(&icons);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_resource_path("/io/github/_6E6B/viewfinder/icons");
    }
    if let Some(window) = application.active_window() {
        window.present();
        return;
    }
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Viewfinder")
        .default_width(1200)
        .default_height(820)
        .build();
    window.set_size_request(360, 400);
    let css = gtk::CssProvider::new();
    css.load_from_string(include_str!("style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::prelude::WidgetExt::display(&window),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let toast = adw::ToastOverlay::new();
    let navigation = adw::NavigationView::new();
    let login_stack = gtk::Stack::new();
    let login_status = adw::StatusPage::builder()
        .icon_name("avatar-default-symbolic")
        .title("Sign in to Viewfinder")
        .build();
    let destinations = adw::ViewStack::new();
    let shell = adw::OverlaySplitView::builder()
        .min_sidebar_width(200.0)
        .max_sidebar_width(240.0)
        .sidebar_width_fraction(0.20)
        .build();
    let ui = Rc::new(Ui {
        window: window.downgrade(),
        navigation: navigation.clone(),
        toast: toast.clone(),
        client: RefCell::new(None),
        images: Images::new(),
        playback: Playback::new(),
        collections: RefCell::new(Vec::new()),
        task: RefCell::new(None),
        login_stack: login_stack.clone(),
        login_status: login_status.clone(),
        destinations: destinations.clone(),
        shell: shell.clone(),
        drafts: RefCell::new(Default::default()),
        likes: RefCell::new(Default::default()),
    });
    let login_actions = gtk::Box::new(gtk::Orientation::Vertical, 12);
    login_actions.set_halign(gtk::Align::Center);
    let sign_in = gtk::Button::with_label("Sign In");
    sign_in.add_css_class("suggested-action");
    sign_in.add_css_class("pill");
    login_actions.append(&sign_in);
    login_status.set_child(Some(&login_actions));
    let login_toolbar = adw::ToolbarView::new();
    login_toolbar.add_top_bar(&adw::HeaderBar::new());
    login_toolbar.set_content(Some(&login_status));
    login_stack.add_named(&login_toolbar, Some("login"));
    sign_in.connect_clicked(glib::clone!(
        #[weak]
        ui,
        #[weak]
        sign_in,
        move |_| {
            ui.task.borrow_mut().take();
            sign_in.set_sensitive(false);
            let dialog = login::open(&ui);
            dialog.connect_closed(glib::clone!(
                #[weak]
                sign_in,
                move |_| {
                    sign_in.set_sensitive(true);
                }
            ));
        }
    ));
    #[cfg(not(test))]
    {
        *ui.task.borrow_mut() = Some(app::background(
            restore_client(),
            glib::clone!(
                #[weak]
                ui,
                move |result| {
                    if let Ok(client) = result {
                        ui.install_client(client);
                    }
                }
            ),
        ));
    }

    let sidebar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Viewfinder", "")));
    let search = icon_button("system-search-symbolic", "Search");
    search.set_action_name(Some("win.search"));
    header.pack_start(&search);
    let menu = gio::Menu::new();
    menu.append(Some("Notifications"), Some("win.notifications"));
    menu.append(Some("Refresh"), Some("win.refresh"));
    menu.append(Some("Sign Out"), Some("win.logout"));
    menu.append(Some("About Viewfinder"), Some("win.about"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Main menu")
        .menu_model(&menu)
        .build();
    header.pack_end(&menu_button);
    sidebar.add_top_bar(&header);
    let destinations_list = gtk::ListBox::new();
    destinations_list.add_css_class("navigation-sidebar");
    destinations_list.set_widget_name("app-navigation");
    destinations_list.set_selection_mode(gtk::SelectionMode::Single);
    sidebar.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&destinations_list)
            .build(),
    ));
    shell.set_sidebar(Some(&sidebar));
    let breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 1050sp").expect("breakpoint"),
    );
    breakpoint.add_setter(&shell, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(breakpoint);
    for (name, title, icon, route) in [
        ("home", "Home", "go-home-symbolic", Route::Home),
        (
            "reels",
            "Reels",
            "media-playback-start-symbolic",
            Route::Reels,
        ),
        ("messages", "Messages", "mail-unread-symbolic", Route::Inbox),
        (
            "profile",
            "Profile",
            "avatar-default-symbolic",
            Route::Profile(User::default()),
        ),
    ] {
        let is_inbox = matches!(route, Route::Inbox);
        let collection = Collection::new(&ui, route);
        if is_inbox {
            destinations.add_titled_with_icon(&collection.root, Some(name), title, icon);
        } else {
            let toolbar = adw::ToolbarView::new();
            let header = adw::HeaderBar::new();
            header.set_title_widget(Some(&adw::WindowTitle::new(title, "")));
            header.pack_start(&ui.sidebar_toggle());
            let refresh = icon_button("view-refresh-symbolic", "Refresh");
            refresh.set_action_name(Some("win.refresh"));
            header.pack_end(&refresh);
            toolbar.add_top_bar(&header);
            toolbar.set_content(Some(&collection.root));
            destinations.add_titled_with_icon(&toolbar, Some(name), title, icon);
        }
        let row = gtk::ListBoxRow::new();
        row.set_widget_name(name);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.append(&gtk::Image::from_icon_name(icon));
        content.append(&gtk::Label::new(Some(title)));
        row.set_child(Some(&content));
        destinations_list.append(&row);
        ui.collections.borrow_mut().push(collection);
    }
    destinations_list.select_row(destinations_list.row_at_index(0).as_ref());
    destinations_list.connect_row_activated(glib::clone!(
        #[weak]
        ui,
        move |_, row| {
            ui.playback.stop();
            ui.destinations.set_visible_child_name(&row.widget_name());
            if ui.shell.is_collapsed() {
                ui.shell.set_show_sidebar(false);
            }
        }
    ));
    destinations.connect_visible_child_notify(glib::clone!(
        #[weak]
        destinations_list,
        move |stack| {
            let Some(name) = stack.visible_child_name() else {
                return;
            };
            let mut index = 0;
            while let Some(row) = destinations_list.row_at_index(index) {
                if row.widget_name() == name {
                    destinations_list.select_row(Some(&row));
                    break;
                }
                index += 1;
            }
        }
    ));
    shell.set_content(Some(&destinations));
    navigation.add(&adw::NavigationPage::new(&shell, "Viewfinder"));
    navigation.connect_popped(glib::clone!(
        #[weak]
        ui,
        move |_, page| {
            ui.playback.stop();
            ui.collections
                .borrow_mut()
                .retain(|c| c.page.borrow().as_ref() != Some(page));
        }
    ));
    login_stack.add_named(&navigation, Some("app"));
    login_stack.set_visible_child_name("login");
    toast.set_child(Some(&login_stack));
    window.set_content(Some(&toast));
    for (name, callback) in [
        ("search", search_dialog as fn(&Rc<Ui>)),
        ("refresh", refresh as fn(&Rc<Ui>)),
        ("notifications", notifications as fn(&Rc<Ui>)),
        ("logout", sign_out as fn(&Rc<Ui>)),
        ("back", back as fn(&Rc<Ui>)),
        ("about", about as fn(&Rc<Ui>)),
    ] {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(glib::clone!(
            #[weak]
            ui,
            move |_, _| callback(&ui)
        ));
        window.add_action(&action);
    }
    // Window owns application presentation state; state only holds a weak window.
    #[cfg(test)]
    review::remember(&ui);
    window.connect_close_request(move |_| {
        ui.playback.stop();
        ui.task.borrow_mut().take();
        ui.collections.borrow_mut().clear();
        glib::Propagation::Proceed
    });
    window.present();
}
fn notifications(ui: &Rc<Ui>) {
    if ui.client.borrow().is_some() {
        ui.push(Route::Notifications);
    }
}
fn sign_out(ui: &Rc<Ui>) {
    ui.logout();
}
fn back(ui: &Rc<Ui>) {
    if ui
        .collections
        .borrow()
        .iter()
        .rev()
        .any(|c| c.root.is_mapped() && c.back())
    {
        return;
    }
    ui.navigation.pop();
}
fn refresh(ui: &Rc<Ui>) {
    if let Some(c) = ui
        .collections
        .borrow()
        .iter()
        .rev()
        .find(|c| c.root.is_mapped())
    {
        c.refresh();
    }
}
fn about(ui: &Rc<Ui>) {
    let dialog = adw::AboutDialog::builder()
        .application_name("Viewfinder")
        .application_icon("io.github._6E6B.viewfinder")
        .developer_name("Nick")
        .version(env!("CARGO_PKG_VERSION"))
        .comments("An independent native GNOME client. Not affiliated with Meta.")
        .website("https://github.com/6E6B/viewfinder")
        .issue_url("https://github.com/6E6B/viewfinder/issues")
        .license_type(gtk::License::Gpl30)
        .build();
    dialog.present(ui.window.upgrade().as_ref());
}
pub fn icon_button(icon: &str, label: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(label)
        .build();
    b.update_property(&[gtk::accessible::Property::Label(label)]);
    b
}
pub fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .build()
}
fn search_dialog(ui: &Rc<Ui>) {
    if ui.client.borrow().is_none() {
        return;
    }
    present_search(ui);
}
fn present_search(ui: &Rc<Ui>) {
    let dialog = adw::Dialog::builder()
        .title("Search")
        .content_width(480)
        .content_height(220)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    for setter in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        setter(box_.upcast_ref(), 24);
    }
    let entry = gtk::SearchEntry::builder()
        .placeholder_text("Search Viewfinder")
        .hexpand(true)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label("Search Viewfinder")]);
    let kind = gtk::DropDown::from_strings(&["Accounts", "Posts"]);
    kind.update_property(&[gtk::accessible::Property::Label("Search type")]);
    let search = gtk::Button::with_label("Search");
    search.add_css_class("suggested-action");
    search.set_sensitive(false);
    entry.connect_changed(glib::clone!(
        #[weak]
        search,
        move |entry| search.set_sensitive(!entry.text().trim().is_empty())
    ));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    actions.append(&kind);
    let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    space.set_hexpand(true);
    actions.append(&space);
    actions.append(&search);
    box_.append(&entry);
    box_.append(&actions);
    toolbar.set_content(Some(&box_));
    dialog.set_child(Some(&toolbar));
    search.connect_clicked(glib::clone!(
        #[weak]
        ui,
        #[weak]
        dialog,
        #[weak]
        entry,
        #[weak]
        kind,
        move |_| {
            let query = entry.text().trim().to_owned();
            if query.is_empty() {
                return;
            }
            dialog.close();
            ui.push(if kind.selected() == 0 {
                Route::Search(query)
            } else {
                Route::Keyword(query)
            });
        }
    ));
    entry.connect_activate(glib::clone!(
        #[weak]
        search,
        move |_| {
            if search.is_sensitive() {
                search.emit_clicked();
            }
        }
    ));
    dialog.present(ui.window.upgrade().as_ref());
    entry.grab_focus();
}
