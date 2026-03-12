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
