-- Sender-authentication signals for phishing detection. Parsed from headers
-- already fetched at sync time (BODY.PEEK[HEADER]); NULL means "not seen",
-- which the heuristics treat as no signal, never as suspicious.
ALTER TABLE messages ADD COLUMN reply_to_addr TEXT;
ALTER TABLE messages ADD COLUMN auth_spf TEXT;
ALTER TABLE messages ADD COLUMN auth_dkim TEXT;
ALTER TABLE messages ADD COLUMN auth_dmarc TEXT;

-- First-contact and lookalike-domain checks aggregate over from_addr.
CREATE INDEX idx_messages_from_addr ON messages(from_addr);
