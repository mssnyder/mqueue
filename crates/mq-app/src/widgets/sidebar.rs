//! Sidebar widget with mailbox/label navigation.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqSidebar {
        pub mailbox_list: RefCell<Option<gtk::ListBox>>,
        pub selected_mailbox: RefCell<String>,
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

            // Scrolled area for mailbox list
            let scrolled = gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build();

            let list_box = gtk::ListBox::builder()
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
                list_box.append(&row);
            }

            // Select Inbox by default
            if let Some(first_row) = list_box.row_at_index(0) {
                list_box.select_row(Some(&first_row));
            }
            *self.selected_mailbox.borrow_mut() = "INBOX".to_string();

            scrolled.set_child(Some(&list_box));
            widget.append(&scrolled);

            *self.mailbox_list.borrow_mut() = Some(list_box);
        }
    }

    impl WidgetImpl for MqSidebar {}
    impl BoxImpl for MqSidebar {}

    impl MqSidebar {
        fn create_mailbox_row(
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
}

impl Default for MqSidebar {
    fn default() -> Self {
        Self::new()
    }
}
