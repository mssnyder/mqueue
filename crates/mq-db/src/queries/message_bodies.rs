use sqlx::SqlitePool;

use crate::models::DbMessageBody;

pub async fn get_body(
    pool: &SqlitePool,
    message_id: i64,
) -> sqlx::Result<Option<DbMessageBody>> {
    sqlx::query_as::<_, DbMessageBody>(
        "SELECT message_id, raw_mime, html_body, text_body, fetched_at
        FROM message_bodies
        WHERE message_id = ?",
    )
    .bind(message_id)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_body(
    pool: &SqlitePool,
    message_id: i64,
    raw_mime: Option<&[u8]>,
    html_body: Option<&str>,
    text_body: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO message_bodies (message_id, raw_mime, html_body, text_body)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(message_id) DO UPDATE SET
            raw_mime = excluded.raw_mime,
            html_body = excluded.html_body,
            text_body = excluded.text_body,
            fetched_at = datetime('now')",
    )
    .bind(message_id)
    .bind(raw_mime)
    .bind(html_body)
    .bind(text_body)
    .execute(pool)
    .await?;
    Ok(())
}

/// Check if a body already exists for a message (avoids re-fetching).
pub async fn has_body(pool: &SqlitePool, message_id: i64) -> sqlx::Result<bool> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM message_bodies WHERE message_id = ?",
    )
    .bind(message_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0 > 0)
}

pub async fn delete_body(pool: &SqlitePool, message_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM message_bodies WHERE message_id = ?")
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(())
}
