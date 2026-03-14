//! Message list widget with virtual scrolling via gtk::ListView.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

use super::message_object::MessageObject;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MqMessageList {
        pub list_view: RefCell<Option<gtk::ListView>>,
        pub model: RefCell<Option<gio::ListStore>>,
        pub selection: RefCell<Option<gtk::SingleSelection>>,
        pub compose_button: RefCell<Option<gtk::Button>>,
        pub search_bar: RefCell<Option<gtk::SearchBar>>,
        pub search_entry: RefCell<Option<gtk::SearchEntry>>,
        pub search_button: RefCell<Option<gtk::ToggleButton>>,
        pub sort_button: RefCell<Option<gtk::MenuButton>>,
        pub stack: RefCell<Option<gtk::Stack>>,
        pub placeholder: RefCell<Option<adw::StatusPage>>,
        pub scrolled_window: RefCell<Option<gtk::ScrolledWindow>>,
        /// Suppresses selection-changed signals during model refresh.
        pub refreshing: std::rc::Rc<std::cell::Cell<bool>>,
        /// Whether a load-more fetch is currently in progress (prevents duplicate requests).
        pub loading_more: std::rc::Rc<std::cell::Cell<bool>>,
        /// Callback fired when the user scrolls near the bottom.
        pub load_more_callback: std::rc::Rc<RefCell<Option<Box<dyn Fn()>>>>,
    }

    impl std::fmt::Debug for MqMessageList {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MqMessageList").finish_non_exhaustive()
        }
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

            // Sort order menu button
            let sort_menu = gio::Menu::new();
            sort_menu.append(Some("Newest first"), Some("sort.newest"));
            sort_menu.append(Some("Oldest first"), Some("sort.oldest"));
            let sort_button = gtk::MenuButton::builder()
                .icon_name("view-sort-descending-symbolic")
                .tooltip_text("Sort order")
                .menu_model(&sort_menu)
                .build();
            header.pack_end(&sort_button);

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

            // Store notify handler IDs so we can disconnect on unbind.
            // Each list item gets its own Rc<RefCell<...>> to hold the signal ID.
            let bind_handler_ids: Rc<RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>> =
                Rc::new(RefCell::new(Vec::new()));
            let bind_ids = bind_handler_ids.clone();

            factory.connect_bind(move |_, list_item| {
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

                // Connect to is-read property changes so the row updates live
                let row_clone = row.clone();
                let handler_id = msg.connect_notify_local(Some("is-read"), move |m, _| {
                    update_read_css(&row_clone, m.is_read());
                });
                bind_ids.borrow_mut().push((msg.upcast::<glib::Object>(), handler_id));
            });

            let unbind_ids = bind_handler_ids;
            factory.connect_unbind(move |_, list_item| {
                let list_item = list_item
                    .downcast_ref::<gtk::ListItem>()
                    .expect("ListItem expected");
                if let Some(msg) = list_item.item() {
                    let mut ids = unbind_ids.borrow_mut();
                    // Disconnect handlers for this item and remove them
                    let mut i = 0;
                    while i < ids.len() {
                        if ids[i].0 == msg {
                            let (obj, id) = ids.remove(i);
                            obj.disconnect(id);
                        } else {
                            i += 1;
                        }
                    }
                }
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

            // Detect scroll near bottom for load-more pagination
            {
                let loading_more = self.loading_more.clone();
                let load_more_cb = self.load_more_callback.clone();
                let adj = scrolled.vadjustment();
                adj.connect_value_changed(move |adj| {
                    let value = adj.value();
                    let upper = adj.upper();
                    let page_size = adj.page_size();
                    if upper > page_size
                        && value + page_size >= upper - 200.0
                        && !loading_more.get()
                    {
                        loading_more.set(true);
                        if let Some(ref cb) = *load_more_cb.borrow() {
                            cb();
                        }
                    }
                });
            }

            // Empty state placeholder
            let placeholder = adw::StatusPage::builder()
                .icon_name("mail-inbox-symbolic")
                .title("No messages")
                .description("Messages you receive will appear here")
                .vexpand(true)
                .build();

            // Stack to toggle between list and placeholder
            let stack = gtk::Stack::builder()
                .vexpand(true)
                .build();
            stack.add_named(&scrolled, Some("list"));
            stack.add_named(&placeholder, Some("placeholder"));
            stack.set_visible_child_name("placeholder");

            // Toggle based on model item count
            let stack_ref = stack.clone();
            model.connect_items_changed(move |model, _, _, _| {
                if model.n_items() > 0 {
                    stack_ref.set_visible_child_name("list");
                } else {
                    stack_ref.set_visible_child_name("placeholder");
                }
            });

            widget.append(&stack);

            *self.list_view.borrow_mut() = Some(list_view);
            *self.model.borrow_mut() = Some(model);
            *self.selection.borrow_mut() = Some(selection);
            *self.compose_button.borrow_mut() = Some(compose_button);
            *self.search_bar.borrow_mut() = Some(search_bar);
            *self.search_entry.borrow_mut() = Some(search_entry);
            *self.search_button.borrow_mut() = Some(search_button);
            *self.sort_button.borrow_mut() = Some(sort_button);
            *self.stack.borrow_mut() = Some(stack);
            *self.placeholder.borrow_mut() = Some(placeholder);
            *self.scrolled_window.borrow_mut() = Some(scrolled);
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
        .margin_top(10)
        .margin_bottom(10)
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

    // Thread count badge (hidden for single messages)
    let thread_count_label = gtk::Label::builder()
        .css_classes(["dim-label", "caption"])
        .visible(false)
        .build();
    thread_count_label.set_widget_name("thread_count");
    top_line.append(&thread_count_label);

    let date_label = gtk::Label::builder()
        .css_classes(["dim-label"])
        .build();
    date_label.set_widget_name("date");
    top_line.append(&date_label);

    row.append(&top_line);

    // Subject + snippet
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

    // Thread count
    if let Some(tc) = find_child_by_name(&top_line, "thread_count") {
        let tc = tc.downcast::<gtk::Label>().unwrap();
        let count = msg.thread_count();
        if count > 1 {
            tc.set_label(&count.to_string());
            tc.set_visible(true);
        } else {
            tc.set_visible(false);
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

/// Update bold CSS classes on sender/subject based on read state.
fn update_read_css(row: &gtk::Box, is_read: bool) {
    if let Some(top_line) = row.first_child().and_downcast::<gtk::Box>() {
        if let Some(sender) = find_child_by_name(&top_line, "sender") {
            if let Ok(sender) = sender.downcast::<gtk::Label>() {
                if is_read {
                    sender.remove_css_class("bold-label");
                } else {
                    sender.add_css_class("bold-label");
                }
            }
        }
    }
    if let Some(subject) = find_child_by_name(row, "subject") {
        if let Ok(subject) = subject.downcast::<gtk::Label>() {
            if is_read {
                subject.remove_css_class("bold-label");
            } else {
                subject.add_css_class("bold-label");
            }
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

/// Format a date string for display using the shared formatter.
fn format_date(date_str: &str) -> String {
    mq_core::email::format_display_date(date_str)
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
    /// Always selects item 0 (used for initial population).
    pub fn set_messages(&self, messages: Vec<MessageObject>) {
        let model = self.model();
        let selection = self.selection();
        // Reset load-more state on full reload (e.g. mailbox switch)
        self.imp().loading_more.set(false);
        model.remove_all();
        for msg in messages {
            model.append(&msg);
        }
        // Force selection of item 0 to trigger selection-changed.
        // We defer via idle_add_local_once so GTK finishes processing all
        // model mutations before we poke the selection — this avoids races
        // where SingleSelection's internal auto-select suppresses our signal.
        if model.n_items() > 0 {
            selection.set_selected(gtk::INVALID_LIST_POSITION);
            let sel = selection.clone();
            glib::idle_add_local_once(move || {
                sel.set_selected(0);
            });
        }
    }

    /// Refresh messages without disrupting the user's current selection.
    ///
    /// Tries to re-select the same message (by db_id) after the model is
    /// replaced. Only selects item 0 if nothing was previously selected.
    pub fn refresh_messages(&self, messages: Vec<MessageObject>) {
        let model = self.model();
        let selection = self.selection();

        // Remember the currently selected db_id BEFORE touching the model.
        let prev_pos = selection.selected();
        let prev_db_id = if prev_pos != gtk::INVALID_LIST_POSITION {
            model
                .item(prev_pos)
                .and_then(|item| item.downcast::<MessageObject>().ok())
                .map(|msg| msg.db_id())
        } else {
            None
        };

        // Suppress selection-changed handler while we swap the model contents
        self.imp().refreshing.set(true);

        model.remove_all();
        for msg in messages {
            model.append(&msg);
        }

        if model.n_items() == 0 {
            self.imp().refreshing.set(false);
            return;
        }

        // Try to re-select the same message (silently — flag still set)
        if let Some(prev_id) = prev_db_id {
            for i in 0..model.n_items() {
                if let Some(item) = model.item(i) {
                    if let Ok(msg) = item.downcast::<MessageObject>() {
                        if msg.db_id() == prev_id {
                            selection.set_selected(i);
                            self.imp().refreshing.set(false);
                            return;
                        }
                    }
                }
            }
        }

        // No previous selection or message not found — select first item
        // This IS a real selection change, so clear the flag before triggering
        self.imp().refreshing.set(false);
        selection.set_selected(gtk::INVALID_LIST_POSITION);
        let sel = selection.clone();
        glib::idle_add_local_once(move || {
            sel.set_selected(0);
        });
    }

    /// Returns true if the list is currently being refreshed (model swap in progress).
    pub fn is_refreshing(&self) -> bool {
        self.imp().refreshing.get()
    }

    /// Connect a callback for the Compose button.
    pub fn connect_compose_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().compose_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for when a message is selected.
    ///
    /// The callback is NOT fired during model refreshes (sync updates) to avoid
    /// reloading the message view while the user is reading.
    pub fn connect_message_selected<F: Fn(&MessageObject) + 'static>(&self, f: F) {
        let selection = self.selection();
        let refreshing = std::rc::Rc::clone(&self.imp().refreshing);
        selection.connect_selection_changed(move |sel, _, _| {
            if refreshing.get() {
                return;
            }
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

    /// Connect a callback for when the sort order changes.
    ///
    /// The callback receives `true` for newest-first, `false` for oldest-first.
    pub fn connect_sort_changed<F: Fn(bool) + 'static>(&self, f: F) {
        let group = gio::SimpleActionGroup::new();
        let f = std::rc::Rc::new(f);

        let f_newest = f.clone();
        let newest_action = gio::SimpleAction::new("newest", None);
        newest_action.connect_activate(move |_, _| {
            f_newest(true);
        });
        group.add_action(&newest_action);

        let f_oldest = f;
        let oldest_action = gio::SimpleAction::new("oldest", None);
        oldest_action.connect_activate(move |_, _| {
            f_oldest(false);
        });
        group.add_action(&oldest_action);

        self.insert_action_group("sort", Some(&group));
    }

    /// Update the empty state placeholder text.
    pub fn set_placeholder_text(&self, title: &str, description: &str) {
        if let Some(placeholder) = self.imp().placeholder.borrow().as_ref() {
            placeholder.set_title(title);
            placeholder.set_description(Some(description));
        }
    }

    /// Reset placeholder to default mailbox text.
    pub fn reset_placeholder(&self) {
        self.set_placeholder_text("No messages", "Messages you receive will appear here");
    }

    /// Insert a message at the given position (for undo re-insertion).
    pub fn insert_message_at(&self, pos: u32, msg: &MessageObject) {
        let model = self.model();
        let clamped = pos.min(model.n_items());
        model.insert(clamped, msg);
    }

    /// Connect a callback for when the user scrolls near the bottom (load-more pagination).
    pub fn connect_load_more<F: Fn() + 'static>(&self, f: F) {
        *self.imp().load_more_callback.borrow_mut() = Some(Box::new(f));
    }

    /// Append messages to the end of the list (for pagination).
    /// Does not reset the selection.
    pub fn append_messages(&self, messages: Vec<MessageObject>) {
        let model = self.model();
        for msg in messages {
            model.append(&msg);
        }
        // Allow future load-more triggers
        self.imp().loading_more.set(false);
    }

    /// Signal that a load-more attempt returned no results (no more pages).
    pub fn load_more_finished(&self) {
        self.imp().loading_more.set(false);
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
