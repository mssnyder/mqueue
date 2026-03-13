use sqlx::SqlitePool;

use crate::models::DbAttachment;

/// Insert attachment metadata for a message.
pub async fn insert_attachment(
    pool: &SqlitePool,
    message_id: i64,
    filename: Option<&str>,
    mime_type: &str,
    size: Option<i64>,
    content_id: Option<&str>,
    imap_section: &str,
) -> sqlx::Result<i64> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO attachments (message_id, filename, mime_type, size, content_id, imap_section)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(message_id)
    .bind(filename)
    .bind(mime_type)
    .bind(size)
    .bind(content_id)
    .bind(imap_section)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Get all attachments for a message.
pub async fn get_attachments(
    pool: &SqlitePool,
    message_id: i64,
) -> sqlx::Result<Vec<DbAttachment>> {
    sqlx::query_as::<_, DbAttachment>(
        "SELECT id, message_id, filename, mime_type, size, content_id, imap_section
         FROM attachments
         WHERE message_id = ?
         ORDER BY id",
    )
    .bind(message_id)
    .fetch_all(pool)
    .await
}

/// Delete all attachments for a message (used when re-syncing body).
pub async fn delete_for_message(
    pool: &SqlitePool,
    message_id: i64,
) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM attachments WHERE message_id = ?")
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Check if a message has any attachments stored.
pub async fn has_attachments(pool: &SqlitePool, message_id: i64) -> sqlx::Result<bool> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM attachments WHERE message_id = ?",
    )
    .bind(message_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0 > 0)
}
