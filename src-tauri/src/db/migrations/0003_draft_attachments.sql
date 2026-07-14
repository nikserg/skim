-- Files staged on an outgoing draft. Bytes live in the DB (not on disk or in
-- window state) because a draft is sent asynchronously by a background op long
-- after the compose window may have closed. Deleting the draft (on send or
-- discard) cascades these rows away.
CREATE TABLE draft_attachments (
  id         INTEGER PRIMARY KEY,
  draft_id   INTEGER NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,
  filename   TEXT NOT NULL,
  mime_type  TEXT NOT NULL,
  size       INTEGER NOT NULL,
  data       BLOB NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE INDEX idx_draft_attachments_draft ON draft_attachments(draft_id);
