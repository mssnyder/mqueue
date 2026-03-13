//! Message list widget with virtual scrolling via gtk::ListView.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;

use super::message_object::MessageObject;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqMessageList {
        pub list_view: RefCell<Option<gtk::ListView>>,
        pub model: RefCell<Option<gio::ListStore>>,
        pub selection: RefCell<Option<gtk::SingleSelection>>,
        pub compose_button: RefCell<Option<gtk::Button>>,
        pub search_bar: RefCell<Option<gtk::SearchBar>>,
        pub search_entry: RefCell<Option<gtk::SearchEntry>>,
        pub search_button: RefCell<Option<gtk::ToggleButton>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MqMessageList {
        const NAME: &'static str = "MqMessageList";
        type Type = super::MqMessageList;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for MqMessageList {
        fn constructed(&self) {
            self.parent_constructed();

            let widget = self.obj();
            widget.set_orientation(gtk::Orientation::Vertical);

            // Header bar for the message list
            let header = adw::HeaderBar::builder()
                .show_start_title_buttons(false)
                .show_end_title_buttons(false)
                .build();

            let title = adw::WindowTitle::new("Inbox", "");
            header.set_title_widget(Some(&title));

            let compose_button = gtk::Button::builder()
                .icon_name("document-edit-symbolic")
                .tooltip_text("Compose")
                .build();
            header.pack_end(&compose_button);

            let search_button = gtk::ToggleButton::builder()
                .icon_name("system-search-symbolic")
                .tooltip_text("Search")
                .build();
            header.pack_end(&search_button);

            widget.append(&header);

            // Search bar (revealed when search button is toggled)
            let search_entry = gtk::SearchEntry::builder()
                .placeholder_text("Search messages...")
                .hexpand(true)
                .build();

            let search_bar = gtk::SearchBar::builder()
                .child(&search_entry)
                .build();
            search_bar.connect_entry(&search_entry);

            // Bind the toggle button to the search bar
            search_button
                .bind_property("active", &search_bar, "search-mode-enabled")
                .bidirectional()
                .sync_create()
                .build();

            widget.append(&search_bar);

            // Create the list model
            let model = gio::ListStore::new::<MessageObject>();

            // Create selection model
            let selection = gtk::SingleSelection::new(Some(model.clone()));

            // Create factory for list items
            let factory = gtk::SignalListItemFactory::new();

            factory.connect_setup(|_, list_item| {
                let list_item = list_item
                    .downcast_ref::<gtk::ListItem>()
                    .expect("ListItem expected");
                let row = create_message_row();
                list_item.set_child(Some(&row));
            });

            factory.connect_bind(|_, list_item| {
                let list_item = list_item
                    .downcast_ref::<gtk::ListItem>()
                    .expect("ListItem expected");
                let msg = list_item
                    .item()
                    .and_downcast::<MessageObject>()
                    .expect("MessageObject expected");
                let row = list_item
                    .child()
                    .and_downcast::<gtk::Box>()
                    .expect("Box expected");
                bind_message_row(&row, &msg);
            });

            // Create the ListView
            let list_view = gtk::ListView::builder()
                .model(&selection)
                .factory(&factory)
                .css_classes(["message-list"])
                .vexpand(true)
                .build();

            let scrolled = gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&list_view)
                .build();

            widget.append(&scrolled);

            *self.list_view.borrow_mut() = Some(list_view);
            *self.model.borrow_mut() = Some(model);
            *self.selection.borrow_mut() = Some(selection);
            *self.compose_button.borrow_mut() = Some(compose_button);
            *self.search_bar.borrow_mut() = Some(search_bar);
            *self.search_entry.borrow_mut() = Some(search_entry);
            *self.search_button.borrow_mut() = Some(search_button);
        }
    }

    impl WidgetImpl for MqMessageList {}
    impl BoxImpl for MqMessageList {}
}

/// Create the row widget structure (setup phase — no data bound yet).
fn create_message_row() -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();

    // Top line: star + sender + date
    let top_line = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();

    let star_label = gtk::Label::builder()
        .label("")
        .width_chars(2)
        .build();
    star_label.set_widget_name("star");
    top_line.append(&star_label);

    let sender_label = gtk::Label::builder()
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    sender_label.set_widget_name("sender");
    top_line.append(&sender_label);

    let date_label = gtk::Label::builder()
        .css_classes(["dim-label"])
        .build();
    date_label.set_widget_name("date");
    top_line.append(&date_label);

    row.append(&top_line);

    // Bottom line: subject + snippet
    let subject_label = gtk::Label::builder()
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    subject_label.set_widget_name("subject");
    row.append(&subject_label);

    let snippet_label = gtk::Label::builder()
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["dim-label"])
        .build();
    snippet_label.set_widget_name("snippet");
    row.append(&snippet_label);

    // Account email badge (hidden unless unified view)
    let account_label = gtk::Label::builder()
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["dim-label", "caption"])
        .visible(false)
        .build();
    account_label.set_widget_name("account");
    row.append(&account_label);

    row
}

