//! Preferences window built with AdwPreferencesWindow.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

use mq_core::config::{AppConfig, ReplyPosition, Theme};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqPreferences {
        pub config: RefCell<AppConfig>,

        // Appearance
        pub theme_row: RefCell<Option<adw::ComboRow>>,

        // Privacy
        pub block_images_row: RefCell<Option<adw::SwitchRow>>,
        pub strip_tracking_row: RefCell<Option<adw::SwitchRow>>,

        // Compose
        pub signature_row: RefCell<Option<adw::EntryRow>>,
        pub reply_position_row: RefCell<Option<adw::ComboRow>>,

        // Logging
        pub file_logging_row: RefCell<Option<adw::SwitchRow>>,
        pub journald_row: RefCell<Option<adw::SwitchRow>>,
        pub log_level_row: RefCell<Option<adw::ComboRow>>,

        // Cache
        pub retention_row: RefCell<Option<adw::SpinRow>>,

        // Sync
        pub sync_all_row: RefCell<Option<adw::SwitchRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MqPreferences {
        const NAME: &'static str = "MqPreferences";
        type Type = super::MqPreferences;
        type ParentType = adw::PreferencesWindow;
    }

    impl ObjectImpl for MqPreferences {
        fn constructed(&self) {
            self.parent_constructed();

            let window = self.obj();
            window.set_title(Some("Preferences"));
            window.set_default_size(600, 700);
            window.set_search_enabled(false);

            // ---- Appearance page ----
            let appearance_page = adw::PreferencesPage::builder()
                .title("Appearance")
                .icon_name("preferences-desktop-appearance-symbolic")
                .build();

            let appearance_group = adw::PreferencesGroup::builder()
                .title("Theme")
                .build();

            let theme_model = gtk::StringList::new(&["System", "Light", "Dark"]);
            let theme_row = adw::ComboRow::builder()
                .title("Color scheme")
                .subtitle("Follow the system theme or choose light/dark")
                .model(&theme_model)
                .build();
            appearance_group.add(&theme_row);
            appearance_page.add(&appearance_group);
            window.add(&appearance_page);

            // ---- Privacy page ----
            let privacy_page = adw::PreferencesPage::builder()
                .title("Privacy")
                .icon_name("security-high-symbolic")
                .build();

            let privacy_group = adw::PreferencesGroup::builder()
                .title("Email Privacy")
                .description("Tracking pixels are always removed automatically.")
                .build();

            let block_images_row = adw::SwitchRow::builder()
                .title("Block remote images")
                .subtitle("Prevent remote image loading unless explicitly allowed")
                .build();
            privacy_group.add(&block_images_row);

            let strip_tracking_row = adw::SwitchRow::builder()
                .title("Strip tracking parameters")
                .subtitle("Remove UTM and other tracking params from links")
                .build();
            privacy_group.add(&strip_tracking_row);

            privacy_page.add(&privacy_group);
            window.add(&privacy_page);

            // ---- Compose page ----
            let compose_page = adw::PreferencesPage::builder()
                .title("Compose")
                .icon_name("document-edit-symbolic")
                .build();

            let compose_group = adw::PreferencesGroup::builder()
                .title("Compose Settings")
                .build();

            let signature_row = adw::EntryRow::builder()
                .title("Default signature")
                .build();
            compose_group.add(&signature_row);

            let reply_model = gtk::StringList::new(&["Above quoted text", "Below quoted text"]);
            let reply_position_row = adw::ComboRow::builder()
                .title("Reply position")
                .model(&reply_model)
                .build();
            compose_group.add(&reply_position_row);

            compose_page.add(&compose_group);
            window.add(&compose_page);

            // ---- Advanced page ----
            let advanced_page = adw::PreferencesPage::builder()
                .title("Advanced")
                .icon_name("preferences-system-symbolic")
                .build();

            let logging_group = adw::PreferencesGroup::builder()
                .title("Logging")
                .build();

            let file_logging_row = adw::SwitchRow::builder()
                .title("File logging")
                .subtitle("Write logs to a file for debugging")
                .build();
            logging_group.add(&file_logging_row);

            let journald_row = adw::SwitchRow::builder()
                .title("Journal logging")
                .subtitle("Send logs to the systemd journal")
                .build();
            logging_group.add(&journald_row);

            let log_level_model =
                gtk::StringList::new(&["Error", "Warn", "Info", "Debug", "Trace"]);
            let log_level_row = adw::ComboRow::builder()
                .title("Log level")
                .model(&log_level_model)
                .build();
            logging_group.add(&log_level_row);

            advanced_page.add(&logging_group);

            let cache_group = adw::PreferencesGroup::builder()
                .title("Cache")
                .build();

            let retention_row = adw::SpinRow::builder()
                .title("Cache retention (days)")
                .subtitle("How long to keep cached message bodies")
                .adjustment(
                    &gtk::Adjustment::new(90.0, 7.0, 365.0, 1.0, 30.0, 0.0),
                )
                .build();
            cache_group.add(&retention_row);

            advanced_page.add(&cache_group);

            let sync_group = adw::PreferencesGroup::builder()
                .title("Sync")
                .build();

            let sync_all_row = adw::SwitchRow::builder()
                .title("Sync all mailboxes")
                .subtitle("Sync Starred, Sent, Drafts, etc. instead of just Inbox")
                .build();
            sync_group.add(&sync_all_row);

            advanced_page.add(&sync_group);
            window.add(&advanced_page);

            // Store references
            *self.theme_row.borrow_mut() = Some(theme_row.clone());
            *self.block_images_row.borrow_mut() = Some(block_images_row.clone());
            *self.strip_tracking_row.borrow_mut() = Some(strip_tracking_row.clone());
            *self.signature_row.borrow_mut() = Some(signature_row.clone());
            *self.reply_position_row.borrow_mut() = Some(reply_position_row.clone());
            *self.file_logging_row.borrow_mut() = Some(file_logging_row.clone());
            *self.journald_row.borrow_mut() = Some(journald_row.clone());
            *self.log_level_row.borrow_mut() = Some(log_level_row.clone());
            *self.retention_row.borrow_mut() = Some(retention_row.clone());
            *self.sync_all_row.borrow_mut() = Some(sync_all_row.clone());

            // If config is Nix-managed, grey out all settings with a notice
            if mq_core::config::AppConfig::is_nix_managed() {
                // Add a banner to the first page explaining Nix management
                let nix_group = adw::PreferencesGroup::builder()
                    .title("Managed by Nix")
                    .description(
                        "Settings are managed by your Nix configuration and cannot be changed here. \
                         Edit your mq-mail module in your NixOS/home-manager config to change these values."
                    )
                    .build();
                appearance_page.add(&nix_group);

                // Grey out all interactive rows
                theme_row.set_sensitive(false);
                block_images_row.set_sensitive(false);
                strip_tracking_row.set_sensitive(false);
                signature_row.set_sensitive(false);
                reply_position_row.set_sensitive(false);
                file_logging_row.set_sensitive(false);
                journald_row.set_sensitive(false);
                log_level_row.set_sensitive(false);
                retention_row.set_sensitive(false);
                sync_all_row.set_sensitive(false);
            }
        }
    }

    impl WidgetImpl for MqPreferences {}
    impl WindowImpl for MqPreferences {}
    impl AdwWindowImpl for MqPreferences {}
    impl PreferencesWindowImpl for MqPreferences {}
}

