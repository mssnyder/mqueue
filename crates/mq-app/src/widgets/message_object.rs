//! GObject wrapper for message data, used as the model item in the ListView.

use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::Properties;
use gtk::glib;
use std::cell::{Cell, RefCell};

mod imp {
    use super::*;

    #[derive(Debug, Default, Properties)]
    #[properties(wrapper_type = super::MessageObject)]
    pub struct MessageObject {
        #[property(get, set)]
        db_id: Cell<i64>,

        #[property(get, set)]
        uid: Cell<u32>,

        #[property(get, set)]
        sender_name: RefCell<String>,

        #[property(get, set)]
        sender_email: RefCell<String>,

        #[property(get, set)]
        subject: RefCell<String>,

        #[property(get, set)]
        date: RefCell<String>,

        #[property(get, set)]
        snippet: RefCell<String>,

        #[property(get, set)]
        is_read: Cell<bool>,

        #[property(get, set)]
        is_flagged: Cell<bool>,

        #[property(get, set)]
        has_attachments: Cell<bool>,

        #[property(get, set)]
        mailbox: RefCell<String>,

        #[property(get, set)]
        account_id: Cell<i64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageObject {
        const NAME: &'static str = "MqMessageObject";
        type Type = super::MessageObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageObject {}
}

glib::wrapper! {
    pub struct MessageObject(ObjectSubclass<imp::MessageObject>);
}

impl MessageObject {
    pub fn new(
        db_id: i64,
        uid: u32,
        sender_name: &str,
        sender_email: &str,
        subject: &str,
        date: &str,
        snippet: &str,
        is_read: bool,
        is_flagged: bool,
        has_attachments: bool,
        mailbox: &str,
        account_id: i64,
    ) -> Self {
        glib::Object::builder()
            .property("db-id", db_id)
            .property("uid", uid)
            .property("sender-name", sender_name)
            .property("sender-email", sender_email)
            .property("subject", subject)
            .property("date", date)
            .property("snippet", snippet)
            .property("is-read", is_read)
            .property("is-flagged", is_flagged)
            .property("has-attachments", has_attachments)
            .property("mailbox", mailbox)
            .property("account-id", account_id)
            .build()
    }
}
