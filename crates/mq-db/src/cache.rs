//! Cache management: pruning old message bodies, VACUUM scheduling.

use sqlx::SqlitePool;
use tracing::info;

/// Delete cached message bodies older than `retention_days`.
pub async fn prune_old_bodies(pool: &SqlitePool, retention_days: u32) -> sqlx::Result<u64> {
    let days = retention_days.to_string();
    let result = sqlx::query(
        "DELETE FROM message_bodies WHERE fetched_at < datetime('now', '-' || ? || ' days')",
    )
    .bind(&days)
    .execute(pool)
    .await?;
    let count = result.rows_affected();
    if count > 0 {
        info!(count, retention_days, "Pruned old message bodies");
    }
    Ok(count)
}

/// Run SQLite VACUUM to reclaim disk space.
pub async fn vacuum(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query("VACUUM").execute(pool).await?;
    info!("Database vacuumed");
    Ok(())
}
