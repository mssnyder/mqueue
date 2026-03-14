use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::{gio, glib};
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::config;
use crate::runtime;
use crate::widgets::account_setup::MqAccountSetup;
use crate::widgets::compose_window::{ComposeMode, ContactEntry, MqComposeWindow};
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

        // WORKAROUND: On Wayland, gtk-xft-dpi can be 0, which causes
        // WebKitGTK's refreshInternalScaling() to compute NaN page zoom
        // (fontDPI/96 = 0/96 = 0, then 0/0 = NaN), breaking all CSS
        // box-model rendering. Set to 96 DPI so WebKitGTK computes
        // zoom = 96/96 = 1.0. The Wayland compositor handles display
        // scaling separately — do NOT multiply by the scale factor.
        if let Some(settings) = gtk::Settings::default() {
            if settings.gtk_xft_dpi() <= 0 {
                settings.set_gtk_xft_dpi(96 * 1024);
                info!("Set temporary gtk-xft-dpi=96 (Wayland safety net)");
            }
        }

        // Register bundled fallback icons as a GResource.
        // These are treated as hicolor fallback — the user's icon theme is
        // always checked first. Only if an icon isn't found anywhere in the
        // theme's inheritance chain do these kick in.
        register_bundled_icons();

        // Set the application icon (used in the window title bar, taskbar, etc.)
        gtk::Window::set_default_icon_name(config::APP_ID);

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
    recipient_cc: Option<String>,
    list_unsubscribe: Option<String>,
    list_unsubscribe_post: Option<String>,
    gmail_thread_id: Option<i64>,
    thread_count: i64,
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

            // Unified inbox: threaded view of all accounts' INBOX
            let threads =
                mq_db::queries::messages::get_threads_for_mailbox(
                    &pool, "INBOX", 200, 0,
                )
                .await
                .unwrap_or_default();

            info!(count = threads.len(), "Loaded cached threads (unified)");

            Ok(AppData {
                accounts: account_infos,
                messages: db_to_threaded_message_data(threads),
                account_emails,
                pool: Arc::new(pool),
            })
        },
        move |result: Result<AppData, String>| match result {
            Ok(data) => {
                let accounts = data.accounts.clone();
                let pool = data.pool.clone();
                let window = window_clone.clone();
                let shared_messages = setup_ui(window_clone, data);

                // Trigger background sync for each existing account
                trigger_sync_all_accounts(&accounts, &pool, &window, &shared_messages);

                // Register the "resync" action so preferences can trigger a
                // re-sync when the user toggles sync_all_mailboxes.
                register_resync_action(&window, &accounts, &pool, &shared_messages);
            }
            Err(e) => {
                error!("Failed to load data: {e}");
                window_clone.show_banner(&format!("Error: {e}"));
            }
        },
    );
}

/// Trigger a background sync for all accounts, refreshing tokens as needed.
fn trigger_sync_all_accounts(
    accounts: &[AccountInfo],
    pool: &Arc<SqlitePool>,
    window: &MqWindow,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    for account in accounts {
        let email = account.email.clone();
        let account_id = account.id;
        let pool = pool.clone();
        let window = window.clone();
        let msgs = shared_messages.clone();

        runtime::spawn_async(
            async move {
                let stored = mq_core::keyring::get_tokens(&email).await.ok().flatten();
                let stored = match stored {
                    Some(t) => t,
                    None => return None,
                };

                let access_token = match mq_core::keyring::refresh_and_store(&email).await {
                    Ok(new_token) => {
                        tracing::info!(email = %email, "Refreshed access token for sync");
                        new_token
                    }
                    Err(e) => {
                        tracing::warn!(email = %email, error = %e, "Token refresh failed, using stored token");
                        stored.access_token
                    }
                };

                Some((access_token, email))
            },
            move |tokens: Option<(String, String)>| {
                if let Some((access_token, email)) = tokens {
                    info!(email = %email, "Starting background sync");
                    start_background_sync(window, pool, account_id, email, access_token, msgs);
                } else {
                    warn!(account_id, "No tokens in keyring — skipping sync");
                }
            },
        );
    }
}

/// Trigger a background sync for a single account (e.g. after send/delete/archive).
fn trigger_sync_account(
    account_id: i64,
    pool: &Arc<SqlitePool>,
    window: &MqWindow,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    let pool_async = pool.clone();
    let pool_cb = pool.clone();
    let window = window.clone();
    let msgs = shared_messages.clone();

    runtime::spawn_async(
        async move {
            let accounts = mq_db::queries::accounts::get_all_accounts(&pool_async)
                .await
                .unwrap_or_default();
            let account = accounts.iter().find(|a| a.id == account_id);
            let email = match account {
                Some(a) => a.email.clone(),
                None => return None,
            };

            let access_token = match mq_core::keyring::refresh_and_store(&email).await {
                Ok(t) => t,
                Err(_) => {
                    let stored = mq_core::keyring::get_tokens(&email).await.ok().flatten();
                    match stored {
                        Some(t) => t.access_token,
                        None => return None,
                    }
                }
            };

            Some((access_token, email))
        },
        move |tokens: Option<(String, String)>| {
            if let Some((access_token, email)) = tokens {
                info!(email = %email, "Triggering post-action sync");
                start_background_sync(window, pool_cb, account_id, email, access_token, msgs);
            }
        },
    );
}

/// Register the `app.resync` GIO action so the preferences window can
/// trigger a re-sync when the user toggles sync settings.
fn register_resync_action(
    window: &MqWindow,
    accounts: &[AccountInfo],
    pool: &Arc<SqlitePool>,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    let Some(app) = window.application() else {
        return;
    };

    let accounts = accounts.to_vec();
    let pool = pool.clone();
    let window = window.clone();
    let msgs = shared_messages.clone();

    let resync_action = gio::SimpleAction::new("resync", None);
    resync_action.connect_activate(move |_, _| {
        info!("Resync triggered by preferences change");
        window.show_banner("Re-syncing\u{2026}");
        trigger_sync_all_accounts(&accounts, &pool, &window, &msgs);
    });
    app.add_action(&resync_action);
}

