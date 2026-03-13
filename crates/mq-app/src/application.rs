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

        // Register bundled fallback icons as a GResource.
        // These are treated as hicolor fallback — the user's icon theme is
        // always checked first. Only if an icon isn't found anywhere in the
        // theme's inheritance chain do these kick in.
        register_bundled_icons();

        load_css();
    });

    app.connect_activate(|app| {
        info!("Application activate");
        let window = MqWindow::new(app);

        // Register actions and keyboard shortcuts
        crate::actions::setup_actions(app, &window);

        // Apply saved theme preference
        if let Ok(cfg) = mq_core::config::AppConfig::load() {
            let style_manager = app.style_manager();
            match cfg.appearance.theme {
                mq_core::config::Theme::System => {
                    style_manager.set_color_scheme(adw::ColorScheme::Default);
                }
                mq_core::config::Theme::Light => {
                    style_manager.set_color_scheme(adw::ColorScheme::ForceLight);
                }
                mq_core::config::Theme::Dark => {
                    style_manager.set_color_scheme(adw::ColorScheme::ForceDark);
                }
            }
        }

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
    list_unsubscribe_post: Option<String>,
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
            Ok(data) => {
                let accounts = data.accounts.clone();
                let pool = data.pool.clone();
                let window = window_clone.clone();
                setup_ui(window_clone, data);

                // Trigger background sync for each existing account
                for account in &accounts {
                    let email = account.email.clone();
                    let account_id = account.id;
                    let pool = pool.clone();
                    let window = window.clone();

                    runtime::spawn_async(
                        async move {
                            mq_core::keyring::get_tokens(&email).await.ok().flatten()
                                .map(|t| (t.access_token, email))
                        },
                        move |tokens: Option<(String, String)>| {
                            if let Some((access_token, email)) = tokens {
                                info!(email = %email, "Starting background sync for existing account");
                                start_background_sync(window, pool, account_id, email, access_token);
                            } else {
                                warn!(account_id, "No tokens in keyring — skipping sync");
                            }
                        },
                    );
                }
            }
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
                let sender = msg.sender_email();
                let account = msg.account_id();
                runtime::spawn_async(
                    async move {
                        let body = load_message_body(&pool, db_id).await;
                        let allowed =
                            mq_db::queries::sender_allowlist::is_allowed(&pool, account, &sender)
                                .await
                                .unwrap_or(false);
                        (body, allowed)
                    },
                    move |(body, sender_allowed): (Option<BodyResult>, bool)| {
                        if let Some(body) = body {
                            v.set_body_text(&body.text);
                            // Show privacy banners
                            let config =
                                mq_core::config::AppConfig::load().unwrap_or_default();
                            let show_images_banner = config.privacy.block_remote_images
                                && !sender_allowed
                                && body.blocked_images > 0;
                            if show_images_banner {
                                v.show_images_banner(body.blocked_images);
                            } else {
                                v.hide_images_banner();
                            }
                            if body.tracking_pixels > 0 {
                                v.show_tracking_info(body.tracking_pixels);
                            } else {
                                v.hide_tracking_info();
                            }
                        }
                    },
                );
            });
    }

    // Wire action buttons (star, read, delete, archive)
    wire_action_buttons(&window, &pool);

    // Wire compose & reply buttons
    wire_compose_buttons(&window, &pool, &data.accounts, &shared_messages);

    // Wire privacy / unsubscribe buttons
    wire_privacy_buttons(&window, &pool, &shared_messages);

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

    // Account removal via right-click
    {
        let w = window.clone();
        let p = pool.clone();
        sidebar.connect_account_remove(move |account_id, email| {
            let w = w.clone();
            let p = p.clone();
            let email = email.clone();

            // Confirmation dialog
            let dialog = adw::AlertDialog::builder()
                .heading("Remove Account")
                .body(format!("Remove {email} and all its cached messages?"))
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("remove", "Remove");
            dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            let w2 = w.clone();
            dialog.connect_response(None, move |_dialog, response| {
                if response != "remove" {
                    return;
                }
                let pool = p.clone();
                let email = email.clone();
                let window = w2.clone();

                runtime::spawn_async(
                    async move {
                        // Delete from keyring
                        if let Err(e) = mq_core::keyring::delete_tokens(&email).await {
                            warn!(error = %e, "Failed to delete keyring tokens");
                        }
                        // Delete messages + account from DB
                        let _ = sqlx::query("DELETE FROM messages WHERE account_id = ?")
                            .bind(account_id)
                            .execute(&*pool)
                            .await;
                        let _ = sqlx::query("DELETE FROM sync_state WHERE account_id = ?")
                            .bind(account_id)
                            .execute(&*pool)
                            .await;
                        let _ = sqlx::query("DELETE FROM labels WHERE account_id = ?")
                            .bind(account_id)
                            .execute(&*pool)
                            .await;
                        let _ = mq_db::queries::accounts::delete_account(&pool, account_id).await;
                        info!(account_id, email = %email, "Account removed");
                    },
                    move |()| {
                        info!("Refreshing UI after account removal");
                        // Re-run full data setup to rebuild UI
                        setup_data(&window);
                    },
                );
            });

            dialog.present(Some(&w));
        });
    }

    // Load and display labels in sidebar
    {
        let sidebar = window.sidebar();
        let pool = pool.clone();
        let account_ids: Vec<i64> = data.accounts.iter().map(|a| a.id).collect();
        runtime::spawn_async(
            async move {
                let mut all_labels: Vec<(String, String)> = Vec::new();
                for aid in &account_ids {
                    let labels = mq_db::queries::labels::get_user_labels(&pool, *aid)
                        .await
                        .unwrap_or_default();
                    for label in labels {
                        let entry = (label.name.clone(), label.imap_name.clone());
                        if !all_labels.contains(&entry) {
                            all_labels.push(entry);
                        }
                    }
                }
                all_labels
            },
            move |labels: Vec<(String, String)>| {
                sidebar.set_labels(&labels);
            },
        );
    }

    // Wire sidebar: label selection → reload messages for that label
    {
        let w = window.clone();
        let p = pool.clone();
        let e = account_emails.clone();
        let m = shared_messages.clone();
        let n = data.accounts.len();
        window.sidebar().connect_label_selected(move |label_imap_name| {
            let account_id = w.sidebar().selected_account_id();
            reload_messages(&w, &p, account_id, label_imap_name, &e, n > 1, &m);
        });
    }

    // Wire search bar
    wire_search(&window, &pool, &account_emails, is_multi_account, &shared_messages);

    // Wire network awareness (offline banner + reconnect)
    wire_network_awareness(&window, &pool, &account_emails, is_multi_account, &shared_messages);
}

