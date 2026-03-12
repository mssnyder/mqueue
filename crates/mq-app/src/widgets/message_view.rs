//! Message view widget for displaying email content.
//!
//! Phase 2: text-based display with action buttons.
//! Phase 5: WebKitGTK sandboxed HTML rendering.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqMessageView {
        pub from_label: RefCell<Option<gtk::Label>>,
        pub to_label: RefCell<Option<gtk::Label>>,
        pub date_label: RefCell<Option<gtk::Label>>,
        pub subject_label: RefCell<Option<gtk::Label>>,
        pub body_view: RefCell<Option<gtk::TextView>>,
        pub unsub_button: RefCell<Option<gtk::Button>>,
        pub placeholder: RefCell<Option<adw::StatusPage>>,
        pub content_box: RefCell<Option<gtk::Box>>,
        pub star_button: RefCell<Option<gtk::ToggleButton>>,
        pub read_button: RefCell<Option<gtk::ToggleButton>>,
        pub archive_button: RefCell<Option<gtk::Button>>,
        pub delete_button: RefCell<Option<gtk::Button>>,
        pub loading_spinner: RefCell<Option<gtk::Spinner>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MqMessageView {
        const NAME: &'static str = "MqMessageView";
        type Type = super::MqMessageView;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for MqMessageView {
        fn constructed(&self) {
            self.parent_constructed();

            let widget = self.obj();
            widget.set_orientation(gtk::Orientation::Vertical);

            // Header bar with action buttons
            let header = adw::HeaderBar::builder()
                .show_start_title_buttons(false)
                .build();
            header.set_title_widget(Some(&adw::WindowTitle::new("Message", "")));

            // Action buttons in header bar
            let star_button = gtk::ToggleButton::builder()
                .icon_name("starred-symbolic")
                .tooltip_text("Star")
                .build();

            let read_button = gtk::ToggleButton::builder()
                .icon_name("mail-read-symbolic")
                .tooltip_text("Mark as read/unread")
                .build();

            let archive_button = gtk::Button::builder()
                .icon_name("folder-symbolic")
                .tooltip_text("Archive")
                .build();

            let delete_button = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text("Delete")
                .css_classes(["destructive-action"])
                .build();

            header.pack_end(&delete_button);
            header.pack_end(&archive_button);
            header.pack_end(&read_button);
            header.pack_end(&star_button);

            widget.append(&header);

            // Stack: placeholder (no message selected) vs content vs loading
            let stack = gtk::Stack::new();

            // Placeholder
            let placeholder = adw::StatusPage::builder()
                .icon_name("mail-read-symbolic")
                .title("No message selected")
                .description("Select a message from the list to read it.")
                .vexpand(true)
                .build();
            stack.add_named(&placeholder, Some("placeholder"));

            // Loading spinner
            let loading_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .valign(gtk::Align::Center)
                .halign(gtk::Align::Center)
                .vexpand(true)
                .spacing(12)
                .build();
            let spinner = gtk::Spinner::builder()
                .spinning(true)
                .width_request(32)
                .height_request(32)
                .build();
            loading_box.append(&spinner);
            loading_box.append(&gtk::Label::new(Some("Loading message…")));
            stack.add_named(&loading_box, Some("loading"));

            // Content area
            let scrolled = gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build();

            let content = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(8)
                .margin_top(16)
                .margin_bottom(16)
                .margin_start(16)
                .margin_end(16)
                .build();

            // Email header section
            let header_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .css_classes(["card"])
                .margin_bottom(8)
                .build();

            let header_inner = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .margin_top(12)
                .margin_bottom(12)
                .margin_start(12)
                .margin_end(12)
                .build();

            // Subject
            let subject_label = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .css_classes(["title-2"])
                .selectable(true)
                .build();
            header_inner.append(&subject_label);

            // From
            let from_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(4)
                .build();
            from_box.append(
                &gtk::Label::builder()
                    .label("From:")
                    .css_classes(["dim-label"])
                    .build(),
            );
            let from_label = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .selectable(true)
                .build();
            from_box.append(&from_label);
            header_inner.append(&from_box);

            // To
            let to_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(4)
                .build();
            to_box.append(
                &gtk::Label::builder()
                    .label("To:")
                    .css_classes(["dim-label"])
                    .build(),
            );
            let to_label = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .selectable(true)
                .build();
            to_box.append(&to_label);
            header_inner.append(&to_box);

            // Date
            let date_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(4)
                .build();
            date_box.append(
                &gtk::Label::builder()
                    .label("Date:")
                    .css_classes(["dim-label"])
                    .build(),
            );
            let date_label = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .selectable(true)
                .build();
            date_box.append(&date_label);
            header_inner.append(&date_box);

            header_box.append(&header_inner);

            // Unsubscribe button (hidden by default)
            let unsub_button = gtk::Button::builder()
                .label("Unsubscribe")
                .css_classes(["destructive-action"])
                .halign(gtk::Align::Start)
                .visible(false)
                .margin_start(12)
                .margin_bottom(8)
                .build();
            header_box.append(&unsub_button);

            content.append(&header_box);

            // Body text view
            let body_view = gtk::TextView::builder()
                .editable(false)
                .cursor_visible(false)
                .wrap_mode(gtk::WrapMode::WordChar)
                .vexpand(true)
                .css_classes(["body"])
                .top_margin(8)
                .bottom_margin(8)
                .left_margin(8)
                .right_margin(8)
                .build();
            content.append(&body_view);

            scrolled.set_child(Some(&content));
            stack.add_named(&scrolled, Some("content"));

            // Show placeholder by default
            stack.set_visible_child_name("placeholder");

            widget.append(&stack);

            // Store references
            *self.from_label.borrow_mut() = Some(from_label);
            *self.to_label.borrow_mut() = Some(to_label);
            *self.date_label.borrow_mut() = Some(date_label);
            *self.subject_label.borrow_mut() = Some(subject_label);
            *self.body_view.borrow_mut() = Some(body_view);
            *self.unsub_button.borrow_mut() = Some(unsub_button);
            *self.placeholder.borrow_mut() = Some(placeholder);
            *self.content_box.borrow_mut() = Some(content);
            *self.star_button.borrow_mut() = Some(star_button);
            *self.read_button.borrow_mut() = Some(read_button);
            *self.archive_button.borrow_mut() = Some(archive_button);
            *self.delete_button.borrow_mut() = Some(delete_button);
            *self.loading_spinner.borrow_mut() = Some(spinner);
        }
    }

    impl WidgetImpl for MqMessageView {}
    impl BoxImpl for MqMessageView {}
}

