//! Queries for the sender image allowlist.
//!
//! Manages per-sender "always load remote images" entries.

use sqlx::SqlitePool;

use crate::models::DbSenderAllowlist;

/// Check if a sender is in the allowlist for the given account.
pub async fn is_allowed(
    pool: &SqlitePool,
    account_id: i64,
    sender_email: &str,
) -> Result<bool, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM sender_image_allowlist
         WHERE account_id = ? AND sender_email = ?
         LIMIT 1",
    )
    .bind(account_id)
    .bind(sender_email)
    .fetch_optional(pool)
    .await?;

    Ok(row.is_some())
}

/// Add a sender to the image allowlist for the given account.
///
/// Does nothing if the sender is already in the list.
pub async fn add_sender(
    pool: &SqlitePool,
    account_id: i64,
    sender_email: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO sender_image_allowlist (account_id, sender_email)
         VALUES (?, ?)",
    )
    .bind(account_id)
    .bind(sender_email)
    .execute(pool)
    .await?;

    Ok(())
}

/// Remove a sender from the image allowlist.
pub async fn remove_sender(
    pool: &SqlitePool,
    account_id: i64,
    sender_email: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM sender_image_allowlist
         WHERE account_id = ? AND sender_email = ?",
    )
    .bind(account_id)
    .bind(sender_email)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get all allowed senders for the given account.
pub async fn get_all_for_account(
    pool: &SqlitePool,
    account_id: i64,
) -> Result<Vec<DbSenderAllowlist>, sqlx::Error> {
    let rows = sqlx::query_as::<_, DbSenderAllowlist>(
        "SELECT id, account_id, sender_email, added_at
         FROM sender_image_allowlist
         WHERE account_id = ?
         ORDER BY sender_email",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
