use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::gio;
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::config;
use crate::runtime;
use crate::widgets::account_setup::MqAccountSetup;
use crate::widgets::compose_window::{ComposeMode, MqComposeWindow};
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

        setup_data(&window);
    });

    app.run().into()
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct AccountInfo {
    id: i64,
    email: String,
    display_name: Option<String>,
}

struct AppData {
    accounts: Vec<AccountInfo>,
    messages: Vec<MessageData>,
    account_emails: HashMap<i64, String>,
    pool: Arc<SqlitePool>,
}

#[derive(Clone)]
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

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

/// Set up data layer: init DB, load accounts, load initial messages.
fn setup_data(window: &MqWindow) {
    let window_clone = window.clone();

    runtime::spawn_async(
        async move {
            let db_path = mq_core::config::AppConfig::data_dir().join("mq-mail.db");

            let pool = match mq_db::init_pool(&db_path).await {
                Ok(pool) => pool,
                Err(e) => {
                    error!("Failed to initialize database: {e}");
                    return Err(format!("Database error: {e}"));
                }
            };

            let accounts = mq_db::queries::accounts::get_all_accounts(&pool)
                .await
                .unwrap_or_default();

            let account_infos: Vec<AccountInfo> = accounts
                .iter()
                .map(|a| AccountInfo {
                    id: a.id,
                    email: a.email.clone(),
                    display_name: a.display_name.clone(),
                })
                .collect();

            let account_emails: HashMap<i64, String> =
                accounts.iter().map(|a| (a.id, a.email.clone())).collect();

            if accounts.is_empty() {
                info!("No accounts configured — showing account setup");
                return Ok(AppData {
                    accounts: account_infos,
                    messages: vec![],
                    account_emails,
                    pool: Arc::new(pool),
                });
            }

            info!(count = accounts.len(), "Loading unified inbox");

            // Unified inbox: all accounts' INBOX
            let messages =
                mq_db::queries::messages::get_messages_all_accounts_for_mailbox(
                    &pool, "INBOX", 200, 0,
                )
                .await
                .unwrap_or_default();

            info!(count = messages.len(), "Loaded cached messages (unified)");

            Ok(AppData {
                accounts: account_infos,
                messages: db_to_message_data(messages),
                account_emails,
                pool: Arc::new(pool),
            })
        },
        move |result: Result<AppData, String>| match result {
            Ok(data) => setup_ui(window_clone, data),
            Err(e) => {
                error!("Failed to load data: {e}");
                window_clone.show_banner(&format!("Error: {e}"));
            }
        },
    );
}

