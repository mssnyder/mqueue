use sqlx::SqlitePool;

use crate::models::DbSyncState;

pub async fn get_sync_state(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
) -> sqlx::Result<Option<DbSyncState>> {
    sqlx::query_as::<_, DbSyncState>(
        "SELECT id, account_id, mailbox, uid_validity, highest_modseq, highest_uid, last_sync
        FROM sync_state
        WHERE account_id = ? AND mailbox = ?",
    )
    .bind(account_id)
    .bind(mailbox)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_sync_state(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
    uid_validity: i64,
    highest_modseq: i64,
    highest_uid: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO sync_state (account_id, mailbox, uid_validity, highest_modseq, highest_uid, last_sync)
        VALUES (?, ?, ?, ?, ?, datetime('now'))
        ON CONFLICT(account_id, mailbox) DO UPDATE SET
            uid_validity = excluded.uid_validity,
            highest_modseq = excluded.highest_modseq,
            highest_uid = excluded.highest_uid,
            last_sync = excluded.last_sync",
    )
    .bind(account_id)
    .bind(mailbox)
    .bind(uid_validity)
    .bind(highest_modseq)
    .bind(highest_uid)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_sync_state(
    pool: &SqlitePool,
    account_id: i64,
    mailbox: &str,
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sync_state WHERE account_id = ? AND mailbox = ?")
        .bind(account_id)
        .bind(mailbox)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_all_sync_states(
    pool: &SqlitePool,
    account_id: i64,
) -> sqlx::Result<Vec<DbSyncState>> {
    sqlx::query_as::<_, DbSyncState>(
        "SELECT id, account_id, mailbox, uid_validity, highest_modseq, highest_uid, last_sync
        FROM sync_state
        WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
}
