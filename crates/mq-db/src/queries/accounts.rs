use sqlx::{Row, SqlitePool};

use crate::models::DbAccount;

pub async fn insert_account(
    pool: &SqlitePool,
    email: &str,
    display_name: Option<&str>,
) -> sqlx::Result<i64> {
    let row = sqlx::query("INSERT INTO accounts (email, display_name) VALUES (?, ?) RETURNING id")
        .bind(email)
        .bind(display_name)
        .fetch_one(pool)
        .await?;
    Ok(row.get("id"))
}

pub async fn get_account_by_email(
    pool: &SqlitePool,
    email: &str,
) -> sqlx::Result<Option<DbAccount>> {
    sqlx::query_as::<_, DbAccount>(
        "SELECT id, email, display_name, created_at, last_sync FROM accounts WHERE email = ?",
    )
    .bind(email)
    .fetch_optional(pool)
    .await
}

pub async fn get_all_accounts(pool: &SqlitePool) -> sqlx::Result<Vec<DbAccount>> {
    sqlx::query_as::<_, DbAccount>(
        "SELECT id, email, display_name, created_at, last_sync FROM accounts ORDER BY email",
    )
    .fetch_all(pool)
    .await
}

pub async fn delete_account(pool: &SqlitePool, account_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_last_sync(pool: &SqlitePool, account_id: i64) -> sqlx::Result<()> {
    sqlx::query("UPDATE accounts SET last_sync = datetime('now') WHERE id = ?")
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}
