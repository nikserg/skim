-- The subject line is part of the message, so it is translated with the body and
-- shown above it. Stored on its own rather than spliced into `body_html`,
-- because the pane renders it as its own heading.
ALTER TABLE message_translations ADD COLUMN subject TEXT;

-- Drop what the earlier version cached. Those rows have no translated subject
-- and were cut off by a request budget that turned out to be three times
-- smaller than it needed to be, so serving them would keep showing a half-done
-- translation no re-run could ever replace. They are a local cache of AI output:
-- nothing is lost that pressing T does not make again.
DELETE FROM message_translations;
