-- SHA-256 of the decoded pixel buffer, computed at upload time -- exact
-- duplicate detection (the same file, or the same image re-saved through a
-- different container, uploaded twice). NULL for anything inserted before
-- this migration existed; that's fine, exact-dedupe just has nothing to
-- compare those rows against.
ALTER TABLE sons ADD COLUMN content_hash TEXT;

CREATE INDEX IF NOT EXISTS idx_sons_content_hash ON sons (content_hash);
