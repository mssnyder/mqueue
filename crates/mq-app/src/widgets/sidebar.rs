//! Sidebar widget with account selector and mailbox/label navigation.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqSidebar {
        pub account_list: RefCell<Option<gtk::ListBox>>,
        pub mailbox_list: RefCell<Option<gtk::ListBox>>,
        pub add_account_button: RefCell<Option<gtk::Button>>,
        pub selected_mailbox: RefCell<String>,
        /// None = All Accounts, Some(id) = specific account.
        pub selected_account_id: RefCell<Option<i64>>,
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

            // Header with app name
            let header = adw::HeaderBar::builder()
                .title_widget(&adw::WindowTitle::new("m'Queue", ""))
                .show_end_title_buttons(false)
                .build();
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
                "mail-inbox-symbolic",
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

            scrolled.set_child(Some(&content));
            widget.append(&scrolled);

            *self.account_list.borrow_mut() = Some(account_list);
            *self.mailbox_list.borrow_mut() = Some(mailbox_list);
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
        let selected_id = imp.selected_account_id.clone();
        if let Some(list_box) = imp.account_list.borrow().as_ref() {
            list_box.connect_row_selected(move |_, row| {
                if let Some(row) = row {
                    let name = row.widget_name();
                    let account_id = if name == "all" {
                        None
                    } else {
                        name.parse::<i64>().ok()
                    };
                    *selected_id.borrow_mut() = account_id;
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
        *self.imp().selected_account_id.borrow()
    }
}

impl Default for MqSidebar {
    fn default() -> Self {
        Self::new()
    }
}
