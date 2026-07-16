-- List-Unsubscribe (RFC 2369) target + one-click flag (RFC 8058), captured at
-- sync time so the reading pane can offer a per-message unsubscribe action.
ALTER TABLE messages ADD COLUMN list_unsubscribe TEXT;
ALTER TABLE messages ADD COLUMN list_unsubscribe_one_click INTEGER NOT NULL DEFAULT 0;
