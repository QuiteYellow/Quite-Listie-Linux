//! The main application window — GNOME counterpart of `qml/Main.qml`. An
//! [`adw::OverlaySplitView`] (so the sidebar can be shown/hidden, mirroring Swift's
//! NavigationSplitView toggle) with a sidebar pane (list index + smart boxes) and a
//! content pane holding an [`adw::NavigationView`] page stack.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::controller::Controller;
use crate::models::{self, ListItemModel, SidebarItem};
use crate::views;

mod imp {
    use super::*;
    use std::cell::OnceCell;

    pub struct QuiteListieWindow {
        pub controller: Controller,
        pub sidebar_store: gio::ListStore,
        pub list_model: Rc<ListItemModel>,
        pub content: OnceCell<adw::NavigationView>,
        pub split: OnceCell<adw::OverlaySplitView>,
        pub toasts: OnceCell<adw::ToastOverlay>,
        /// id of the list page currently shown (so we replace rather than stack).
        pub open_list_page: RefCell<Option<adw::NavigationPage>>,
        /// Smart-box cards [Today, Scheduled, Locations] for the active-selection highlight.
        pub smart_cards: OnceCell<Vec<gtk::Button>>,
        /// Sidebar list selection, so opening a smart box can clear the list-row highlight.
        pub sidebar_selection: OnceCell<gtk::SingleSelection>,
    }

    impl Default for QuiteListieWindow {
        fn default() -> Self {
            let controller = Controller::new();
            let list_model = ListItemModel::new(controller.provider());
            Self {
                controller,
                sidebar_store: gio::ListStore::new::<SidebarItem>(),
                list_model,
                content: OnceCell::new(),
                split: OnceCell::new(),
                toasts: OnceCell::new(),
                open_list_page: RefCell::new(None),
                smart_cards: OnceCell::new(),
                sidebar_selection: OnceCell::new(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for QuiteListieWindow {
        const NAME: &'static str = "QuiteListieWindow";
        type Type = super::QuiteListieWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for QuiteListieWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.build_ui();
            obj.connect_signals();
            // Kick off the initial load (disk cache → network).
            self.controller.refresh_lists();
        }
    }

    impl WidgetImpl for QuiteListieWindow {}
    impl WindowImpl for QuiteListieWindow {}
    impl ApplicationWindowImpl for QuiteListieWindow {}
    impl AdwApplicationWindowImpl for QuiteListieWindow {}
}

glib::wrapper! {
    pub struct QuiteListieWindow(ObjectSubclass<imp::QuiteListieWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl QuiteListieWindow {
    pub fn new(app: &impl IsA<gtk::Application>) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    fn controller(&self) -> &Controller {
        &self.imp().controller
    }

    /// A sidebar show/hide toggle bound to the split's `show-sidebar` (Swift
    /// NavigationSplitView's sidebar toggle). Built fresh per header bar — a widget has a
    /// single parent — with every instance kept in sync through the shared property.
    fn sidebar_toggle(&self) -> gtk::ToggleButton {
        let btn = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle Sidebar")
            .build();
        if let Some(split) = self.imp().split.get() {
            split
                .bind_property("show-sidebar", &btn, "active")
                .sync_create()
                .bidirectional()
                .build();
        }
        btn
    }

    /// Route a file/URI handed to the app via `HANDLES_OPEN`. `quitelistie://`
    /// and legacy `listie://` deeplinks go through the controller's decoder;
    /// anything with a local path is opened as an external `.listie` file.
    pub fn open_file(&self, file: &gio::File) {
        let uri = file.uri();
        if uri.starts_with("quitelistie://") || uri.starts_with("listie://") {
            self.controller().handle_deeplink(&uri);
        } else if let Some(path) = file.path() {
            self.controller().open_external_file(&path.to_string_lossy());
        }
    }

    fn build_ui(&self) {
        let imp = self.imp();
        self.set_title(Some("Quite Listie"));
        self.set_default_size(960, 680);
        self.set_width_request(360);
        self.set_height_request(480);
        install_css();

        // Create the split first (empty) so the header bars built below can bind their
        // sidebar toggles to its `show-sidebar` property.
        let split = adw::OverlaySplitView::builder()
            .min_sidebar_width(280.0)
            .max_sidebar_width(400.0)
            .build();
        imp.split.set(split.clone()).ok();

        // --- sidebar pane --------------------------------------------------
        let sidebar_page = self.build_sidebar();

        // --- content pane --------------------------------------------------
        let content_nav = adw::NavigationView::new();
        content_nav.add(&self.welcome_page());
        let content_page = adw::NavigationPage::builder()
            .title("Quite Listie")
            .child(&content_nav)
            .build();

        split.set_sidebar(Some(&sidebar_page));
        split.set_content(Some(&content_page));

        // Collapse to an overlay sidebar on narrow widths. The threshold must clear the
        // expanded minimum (sidebar min 280 + content min): below it OverlaySplitView
        // would keep both panes side by side and squeeze the content under its min width.
        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            560.0,
            adw::LengthUnit::Sp,
        ));
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        // Narrow: hide the sidebar to an overlay (revealed by the header toggle). The
        // setter's value is restored when the breakpoint no longer applies, so widening
        // back to desktop brings the sidebar back.
        breakpoint.add_setter(&split, "show-sidebar", Some(&false.to_value()));
        self.add_breakpoint(breakpoint);

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));
        self.set_content(Some(&toasts));

        imp.content.set(content_nav).ok();
        imp.toasts.set(toasts).ok();
    }

