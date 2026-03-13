//! Compose window for writing and sending emails.
//!
//! Supports new messages, replies, reply-all, and forwarding.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

/// The kind of compose action.
#[derive(Debug, Clone, Default)]
pub enum ComposeMode {
    #[default]
    New,
    Reply {
        from: String,
        subject: String,
        date: String,
        body: String,
        message_id: Option<String>,
        references: Option<String>,
    },
    ReplyAll {
        from: String,
        to: Vec<String>,
        cc: Vec<String>,
        subject: String,
        date: String,
        body: String,
        message_id: Option<String>,
        references: Option<String>,
    },
    Forward {
        from: String,
        subject: String,
        date: String,
        to: String,
        body: String,
    },
}

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqComposeWindow {
        pub from_dropdown: RefCell<Option<gtk::DropDown>>,
        pub to_entry: RefCell<Option<gtk::Entry>>,
        pub cc_entry: RefCell<Option<gtk::Entry>>,
        pub bcc_entry: RefCell<Option<gtk::Entry>>,
        pub subject_entry: RefCell<Option<gtk::Entry>>,
        pub body_view: RefCell<Option<gtk::TextView>>,
        pub send_button: RefCell<Option<gtk::Button>>,
        pub cc_row: RefCell<Option<gtk::Box>>,
        pub bcc_row: RefCell<Option<gtk::Box>>,
        pub accounts: RefCell<Vec<(i64, String)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MqComposeWindow {
        const NAME: &'static str = "MqComposeWindow";
        type Type = super::MqComposeWindow;
        type ParentType = adw::Window;
    }

    impl ObjectImpl for MqComposeWindow {
        fn constructed(&self) {
            self.parent_constructed();

            let window = self.obj();
            window.set_title(Some("New Message"));
            window.set_default_size(640, 500);
            window.set_size_request(360, 350);

            let main_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .build();

            // Header bar
            let header = adw::HeaderBar::new();

            let send_button = gtk::Button::builder()
                .label("Send")
                .css_classes(["suggested-action"])
                .build();
            header.pack_end(&send_button);

            // Cc/Bcc toggle button
            let cc_toggle = gtk::ToggleButton::builder()
                .label("Cc/Bcc")
                .build();
            header.pack_end(&cc_toggle);

            main_box.append(&header);

            // Form fields
            let form = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(0)
                .margin_start(12)
                .margin_end(12)
                .margin_top(8)
                .build();

            // From
            let from_row = Self::make_field_row("From:");
            let from_dropdown = gtk::DropDown::from_strings(&[]);
            from_dropdown.set_hexpand(true);
            from_row.append(&from_dropdown);
            form.append(&from_row);

            // To
            let to_row = Self::make_field_row("To:");
            let to_entry = gtk::Entry::builder()
                .hexpand(true)
                .placeholder_text("recipient@example.com")
                .build();
            to_row.append(&to_entry);
            form.append(&to_row);

            // Cc (hidden by default)
            let cc_row = Self::make_field_row("Cc:");
            let cc_entry = gtk::Entry::builder()
                .hexpand(true)
                .placeholder_text("cc@example.com")
                .build();
            cc_row.append(&cc_entry);
            cc_row.set_visible(false);
            form.append(&cc_row);

            // Bcc (hidden by default)
            let bcc_row = Self::make_field_row("Bcc:");
            let bcc_entry = gtk::Entry::builder()
                .hexpand(true)
                .placeholder_text("bcc@example.com")
                .build();
            bcc_row.append(&bcc_entry);
            bcc_row.set_visible(false);
            form.append(&bcc_row);

            // Subject
            let subject_row = Self::make_field_row("Subject:");
            let subject_entry = gtk::Entry::builder()
                .hexpand(true)
                .build();
            subject_row.append(&subject_entry);
            form.append(&subject_row);

            main_box.append(&form);

            // Separator
            main_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

            // Body text view
            let scrolled = gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build();

            let body_view = gtk::TextView::builder()
                .wrap_mode(gtk::WrapMode::WordChar)
                .top_margin(12)
                .bottom_margin(12)
                .left_margin(12)
                .right_margin(12)
                .build();
            scrolled.set_child(Some(&body_view));
            main_box.append(&scrolled);

            window.set_content(Some(&main_box));

            // Wire Cc/Bcc toggle
            let cc_row_clone = cc_row.clone();
            let bcc_row_clone = bcc_row.clone();
            cc_toggle.connect_toggled(move |btn| {
                let show = btn.is_active();
                cc_row_clone.set_visible(show);
                bcc_row_clone.set_visible(show);
            });

            // Store references
            *self.from_dropdown.borrow_mut() = Some(from_dropdown);
            *self.to_entry.borrow_mut() = Some(to_entry);
            *self.cc_entry.borrow_mut() = Some(cc_entry);
            *self.bcc_entry.borrow_mut() = Some(bcc_entry);
            *self.subject_entry.borrow_mut() = Some(subject_entry);
            *self.body_view.borrow_mut() = Some(body_view);
            *self.send_button.borrow_mut() = Some(send_button);
            *self.cc_row.borrow_mut() = Some(cc_row);
            *self.bcc_row.borrow_mut() = Some(bcc_row);
        }
    }

    impl WidgetImpl for MqComposeWindow {}
    impl WindowImpl for MqComposeWindow {}
    impl AdwWindowImpl for MqComposeWindow {}

    impl MqComposeWindow {
        fn make_field_row(label_text: &str) -> gtk::Box {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_top(4)
                .margin_bottom(4)
                .build();
            let label = gtk::Label::builder()
                .label(label_text)
                .width_chars(8)
                .xalign(1.0)
                .css_classes(["dim-label"])
                .build();
            row.append(&label);
            row
        }
    }
}