/// Wire up the entire UI after data loads.
fn setup_ui(window: MqWindow, data: AppData) {
    let pool = data.pool.clone();
    let account_emails = Arc::new(data.account_emails);
    let is_multi_account = data.accounts.len() > 1;

    // Populate sidebar with accounts
    let sidebar = window.sidebar();
    let tuples: Vec<(i64, String, Option<String>)> = data
        .accounts
        .iter()
        .map(|a| (a.id, a.email.clone(), a.display_name.clone()))
        .collect();
    sidebar.set_accounts(&tuples);

    // No accounts → show setup dialog
    if data.accounts.is_empty() {
        show_account_setup(&window, &pool);
        return;
    }

    // Shared messages data — updated on each reload, read by selection handler.
    let shared_messages: Rc<RefCell<Vec<MessageData>>> =
        Rc::new(RefCell::new(data.messages.clone()));

    // Populate message list (unified view)
    let show_badge = is_multi_account;
    let objects = make_message_objects(&data.messages, &account_emails, show_badge);
    window.message_list().set_messages(objects);

    // Wire message selection (ONCE — reads from shared_messages)
    {
        let view = window.message_view();
        let pool_sel = pool.clone();
        let msgs = shared_messages.clone();

        window
            .message_list()
            .connect_message_selected(move |msg| {
                let db_id = msg.db_id();
                let from = if msg.sender_name().is_empty() {
                    msg.sender_email()
                } else {
                    format!("{} <{}>", msg.sender_name(), msg.sender_email())
                };

                let msgs = msgs.borrow();
                let msg_data = msgs.iter().find(|m| m.db_id == db_id);
                let to = msg_data.map(|m| m.recipient_to.as_str()).unwrap_or("");
                let has_unsub = msg_data
                    .map(|m| m.list_unsubscribe.is_some())
                    .unwrap_or(false);

                view.show_message(
                    &from,
                    to,
                    &msg.date(),
                    &msg.subject(),
                    &msg.snippet(),
                    has_unsub,
                    msg.is_flagged(),
                    msg.is_read(),
                );

                let pool = pool_sel.clone();
                let v = view.clone();
                runtime::spawn_async(
                    async move { load_message_body(&pool, db_id).await },
                    move |body: Option<String>| {
                        if let Some(text) = body {
                            v.set_body_text(&text);
                        }
                    },
                );
            });
    }

    // Wire action buttons (star, read, delete, archive)
    wire_action_buttons(&window, &pool);

    // Wire compose & reply buttons
    wire_compose_buttons(&window, &pool, &data.accounts, &shared_messages);

    // Wire sidebar: account selection → reload messages
    {
        let w = window.clone();
        let p = pool.clone();
        let e = account_emails.clone();
        let m = shared_messages.clone();
        let n = data.accounts.len();
        sidebar.connect_account_selected(move |account_id| {
            let mailbox = w.sidebar().selected_mailbox();
            reload_messages(&w, &p, account_id, &mailbox, &e, n > 1, &m);
        });
    }

    // Wire sidebar: mailbox selection → reload messages
    {
        let w = window.clone();
        let p = pool.clone();
        let e = account_emails.clone();
        let m = shared_messages.clone();
        let n = data.accounts.len();
        sidebar.connect_mailbox_selected(move |mailbox| {
            let account_id = w.sidebar().selected_account_id();
            reload_messages(&w, &p, account_id, mailbox, &e, n > 1, &m);
        });
    }

    // Wire "Add Account"
    {
        let w = window.clone();
        let p = pool.clone();
        sidebar.connect_add_account(move || {
            show_account_setup(&w, &p);
        });
    }
}

// ---------------------------------------------------------------------------
// Account setup
// ---------------------------------------------------------------------------

fn show_account_setup(window: &MqWindow, pool: &Arc<SqlitePool>) {
    let dialog = MqAccountSetup::new(window);
    dialog.present();

    let dialog_ref = dialog.clone();
    let pool = pool.clone();
    let window = window.clone();

    dialog.connect_sign_in(move || {
        dialog_ref.show_loading();

        let dialog = dialog_ref.clone();
        let pool = pool.clone();
        let window = window.clone();

        runtime::spawn_async(
            async move {
                let config = mq_core::config::AppConfig::load().unwrap_or_default();
                let client_id = config
                    .resolve_client_id()
                    .ok()
                    .flatten()
                    .ok_or("OAuth client_id not configured in config.toml")?;
                let client_secret = config
                    .resolve_client_secret()
                    .ok()
                    .flatten()
                    .ok_or("OAuth client_secret not configured in config.toml")?;

                let port = mq_core::oauth::find_free_port()
                    .map_err(|e| format!("{e}"))?;
                let client =
                    mq_core::oauth::build_client(&client_id, &client_secret, port)
                        .map_err(|e| format!("{e}"))?;

                let (auth_url, csrf_token, pkce_verifier) =
                    mq_core::oauth::authorization_url(&client);

                // Open browser
                let _ = gio::AppInfo::launch_default_for_uri(
                    &auth_url,
                    None::<&gio::AppLaunchContext>,
                );

                // Wait for redirect
                let code =
                    mq_core::oauth::run_callback_server(port, csrf_token.secret())
                        .await
                        .map_err(|e| format!("{e}"))?;

                // Exchange for tokens
                let tokens =
                    mq_core::oauth::exchange_code(&client, code, pkce_verifier)
                        .await
                        .map_err(|e| format!("{e}"))?;

                // Get user email
                let (email, display_name) =
                    mq_core::oauth::get_user_info(&tokens.access_token)
                        .await
                        .map_err(|e| format!("{e}"))?;

                // Save to DB
                let account_id = mq_db::queries::accounts::insert_account(
                    &pool,
                    &email,
                    display_name.as_deref(),
                )
                .await
                .map_err(|e| format!("Failed to save account: {e}"))?;

                info!(account_id, %email, "Account added successfully");

                // TODO: Store refresh_token in keyring (Phase 6)
                if tokens.refresh_token.is_some() {
                    debug!("Refresh token obtained (keyring storage deferred)");
                }

                Ok((email, pool))
            },
            move |result: Result<(String, Arc<SqlitePool>), String>| match result {
                Ok((email, pool)) => {
                    dialog.show_success(&email);

                    // Refresh sidebar account list
                    let window2 = window.clone();
                    runtime::spawn_async(
                        async move {
                            mq_db::queries::accounts::get_all_accounts(&pool)
                                .await
                                .unwrap_or_default()
                        },
                        move |db_accounts| {
                            let tuples: Vec<(i64, String, Option<String>)> = db_accounts
                                .iter()
                                .map(|a| (a.id, a.email.clone(), a.display_name.clone()))
                                .collect();
                            window2.sidebar().set_accounts(&tuples);
                        },
                    );
                }
                Err(e) => {
                    error!("Account setup failed: {e}");
                    dialog.show_error(&e);
                }
            },
        );
    });
}