    fn welcome_page(&self) -> adw::NavigationPage {
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.pack_start(&self.sidebar_toggle());
        toolbar.add_top_bar(&header);
        let status = adw::StatusPage::builder()
            .icon_name("view-list-symbolic")
            .title("Quite Listie")
            .description("Select a list from the sidebar, or create a new one.")
            .build();

        // K4: until an account is connected, list creation is unavailable — offer a
        // "Connect to Nextcloud" call to action instead, mirroring the KDE placeholder.
        let connect = gtk::Button::builder()
            .label("Connect to Nextcloud to get started")
            .halign(gtk::Align::Center)
            .build();
        connect.add_css_class("pill");
        connect.add_css_class("suggested-action");
        connect.connect_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| views::nextcloud_setup::present(&win, win.controller())
        ));
        // Shown only while unauthenticated.
        self.controller()
            .bind_property("is-nc-authenticated", &connect, "visible")
            .sync_create()
            .invert_boolean()
            .build();
        status.set_child(Some(&connect));

        toolbar.set_content(Some(&status));
        adw::NavigationPage::builder()
            .title("Quite Listie")
            .tag("welcome")
            .child(&toolbar)
            .build()
    }

    /// Install the "Open" menu actions (local file + Nextcloud connect).
    fn install_open_actions(&self) {
        let open_file = gio::SimpleAction::new("open-file", None);
        open_file.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _| win.present_open_file_dialog()
        ));
        self.add_action(&open_file);

        let connect_nc = gio::SimpleAction::new("connect-nextcloud", None);
        connect_nc.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _| views::nextcloud_setup::present(&win, win.controller())
        ));
        self.add_action(&connect_nc);

        // Browse opens the file browser when connected; otherwise route to setup first.
        let browse_nc = gio::SimpleAction::new("browse-nextcloud", None);
        browse_nc.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _| {
                let controller = win.controller();
                if controller.is_nc_authenticated() {
                    views::nextcloud_browser::present(&win, controller);
                } else {
                    views::nextcloud_setup::present(&win, controller);
                }
            }
        ));
        self.add_action(&browse_nc);

        let import_md = gio::SimpleAction::new("import-markdown", None);
        import_md.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _| views::markdown_import::present(&win, win.controller())
        ));
        self.add_action(&import_md);
    }

    /// Install the primary-menu actions (Preferences, About) + their accelerators.
    fn install_app_actions(&self) {
        let refresh = gio::SimpleAction::new("refresh", None);
        refresh.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _| win.controller().refresh_lists()
        ));
        self.add_action(&refresh);

        let preferences = gio::SimpleAction::new("preferences", None);
        preferences.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _| views::settings::present(&win, win.controller())
        ));
        self.add_action(&preferences);

        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _| win.present_about()
        ));
        self.add_action(&about);

        if let Some(app) = self.application() {
            app.set_accels_for_action("win.preferences", &["<Primary>comma"]);
        }
    }

    /// Show the GNOME-standard about dialog (app info + open-source acknowledgements).
    fn present_about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Quite Listie")
            .application_icon(crate::APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .developer_name("Quite Yellow")
            .license_type(gtk::License::Gpl30)
            .website("https://github.com/QuiteYellow/Quite-Listie")
            .build();
        about.add_acknowledgement_section(
            Some("Built with"),
            &[
                "GTK https://gtk.org",
                "libadwaita https://gnome.pages.gitlab.gnome.org/libadwaita/",
                "libshumate https://gitlab.gnome.org/GNOME/libshumate",
                "pulldown-cmark https://github.com/pulldown-cmark/pulldown-cmark",
            ],
        );
        about.present(Some(self));
    }

    /// Pick a local `.listie` file and open it via the controller.
    fn present_open_file_dialog(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Listie files"));
        filter.add_pattern("*.listie");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);

        let dialog = gtk::FileDialog::builder()
            .title("Open List")
            .filters(&filters)
            .modal(true)
            .build();

        dialog.open(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            win.controller().open_external_file(&path.to_string_lossy());
                        }
                    }
                }
            ),
        );
    }

    // -----------------------------------------------------------------------
    // Sidebar
    // -----------------------------------------------------------------------

    fn build_sidebar(&self) -> adw::NavigationPage {
        let imp = self.imp();
        let toolbar = adw::ToolbarView::new();

        let header = adw::HeaderBar::new();

        header.pack_start(&self.sidebar_toggle());

        let new_button = gtk::Button::from_icon_name("list-add-symbolic");
        new_button.set_tooltip_text(Some("New List"));
        new_button.connect_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| win.present_new_list_dialog()
        ));
        // New List now creates a local list (Swift NewListView), so it's always available
        // — no Nextcloud account required. (Nextcloud lists are still created in the
        // browser via "New List Here".)
        header.pack_start(&new_button);

        // Open menu: local .listie files + Nextcloud browse/account.
        let open_menu = gio::Menu::new();
        open_menu.append(Some("Open from Files…"), Some("win.open-file"));
        open_menu.append(Some("Open from Nextcloud…"), Some("win.browse-nextcloud"));
        open_menu.append(Some("Import Markdown…"), Some("win.import-markdown"));
        open_menu.append(Some("Nextcloud Account…"), Some("win.connect-nextcloud"));
        let open_button = gtk::MenuButton::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Open")
            .menu_model(&open_menu)
            .build();
        header.pack_end(&open_button);

        // Primary (hamburger) menu. Order mirrors the Swift SidebarView ellipsis menu
        // (Settings, then Refresh); About stays last per GNOME convention.
        let primary_menu = gio::Menu::new();
        let window_section = gio::Menu::new();
        window_section.append(Some("New Window"), Some("app.new-window"));
        primary_menu.append_section(None, &window_section);
        let prefs_section = gio::Menu::new();
        prefs_section.append(Some("Preferences"), Some("win.preferences"));
        primary_menu.append_section(None, &prefs_section);
        let refresh_section = gio::Menu::new();
        refresh_section.append(Some("Refresh"), Some("win.refresh"));
        primary_menu.append_section(None, &refresh_section);
        let about_section = gio::Menu::new();
        about_section.append(Some("About Quite Listie"), Some("win.about"));
        primary_menu.append_section(None, &about_section);
        let primary_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Main Menu")
            .menu_model(&primary_menu)
            .primary(true)
            .build();
        header.pack_end(&primary_button);

        self.install_open_actions();
        self.install_app_actions();
        toolbar.add_top_bar(&header);

        // Smart boxes (Today / Scheduled / Locations) as cards: Today + Scheduled sit
        // side by side, Locations spans full width below — Swift SidebarView's ViewThatFits
        // 2-up grid plus the full-width Locations card. Counts bound to the controller;
        // visibility bound to the `hide-*-card` settings (toggled from Preferences).
        let smart = gtk::Box::new(gtk::Orientation::Vertical, 6);
        smart.set_margin_top(6);
        smart.set_margin_start(6);
        smart.set_margin_end(6);
        smart.set_margin_bottom(8);
        let settings = self.controller().settings().clone();

        let today = self.smart_card("Today", "today-count", "alarm-symbolic", "ql-smart-today");
        let scheduled =
            self.smart_card("Scheduled", "scheduled-count", "x-office-calendar-symbolic", "ql-smart-scheduled");
        let locations =
            self.smart_card("Locations", "location-count", "mark-location-symbolic", "ql-smart-locations");

        today.connect_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| win.show_reminder_page(true)
        ));
        scheduled.connect_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| win.show_reminder_page(false)
        ));
        locations.connect_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| win.show_global_map()
        ));

        settings.bind("hide-today-card", &today, "visible").invert_boolean().build();
        settings.bind("hide-scheduled-card", &scheduled, "visible").invert_boolean().build();
        settings.bind("hide-locations-card", &locations, "visible").invert_boolean().build();

        let pair = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        pair.set_homogeneous(true);
        pair.append(&today);
        pair.append(&scheduled);
        smart.append(&pair);
        smart.append(&locations);
        imp.smart_cards.set(vec![today, scheduled, locations]).ok();

        // List index.
        let selection = gtk::SingleSelection::builder()
            .model(&imp.sidebar_store)
            .autoselect(false)
            .can_unselect(true)
            .build();
        imp.sidebar_selection.set(selection.clone()).ok();
        let factory = sidebar_factory(self);
        let list_view = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list_view.add_css_class("navigation-sidebar");
        // Open on selection change (single click selects), rather than `activate`. Using
        // single-click-activate suppressed the persistent `:selected` highlight, so the
        // open list wasn't indicated. The current-id guard stops the programmatic
        // re-selection in `sync_sidebar_selection` from re-triggering an open.
        selection.connect_selection_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[weak]
            selection,
            move |_, _, _| {
                if let Some(item) = selection.selected_item().and_downcast::<SidebarItem>() {
                    if !item.is_header() && win.controller().current_list_id() != item.list_id() {
                        win.controller().select_list(&item.list_id());
                    }
                }
            }
        ));

        let scroller = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list_view)
            .build();

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&smart);
        body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        body.append(&scroller);
        toolbar.set_content(Some(&body));

        adw::NavigationPage::builder()
            .title("Lists")
            .child(&toolbar)
            .build()
    }

    /// A smart-box card (Swift `ReminderSmartBox`): a coloured icon + live count on the top
    /// row and the title below, on a rounded card surface. `accent_class` carries the
    /// per-box icon colour (orange / blue / green).
    fn smart_card(&self, title: &str, count_prop: &str, icon: &str, accent_class: &str) -> gtk::Button {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let image = gtk::Image::from_icon_name(icon);
        image.set_pixel_size(22);
        image.add_css_class("ql-smart-icon");
        top.append(&image);
        let count = gtk::Label::builder().label("0").hexpand(true).xalign(1.0).build();
        count.add_css_class("ql-smart-count");
        self.controller()
            .bind_property(count_prop, &count, "label")
            .sync_create()
            .build();
        top.append(&count);
        content.append(&top);

        let label = gtk::Label::builder().label(title).xalign(0.0).build();
        label.add_css_class("ql-smart-title");
        label.add_css_class("dim-label");
        content.append(&label);

        let button = gtk::Button::builder().child(&content).build();
        button.add_css_class("ql-smart-card");
        button.add_css_class(accent_class);
        button
    }

    // -----------------------------------------------------------------------
    // Signal wiring
    // -----------------------------------------------------------------------

    fn connect_signals(&self) {
        let controller = self.controller();

        controller.connect_lists_updated(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |c| {
                models::populate_sidebar(&win.imp().sidebar_store, c);
                // Keep the open list's row highlighted after the store is rebuilt.
                win.sync_sidebar_selection();
            }
        ));

        controller.connect_current_list_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |c| win.show_current_list(c)
        ));

        controller.connect_current_list_externally_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| win.imp().list_model.reload()
        ));

        // List settings saved — rebuild the open page (title/icon/background changed).
        controller.connect_list_settings_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |c| {
                if !c.current_list_id().is_empty() {
                    win.show_current_list(c);
                }
            }
        ));

        controller.connect_error_occurred(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, msg| win.toast(&msg)
        ));

        // `quitelistie://import` deeplink — open the import dialog pre-filled.
        controller.connect_deeplink_import(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |c, target, md, _preview| {
                views::markdown_import::present_prefilled(&win, c, &md, &target);
            }
        ));

        // `quitelistie://item` deeplink — select the list, then open the editor
        // once `show_current_list` has built the page and pointed the model at it.
        controller.connect_deeplink_item(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |c, list_id, item_id| {
                c.select_list(&list_id);
                let win_weak = win.downgrade();
                glib::idle_add_local_once(move || {
                    let Some(win) = win_weak.upgrade() else { return };
                    let imp = win.imp();
                    let page = imp.open_list_page.borrow().clone();
                    if let Some(page) = page {
                        views::item_editor::open(&page, imp.list_model.clone(), &item_id);
                    }
                });
            }
        ));

        // Preferences changed live — re-render the affected UI. (Display toggles rebuild
        // the open list; the welcome-list toggle repopulates the sidebar.)
        controller.settings().connect_changed(
            None,
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |_, key| match key {
                    "show-completed-at-bottom" | "hide-empty-labels" => {
                        let c = win.controller();
                        if !c.current_list_id().is_empty() {
                            win.show_current_list(&c);
                        }
                    }
                    "hide-welcome-list" => {
                        models::populate_sidebar(&win.imp().sidebar_store, &win.controller());
                    }
                    _ => {}
                }
            ),
        );
    }

    /// Highlight the active smart card (Swift `selectedListID == "__reminders_*"/"__map"`).
    /// `which` is the index into `smart_cards` (0=Today, 1=Scheduled, 2=Locations), or
    /// `None` when a regular list / the welcome page is showing. Selecting a smart box also
    /// clears the sidebar list-row selection (Swift's single `selectedListID`).
    fn set_smart_selected(&self, which: Option<usize>) {
        let imp = self.imp();
        if let Some(cards) = imp.smart_cards.get() {
            for (i, card) in cards.iter().enumerate() {
                if Some(i) == which {
                    card.add_css_class("ql-smart-selected");
                } else {
                    card.remove_css_class("ql-smart-selected");
                }
            }
        }
        if which.is_some() {
            if let Some(sel) = imp.sidebar_selection.get() {
                sel.set_selected(gtk::INVALID_LIST_POSITION);
            }
        }
    }

    /// Highlight the sidebar row of the open list, or clear the selection when a smart /
    /// welcome page is showing. Re-run after the store is rebuilt so the active-list
    /// indicator stays on the right row (positions shift on repopulate).
    fn sync_sidebar_selection(&self) {
        let imp = self.imp();
        let Some(sel) = imp.sidebar_selection.get() else { return };
        let id = imp.controller.current_list_id();
        if !id.is_empty() {
            let n = imp.sidebar_store.n_items();
            for i in 0..n {
                if let Some(item) = imp.sidebar_store.item(i).and_downcast::<SidebarItem>() {
                    if !item.is_header() && item.list_id() == id {
                        sel.set_selected(i);
                        return;
                    }
                }
            }
        }
        sel.set_selected(gtk::INVALID_LIST_POSITION);
    }

    /// React to a list selection: point the model at it and swap in a list page.
    fn show_current_list(&self, controller: &Controller) {
        let imp = self.imp();
        let id = controller.current_list_id();
        let Some(content) = imp.content.get() else { return };
        self.set_smart_selected(None);

        if id.is_empty() {
            // Cleared selection — return to the welcome page (no slide animation), and
            // fall back to the sidebar when collapsed.
            content.replace(std::slice::from_ref(&self.welcome_page()));
            *imp.open_list_page.borrow_mut() = None;
            // When collapsed, fall back to the (overlay) sidebar; on desktop leave the
            // user's manual sidebar visibility untouched.
            if let Some(split) = imp.split.get() {
                if split.is_collapsed() {
                    split.set_show_sidebar(true);
                }
            }
            return;
        }

        imp.list_model.set_list_id(&id);
        let title = controller.list_name(&id);
        let page = if controller.list_view_mode(&id) == "kanban" {
            views::kanban_page::build(controller, imp.list_model.clone(), &title, &self.sidebar_toggle())
        } else {
            views::list_page::build(controller, imp.list_model.clone(), &title, &self.sidebar_toggle())
        };

        // Swap the list page in as the content root. `replace` (unlike `push`) is not
        // animated, so the page doesn't slide in from the right, and navigation stays
        // one-deep (the item editor still pushes onto this stack as a drill-in).
        content.replace(std::slice::from_ref(&page));
        *imp.open_list_page.borrow_mut() = Some(page);
        self.sync_sidebar_selection();

        // In collapsed (narrow) mode the split shows one pane at a time, defaulting to the
        // sidebar — bring the content pane forward so the tapped list actually appears.
        if let Some(split) = imp.split.get() {
            if split.is_collapsed() {
                split.set_show_sidebar(false);
            }
        }
    }

    /// Show the cross-list reminder page (Today or Scheduled smart box) as the content
    /// root. Like a list page it `replace`s the stack (no slide) and brings the content
    /// pane forward when collapsed.
    fn show_reminder_page(&self, today_only: bool) {
        let imp = self.imp();
        let Some(content) = imp.content.get() else { return };
        // No list is "current" while a smart page is open, so a later sidebar rebuild
        // doesn't re-highlight a stale row.
        imp.controller.set_current_list_id(String::new());
        self.set_smart_selected(Some(if today_only { 0 } else { 1 }));
        let page = views::reminder_page::build(&self.controller(), today_only, &self.sidebar_toggle());
        content.replace(std::slice::from_ref(&page));
        *imp.open_list_page.borrow_mut() = None;
        if let Some(split) = imp.split.get() {
            if split.is_collapsed() {
                split.set_show_sidebar(false);
            }
        }
    }

    /// Show the global map (Locations smart box) as the content root.
    fn show_global_map(&self) {
        let imp = self.imp();
        let Some(content) = imp.content.get() else { return };
        imp.controller.set_current_list_id(String::new());
        self.set_smart_selected(Some(2));
        let page = views::global_map::build(&self.controller(), &self.sidebar_toggle());
        content.replace(std::slice::from_ref(&page));
        *imp.open_list_page.borrow_mut() = None;
        if let Some(split) = imp.split.get() {
            if split.is_collapsed() {
                split.set_show_sidebar(false);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Dialogs
    // -----------------------------------------------------------------------

    fn present_new_list_dialog(&self) {
        let dialog = adw::AlertDialog::new(
            Some("New Local List"),
            Some("Stored on this device. To create one on Nextcloud, use Open → Open from Nextcloud → New List Here."),
        );

        // Name + an emoji icon picker (Swift NewListView's title + SymbolPicker; the GNOME
        // app uses emoji glyphs as list icons, as the label editor does).
        let entry = gtk::Entry::builder().placeholder_text("List name").hexpand(true).build();
        let emoji_value = Rc::new(RefCell::new(String::new()));
        let emoji_button = gtk::MenuButton::builder().label("Icon…").valign(gtk::Align::Center).build();
        let chooser = gtk::EmojiChooser::new();
        emoji_button.set_popover(Some(&chooser));
        chooser.connect_emoji_picked(glib::clone!(
            #[weak]
            emoji_button,
            #[strong]
            emoji_value,
            move |_, emoji| {
                *emoji_value.borrow_mut() = emoji.to_string();
                emoji_button.set_label(emoji);
            }
        ));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&entry);
        row.append(&emoji_button);
        dialog.set_extra_child(Some(&row));

        dialog.add_response("cancel", "Cancel");
        dialog.add_response("create", "Create");
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("create"));
        dialog.set_close_response("cancel");

        // Shared create action used by both the Create button and the Enter key.
        let do_create = glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[weak]
            entry,
            #[strong]
            emoji_value,
            move || {
                let name = entry.text();
                let name = name.trim();
                if !name.is_empty() {
                    win.controller().create_local_list(name, &emoji_value.borrow());
                }
            }
        );

        entry.connect_activate(glib::clone!(
            #[weak]
            dialog,
            #[strong]
            do_create,
            move |_| {
                do_create();
                dialog.close();
            }
        ));

        dialog.connect_response(
            None,
            move |_, response| {
                if response == "create" {
                    do_create();
                }
            },
        );
        dialog.present(Some(self));
    }

    /// Build the right-click menu for a sidebar list row (Swift `SidebarView.listRow`
    /// context menu): Favourite/Unfavourite, List Settings, Close List, Delete List.
    fn sidebar_context_menu(&self, item: &SidebarItem, anchor: &gtk::Box) -> gtk::Popover {
        let controller = self.controller();
        let list_id = item.list_id();
        let popover = gtk::Popover::new();
        let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let is_fav = controller.is_favourite(&list_id);
        let fav_btn = flat_sidebar_button(if is_fav { "Unfavourite" } else { "Favourite" });
        let settings_btn = flat_sidebar_button("List Settings…");
        let close_btn = flat_sidebar_button("Close List");
        let delete_btn = flat_sidebar_button("Delete List…");
        delete_btn.add_css_class("destructive-action");
        menu_box.append(&fav_btn);
        menu_box.append(&settings_btn);
        menu_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu_box.append(&close_btn);
        menu_box.append(&delete_btn);
        popover.set_child(Some(&menu_box));

        fav_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[weak]
            popover,
            #[strong]
            list_id,
            move |_| {
                popover.popdown();
                let controller = win.controller();
                controller.set_favourite(&list_id, !controller.is_favourite(&list_id));
                // Favourites are a sidebar section — re-section so the row moves.
                models::populate_sidebar(&win.imp().sidebar_store, controller);
            }
        ));
        settings_btn.connect_clicked(glib::clone!(
            #[weak]
            controller,
            #[weak]
            popover,
            #[weak]
            anchor,
            #[strong]
            list_id,
            move |_| {
                popover.popdown();
                views::list_settings::present(&anchor, &controller, &list_id);
            }
        ));
        close_btn.connect_clicked(glib::clone!(
            #[weak]
            controller,
            #[weak]
            popover,
            #[strong]
            list_id,
            move |_| {
                popover.popdown();
                controller.exclude_list(&list_id);
                if controller.current_list_id() == list_id {
                    controller.select_list("");
                }
            }
        ));
        delete_btn.connect_clicked(glib::clone!(
            #[weak]
            controller,
            #[weak]
            popover,
            #[weak]
            anchor,
            #[strong]
            list_id,
            move |_| {
                popover.popdown();
                views::list_page::present_delete_confirm(&anchor, &controller, &list_id);
            }
        ));

        popover
    }

    fn toast(&self, message: &str) {
        if let Some(overlay) = self.imp().toasts.get() {
            overlay.add_toast(adw::Toast::new(message));
        }
    }
}

