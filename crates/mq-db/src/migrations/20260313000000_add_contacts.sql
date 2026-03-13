-- Contacts table for Google People API sync (autocomplete in compose).

CREATE TABLE IF NOT EXISTS contacts (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    resource_id TEXT,
    display_name TEXT,
    email       TEXT NOT NULL,
    synced_at   TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(account_id, email)
);

CREATE INDEX IF NOT EXISTS idx_contacts_account ON contacts(account_id);
CREATE INDEX IF NOT EXISTS idx_contacts_email ON contacts(account_id, email);