// ---------------------------------------------------------------------------
// Message loading
// ---------------------------------------------------------------------------

fn reload_messages(
    window: &MqWindow,
    pool: &Arc<SqlitePool>,
    account_id: Option<i64>,
    mailbox: &str,
    account_emails: &Arc<HashMap<i64, String>>,
    is_multi_account: bool,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    let mailbox = mailbox.to_string();
    let show_badge = is_multi_account && account_id.is_none();
    let pool = pool.clone();
    let emails = account_emails.clone();
    let msgs = shared_messages.clone();
    let ml = window.message_list();
    let mv = window.message_view();

    runtime::spawn_async(
        async move {
            let messages = match account_id {
                Some(aid) => mq_db::queries::messages::get_messages_for_mailbox(
                    &pool, aid, &mailbox, 200, 0,
                )
                .await
                .unwrap_or_default(),
                None => {
                    mq_db::queries::messages::get_messages_all_accounts_for_mailbox(
                        &pool, &mailbox, 200, 0,
                    )
                    .await
                    .unwrap_or_default()
                }
            };
            (db_to_message_data(messages), emails, show_badge)
        },
        move |(messages, emails, show_badge): (
            Vec<MessageData>,
            Arc<HashMap<i64, String>>,
            bool,
        )| {
            let objects = make_message_objects(&messages, &emails, show_badge);
            // Update shared state so the selection handler has current data
            *msgs.borrow_mut() = messages;
            ml.set_messages(objects);
            mv.show_placeholder();
        },
    );
}

fn db_to_message_data(messages: Vec<mq_db::models::DbMessage>) -> Vec<MessageData> {
    messages
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
        .collect()
}