/// Wire up the entire UI after data loads.
/// Returns the shared message list so callers (e.g. background sync) can keep it updated.
fn setup_ui(window: MqWindow, data: AppData) -> Rc<RefCell<Vec<MessageData>>> {
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
        return Rc::new(RefCell::new(vec![]));
    }

    // Shared messages data — updated on each reload, read by selection handler.
    let shared_messages: Rc<RefCell<Vec<MessageData>>> =
        Rc::new(RefCell::new(data.messages.clone()));

    let show_badge = is_multi_account;

    // Wire message selection BEFORE populating the list, so the initial
    // selection (item 0) triggers the handler and shows the first message.
    {
        let view = window.message_view();
        let pool_sel = pool.clone();
        let msgs = shared_messages.clone();
        let win_gen = window.clone();

        window
            .message_list()
            .connect_message_selected(move |msg| {
                // Bump generation so any in-flight body loads are discarded.
                let my_gen = win_gen.bump_body_load_generation();
                let gen_win = win_gen.clone();

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
                    db_id,
                );

                // Auto-mark as read when viewing (single messages and
                // the thread representative in the list)
                if !msg.is_read() {
                    msg.set_is_read(true);
                    let pool_read = pool_sel.clone();
                    let uid = msg.uid();
                    let account_id = msg.account_id();
                    let mailbox = msg.mailbox();
                    // Local DB update (fast)
                    runtime::spawn_async(
                        async move {
                            toggle_flag(&pool_read, db_id, "\\Seen", true).await;
                        },
                        |_| {},
                    );
                    // IMAP sync in background — don't block UI
                    let pool_imap = pool_sel.clone();
                    runtime::spawn_async(
                        async move {
                            if let Err(e) = imap_store_flag(
                                &pool_imap, account_id, &mailbox, uid, "\\Seen", true,
                            ).await {
                                warn!("Failed to mark as read on server: {e}");
                            }
                        },
                        |_| {},
                    );
                }

                let pool = pool_sel.clone();
                let pool_dl = pool_sel.clone();
                let v = view.clone();
                let sender = msg.sender_email();
                let account = msg.account_id();
                let thread_id = msg.gmail_thread_id();
                let thread_count = msg.thread_count();

                runtime::spawn_async(
                    async move {
                        let allowed =
                            mq_db::queries::sender_allowlist::is_allowed(&pool, account, &sender)
                                .await
                                .unwrap_or(false);

                        // If this is a thread with multiple messages, load the full thread
                        // Load attachments for this message
                        let attachments = mq_db::queries::attachments::get_attachments(&pool, db_id)
                            .await
                            .unwrap_or_default();
                        let att_data: Vec<(i64, String, String, Option<u64>)> = attachments
                            .iter()
                            .map(|a| (
                                a.id,
                                a.filename.clone().unwrap_or_default(),
                                a.mime_type.clone(),
                                a.size.map(|s| s as u64),
                            ))
                            .collect();

                        if thread_count > 1 && thread_id != 0 {
                            let thread_msgs = mq_db::queries::messages::get_thread_messages(
                                &pool, thread_id,
                            )
                            .await
                            .unwrap_or_default();

                            // Load bodies for all messages in the thread
                            // Tuple: (from, date, html, text)
                            // (from, date, html, text, is_read)
                            let mut conversation: Vec<(String, String, String, String, bool)> = Vec::new();
                            let mut total_blocked = 0usize;
                            let mut total_tracking = 0usize;

                            let thread_len = thread_msgs.len();
                            for (idx, tmsg) in thread_msgs.iter().enumerate() {
                                let from_display = tmsg
                                    .sender_name
                                    .as_deref()
                                    .filter(|n| !n.is_empty())
                                    .map(|n| format!("{n} <{}>", tmsg.sender_email))
                                    .unwrap_or_else(|| tmsg.sender_email.clone());
                                let is_read = tmsg.flags.contains("\\Seen");

                                let body = load_message_body(&pool, tmsg.id).await;
                                let body_html = body
                                    .as_ref()
                                    .and_then(|b| b.html.clone())
                                    .unwrap_or_default();
                                let body_text = body
                                    .as_ref()
                                    .map(|b| b.text.clone())
                                    .unwrap_or_else(|| {
                                        tmsg.snippet.clone().unwrap_or_default()
                                    });
                                if let Some(ref b) = body {
                                    total_blocked += b.blocked_images;
                                    total_tracking += b.tracking_pixels;
                                }

                                // Last message is auto-expanded and always shown as read
                                let show_read = is_read || idx == thread_len - 1;
                                conversation.push((from_display, tmsg.date.clone(), body_html, body_text, show_read));
                            }

                            // Mark only the last message as read (it's auto-expanded)
                            if let Some(last) = thread_msgs.last() {
                                if !last.flags.contains("\\Seen") {
                                    let last_id = last.id;
                                    let last_uid = last.uid as u32;
                                    let last_acct = last.account_id;
                                    let last_mbox = last.mailbox.clone();
                                    toggle_flag(&pool, last_id, "\\Seen", true).await;
                                    let pool_bg = pool.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = imap_store_flag(
                                            &pool_bg, last_acct, &last_mbox, last_uid, "\\Seen", true,
                                        ).await {
                                            tracing::warn!(last_id, "Failed to mark last thread msg as read: {e}");
                                        }
                                    });
                                }
                            }

                            // Collect thread metadata for expand-to-read
                            let thread_meta: Vec<(i64, u32, i64, String, bool)> = thread_msgs
                                .iter()
                                .map(|m| (m.id, m.uid as u32, m.account_id, m.mailbox.clone(), m.flags.contains("\\Seen")))
                                .collect();

                            (None, Some(conversation), allowed, total_blocked, total_tracking, true, att_data, Some(thread_meta))
                        } else {
                            // Single message — load just this one
                            let body = load_message_body(&pool, db_id).await;
                            let blocked = body.as_ref().map(|b| b.blocked_images).unwrap_or(0);
                            let tracking = body.as_ref().map(|b| b.tracking_pixels).unwrap_or(0);
                            let is_html = body.as_ref().map(|b| b.is_html).unwrap_or(false);
                            (body, None, allowed, blocked, tracking, is_html, att_data, None)
                        }
                    },
                    move |(body, conversation, sender_allowed, blocked, tracking, _is_html, att_data, thread_meta): (
                        Option<BodyResult>,
                        Option<Vec<(String, String, String, String, bool)>>,
                        bool,
                        usize,
                        usize,
                        bool,
                        Vec<(i64, String, String, Option<u64>)>,
                        Option<Vec<(i64, u32, i64, String, bool)>>,
                    )| {
                        // Discard stale result if user has already selected another message.
                        if gen_win.body_load_generation() != my_gen {
                            return;
                        }

                        if let Some(conversation) = conversation {
                            // Thread view: show full conversation
                            if let Some(meta) = thread_meta {
                                v.set_thread_message_meta(meta);
                            }
                            v.set_conversation(&conversation);
                        } else if let Some(body) = body {
                            // Single message view — prefer HTML rendering
                            if let Some(ref html) = body.html {
                                v.set_body_html(html);
                            } else {
                                v.set_body_text(&body.text);
                            }
                        }

                        // Show images banner only when remote images were actually blocked.
                        // CID (inline) images are resolved to data: URIs and don't count.
                        if !v.images_force_loaded() {
                            if blocked > 0 && !sender_allowed {
                                v.show_images_banner(blocked);
                            } else {
                                v.hide_images_banner();
                            }
                        }
                        if tracking > 0 {
                            v.show_tracking_info(tracking);
                        } else {
                            v.hide_tracking_info();
                        }

                        // Show attachments if any
                        if !att_data.is_empty() {
                            if let Some(win) = v.root().and_then(|r| r.downcast::<MqWindow>().ok()) {
                                let cb = make_attachment_download_callback(pool_dl.clone(), db_id, win);
                                v.set_attachments(&att_data, cb);
                            }
                        } else {
                            v.hide_attachments();
                        }
                    },
                );
            });
    }

    // Populate message list AFTER the selection handler is connected,
    // so the auto-selection of item 0 triggers the handler.
    let objects = make_message_objects(&data.messages, &account_emails, show_badge);
    window.message_list().set_messages(objects.clone());

    // Directly show the first message — we can't rely on selection-changed
    // because SingleSelection's internal auto-select of position 0 suppresses
    // our signal when items are added to an empty model.
    if !objects.is_empty() {
        let msg = &objects[0];
        let view = window.message_view();
        let from = if msg.sender_name().is_empty() {
            msg.sender_email()
        } else {
            format!("{} <{}>", msg.sender_name(), msg.sender_email())
        };
        let db_id = msg.db_id();
        let to = shared_messages
            .borrow()
            .iter()
            .find(|m| m.db_id == db_id)
            .map(|m| m.recipient_to.clone())
            .unwrap_or_default();
        let has_unsub = shared_messages
            .borrow()
            .iter()
            .find(|m| m.db_id == db_id)
            .map(|m| m.list_unsubscribe.is_some())
            .unwrap_or(false);
        view.show_message(
            &from,
            &to,
            &msg.date(),
            &msg.subject(),
            &msg.snippet(),
            has_unsub,
            msg.is_flagged(),
            msg.is_read(),
            db_id,
        );
        // Auto-mark first message as read in the list
        if !msg.is_read() {
            msg.set_is_read(true);
        }

        // Trigger async body load for the first message
        let v = view.clone();
        let pool_init = pool.clone();
        let pool_dl2 = pool.clone();
        let sender = msg.sender_email();
        let account = msg.account_id();
        let uid = msg.uid();
        let mailbox = msg.mailbox();
        let my_gen = window.body_load_generation();
        let gen_win2 = window.clone();
        let thread_id = msg.gmail_thread_id();
        let thread_count = msg.thread_count();
        runtime::spawn_async(
            async move {
                let allowed =
                    mq_db::queries::sender_allowlist::is_allowed(&pool_init, account, &sender)
                        .await
                        .unwrap_or(false);
                let attachments = mq_db::queries::attachments::get_attachments(&pool_init, db_id)
                    .await
                    .unwrap_or_default();
                let att_data: Vec<(i64, String, String, Option<u64>)> = attachments
                    .iter()
                    .map(|a| (
                        a.id,
                        a.filename.clone().unwrap_or_default(),
                        a.mime_type.clone(),
                        a.size.map(|s| s as u64),
                    ))
                    .collect();

                if thread_count > 1 && thread_id != 0 {
                    let thread_msgs =
                        mq_db::queries::messages::get_thread_messages(&pool_init, thread_id)
                            .await
                            .unwrap_or_default();
                    let mut conversation: Vec<(String, String, String, String, bool)> = Vec::new();
                    let mut total_blocked = 0usize;
                    let mut total_tracking = 0usize;
                    let thread_len = thread_msgs.len();
                    for (idx, tmsg) in thread_msgs.iter().enumerate() {
                        let from_display = tmsg
                            .sender_name
                            .as_deref()
                            .filter(|n| !n.is_empty())
                            .map(|n| format!("{n} <{}>", tmsg.sender_email))
                            .unwrap_or_else(|| tmsg.sender_email.clone());
                        let is_read = tmsg.flags.contains("\\Seen");
                        let body = load_message_body(&pool_init, tmsg.id).await;
                        let body_html = body
                            .as_ref()
                            .and_then(|b| b.html.clone())
                            .unwrap_or_default();
                        let body_text = body
                            .as_ref()
                            .map(|b| b.text.clone())
                            .unwrap_or_else(|| tmsg.snippet.clone().unwrap_or_default());
                        if let Some(ref b) = body {
                            total_blocked += b.blocked_images;
                            total_tracking += b.tracking_pixels;
                        }
                        let show_read = is_read || idx == thread_len - 1;
                        conversation.push((from_display, tmsg.date.clone(), body_html, body_text, show_read));
                    }
                    // Mark only the last thread message as read
                    if let Some(last) = thread_msgs.last() {
                        if !last.flags.contains("\\Seen") {
                            let last_id = last.id;
                            let last_uid = last.uid as u32;
                            let last_acct = last.account_id;
                            let last_mbox = last.mailbox.clone();
                            toggle_flag(&pool_init, last_id, "\\Seen", true).await;
                            let pool_bg = pool_init.clone();
                            tokio::spawn(async move {
                                if let Err(e) = imap_store_flag(
                                    &pool_bg, last_acct, &last_mbox, last_uid, "\\Seen", true,
                                ).await {
                                    tracing::warn!(last_id, "Failed to mark last thread msg as read: {e}");
                                }
                            });
                        }
                    }
                    (None, Some(conversation), allowed, total_blocked, total_tracking, true, att_data)
                } else {
                    // Single message — mark as read
                    toggle_flag(&pool_init, db_id, "\\Seen", true).await;
                    let pool_bg = pool_init.clone();
                    let uid_val = uid;
                    let acct_val = account;
                    let mbox_val = mailbox.clone();
                    tokio::spawn(async move {
                        if let Err(e) = imap_store_flag(
                            &pool_bg, acct_val, &mbox_val, uid_val, "\\Seen", true,
                        ).await {
                            tracing::warn!(db_id, "Failed to mark as read on server: {e}");
                        }
                    });
                    let body = load_message_body(&pool_init, db_id).await;
                    let blocked = body.as_ref().map(|b| b.blocked_images).unwrap_or(0);
                    let tracking = body.as_ref().map(|b| b.tracking_pixels).unwrap_or(0);
                    let is_html = body.as_ref().map(|b| b.is_html).unwrap_or(false);
                    (body, None, allowed, blocked, tracking, is_html, att_data)
                }
            },
            move |(body, conversation, sender_allowed, blocked, tracking, _is_html, att_data): (
                Option<BodyResult>,
                Option<Vec<(String, String, String, String, bool)>>,
                bool,
                usize,
                usize,
                bool,
                Vec<(i64, String, String, Option<u64>)>,
            )| {
                if gen_win2.body_load_generation() != my_gen { return; }

                if let Some(conversation) = conversation {
                    v.set_conversation(&conversation);
                } else if let Some(body) = body {
                    if let Some(ref html) = body.html {
                        v.set_body_html(html);
                    } else {
                        v.set_body_text(&body.text);
                    }
                }
                if !v.images_force_loaded() {
                    if blocked > 0 && !sender_allowed {
                        v.show_images_banner(blocked);
                    } else {
                        v.hide_images_banner();
                    }
                }
                if tracking > 0 {
                    v.show_tracking_info(tracking);
                } else {
                    v.hide_tracking_info();
                }
                if !att_data.is_empty() {
                    if let Some(win) = v.root().and_then(|r| r.downcast::<MqWindow>().ok()) {
                        let cb = make_attachment_download_callback(pool_dl2.clone(), db_id, win);
                        v.set_attachments(&att_data, cb);
                    }
                } else {
                    v.hide_attachments();
                }
            },
        );
    }

    // Wire action buttons (star, read, delete, archive)
    wire_action_buttons(&window, &pool, &shared_messages);

    // Wire compose & reply buttons
    wire_compose_buttons(&window, &pool, &data.accounts, &shared_messages);

    // Wire privacy / unsubscribe buttons
    wire_privacy_buttons(&window, &pool, &shared_messages);

    // Re-render email view when the user switches between light and dark mode
    {
        let view = window.message_view();
        adw::StyleManager::default().connect_dark_notify(move |_| {
            view.reload_for_theme_change();
        });
    }

    // Wire sort order toggle
    {
        let ml = window.message_list();
        let ml2 = ml.clone();
        let msgs = shared_messages.clone();
        let sort_win = window.clone();
        ml.connect_sort_changed(move |newest_first| {
            sort_win.imp().sort_newest_first.set(newest_first);
            let model = ml2.model();
            let n = model.n_items();
            if n == 0 {
                return;
            }

            // Collect current items, reverse, and re-populate
            let mut items: Vec<MessageObject> = Vec::with_capacity(n as usize);
            for i in 0..n {
                if let Some(item) = model.item(i) {
                    if let Ok(msg) = item.downcast::<MessageObject>() {
                        items.push(msg);
                    }
                }
            }

            // Sort by date string (ISO format sorts lexicographically)
            if newest_first {
                items.sort_by(|a, b| b.date().cmp(&a.date()));
            } else {
                items.sort_by(|a, b| a.date().cmp(&b.date()));
            }

            // Also update shared messages to match
            {
                let mut m = msgs.borrow_mut();
                if newest_first {
                    m.sort_by(|a, b| b.date.cmp(&a.date));
                } else {
                    m.sort_by(|a, b| a.date.cmp(&b.date));
                }
            }

            ml2.refresh_messages(items);
        });
    }

    // Wire load-more pagination: when user scrolls near bottom, fetch next batch
    {
        let w = window.clone();
        let p = pool.clone();
        let m = shared_messages.clone();
        let e = account_emails.clone();
        let n = data.accounts.len();
        window.message_list().connect_load_more(move || {
            // Don't load more messages if user is viewing search results
            if w.is_search_active() {
                w.message_list().load_more_finished();
                return;
            }
            let mailbox = w.sidebar().selected_mailbox();
            let account_id = w.sidebar().selected_account_id();
            let current_count = m.borrow().len() as i64;
            let pool = p.clone();
            let msgs = m.clone();
            let emails = e.clone();
            let ml = w.message_list();
            let show_badge = n > 1 && account_id.is_none();
            let sort_newest = w.imp().sort_newest_first.get();
            runtime::spawn_async(
                async move {
                    match account_id {
                        Some(aid) => {
                            mq_db::queries::messages::get_threads_for_account_mailbox(
                                &pool, aid, &mailbox, 200, current_count,
                            )
                            .await
                            .unwrap_or_default()
                        }
                        None => {
                            mq_db::queries::messages::get_threads_for_mailbox(
                                &pool, &mailbox, 200, current_count,
                            )
                            .await
                            .unwrap_or_default()
                        }
                    }
                },
                move |threads: Vec<(mq_db::models::DbMessage, i64)>| {
                    if threads.is_empty() {
                        ml.load_more_finished();
                        return;
                    }
                    let mut data = db_to_threaded_message_data(threads);
                    if !sort_newest {
                        data.sort_by(|a, b| a.date.cmp(&b.date));
                    }
                    let objects = make_message_objects(&data, &emails, show_badge);
                    msgs.borrow_mut().extend(data);
                    ml.append_messages(objects);
                },
            );
        });
    }

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
                .body(format!("This will permanently remove all cached emails, contacts, and settings for {email}. This cannot be undone."))
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
                        // Delete all account-related data from DB
                        // First delete child tables that reference messages
                        let _ = sqlx::query(
                            "DELETE FROM message_bodies WHERE message_id IN (SELECT id FROM messages WHERE account_id = ?)"
                        )
                            .bind(account_id)
                            .execute(&*pool)
                            .await;
                        let _ = sqlx::query(
                            "DELETE FROM attachments WHERE message_id IN (SELECT id FROM messages WHERE account_id = ?)"
                        )
                            .bind(account_id)
                            .execute(&*pool)
                            .await;
                        let _ = sqlx::query("DELETE FROM offline_queue WHERE account_id = ?")
                            .bind(account_id)
                            .execute(&*pool)
                            .await;
                        let _ = sqlx::query("DELETE FROM sender_image_allowlist WHERE account_id = ?")
                            .bind(account_id)
                            .execute(&*pool)
                            .await;
                        let _ = sqlx::query("DELETE FROM drafts WHERE account_id = ?")
                            .bind(account_id)
                            .execute(&*pool)
                            .await;
                        let _ = sqlx::query("DELETE FROM contacts WHERE account_id = ?")
                            .bind(account_id)
                            .execute(&*pool)
                            .await;
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

    // Wire the banner's action button: Retry → re-sync, Re-authenticate → add account flow
    {
        let w = window.clone();
        let p = pool.clone();
        window.connect_banner_button(move || {
            let banner_label = w.banner_button_label().unwrap_or_default();
            w.hide_banner();
            if banner_label.contains("Re-authenticate") {
                show_account_setup(&w, &p);
            } else {
                adw::prelude::ActionGroupExt::activate_action(&w, "app.resync", None);
            }
        });
    }

    shared_messages
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
                            let shared_messages = setup_ui(window2.clone(), data);

                            // Phase 3: Background sync with progress banner
                            window2.show_banner("Syncing inbox\u{2026}");
                            start_background_sync(window2, pool2, account_id, email, access_token, shared_messages);
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
    shared_messages: Rc<RefCell<Vec<MessageData>>>,
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
                tracing::warn!(email = %email, error = %e, "IMAP connect failed, attempting token refresh");
                let _ = tx.send(SyncProgress::Status("Refreshing token\u{2026}".into())).await;

                match mq_core::keyring::refresh_and_store(&email).await {
                    Ok(new_token) => {
                        tracing::info!(email = %email, "Token refreshed, retrying IMAP connect");
                        match ImapSession::connect(&email, &new_token).await {
                            Ok(s) => s,
                            Err(e2) => {
                                let _ = tx.send(SyncProgress::Error { message: format!("IMAP connect failed after token refresh: {e2}"), is_auth: true }).await;
                                return;
                            }
                        }
                    }
                    Err(refresh_err) => {
                        let _ = tx.send(SyncProgress::Error { message: format!("IMAP connect failed: {e} (token refresh also failed: {refresh_err})"), is_auth: true }).await;
                        return;
                    }
                }
            }
        };

        let _ = tx.send(SyncProgress::Status("Fetching messages\u{2026}".into())).await;

        // Load config to determine which mailboxes to sync
        let config = mq_core::config::AppConfig::load().unwrap_or_default();

        // Always sync the standard Gmail mailboxes shown in the sidebar.
        // When sync_all_mailboxes is enabled, also sync user labels and
        // any other mailboxes the server advertises.
        let mut mailboxes: Vec<String> = vec![
            "INBOX".to_string(),
            "[Gmail]/Starred".to_string(),
            "[Gmail]/Sent Mail".to_string(),
            "[Gmail]/Drafts".to_string(),
            "[Gmail]/Trash".to_string(),
            "[Gmail]/Spam".to_string(),
            "[Gmail]/All Mail".to_string(),
        ];

        if config.sync.sync_all_mailboxes {
            match session.list_mailboxes().await {
                Ok(list) => {
                    // Add any mailboxes not already in our standard set
                    for mb in list {
                        if !mailboxes.contains(&mb) {
                            mailboxes.push(mb);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to list additional mailboxes: {e}");
                }
            }
        }

        let mut idle_started = false;
        let mut new_inbox_messages: Vec<NewMailInfo> = Vec::new();

        for mailbox in &mailboxes {
            // Simple display name: strip leading "[Gmail]/" prefix if present
            let display = if let Some(stripped) = mailbox.strip_prefix("[Gmail]/") {
                stripped
            } else {
                mailbox.as_str()
            };

            let _ = tx.send(SyncProgress::Status(format!("Syncing {display}\u{2026}"))).await;

            // Load previous sync state for incremental sync
            let prev_state = mq_db::queries::sync_state::get_sync_state(&pool, account_id, mailbox)
                .await
                .ok()
                .flatten()
                .map(|s| sync::SyncState {
                    mailbox: s.mailbox,
                    uid_validity: s.uid_validity as u32,
                    highest_modseq: s.highest_modseq as u64,
                    highest_uid: s.highest_uid as u32,
                });

            let known_uids: Vec<u32> = mq_db::queries::messages::get_known_uids(&pool, account_id, mailbox)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|u| u as u32)
                .collect();

            if prev_state.is_some() {
                tracing::info!(
                    mailbox = %mailbox,
                    known_uids = known_uids.len(),
                    "Resuming incremental sync with previous state"
                );
            }

            let outcome = match sync::sync_mailbox(&mut session, mailbox, prev_state.as_ref(), &known_uids).await {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!(mailbox = %mailbox, error = %e, "Sync failed for mailbox, skipping");
                    let _ = tx.send(SyncProgress::Status(format!("Skipping {display} (error)"))).await;
                    continue;
                }
            };

            // Delete expunged messages (no longer on server)
            if !outcome.expunged_uids.is_empty() {
                match mq_db::queries::messages::delete_expunged(
                    &pool, account_id, mailbox, &outcome.expunged_uids,
                ).await {
                    Ok(count) => {
                        if count > 0 {
                            tracing::info!(mailbox = %mailbox, count, "Deleted expunged messages from local DB");
                        }
                    }
                    Err(e) => tracing::warn!(mailbox = %mailbox, error = %e, "Failed to delete expunged messages"),
                }
            }

            let total = outcome.new_messages.len();
            if total > 0 {
                let _ = tx.send(SyncProgress::Status(format!("Syncing {display}\u{2026} 0/{total} messages"))).await;
            }

            // Track highest UID seen so far for incremental state saves.
            // We always track the true highest UID regardless of processing order.
            let mut running_highest_uid = prev_state.as_ref().map(|s| s.highest_uid).unwrap_or(0) as i64;
            if let Some(max_uid) = outcome.new_messages.iter().map(|m| m.uid as i64).max() {
                if max_uid > running_highest_uid {
                    running_highest_uid = max_uid;
                }
            }

            // Process newest messages first so the user sees recent mail quickly.
            let mut messages_to_process: Vec<_> = outcome.new_messages.iter().collect();
            messages_to_process.reverse();

            // Persist messages to DB in batches, sending progress
            for (i, email_msg) in messages_to_process.iter().enumerate() {
                let db_id = persist_email_to_db(&pool, account_id, mailbox, email_msg, &outcome.new_state).await;

                // Fetch and persist message body (skip if already cached)
                if let Some(db_id) = db_id {
                    let already_has_body = mq_db::queries::message_bodies::has_body(&pool, db_id)
                        .await
                        .unwrap_or(false);

                    if !already_has_body {
                        match session.fetch_body(email_msg.uid).await {
                            Ok(Some(raw)) => {
                                let parsed = mq_core::body::parse_mime(&raw);

                                if let Err(e) = mq_db::queries::message_bodies::upsert_body(
                                    &pool,
                                    db_id,
                                    Some(&raw),
                                    parsed.html.as_deref(),
                                    parsed.text.as_deref(),
                                )
                                .await
                                {
                                    tracing::warn!(db_id, error = %e, "Failed to persist message body");
                                }

                                // Save snippet to messages table for list display
                                if let Some(ref snippet) = parsed.snippet {
                                    let _ = mq_db::queries::messages::update_snippet(
                                        &pool, db_id, snippet,
                                    ).await;
                                }

                                // Store attachment metadata
                                if !parsed.attachments.is_empty() {
                                    for att in &parsed.attachments {
                                        let _ = mq_db::queries::attachments::insert_attachment(
                                            &pool,
                                            db_id,
                                            att.filename.as_deref(),
                                            &att.mime_type,
                                            att.size.map(|s| s as i64),
                                            att.content_id.as_deref(),
                                            &att.imap_section,
                                        ).await;
                                    }
                                    // Update has_attachments flag on message
                                    let _ = sqlx::query(
                                        "UPDATE messages SET has_attachments = 1 WHERE id = ?",
                                    )
                                    .bind(db_id)
                                    .execute(&*pool)
                                    .await;
                                }
                            }
                            Ok(None) => {
                                tracing::debug!(uid = email_msg.uid, "No body returned for message");
                            }
                            Err(e) => {
                                tracing::warn!(uid = email_msg.uid, error = %e, "Failed to fetch message body");
                            }
                        }
                    }

                    // Persist labels
                    if !email_msg.labels.is_empty() {
                        let mut label_ids = Vec::new();
                        for label in &email_msg.labels {
                            let label_type = if label.starts_with('\\') { "system" } else { "user" };
                            match mq_db::queries::labels::upsert_label(
                                &pool,
                                account_id,
                                label,
                                label,
                                label_type,
                            )
                            .await
                            {
                                Ok(lid) => label_ids.push(lid),
                                Err(e) => {
                                    tracing::warn!(label = %label, error = %e, "Failed to upsert label");
                                }
                            }
                        }
                        if !label_ids.is_empty() {
                            if let Err(e) = mq_db::queries::labels::set_message_labels(&pool, db_id, &label_ids).await {
                                tracing::warn!(db_id, error = %e, "Failed to set message labels");
                            }
                        }
                    }
                }

                // Send progress + save sync state every 50 messages
                // so restarts don't re-download everything
                if (i + 1) % 25 == 0 || i + 1 == total {
                    let _ = tx.send(SyncProgress::Status(format!("Syncing {display}\u{2026} {}/{total} messages", i + 1))).await;
                    let _ = tx.send(SyncProgress::Count(i + 1, total)).await;
                }
                if (i + 1) % 50 == 0 {
                    let _ = mq_db::queries::sync_state::upsert_sync_state(
                        &pool,
                        account_id,
                        mailbox,
                        outcome.new_state.uid_validity as i64,
                        outcome.new_state.highest_modseq as i64,
                        running_highest_uid,
                    )
                    .await;
                }
            }

            // Collect new INBOX messages for notifications
            if mailbox == "INBOX" && total > 0 {
                for email_msg in &outcome.new_messages {
                    let sender = email_msg.from.as_ref()
                        .map(|a| a.name.as_deref().unwrap_or(&a.email).to_string())
                        .unwrap_or_else(|| "Unknown sender".to_string());
                    let subject = email_msg.subject.as_deref().unwrap_or("(no subject)").to_string();
                    let snippet = email_msg.snippet.as_deref().unwrap_or("").to_string();
                    new_inbox_messages.push(NewMailInfo {
                        db_id: 0,
                        sender,
                        subject,
                        snippet,
                    });
                }
            }
            // Save final sync state for this mailbox
            let _ = mq_db::queries::sync_state::upsert_sync_state(
                &pool,
                account_id,
                mailbox,
                outcome.new_state.uid_validity as i64,
                outcome.new_state.highest_modseq as i64,
                outcome.new_state.highest_uid as i64,
            )
            .await;

            // After INBOX sync, start IDLE on a second connection so new
            // incoming mail is immediately detected even while remaining
            // mailboxes are still syncing.
            if mailbox == "INBOX" && !idle_started {
                idle_started = true;
                let idle_email = email.clone();
                let idle_token = access_token.clone();
                let idle_pool = pool.clone();
                let idle_tx = tx.clone();
                let idle_account_id = account_id;
                tokio::spawn(async move {
                    use mq_core::imap::client::ImapSession;
                    use mq_core::imap::idle::{idle_loop, IdleEvent};
                    use tokio::sync::mpsc as tokio_mpsc;

                    let idle_session = match ImapSession::connect(&idle_email, &idle_token).await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("Failed to open IDLE connection: {e}");
                            return;
                        }
                    };
                    tracing::info!("Started IDLE connection alongside sync");

                    let (cancel_tx, cancel_rx) = tokio_mpsc::channel::<()>(1);
                    let (event_tx, mut event_rx) = tokio_mpsc::unbounded_channel::<IdleEvent>();
                    let _cancel_guard = cancel_tx;

                    let mut maybe_session = idle_loop(idle_session, "INBOX", event_tx.clone(), cancel_rx).await;

                    while let Some(event) = event_rx.recv().await {
                        match event {
                            IdleEvent::NewData => {
                                let session = match maybe_session.take() {
                                    Some(s) => s,
                                    None => break,
                                };

                                let _ = idle_tx.send(SyncProgress::NewMail).await;

                                let (synced_session, new_msgs) = run_idle_sync(
                                    session, &idle_pool, idle_account_id, &idle_tx,
                                ).await;

                                match synced_session {
                                    Some(s) => {
                                        if !new_msgs.is_empty() {
                                            let _ = idle_tx.send(SyncProgress::Done(new_msgs)).await;
                                        } else {
                                            let _ = idle_tx.send(SyncProgress::IdleResumed).await;
                                        }
                                        let (_new_cancel_tx, new_cancel_rx) = tokio_mpsc::channel::<()>(1);
                                        maybe_session = idle_loop(s, "INBOX", event_tx.clone(), new_cancel_rx).await;
                                    }
                                    None => {
                                        let _ = idle_tx.send(SyncProgress::Error { message: "IDLE connection lost during sync".into(), is_auth: false }).await;
                                        break;
                                    }
                                }
                            }
                            IdleEvent::Timeout => continue,
                            IdleEvent::ConnectionLost => {
                                tracing::warn!("IDLE connection lost during bulk sync");
                                // Will be re-established after bulk sync finishes
                                break;
                            }
                        }
                    }

                    if let Some(session) = maybe_session {
                        let _ = session.logout().await;
                    }
                });
            }
        }

        // --- Contacts sync (Google People API) ---
        let _ = tx.send(SyncProgress::Status("Syncing contacts\u{2026}".into())).await;
        {
            // Get a fresh token for the People API
            let contacts_token = match mq_core::keyring::get_tokens(&email).await {
                Ok(Some(t)) => t.access_token,
                _ => access_token.clone(),
            };
            let contacts = mq_core::contacts::fetch_contacts_graceful(&contacts_token).await;
            if !contacts.is_empty() {
                let mut saved = 0u32;
                for c in &contacts {
                    if mq_db::queries::contacts::upsert_contact(
                        &pool,
                        account_id,
                        if c.resource_id.is_empty() { None } else { Some(c.resource_id.as_str()) },
                        c.display_name.as_deref(),
                        &c.email,
                    )
                    .await
                    .is_ok()
                    {
                        saved += 1;
                    }
                }
                tracing::info!(saved, "Contacts synced to local DB");
            }
        }

        let _ = tx.send(SyncProgress::Done(new_inbox_messages)).await;

        // --- IMAP IDLE: stay connected and watch for new mail ---

        use mq_core::imap::idle::{idle_loop, IdleEvent};
        use tokio::sync::mpsc as tokio_mpsc;

        let (cancel_tx, cancel_rx) = tokio_mpsc::channel::<()>(1);
        let (event_tx, mut event_rx) = tokio_mpsc::unbounded_channel::<IdleEvent>();

        // Keep cancel_tx alive so IDLE doesn't get cancelled prematurely.
        // It will be dropped when the entire spawned task exits (app shutdown).
        let _cancel_guard = cancel_tx;

        // Enter initial IDLE
        let mut maybe_session = idle_loop(session, "INBOX", event_tx.clone(), cancel_rx).await;

        // Process IDLE events in a loop
        while let Some(event) = event_rx.recv().await {
            match event {
                IdleEvent::NewData => {
                    let session = match maybe_session.take() {
                        Some(s) => s,
                        None => break, // lost session
                    };

                    let _ = tx.send(SyncProgress::Status("New mail — syncing\u{2026}".into())).await;
                    let _ = tx.send(SyncProgress::NewMail).await;

                    // Run incremental sync on INBOX
                    let (synced_session, new_msgs) = run_idle_sync(
                        session, &pool, account_id, &tx,
                    ).await;

                    match synced_session {
                        Some(s) => {
                            if !new_msgs.is_empty() {
                                let _ = tx.send(SyncProgress::Done(new_msgs)).await;
                            } else {
                                let _ = tx.send(SyncProgress::IdleResumed).await;
                            }
                            // Re-enter IDLE
                            let (_new_cancel_tx, new_cancel_rx) = tokio_mpsc::channel::<()>(1);
                            maybe_session = idle_loop(s, "INBOX", event_tx.clone(), new_cancel_rx).await;
                        }
                        None => {
                            let _ = tx.send(SyncProgress::Error { message: "Lost connection during sync".into(), is_auth: false }).await;
                            break;
                        }
                    }
                }
                IdleEvent::Timeout => {
                    // Re-entered automatically by idle_loop
                    continue;
                }
                IdleEvent::ConnectionLost => {
                    tracing::warn!("IDLE connection lost, will retry in 30s");
                    let _ = tx.send(SyncProgress::Status("Connection lost — reconnecting\u{2026}".into())).await;

                    // Wait and try to reconnect
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;

                    // Get fresh token
                    let new_token = match mq_core::keyring::refresh_and_store(&email).await {
                        Ok(t) => t,
                        Err(e) => {
                            let _ = tx.send(SyncProgress::Error { message: format!("Reconnect failed: {e}"), is_auth: true }).await;
                            break;
                        }
                    };

                    match ImapSession::connect(&email, &new_token).await {
                        Ok(s) => {
                            let _ = tx.send(SyncProgress::IdleResumed).await;
                            let (_new_cancel_tx, new_cancel_rx) = tokio_mpsc::channel::<()>(1);
                            maybe_session = idle_loop(s, "INBOX", event_tx.clone(), new_cancel_rx).await;
                        }
                        Err(e) => {
                            let _ = tx.send(SyncProgress::Error { message: format!("Reconnect failed: {e}"), is_auth: false }).await;
                            break;
                        }
                    }
                }
            }
        }

        // If we exit the IDLE loop, try to log out cleanly
        if let Some(session) = maybe_session {
            let _ = session.logout().await;
        }
    });

    // UI-side: receive progress updates on the GTK main thread.
    // This channel stays open for the lifetime of the IDLE connection,
    // so we don't break on Done — only on Error or channel close.
    let w = window.clone();
    let p = pool;
    let sm = shared_messages;
    glib::spawn_future_local(async move {
        while let Ok(progress) = rx.recv().await {
            match progress {
                SyncProgress::Status(msg) => {
                    w.show_banner(&msg);
                }
                SyncProgress::Count(done, total) => {
                    let fraction = if total > 0 {
                        done as f64 / total as f64
                    } else {
                        0.0
                    };
                    w.hide_banner();
                    w.show_progress(
                        &format!("Syncing\u{2026} {done}/{total} messages"),
                        fraction,
                    );
                    refresh_message_list_from_db(&w, &p, &sm);
                }
                SyncProgress::Error { message: msg, is_auth } => {
                    error!("Background sync error: {msg}");
                    w.hide_progress();
                    if is_auth {
                        // Show auth error with actionable re-authenticate button
                        w.show_banner_with_action(
                            "Session expired. Click to re-authenticate.",
                            "Re-authenticate",
                        );
                    } else {
                        // Show error with retry button (no auto-dismiss)
                        w.show_banner_with_action(&format!("Sync error: {msg}"), "Retry");
                    }
                    break;
                }
                SyncProgress::Done(new_msgs) => {
                    let total = new_msgs.len();
                    info!(total, "Sync complete");
                    w.hide_progress();
                    w.hide_banner();
                    // Reload the selected thread so new messages show up
                    refresh_and_reload_selected(&w, &p, &sm);
                    // Send per-message desktop notifications for new inbox mail
                    if !new_msgs.is_empty() && !w.is_active() {
                        if let Some(app) = w.application() {
                            if let Some(adw_app) = app.downcast_ref::<adw::Application>() {
                                send_per_message_notifications(adw_app, &new_msgs);
                            }
                        }
                    }
                    // Don't break — IDLE keeps the channel alive
                }
                SyncProgress::NewMail => {
                    // Brief banner while IDLE-triggered sync runs
                    w.show_banner("New mail — syncing\u{2026}");
                }
                SyncProgress::IdleResumed => {
                    w.hide_progress();
                    w.hide_banner();
                }
            }
        }
    });
}

