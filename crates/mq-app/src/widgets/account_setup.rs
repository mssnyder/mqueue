//! Account setup dialog for adding Gmail accounts via OAuth2.
//!
//! Uses `adw::Dialog` so the sheet is rendered *inside* the parent window —
//! guaranteed to be centered regardless of compositor / tiling WM behaviour.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::RefCell;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct MqAccountSetup {
        pub sign_in_button: RefCell<Option<gtk::Button>>,
        pub spinner: RefCell<Option<gtk::Spinner>>,
        pub stack: RefCell<Option<gtk::Stack>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MqAccountSetup {
        const NAME: &'static str = "MqAccountSetup";
        type Type = super::MqAccountSetup;
        type ParentType = adw::Dialog;
    }

    impl ObjectImpl for MqAccountSetup {
        fn constructed(&self) {
            self.parent_constructed();

            let dialog = self.obj();
            dialog.set_title("Add Gmail Account");
            dialog.set_content_width(400);
            dialog.set_content_height(460);

            let stack = gtk::Stack::builder()
                .transition_type(gtk::StackTransitionType::Crossfade)
                .build();

            // --- Welcome page ---
            let welcome_page = adw::StatusPage::builder()
                .icon_name("mail-send-symbolic")
                .title("Sign in with Gmail")
                .description(
                    "m'Queue needs access to your Gmail account to sync your email.\n\n\
                     Click below to open Google's sign-in page in your browser.",
                )
                .build();

            let button_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .halign(gtk::Align::Center)
                .spacing(12)
                .build();

            let sign_in_button = gtk::Button::builder()
                .label("Sign in with Google")
                .css_classes(["suggested-action", "pill"])
                .halign(gtk::Align::Center)
                .build();
            button_box.append(&sign_in_button);

            welcome_page.set_child(Some(&button_box));
            stack.add_named(&welcome_page, Some("welcome"));

            // --- Loading page ---
            let loading_page = adw::StatusPage::builder()
                .title("Waiting for sign-in\u{2026}")
                .description("Complete the sign-in in your browser, then return here.")
                .build();

            let spinner = gtk::Spinner::builder()
                .spinning(true)
                .width_request(48)
                .height_request(48)
                .halign(gtk::Align::Center)
                .build();
            loading_page.set_child(Some(&spinner));
            stack.add_named(&loading_page, Some("loading"));

            // --- Success page ---
            let success_page = adw::StatusPage::builder()
                .icon_name("emblem-ok-symbolic")
                .title("Account Added!")
                .description("Your Gmail account has been connected successfully.")
                .build();
            stack.add_named(&success_page, Some("success"));

            // --- Error page ---
            let error_page = adw::StatusPage::builder()
                .icon_name("dialog-error-symbolic")
                .title("Authentication Failed")
                .description("Something went wrong. Please try again.")
                .build();

            let retry_button = gtk::Button::builder()
                .label("Try Again")
                .css_classes(["suggested-action", "pill"])
                .halign(gtk::Align::Center)
                .build();

            let stack_clone = stack.clone();
            retry_button.connect_clicked(move |_| {
                stack_clone.set_visible_child_name("welcome");
            });

            error_page.set_child(Some(&retry_button));
            stack.add_named(&error_page, Some("error"));

            stack.set_visible_child_name("welcome");

            // adw::Dialog provides its own header bar; just set the child content.
            dialog.set_child(Some(&stack));

            *self.sign_in_button.borrow_mut() = Some(sign_in_button);
            *self.spinner.borrow_mut() = Some(spinner);
            *self.stack.borrow_mut() = Some(stack);
        }
    }

    impl WidgetImpl for MqAccountSetup {}
    impl AdwDialogImpl for MqAccountSetup {}
}

glib::wrapper! {
    pub struct MqAccountSetup(ObjectSubclass<imp::MqAccountSetup>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl MqAccountSetup {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    /// Connect the sign-in button click. The callback receives no arguments;
    /// the caller is responsible for starting the OAuth flow.
    pub fn connect_sign_in<F: Fn() + 'static>(&self, f: F) {
        if let Some(btn) = self.imp().sign_in_button.borrow().as_ref() {
            btn.connect_clicked(move |_| f());
        }
    }

    /// Switch to the "waiting for browser" state.
    pub fn show_loading(&self) {
        if let Some(stack) = self.imp().stack.borrow().as_ref() {
            stack.set_visible_child_name("loading");
        }
    }

    /// Switch to the success state and auto-close after 2 seconds.
    pub fn show_success(&self, email: &str) {
        if let Some(stack) = self.imp().stack.borrow().as_ref() {
            if let Some(child) = stack.child_by_name("success") {
                if let Ok(page) = child.downcast::<adw::StatusPage>() {
                    page.set_description(Some(&format!(
                        "{email} has been connected successfully."
                    )));
                }
            }
            stack.set_visible_child_name("success");
        }

        let dialog = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            dialog.close();
        });
    }

    /// Switch to the error state with a message.
    pub fn show_error(&self, message: &str) {
        if let Some(stack) = self.imp().stack.borrow().as_ref() {
            if let Some(child) = stack.child_by_name("error") {
                if let Ok(page) = child.downcast::<adw::StatusPage>() {
                    page.set_description(Some(message));
                }
            }
            stack.set_visible_child_name("error");
        }
    }
}