fn make_message_objects(
    messages: &[MessageData],
    account_emails: &HashMap<i64, String>,
    show_badge: bool,
) -> Vec<MessageObject> {
    messages
        .iter()
        .map(|m| {
            let badge = if show_badge {
                account_emails
                    .get(&m.account_id)
                    .cloned()
                    .unwrap_or_default()
            } else {
                String::new()
            };
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
                &badge,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Message selection & actions
// ---------------------------------------------------------------------------

fn wire_action_buttons(window: &MqWindow, pool: &Arc<SqlitePool>) {
    let pool2 = pool.clone();
    let ml2 = window.message_list();
    window
        .message_view()
        .connect_star_toggled(move |starred| {
            if let Some(msg) = selected_message(&ml2) {
                let db_id = msg.db_id();
                let pool = pool2.clone();
                msg.set_is_flagged(starred);
                debug!(db_id, starred, "Star toggled");
                runtime::spawn_async(
                    async move { toggle_flag(&pool, db_id, "\\Flagged", starred).await },
                    |_| {},
                );
            }
        });

    let pool3 = pool.clone();
    let ml3 = window.message_list();
    window.message_view().connect_read_toggled(move |read| {
        if let Some(msg) = selected_message(&ml3) {
            let db_id = msg.db_id();
            let pool = pool3.clone();
            msg.set_is_read(read);
            debug!(db_id, read, "Read toggled");
            runtime::spawn_async(
                async move { toggle_flag(&pool, db_id, "\\Seen", read).await },
                |_| {},
            );
        }
    });

    let pool4 = pool.clone();
    let ml4 = window.message_list();
    let view4 = window.message_view();
    window.message_view().connect_delete_clicked(move || {
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

    let ml5 = window.message_list();
    let view5 = window.message_view();
    window.message_view().connect_archive_clicked(move || {
        if let Some(msg) = selected_message(&ml5) {
            debug!(db_id = msg.db_id(), "Archive clicked");
            remove_selected(&ml5);
            view5.show_placeholder();
        }
    });
}

// ---------------------------------------------------------------------------
// Compose
// ---------------------------------------------------------------------------

fn wire_compose_buttons(
    window: &MqWindow,
    pool: &Arc<SqlitePool>,
    accounts: &[AccountInfo],
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    let account_tuples: Vec<(i64, String)> = accounts
        .iter()
        .map(|a| (a.id, a.email.clone()))
        .collect();

    // Compose new message
    {
        let w = window.clone();
        let accts = account_tuples.clone();
        window.message_list().connect_compose_clicked(move || {
            open_compose(&w, &accts, ComposeMode::New, None);
        });
    }

    // Reply
    {
        let w = window.clone();
        let ml = window.message_list();
        let accts = account_tuples.clone();
        let pool = pool.clone();
        window.message_view().connect_reply_clicked(move || {
            if let Some(msg) = selected_message(&ml) {
                let db_id = msg.db_id();
                let from = format_sender(&msg);
                let subject = msg.subject();
                let date = msg.date();
                let account_id = msg.account_id();
                let accts = accts.clone();
                let w = w.clone();
                let pool = pool.clone();
                runtime::spawn_async(
                    async move { load_message_body(&pool, db_id).await },
                    move |body: Option<String>| {
                        let body_text = body.unwrap_or_default();
                        let mode = ComposeMode::Reply {
                            from: from.clone(),
                            subject: subject.clone(),
                            date: date.clone(),
                            body: body_text,
                            message_id: None,
                            references: None,
                        };
                        open_compose(&w, &accts, mode, Some(account_id));
                    },
                );
            }
        });
    }

    // Reply All
    {
        let w = window.clone();
        let ml = window.message_list();
        let accts = account_tuples.clone();
        let msgs = shared_messages.clone();
        let pool = pool.clone();
        window.message_view().connect_reply_all_clicked(move || {
            if let Some(msg) = selected_message(&ml) {
                let db_id = msg.db_id();
                let from = format_sender(&msg);
                let subject = msg.subject();
                let date = msg.date();
                let msgs = msgs.borrow();
                let msg_data = msgs.iter().find(|m| m.db_id == db_id);
                let to_str = msg_data.map(|m| m.recipient_to.clone()).unwrap_or_default();
                let to_addrs: Vec<String> = to_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let account_id = msg.account_id();
                let accts = accts.clone();
                let w = w.clone();
                let pool = pool.clone();
                runtime::spawn_async(
                    async move { load_message_body(&pool, db_id).await },
                    move |body: Option<String>| {
                        let body_text = body.unwrap_or_default();
                        let mode = ComposeMode::ReplyAll {
                            from: from.clone(),
                            to: to_addrs,
                            cc: vec![],
                            subject: subject.clone(),
                            date: date.clone(),
                            body: body_text,
                            message_id: None,
                            references: None,
                        };
                        open_compose(&w, &accts, mode, Some(account_id));
                    },
                );
            }
        });
    }

    // Forward
    {
        let w = window.clone();
        let ml = window.message_list();
        let accts = account_tuples.clone();
        let msgs = shared_messages.clone();
        let pool = pool.clone();
        window.message_view().connect_forward_clicked(move || {
            if let Some(msg) = selected_message(&ml) {
                let db_id = msg.db_id();
                let from = format_sender(&msg);
                let subject = msg.subject();
                let date = msg.date();
                let msgs = msgs.borrow();
                let msg_data = msgs.iter().find(|m| m.db_id == db_id);
                let to = msg_data.map(|m| m.recipient_to.clone()).unwrap_or_default();
                let account_id = msg.account_id();
                let accts = accts.clone();
                let w = w.clone();
                let pool = pool.clone();
                runtime::spawn_async(
                    async move { load_message_body(&pool, db_id).await },
                    move |body: Option<String>| {
                        let body_text = body.unwrap_or_default();
                        let mode = ComposeMode::Forward {
                            from: from.clone(),
                            subject: subject.clone(),
                            date: date.clone(),
                            to,
                            body: body_text,
                        };
                        open_compose(&w, &accts, mode, Some(account_id));
                    },
                );
            }
        });
    }
}

fn open_compose(
    window: &MqWindow,
    accounts: &[(i64, String)],
    mode: ComposeMode,
    selected_account_id: Option<i64>,
) {
    let config = mq_core::config::AppConfig::load().unwrap_or_default();
    let signature = config.compose.default_signature;

    let compose = MqComposeWindow::new(window);
    compose.set_accounts(accounts);
    if let Some(aid) = selected_account_id {
        compose.select_account(aid);
    }
    compose.apply_mode(&mode, &signature);

    let (in_reply_to, references) = MqComposeWindow::reply_headers(&mode);

    // Wire Send button
    let compose_ref = compose.clone();
    compose.connect_send(move || {
        let Some((_, from_email)) = compose_ref.selected_account() else {
            warn!("No account selected for sending");
            return;
        };

        let to = compose_ref.to_addresses();
        if to.is_empty() {
            warn!("No recipients specified");
            return;
        }

        let email = mq_core::smtp::OutgoingEmail {
            from_email: from_email.clone(),
            from_name: None,
            to,
            cc: compose_ref.cc_addresses(),
            bcc: compose_ref.bcc_addresses(),
            subject: compose_ref.subject(),
            body_text: compose_ref.body_text(),
            body_html: None,
            in_reply_to: in_reply_to.clone(),
            references: references.clone(),
        };

        info!(to = ?email.to, subject = %email.subject, "Sending email");

        // TODO: Get access token from keyring (Phase 6).
        // For now, log a placeholder message.
        let compose_close = compose_ref.clone();
        runtime::spawn_async(
            async move {
                // When token storage is implemented, this will call:
                // mq_core::smtp::send_email(&email, &access_token).await
                warn!(
                    "SMTP send deferred: token storage not yet implemented. \
                     Would send to {:?} with subject '{}'",
                    email.to, email.subject
                );
                Ok::<(), String>(())
            },
            move |result: Result<(), String>| match result {
                Ok(()) => {
                    info!("Compose window closing (send queued)");
                    compose_close.close();
                }
                Err(e) => {
                    error!("Failed to send: {e}");
                }
            },
        );
    });

    compose.present();
}

fn format_sender(msg: &MessageObject) -> String {
    if msg.sender_name().is_empty() {
        msg.sender_email()
    } else {
        format!("{} <{}>", msg.sender_name(), msg.sender_email())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn load_message_body(pool: &SqlitePool, message_id: i64) -> Option<String> {
    match mq_db::queries::message_bodies::get_body(pool, message_id).await {
        Ok(Some(body)) => {
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
        Ok(None) => warn!(message_id, "Message not found for flag update"),
        Err(e) => warn!("Failed to read flags: {e}"),
    }
}

fn selected_message(
    list: &crate::widgets::message_list::MqMessageList,
) -> Option<MessageObject> {
    list.selection()
        .selected_item()
        .and_then(|item| item.downcast::<MessageObject>().ok())
}

fn remove_selected(list: &crate::widgets::message_list::MqMessageList) {
    let selection = list.selection();
    let pos = selection.selected();
    let model = list.model();
    if pos < model.n_items() {
        model.remove(pos);
    }
}

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

        .caption {
            font-size: 0.8em;
        }

        .heading {
            font-weight: 700;
            text-transform: uppercase;
            letter-spacing: 0.05em;
            font-size: 0.75em;
        }
        ",
    );

    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("Could not get default display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
