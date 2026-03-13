use sqlx::SqlitePool;

use crate::models::DbContact;

/// Insert or update a contact for a given account.
pub async fn upsert_contact(
    pool: &SqlitePool,
    account_id: i64,
    resource_id: Option<&str>,
    display_name: Option<&str>,
    email: &str,
) -> sqlx::Result<i64> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO contacts (account_id, resource_id, display_name, email, synced_at)
         VALUES (?, ?, ?, ?, datetime('now'))
         ON CONFLICT(account_id, email) DO UPDATE SET
             resource_id = excluded.resource_id,
             display_name = excluded.display_name,
             synced_at = excluded.synced_at
         RETURNING id",
    )
    .bind(account_id)
    .bind(resource_id)
    .bind(display_name)
    .bind(email)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Search contacts by email or name prefix (for autocomplete).
pub async fn search_contacts(
    pool: &SqlitePool,
    account_id: i64,
    query: &str,
    limit: i64,
) -> sqlx::Result<Vec<DbContact>> {
    let pattern = format!("%{query}%");
    sqlx::query_as::<_, DbContact>(
        "SELECT id, account_id, resource_id, display_name, email, synced_at
         FROM contacts
         WHERE account_id = ? AND (email LIKE ? OR display_name LIKE ?)
         ORDER BY display_name, email
         LIMIT ?",
    )
    .bind(account_id)
    .bind(&pattern)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Get all contacts for an account.
pub async fn get_all_for_account(
    pool: &SqlitePool,
    account_id: i64,
) -> sqlx::Result<Vec<DbContact>> {
    sqlx::query_as::<_, DbContact>(
        "SELECT id, account_id, resource_id, display_name, email, synced_at
         FROM contacts
         WHERE account_id = ?
         ORDER BY display_name, email",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
}

/// Delete all contacts for an account (used before full re-sync).
pub async fn delete_all_for_account(
    pool: &SqlitePool,
    account_id: i64,
) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM contacts WHERE account_id = ?")
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
