use sqlx::{Row, SqlitePool};

use crate::models::DbLabel;

pub async fn upsert_label(
    pool: &SqlitePool,
    account_id: i64,
    name: &str,
    imap_name: &str,
    label_type: &str,
) -> sqlx::Result<i64> {
    let row = sqlx::query(
        "INSERT INTO labels (account_id, name, imap_name, label_type)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(account_id, name) DO UPDATE SET
            imap_name = excluded.imap_name,
            label_type = excluded.label_type
        RETURNING id",
    )
    .bind(account_id)
    .bind(name)
    .bind(imap_name)
    .bind(label_type)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn get_all_labels(
    pool: &SqlitePool,
    account_id: i64,
) -> sqlx::Result<Vec<DbLabel>> {
    sqlx::query_as::<_, DbLabel>(
        "SELECT id, account_id, name, imap_name, label_type, color, unread_count, total_count
        FROM labels
        WHERE account_id = ?
        ORDER BY label_type DESC, name",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
}

pub async fn set_message_labels(
    pool: &SqlitePool,
    message_id: i64,
    label_ids: &[i64],
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM message_labels WHERE message_id = ?")
        .bind(message_id)
        .execute(pool)
        .await?;

    for label_id in label_ids {
        sqlx::query("INSERT INTO message_labels (message_id, label_id) VALUES (?, ?)")
            .bind(message_id)
            .bind(label_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn update_label_counts(
    pool: &SqlitePool,
    label_id: i64,
    unread_count: i64,
    total_count: i64,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE labels SET unread_count = ?, total_count = ? WHERE id = ?")
        .bind(unread_count)
        .bind(total_count)
        .bind(label_id)
        .execute(pool)
        .await?;
    Ok(())
}