// ---------------------------------------------------------------------------
// Account setup
// ---------------------------------------------------------------------------

fn show_account_setup(window: &MqWindow, pool: &Arc<SqlitePool>) {
    let dialog = MqAccountSetup::new();
    dialog.present(Some(window));

    let dialog_ref = dialog.clone();
    let pool = pool.clone();
    let window = window.clone();

    dialog.connect_sign_in(move || {
        dialog_ref.show_loading();

        let dialog = dialog_ref.clone();
        let pool = pool.clone();
        let window = window.clone();

        // Phase 1: OAuth only (quick) — get tokens and save account
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

                // Store tokens in keyring
                mq_core::keyring::store_tokens(
                    &email,
                    &tokens.access_token,
                    tokens.refresh_token.as_deref(),
                )
                .await
                .map_err(|e| format!("Failed to store tokens: {e}"))?;

                // Save to DB
                let account_id = mq_db::queries::accounts::insert_account(
                    &pool,
                    &email,
                    display_name.as_deref(),
                )
                .await
                .map_err(|e| format!("Failed to save account: {e}"))?;

                info!(account_id, %email, "Account added successfully");

                Ok((account_id, email, tokens.access_token, pool))
            },
            move |result: Result<(i64, String, String, Arc<SqlitePool>), String>| match result {
                Ok((account_id, email, access_token, pool)) => {
                    dialog.show_success(&email);

                    // Phase 2: Re-init UI immediately (empty inbox is fine)
                    let window2 = window.clone();
                    let pool2 = pool.clone();
                    runtime::spawn_async(
                        {
                            let pool = pool.clone();
                            async move {
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
                                AppData {
                                    accounts: account_infos,
                                    messages: vec![],
                                    account_emails,
                                    pool: Arc::new((*pool).clone()),
                                }
                            }
                        },
                        move |data: AppData| {
                            setup_ui(window2.clone(), data);

                            // Phase 3: Background sync with progress banner
                            window2.show_banner("Syncing inbox\u{2026}");
                            start_background_sync(window2, pool2, account_id, email, access_token);
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

/// Run initial IMAP sync in the background, updating the UI as messages arrive.
fn start_background_sync(
    window: MqWindow,
    pool: Arc<SqlitePool>,
    account_id: i64,
    email: String,
    access_token: String,
) {
    let (tx, rx) = async_channel::unbounded::<SyncProgress>();

    // Background sync task — sends progress updates via channel
    let bg_pool = pool.clone();
    runtime::runtime().spawn(async move {
        let pool = bg_pool;
        use mq_core::imap::client::ImapSession;
        use mq_core::imap::sync;

        let _ = tx.send(SyncProgress::Status("Connecting to Gmail\u{2026}".into())).await;

        let mut session = match ImapSession::connect(&email, &access_token).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(SyncProgress::Error(format!("IMAP connect failed: {e}"))).await;
                return;
            }
        };

        let _ = tx.send(SyncProgress::Status("Fetching messages\u{2026}".into())).await;

        let outcome = match sync::sync_mailbox(&mut session, "INBOX", None, &[]).await {
            Ok(o) => o,
            Err(e) => {
                let _ = tx.send(SyncProgress::Error(format!("Sync failed: {e}"))).await;
                return;
            }
        };

        let total = outcome.new_messages.len();
        let _ = tx.send(SyncProgress::Status(format!("Saving {total} messages\u{2026}"))).await;

        // Persist messages to DB in batches, sending progress
        for (i, email_msg) in outcome.new_messages.iter().enumerate() {
            persist_email_to_db(&pool, account_id, "INBOX", email_msg, &outcome.new_state).await;

            // Send progress every 25 messages
            if (i + 1) % 25 == 0 || i + 1 == total {
                let _ = tx.send(SyncProgress::Count(i + 1, total)).await;
            }
        }

        // Save sync state
        let _ = mq_db::queries::sync_state::upsert_sync_state(
            &pool,
            account_id,
            "INBOX",
            outcome.new_state.uid_validity as i64,
            outcome.new_state.highest_modseq as i64,
            outcome.new_state.highest_uid as i64,
        )
        .await;

        let _ = session.logout().await;
        let _ = tx.send(SyncProgress::Done(total)).await;
    });

    // UI-side: receive progress updates on the GTK main thread
    let w = window.clone();
    let p = pool;
    glib::spawn_future_local(async move {
        while let Ok(progress) = rx.recv().await {
            match progress {
                SyncProgress::Status(msg) => {
                    w.show_banner(&msg);
                }
                SyncProgress::Count(done, total) => {
                    w.show_banner(&format!("Syncing inbox\u{2026} {done}/{total} messages"));
                    refresh_message_list_from_db(&w, &p);
                }
                SyncProgress::Error(msg) => {
                    error!("Background sync error: {msg}");
                    w.show_banner(&format!("Sync error: {msg}"));
                    let w2 = w.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_secs(5), move || {
                        w2.hide_banner();
                    });
                    break;
                }
                SyncProgress::Done(total) => {
                    info!(total, "Background sync complete");
                    w.hide_banner();
                    refresh_message_list_from_db(&w, &p);
                    break;
                }
            }
        }
    });
}

enum SyncProgress {
    Status(String),
    Count(usize, usize),
    Error(String),
    Done(usize),
}

/// Quick refresh of the message list from DB (called during/after sync).
fn refresh_message_list_from_db(window: &MqWindow, pool: &Arc<SqlitePool>) {
    let ml = window.message_list();
    let pool = pool.clone();
    // We need account_emails for badge display but for a quick refresh
    // we just reload without badges.
    runtime::spawn_async(
        async move {
            mq_db::queries::messages::get_messages_all_accounts_for_mailbox(
                &pool, "INBOX", 200, 0,
            )
            .await
            .unwrap_or_default()
        },
        move |messages: Vec<mq_db::models::DbMessage>| {
            let data = db_to_message_data(messages);
            let empty_emails = HashMap::new();
            let objects = make_message_objects(&data, &empty_emails, false);
            ml.set_messages(objects);
        },
    );
}

/// Persist a single email from sync to the database.
async fn persist_email_to_db(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
    email_msg: &mq_core::email::Email,
    new_state: &mq_core::imap::sync::SyncState,
) {
    let flags = flags_to_string(&email_msg.flags);
    let from = email_msg.from.as_ref();
    let sender_name = from.and_then(|a| a.name.as_deref());
    let sender_email = from.map(|a| a.email.as_str()).unwrap_or("unknown@unknown");
    let recipient_to: String = email_msg
        .to
        .iter()
        .map(|a| {
            a.name
                .as_ref()
                .map(|n| format!("{n} <{}>", a.email))
                .unwrap_or_else(|| a.email.clone())
        })
        .collect::<Vec<_>>()
        .join(", ");
    let recipient_cc: Option<String> = if email_msg.cc.is_empty() {
        None
    } else {
        Some(
            email_msg
                .cc
                .iter()
                .map(|a| a.email.clone())
                .collect::<Vec<_>>()
                .join(", "),
        )
    };
    let refs_json = if email_msg.references.is_empty() {
        None
    } else {
        serde_json::to_string(&email_msg.references).ok()
    };

    let _ = mq_db::queries::messages::upsert_message(
        pool,
        account_id,
        email_msg.uid as i64,
        mailbox,
        email_msg.gmail_msg_id.map(|v| v as i64),
        email_msg.gmail_thread_id.map(|v| v as i64),
        email_msg.message_id.as_deref(),
        email_msg.in_reply_to.as_deref(),
        refs_json.as_deref(),
        sender_name,
        sender_email,
        &recipient_to,
        recipient_cc.as_deref(),
        email_msg.subject.as_deref(),
        email_msg.snippet.as_deref(),
        email_msg.date.as_deref().unwrap_or(""),
        &flags,
        email_msg.has_attachments,
        None,
        email_msg.list_unsubscribe.as_deref(),
        email_msg.list_unsubscribe_post.as_deref(),
        None,
        new_state.uid_validity as i64,
    )
    .await;
}

/// Convert MessageFlags to a space-separated IMAP flag string.
fn flags_to_string(flags: &mq_core::email::MessageFlags) -> String {
    let mut parts = Vec::new();
    if flags.seen {
        parts.push("\\Seen");
    }
    if flags.flagged {
        parts.push("\\Flagged");
    }
    if flags.answered {
        parts.push("\\Answered");
    }
    if flags.deleted {
        parts.push("\\Deleted");
    }
    if flags.draft {
        parts.push("\\Draft");
    }
    parts.join(" ")
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
            list_unsubscribe_post: m.list_unsubscribe_post,
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
                    async move { load_message_body_text(&pool, db_id).await },
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
                    async move { load_message_body_text(&pool, db_id).await },
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
                    async move { load_message_body_text(&pool, db_id).await },
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

// ---------------------------------------------------------------------------
// Privacy / Unsubscribe
// ---------------------------------------------------------------------------

fn wire_privacy_buttons(
    window: &MqWindow,
    pool: &Arc<SqlitePool>,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    let view = window.message_view();

    // Unsubscribe button
    {
        let ml = window.message_list();
        let msgs = shared_messages.clone();
        let pool = pool.clone();
        let w = window.clone();
        view.connect_unsubscribe_clicked(move || {
            if let Some(msg) = selected_message(&ml) {
                let db_id = msg.db_id();
                let msgs = msgs.borrow();
                let msg_data = msgs.iter().find(|m| m.db_id == db_id);

                if let Some(data) = msg_data {
                    if let Some(ref header) = data.list_unsubscribe {
                        let info = mq_core::privacy::unsubscribe::UnsubscribeInfo::parse(
                            header,
                            data.list_unsubscribe_post.as_deref(),
                        );

                        match info.recommended_action() {
                            Some(mq_core::privacy::unsubscribe::UnsubscribeAction::OneClickPost {
                                url,
                            }) => {
                                info!(%url, "One-click unsubscribe (RFC 8058)");
                                let pool = pool.clone();
                                let ml = ml.clone();
                                let w = w.clone();
                                runtime::spawn_async(
                                    async move {
                                        mq_core::privacy::unsubscribe::one_click_unsubscribe(&url)
                                            .await
                                    },
                                    move |result| match result {
                                        Ok(()) => {
                                            info!("Unsubscribe successful");
                                            // Delete the message
                                            let db_id_del = db_id;
                                            remove_selected(&ml);
                                            w.message_view().show_placeholder();
                                            runtime::spawn_async(
                                                async move {
                                                    let _ = mq_db::queries::messages::delete_message(
                                                        &pool, db_id_del,
                                                    )
                                                    .await;
                                                },
                                                |_| {},
                                            );
                                        }
                                        Err(e) => {
                                            error!("Unsubscribe failed: {e}");
                                            w.show_banner(&format!(
                                                "Unsubscribe failed: {}",
                                                e.user_message()
                                            ));
                                        }
                                    },
                                );
                            }
                            Some(
                                mq_core::privacy::unsubscribe::UnsubscribeAction::OpenInBrowser {
                                    url,
                                },
                            ) => {
                                info!(%url, "Opening unsubscribe URL in browser");
                                let cleaned =
                                    mq_core::privacy::links::strip_tracking_params(&url);
                                let _ = gio::AppInfo::launch_default_for_uri(
                                    &cleaned,
                                    None::<&gio::AppLaunchContext>,
                                );
                            }
                            Some(mq_core::privacy::unsubscribe::UnsubscribeAction::Mailto {
                                address,
                            }) => {
                                info!(%address, "Opening mailto unsubscribe");
                                let _ = gio::AppInfo::launch_default_for_uri(
                                    &address,
                                    None::<&gio::AppLaunchContext>,
                                );
                            }
                            None => {
                                warn!("No unsubscribe action available");
                            }
                        }
                    }
                }
            }
        });
    }

    // "Load images" button — reload body without blocking
    {
        let ml = window.message_list();
        let pool = pool.clone();
        let v = view.clone();
        view.connect_load_images(move || {
            if let Some(msg) = selected_message(&ml) {
                let db_id = msg.db_id();
                let pool = pool.clone();
                let v = v.clone();
                runtime::spawn_async(
                    async move { load_message_body_unblocked(&pool, db_id).await },
                    move |body: Option<BodyResult>| {
                        if let Some(body) = body {
                            v.set_body_text(&body.text);
                            v.hide_images_banner();
                        }
                    },
                );
            }
        });
    }

    // "Always load from this sender" button — add to allowlist + reload
    {
        let ml = window.message_list();
        let pool = pool.clone();
        let v = view.clone();
        view.connect_always_load_images(move || {
            if let Some(msg) = selected_message(&ml) {
                let db_id = msg.db_id();
                let sender_email = msg.sender_email();
                let account_id = msg.account_id();
                let pool = pool.clone();
                let v = v.clone();
                runtime::spawn_async(
                    async move {
                        // Add to allowlist
                        if let Err(e) = mq_db::queries::sender_allowlist::add_sender(
                            &pool,
                            account_id,
                            &sender_email,
                        )
                        .await
                        {
                            warn!("Failed to add sender to allowlist: {e}");
                        }
                        // Reload body unblocked
                        load_message_body_unblocked(&pool, db_id).await
                    },
                    move |body: Option<BodyResult>| {
                        if let Some(body) = body {
                            v.set_body_text(&body.text);
                            v.hide_images_banner();
                        }
                    },
                );
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

fn wire_search(
    window: &MqWindow,
    pool: &Arc<SqlitePool>,
    account_emails: &Arc<HashMap<i64, String>>,
    is_multi_account: bool,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    let ml = window.message_list();
    let w = window.clone();
    let p = pool.clone();
    let e = account_emails.clone();
    let m = shared_messages.clone();

    // When search is activated (Enter pressed), perform FTS search
    ml.connect_search_activated(move |query| {
        if query.trim().is_empty() {
            // Empty search → restore normal mailbox view
            let mailbox = w.sidebar().selected_mailbox();
            let account_id = w.sidebar().selected_account_id();
            reload_messages(&w, &p, account_id, &mailbox, &e, is_multi_account, &m);
            return;
        }

        let pool = p.clone();
        let emails = e.clone();
        let msgs = m.clone();
        let ml_inner = w.message_list();
        let mv = w.message_view();
        let query_owned = query.clone();
        let account_id = w.sidebar().selected_account_id();
        let show_badge = is_multi_account && account_id.is_none();

        runtime::spawn_async(
            async move {
                let messages = match account_id {
                    Some(aid) => {
                        mq_db::queries::messages::search_fts_for_account(
                            &pool, aid, &query_owned, 200,
                        )
                        .await
                    }
                    None => {
                        mq_db::queries::messages::search_fts(&pool, &query_owned, 200).await
                    }
                };

                match messages {
                    Ok(msgs) => Ok((db_to_message_data(msgs), emails, show_badge)),
                    Err(e) => Err(format!("Search failed: {e}")),
                }
            },
            move |result: Result<
                (Vec<MessageData>, Arc<HashMap<i64, String>>, bool),
                String,
            >| match result {
                Ok((messages, emails, show_badge)) => {
                    let objects = make_message_objects(&messages, &emails, show_badge);
                    *msgs.borrow_mut() = messages;
                    ml_inner.set_messages(objects);
                    ml_inner.set_mailbox_title("Search Results");
                    mv.show_placeholder();
                }
                Err(e) => {
                    warn!("Search error: {e}");
                }
            },
        );
    });

    // When search text is cleared, restore normal view
    let w2 = window.clone();
    let p2 = pool.clone();
    let e2 = account_emails.clone();
    let m2 = shared_messages.clone();

    window.message_list().connect_search_changed(move |query| {
        if query.is_empty() {
            let mailbox = w2.sidebar().selected_mailbox();
            let account_id = w2.sidebar().selected_account_id();
            reload_messages(&w2, &p2, account_id, &mailbox, &e2, is_multi_account, &m2);
        }
    });
}

// ---------------------------------------------------------------------------
// Network awareness & notifications
// ---------------------------------------------------------------------------

fn wire_network_awareness(
    window: &MqWindow,
    pool: &Arc<SqlitePool>,
    account_emails: &Arc<HashMap<i64, String>>,
    is_multi_account: bool,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    let w = window.clone();
    let pool = pool.clone();
    let emails = account_emails.clone();
    let msgs = shared_messages.clone();

    // Start network monitor on tokio runtime
    runtime::spawn_async(
        async move {
            let monitor = mq_net::monitor::NetworkMonitor::new().await;
            let connectivity = monitor.connectivity();
            let rx = monitor.subscribe();
            (monitor, connectivity, rx)
        },
        move |(monitor, initial_connectivity, mut rx): (
            mq_net::monitor::NetworkMonitor,
            mq_net::monitor::Connectivity,
            tokio::sync::broadcast::Receiver<mq_net::monitor::Connectivity>,
        )| {
            // Show initial offline banner if needed
            if initial_connectivity != mq_net::monitor::Connectivity::Online {
                w.show_banner(&format!("{initial_connectivity} — some features may be unavailable"));
            }

            // Store monitor in a static for the app lifetime
            let monitor = Arc::new(monitor);

            // Watch for connectivity changes
            let w2 = w.clone();
            let pool2 = pool.clone();
            let emails2 = emails.clone();
            let msgs2 = msgs.clone();
            glib::spawn_future_local(async move {
                loop {
                    match rx.recv().await {
                        Ok(mq_net::monitor::Connectivity::Online) => {
                            info!("Network restored — hiding offline banner");
                            w2.hide_banner();

                            // Replay offline queue for all accounts
                            let pool3 = pool2.clone();
                            let pool4 = pool2.clone();
                            let w3 = w2.clone();
                            let emails3 = emails2.clone();
                            let msgs3 = msgs2.clone();
                            runtime::spawn_async(
                                async move {
                                    let queue = mq_net::queue::OfflineQueue::new(pool3);
                                    let total = queue.total_pending_count().await.unwrap_or(0);
                                    if total > 0 {
                                        info!(total, "Replaying offline queue");
                                    }
                                    total
                                },
                                move |total: i64| {
                                    if total > 0 {
                                        // Reload messages to reflect any changes
                                        let mailbox = w3.sidebar().selected_mailbox();
                                        let account_id = w3.sidebar().selected_account_id();
                                        reload_messages(
                                            &w3,
                                            &pool4,
                                            account_id,
                                            &mailbox,
                                            &emails3,
                                            is_multi_account,
                                            &msgs3,
                                        );
                                    }
                                },
                            );
                        }
                        Ok(connectivity) => {
                            info!(?connectivity, "Network state changed");
                            // Show offline/limited banner with pending op count
                            let pool3 = pool2.clone();
                            let w3 = w2.clone();
                            let conn = connectivity;
                            runtime::spawn_async(
                                async move {
                                    let queue = mq_net::queue::OfflineQueue::new(pool3);
                                    let count = queue.total_pending_count().await.unwrap_or(0);
                                    (conn, count)
                                },
                                move |(conn, pending): (mq_net::monitor::Connectivity, i64)| {
                                    let msg = if pending > 0 {
                                        format!("{conn} — {pending} pending operation{}", if pending == 1 { "" } else { "s" })
                                    } else {
                                        format!("{conn} — some features may be unavailable")
                                    };
                                    w3.show_banner(&msg);
                                },
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            debug!(skipped = n, "Connectivity receiver lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            debug!("Connectivity receiver closed");
                            break;
                        }
                    }
                }
            });

            // Keep monitor alive for the application lifetime
            std::mem::forget(monitor);
        },
    );
}

/// Send a desktop notification for new mail.
///
/// Called when IDLE detects new messages after a sync. Will be wired
/// into the full IDLE → sync → notify pipeline once token storage is complete.
#[allow(dead_code)]
fn send_new_mail_notification(app: &adw::Application, sender: &str, subject: &str) {
    let notification = gio::Notification::new("New mail");
    let body = format!("{sender}: {subject}");
    notification.set_body(Some(&body));
    app.send_notification(Some("new-mail"), &notification);
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

struct BodyResult {
    text: String,
    blocked_images: usize,
    tracking_pixels: usize,
}

async fn load_message_body(pool: &SqlitePool, message_id: i64) -> Option<BodyResult> {
    let config = mq_core::config::AppConfig::load().unwrap_or_default();
    match mq_db::queries::message_bodies::get_body(pool, message_id).await {
        Ok(Some(body)) => {
            if let Some(text) = body.text_body {
                Some(BodyResult {
                    text,
                    blocked_images: 0,
                    tracking_pixels: 0,
                })
            } else if let Some(html) = body.html_body {
                // Apply privacy sanitization to HTML
                let sanitized = mq_core::privacy::images::sanitize_html(
                    &html,
                    config.privacy.block_remote_images,
                    config.privacy.detect_tracking_pixels,
                );
                // Convert to plain text for display (no WebKitGTK yet)
                let text = mq_core::privacy::images::html_to_plain_text(&sanitized.html);
                Some(BodyResult {
                    text,
                    blocked_images: sanitized.blocked_image_count,
                    tracking_pixels: sanitized.tracking_pixel_count,
                })
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

/// Load message body without image blocking (user clicked "Load images").
async fn load_message_body_unblocked(pool: &SqlitePool, message_id: i64) -> Option<BodyResult> {
    let config = mq_core::config::AppConfig::load().unwrap_or_default();
    match mq_db::queries::message_bodies::get_body(pool, message_id).await {
        Ok(Some(body)) => {
            if let Some(text) = body.text_body {
                Some(BodyResult {
                    text,
                    blocked_images: 0,
                    tracking_pixels: 0,
                })
            } else if let Some(html) = body.html_body {
                // Don't block images, but still detect tracking pixels
                let sanitized = mq_core::privacy::images::sanitize_html(
                    &html,
                    false, // don't block
                    config.privacy.detect_tracking_pixels,
                );
                let text = mq_core::privacy::images::html_to_plain_text(&sanitized.html);
                Some(BodyResult {
                    text,
                    blocked_images: 0,
                    tracking_pixels: sanitized.tracking_pixel_count,
                })
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

/// Load message body as plain text (for compose reply/forward — no privacy counters needed).
async fn load_message_body_text(pool: &SqlitePool, message_id: i64) -> Option<String> {
    match mq_db::queries::message_bodies::get_body(pool, message_id).await {
        Ok(Some(body)) => {
            if let Some(text) = body.text_body {
                Some(text)
            } else if let Some(html) = body.html_body {
                Some(mq_core::privacy::images::html_to_plain_text(&html))
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

/// Register bundled symbolic icons as fallback.
///
/// Many icon themes (e.g. Papirus-Dark) don't include `-symbolic` icon variants
/// or don't inherit from a theme that does. We ship a small set of standard
/// symbolic icons and add them as a search path. GTK treats additional search
/// paths as fallback — the user's theme is always checked first.
fn register_bundled_icons() {
    let display = match gtk::gdk::Display::default() {
        Some(d) => d,
        None => return,
    };
    let icon_theme = gtk::IconTheme::for_display(&display);

    // Check candidate locations for the bundled icons directory:
    // 1. Relative to the binary (development: project_root/data/icons)
    // 2. Installed location (prefix/share/mq-mail/icons)
    let candidates = [
        // Development: binary is in target/debug/, icons are in data/icons
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent()?.parent()?.parent().map(|p| p.join("data/icons"))),
        // Installed via Nix or Meson: prefix/share/mq-mail/icons
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent()?.parent().map(|p| p.join("share/mq-mail/icons"))),
        // Flatpak / system install
        Some(std::path::PathBuf::from("/app/share/mq-mail/icons")),
    ];

    for candidate in &candidates {
        if let Some(path) = candidate {
            if path.join("hicolor/index.theme").exists() {
                info!("Adding bundled icon path: {}", path.display());
                icon_theme.add_search_path(path);
                return;
            }
        }
    }

    debug!("No bundled icon directory found (icons will come from system theme only)");
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