/// A flat, full-width button for the sidebar context-menu popover.
fn flat_sidebar_button(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .has_frame(false)
        .halign(gtk::Align::Fill)
        .build()
}

/// Factory for sidebar rows: emoji-or-icon + name + unchecked-count badge. `win` is held
/// weakly so each row's right-click context menu can reach the controller.
fn sidebar_factory(win: &QuiteListieWindow) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let win = win.downgrade();
    factory.connect_setup(move |_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();

        // A header row (Favourites / folder name) and a normal list row share the same
        // ListItem; bind shows one. The header is rendered non-selectable.
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_start(6)
            .margin_end(6)
            .margin_top(8)
            .margin_bottom(2)
            .build();
        let header_icon = gtk::Image::new();
        header_icon.add_css_class("dim-label");
        let header_label = gtk::Label::builder().xalign(0.0).build();
        header_label.add_css_class("dim-label");
        header_label.add_css_class("caption-heading");
        header.append(&header_icon);
        header.append(&header_label);

        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_start(6)
            .margin_end(6)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        // EmojiOrIcon: an Image (themed icon) and a Label (emoji glyph), one shown.
        let icon = gtk::Image::new();
        let emoji = gtk::Label::new(None);
        let icon_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon_slot.set_size_request(20, -1);
        icon_slot.append(&icon);
        icon_slot.append(&emoji);
        let name = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let badge = gtk::Label::new(None);
        badge.add_css_class("dim-label");
        row.append(&icon_slot);
        row.append(&name);
        row.append(&badge);

        let stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
        stack.append(&header);
        stack.append(&row);
        list_item.set_child(Some(&stack));

        // Right-click context menu (Swift SidebarView.listRow context menu). Reads the
        // currently-bound item at press time, so it survives row recycling.
        let menu_gesture = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
        menu_gesture.connect_pressed(glib::clone!(
            #[strong]
            win,
            #[weak]
            list_item,
            #[weak]
            stack,
            move |_, _, x, y| {
                let Some(win) = win.upgrade() else { return };
                let Some(item) = list_item.item().and_downcast::<SidebarItem>() else { return };
                if item.is_header() {
                    return;
                }
                let popover = win.sidebar_context_menu(&item, &stack);
                popover.set_parent(&stack);
                popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                popover.connect_closed(|p| p.unparent());
                popover.popup();
            }
        ));
        stack.add_controller(menu_gesture);
        unsafe {
            list_item.set_data("ql-header", header);
            list_item.set_data("ql-header-icon", header_icon);
            list_item.set_data("ql-header-label", header_label);
            list_item.set_data("ql-row", row);
            list_item.set_data("ql-icon", icon);
            list_item.set_data("ql-emoji", emoji);
            list_item.set_data("ql-name", name);
            list_item.set_data("ql-badge", badge);
        }
    });
    factory.connect_bind(|_, list_item| {
        let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(obj) = list_item.item().and_downcast::<SidebarItem>() else {
            return;
        };
        let (header, header_icon, header_label, row, icon, emoji_label, name, badge) = unsafe {
            (
                list_item.data::<gtk::Box>("ql-header").unwrap().as_ref().clone(),
                list_item.data::<gtk::Image>("ql-header-icon").unwrap().as_ref().clone(),
                list_item.data::<gtk::Label>("ql-header-label").unwrap().as_ref().clone(),
                list_item.data::<gtk::Box>("ql-row").unwrap().as_ref().clone(),
                list_item.data::<gtk::Image>("ql-icon").unwrap().as_ref().clone(),
                list_item.data::<gtk::Label>("ql-emoji").unwrap().as_ref().clone(),
                list_item.data::<gtk::Label>("ql-name").unwrap().as_ref().clone(),
                list_item.data::<gtk::Label>("ql-badge").unwrap().as_ref().clone(),
            )
        };

        if obj.is_header() {
            list_item.set_selectable(false);
            list_item.set_activatable(false);
            header.set_visible(true);
            row.set_visible(false);
            header_icon.set_icon_name(Some(&obj.icon()));
            header_label.set_text(&obj.name());
            return;
        }
        list_item.set_selectable(true);
        list_item.set_activatable(true);
        header.set_visible(false);
        row.set_visible(true);

        let emoji = obj.emoji_icon();
        if emoji.is_empty() {
            icon.set_visible(true);
            emoji_label.set_visible(false);
            icon.set_icon_name(Some(&obj.icon()));
        } else {
            icon.set_visible(false);
            emoji_label.set_visible(true);
            emoji_label.set_text(&emoji);
        }
        name.set_text(&obj.name());
        let count = obj.unchecked_count();
        badge.set_visible(count > 0);
        badge.set_text(&count.to_string());
    });
    factory
}

