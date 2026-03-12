use adw::prelude::*;
use gtk::gio;
use tracing::info;

use crate::config;
use crate::widgets::window::MqWindow;

/// Run the GTK application, returning the exit code.
pub fn run() -> i32 {
    let app = adw::Application::builder()
        .application_id(config::APP_ID)
        .flags(gio::ApplicationFlags::default())
        .build();

    app.connect_startup(|_app| {
        info!("Application startup");
    });

    app.connect_activate(|app| {
        info!("Application activate");
        let window = MqWindow::new(app);
        window.present();
    });

    app.run().into()
}
