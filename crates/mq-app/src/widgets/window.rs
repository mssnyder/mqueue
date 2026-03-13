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
        pub progress_bar: RefCell<Option<gtk::ProgressBar>>,
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
            window.set_default_size(1100, 700);
            window.set_size_request(360, 294);

            // Main vertical box: banner + content
            let main_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .build();

            // Offline/error banner (hidden by default)
            let banner = adw::Banner::builder()
                .revealed(false)
                .build();
            main_box.append(&banner);

            // Sync progress bar (hidden by default)
            let progress_bar = gtk::ProgressBar::builder()
                .visible(false)
                .show_text(true)
                .build();
            progress_bar.add_css_class("osd");
            main_box.append(&progress_bar);

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
            // When collapsed (narrow window), the message list becomes a
            // slide-over overlay so the message view gets full width.
            let content_split = adw::OverlaySplitView::builder()
                .sidebar(&message_list)
                .content(&message_view)
                .collapsed(false)
                .sidebar_position(gtk::PackType::Start)
                .min_sidebar_width(320.0)
                .max_sidebar_width(520.0)
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
                .max_sidebar_width(280.0)
                .vexpand(true)
                .build();

            main_box.append(&split_view);

            window.set_content(Some(&main_box));

            // --- Adaptive breakpoints ---
            // Collapse the inner OverlaySplitView when the window is narrow
            // (e.g. half-tiled on a 1920px display → ~960px, or third-tiled → ~640px).
            let bp_inner = adw::Breakpoint::new(
                adw::BreakpointCondition::parse("max-width: 720sp")
                    .expect("valid breakpoint condition"),
            );
            bp_inner.add_setter(&content_split, "collapsed", Some(&true.to_value()));
            window.add_breakpoint(bp_inner);

            // --- Wire the message-view sidebar button to the overlay split ---
            // The button is only visible when the OverlaySplitView is collapsed.
            if let Some(sidebar_btn) = message_view.sidebar_button() {
                // Show the button only when collapsed
                content_split
                    .bind_property("collapsed", &sidebar_btn, "visible")
                    .sync_create()
                    .build();

                // Bind the button's active state ↔ show-sidebar
                content_split
                    .bind_property("show-sidebar", &sidebar_btn, "active")
                    .bidirectional()
                    .sync_create()
                    .build();
            }

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
            *self.progress_bar.borrow_mut() = Some(progress_bar);
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

    /// Show the sync progress bar with a text label and fraction (0.0–1.0).
    pub fn show_progress(&self, text: &str, fraction: f64) {
        if let Some(pb) = self.imp().progress_bar.borrow().clone() {
            pb.set_text(Some(text));
            pb.set_fraction(fraction);
            pb.set_visible(true);
        }
    }

    /// Hide the sync progress bar.
    pub fn hide_progress(&self) {
        if let Some(pb) = self.imp().progress_bar.borrow().clone() {
            pb.set_visible(false);
        }
    }

    /// Show the preferences window.
    pub fn show_preferences(&self) {
        use super::preferences::MqPreferences;
        use tracing::error;

        let prefs = MqPreferences::new(self);
        let old_sync_all = match mq_core::config::AppConfig::load() {
            Ok(config) => {
                let old = config.sync.sync_all_mailboxes;
                prefs.load_config(&config);
                old
            }
            Err(e) => {
                error!("Failed to load config for preferences: {e}");
                false
            }
        };

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

            let sync_changed = config.sync.sync_all_mailboxes != old_sync_all;

            if let Err(e) = config.save() {
                tracing::error!("Failed to save config: {e}");
            }

            // If sync_all_mailboxes was toggled, trigger a re-sync
            if sync_changed {
                adw::prelude::ActionGroupExt::activate_action(
                    &window_clone,
                    "app.resync",
                    None,
                );
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
