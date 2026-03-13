use sqlx::{Row, SqlitePool};

use crate::models::DbMessage;

pub async fn upsert_message(
    pool: &SqlitePool,
    account_id: i64,
    uid: i64,
    mailbox: &str,
    gmail_msg_id: Option<i64>,
    gmail_thread_id: Option<i64>,
    message_id: Option<&str>,
    in_reply_to: Option<&str>,
    references_json: Option<&str>,
    sender_name: Option<&str>,
    sender_email: &str,
    recipient_to: &str,
    recipient_cc: Option<&str>,
    subject: Option<&str>,
    snippet: Option<&str>,
    date: &str,
    flags: &str,
    has_attachments: bool,
    body_structure: Option<&str>,
    list_unsubscribe: Option<&str>,
    list_unsubscribe_post: Option<&str>,
    modseq: Option<i64>,
    uid_validity: i64,
) -> sqlx::Result<i64> {
    let has_att_i: i32 = has_attachments as i32;
    let row = sqlx::query(
        "INSERT INTO messages (
            account_id, uid, mailbox, gmail_msg_id, gmail_thread_id,
            message_id, in_reply_to, references_json,
            sender_name, sender_email, recipient_to, recipient_cc,
            subject, snippet, date, flags, has_attachments, body_structure,
            list_unsubscribe, list_unsubscribe_post, modseq, uid_validity
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(account_id, mailbox, uid) DO UPDATE SET
            flags = excluded.flags,
            modseq = excluded.modseq,
            gmail_msg_id = COALESCE(excluded.gmail_msg_id, messages.gmail_msg_id),
            gmail_thread_id = COALESCE(excluded.gmail_thread_id, messages.gmail_thread_id)
        RETURNING id",
    )
    .bind(account_id)
    .bind(uid)
    .bind(mailbox)
    .bind(gmail_msg_id)
    .bind(gmail_thread_id)
    .bind(message_id)
    .bind(in_reply_to)
    .bind(references_json)
    .bind(sender_name)
    .bind(sender_email)
    .bind(recipient_to)
    .bind(recipient_cc)
    .bind(subject)
    .bind(snippet)
    .bind(date)
    .bind(flags)
    .bind(has_att_i)
    .bind(body_structure)
    .bind(list_unsubscribe)
    .bind(list_unsubscribe_post)
    .bind(modseq)
    .bind(uid_validity)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn get_messages_for_mailbox(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<DbMessage>> {
    sqlx::query_as::<_, DbMessage>(
        "SELECT id, account_id, uid, mailbox, gmail_msg_id, gmail_thread_id,
            message_id, in_reply_to, references_json,
            sender_name, sender_email, recipient_to, recipient_cc,
            subject, snippet, date, flags, has_attachments, body_structure,
            list_unsubscribe, list_unsubscribe_post, modseq, uid_validity, cached_at
        FROM messages
        WHERE account_id = ? AND mailbox = ?
        ORDER BY date DESC
        LIMIT ? OFFSET ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn get_message_by_uid(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
    uid: i64,
) -> sqlx::Result<Option<DbMessage>> {
    sqlx::query_as::<_, DbMessage>(
        "SELECT id, account_id, uid, mailbox, gmail_msg_id, gmail_thread_id,
            message_id, in_reply_to, references_json,
            sender_name, sender_email, recipient_to, recipient_cc,
            subject, snippet, date, flags, has_attachments, body_structure,
            list_unsubscribe, list_unsubscribe_post, modseq, uid_validity, cached_at
        FROM messages
        WHERE account_id = ? AND mailbox = ? AND uid = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid)
    .fetch_optional(pool)
    .await
}

pub async fn update_flags(
    pool: &SqlitePool,
    message_id: i64,
    flags: &str,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE messages SET flags = ? WHERE id = ?")
        .bind(flags)
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_message(pool: &SqlitePool, message_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_highest_uid(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
) -> sqlx::Result<Option<i64>> {
    let row = sqlx::query("SELECT MAX(uid) as max_uid FROM messages WHERE account_id = ? AND mailbox = ?")
        .bind(account_id)
        .bind(mailbox)
        .fetch_one(pool)
        .await?;
    Ok(row.get("max_uid"))
}

/// Get messages across ALL accounts for a given mailbox (unified inbox view).
pub async fn get_messages_all_accounts_for_mailbox(
    pool: &SqlitePool,
    mailbox: &str,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<DbMessage>> {
    sqlx::query_as::<_, DbMessage>(
        "SELECT id, account_id, uid, mailbox, gmail_msg_id, gmail_thread_id,
            message_id, in_reply_to, references_json,
            sender_name, sender_email, recipient_to, recipient_cc,
            subject, snippet, date, flags, has_attachments, body_structure,
            list_unsubscribe, list_unsubscribe_post, modseq, uid_validity, cached_at
        FROM messages
        WHERE mailbox = ?
        ORDER BY date DESC
        LIMIT ? OFFSET ?",
    )
    .bind(mailbox)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn search_fts(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
) -> sqlx::Result<Vec<DbMessage>> {
    sqlx::query_as::<_, DbMessage>(
        "SELECT m.id, m.account_id, m.uid, m.mailbox, m.gmail_msg_id, m.gmail_thread_id,
            m.message_id, m.in_reply_to, m.references_json,
            m.sender_name, m.sender_email, m.recipient_to, m.recipient_cc,
            m.subject, m.snippet, m.date, m.flags, m.has_attachments, m.body_structure,
            m.list_unsubscribe, m.list_unsubscribe_post, m.modseq, m.uid_validity, m.cached_at
        FROM messages m
        JOIN messages_fts ON messages_fts.rowid = m.id
        WHERE messages_fts MATCH ?
        ORDER BY rank
        LIMIT ?",
    )
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// FTS5 search scoped to a single account.
pub async fn search_fts_for_account(
    pool: &SqlitePool,
    account_id: i64,
    query: &str,
    limit: i64,
) -> sqlx::Result<Vec<DbMessage>> {
    sqlx::query_as::<_, DbMessage>(
        "SELECT m.id, m.account_id, m.uid, m.mailbox, m.gmail_msg_id, m.gmail_thread_id,
            m.message_id, m.in_reply_to, m.references_json,
            m.sender_name, m.sender_email, m.recipient_to, m.recipient_cc,
            m.subject, m.snippet, m.date, m.flags, m.has_attachments, m.body_structure,
            m.list_unsubscribe, m.list_unsubscribe_post, m.modseq, m.uid_validity, m.cached_at
        FROM messages m
        JOIN messages_fts ON messages_fts.rowid = m.id
        WHERE messages_fts MATCH ? AND m.account_id = ?
        ORDER BY rank
        LIMIT ?",
    )
    .bind(query)
    .bind(account_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Resolve IMAP UIDs to local database message IDs.
///
/// Used by server-side search to map Gmail X-GM-RAW results back to local rows.
pub async fn resolve_uids_to_ids(
    pool: &SqlitePool,
    account_id: Option<i64>,
    uids: &[u32],
) -> sqlx::Result<Vec<i64>> {
    if uids.is_empty() {
        return Ok(vec![]);
    }

    // Build a comma-separated list of UID placeholders
    let placeholders: Vec<String> = uids.iter().map(|_| "?".to_string()).collect();
    let placeholder_str = placeholders.join(",");

    let sql = match account_id {
        Some(_) => format!(
            "SELECT id FROM messages WHERE account_id = ? AND uid IN ({placeholder_str}) ORDER BY date DESC"
        ),
        None => format!(
            "SELECT id FROM messages WHERE uid IN ({placeholder_str}) ORDER BY date DESC"
        ),
    };

    let mut query = sqlx::query_scalar::<_, i64>(&sql);

    if let Some(aid) = account_id {
        query = query.bind(aid);
    }

    for uid in uids {
        query = query.bind(*uid as i64);
    }

    query.fetch_all(pool).await
}

/// Get messages by a list of database IDs.
pub async fn get_messages_by_ids(
    pool: &SqlitePool,
    ids: &[i64],
) -> sqlx::Result<Vec<DbMessage>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
    let placeholder_str = placeholders.join(",");

    let sql = format!(
        "SELECT id, account_id, uid, mailbox, gmail_msg_id, gmail_thread_id,
            message_id, in_reply_to, references_json,
            sender_name, sender_email, recipient_to, recipient_cc,
            subject, snippet, date, flags, has_attachments, body_structure,
            list_unsubscribe, list_unsubscribe_post, modseq, uid_validity, cached_at
        FROM messages
        WHERE id IN ({placeholder_str})
        ORDER BY date DESC"
    );

    let mut query = sqlx::query_as::<_, DbMessage>(&sql);
    for id in ids {
        query = query.bind(id);
    }

    query.fetch_all(pool).await
}

/// Update the FTS body_text for a message (called when body is fetched).
pub async fn update_fts_body_text(
    pool: &SqlitePool,
    message_id: i64,
    body_text: &str,
) -> sqlx::Result<()> {
    // The FTS5 content-sync triggers handle INSERT/DELETE, but we need to
    // manually update body_text since it's not in the messages table.
    // We do a DELETE + INSERT to update the FTS entry.
    sqlx::query(
        "INSERT INTO messages_fts(messages_fts, rowid, subject, sender_name, sender_email, snippet, body_text)
         VALUES('delete', ?, (SELECT subject FROM messages WHERE id = ?),
                (SELECT sender_name FROM messages WHERE id = ?),
                (SELECT sender_email FROM messages WHERE id = ?),
                (SELECT snippet FROM messages WHERE id = ?), '')"
    )
    .bind(message_id)
    .bind(message_id)
    .bind(message_id)
    .bind(message_id)
    .bind(message_id)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO messages_fts(rowid, subject, sender_name, sender_email, snippet, body_text)
         VALUES(?, (SELECT subject FROM messages WHERE id = ?),
                (SELECT sender_name FROM messages WHERE id = ?),
                (SELECT sender_email FROM messages WHERE id = ?),
                (SELECT snippet FROM messages WHERE id = ?), ?)"
    )
    .bind(message_id)
    .bind(message_id)
    .bind(message_id)
    .bind(message_id)
    .bind(message_id)
    .bind(body_text)
    .execute(pool)
    .await?;

    Ok(())
}