glib::wrapper! {
    pub struct MqMessageView(ObjectSubclass<imp::MqMessageView>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl MqMessageView {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Show the email with full details.
    pub fn show_message(
        &self,
        from: &str,
        to: &str,
        date: &str,
        subject: &str,
        body_text: &str,
        has_unsubscribe: bool,
        is_flagged: bool,
        is_read: bool,
    ) {
        let imp = self.imp();

        if let Some(label) = imp.from_label.borrow().as_ref() {
            label.set_label(from);
        }
        if let Some(label) = imp.to_label.borrow().as_ref() {
            label.set_label(to);
        }
        if let Some(label) = imp.date_label.borrow().as_ref() {
            label.set_label(date);
        }
        if let Some(label) = imp.subject_label.borrow().as_ref() {
            label.set_label(subject);
        }
        if let Some(tv) = imp.body_view.borrow().as_ref() {
            tv.buffer().set_text(body_text);
        }
        if let Some(btn) = imp.unsub_button.borrow().as_ref() {
            btn.set_visible(has_unsubscribe);
        }

        // Update toggle button states (without triggering signal handlers)
        if let Some(btn) = imp.star_button.borrow().as_ref() {
            btn.set_active(is_flagged);
        }
        if let Some(btn) = imp.read_button.borrow().as_ref() {
            btn.set_active(is_read);
        }

        self.show_content();
    }

    /// Show a loading spinner while fetching the message body.
    pub fn show_loading(&self) {
        if let Some(stack) = self.find_stack() {
            stack.set_visible_child_name("loading");
        }
    }

    /// Show the placeholder (no message selected).
    pub fn show_placeholder(&self) {
        if let Some(stack) = self.find_stack() {
            stack.set_visible_child_name("placeholder");
        }
    }

    /// Show the content view.
    fn show_content(&self) {
        if let Some(stack) = self.find_stack() {
            stack.set_visible_child_name("content");
        }
    }

    /// Set the body text content.
    pub fn set_body_text(&self, text: &str) {
        let imp = self.imp();
        if let Some(tv) = imp.body_view.borrow().as_ref() {
            tv.buffer().set_text(text);
        }
    }

    /// Connect a callback for when the star button is toggled.
    pub fn connect_star_toggled<F: Fn(bool) + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().star_button.borrow().as_ref() {
            btn.connect_toggled(move |btn| {
                f(btn.is_active());
            });
        }
    }

    /// Connect a callback for when the read button is toggled.
    pub fn connect_read_toggled<F: Fn(bool) + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().read_button.borrow().as_ref() {
            btn.connect_toggled(move |btn| {
                f(btn.is_active());
            });
        }
    }

    /// Connect a callback for the archive button.
    pub fn connect_archive_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().archive_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Connect a callback for the delete button.
    pub fn connect_delete_clicked<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().delete_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    fn find_stack(&self) -> Option<gtk::Stack> {
        let mut child = self.first_child();
        while let Some(c) = child {
            if let Ok(stack) = c.clone().downcast::<gtk::Stack>() {
                return Some(stack);
            }
            child = c.next_sibling();
        }
        None
    }
}

impl Default for MqMessageView {
    fn default() -> Self {
        Self::new()
    }
}
