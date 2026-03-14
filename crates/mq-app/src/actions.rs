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
}

/// Show the keyboard shortcuts window.
fn show_shortcuts_window(window: &crate::widgets::window::MqWindow) {
    let section = gtk::ShortcutsSection::builder()
        .section_name("shortcuts")
        .max_height(10)
        .build();

    // General group
    let general_group = gtk::ShortcutsGroup::builder()
        .title("General")
        .build();
    add_shortcut(&general_group, "<Control>n", "Compose new message");
    add_shortcut(&general_group, "<Control>f", "Search messages");
    add_shortcut(&general_group, "<Control>comma", "Preferences");
    add_shortcut(&general_group, "<Control>q", "Quit");
    add_shortcut(&general_group, "<Control>question", "Keyboard shortcuts");
    section.append(&general_group);

    // Messages group
    let messages_group = gtk::ShortcutsGroup::builder()
        .title("Messages")
        .build();
    add_shortcut(&messages_group, "r", "Reply");
    add_shortcut(&messages_group, "<Shift>r", "Reply all");
    add_shortcut(&messages_group, "<Shift>f", "Forward");
    add_shortcut(&messages_group, "Delete", "Delete");
    add_shortcut(&messages_group, "e", "Archive");
    add_shortcut(&messages_group, "s", "Star");
    add_shortcut(&messages_group, "<Shift>u", "Toggle read/unread");
    section.append(&messages_group);

    let shortcuts_window = gtk::ShortcutsWindow::builder()
        .transient_for(window)
        .modal(true)
        .child(&section)
        .build();
    shortcuts_window.present();
}

fn add_shortcut(group: &gtk::ShortcutsGroup, accel: &str, title: &str) {
    let shortcut = gtk::ShortcutsShortcut::builder()
        .accelerator(accel)
        .title(title)
        .build();
    group.append(&shortcut);
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
