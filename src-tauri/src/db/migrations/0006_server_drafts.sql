-- Link a local draft to the server (IMAP Drafts folder) message it mirrors, so
-- edits to a synced draft round-trip back to the Drafts folder. `origin_message_id`
-- is the current local `messages.id` the draft was opened from; `imap_message_id`
-- is the stable RFC822 Message-ID (without angle brackets) that identifies the
-- server copy across resyncs. Both NULL for ordinary local-only drafts.
ALTER TABLE drafts ADD COLUMN origin_message_id INTEGER;
ALTER TABLE drafts ADD COLUMN imap_message_id TEXT;
