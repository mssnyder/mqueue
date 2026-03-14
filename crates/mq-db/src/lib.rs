pub mod cache;
pub mod models;
pub mod queries;

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
pub use sqlx::SqlitePool;
use tracing::info;

/// The initial migration SQL.
const MIGRATION_001: &str = include_str!("migrations/20260312000000_initial.sql");
/// Add contacts table.
const MIGRATION_002: &str = include_str!("migrations/20260313000000_add_contacts.sql");
/// Add drafts table.
const MIGRATION_003: &str = include_str!("migrations/20260314000000_add_drafts.sql");
/// Add body_html column to drafts.
const MIGRATION_004: &str = include_str!("migrations/20260315000000_add_draft_html.sql");

/// Initialize the SQLite database, running migrations if needed.
pub async fn init_pool(db_path: &Path) -> anyhow::Result<SqlitePool> {
    // Ensure parent directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(options)
        .await?;

    run_migrations(&pool).await?;
    info!(path = %db_path.display(), "Database initialized");

    Ok(pool)
}

/// Run migrations manually (avoids compile-time DATABASE_URL requirement).
async fn run_migrations(pool: &SqlitePool) -> anyhow::Result<()> {
    // Create migrations tracking table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _mq_migrations (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(pool)
    .await?;

    // Check if migration 001 has been applied
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM _mq_migrations WHERE name = '001_initial'")
        .fetch_one(pool)
        .await?;
    let count: i64 = sqlx::Row::get(&row, "cnt");

    if count == 0 {
        info!("Applying migration 001_initial");
        // Execute multi-statement migration
        // SQLite doesn't support multiple statements in one query via sqlx,
        // so we split by semicolons (being careful with triggers).
        for statement in split_sql_statements(MIGRATION_001) {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed).execute(pool).await?;
            }
        }

        sqlx::query("INSERT INTO _mq_migrations (name) VALUES ('001_initial')")
            .execute(pool)
            .await?;
        info!("Migration 001_initial applied successfully");
    }

    // Migration 002: contacts table
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM _mq_migrations WHERE name = '002_add_contacts'")
        .fetch_one(pool)
        .await?;
    let count: i64 = sqlx::Row::get(&row, "cnt");

    if count == 0 {
        info!("Applying migration 002_add_contacts");
        for statement in split_sql_statements(MIGRATION_002) {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed).execute(pool).await?;
            }
        }

        sqlx::query("INSERT INTO _mq_migrations (name) VALUES ('002_add_contacts')")
            .execute(pool)
            .await?;
        info!("Migration 002_add_contacts applied successfully");
    }

    // Migration 003: drafts table
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM _mq_migrations WHERE name = '003_add_drafts'")
        .fetch_one(pool)
        .await?;
    let count: i64 = sqlx::Row::get(&row, "cnt");

    if count == 0 {
        info!("Applying migration 003_add_drafts");
        for statement in split_sql_statements(MIGRATION_003) {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed).execute(pool).await?;
            }
        }

        sqlx::query("INSERT INTO _mq_migrations (name) VALUES ('003_add_drafts')")
            .execute(pool)
            .await?;
        info!("Migration 003_add_drafts applied successfully");
    }

    // Migration 004: add body_html to drafts
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM _mq_migrations WHERE name = '004_add_draft_html'")
        .fetch_one(pool)
        .await?;
    let count: i64 = sqlx::Row::get(&row, "cnt");

    if count == 0 {
        info!("Applying migration 004_add_draft_html");
        for statement in split_sql_statements(MIGRATION_004) {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed).execute(pool).await?;
            }
        }

        sqlx::query("INSERT INTO _mq_migrations (name) VALUES ('004_add_draft_html')")
            .execute(pool)
            .await?;
        info!("Migration 004_add_draft_html applied successfully");
    }

    Ok(())
}

/// Split SQL text into individual statements, respecting trigger bodies
/// that contain semicolons.
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_trigger = false;

    for line in sql.lines() {
        let trimmed = line.trim().to_uppercase();

        if trimmed.starts_with("CREATE TRIGGER") {
            in_trigger = true;
        }

        current.push_str(line);
        current.push('\n');

        if in_trigger {
            if trimmed == "END;" {
                in_trigger = false;
                statements.push(std::mem::take(&mut current));
            }
        } else if line.trim().ends_with(';') {
            statements.push(std::mem::take(&mut current));
        }
    }

    if !current.trim().is_empty() {
        statements.push(current);
    }

    statements
}