glib::wrapper! {
    pub struct MqComposeWindow(ObjectSubclass<imp::MqComposeWindow>)
        @extends adw::Window, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl MqComposeWindow {
    pub fn new(parent: &impl IsA<gtk::Window>) -> Self {
        let win: Self = glib::Object::builder().build();
        win.set_transient_for(Some(parent));
        win
    }

    /// Set the available accounts in the From dropdown.
    /// Returns the list of account IDs in dropdown order.
    pub fn set_accounts(&self, accounts: &[(i64, String)]) {
        let imp = self.imp();
        let labels: Vec<&str> = accounts.iter().map(|(_, email)| email.as_str()).collect();
        if let Some(dropdown) = imp.from_dropdown.borrow().as_ref() {
            dropdown.set_model(Some(&gtk::StringList::new(&labels)));
        }
        *imp.accounts.borrow_mut() = accounts.to_vec();
    }

    /// Select a specific account in the From dropdown by account ID.
    pub fn select_account(&self, account_id: i64) {
        let imp = self.imp();
        let accounts = imp.accounts.borrow();
        if let Some(pos) = accounts.iter().position(|(id, _)| *id == account_id) {
            if let Some(dropdown) = imp.from_dropdown.borrow().as_ref() {
                dropdown.set_selected(pos as u32);
            }
        }
    }

    /// Get the currently selected account ID.
    pub fn selected_account(&self) -> Option<(i64, String)> {
        let imp = self.imp();
        let accounts = imp.accounts.borrow();
        if let Some(dropdown) = imp.from_dropdown.borrow().as_ref() {
            let idx = dropdown.selected() as usize;
            return accounts.get(idx).cloned();
        }
        None
    }

    /// Set the To field.
    pub fn set_to(&self, to: &str) {
        if let Some(entry) = self.imp().to_entry.borrow().as_ref() {
            entry.set_text(to);
        }
    }

    /// Set the Cc field and make it visible.
    pub fn set_cc(&self, cc: &str) {
        let imp = self.imp();
        if let Some(entry) = imp.cc_entry.borrow().as_ref() {
            entry.set_text(cc);
        }
        if !cc.is_empty() {
            if let Some(row) = imp.cc_row.borrow().as_ref() {
                row.set_visible(true);
            }
            if let Some(row) = imp.bcc_row.borrow().as_ref() {
                row.set_visible(true);
            }
        }
    }

    /// Set the Subject field.
    pub fn set_subject(&self, subject: &str) {
        if let Some(entry) = self.imp().subject_entry.borrow().as_ref() {
            entry.set_text(subject);
        }
    }

    /// Set the body text.
    pub fn set_body(&self, text: &str) {
        if let Some(tv) = self.imp().body_view.borrow().as_ref() {
            tv.buffer().set_text(text);
        }
    }

    /// Get the To addresses (comma-separated string → Vec).
    pub fn to_addresses(&self) -> Vec<String> {
        self.imp()
            .to_entry
            .borrow()
            .as_ref()
            .map(|e| parse_addresses(&e.text()))
            .unwrap_or_default()
    }

    /// Get the Cc addresses.
    pub fn cc_addresses(&self) -> Vec<String> {
        self.imp()
            .cc_entry
            .borrow()
            .as_ref()
            .map(|e| parse_addresses(&e.text()))
            .unwrap_or_default()
    }

    /// Get the Bcc addresses.
    pub fn bcc_addresses(&self) -> Vec<String> {
        self.imp()
            .bcc_entry
            .borrow()
            .as_ref()
            .map(|e| parse_addresses(&e.text()))
            .unwrap_or_default()
    }

    /// Get the subject text.
    pub fn subject(&self) -> String {
        self.imp()
            .subject_entry
            .borrow()
            .as_ref()
            .map(|e| e.text().to_string())
            .unwrap_or_default()
    }

    /// Get the body text.
    pub fn body_text(&self) -> String {
        self.imp()
            .body_view
            .borrow()
            .as_ref()
            .map(|tv| {
                let buf = tv.buffer();
                buf.text(&buf.start_iter(), &buf.end_iter(), false)
                    .to_string()
            })
            .unwrap_or_default()
    }

    /// Connect a callback for the Send button.
    pub fn connect_send<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().send_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Apply a compose mode: pre-fill fields for reply, reply-all, or forward.
    pub fn apply_mode(&self, mode: &ComposeMode, signature: &str) {
        match mode {
            ComposeMode::New => {
                self.set_title(Some("New Message"));
                if !signature.is_empty() {
                    self.set_body(&format!("\n\n-- \n{signature}"));
                }
            }
            ComposeMode::Reply {
                from,
                subject,
                date,
                body,
                ..
            } => {
                self.set_title(Some("Reply"));
                self.set_to(from);
                self.set_subject(&reply_subject(subject));
                let quoted = quote_body(from, date, body);
                if signature.is_empty() {
                    self.set_body(&format!("\n\n{quoted}"));
                } else {
                    self.set_body(&format!("\n\n-- \n{signature}\n\n{quoted}"));
                }
            }
            ComposeMode::ReplyAll {
                from,
                to,
                cc,
                subject,
                date,
                body,
                ..
            } => {
                self.set_title(Some("Reply All"));
                self.set_to(from);
                // Merge original To + Cc into Cc (excluding the sender)
                let mut all_cc: Vec<String> = to
                    .iter()
                    .chain(cc.iter())
                    .filter(|a| a != &from)
                    .cloned()
                    .collect();
                all_cc.dedup();
                if !all_cc.is_empty() {
                    self.set_cc(&all_cc.join(", "));
                }
                self.set_subject(&reply_subject(subject));
                let quoted = quote_body(from, date, body);
                if signature.is_empty() {
                    self.set_body(&format!("\n\n{quoted}"));
                } else {
                    self.set_body(&format!("\n\n-- \n{signature}\n\n{quoted}"));
                }
            }
            ComposeMode::Forward {
                from,
                subject,
                date,
                to,
                body,
            } => {
                self.set_title(Some("Forward"));
                self.set_subject(&forward_subject(subject));
                let fwd = format!(
                    "---------- Forwarded message ----------\n\
                     From: {from}\n\
                     Date: {date}\n\
                     Subject: {subject}\n\
                     To: {to}\n\
                     \n\
                     {body}"
                );
                if signature.is_empty() {
                    self.set_body(&format!("\n\n{fwd}"));
                } else {
                    self.set_body(&format!("\n\n-- \n{signature}\n\n{fwd}"));
                }
            }
        }
    }

    /// Get In-Reply-To and References headers from the compose mode.
    pub fn reply_headers(mode: &ComposeMode) -> (Option<String>, Option<String>) {
        match mode {
            ComposeMode::Reply {
                message_id,
                references,
                ..
            }
            | ComposeMode::ReplyAll {
                message_id,
                references,
                ..
            } => {
                let in_reply_to = message_id.clone();
                let new_refs = match (references, message_id) {
                    (Some(refs), Some(mid)) => Some(format!("{refs} {mid}")),
                    (None, Some(mid)) => Some(mid.clone()),
                    (Some(refs), None) => Some(refs.clone()),
                    (None, None) => None,
                };
                (in_reply_to, new_refs)
            }
            _ => (None, None),
        }
    }
}

fn parse_addresses(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn reply_subject(subject: &str) -> String {
    if subject.to_lowercase().starts_with("re:") {
        subject.to_string()
    } else {
        format!("Re: {subject}")
    }
}

fn forward_subject(subject: &str) -> String {
    if subject.to_lowercase().starts_with("fwd:") {
        subject.to_string()
    } else {
        format!("Fwd: {subject}")
    }
}

fn quote_body(from: &str, date: &str, body: &str) -> String {
    let quoted_lines: String = body
        .lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("On {date}, {from} wrote:\n{quoted_lines}")
}