/// Info about a new inbox message (for notifications).
#[derive(Debug, Clone)]
struct NewMailInfo {
    #[allow(dead_code)]
    db_id: i64,
    sender: String,
    subject: String,
    snippet: String,
}

enum SyncProgress {
    Status(String),
    Count(usize, usize),
    Error { message: String, is_auth: bool },
    /// Sync complete. Carries new inbox messages for notification display.
    Done(Vec<NewMailInfo>),
    /// IDLE detected new mail — show a brief indicator.
    NewMail,
    /// IDLE resumed watching — clear any banners.
    IdleResumed,
}

/// Run an incremental sync on INBOX after IDLE detects new data.
/// Returns the session (if still valid) and the count of new messages.
async fn run_idle_sync(
    mut session: mq_core::imap::client::ImapSession,
    pool: &SqlitePool,
    account_id: i64,
    tx: &async_channel::Sender<SyncProgress>,
) -> (Option<mq_core::imap::client::ImapSession>, Vec<NewMailInfo>) {
    use mq_core::imap::sync;

    let mailbox = "INBOX";

    let prev_state = mq_db::queries::sync_state::get_sync_state(pool, account_id, mailbox)
        .await
        .ok()
        .flatten()
        .map(|s| sync::SyncState {
            mailbox: s.mailbox,
            uid_validity: s.uid_validity as u32,
            highest_modseq: s.highest_modseq as u64,
            highest_uid: s.highest_uid as u32,
        });

    let known_uids: Vec<u32> = mq_db::queries::messages::get_known_uids(pool, account_id, mailbox)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|u| u as u32)
        .collect();

    let outcome = match sync::sync_mailbox(&mut session, mailbox, prev_state.as_ref(), &known_uids).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "IDLE sync failed");
            let _ = tx.send(SyncProgress::Error { message: format!("Sync failed: {e}"), is_auth: false }).await;
            return (None, vec![]);
        }
    };

    // Delete expunged messages
    if !outcome.expunged_uids.is_empty() {
        if let Ok(count) = mq_db::queries::messages::delete_expunged(
            pool, account_id, mailbox, &outcome.expunged_uids,
        ).await {
            if count > 0 {
                tracing::info!(count, "IDLE sync: deleted expunged messages");
            }
        }
    }

    let total = outcome.new_messages.len();

    for (i, email_msg) in outcome.new_messages.iter().enumerate() {
        let db_id = persist_email_to_db(pool, account_id, mailbox, email_msg, &outcome.new_state).await;

        if let Some(db_id) = db_id {
            // Skip body fetch if already cached
            let already_has_body = mq_db::queries::message_bodies::has_body(pool, db_id)
                .await
                .unwrap_or(false);

            if !already_has_body {
                match session.fetch_body(email_msg.uid).await {
                    Ok(Some(raw)) => {
                        let parsed = mq_core::body::parse_mime(&raw);
                        let _ = mq_db::queries::message_bodies::upsert_body(
                            pool, db_id, Some(&raw), parsed.html.as_deref(), parsed.text.as_deref(),
                        ).await;
                        // Save snippet to messages table for list display
                        if let Some(ref snippet) = parsed.snippet {
                            let _ = mq_db::queries::messages::update_snippet(pool, db_id, snippet).await;
                        }
                        // Store attachment metadata
                        if !parsed.attachments.is_empty() {
                            for att in &parsed.attachments {
                                let _ = mq_db::queries::attachments::insert_attachment(
                                    pool,
                                    db_id,
                                    att.filename.as_deref(),
                                    &att.mime_type,
                                    att.size.map(|s| s as i64),
                                    att.content_id.as_deref(),
                                    &att.imap_section,
                                ).await;
                            }
                            let _ = sqlx::query(
                                "UPDATE messages SET has_attachments = 1 WHERE id = ?",
                            )
                            .bind(db_id)
                            .execute(pool)
                            .await;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(uid = email_msg.uid, error = %e, "Failed to fetch body during IDLE sync");
                    }
                }
            }

            if !email_msg.labels.is_empty() {
                let mut label_ids = Vec::new();
                for label in &email_msg.labels {
                    let label_type = if label.starts_with('\\') { "system" } else { "user" };
                    if let Ok(lid) = mq_db::queries::labels::upsert_label(pool, account_id, label, label, label_type).await {
                        label_ids.push(lid);
                    }
                }
                if !label_ids.is_empty() {
                    let _ = mq_db::queries::labels::set_message_labels(pool, db_id, &label_ids).await;
                }
            }
        }

        if total > 5 && ((i + 1) % 5 == 0 || i + 1 == total) {
            let _ = tx.send(SyncProgress::Count(i + 1, total)).await;
        }
    }

    // Save sync state
    let _ = mq_db::queries::sync_state::upsert_sync_state(
        pool, account_id, mailbox,
        outcome.new_state.uid_validity as i64,
        outcome.new_state.highest_modseq as i64,
        outcome.new_state.highest_uid as i64,
    ).await;

    // Collect new inbox message info for notifications
    let new_msgs: Vec<NewMailInfo> = outcome.new_messages.iter().map(|m| NewMailInfo {
        db_id: 0,
        sender: m.from.as_ref()
            .map(|a| a.name.as_deref().unwrap_or(&a.email).to_string())
            .unwrap_or_else(|| "Unknown sender".to_string()),
        subject: m.subject.as_deref().unwrap_or("(no subject)").to_string(),
        snippet: m.snippet.as_deref().unwrap_or("").to_string(),
    }).collect();

    (Some(session), new_msgs)
}

