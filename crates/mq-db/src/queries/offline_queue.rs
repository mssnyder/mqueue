use sqlx::{Row, SqlitePool};

use crate::models::DbOfflineOp;

pub async fn enqueue_op(
    pool: &SqlitePool,
    account_id: i64,
    op_type: &str,
    payload: &str,
) -> sqlx::Result<i64> {
    let row = sqlx::query(
        "INSERT INTO offline_queue (account_id, op_type, payload) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(account_id)
    .bind(op_type)
    .bind(payload)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn get_pending_ops(
    pool: &SqlitePool,
    account_id: i64,
) -> sqlx::Result<Vec<DbOfflineOp>> {
    sqlx::query_as::<_, DbOfflineOp>(
        "SELECT id, account_id, op_type, payload, status, retry_count,
            created_at, last_attempt, error_msg
        FROM offline_queue
        WHERE account_id = ? AND status = 'pending'
        ORDER BY created_at ASC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
}

pub async fn mark_op_complete(pool: &SqlitePool, op_id: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE offline_queue SET status = 'completed', last_attempt = datetime('now') WHERE id = ?",
    )
    .bind(op_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_op_failed(
    pool: &SqlitePool,
    op_id: i64,
    error_msg: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE offline_queue SET status = 'failed', retry_count = retry_count + 1, last_attempt = datetime('now'), error_msg = ? WHERE id = ?",
    )
    .bind(error_msg)
    .bind(op_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn pending_op_count(pool: &SqlitePool, account_id: i64) -> sqlx::Result<i64> {
    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM offline_queue WHERE account_id = ? AND status = 'pending'",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    Ok(row.get("cnt"))
}

pub async fn total_pending_count(pool: &SqlitePool) -> sqlx::Result<i64> {
    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM offline_queue WHERE status = 'pending'",
    )
    .fetch_one(pool)
    .await?;
    Ok(row.get("cnt"))
}
