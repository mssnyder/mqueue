use sqlx::SqlitePool;

use crate::models::DbDraft;

/// Insert or update a draft. Returns the draft ID.
pub async fn upsert_draft(
    pool: &SqlitePool,
    draft_id: Option<i64>,
    account_id: i64,
    to_addrs: &str,
    cc_addrs: &str,
    bcc_addrs: &str,
    subject: &str,
    body_text: &str,
    body_html: &str,
    compose_mode: &str,
    compose_data: Option<&str>,
) -> anyhow::Result<i64> {
    if let Some(id) = draft_id {
        sqlx::query(
            "UPDATE drafts SET to_addrs = ?, cc_addrs = ?, bcc_addrs = ?, \
             subject = ?, body_text = ?, body_html = ?, compose_mode = ?, compose_data = ?, \
             updated_at = datetime('now') WHERE id = ?",
        )
        .bind(to_addrs)
        .bind(cc_addrs)
        .bind(bcc_addrs)
        .bind(subject)
        .bind(body_text)
        .bind(body_html)
        .bind(compose_mode)
        .bind(compose_data)
        .bind(id)
        .execute(pool)
        .await?;
        Ok(id)
    } else {
        let row = sqlx::query(
            "INSERT INTO drafts (account_id, to_addrs, cc_addrs, bcc_addrs, \
             subject, body_text, body_html, compose_mode, compose_data) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(account_id)
        .bind(to_addrs)
        .bind(cc_addrs)
        .bind(bcc_addrs)
        .bind(subject)
        .bind(body_text)
        .bind(body_html)
        .bind(compose_mode)
        .bind(compose_data)
        .fetch_one(pool)
        .await?;
        let id: i64 = sqlx::Row::get(&row, "id");
        Ok(id)
    }
}

/// Delete a draft by ID.
pub async fn delete_draft(pool: &SqlitePool, draft_id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM drafts WHERE id = ?")
        .bind(draft_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get a draft by ID.
pub async fn get_draft(pool: &SqlitePool, draft_id: i64) -> anyhow::Result<Option<DbDraft>> {
    let draft = sqlx::query_as::<_, DbDraft>("SELECT * FROM drafts WHERE id = ?")
        .bind(draft_id)
        .fetch_optional(pool)
        .await?;
    Ok(draft)
}

/// List all drafts for an account.
pub async fn list_drafts(
    pool: &SqlitePool,
    account_id: i64,
) -> anyhow::Result<Vec<DbDraft>> {
    let drafts = sqlx::query_as::<_, DbDraft>(
        "SELECT * FROM drafts WHERE account_id = ? ORDER BY updated_at DESC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(drafts)
}