/// Bind data from a MessageObject to the row widget (bind phase).
fn bind_message_row(row: &gtk::Box, msg: &MessageObject) {
    // Walk the widget tree to find named children
    let top_line = row
        .first_child()
        .and_downcast::<gtk::Box>()
        .expect("top_line Box");

    // Star
    if let Some(star) = find_child_by_name(&top_line, "star") {
        let star = star.downcast::<gtk::Label>().unwrap();
        star.set_label(if msg.is_flagged() { "\u{2605}" } else { "" });
    }

    // Sender
    if let Some(sender) = find_child_by_name(&top_line, "sender") {
        let sender = sender.downcast::<gtk::Label>().unwrap();
        let name = msg.sender_name();
        let display = if name.is_empty() {
            msg.sender_email()
        } else {
            name
        };
        sender.set_label(&display);

        if !msg.is_read() {
            sender.add_css_class("bold-label");
        } else {
            sender.remove_css_class("bold-label");
        }
    }

    // Date
    if let Some(date) = find_child_by_name(&top_line, "date") {
        let date = date.downcast::<gtk::Label>().unwrap();
        date.set_label(&format_date(&msg.date()));
    }

    // Subject
    if let Some(subject) = find_child_by_name(row, "subject") {
        let subject = subject.downcast::<gtk::Label>().unwrap();
        let subj_text = msg.subject();
        subject.set_label(if subj_text.is_empty() {
            "(no subject)"
        } else {
            &subj_text
        });

        if !msg.is_read() {
            subject.add_css_class("bold-label");
        } else {
            subject.remove_css_class("bold-label");
        }
    }

    // Snippet
    if let Some(snippet) = find_child_by_name(row, "snippet") {
        let snippet = snippet.downcast::<gtk::Label>().unwrap();
        snippet.set_label(&msg.snippet());
    }

    // Account badge (visible only in unified view)
    if let Some(account) = find_child_by_name(row, "account") {
        let account = account.downcast::<gtk::Label>().unwrap();
        let email = msg.account_email();
        if email.is_empty() {
            account.set_visible(false);
        } else {
            account.set_label(&email);
            account.set_visible(true);
        }
    }
}

/// Find a child widget by its widget name.
fn find_child_by_name(parent: &impl IsA<gtk::Widget>, name: &str) -> Option<gtk::Widget> {
    let mut child = parent.as_ref().first_child();
    while let Some(c) = child {
        if c.widget_name() == name {
            return Some(c);
        }
        // Check children of children (one level deep)
        if let Some(found) = find_child_by_name(&c, name) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// Format a date string for display.
fn format_date(date_str: &str) -> String {
    // For now, just truncate to a reasonable length
    // TODO: proper relative date formatting (Today, Yesterday, Mon, etc.)
    if date_str.len() > 16 {
        date_str[..16].to_string()
    } else {
        date_str.to_string()
    }
}

glib::wrapper! {
    pub struct MqMessageList(ObjectSubclass<imp::MqMessageList>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl MqMessageList {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Get the underlying list store model.
    pub fn model(&self) -> gio::ListStore {
        self.imp()
            .model
            .borrow()
            .clone()
            .expect("Model not initialized")
    }

    /// Get the selection model.
    pub fn selection(&self) -> gtk::SingleSelection {
        self.imp()
            .selection
            .borrow()
            .clone()
            .expect("Selection not initialized")
    }

    /// Set the title (mailbox name) shown in the header.
    pub fn set_mailbox_title(&self, title: &str) {
        // Find the header bar's title widget and update it
        let mut child = self.first_child();
        while let Some(c) = child {
            let next = c.next_sibling();
            if let Ok(header) = c.downcast::<adw::HeaderBar>() {
                if let Some(title_widget) = header.title_widget() {
                    if let Ok(window_title) = title_widget.downcast::<adw::WindowTitle>() {
                        window_title.set_title(title);
                    }
                }
                return;
            }
            child = next;
        }
    }

    /// Replace all messages in the list with new data.
    pub fn set_messages(&self, messages: Vec<MessageObject>) {
        let model = self.model();
        model.remove_all();
        for msg in messages {
            model.append(&msg);
        }
    }

    /// Connect a callback for the Compose button.
    pub fn connect_compose_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().compose_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for when a message is selected.
    pub fn connect_message_selected<F: Fn(&MessageObject) + 'static>(&self, f: F) {
        let selection = self.selection();
        selection.connect_selection_changed(move |sel, _, _| {
            if let Some(item) = sel.selected_item() {
                if let Ok(msg) = item.downcast::<MessageObject>() {
                    f(&msg);
                }
            }
        });
    }

    /// Connect a callback for when a search is submitted.
    ///
    /// The callback receives the search query string.
    pub fn connect_search_activated<F: Fn(String) + 'static>(&self, f: F) {
        if let Some(entry) = self.imp().search_entry.borrow().as_ref() {
            entry.connect_activate(move |entry| {
                let text = entry.text().to_string();
                f(text);
            });
        }
    }

    /// Connect a callback for when the search text changes (for live search).
    ///
    /// The callback receives the search query string. An empty string means
    /// the search was cleared and the normal view should be restored.
    pub fn connect_search_changed<F: Fn(String) + 'static>(&self, f: F) {
        if let Some(entry) = self.imp().search_entry.borrow().as_ref() {
            entry.connect_search_changed(move |entry| {
                let text = entry.text().to_string();
                f(text);
            });
        }
    }

    /// Programmatically close the search bar.
    pub fn close_search(&self) {
        if let Some(btn) = self.imp().search_button.borrow().as_ref() {
            btn.set_active(false);
        }
    }
}

impl Default for MqMessageList {
    fn default() -> Self {
        Self::new()
    }
}