/// Quick refresh of the message list from DB (called during/after sync).
/// Uses threaded query to group messages by gmail_thread_id.
/// Also updates `shared_messages` so the selection handler has current data.
/// Uses `refresh_messages` to preserve the user's current selection.
/// When no message was previously selected, directly triggers the message
/// view for item 0 (cannot rely on `selection-changed` signal due to
/// GTK `SingleSelection` auto-select races).
fn refresh_message_list_from_db(
    window: &MqWindow,
    pool: &Arc<SqlitePool>,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    refresh_message_list_impl(window, pool, shared_messages, false);
}

/// Like `refresh_message_list_from_db` but also reloads the currently selected
/// message/thread view (e.g. after send or IDLE sync delivers new messages).
fn refresh_and_reload_selected(
    window: &MqWindow,
    pool: &Arc<SqlitePool>,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
) {
    refresh_message_list_impl(window, pool, shared_messages, true);
}

fn refresh_message_list_impl(
    window: &MqWindow,
    pool: &Arc<SqlitePool>,
    shared_messages: &Rc<RefCell<Vec<MessageData>>>,
    reload_selected: bool,
) {
    let ml = window.message_list();
    let view = window.message_view();
    let pool = pool.clone();
    let pool2 = pool.clone();
    let msgs = shared_messages.clone();
    let msgs2 = shared_messages.clone();
    let my_gen = window.body_load_generation();
    let gen_win = window.clone();
    // Use the currently selected mailbox, not hardcoded INBOX
    let current_mailbox = window.sidebar().selected_mailbox();
    runtime::spawn_async(
        async move {
            mq_db::queries::messages::get_threads_for_mailbox(
                &pool, &current_mailbox, 200, 0,
            )
            .await
            .unwrap_or_default()
        },
        move |threads: Vec<(mq_db::models::DbMessage, i64)>| {
            let data = db_to_threaded_message_data(threads);
            let empty_emails = HashMap::new();
            let objects = make_message_objects(&data, &empty_emails, false);
            *msgs.borrow_mut() = data;

            // refresh_messages suppresses selection-changed during model swap
            // when re-selecting the same message, so the message view won't reload.
            let had_selection = {
                let sel = ml.selection();
                sel.selected() != gtk::INVALID_LIST_POSITION
                    && sel.selected_item().is_some()
            };
            ml.refresh_messages(objects);

            if had_selection {
                if reload_selected {
                    // Force the selection handler to re-fire so the thread
                    // view reloads (e.g. after send or IDLE sync).
                    let sel = ml.selection();
                    let pos = sel.selected();
                    sel.set_selected(gtk::INVALID_LIST_POSITION);
                    let s = sel.clone();
                    glib::idle_add_local_once(move || {
                        s.set_autoselect(true);
                        s.set_selected(pos);
                    });
                }
                return;
            }

            // If there was no prior selection, directly show the first message.
            // We can't rely on `selection-changed` because SingleSelection's
            // internal auto-select of position 0 can suppress our signal.
            let model = ml.model();
            if model.n_items() > 0 {
                if let Some(item) = model.item(0) {
                    if let Ok(msg) = item.downcast::<MessageObject>() {
                        let from = if msg.sender_name().is_empty() {
                            msg.sender_email()
                        } else {
                            format!("{} <{}>", msg.sender_name(), msg.sender_email())
                        };
                        let db_id = msg.db_id();
                        let to = {
                            let m = msgs2.borrow();
                            m.iter()
                                .find(|m| m.db_id == db_id)
                                .map(|m| m.recipient_to.clone())
                                .unwrap_or_default()
                        };
                        let has_unsub = {
                            let m = msgs2.borrow();
                            m.iter()
                                .find(|m| m.db_id == db_id)
                                .map(|m| m.list_unsubscribe.is_some())
                                .unwrap_or(false)
                        };
                        view.show_message(
                            &from,
                            &to,
                            &msg.date(),
                            &msg.subject(),
                            &msg.snippet(),
                            has_unsub,
                            msg.is_flagged(),
                            msg.is_read(),
                            db_id,
                        );
                        let v = view.clone();
                        let pool_dl3 = pool2.clone();
                        let sender = msg.sender_email();
                        let account = msg.account_id();
                        let gen_win3 = gen_win.clone();
                        runtime::spawn_async(
                            async move {
                                let allowed =
                                    mq_db::queries::sender_allowlist::is_allowed(
                                        &pool2, account, &sender,
                                    )
                                    .await
                                    .unwrap_or(false);
                                let body = load_message_body(&pool2, db_id).await;
                                let blocked =
                                    body.as_ref().map(|b| b.blocked_images).unwrap_or(0);
                                let tracking =
                                    body.as_ref().map(|b| b.tracking_pixels).unwrap_or(0);
                                let is_html =
                                    body.as_ref().map(|b| b.is_html).unwrap_or(false);
                                let attachments = mq_db::queries::attachments::get_attachments(&pool2, db_id)
                                    .await
                                    .unwrap_or_default();
                                let att_data: Vec<(i64, String, String, Option<u64>)> = attachments
                                    .iter()
                                    .map(|a| (
                                        a.id,
                                        a.filename.clone().unwrap_or_default(),
                                        a.mime_type.clone(),
                                        a.size.map(|s| s as u64),
                                    ))
                                    .collect();
                                (body, allowed, blocked, tracking, is_html, att_data)
                            },
                            move |(body, sender_allowed, blocked, tracking, _is_html, att_data): (
                                Option<BodyResult>,
                                bool,
                                usize,
                                usize,
                                bool,
                                Vec<(i64, String, String, Option<u64>)>,
                            )| {
                                if gen_win3.body_load_generation() != my_gen { return; }
                                if let Some(body) = body {
                                    if let Some(ref html) = body.html {
                                        v.set_body_html(html);
                                    } else {
                                        v.set_body_text(&body.text);
                                    }
                                }
                                if blocked > 0 && !sender_allowed {
                                    v.show_images_banner(blocked);
                                } else {
                                    v.hide_images_banner();
                                }
                                if tracking > 0 {
                                    v.show_tracking_info(tracking);
                                } else {
                                    v.hide_tracking_info();
                                }
                                if !att_data.is_empty() {
                                    if let Some(win) = v.root().and_then(|r| r.downcast::<MqWindow>().ok()) {
                                        let cb = make_attachment_download_callback(pool_dl3.clone(), db_id, win);
                                        v.set_attachments(&att_data, cb);
                                    }
                                } else {
                                    v.hide_attachments();
                                }
                            },
                        );
                    }
                }
            }
        },
    );
}