/// App-wide CSS for the bespoke bits (label dot, reminder chip, quantity badge).
fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        "
        .ql-quantity-badge {
            background: alpha(@accent_color, 0.15);
            color: @accent_color;
            border-radius: 8px;
            padding: 1px 7px;
            font-size: 0.85em;
        }
        .ql-reminder-chip { padding: 0 2px; }
        /* Sidebar smart cards (Swift ReminderSmartBox): rounded surface, big count,
           per-box coloured icon. */
        .ql-smart-card {
            background-color: @card_bg_color;
            border-radius: 16px;
            border: none;
            box-shadow: none;
            padding: 0;
            min-height: 0;
        }
        .ql-smart-card:hover  { background-color: mix(@card_bg_color, @card_fg_color, 0.06); }
        .ql-smart-card:active { background-color: mix(@card_bg_color, @card_fg_color, 0.10); }
        .ql-smart-count { font-size: 1.5em; font-weight: bold; }
        .ql-smart-title { font-size: 0.95em; }
        .ql-smart-today .ql-smart-icon     { color: #e66100; }
        .ql-smart-scheduled .ql-smart-icon { color: #1c71d8; }
        .ql-smart-locations .ql-smart-icon { color: #2ec27e; }
        /* Active selection (Swift selectedListID): per-box tint + stroke. Declared after
           :hover so a selected card keeps its highlight. */
        .ql-smart-today.ql-smart-selected     { background-color: alpha(#e66100, 0.15); box-shadow: inset 0 0 0 1px alpha(#e66100, 0.45); }
        .ql-smart-scheduled.ql-smart-selected { background-color: alpha(#1c71d8, 0.15); box-shadow: inset 0 0 0 1px alpha(#1c71d8, 0.45); }
        .ql-smart-locations.ql-smart-selected { background-color: alpha(#2ec27e, 0.15); box-shadow: inset 0 0 0 1px alpha(#2ec27e, 0.45); }
        .ql-map-pin { padding: 0; min-width: 0; min-height: 0; background: none; box-shadow: none; }
        /* White-gradient circle with the label colour as the ring (set per-pin via
           border-color), plus a black depth shadow. 10% smaller than before. */
        .ql-map-pin-body {
            min-width: 24px;
            min-height: 24px;
            border-radius: 999px;
            border: 3px solid rgba(0,0,0,0.45);
            background-image: linear-gradient(to bottom, #555555, #333333);
            box-shadow: 0 1px 3px rgba(0,0,0,0.45),
                        inset 0 1px 1px rgba(255,255,255,0.18);
            padding: 2px;
        }
        /* Apple-Maps-style pointer under the circle, coloured per-pin via border-top-color.
           Transparent left/right + a coloured top border render a downward triangle. */
        .ql-map-pin-tail {
            min-width: 0;
            min-height: 0;
            border-left: 6px solid transparent;
            border-right: 6px solid transparent;
            border-top: 8px solid rgba(0,0,0,0.45);
            margin-top: -3px;
        }
        /* Emoji gets a dark outline ('stroke') + a softer drop shadow ('depth') so it reads
           cleanly on any label colour. TWEAK HERE:
             - stroke size/darkness: 1st text-shadow blur (2px) + alpha (0.9)
             - shadow/depth:         2nd text-shadow offset/blur (0 1px 2px) + alpha
             - emoji size:           font-size
             (pin circle size lives in .ql-map-pin-body above: min-width/height + padding) */
        .ql-map-pin-emoji {
            font-size: 14px;
            padding: 0;
            text-shadow: 0 0 3px rgba(0,0,0,1),
                         0 1px 2px rgba(0,0,0,1);
        }
        /* Marker name pill below the circle. */
        .ql-map-pin-label {
            font-size: 0.72em;
            font-weight: bold;
            color: @window_fg_color;
            background-color: alpha(@window_bg_color, 0.82);
            border-radius: 6px;
            padding: 0 5px;
            box-shadow: 0 1px 2px rgba(0,0,0,0.35);
        }
        .ql-coord-readout {
            background: alpha(@window_bg_color, 0.85);
            border-radius: 12px;
            padding: 4px 12px;
            font-feature-settings: \"tnum\";
        }
        /* User-location dot (Swift UserAnnotation): blue core, white ring, soft halo. */
        .ql-user-location {
            min-width: 14px;
            min-height: 14px;
            border-radius: 999px;
            background-color: #1c71d8;
            border: 2px solid #ffffff;
            box-shadow: 0 0 0 4px alpha(#1c71d8, 0.25), 0 1px 2px rgba(0,0,0,0.4);
        }
        /* Reminder proximity colours — match Swift ReminderStatus.color:
           overdue=red, today=orange, tomorrow=blue, future=gray. */
        .ql-overdue  { color: #e01b24; }
        .ql-today    { color: #e66100; }
        .ql-tomorrow { color: #1c71d8; }
        .ql-future   { color: #77767b; }
        .strikethrough { text-decoration: line-through; }
        .ql-item-chip {
            background: alpha(@accent_bg_color, 0.12);
            color: @accent_color;
            border-radius: 9999px;
            padding: 2px 8px;
        }
        .ql-notes-border { border-left: 1px solid @borders; }
        .ql-transparent, .ql-transparent > viewport { background-color: transparent; }
        ",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
