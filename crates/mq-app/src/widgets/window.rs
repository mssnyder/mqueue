use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::config;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqWindow;

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

            // Placeholder content until Phase 2
            let placeholder = adw::StatusPage::builder()
                .icon_name("mail-unread-symbolic")
                .title("m'Queue")
                .description("Welcome to m'Queue. Set up your Gmail account to get started.")
                .build();

            window.set_content(Some(&placeholder));
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
}
