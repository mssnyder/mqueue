-- Draft storage for compose-in-progress emails.
CREATE TABLE IF NOT EXISTS drafts (
    id           INTEGER PRIMARY KEY,
    account_id   INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    to_addrs     TEXT NOT NULL DEFAULT '',
    cc_addrs     TEXT NOT NULL DEFAULT '',
    bcc_addrs    TEXT NOT NULL DEFAULT '',
    subject      TEXT NOT NULL DEFAULT '',
    body_text    TEXT NOT NULL DEFAULT '',
    compose_mode TEXT NOT NULL DEFAULT 'new',
    compose_data TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