glib::wrapper! {
    pub struct MqPreferences(ObjectSubclass<imp::MqPreferences>)
        @extends adw::PreferencesWindow, adw::Window, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl MqPreferences {
    pub fn new(parent: &impl IsA<gtk::Window>) -> Self {
        let prefs: Self = glib::Object::builder()
            .property("transient-for", parent)
            .property("modal", true)
            .build();
        prefs
    }

    /// Load current config values into the UI widgets.
    pub fn load_config(&self, config: &AppConfig) {
        let imp = self.imp();
        *imp.config.borrow_mut() = config.clone();

        // Theme
        if let Some(row) = imp.theme_row.borrow().as_ref() {
            let idx = match config.appearance.theme {
                Theme::System => 0,
                Theme::Light => 1,
                Theme::Dark => 2,
            };
            row.set_selected(idx);
        }

        // Privacy
        if let Some(row) = imp.block_images_row.borrow().as_ref() {
            row.set_active(config.privacy.block_remote_images);
        }
        if let Some(row) = imp.strip_tracking_row.borrow().as_ref() {
            row.set_active(config.privacy.strip_tracking_params);
        }

        // Compose
        if let Some(row) = imp.signature_row.borrow().as_ref() {
            row.set_text(&config.compose.default_signature);
        }
        if let Some(row) = imp.reply_position_row.borrow().as_ref() {
            let idx = match config.compose.reply_position {
                ReplyPosition::Above => 0,
                ReplyPosition::Below => 1,
            };
            row.set_selected(idx);
        }

        // Logging
        if let Some(row) = imp.file_logging_row.borrow().as_ref() {
            row.set_active(config.logging.file_enabled);
        }
        if let Some(row) = imp.journald_row.borrow().as_ref() {
            row.set_active(config.logging.journald_enabled);
        }
        if let Some(row) = imp.log_level_row.borrow().as_ref() {
            let idx = match config.logging.level.as_str() {
                "error" => 0,
                "warn" => 1,
                "info" => 2,
                "debug" => 3,
                "trace" => 4,
                _ => 2,
            };
            row.set_selected(idx);
        }

        // Cache
        if let Some(row) = imp.retention_row.borrow().as_ref() {
            row.set_value(config.cache.retention_days as f64);
        }

        // Sync
        if let Some(row) = imp.sync_all_row.borrow().as_ref() {
            row.set_active(config.sync.sync_all_mailboxes);
        }
    }

    /// Read current UI state back into an AppConfig.
    pub fn collect_config(&self) -> AppConfig {
        let imp = self.imp();
        let mut config = imp.config.borrow().clone();

        // Theme
        if let Some(row) = imp.theme_row.borrow().as_ref() {
            config.appearance.theme = match row.selected() {
                0 => Theme::System,
                1 => Theme::Light,
                2 => Theme::Dark,
                _ => Theme::System,
            };
        }

        // Privacy
        if let Some(row) = imp.block_images_row.borrow().as_ref() {
            config.privacy.block_remote_images = row.is_active();
        }
        if let Some(row) = imp.strip_tracking_row.borrow().as_ref() {
            config.privacy.strip_tracking_params = row.is_active();
        }

        // Compose
        if let Some(row) = imp.signature_row.borrow().as_ref() {
            config.compose.default_signature = row.text().to_string();
        }
        if let Some(row) = imp.reply_position_row.borrow().as_ref() {
            config.compose.reply_position = match row.selected() {
                0 => ReplyPosition::Above,
                1 => ReplyPosition::Below,
                _ => ReplyPosition::Above,
            };
        }

        // Logging
        if let Some(row) = imp.file_logging_row.borrow().as_ref() {
            config.logging.file_enabled = row.is_active();
        }
        if let Some(row) = imp.journald_row.borrow().as_ref() {
            config.logging.journald_enabled = row.is_active();
        }
        if let Some(row) = imp.log_level_row.borrow().as_ref() {
            config.logging.level = match row.selected() {
                0 => "error",
                1 => "warn",
                2 => "info",
                3 => "debug",
                4 => "trace",
                _ => "info",
            }
            .to_string();
        }

        // Cache
        if let Some(row) = imp.retention_row.borrow().as_ref() {
            config.cache.retention_days = row.value() as u32;
        }

        // Sync
        if let Some(row) = imp.sync_all_row.borrow().as_ref() {
            config.sync.sync_all_mailboxes = row.is_active();
        }

        config
    }
}
