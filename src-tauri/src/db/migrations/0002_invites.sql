-- The user's own RSVP to a calendar invitation, keyed by event UID so
-- Gmail label-duplicates and re-sent updates of the same event share it.
CREATE TABLE invite_rsvps (
  account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  event_uid    TEXT NOT NULL,
  partstat     TEXT NOT NULL, -- 'ACCEPTED' | 'DECLINED' | 'TENTATIVE'
  sequence     INTEGER NOT NULL DEFAULT 0,
  responded_at INTEGER NOT NULL,
  PRIMARY KEY (account_id, event_uid)
);
