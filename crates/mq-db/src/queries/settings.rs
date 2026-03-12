use sqlx::{Row, SqlitePool};

pub async fn get_setting(pool: &SqlitePool, key: &str) -> sqlx::Result<Option<String>> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("value")))
}

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn is_sender_in_image_allowlist(
    pool: &SqlitePool,
    account_id: i64,
    sender_email: &str,
) -> sqlx::Result<bool> {
    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM sender_image_allowlist WHERE account_id = ? AND sender_email = ?",
    )
    .bind(account_id)
    .bind(sender_email)
    .fetch_one(pool)
    .await?;
    let count: i64 = row.get("cnt");
    Ok(count > 0)
}

pub async fn add_sender_to_image_allowlist(
    pool: &SqlitePool,
    account_id: i64,
    sender_email: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO sender_image_allowlist (account_id, sender_email) VALUES (?, ?)",
    )
    .bind(account_id)
    .bind(sender_email)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_sender_from_image_allowlist(
    pool: &SqlitePool,
    account_id: i64,
    sender_email: &str,
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sender_image_allowlist WHERE account_id = ? AND sender_email = ?")
        .bind(account_id)
        .bind(sender_email)
        .execute(pool)
        .await?;
    Ok(())
}
