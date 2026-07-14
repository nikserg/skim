-- Cached STATUS snapshot per folder, used to skip the expensive SELECT + flag
-- fetch during a poll when a cheap STATUS probe proves nothing changed.
ALTER TABLE folders ADD COLUMN status_uidvalidity INTEGER;
ALTER TABLE folders ADD COLUMN status_uidnext INTEGER;
ALTER TABLE folders ADD COLUMN status_exists INTEGER;
ALTER TABLE folders ADD COLUMN status_unseen INTEGER;
