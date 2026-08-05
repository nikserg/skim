-- AI translations of message bodies, one row per (message, target language).
--
-- The shape mirrors `message_bodies` on purpose: the render path can then swap
-- a translation in for the original and run the same sanitize / image-policy /
-- link-check pipeline over it, unchanged.
--
-- Cached so reopening a translated message never spends tokens again. The row's
-- existence is also what makes the pane come up translated, which is why
-- "show original" must never delete it. A local derived artifact, not a mailbox
-- mutation, so it stays out of `pending_ops`.
CREATE TABLE message_translations (
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  lang       TEXT    NOT NULL,
  body_html  TEXT,
  body_text  TEXT,
  -- The message was longer than one request's budget: everything past a point
  -- is still in the original language.
  truncated  INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (message_id, lang)
) WITHOUT ROWID;
