-- m'Queue initial schema

CREATE TABLE IF NOT EXISTS accounts (
    id          INTEGER PRIMARY KEY,
    email       TEXT NOT NULL UNIQUE,
    display_name TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    last_sync   TEXT
);

CREATE TABLE IF NOT EXISTS labels (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    imap_name   TEXT NOT NULL,
    label_type  TEXT NOT NULL DEFAULT 'user',
    color       TEXT,
    unread_count INTEGER NOT NULL DEFAULT 0,
    total_count  INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account_id, name)
);

CREATE TABLE IF NOT EXISTS messages (
    id                  INTEGER PRIMARY KEY,
    account_id          INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    uid                 INTEGER NOT NULL,
    mailbox             TEXT NOT NULL,
    gmail_msg_id        INTEGER,
    gmail_thread_id     INTEGER,
    message_id          TEXT,
    in_reply_to         TEXT,
    references_json     TEXT,
    sender_name         TEXT,
    sender_email        TEXT NOT NULL,
    recipient_to        TEXT NOT NULL,
    recipient_cc        TEXT,
    subject             TEXT,
    snippet             TEXT,
    date                TEXT NOT NULL,
    flags               TEXT NOT NULL DEFAULT '[]',
    has_attachments     INTEGER NOT NULL DEFAULT 0,
    body_structure      TEXT,
    list_unsubscribe    TEXT,
    list_unsubscribe_post TEXT,
    modseq              INTEGER,
    uid_validity        INTEGER NOT NULL,
    cached_at           TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(account_id, mailbox, uid)
);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    subject,
    sender_name,
    sender_email,
    snippet,
    body_text,
    content=messages,
    content_rowid=id
);

CREATE TRIGGER IF NOT EXISTS messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, sender_name, sender_email, snippet, body_text)
    VALUES (new.id, new.subject, new.sender_name, new.sender_email, new.snippet, '');
END;

CREATE TRIGGER IF NOT EXISTS messages_fts_delete AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, sender_name, sender_email, snippet, body_text)
    VALUES ('delete', old.id, old.subject, old.sender_name, old.sender_email, old.snippet, '');
END;

CREATE TRIGGER IF NOT EXISTS messages_fts_update AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, sender_name, sender_email, snippet, body_text)
    VALUES ('delete', old.id, old.subject, old.sender_name, old.sender_email, old.snippet, '');
    INSERT INTO messages_fts(rowid, subject, sender_name, sender_email, snippet, body_text)
    VALUES (new.id, new.subject, new.sender_name, new.sender_email, new.snippet, '');
END;

CREATE TABLE IF NOT EXISTS message_bodies (
    message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    raw_mime    BLOB,
    html_body   TEXT,
    text_body   TEXT,
    fetched_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS attachments (
    id          INTEGER PRIMARY KEY,
    message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename    TEXT,
    mime_type   TEXT NOT NULL,
    size        INTEGER,
    content_id  TEXT,
    imap_section TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS message_labels (
    message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label_id    INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, label_id)
);

CREATE TABLE IF NOT EXISTS sender_image_allowlist (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    sender_email TEXT NOT NULL,
    added_at    TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(account_id, sender_email)
);

CREATE TABLE IF NOT EXISTS offline_queue (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    op_type     TEXT NOT NULL,
    payload     TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending',
    retry_count INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    last_attempt TEXT,
    error_msg   TEXT
);

CREATE TABLE IF NOT EXISTS settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_state (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox         TEXT NOT NULL,
    uid_validity    INTEGER NOT NULL,
    highest_modseq  INTEGER NOT NULL DEFAULT 0,
    highest_uid     INTEGER NOT NULL DEFAULT 0,
    last_sync       TEXT,
    UNIQUE(account_id, mailbox)
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_messages_account_mailbox ON messages(account_id, mailbox, uid);
CREATE INDEX IF NOT EXISTS idx_messages_gmail_thread ON messages(gmail_thread_id);
CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(date DESC);
CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender_email);
CREATE INDEX IF NOT EXISTS idx_message_labels_label ON message_labels(label_id);
CREATE INDEX IF NOT EXISTS idx_offline_queue_status ON offline_queue(status, created_at);
