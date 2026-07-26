-- The server's mailbox hierarchy delimiter, learned from the LIST response.
-- Needed only when creating a folder: a nested name typed as "Work/Taxes" has
-- to be sent with the server's own separator ("/" on Gmail, usually "." on
-- Dovecot), otherwise the server either rejects it or creates one flat mailbox
-- with a slash in its name.
--
-- NULL means "not learned yet" (no sync has run, or the server reported none);
-- callers fall back to "/".
ALTER TABLE accounts ADD COLUMN folder_delimiter TEXT;
