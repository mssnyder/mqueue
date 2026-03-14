//! GIO actions and keyboard shortcuts for the application.

use adw::prelude::*;
use gtk::gio;

/// Register application-level keyboard shortcuts and actions.
///
/// These are accelerators that work anywhere in the app window.
pub fn setup_actions(app: &adw::Application, window: &crate::widgets::window::MqWindow) {
    // --- Application-scoped actions (prefixed "app.") ---

    // Quit
    let quit_action = gio::SimpleAction::new("quit", None);
    let app_clone = app.clone();
    quit_action.connect_activate(move |_, _| {
        app_clone.quit();
    });
    app.add_action(&quit_action);

    // Preferences
    let preferences_action = gio::SimpleAction::new("preferences", None);
    let window_clone = window.clone();
    preferences_action.connect_activate(move |_, _| {
        crate::widgets::window::MqWindow::show_preferences(&window_clone);
    });
    app.add_action(&preferences_action);

    // Keyboard shortcuts help
    let shortcuts_action = gio::SimpleAction::new("shortcuts", None);
    let window_clone = window.clone();
    shortcuts_action.connect_activate(move |_, _| {
        show_shortcuts_window(&window_clone);
    });
    app.add_action(&shortcuts_action);

    // About
    let about_action = gio::SimpleAction::new("about", None);
    let window_clone = window.clone();
    about_action.connect_activate(move |_, _| {
        show_about_dialog(&window_clone);
    });
    app.add_action(&about_action);

    // --- Window-scoped actions (prefixed "win.") ---

    // Compose new message
    let compose_action = gio::SimpleAction::new("compose", None);
    let window_clone = window.clone();
    compose_action.connect_activate(move |_, _| {
        window_clone.activate_compose();
    });
    window.add_action(&compose_action);

    // Search
    let search_action = gio::SimpleAction::new("search", None);
    let window_clone = window.clone();
    search_action.connect_activate(move |_, _| {
        window_clone.activate_search();
    });
    window.add_action(&search_action);

    // --- Message actions ---

    let reply_action = gio::SimpleAction::new("reply", None);
    let window_clone = window.clone();
    reply_action.connect_activate(move |_, _| {
        window_clone.activate_reply();
    });
    window.add_action(&reply_action);

    let reply_all_action = gio::SimpleAction::new("reply-all", None);
    let window_clone = window.clone();
    reply_all_action.connect_activate(move |_, _| {
        window_clone.activate_reply_all();
    });
    window.add_action(&reply_all_action);

    let forward_action = gio::SimpleAction::new("forward", None);
    let window_clone = window.clone();
    forward_action.connect_activate(move |_, _| {
        window_clone.activate_forward();
    });
    window.add_action(&forward_action);

    let delete_action = gio::SimpleAction::new("delete", None);
    let window_clone = window.clone();
    delete_action.connect_activate(move |_, _| {
        window_clone.activate_delete();
    });
    window.add_action(&delete_action);

    let archive_action = gio::SimpleAction::new("archive", None);
    let window_clone = window.clone();
    archive_action.connect_activate(move |_, _| {
        window_clone.activate_archive();
    });
    window.add_action(&archive_action);

    let star_action = gio::SimpleAction::new("star", None);
    let window_clone = window.clone();
    star_action.connect_activate(move |_, _| {
        window_clone.activate_star();
    });
    window.add_action(&star_action);

    let read_toggle_action = gio::SimpleAction::new("read-toggle", None);
    let window_clone = window.clone();
    read_toggle_action.connect_activate(move |_, _| {
        window_clone.activate_read_toggle();
    });
    window.add_action(&read_toggle_action);

    // Next/previous message navigation
    let next_msg_action = gio::SimpleAction::new("next-message", None);
    let window_clone = window.clone();
    next_msg_action.connect_activate(move |_, _| {
        window_clone.activate_next_message();
    });
    window.add_action(&next_msg_action);

    let prev_msg_action = gio::SimpleAction::new("prev-message", None);
    let window_clone = window.clone();
    prev_msg_action.connect_activate(move |_, _| {
        window_clone.activate_prev_message();
    });
    window.add_action(&prev_msg_action);

    // Close search (Escape)
    let close_search_action = gio::SimpleAction::new("close-search", None);
    let window_clone = window.clone();
    close_search_action.connect_activate(move |_, _| {
        window_clone.activate_close_search();
    });
    window.add_action(&close_search_action);

    // --- Set accelerators ---
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("app.preferences", &["<Control>comma"]);
    app.set_accels_for_action("app.shortcuts", &["<Control>question"]);
    app.set_accels_for_action("win.compose", &["<Control>n"]);
    app.set_accels_for_action("win.search", &["<Control>f"]);
    app.set_accels_for_action("win.reply", &["r"]);
    app.set_accels_for_action("win.reply-all", &["<Shift>r"]);
    app.set_accels_for_action("win.forward", &["<Shift>f"]);
    app.set_accels_for_action("win.delete", &["Delete"]);
    app.set_accels_for_action("win.archive", &["e"]);
    app.set_accels_for_action("win.star", &["s"]);
    app.set_accels_for_action("win.read-toggle", &["<Shift>u"]);
    app.set_accels_for_action("win.next-message", &["j"]);
    app.set_accels_for_action("win.prev-message", &["k"]);
    app.set_accels_for_action("win.close-search", &["Escape"]);
}

