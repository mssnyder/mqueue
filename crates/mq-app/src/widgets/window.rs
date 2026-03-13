use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;

use crate::config;
use super::message_list::MqMessageList;
use super::message_view::MqMessageView;
use super::sidebar::MqSidebar;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqWindow {
        pub sidebar: RefCell<Option<MqSidebar>>,
        pub message_list: RefCell<Option<MqMessageList>>,
        pub message_view: RefCell<Option<MqMessageView>>,
        pub split_view: RefCell<Option<adw::NavigationSplitView>>,
        pub banner: RefCell<Option<adw::Banner>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MqWindow {
        const NAME: &'static str = "MqWindow";
        type Type = super::MqWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for MqWindow {
        fn constructed(&self) {
            self.parent_constructed();

            let window = self.obj();
            window.set_title(Some(config::APP_NAME));
            window.set_default_size(1200, 800);

            // Main vertical box: banner + content
            let main_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .build();

            // Offline/error banner (hidden by default)
            let banner = adw::Banner::builder()
                .revealed(false)
                .build();
            main_box.append(&banner);

            // Create widgets
            let sidebar = MqSidebar::new();
            let message_list = MqMessageList::new();
            let message_view = MqMessageView::new();

            // Sidebar navigation page
            let sidebar_page = adw::NavigationPage::builder()
                .title("m'Queue")
                .child(&sidebar)
                .build();

            // Content: overlay split view (message list + message view)
            let content_split = adw::OverlaySplitView::builder()
                .sidebar(&message_list)
                .content(&message_view)
                .collapsed(false)
                .sidebar_position(gtk::PackType::Start)
                .min_sidebar_width(350.0)
                .max_sidebar_width(500.0)
                .build();

            let content_page = adw::NavigationPage::builder()
                .title("Inbox")
                .child(&content_split)
                .build();

            // Main navigation split view (sidebar + content)
            let split_view = adw::NavigationSplitView::builder()
                .sidebar(&sidebar_page)
                .content(&content_page)
                .min_sidebar_width(200.0)
                .max_sidebar_width(300.0)
                .build();

            main_box.append(&split_view);

            window.set_content(Some(&main_box));

            // Wire sidebar selection to update message list title
            let content_page_clone = content_page.clone();
            let message_list_clone = message_list.clone();
            sidebar.connect_mailbox_selected(move |mailbox| {
                let display = mailbox_display_name(mailbox);
                content_page_clone.set_title(&display);
                message_list_clone.set_mailbox_title(&display);
            });

            // Store references
            *self.sidebar.borrow_mut() = Some(sidebar);
            *self.message_list.borrow_mut() = Some(message_list);
            *self.message_view.borrow_mut() = Some(message_view);
            *self.split_view.borrow_mut() = Some(split_view);
            *self.banner.borrow_mut() = Some(banner);
        }
    }

    impl WidgetImpl for MqWindow {}
    impl WindowImpl for MqWindow {}
    impl ApplicationWindowImpl for MqWindow {}
    impl AdwApplicationWindowImpl for MqWindow {}
}

glib::wrapper! {
    pub struct MqWindow(ObjectSubclass<imp::MqWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap,
            gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl MqWindow {
    pub fn new(app: &adw::Application) -> Self {
        glib::Object::builder()
            .property("application", app)
            .build()
    }

    /// Get the sidebar widget.
    pub fn sidebar(&self) -> MqSidebar {
        self.imp()
            .sidebar
            .borrow()
            .clone()
            .expect("Sidebar not initialized")
    }

    /// Get the message list widget.
    pub fn message_list(&self) -> MqMessageList {
        self.imp()
            .message_list
            .borrow()
            .clone()
            .expect("Message list not initialized")
    }

    /// Get the message view widget.
    pub fn message_view(&self) -> MqMessageView {
        self.imp()
            .message_view
            .borrow()
            .clone()
            .expect("Message view not initialized")
    }

    /// Show a banner message (e.g., offline status, sync errors).
    pub fn show_banner(&self, message: &str) {
        if let Some(banner) = self.imp().banner.borrow().clone() {
            banner.set_title(message);
            banner.set_revealed(true);
        }
    }

    /// Hide the banner.
    pub fn hide_banner(&self) {
        if let Some(banner) = self.imp().banner.borrow().clone() {
            banner.set_revealed(false);
        }
    }

    /// Show the preferences window.
    pub fn show_preferences(&self) {
        use super::preferences::MqPreferences;
        use tracing::error;

        let prefs = MqPreferences::new(self);
        match mq_core::config::AppConfig::load() {
            Ok(config) => prefs.load_config(&config),
            Err(e) => error!("Failed to load config for preferences: {e}"),
        }

        let window_clone = self.clone();
        prefs.connect_close_request(move |prefs_win| {
            let prefs_win = prefs_win
                .downcast_ref::<MqPreferences>()
                .expect("MqPreferences expected");
            let config = prefs_win.collect_config();

            // Apply theme immediately
            if let Some(app) = window_clone.application() {
                let adw_app = app.downcast_ref::<adw::Application>().unwrap();
                let style_manager = adw_app.style_manager();
                match config.appearance.theme {
                    mq_core::config::Theme::System => {
                        style_manager.set_color_scheme(adw::ColorScheme::Default);
                    }
                    mq_core::config::Theme::Light => {
                        style_manager.set_color_scheme(adw::ColorScheme::ForceLight);
                    }
                    mq_core::config::Theme::Dark => {
                        style_manager.set_color_scheme(adw::ColorScheme::ForceDark);
                    }
                }
            }

            if let Err(e) = config.save() {
                tracing::error!("Failed to save preferences: {e}");
            }
            glib::Propagation::Proceed
        });

        prefs.present();
    }

    /// Activate the compose flow (triggered by Ctrl+N).
    pub fn activate_compose(&self) {
        let list = self.message_list();
        let btn = list.imp().compose_button.borrow().clone();
        if let Some(btn) = btn {
            btn.emit_clicked();
        }
    }

    /// Toggle the search bar (triggered by Ctrl+F).
    pub fn activate_search(&self) {
        let list = self.message_list();
        let btn = list.imp().search_button.borrow().clone();
        if let Some(btn) = btn {
            btn.set_active(!btn.is_active());
        }
    }
}

/// Convert IMAP mailbox name to a user-friendly display name.
fn mailbox_display_name(imap_name: &str) -> String {
    match imap_name {
        "INBOX" => "Inbox".to_string(),
        "[Gmail]/Starred" => "Starred".to_string(),
        "[Gmail]/Sent Mail" => "Sent".to_string(),
        "[Gmail]/Drafts" => "Drafts".to_string(),
        "[Gmail]/Trash" => "Trash".to_string(),
        "[Gmail]/Spam" => "Spam".to_string(),
        "[Gmail]/All Mail" => "All Mail".to_string(),
        "[Gmail]/Important" => "Important".to_string(),
        other => {
            // Strip [Gmail]/ prefix if present
            other
                .strip_prefix("[Gmail]/")
                .unwrap_or(other)
                .to_string()
        }
    }
}