/// Persist a single email from sync to the database.
/// Returns the DB message ID on success, or `None` if the upsert failed.
async fn persist_email_to_db(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
    email_msg: &mq_core::email::Email,
    new_state: &mq_core::imap::sync::SyncState,
) -> Option<i64> {
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

    match mq_db::queries::messages::upsert_message(
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
        &email_msg.date.as_deref().map(mq_core::email::normalize_date).unwrap_or_default(),
        &flags,
        email_msg.has_attachments,
        None,
        email_msg.list_unsubscribe.as_deref(),
        email_msg.list_unsubscribe_post.as_deref(),
        None,
        new_state.uid_validity as i64,
    )
    .await
    {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!("Failed to persist email: {e}");
            None
        }
    }
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
    let pool_for_counts = pool.clone();
    let pool = pool.clone();
    let pool2 = pool.clone();
    let emails = account_emails.clone();
    let msgs = shared_messages.clone();
    let msgs2 = shared_messages.clone();
    let ml = window.message_list();
    let mv = window.message_view();
    let my_gen = window.body_load_generation();
    let gen_win = window.clone();

    // Set context-aware placeholder text for this mailbox
    set_mailbox_placeholder(&ml, &mailbox);

    let account_id_filter = account_id;
    runtime::spawn_async(
        async move {
            // Use threaded query to group by gmail_thread_id.
            // Filter by account_id if a specific account is selected.
            let threads = match account_id_filter {
                Some(aid) => {
                    mq_db::queries::messages::get_threads_for_account_mailbox(
                        &pool, aid, &mailbox, 200, 0,
                    )
                    .await
                    .unwrap_or_default()
                }
                None => {
                    mq_db::queries::messages::get_threads_for_mailbox(
                        &pool, &mailbox, 200, 0,
                    )
                    .await
                    .unwrap_or_default()
                }
            };
            (db_to_threaded_message_data(threads), emails, show_badge)
        },
        move |(messages, emails, show_badge): (
            Vec<MessageData>,
            Arc<HashMap<i64, String>>,
            bool,
        )| {
            let mut messages = messages;
            // Apply persisted sort order
            if !gen_win.imp().sort_newest_first.get() {
                messages.sort_by(|a, b| a.date.cmp(&b.date));
            }
            let objects = make_message_objects(&messages, &emails, show_badge);
            let has_messages = !objects.is_empty();
            // Update shared state so the selection handler has current data
            *msgs.borrow_mut() = messages;
            ml.set_messages(objects);

            if !has_messages {
                mv.show_placeholder();
                return;
            }

            // GTK SingleSelection may suppress selection-changed when it
            // auto-selects position 0. Directly load the first message so
            // switching views always updates the message pane.
            if let Some(item) = ml.model().item(0) {
                if let Ok(msg) = item.downcast::<MessageObject>() {
                    let from = if msg.sender_name().is_empty() {
                        msg.sender_email()
                    } else {
                        format!("{} <{}>", msg.sender_name(), msg.sender_email())
                    };
                    let db_id = msg.db_id();
                    let to = {
                        let m = msgs2.borrow();
                        m.iter()
                            .find(|m| m.db_id == db_id)
                            .map(|m| m.recipient_to.clone())
                            .unwrap_or_default()
                    };
                    let has_unsub = {
                        let m = msgs2.borrow();
                        m.iter()
                            .find(|m| m.db_id == db_id)
                            .map(|m| m.list_unsubscribe.is_some())
                            .unwrap_or(false)
                    };
                    mv.show_message(
                        &from,
                        &to,
                        &msg.date(),
                        &msg.subject(),
                        &msg.snippet(),
                        has_unsub,
                        msg.is_flagged(),
                        msg.is_read(),
                        db_id,
                    );
                    let v = mv.clone();
                    let pool_dl4 = pool2.clone();
                    let sender = msg.sender_email();
                    let account = msg.account_id();
                    let thread_id = msg.gmail_thread_id();
                    let thread_count = msg.thread_count();
                    let gen_win4 = gen_win.clone();
                    runtime::spawn_async(
                        async move {
                            let allowed =
                                mq_db::queries::sender_allowlist::is_allowed(
                                    &pool2, account, &sender,
                                )
                                .await
                                .unwrap_or(false);

                            let attachments = mq_db::queries::attachments::get_attachments(&pool2, db_id)
                                .await
                                .unwrap_or_default();
                            let att_data: Vec<(i64, String, String, Option<u64>)> = attachments
                                .iter()
                                .map(|a| (
                                    a.id,
                                    a.filename.clone().unwrap_or_default(),
                                    a.mime_type.clone(),
                                    a.size.map(|s| s as u64),
                                ))
                                .collect();

                            if thread_count > 1 && thread_id != 0 {
                                let thread_msgs = mq_db::queries::messages::get_thread_messages(
                                    &pool2, thread_id,
                                )
                                .await
                                .unwrap_or_default();

                                let mut conversation: Vec<(String, String, String, String, bool)> = Vec::new();
                                let mut total_blocked = 0usize;
                                let mut total_tracking = 0usize;

                                for tmsg in &thread_msgs {
                                    let from_display = tmsg
                                        .sender_name
                                        .as_deref()
                                        .filter(|n| !n.is_empty())
                                        .map(|n| format!("{n} <{}>", tmsg.sender_email))
                                        .unwrap_or_else(|| tmsg.sender_email.clone());
                                    let is_read = tmsg.flags.contains("\\Seen");

                                    let body = load_message_body(&pool2, tmsg.id).await;
                                    let body_html = body
                                        .as_ref()
                                        .and_then(|b| b.html.clone())
                                        .unwrap_or_default();
                                    let body_text = body
                                        .as_ref()
                                        .map(|b| b.text.clone())
                                        .unwrap_or_else(|| {
                                            tmsg.snippet.clone().unwrap_or_default()
                                        });
                                    if let Some(ref b) = body {
                                        total_blocked += b.blocked_images;
                                        total_tracking += b.tracking_pixels;
                                    }

                                    conversation.push((from_display, tmsg.date.clone(), body_html, body_text, is_read));
                                }

                                (None, Some(conversation), allowed, total_blocked, total_tracking, att_data)
                            } else {
                                let body = load_message_body(&pool2, db_id).await;
                                let blocked = body.as_ref().map(|b| b.blocked_images).unwrap_or(0);
                                let tracking = body.as_ref().map(|b| b.tracking_pixels).unwrap_or(0);
                                (body, None, allowed, blocked, tracking, att_data)
                            }
                        },
                        move |(body, conversation, sender_allowed, blocked, tracking, att_data): (
                            Option<BodyResult>,
                            Option<Vec<(String, String, String, String, bool)>>,
                            bool,
                            usize,
                            usize,
                            Vec<(i64, String, String, Option<u64>)>,
                        )| {
                            if gen_win4.body_load_generation() != my_gen { return; }
                            if let Some(ref conv) = conversation {
                                v.set_conversation(conv);
                            } else if let Some(body) = body {
                                if let Some(ref html) = body.html {
                                    v.set_body_html(html);
                                } else {
                                    v.set_body_text(&body.text);
                                }
                            }
                            if blocked > 0 && !sender_allowed {
                                v.show_images_banner(blocked);
                            } else {
                                v.hide_images_banner();
                            }
                            if tracking > 0 {
                                v.show_tracking_info(tracking);
                            } else {
                                v.hide_tracking_info();
                            }
                            if !att_data.is_empty() {
                                if let Some(win) = v.root().and_then(|r| r.downcast::<MqWindow>().ok()) {
                                    let cb = make_attachment_download_callback(pool_dl4.clone(), db_id, win);
                                    v.set_attachments(&att_data, cb);
                                }
                            } else {
                                v.hide_attachments();
                            }
                        },
                    );
                }
            }
        },
    );

    // Refresh unread count badges in the sidebar
    refresh_unread_counts(window, &pool_for_counts, account_id);
}

/// Fetch unread counts from the DB and update the sidebar badges.
fn refresh_unread_counts(window: &MqWindow, pool: &Arc<SqlitePool>, account_id: Option<i64>) {
    let pool = pool.clone();
    let sidebar = window.sidebar();
    runtime::spawn_async(
        async move {
            mq_db::queries::messages::get_unread_counts(&pool, account_id)
                .await
                .unwrap_or_default()
        },
        move |counts: std::collections::HashMap<String, i64>| {
            sidebar.update_unread_counts(&counts);
        },
    );
}

fn db_to_message_data(messages: Vec<mq_db::models::DbMessage>) -> Vec<MessageData> {
    let mut data: Vec<MessageData> = messages
        .into_iter()
        .map(|m| MessageData {
            db_id: m.id,
            uid: m.uid as u32,
            sender_name: m.sender_name.unwrap_or_default(),
            sender_email: m.sender_email,
            subject: m.subject.unwrap_or_default(),
            date: mq_core::email::normalize_date(&m.date),
            snippet: m.snippet.unwrap_or_default(),
            is_read: m.flags.contains("\\Seen"),
            is_flagged: m.flags.contains("\\Flagged"),
            has_attachments: m.has_attachments,
            mailbox: m.mailbox,
            account_id: m.account_id,
            recipient_to: m.recipient_to,
            recipient_cc: m.recipient_cc,
            list_unsubscribe: m.list_unsubscribe,
            list_unsubscribe_post: m.list_unsubscribe_post,
            gmail_thread_id: m.gmail_thread_id,
            thread_count: 1,
        })
        .collect();
    data.sort_by(|a, b| b.date.cmp(&a.date));
    data
}