/// Show the keyboard shortcuts window.
///
/// Uses a plain adw::Window instead of gtk::ShortcutsWindow because the
/// latter segfaults on close with some GTK4 builds / Wayland compositors.
fn show_shortcuts_window(window: &crate::widgets::window::MqWindow) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();

    // General shortcuts
    add_shortcut_section(&content, "General", &[
        ("Ctrl+N", "Compose new message"),
        ("Ctrl+F", "Search messages"),
        ("Escape", "Close search / compose"),
        ("Ctrl+,", "Preferences"),
        ("Ctrl+Q", "Quit"),
        ("Ctrl+?", "Keyboard shortcuts"),
    ]);

    // Message shortcuts
    add_shortcut_section(&content, "Messages", &[
        ("R", "Reply"),
        ("Shift+R", "Reply all"),
        ("Shift+F", "Forward"),
        ("Delete", "Delete"),
        ("E", "Archive"),
        ("S", "Star"),
        ("Shift+U", "Toggle read/unread"),
        ("J", "Next message"),
        ("K", "Previous message"),
    ]);

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&content)
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scrolled));

    let dialog = adw::Window::builder()
        .title("Keyboard Shortcuts")
        .transient_for(window)
        .modal(true)
        .default_width(400)
        .default_height(480)
        .content(&toolbar)
        .build();
    dialog.present();
}

/// Add a titled group of shortcut rows to a container.
fn add_shortcut_section(container: &gtk::Box, title: &str, shortcuts: &[(&str, &str)]) {
    let heading = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(["heading"])
        .margin_top(12)
        .build();
    container.append(&heading);

    for (key, desc) in shortcuts {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .build();

        let key_label = gtk::Label::builder()
            .label(*key)
            .width_chars(12)
            .xalign(1.0)
            .css_classes(["dim-label", "caption", "monospace"])
            .build();
        let desc_label = gtk::Label::builder()
            .label(*desc)
            .xalign(0.0)
            .hexpand(true)
            .build();

        row.append(&key_label);
        row.append(&desc_label);
        container.append(&row);
    }
}

/// Show the About dialog.
fn show_about_dialog(window: &crate::widgets::window::MqWindow) {
    let dialog = adw::AboutWindow::builder()
        .application_name(crate::config::APP_NAME)
        .application_icon(crate::config::APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("m'Queue Contributors")
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/mqmail/mq-mail")
        .issue_url("https://github.com/mqmail/mq-mail/issues")
        .transient_for(window)
        .modal(true)
        .build();

    dialog.add_credit_section(Some("Built with"), &[
        "GTK4 &amp; libadwaita https://gtk.org",
        "Rust https://www.rust-lang.org",
    ]);

    dialog.present();
}

/// Build the application primary menu (hamburger menu).
pub fn build_primary_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("_Preferences"), Some("app.preferences"));
    menu.append(Some("_Keyboard Shortcuts"), Some("app.shortcuts"));
    menu.append(Some("_About m'Queue"), Some("app.about"));
    menu.append(Some("_Quit"), Some("app.quit"));
    menu
}
