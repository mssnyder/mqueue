//! Sidebar widget with account selector and mailbox/label navigation.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqSidebar {
        pub account_list: RefCell<Option<gtk::ListBox>>,
        pub mailbox_list: RefCell<Option<gtk::ListBox>>,
        pub label_list: RefCell<Option<gtk::ListBox>>,
        pub labels_header: RefCell<Option<gtk::Label>>,
        pub labels_separator: RefCell<Option<gtk::Separator>>,
        pub add_account_button: RefCell<Option<gtk::Button>>,
        pub selected_mailbox: RefCell<String>,
        /// None = All Accounts, Some(id) = specific account.
        pub selected_account_id: Rc<Cell<Option<i64>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MqSidebar {
        const NAME: &'static str = "MqSidebar";
        type Type = super::MqSidebar;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for MqSidebar {
        fn constructed(&self) {
            self.parent_constructed();

            let widget = self.obj();
            widget.set_orientation(gtk::Orientation::Vertical);
            widget.set_spacing(0);

            // Header with app name + hamburger menu
            let header = adw::HeaderBar::builder()
                .title_widget(&adw::WindowTitle::new("m'Queue", ""))
                .show_end_title_buttons(false)
                .build();

            let menu_button = gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .tooltip_text("Main Menu")
                .menu_model(&crate::actions::build_primary_menu())
                .build();
            header.pack_start(&menu_button);

            widget.append(&header);

            // Scrolled area for both lists
            let scrolled = gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build();

            let content = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(0)
                .build();

            // --- Accounts section ---
            let accounts_header = gtk::Label::builder()
                .label("Accounts")
                .xalign(0.0)
                .css_classes(["dim-label", "caption", "heading"])
                .margin_top(8)
                .margin_bottom(4)
                .margin_start(16)
                .build();
            content.append(&accounts_header);

            let account_list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::Single)
                .css_classes(["navigation-sidebar"])
                .build();

            // "All Accounts" is the first row (always present)
            let all_row = Self::create_account_row(
                "view-list-bullet-symbolic",
                "All Accounts",
                "all",
            );
            account_list.append(&all_row);

            // Select "All Accounts" by default
            if let Some(first_row) = account_list.row_at_index(0) {
                account_list.select_row(Some(&first_row));
            }

            content.append(&account_list);

            // Add Account button
            let add_button = gtk::Button::builder()
                .label("Add Account")
                .css_classes(["flat"])
                .margin_start(8)
                .margin_end(8)
                .margin_top(4)
                .margin_bottom(4)
                .build();
            // Add a + icon
            let add_content = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .halign(gtk::Align::Center)
                .build();
            add_content.append(&gtk::Image::from_icon_name("list-add-symbolic"));
            add_content.append(&gtk::Label::new(Some("Add Account")));
            add_button.set_child(Some(&add_content));
            content.append(&add_button);

            // Separator
            content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

            // --- Mailboxes section ---
            let mailboxes_header = gtk::Label::builder()
                .label("Mailboxes")
                .xalign(0.0)
                .css_classes(["dim-label", "caption", "heading"])
                .margin_top(8)
                .margin_bottom(4)
                .margin_start(16)
                .build();
            content.append(&mailboxes_header);

            let mailbox_list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::Single)
                .css_classes(["navigation-sidebar"])
                .build();

            // Add default Gmail mailboxes
            let mailboxes = [
                ("mail-inbox-symbolic", "INBOX", "Inbox"),
                ("starred-symbolic", "[Gmail]/Starred", "Starred"),
                ("mail-send-symbolic", "[Gmail]/Sent Mail", "Sent"),
                ("document-edit-symbolic", "[Gmail]/Drafts", "Drafts"),
                ("user-trash-symbolic", "[Gmail]/Trash", "Trash"),
                ("mail-mark-junk-symbolic", "[Gmail]/Spam", "Spam"),
                ("mail-archive-symbolic", "[Gmail]/All Mail", "All Mail"),
            ];

            for (icon, imap_name, display_name) in &mailboxes {
                let row = Self::create_mailbox_row(icon, display_name, imap_name);
                mailbox_list.append(&row);
            }

            // Select Inbox by default
            if let Some(first_row) = mailbox_list.row_at_index(0) {
                mailbox_list.select_row(Some(&first_row));
            }
            *self.selected_mailbox.borrow_mut() = "INBOX".to_string();

            content.append(&mailbox_list);

            // Separator before labels
            let labels_sep = gtk::Separator::new(gtk::Orientation::Horizontal);
            labels_sep.set_visible(false); // hidden until labels are loaded
            content.append(&labels_sep);

            // --- Labels section (populated dynamically) ---
            let labels_header = gtk::Label::builder()
                .label("Labels")
                .xalign(0.0)
                .css_classes(["dim-label", "caption", "heading"])
                .margin_top(8)
                .margin_bottom(4)
                .margin_start(16)
                .visible(false) // hidden until labels are loaded
                .build();
            content.append(&labels_header);

            let label_list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::Single)
                .css_classes(["navigation-sidebar"])
                .visible(false) // hidden until labels are loaded
                .build();
            content.append(&label_list);

            scrolled.set_child(Some(&content));
            widget.append(&scrolled);

            *self.account_list.borrow_mut() = Some(account_list);
            *self.mailbox_list.borrow_mut() = Some(mailbox_list);
            *self.label_list.borrow_mut() = Some(label_list);
            *self.labels_header.borrow_mut() = Some(labels_header);
            *self.labels_separator.borrow_mut() = Some(labels_sep);
            *self.add_account_button.borrow_mut() = Some(add_button);
        }
    }

    impl WidgetImpl for MqSidebar {}
    impl BoxImpl for MqSidebar {}

    impl MqSidebar {
        pub(super) fn create_account_row(
            icon_name: &str,
            display_name: &str,
            widget_name: &str,
        ) -> gtk::ListBoxRow {
            let hbox = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .margin_top(6)
                .margin_bottom(6)
                .margin_start(12)
                .margin_end(12)
                .build();

            let icon = gtk::Image::from_icon_name(icon_name);
            hbox.append(&icon);

            let label = gtk::Label::builder()
                .label(display_name)
                .hexpand(true)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            hbox.append(&label);

            let row = gtk::ListBoxRow::builder().child(&hbox).build();
            row.set_widget_name(widget_name);
            row
        }

        pub(super) fn create_mailbox_row(
            icon_name: &str,
            display_name: &str,
            imap_name: &str,
        ) -> gtk::ListBoxRow {
            let hbox = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(12)
                .margin_end(12)
                .build();

            let icon = gtk::Image::from_icon_name(icon_name);
            hbox.append(&icon);

            let label = gtk::Label::builder()
                .label(display_name)
                .hexpand(true)
                .xalign(0.0)
                .build();
            hbox.append(&label);

            // Unread count badge (hidden by default)
            let badge = gtk::Label::builder()
                .css_classes(["dim-label", "caption"])
                .visible(false)
                .build();
            badge.set_widget_name("unread_badge");
            hbox.append(&badge);

            let row = gtk::ListBoxRow::builder().child(&hbox).build();

            // Store the IMAP name as widget name for lookup
            row.set_widget_name(imap_name);

            row
        }
    }
}

