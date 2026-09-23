-- mail-parser was built without its CJK decoders, so GBK/Big5/Shift_JIS/EUC-KR
-- headers were stored as U+FFFD — and sync never rewrites a message it already
-- has. Make the affected folders look as if their UIDVALIDITY changed: the next
-- pass wipes them and fetches them again, this time decoded. -1 rather than
-- NULL, since a NULL validity is "never synced" and skips the wipe; the status
-- snapshot goes too, or the unchanged-folder shortcut would skip the pass.
UPDATE folders
   SET uidvalidity = -1, status_uidvalidity = NULL
 WHERE id IN (SELECT DISTINCT folder_id FROM messages
               WHERE instr(subject, char(65533)) > 0
                  OR instr(from_name, char(65533)) > 0
                  OR instr(snippet, char(65533)) > 0);
