CREATE TABLE accounts (
  id            TEXT PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE,
  display_name  TEXT,
  provider      TEXT NOT NULL,
  imap_host     TEXT NOT NULL,
  imap_port     INTEGER NOT NULL DEFAULT 993,
  smtp_host     TEXT NOT NULL,
  smtp_port     INTEGER NOT NULL DEFAULT 587,
  smtp_security TEXT NOT NULL DEFAULT 'starttls',
  auth_kind     TEXT NOT NULL DEFAULT 'password',
  created_at    INTEGER NOT NULL
);

CREATE TABLE folders (
  id            INTEGER PRIMARY KEY,
  account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  imap_name     TEXT NOT NULL,
  role          TEXT,
  display_name  TEXT NOT NULL,
  uidvalidity   INTEGER,
  last_seen_uid INTEGER NOT NULL DEFAULT 0,
  backfill_done INTEGER NOT NULL DEFAULT 0,
  unread_count  INTEGER NOT NULL DEFAULT 0,
  sort_order    INTEGER NOT NULL DEFAULT 0,
  UNIQUE(account_id, imap_name)
);

CREATE TABLE threads (
  id             INTEGER PRIMARY KEY,
  account_id     TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  subject_norm   TEXT,
  last_date      INTEGER NOT NULL,
  message_count  INTEGER NOT NULL DEFAULT 1,
  unread_count   INTEGER NOT NULL DEFAULT 0,
  starred        INTEGER NOT NULL DEFAULT 0,
  snippet        TEXT
);
CREATE INDEX idx_threads_date ON threads(account_id, last_date DESC);

CREATE TABLE messages (
  id              INTEGER PRIMARY KEY,
  account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  folder_id       INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  uid             INTEGER NOT NULL,
  thread_id       INTEGER REFERENCES threads(id),
  message_id      TEXT,
  in_reply_to     TEXT,
  references_ids  TEXT,
  subject         TEXT,
  from_name       TEXT,
  from_addr       TEXT,
  to_addrs        TEXT,
  cc_addrs        TEXT,
  date            INTEGER NOT NULL,
  snippet         TEXT,
  size            INTEGER,
  is_read         INTEGER NOT NULL DEFAULT 0,
  is_starred      INTEGER NOT NULL DEFAULT 0,
  has_attachments INTEGER NOT NULL DEFAULT 0,
  body_state      INTEGER NOT NULL DEFAULT 0,
  UNIQUE(folder_id, uid)
);
CREATE INDEX idx_messages_thread ON messages(thread_id);
CREATE INDEX idx_messages_msgid ON messages(account_id, message_id);
CREATE INDEX idx_messages_folder_date ON messages(folder_id, date DESC);

-- Reference identifiers (References + In-Reply-To) per message, for
-- reverse threading lookups: a parent that syncs after its replies must
-- find the children that already reference it.
CREATE TABLE message_refs (
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  ref        TEXT NOT NULL
);
CREATE INDEX idx_message_refs_ref ON message_refs(ref);
CREATE INDEX idx_message_refs_message ON message_refs(message_id);

CREATE TABLE message_bodies (
  message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
  body_html   TEXT,
  body_text   TEXT,
  headers_raw TEXT
);

CREATE TABLE attachments (
  id         INTEGER PRIMARY KEY,
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  part_id    TEXT NOT NULL,
  filename   TEXT,
  mime_type  TEXT,
  size       INTEGER,
  content_id TEXT,
  is_inline  INTEGER NOT NULL DEFAULT 0,
  cache_path TEXT
);
CREATE INDEX idx_attachments_message ON attachments(message_id);

CREATE TABLE drafts (
  id                  INTEGER PRIMARY KEY,
  account_id          TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  reply_to_message_id INTEGER,
  mode                TEXT NOT NULL DEFAULT 'new',
  to_addrs            TEXT,
  cc_addrs            TEXT,
  bcc_addrs           TEXT,
  subject             TEXT,
  body_text           TEXT,
  updated_at          INTEGER NOT NULL
);

CREATE TABLE pending_ops (
  id         INTEGER PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  kind       TEXT NOT NULL,
  payload    TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  attempts   INTEGER NOT NULL DEFAULT 0,
  state      TEXT NOT NULL DEFAULT 'pending'
);

CREATE TABLE remote_image_senders (
  addr       TEXT PRIMARY KEY,
  allowed_at INTEGER NOT NULL
);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Contentless FTS5 index over messages; rows keyed by messages.id.
-- Populated from Rust when headers arrive, updated when bodies are cached.
CREATE VIRTUAL TABLE messages_fts USING fts5(
  subject, from_text, to_text, body,
  content='', contentless_delete=1,
  tokenize='unicode61 remove_diacritics 2'
);
