use std::sync::Arc;

use adw::prelude::*;
use gtk::gio;
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::config;
use crate::runtime;
use crate::widgets::message_object::MessageObject;
use crate::widgets::window::MqWindow;

/// Run the GTK application, returning the exit code.
pub fn run() -> i32 {
    let app = adw::Application::builder()
        .application_id(config::APP_ID)
        .flags(gio::ApplicationFlags::default())
        .build();

    app.connect_startup(|_app| {
        info!("Application startup");
        load_css();
    });

    app.connect_activate(|app| {
        info!("Application activate");
        let window = MqWindow::new(app);
        window.present();

        // Initialize database and load data
        setup_data(&window);
    });

    app.run().into()
}

/// Set up data layer: init DB, load accounts, trigger sync.
fn setup_data(window: &MqWindow) {
    let window_clone = window.clone();

    runtime::spawn_async(
        async move {
            // Initialize database
            let _config = mq_core::config::AppConfig::load().unwrap_or_default();
            let db_path = mq_core::config::AppConfig::data_dir().join("mq-mail.db");

            let pool = match mq_db::init_pool(&db_path).await {
                Ok(pool) => pool,
                Err(e) => {
                    error!("Failed to initialize database: {e}");
                    return Err(format!("Database error: {e}"));
                }
            };

            // Check for existing accounts
            let accounts = mq_db::queries::accounts::get_all_accounts(&pool)
                .await
                .unwrap_or_default();

            if accounts.is_empty() {
                info!("No accounts configured — showing welcome screen");
                return Ok(AppData {
                    has_accounts: false,
                    messages: vec![],
                    pool: Arc::new(pool),
                });
            }

            info!(
                count = accounts.len(),
                "Found existing accounts, loading messages"
            );

            // Load messages for the first account's inbox
            let account = &accounts[0];
            let messages = mq_db::queries::messages::get_messages_for_mailbox(
                &pool,
                account.id,
                "INBOX",
                100,
                0,
            )
            .await
            .unwrap_or_default();

            info!(count = messages.len(), "Loaded cached messages");

            Ok(AppData {
                has_accounts: true,
                messages: messages
                    .into_iter()
                    .map(|m| MessageData {
                        db_id: m.id,
                        uid: m.uid as u32,
                        sender_name: m.sender_name.unwrap_or_default(),
                        sender_email: m.sender_email,
                        subject: m.subject.unwrap_or_default(),
                        date: m.date,
                        snippet: m.snippet.unwrap_or_default(),
                        is_read: m.flags.contains("\\Seen"),
                        is_flagged: m.flags.contains("\\Flagged"),
                        has_attachments: m.has_attachments,
                        mailbox: m.mailbox,
                        account_id: m.account_id,
                        recipient_to: m.recipient_to,
                        list_unsubscribe: m.list_unsubscribe,
                    })
                    .collect(),
                pool: Arc::new(pool),
            })
        },
        move |result: Result<AppData, String>| {
            match result {
                Ok(data) => {
                    if !data.has_accounts {
                        return;
                    }

                    let pool = data.pool.clone();

                    // Populate the message list with cached data
                    let message_list = window_clone.message_list();
                    let objects: Vec<MessageObject> = data
                        .messages
                        .iter()
                        .map(|m| {
                            MessageObject::new(
                                m.db_id,
                                m.uid,
                                &m.sender_name,
                                &m.sender_email,
                                &m.subject,
                                &m.date,
                                &m.snippet,
                                m.is_read,
                                m.is_flagged,
                                m.has_attachments,
                                &m.mailbox,
                                m.account_id,
                            )
                        })
                        .collect();
                    message_list.set_messages(objects);

                    // Wire message selection to load full message details
                    let message_view = window_clone.message_view();
                    let messages_data = Arc::new(data.messages);
                    message_list.connect_message_selected(move |msg| {
                        let db_id = msg.db_id();
                        let from = if msg.sender_name().is_empty() {
                            msg.sender_email()
                        } else {
                            format!("{} <{}>", msg.sender_name(), msg.sender_email())
                        };

                        // Find the full message data for To/unsubscribe fields
                        let msg_data = messages_data.iter().find(|m| m.db_id == db_id);
                        let to = msg_data
                            .map(|m| m.recipient_to.as_str())
                            .unwrap_or("");
                        let has_unsub = msg_data
                            .map(|m| m.list_unsubscribe.is_some())
                            .unwrap_or(false);

                        // Show headers immediately with snippet
                        message_view.show_message(
                            &from,
                            to,
                            &msg.date(),
                            &msg.subject(),
                            &msg.snippet(),
                            has_unsub,
                            msg.is_flagged(),
                            msg.is_read(),
                        );

                        // Async-load the cached body (if available)
                        let pool = pool.clone();
                        let view = message_view.clone();
                        runtime::spawn_async(
                            async move {
                                load_message_body(&pool, db_id).await
                            },
                            move |body: Option<String>| {
                                if let Some(text) = body {
                                    view.set_body_text(&text);
                                }
                            },
                        );
                    });

                    // Wire action button callbacks
                    let pool2 = data.pool.clone();
                    let ml2 = window_clone.message_list();
                    window_clone.message_view().connect_star_toggled(move |starred| {
                        if let Some(msg) = selected_message(&ml2) {
                            let db_id = msg.db_id();
                            let pool = pool2.clone();
                            msg.set_is_flagged(starred);
                            debug!(db_id, starred, "Star toggled");
                            runtime::spawn_async(
                                async move {
                                    toggle_flag(&pool, db_id, "\\Flagged", starred).await;
                                },
                                |_| {},
                            );
                        }
                    });

                    let pool3 = data.pool.clone();
                    let ml3 = window_clone.message_list();
                    window_clone.message_view().connect_read_toggled(move |read| {
                        if let Some(msg) = selected_message(&ml3) {
                            let db_id = msg.db_id();
                            let pool = pool3.clone();
                            msg.set_is_read(read);
                            debug!(db_id, read, "Read toggled");
                            runtime::spawn_async(
                                async move {
                                    toggle_flag(&pool, db_id, "\\Seen", read).await;
                                },
                                |_| {},
                            );
                        }
                    });

                    let pool4 = data.pool.clone();
                    let ml4 = window_clone.message_list();
                    let view4 = window_clone.message_view();
                    window_clone.message_view().connect_delete_clicked(move || {
                        if let Some(msg) = selected_message(&ml4) {
                            let db_id = msg.db_id();
                            let pool = pool4.clone();
                            debug!(db_id, "Delete clicked");
                            remove_selected(&ml4);
                            view4.show_placeholder();
                            runtime::spawn_async(
                                async move {
                                    if let Err(e) =
                                        mq_db::queries::messages::delete_message(&pool, db_id).await
                                    {
                                        warn!("Failed to delete message: {e}");
                                    }
                                },
                                |_| {},
                            );
                        }
                    });

                    let ml5 = window_clone.message_list();
                    let view5 = window_clone.message_view();
                    window_clone.message_view().connect_archive_clicked(move || {
                        if let Some(msg) = selected_message(&ml5) {
                            debug!(db_id = msg.db_id(), "Archive clicked");
                            remove_selected(&ml5);
                            view5.show_placeholder();
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to load data: {e}");
                    window_clone.show_banner(&format!("Error: {e}"));
                }
            }
        },
    );
}

/// Load the cached body for a message from the database.
async fn load_message_body(pool: &SqlitePool, message_id: i64) -> Option<String> {
    match mq_db::queries::message_bodies::get_body(pool, message_id).await {
        Ok(Some(body)) => {
            // Prefer text body for Phase 2; HTML rendering comes in Phase 5
            if let Some(text) = body.text_body {
                Some(text)
            } else if let Some(html) = body.html_body {
                Some(strip_html_tags(&html))
            } else {
                None
            }
        }
        Ok(None) => None,
        Err(e) => {
            warn!("Failed to load message body: {e}");
            None
        }
    }
}

/// Toggle a flag in the database for a message.
async fn toggle_flag(pool: &SqlitePool, message_id: i64, flag: &str, add: bool) {
    let current: Result<Option<String>, sqlx::Error> =
        sqlx::query_scalar("SELECT flags FROM messages WHERE id = ?")
            .bind(message_id)
            .fetch_optional(pool)
            .await;

    match current {
        Ok(Some(flags_str)) => {
            let new_flags = if add {
                if flags_str.contains(flag) {
                    flags_str
                } else if flags_str.is_empty() {
                    flag.to_string()
                } else {
                    format!("{flags_str} {flag}")
                }
            } else {
                flags_str
                    .split_whitespace()
                    .filter(|f| *f != flag)
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            if let Err(e) =
                mq_db::queries::messages::update_flags(pool, message_id, &new_flags).await
            {
                warn!("Failed to update flags: {e}");
            }
        }
        Ok(None) => {
            warn!(message_id, "Message not found for flag update");
        }
        Err(e) => {
            warn!("Failed to read flags: {e}");
        }
    }
}

/// Get the currently selected MessageObject from a message list.
fn selected_message(
    list: &crate::widgets::message_list::MqMessageList,
) -> Option<MessageObject> {
    let selection = list.selection();
    selection
        .selected_item()
        .and_then(|item| item.downcast::<MessageObject>().ok())
}

/// Remove the currently selected item from the message list.
fn remove_selected(list: &crate::widgets::message_list::MqMessageList) {
    let selection = list.selection();
    let pos = selection.selected();
    let model = list.model();
    if pos < model.n_items() {
        model.remove(pos);
    }
}

/// Very basic HTML tag stripper for Phase 2 fallback.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Load custom CSS for the application.
fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(
        "
        .bold-label {
            font-weight: bold;
        }

        .message-list {
            background: transparent;
        }

        .navigation-sidebar row {
            min-height: 36px;
        }
        ",
    );

    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("Could not get default display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Intermediate data types for passing between async and GTK threads.
struct AppData {
    has_accounts: bool,
    messages: Vec<MessageData>,
    pool: Arc<SqlitePool>,
}

struct MessageData {
    db_id: i64,
    uid: u32,
    sender_name: String,
    sender_email: String,
    subject: String,
    date: String,
    snippet: String,
    is_read: bool,
    is_flagged: bool,
    has_attachments: bool,
    mailbox: String,
    account_id: i64,
    recipient_to: String,
    list_unsubscribe: Option<String>,
}
