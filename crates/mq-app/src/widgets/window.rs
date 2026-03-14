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
        pub toast_overlay: RefCell<Option<adw::ToastOverlay>>,
        /// Generation counter to prevent stale async body-load results from
        /// overwriting the message view when the user rapidly clicks messages.
        pub body_load_generation: std::cell::Cell<u64>,
        /// Sort order: true = newest first (default), false = oldest first.
        pub sort_newest_first: std::cell::Cell<bool>,
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

            // Wrap everything in a ToastOverlay for undo toasts
            let toast_overlay = adw::ToastOverlay::new();
            toast_overlay.set_child(Some(&main_box));

            window.set_content(Some(&toast_overlay));

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
            *self.toast_overlay.borrow_mut() = Some(toast_overlay);
            self.sort_newest_first.set(true);
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

    /// Bump the body-load generation counter and return the new value.
    /// Call this before spawning an async body load. In the callback,
    /// compare with `body_load_generation()` to detect stale results.
    pub fn bump_body_load_generation(&self) -> u64 {
        let gen = &self.imp().body_load_generation;
        let next = gen.get().wrapping_add(1);
        gen.set(next);
        next
    }

    /// Current body-load generation value.
    pub fn body_load_generation(&self) -> u64 {
        self.imp().body_load_generation.get()
    }

    /// Show a banner message (e.g., offline status, sync errors).
    pub fn show_banner(&self, message: &str) {
        if let Some(banner) = self.imp().banner.borrow().clone() {
            banner.set_title(message);
            banner.set_revealed(true);
        }
    }

    /// Show a banner with an action button (e.g., "Retry").
    pub fn show_banner_with_action(&self, message: &str, button_label: &str) {
        if let Some(banner) = self.imp().banner.borrow().clone() {
            banner.set_title(message);
            banner.set_button_label(Some(button_label));
            banner.set_revealed(true);
        }
    }

    /// Connect a callback for the banner's action button.
    pub fn connect_banner_button<F: Fn() + 'static>(&self, f: F) {
        if let Some(banner) = self.imp().banner.borrow().clone() {
            banner.connect_button_clicked(move |_| f());
        }
    }

    /// Hide the banner.
    pub fn hide_banner(&self) {
        if let Some(banner) = self.imp().banner.borrow().clone() {
            banner.set_button_label(None::<&str>);
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

    /// Show a toast notification (e.g., undo feedback).
    pub fn show_toast(&self, toast: &adw::Toast) {
        if let Some(overlay) = self.imp().toast_overlay.borrow().clone() {
            overlay.add_toast(toast.clone());
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

        // Live theme preview: apply theme changes immediately as the user picks
        if let Some(app) = self.application() {
            let style_manager = app.downcast_ref::<adw::Application>().unwrap().style_manager();
            prefs.connect_theme_changed(move |theme| {
                match theme {
                    mq_core::config::Theme::System => style_manager.set_color_scheme(adw::ColorScheme::Default),
                    mq_core::config::Theme::Light => style_manager.set_color_scheme(adw::ColorScheme::ForceLight),
                    mq_core::config::Theme::Dark => style_manager.set_color_scheme(adw::ColorScheme::ForceDark),
                }
            });
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

    /// Returns true if the search bar is currently active (user is typing a query).
    /// Used to suppress single-letter shortcuts (r, e, s, j, k, Delete, etc.)
    /// that would otherwise fire while the user is typing in the search entry.
    pub fn is_search_active(&self) -> bool {
        self.message_list()
            .imp()
            .search_button
            .borrow()
            .as_ref()
            .map(|b| b.is_active())
            .unwrap_or(false)
    }

    /// Activate the compose flow (triggered by Ctrl+N).
    pub fn activate_compose(&self) {
        let list = self.message_list();
        let btn = list.imp().compose_button.borrow().clone();
        if let Some(btn) = btn {
            btn.emit_clicked();
        }
    }

    /// Activate reply (triggered by 'r' shortcut).
    pub fn activate_reply(&self) {
        if self.is_search_active() { return; }
        if let Some(btn) = self.message_view().imp().reply_button.borrow().as_ref() {
            btn.emit_clicked();
        }
    }

    /// Activate reply-all (triggered by Shift+R shortcut).
    pub fn activate_reply_all(&self) {
        if self.is_search_active() { return; }
        if let Some(btn) = self.message_view().imp().reply_all_button.borrow().as_ref() {
            btn.emit_clicked();
        }
    }

    /// Activate forward (triggered by Shift+F shortcut).
    pub fn activate_forward(&self) {
        if self.is_search_active() { return; }
        if let Some(btn) = self.message_view().imp().forward_button.borrow().as_ref() {
            btn.emit_clicked();
        }
    }

    /// Activate delete (triggered by Delete shortcut).
    pub fn activate_delete(&self) {
        if self.is_search_active() { return; }
        if let Some(btn) = self.message_view().imp().delete_button.borrow().as_ref() {
            btn.emit_clicked();
        }
    }

    /// Activate archive (triggered by 'e' shortcut).
    pub fn activate_archive(&self) {
        if self.is_search_active() { return; }
        if let Some(btn) = self.message_view().imp().archive_button.borrow().as_ref() {
            btn.emit_clicked();
        }
    }

    /// Toggle star (triggered by 's' shortcut).
    pub fn activate_star(&self) {
        if self.is_search_active() { return; }
        if let Some(btn) = self.message_view().imp().star_button.borrow().as_ref() {
            btn.set_active(!btn.is_active());
        }
    }

    /// Toggle read/unread (triggered by Shift+U shortcut).
    pub fn activate_read_toggle(&self) {
        if self.is_search_active() { return; }
        if let Some(btn) = self.message_view().imp().read_button.borrow().as_ref() {
            btn.set_active(!btn.is_active());
        }
    }

    /// Select the next message in the list (triggered by 'j' / Down).
    pub fn activate_next_message(&self) {
        if self.is_search_active() { return; }
        let sel = self.message_list().selection();
        let pos = sel.selected();
        let n = sel.n_items();
        if pos != gtk::INVALID_LIST_POSITION && pos + 1 < n {
            sel.set_selected(pos + 1);
        }
    }

    /// Select the previous message in the list (triggered by 'k' / Up).
    pub fn activate_prev_message(&self) {
        if self.is_search_active() { return; }
        let sel = self.message_list().selection();
        let pos = sel.selected();
        if pos != gtk::INVALID_LIST_POSITION && pos > 0 {
            sel.set_selected(pos - 1);
        }
    }

    /// Toggle the search bar (triggered by Ctrl+F).
    pub fn activate_search(&self) {
        let list = self.message_list();
        let btn = list.imp().search_button.borrow().clone();
        if let Some(btn) = btn {
            let will_activate = !btn.is_active();
            btn.set_active(will_activate);
            // Auto-focus the search entry when opening
            if will_activate {
                if let Some(entry) = list.imp().search_entry.borrow().as_ref() {
                    entry.grab_focus();
                }
            }
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