glib::wrapper! {
    pub struct MqSidebar(ObjectSubclass<imp::MqSidebar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl MqSidebar {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Populate the account list with loaded accounts.
    ///
    /// The "All Accounts" row is always present as the first entry.
    /// Each account row's widget_name is set to the account's DB id (as string).
    pub fn set_accounts(&self, accounts: &[(i64, String, Option<String>)]) {
        let imp = self.imp();
        if let Some(list_box) = imp.account_list.borrow().as_ref() {
            // Remove all rows except the first one ("All Accounts")
            while let Some(row) = list_box.row_at_index(1) {
                list_box.remove(&row);
            }

            for (id, email, display_name) in accounts {
                let label = display_name
                    .as_deref()
                    .unwrap_or(email.as_str());
                let row = imp::MqSidebar::create_account_row(
                    "avatar-default-symbolic",
                    label,
                    &id.to_string(),
                );
                // Store email as tooltip for the badge
                row.set_tooltip_text(Some(email));
                list_box.append(&row);
            }
        }
    }

    /// Connect a callback for when the selected account changes.
    ///
    /// The callback receives `None` for "All Accounts" or `Some(account_id)`.
    pub fn connect_account_selected<F: Fn(Option<i64>) + 'static>(&self, f: F) {
        let imp = self.imp();
        let selected_id = Rc::clone(&imp.selected_account_id);
        if let Some(list_box) = imp.account_list.borrow().as_ref() {
            list_box.connect_row_selected(move |_, row| {
                if let Some(row) = row {
                    let name = row.widget_name();
                    let account_id = if name == "all" {
                        None
                    } else {
                        name.parse::<i64>().ok()
                    };
                    selected_id.set(account_id);
                    f(account_id);
                }
            });
        }
    }

    /// Connect a callback for when the selected mailbox changes.
    ///
    /// The callback receives the IMAP mailbox name (e.g. "INBOX", "[Gmail]/Sent Mail").
    pub fn connect_mailbox_selected<F: Fn(&str) + 'static>(&self, f: F) {
        let imp = self.imp();
        if let Some(list_box) = imp.mailbox_list.borrow().as_ref() {
            list_box.connect_row_selected(move |_, row| {
                if let Some(row) = row {
                    let mailbox = row.widget_name();
                    f(mailbox.as_str());
                }
            });
        }
    }

    /// Connect a callback for when the "Add Account" button is clicked.
    pub fn connect_add_account<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().add_account_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for when an account is requested to be removed.
    ///
    /// The callback receives (account_id, email). A right-click context menu
    /// with "Remove Account" is shown on each account row (except "All Accounts").
    pub fn connect_account_remove<F: Fn(i64, String) + Clone + 'static>(&self, f: F) {
        let imp = self.imp();
        if let Some(list_box) = imp.account_list.borrow().as_ref() {
            // Attach right-click gesture to the list box
            let cb = f.clone();
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3); // right click
            gesture.connect_pressed(glib::clone!(
                #[weak]
                list_box,
                move |_gesture, _n_press, x, y| {
                    // Find which row was clicked
                    if let Some(row) = list_box.row_at_y(y as i32) {
                        let name = row.widget_name();
                        if name == "all" {
                            return; // Can't remove "All Accounts"
                        }
                        let Some(account_id) = name.parse::<i64>().ok() else {
                            return;
                        };
                        let email = row.tooltip_text().map(|s| s.to_string()).unwrap_or_default();

                        // Build popover menu
                        let menu = gio::Menu::new();
                        menu.append(Some("Remove Account"), Some("sidebar.remove-account"));

                        let popover = gtk::PopoverMenu::from_model(Some(&menu));
                        popover.set_parent(&row);
                        let row_y = row.compute_bounds(&row).map(|b| b.y() as i32).unwrap_or(0);
                        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                            x as i32, y as i32 - row_y, 1, 1,
                        )));
                        popover.set_has_arrow(true);

                        // Register the action on the row
                        let action_group = gio::SimpleActionGroup::new();
                        let cb = cb.clone();
                        let remove_action = gio::SimpleAction::new("remove-account", None);
                        remove_action.connect_activate(move |_, _| {
                            cb(account_id, email.clone());
                        });
                        action_group.add_action(&remove_action);
                        row.insert_action_group("sidebar", Some(&action_group));

                        popover.popup();
                    }
                }
            ));
            list_box.add_controller(gesture);
        }
    }

    /// Get the currently selected mailbox IMAP name.
    pub fn selected_mailbox(&self) -> String {
        let imp = self.imp();
        if let Some(list_box) = imp.mailbox_list.borrow().as_ref() {
            if let Some(row) = list_box.selected_row() {
                return row.widget_name().to_string();
            }
        }
        "INBOX".to_string()
    }

    /// Get the currently selected account ID (None = All Accounts).
    pub fn selected_account_id(&self) -> Option<i64> {
        self.imp().selected_account_id.get()
    }

    /// Populate the labels list with user-defined labels.
    ///
    /// Each tuple is (label_name, imap_name). The labels section is shown
    /// only when there are labels to display.
    pub fn set_labels(&self, labels: &[(String, String)]) {
        let imp = self.imp();

        let has_labels = !labels.is_empty();

        if let Some(header) = imp.labels_header.borrow().as_ref() {
            header.set_visible(has_labels);
        }
        if let Some(sep) = imp.labels_separator.borrow().as_ref() {
            sep.set_visible(has_labels);
        }
        if let Some(list_box) = imp.label_list.borrow().as_ref() {
            list_box.set_visible(has_labels);

            // Clear existing rows
            while let Some(row) = list_box.row_at_index(0) {
                list_box.remove(&row);
            }

            for (name, imap_name) in labels {
                let row = imp::MqSidebar::create_mailbox_row(
                    "tag-symbolic",
                    name,
                    imap_name,
                );
                list_box.append(&row);
            }
        }
    }

    /// Update unread count badges on mailbox and label rows.
    pub fn update_unread_counts(&self, counts: &std::collections::HashMap<String, i64>) {
        // Update both mailbox rows and label rows
        for list_box in [
            self.imp().mailbox_list.borrow().as_ref().cloned(),
            self.imp().label_list.borrow().as_ref().cloned(),
        ]
        .into_iter()
        .flatten()
        {
            let mut i = 0;
            while let Some(row) = list_box.row_at_index(i) {
                let mailbox = row.widget_name().to_string();
                if let Some(child) = row.child() {
                    if let Ok(hbox) = child.downcast::<gtk::Box>() {
                        let mut c = hbox.first_child();
                        while let Some(widget) = c {
                            if widget.widget_name() == "unread_badge" {
                                if let Ok(label) = widget.downcast::<gtk::Label>() {
                                    let count = counts.get(&mailbox).copied().unwrap_or(0);
                                    if count > 0 {
                                        label.set_label(&count.to_string());
                                        label.set_visible(true);
                                    } else {
                                        label.set_visible(false);
                                    }
                                }
                                break;
                            }
                            c = widget.next_sibling();
                        }
                    }
                }
                i += 1;
            }
        }
    }

    /// Connect a callback for when a label is selected.
    ///
    /// The callback receives the label's IMAP name.
    pub fn connect_label_selected<F: Fn(&str) + 'static>(&self, f: F) {
        let imp = self.imp();
        if let Some(list_box) = imp.label_list.borrow().as_ref() {
            list_box.connect_row_selected(move |_, row| {
                if let Some(row) = row {
                    let label = row.widget_name();
                    f(label.as_str());
                }
            });
        }
    }
}

impl Default for MqSidebar {
    fn default() -> Self {
        Self::new()
    }
}
