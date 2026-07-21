-- Unified inbox ("All inboxes") sorts messages and threads globally by date,
-- across folders and accounts; the existing indexes are folder/account-scoped.
CREATE INDEX idx_messages_date ON messages(date DESC);
CREATE INDEX idx_threads_last_date ON threads(last_date DESC);
