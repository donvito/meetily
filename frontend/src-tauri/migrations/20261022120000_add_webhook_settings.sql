-- Live meeting webhook preferences.
-- The sender key (header_value) is stored in this SQLite database, the same
-- place Meetily stores provider API keys. It is not written to the JSON store.

CREATE TABLE IF NOT EXISTS webhook_settings (
    id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 0,
    url TEXT NOT NULL DEFAULT '',
    header_name TEXT NOT NULL DEFAULT 'Authorization',
    header_value TEXT NOT NULL DEFAULT '',
    mode TEXT NOT NULL DEFAULT 'wake_word',
    interval_minutes INTEGER NOT NULL DEFAULT 5,
    wake_word TEXT NOT NULL DEFAULT 'Emma'
);