fn db_to_threaded_message_data(
    messages: Vec<(mq_db::models::DbMessage, i64)>,
) -> Vec<MessageData> {
    let mut data: Vec<MessageData> = messages
        .into_iter()
        .map(|(m, count)| MessageData {
            db_id: m.id,
            uid: m.uid as u32,
            sender_name: m.sender_name.unwrap_or_default(),
            sender_email: m.sender_email,
            subject: m.subject.unwrap_or_default(),
            // Normalize date on read so old RFC 2822 entries sort correctly
            date: mq_core::email::normalize_date(&m.date),
            snippet: m.snippet.unwrap_or_default(),
            is_read: m.flags.contains("\\Seen"),
            is_flagged: m.flags.contains("\\Flagged"),
            has_attachments: m.has_attachments,
            mailbox: m.mailbox,
            account_id: m.account_id,
            recipient_to: m.recipient_to,
            recipient_cc: m.recipient_cc,
            list_unsubscribe: m.list_unsubscribe,
            list_unsubscribe_post: m.list_unsubscribe_post,
            gmail_thread_id: m.gmail_thread_id,
            thread_count: count,
        })
        .collect();
    // Sort by normalized date (newest first) since DB ORDER BY may be wrong
    // for un-normalized RFC 2822 dates
    data.sort_by(|a, b| b.date.cmp(&a.date));
    data
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
                m.gmail_thread_id.unwrap_or(0),
                m.thread_count,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Message selection & actions
// ---------------------------------------------------------------------------

fn wire_action_buttons(window: &MqWindow, pool: &Arc<SqlitePool>, shared_messages: &Rc<RefCell<Vec<MessageData>>>) {
    let pool2 = pool.clone();
    let ml2 = window.message_list();
    let star_win = window.clone();
    window
        .message_view()
        .connect_star_toggled(move |starred| {
            if let Some(msg) = selected_message(&ml2) {
                let db_id = msg.db_id();
                let uid = msg.uid();
                let account_id = msg.account_id();
                let mailbox = msg.mailbox();
                let pool = pool2.clone();
                msg.set_is_flagged(starred);
                debug!(db_id, starred, "Star toggled");
                let pool_imap = pool.clone();
                runtime::spawn_async(
                    async move { toggle_flag(&pool, db_id, "\\Flagged", starred).await },
                    |_| {},
                );
                let msg_revert = msg.clone();
                let w = star_win.clone();
                runtime::spawn_async(
                    async move {
                        imap_store_flag(
                            &pool_imap, account_id, &mailbox, uid, "\\Flagged", starred,
                        ).await
                    },
                    move |result: Result<(), Box<dyn std::error::Error + Send + Sync>>| {
                        if let Err(e) = result {
                            warn!("Failed to sync star flag to server: {e}");
                            msg_revert.set_is_flagged(!starred);
                            w.show_toast(&adw::Toast::new("Failed to sync \u{2014} will retry on next sync"));
                        }
                    },
                );
            }
        });

    let pool3 = pool.clone();
    let ml3 = window.message_list();
    let read_win = window.clone();
    window.message_view().connect_read_toggled(move |read| {
        if let Some(msg) = selected_message(&ml3) {
            let db_id = msg.db_id();
            let uid = msg.uid();
            let account_id = msg.account_id();
            let mailbox = msg.mailbox();
            let pool = pool3.clone();
            msg.set_is_read(read);
            debug!(db_id, read, "Read toggled");
            let pool_imap = pool.clone();
            runtime::spawn_async(
                async move { toggle_flag(&pool, db_id, "\\Seen", read).await },
                |_| {},
            );
            let msg_revert = msg.clone();
            let w = read_win.clone();
            runtime::spawn_async(
                async move {
                    imap_store_flag(
                        &pool_imap, account_id, &mailbox, uid, "\\Seen", read,
                    ).await
                },
                move |result: Result<(), Box<dyn std::error::Error + Send + Sync>>| {
                    if let Err(e) = result {
                        warn!("Failed to sync read flag to server: {e}");
                        msg_revert.set_is_read(!read);
                        w.show_toast(&adw::Toast::new("Failed to sync \u{2014} will retry on next sync"));
                    }
                },
            );
        }
    });

    // --- Delete with undo ---
    let pool4 = pool.clone();
    let ml4 = window.message_list();
    let view4 = window.message_view();
    let del_win = window.clone();
    let del_msgs = shared_messages.clone();
    // Shared pending-undo state: stores the timer source ID and an execute
    // callback so cancelling a previous undo runs its server operation immediately.
    struct PendingUndo {
        source_id: glib::SourceId,
        execute_now: Rc<dyn Fn()>,
    }
    let pending_undo: Rc<RefCell<Option<PendingUndo>>> = Rc::new(RefCell::new(None));
    let pending_undo_del = pending_undo.clone();
    window.message_view().connect_delete_clicked(move || {
        if let Some(msg) = selected_message(&ml4) {
            let db_id = msg.db_id();
            let uid = msg.uid();
            let account_id = msg.account_id();
            let mailbox = msg.mailbox();
            let pool = pool4.clone();
            debug!(db_id, "Delete clicked");

            // Execute any previous pending undo immediately before starting new one
            if let Some(prev) = pending_undo_del.borrow_mut().take() {
                prev.source_id.remove();
                (prev.execute_now)();
            }

            let pos = ml4.selection().selected();
            let has_remaining = ml4.model().n_items() > 1;

            // Remove from shared_messages so the selection handler stays consistent
            let removed_data: Option<MessageData> = {
                let mut msgs = del_msgs.borrow_mut();
                if let Some(idx) = msgs.iter().position(|m| m.db_id == db_id) {
                    Some(msgs.remove(idx))
                } else {
                    None
                }
            };

            remove_selected(&ml4);
            if !has_remaining {
                view4.show_placeholder();
            }

            // Clone the message for potential undo re-insertion
            let undo_msg = MessageObject::new(
                db_id, uid,
                &msg.sender_name(), &msg.sender_email(),
                &msg.subject(), &msg.date(), &msg.snippet(),
                msg.is_read(), msg.is_flagged(), msg.has_attachments(),
                &msg.mailbox(), msg.account_id(), &msg.account_email(),
                msg.gmail_thread_id(), msg.thread_count(),
            );

            // Set up the delayed server operation
            let execute = Rc::new(std::cell::Cell::new(true));
            let pending_ref = pending_undo_del.clone();

            // Build execute callback for immediate execution when superseded
            let exec_pool = pool.clone();
            let exec_sync_pool = pool.clone();
            let exec_win = del_win.clone();
            let exec_msgs = del_msgs.clone();
            let exec_execute = execute.clone();
            let exec_mailbox = mailbox.clone();
            let execute_now: Rc<dyn Fn()> = Rc::new(move || {
                if !exec_execute.get() {
                    return;
                }
                exec_execute.set(false);
                let pool = exec_pool.clone();
                let sync_pool = exec_sync_pool.clone();
                let sync_win = exec_win.clone();
                let sync_msgs = exec_msgs.clone();
                let mailbox = exec_mailbox.clone();
                runtime::spawn_async(
                    async move {
                        if let Err(e) = imap_trash_message(&pool, account_id, &mailbox, uid).await {
                            warn!("Failed to trash message on server: {e}");
                        }
                        if let Err(e) = mq_db::queries::messages::delete_message(&pool, db_id).await {
                            warn!("Failed to delete message locally: {e}");
                        }
                        account_id
                    },
                    move |account_id: i64| {
                        trigger_sync_account(account_id, &sync_pool, &sync_win, &sync_msgs);
                    },
                );
            });

            let timer_execute_now = execute_now.clone();
            let source_id = glib::timeout_add_local_once(
                std::time::Duration::from_secs(5),
                move || {
                    // Clear the pending undo reference
                    pending_ref.borrow_mut().take();
                    (timer_execute_now)();
                },
            );
            *pending_undo_del.borrow_mut() = Some(PendingUndo { source_id, execute_now });

            // Show undo toast
            let toast = adw::Toast::builder()
                .title("Message deleted")
                .button_label("Undo")
                .timeout(5)
                .build();
            let undo_ml = ml4.clone();
            let undo_execute = execute;
            let undo_msgs = del_msgs.clone();
            toast.connect_button_clicked(move |_| {
                undo_execute.set(false);
                // Restore shared_messages data so the selection handler works
                if let Some(ref data) = removed_data {
                    let mut msgs = undo_msgs.borrow_mut();
                    let insert_idx = pos.min(msgs.len() as u32) as usize;
                    msgs.insert(insert_idx, data.clone());
                }
                undo_ml.insert_message_at(pos, &undo_msg);
                undo_ml.selection().set_selected(pos);
            });
            del_win.show_toast(&toast);
        }
    });

    // --- Archive with undo ---
    let pool5 = pool.clone();
    let ml5 = window.message_list();
    let view5 = window.message_view();
    let arch_win = window.clone();
    let arch_msgs = shared_messages.clone();
    let pending_undo_arch = pending_undo;
    window.message_view().connect_archive_clicked(move || {
        if let Some(msg) = selected_message(&ml5) {
            let db_id = msg.db_id();
            let uid = msg.uid();
            let account_id = msg.account_id();
            let mailbox = msg.mailbox();
            let pool = pool5.clone();
            debug!(db_id, "Archive clicked");

            // Execute any previous pending undo immediately before starting new one
            if let Some(prev) = pending_undo_arch.borrow_mut().take() {
                prev.source_id.remove();
                (prev.execute_now)();
            }

            let pos = ml5.selection().selected();
            let has_remaining = ml5.model().n_items() > 1;

            // Remove from shared_messages so the selection handler stays consistent
            let removed_data: Option<MessageData> = {
                let mut msgs = arch_msgs.borrow_mut();
                if let Some(idx) = msgs.iter().position(|m| m.db_id == db_id) {
                    Some(msgs.remove(idx))
                } else {
                    None
                }
            };

            remove_selected(&ml5);
            if !has_remaining {
                view5.show_placeholder();
            }

            let undo_msg = MessageObject::new(
                db_id, uid,
                &msg.sender_name(), &msg.sender_email(),
                &msg.subject(), &msg.date(), &msg.snippet(),
                msg.is_read(), msg.is_flagged(), msg.has_attachments(),
                &msg.mailbox(), msg.account_id(), &msg.account_email(),
                msg.gmail_thread_id(), msg.thread_count(),
            );

            let execute = Rc::new(std::cell::Cell::new(true));
            let pending_ref = pending_undo_arch.clone();

            // Build execute callback for immediate execution when superseded
            let exec_pool = pool.clone();
            let exec_sync_pool = pool.clone();
            let exec_win = arch_win.clone();
            let exec_msgs = arch_msgs.clone();
            let exec_execute = execute.clone();
            let exec_mailbox = mailbox.clone();
            let execute_now: Rc<dyn Fn()> = Rc::new(move || {
                if !exec_execute.get() {
                    return;
                }
                exec_execute.set(false);
                let pool = exec_pool.clone();
                let sync_pool = exec_sync_pool.clone();
                let sync_win = exec_win.clone();
                let sync_msgs = exec_msgs.clone();
                let mailbox = exec_mailbox.clone();
                runtime::spawn_async(
                    async move {
                        if let Err(e) = imap_archive_message(&pool, account_id, &mailbox, uid).await {
                            warn!("Failed to archive message on server: {e}");
                        }
                        if let Err(e) = mq_db::queries::messages::delete_message(&pool, db_id).await {
                            warn!("Failed to delete archived message locally: {e}");
                        }
                        account_id
                    },
                    move |account_id: i64| {
                        trigger_sync_account(account_id, &sync_pool, &sync_win, &sync_msgs);
                    },
                );
            });

            let timer_execute_now = execute_now.clone();
            let source_id = glib::timeout_add_local_once(
                std::time::Duration::from_secs(5),
                move || {
                    pending_ref.borrow_mut().take();
                    (timer_execute_now)();
                },
            );
            *pending_undo_arch.borrow_mut() = Some(PendingUndo { source_id, execute_now });

            let toast = adw::Toast::builder()
                .title("Message archived")
                .button_label("Undo")
                .timeout(5)
                .build();
            let undo_ml = ml5.clone();
            let undo_execute = execute;
            let undo_msgs = arch_msgs.clone();
            toast.connect_button_clicked(move |_| {
                undo_execute.set(false);
                // Restore shared_messages data so the selection handler works
                if let Some(ref data) = removed_data {
                    let mut msgs = undo_msgs.borrow_mut();
                    let insert_idx = pos.min(msgs.len() as u32) as usize;
                    msgs.insert(insert_idx, data.clone());
                }
                undo_ml.insert_message_at(pos, &undo_msg);
                undo_ml.selection().set_selected(pos);
            });
            arch_win.show_toast(&toast);
        }
    });

    // --- Thread message expansion: mark as read ---
    let pool_expand = pool.clone();
    let view_expand = window.message_view();
    window.message_view().connect_thread_message_expanded({
        let view = view_expand.clone();
        let pool = pool_expand.clone();
        move |idx| {
            if let Some((db_id, uid, account_id, mailbox, is_read)) = view.thread_message_meta_at(idx) {
                if is_read {
                    return; // Already read
                }
                // Mark read in our metadata
                view.mark_thread_meta_read(idx);
                // Update CSS immediately (no page reload)
                view.mark_thread_card_read(idx);
                // Update local DB
                let pool = pool.clone();
                let mailbox = mailbox.clone();
                runtime::spawn_async(
                    async move {
                        toggle_flag(&pool, db_id, "\\Seen", true).await;
                        if let Err(e) = imap_store_flag(&pool, account_id, &mailbox, uid, "\\Seen", true).await {
                            tracing::warn!(db_id, "Failed to mark expanded thread msg as read: {e}");
                        }
                    },
                    |_: ()| {},
                );
            }
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
        let pool = pool.clone();
        window.message_list().connect_compose_clicked(move || {
            open_compose(&w, &accts, ComposeMode::New, None, &pool);
        });
    }

    // Reply
    {
        let w = window.clone();
        let ml = window.message_list();
        let accts = account_tuples.clone();
        let pool = pool.clone();
        let sm = shared_messages.clone();
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
                let pool2 = pool.clone();
                let sm = sm.clone();
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
                        open_compose_with_refresh(&w, &accts, mode, Some(account_id), &pool2, sm);
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
        let sm2 = shared_messages.clone();
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
                let cc_str = msg_data.and_then(|m| m.recipient_cc.clone()).unwrap_or_default();
                let cc_addrs: Vec<String> = cc_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let account_id = msg.account_id();
                let accts = accts.clone();
                let w = w.clone();
                let pool = pool.clone();
                let pool2 = pool.clone();
                let sm = sm2.clone();
                runtime::spawn_async(
                    async move { load_message_body_text(&pool, db_id).await },
                    move |body: Option<String>| {
                        let body_text = body.unwrap_or_default();
                        let mode = ComposeMode::ReplyAll {
                            from: from.clone(),
                            to: to_addrs,
                            cc: cc_addrs,
                            subject: subject.clone(),
                            date: date.clone(),
                            body: body_text,
                            message_id: None,
                            references: None,
                        };
                        open_compose_with_refresh(&w, &accts, mode, Some(account_id), &pool2, sm);
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
        let sm3 = shared_messages.clone();
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
                let pool2 = pool.clone();
                let sm = sm3.clone();
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
                        open_compose_with_refresh(&w, &accts, mode, Some(account_id), &pool2, sm);
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
    pool: &Arc<SqlitePool>,
) {
    open_compose_impl(window, accounts, mode, selected_account_id, pool, None);
}

fn open_compose_with_refresh(
    window: &MqWindow,
    accounts: &[(i64, String)],
    mode: ComposeMode,
    selected_account_id: Option<i64>,
    pool: &Arc<SqlitePool>,
    shared_messages: Rc<RefCell<Vec<MessageData>>>,
) {
    open_compose_impl(window, accounts, mode, selected_account_id, pool, Some(shared_messages));
}

fn open_compose_impl(
    window: &MqWindow,
    accounts: &[(i64, String)],
    mode: ComposeMode,
    selected_account_id: Option<i64>,
    pool: &Arc<SqlitePool>,
    shared_messages: Option<Rc<RefCell<Vec<MessageData>>>>,
) {
    let config = mq_core::config::AppConfig::load().unwrap_or_default();
    let signature = config.compose.default_signature;

    let compose = MqComposeWindow::new(window);
    compose.set_accounts(accounts);
    if let Some(aid) = selected_account_id {
        compose.select_account(aid);
    }

    // Load contacts for autocomplete
    let compose_for_contacts = compose.clone();
    let pool_for_contacts = pool.clone();
    let account_ids: Vec<i64> = accounts.iter().map(|(id, _)| *id).collect();
    runtime::spawn_async(
        async move {
            let mut all_contacts = Vec::new();
            for account_id in &account_ids {
                if let Ok(contacts) = mq_db::queries::contacts::get_all_for_account(
                    &pool_for_contacts, *account_id,
                ).await {
                    for c in contacts {
                        all_contacts.push(ContactEntry {
                            display_name: c.display_name,
                            email: c.email,
                        });
                    }
                }
            }
            all_contacts
        },
        move |contacts: Vec<ContactEntry>| {
            compose_for_contacts.set_contacts(contacts);
        },
    );

    compose.apply_mode(&mode, &signature, config.compose.reply_position);

    // Wire "Save Draft" callback for close dialog
    {
        let draft_pool = pool.clone();
        compose.set_save_draft_callback(move |compose_win| {
            let account = compose_win.selected_account();
            let (account_id, _) = match account {
                Some(a) => a,
                None => return,
            };
            let to = compose_win.to_addresses().join(", ");
            let cc = compose_win.cc_addresses().join(", ");
            let bcc = compose_win.bcc_addresses().join(", ");
            let subject = compose_win.subject();
            let body = compose_win.body_text();
            let body_html = compose_win.body_html();
            let draft_id = compose_win.draft_id();
            let pool = draft_pool.clone();
            runtime::spawn_async(
                async move {
                    let _ = mq_db::queries::drafts::upsert_draft(
                        &pool, draft_id, account_id,
                        &to, &cc, &bcc, &subject, &body, &body_html,
                        "new", None,
                    ).await;
                },
                |_| {},
            );
        });
    }

    let (in_reply_to, references) = MqComposeWindow::reply_headers(&mode);

    // Wire Send button
    let compose_ref = compose.clone();
    // Capture refresh context (available when opened with open_compose_with_refresh)
    let refresh_window: Option<MqWindow> = shared_messages.as_ref().map(|_| window.clone());
    let refresh_pool_ref: Option<Arc<SqlitePool>> = shared_messages.as_ref().map(|_| pool.clone());
    let refresh_shared: Option<Rc<RefCell<Vec<MessageData>>>> = shared_messages.clone();
    let send_pool = pool.clone();
    let send_window = window.clone();
    compose.connect_send(move || {
        let Some((send_account_id, from_email)) = compose_ref.selected_account() else {
            warn!("No account selected for sending");
            return;
        };

        let to = compose_ref.to_addresses();
        if to.is_empty() {
            let toast = adw::Toast::new("Please add at least one recipient");
            send_window.show_toast(&toast);
            return;
        }

        // Validate email addresses
        let validation_errors = compose_ref.validate_addresses();
        if !validation_errors.is_empty() {
            let toast = adw::Toast::new(&validation_errors[0]);
            send_window.show_toast(&toast);
            return;
        }

        // Read attachment files (validate they still exist)
        let raw_attachments = compose_ref.attachments();
        let mut attachments: Vec<(String, String, Vec<u8>)> = Vec::new();
        for (filename, path) in &raw_attachments {
            match std::fs::read(path) {
                Ok(data) => {
                    let mime = mime_guess::from_path(path)
                        .first_or_octet_stream()
                        .to_string();
                    attachments.push((filename.clone(), mime, data));
                }
                Err(_) => {
                    let toast = adw::Toast::new(&format!("Attachment not found: {filename}"));
                    send_window.show_toast(&toast);
                    return;
                }
            }
        }

        let email = mq_core::smtp::OutgoingEmail {
            from_email: from_email.clone(),
            from_name: None,
            to,
            cc: compose_ref.cc_addresses(),
            bcc: compose_ref.bcc_addresses(),
            subject: compose_ref.subject(),
            body_text: compose_ref.body_text(),
            body_html: {
                let html = compose_ref.body_html();
                if html.is_empty() { None } else { Some(html) }
            },
            in_reply_to: in_reply_to.clone(),
            references: references.clone(),
            attachments,
        };

        info!(to = ?email.to, subject = %email.subject, "Sending email");

        // Enter sending state — grey out everything, show progress
        compose_ref.set_sending(true);

        let compose_close = compose_ref.clone();
        let refresh_win = refresh_window.clone();
        let refresh_pool = refresh_pool_ref.clone();
        let refresh_msgs = refresh_shared.clone();
        let queue_pool = send_pool.clone();
        let toast_win = send_window.clone();
        // "queued" signals the callback that the email was queued offline (not sent immediately)
        runtime::spawn_async(
            async move {
                // Get access token from keyring
                let access_token = match mq_core::keyring::get_tokens(&from_email).await {
                    Ok(Some(tokens)) => tokens.access_token,
                    Ok(None) => {
                        // No stored token — try a refresh in case we have a refresh token
                        mq_core::keyring::refresh_and_store(&from_email)
                            .await
                            .map_err(|e| format!("No access token for {from_email}: {e}"))?
                    }
                    Err(e) => return Err(format!("Failed to read keyring: {e}")),
                };

                // Try sending with the current access token
                match mq_core::smtp::send_email(&email, &access_token).await {
                    Ok(()) => Ok("sent".to_string()),
                    Err(e) if e.is_auth_failure() => {
                        // Token may be expired — refresh and retry once
                        info!("Auth failure, refreshing token and retrying");
                        let new_token = mq_core::keyring::refresh_and_store(&from_email)
                            .await
                            .map_err(|e| format!("Token refresh failed: {e}"))?;
                        mq_core::smtp::send_email(&email, &new_token)
                            .await
                            .map(|()| "sent".to_string())
                            .map_err(|e| format!("Send failed after token refresh: {e}"))
                    }
                    Err(e) if e.is_retryable() => {
                        // Network/transient error — queue for offline retry
                        info!("Send failed with retryable error, queuing for offline retry");
                        let attachments_json = serde_json::to_string(&email.attachments
                            .iter()
                            .map(|(name, mime, data)| {
                                use base64::Engine;
                                (name.clone(), mime.clone(), base64::engine::general_purpose::STANDARD.encode(data))
                            })
                            .collect::<Vec<(String, String, String)>>(),
                        ).unwrap_or_default();
                        let queue = mq_net::queue::OfflineQueue::new(queue_pool);
                        let op = mq_net::queue::OfflineOp::SendEmail {
                            from_email: email.from_email.clone(),
                            to: email.to.clone(),
                            cc: email.cc.clone(),
                            bcc: email.bcc.clone(),
                            subject: email.subject.clone(),
                            body_text: email.body_text.clone(),
                            in_reply_to: email.in_reply_to.clone(),
                            references: email.references.clone(),
                            attachments_json,
                        };
                        match queue.enqueue(send_account_id, op).await {
                            Ok(_) => Ok("queued".to_string()),
                            Err(queue_err) => Err(format!("Failed to queue email: {queue_err}")),
                        }
                    }
                    Err(e) => Err(format!("Failed to send email: {e}")),
                }
            },
            move |result: Result<String, String>| match result {
                Ok(status) => {
                    compose_close.set_sent_successfully();
                    compose_close.close();
                    if status == "queued" {
                        info!("Email queued for offline send");
                        let toast = adw::Toast::new("Email queued \u{2014} will send when online");
                        toast_win.show_toast(&toast);
                    } else {
                        info!("Email sent successfully, closing compose window");
                        // Refresh the message list + thread view to show the sent reply.
                        // Delay the sync slightly to give Gmail time to process the sent message.
                        if let (Some(win), Some(pool), Some(msgs)) =
                            (refresh_win.as_ref(), refresh_pool.as_ref(), refresh_msgs.as_ref())
                        {
                            refresh_and_reload_selected(win, pool, msgs);
                            let sync_win = win.clone();
                            let sync_pool = pool.clone();
                            let sync_msgs = msgs.clone();
                            glib::timeout_add_seconds_local_once(2, move || {
                                trigger_sync_account(send_account_id, &sync_pool, &sync_win, &sync_msgs);
                            });
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to send: {e}");
                    // Restore compose window so user can retry
                    compose_close.set_sending(false);
                    let toast = adw::Toast::new(&format!("Send failed: {e}"));
                    toast_win.show_toast(&toast);
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

                // Show confirmation dialog before unsubscribing
                let dialog = adw::AlertDialog::builder()
                    .heading("Unsubscribe?")
                    .body("This will send an unsubscribe request and move the message to trash.")
                    .close_response("cancel")
                    .default_response("cancel")
                    .build();
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("unsub", "Unsubscribe");
                dialog.set_response_appearance("unsub", adw::ResponseAppearance::Destructive);

                let msgs_ref = msgs.clone();
                let pool_ref = pool.clone();
                let ml_ref = ml.clone();
                let w_ref = w.clone();
                dialog.connect_response(None, move |_, response| {
                    if response != "unsub" {
                        return;
                    }

                    let msgs = msgs_ref.borrow();
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
                                    let pool = pool_ref.clone();
                                    let ml = ml_ref.clone();
                                    let w = w_ref.clone();
                                    runtime::spawn_async(
                                        async move {
                                            mq_core::privacy::unsubscribe::one_click_unsubscribe(&url)
                                                .await
                                        },
                                        move |result| match result {
                                            Ok(()) => {
                                                info!("Unsubscribe successful");
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
                });
                dialog.present(Some(&w.clone().upcast::<gtk::Window>()));
            }
        });
    }

    // "Load images" button — reload body without blocking
    {
        let ml = window.message_list();
        let pool = pool.clone();
        let v = view.clone();
        view.connect_load_images(move || {
            v.set_images_force_loaded();
            if let Some(msg) = selected_message(&ml) {
                let db_id = msg.db_id();
                let thread_id = msg.gmail_thread_id();
                let thread_count = msg.thread_count();
                let pool = pool.clone();
                let v = v.clone();

                if thread_count > 1 && thread_id != 0 {
                    // Thread view — reload entire conversation unblocked
                    runtime::spawn_async(
                        async move {
                            load_conversation_unblocked(&pool, thread_id).await
                        },
                        move |conversation: Vec<(String, String, String, String, bool)>| {
                            if !conversation.is_empty() {
                                v.set_conversation(&conversation);
                            }
                            v.hide_images_banner();
                        },
                    );
                } else {
                    // Single message
                    runtime::spawn_async(
                        async move { load_message_body_unblocked(&pool, db_id).await },
                        move |body: Option<BodyResult>| {
                            if let Some(body) = body {
                                if let Some(ref html) = body.html {
                                    v.set_body_html(html);
                                } else {
                                    v.set_body_text(&body.text);
                                }
                                v.hide_images_banner();
                            }
                        },
                    );
                }
            }
        });
    }

    // "Always load from this sender" button — add to allowlist + reload
    {
        let ml = window.message_list();
        let pool = pool.clone();
        let v = view.clone();
        view.connect_always_load_images(move || {
            v.set_images_force_loaded();
            if let Some(msg) = selected_message(&ml) {
                let db_id = msg.db_id();
                let sender_email = msg.sender_email();
                let account_id = msg.account_id();
                let thread_id = msg.gmail_thread_id();
                let thread_count = msg.thread_count();
                let pool = pool.clone();
                let v = v.clone();

                if thread_count > 1 && thread_id != 0 {
                    runtime::spawn_async(
                        async move {
                            if let Err(e) = mq_db::queries::sender_allowlist::add_sender(
                                &pool, account_id, &sender_email,
                            ).await {
                                warn!("Failed to add sender to allowlist: {e}");
                            }
                            load_conversation_unblocked(&pool, thread_id).await
                        },
                        move |conversation: Vec<(String, String, String, String, bool)>| {
                            if !conversation.is_empty() {
                                v.set_conversation(&conversation);
                            }
                            v.hide_images_banner();
                        },
                    );
                } else {
                    runtime::spawn_async(
                        async move {
                            if let Err(e) = mq_db::queries::sender_allowlist::add_sender(
                                &pool, account_id, &sender_email,
                            ).await {
                                warn!("Failed to add sender to allowlist: {e}");
                            }
                            load_message_body_unblocked(&pool, db_id).await
                        },
                        move |body: Option<BodyResult>| {
                            if let Some(body) = body {
                                if let Some(ref html) = body.html {
                                    v.set_body_html(html);
                                } else {
                                    v.set_body_text(&body.text);
                                }
                                v.hide_images_banner();
                            }
                        },
                    );
                }
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
                    let is_empty = objects.is_empty();
                    *msgs.borrow_mut() = messages;
                    if is_empty {
                        ml_inner.set_placeholder_text("No results found", "Try a different search term");
                    }
                    ml_inner.set_messages(objects);
                    ml_inner.set_mailbox_title("Search Results");
                    if is_empty {
                        mv.show_placeholder();
                    }
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

/// Send individual desktop notifications for new inbox messages.
///
/// Each notification shows the sender and subject, with a unique ID
/// so they can be individually dismissed.
///
/// Note: GNOME Shell only shows "Open" (default action) and "Dismiss"
/// for GNotification. Custom action buttons require KDE or other DEs.
fn send_per_message_notifications(app: &adw::Application, messages: &[NewMailInfo]) {
    // Limit to 5 notifications to avoid flooding
    let max_notifications = 5;
    for (i, msg) in messages.iter().take(max_notifications).enumerate() {
        let notification = gio::Notification::new(&msg.sender);
        let body = if msg.snippet.is_empty() {
            msg.subject.clone()
        } else {
            format!("{}\n{}", msg.subject, msg.snippet)
        };
        notification.set_body(Some(&body));
        notification.set_default_action("app.activate");

        // Unique notification ID per message
        let notif_id = format!("new-mail-{i}");
        app.send_notification(Some(&notif_id), &notification);
    }

    // If there are more than max, show a summary for the rest
    if messages.len() > max_notifications {
        let remaining = messages.len() - max_notifications;
        let notification = gio::Notification::new("New mail");
        notification.set_body(Some(&format!("…and {remaining} more new message{}", if remaining == 1 { "" } else { "s" })));
        app.send_notification(Some("new-mail-overflow"), &notification);
    }
}

/// Set context-aware placeholder text for an empty mailbox.
fn set_mailbox_placeholder(ml: &crate::widgets::message_list::MqMessageList, mailbox: &str) {
    match mailbox {
        "[Gmail]/Drafts" => ml.set_placeholder_text("No drafts", "Press Ctrl+N to compose a new message"),
        "[Gmail]/Trash" => ml.set_placeholder_text("No deleted messages", "Messages you delete will appear here"),
        "[Gmail]/Starred" => ml.set_placeholder_text("No starred messages", "Press S to star important messages"),
        "[Gmail]/Spam" => ml.set_placeholder_text("No spam", "Messages marked as spam will appear here"),
        "[Gmail]/Sent Mail" => ml.set_placeholder_text("No sent messages", "Messages you send will appear here"),
        _ => ml.reset_placeholder(),
    }
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
    html: Option<String>,
    blocked_images: usize,
    tracking_pixels: usize,
    /// Whether the original email had an HTML body (used to decide
    /// whether to show the "load images" banner even when blocked_images == 0,
    /// since CSS backgrounds and other remote resources aren't counted).
    is_html: bool,
}

async fn load_message_body(pool: &SqlitePool, message_id: i64) -> Option<BodyResult> {
    let config = mq_core::config::AppConfig::load().unwrap_or_default();
    match mq_db::queries::message_bodies::get_body(pool, message_id).await {
        Ok(Some(body)) => {
            let is_html = body.html_body.is_some();
            // Sanitize HTML: block remote images, detect tracking pixels
            let (sanitized_html, blocked_images, tracking_pixels) =
                if let Some(ref html) = body.html_body {
                    // Resolve CID (inline) images before sanitization
                    let with_cid = if let Some(ref raw) = body.raw_mime {
                        mq_core::body::resolve_cid_images(html, raw)
                    } else {
                        html.clone()
                    };
                    let sanitized = mq_core::privacy::images::sanitize_html(
                        &with_cid,
                        config.privacy.block_remote_images,
                        config.privacy.detect_tracking_pixels,
                    );
                    (
                        Some(sanitized.html),
                        sanitized.blocked_image_count,
                        sanitized.tracking_pixel_count,
                    )
                } else {
                    (None, 0, 0)
                };

            // Plain text fallback (used for snippets, compose, etc.)
            let text = body
                .text_body
                .or_else(|| {
                    sanitized_html
                        .as_ref()
                        .map(|h| mq_core::privacy::images::html_to_plain_text(h))
                })
                .unwrap_or_default();

            Some(BodyResult {
                text,
                html: sanitized_html,
                blocked_images,
                tracking_pixels,
                is_html,
            })
        }
        Ok(None) => None,
        Err(e) => {
            warn!("Failed to load message body: {e}");
            None
        }
    }
}

/// Load message body without image blocking (user clicked "Load images").
/// Downloads remote images and inlines them as data: URIs so that WebKitGTK's
/// `load_html()` can display them without needing network access.
async fn load_message_body_unblocked(pool: &SqlitePool, message_id: i64) -> Option<BodyResult> {
    let config = mq_core::config::AppConfig::load().unwrap_or_default();
    match mq_db::queries::message_bodies::get_body(pool, message_id).await {
        Ok(Some(body)) => {
            let (sanitized_html, tracking_pixels) = if let Some(ref html) = body.html_body {
                // Resolve CID (inline) images first
                let with_cid = if let Some(ref raw) = body.raw_mime {
                    mq_core::body::resolve_cid_images(html, raw)
                } else {
                    html.clone()
                };
                let sanitized = mq_core::privacy::images::sanitize_html(
                    &with_cid,
                    false, // don't block
                    config.privacy.detect_tracking_pixels,
                );
                // Download remote images and inline as data: URIs
                let inlined = inline_remote_images(&sanitized.html).await;
                (Some(inlined), sanitized.tracking_pixel_count)
            } else {
                (None, 0)
            };

            let text = body
                .text_body
                .or_else(|| {
                    sanitized_html
                        .as_ref()
                        .map(|h| mq_core::privacy::images::html_to_plain_text(h))
                })
                .unwrap_or_default();

            Some(BodyResult {
                text,
                html: sanitized_html,
                blocked_images: 0,
                tracking_pixels,
                is_html: body.html_body.is_some(),
            })
        }
        Ok(None) => None,
        Err(e) => {
            warn!("Failed to load message body: {e}");
            None
        }
    }
}

/// Download remote images referenced in HTML and replace their `src` attributes
/// with inline `data:` URIs. This allows WebKitGTK's `load_html()` to display
/// images without requiring network access from the WebView.
async fn inline_remote_images(html: &str) -> String {
    use base64::Engine;

    // Extract all <img src="https://..."> URLs
    let mut urls: Vec<(usize, usize, String)> = Vec::new();
    let lower = html.to_lowercase();
    let mut search_from = 0;
    while let Some(img_pos) = lower[search_from..].find("<img") {
        let abs_pos = search_from + img_pos;
        if let Some(tag_end) = html[abs_pos..].find('>') {
            let tag = &html[abs_pos..abs_pos + tag_end + 1];
            if let Some(src) = extract_img_src(tag) {
                if src.starts_with("http://") || src.starts_with("https://") {
                    // Record the tag boundaries and URL
                    urls.push((abs_pos, abs_pos + tag_end + 1, src));
                }
            }
            search_from = abs_pos + tag_end + 1;
        } else {
            break;
        }
    }

    if urls.is_empty() {
        return html.to_string();
    }

    // Download images concurrently (with timeout)
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("Mozilla/5.0")
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let mut replacements: Vec<(usize, usize, String)> = Vec::new();

    for (tag_start, tag_end, url) in &urls {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let content_type = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("image/png")
                    .split(';')
                    .next()
                    .unwrap_or("image/png")
                    .to_string();

                if let Ok(bytes) = resp.bytes().await {
                    // Limit to 5MB per image
                    if bytes.len() < 5_000_000 {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        let data_uri = format!("data:{content_type};base64,{b64}");
                        // Replace src="<url>" with src="<data_uri>" in the tag
                        let tag = &html[*tag_start..*tag_end];
                        let new_tag = replace_src_in_tag(tag, &data_uri);
                        replacements.push((*tag_start, *tag_end, new_tag));
                    }
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }

    if replacements.is_empty() {
        return html.to_string();
    }

    // Build result with replacements applied in reverse order
    let mut result = html.to_string();
    for (start, end, new_tag) in replacements.into_iter().rev() {
        result.replace_range(start..end, &new_tag);
    }

    result
}

/// Extract the `src` attribute value from an `<img>` tag.
fn extract_img_src(tag: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let src_pos = lower.find("src=")?;
    let after_eq = src_pos + 4;
    let bytes = tag.as_bytes();
    if after_eq >= bytes.len() {
        return None;
    }
    let quote = bytes[after_eq] as char;
    if quote == '"' || quote == '\'' {
        let value_start = after_eq + 1;
        let value_end = tag[value_start..].find(quote)?;
        Some(tag[value_start..value_start + value_end].to_string())
    } else {
        let value_start = after_eq;
        let value_end = tag[value_start..]
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(tag.len() - value_start);
        Some(tag[value_start..value_start + value_end].to_string())
    }
}

/// Replace the `src` attribute in an `<img>` tag with a new value.
fn replace_src_in_tag(tag: &str, new_src: &str) -> String {
    let lower = tag.to_lowercase();
    if let Some(src_pos) = lower.find("src=") {
        let after_eq = src_pos + 4;
        let bytes = tag.as_bytes();
        if after_eq < bytes.len() {
            let quote = bytes[after_eq] as char;
            if quote == '"' || quote == '\'' {
                let value_start = after_eq + 1;
                if let Some(value_end) = tag[value_start..].find(quote) {
                    let end = value_start + value_end;
                    return format!("{}{new_src}{}", &tag[..value_start], &tag[end..]);
                }
            }
        }
    }
    tag.to_string()
}

/// Create an attachment download callback that extracts content from raw MIME and saves to file.
fn make_attachment_download_callback(
    pool: Arc<SqlitePool>,
    message_id: i64,
    window: MqWindow,
) -> impl Fn(i64, String) + Clone + 'static {
    move |_att_id: i64, filename: String| {
        let pool = pool.clone();
        let win = window.clone();
        // Look up the attachment's imap_section from the DB, then extract from raw MIME
        runtime::spawn_async(
            async move {
                let attachments =
                    mq_db::queries::attachments::get_attachments(&pool, message_id)
                        .await
                        .unwrap_or_default();
                let att = attachments.iter().find(|a| a.id == _att_id);
                if let Some(att) = att {
                    let section_idx: usize =
                        att.imap_section.parse().unwrap_or(0);
                    let body =
                        mq_db::queries::message_bodies::get_body(&pool, message_id)
                            .await
                            .ok()
                            .flatten();
                    if let Some(body) = body {
                        if let Some(raw) = body.raw_mime {
                            let content =
                                mq_core::body::extract_attachment_content(&raw, section_idx);
                            return (content, filename.clone());
                        }
                    }
                }
                (None, filename.clone())
            },
            move |(content, filename): (Option<Vec<u8>>, String)| {
                if let Some(data) = content {
                    // Use GTK file dialog to let user pick save location
                    let dialog = gtk::FileDialog::builder()
                        .title("Save Attachment")
                        .initial_name(&filename)
                        .build();
                    let data = std::rc::Rc::new(data);
                    dialog.save(
                        Some(&win.upcast_ref::<gtk::Window>().clone()),
                        Option::<&gio::Cancellable>::None,
                        move |result: Result<gio::File, glib::Error>| {
                            if let Ok(file) = result {
                                if let Some(path) = file.path() {
                                    if let Err(e) = std::fs::write(path.as_path(), data.as_slice()) {
                                        error!("Failed to save attachment: {e}");
                                    } else {
                                        info!("Saved attachment to {}", path.display());
                                    }
                                }
                            }
                        },
                    );
                } else {
                    warn!("Could not extract attachment content for: {filename}");
                }
            },
        );
    }
}

/// Load all messages in a thread with images unblocked (user clicked "Load images").
/// Returns `(from, date, html, text)` tuples for `set_conversation`.
async fn load_conversation_unblocked(
    pool: &SqlitePool,
    thread_id: i64,
) -> Vec<(String, String, String, String, bool)> {
    let thread_msgs = mq_db::queries::messages::get_thread_messages(pool, thread_id)
        .await
        .unwrap_or_default();

    let mut conversation = Vec::new();
    for tmsg in &thread_msgs {
        let from_display = tmsg
            .sender_name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| format!("{n} <{}>", tmsg.sender_email))
            .unwrap_or_else(|| tmsg.sender_email.clone());

        let body = load_message_body_unblocked(pool, tmsg.id).await;
        let body_html = body
            .as_ref()
            .and_then(|b| b.html.clone())
            .unwrap_or_default();
        let body_text = body
            .as_ref()
            .map(|b| b.text.clone())
            .unwrap_or_else(|| tmsg.snippet.clone().unwrap_or_default());

        let is_read = tmsg.flags.contains("\\Seen");
        conversation.push((from_display, tmsg.date.clone(), body_html, body_text, is_read));
    }
    conversation
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

/// Store (add/remove) a flag on the IMAP server.
async fn imap_store_flag(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
    uid: u32,
    flag: &str,
    add: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use mq_core::imap::client::ImapSession;

    let accounts = mq_db::queries::accounts::get_all_accounts(pool).await?;
    let account = accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or("Account not found")?;
    let email = &account.email;

    let tokens = mq_core::keyring::get_tokens(email)
        .await?
        .ok_or("No tokens found for account")?;
    let token = tokens.access_token;

    let mut session = ImapSession::connect(email, &token).await?;
    session.select(mailbox).await?;
    session.store_flags(uid, flag, add).await?;
    let _ = session.logout().await;
    Ok(())
}

/// Move a message to Gmail Trash via IMAP.
async fn imap_archive_message(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
    uid: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use mq_core::imap::client::ImapSession;

    let accounts = mq_db::queries::accounts::get_all_accounts(pool).await?;
    let account = accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or("Account not found")?;
    let email = &account.email;

    let tokens = mq_core::keyring::get_tokens(email)
        .await?
        .ok_or("No tokens found for account")?;
    let token = tokens.access_token;

    let mut session = ImapSession::connect(email, &token).await?;
    session.select(mailbox).await?;
    session.move_message(uid, "[Gmail]/All Mail").await?;
    let _ = session.logout().await;
    Ok(())
}

async fn imap_trash_message(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
    uid: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use mq_core::imap::client::ImapSession;

    // Look up account email
    let accounts = mq_db::queries::accounts::get_all_accounts(pool).await?;
    let account = accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or("Account not found")?;
    let email = &account.email;

    // Get access token from keyring
    let tokens = mq_core::keyring::get_tokens(email)
        .await?
        .ok_or("No tokens found for account")?;
    let token = tokens.access_token;

    // Connect and move to trash
    let mut session = ImapSession::connect(email, &token).await?;
    session.select(mailbox).await?;
    session.move_message(uid, "[Gmail]/Trash").await?;
    let _ = session.logout().await;
    Ok(())
}

fn selected_message(
    list: &crate::widgets::message_list::MqMessageList,
) -> Option<MessageObject> {
    list.selection()
        .selected_item()
        .and_then(|item| item.downcast::<MessageObject>().ok())
}

/// Remove the selected message from the list and trigger a selection change
/// so that the message view loads the newly selected item.
///
/// GTK4's SingleSelection with `autoselect=true` silently re-selects after a
/// model removal, which can suppress the `selection-changed` signal (the
/// position index stays the same even though the item changed). We work
/// around this by: disabling autoselect → deselecting → removing → then
/// re-selecting in an idle callback so GTK sees a genuine position change.
fn remove_selected(list: &crate::widgets::message_list::MqMessageList) {
    let selection = list.selection();
    let pos = selection.selected();
    let model = list.model();
    if pos >= model.n_items() {
        return;
    }

    // Disable autoselect so GTK doesn't silently re-select after removal
    selection.set_autoselect(false);
    // Deselect first — this fires selection-changed with no item (handler no-ops)
    selection.set_selected(gtk::INVALID_LIST_POSITION);
    // Now remove the item from the model
    model.remove(pos);

    let new_pos = if model.n_items() == 0 {
        gtk::INVALID_LIST_POSITION
    } else {
        pos.min(model.n_items() - 1)
    };

    if new_pos != gtk::INVALID_LIST_POSITION {
        let sel = selection.clone();
        glib::idle_add_local_once(move || {
            sel.set_autoselect(true);
            sel.set_selected(new_pos);
        });
    } else {
        selection.set_autoselect(true);
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
    provider.load_from_string(
        "
        /* Prevent the read/unread toggle from showing GTK's checked highlight. */
        .read-toggle:checked {
            background: transparent;
            color: inherit;
        }

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

