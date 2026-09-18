-- IMAP/SMTP login when it differs from the email. NULL means use the email.
ALTER TABLE accounts ADD COLUMN imap_user TEXT;
