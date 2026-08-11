CREATE TABLE IF NOT EXISTS tags (
    id   TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS son_tags (
    son_id TEXT NOT NULL REFERENCES sons(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (son_id, tag_id)
);

CREATE INDEX IF NOT EXISTS idx_son_tags_tag ON son_tags (tag_id);

-- Real full-text search over titles, not a LIKE '%term%' scan. External-
-- content mode (content='sons') so the indexed text isn't duplicated in the
-- FTS table -- it stays a pointer into sons.title, kept in sync by the
-- triggers below. Scoped to title only for now; folding in tag names needs
-- triggers on son_tags too and is a reasonable follow-up, not a blocker here.
CREATE VIRTUAL TABLE IF NOT EXISTS sons_fts USING fts5(
    title,
    content = 'sons',
    content_rowid = 'rowid'
);

-- Keeps sons_fts in sync with sons.title. The 'delete' form (an FTS5 command,
-- not a plain DELETE) is what external-content mode requires for removing an
-- entry -- verified directly against D1 before relying on it here.
CREATE TRIGGER IF NOT EXISTS sons_fts_ai AFTER INSERT ON sons BEGIN
    INSERT INTO sons_fts (rowid, title) VALUES (new.rowid, new.title);
END;

CREATE TRIGGER IF NOT EXISTS sons_fts_ad AFTER DELETE ON sons BEGIN
    INSERT INTO sons_fts (sons_fts, rowid, title) VALUES ('delete', old.rowid, old.title);
END;

CREATE TRIGGER IF NOT EXISTS sons_fts_au AFTER UPDATE ON sons BEGIN
    INSERT INTO sons_fts (sons_fts, rowid, title) VALUES ('delete', old.rowid, old.title);
    INSERT INTO sons_fts (rowid, title) VALUES (new.rowid, new.title);
END;

-- Backfill: rows inserted before this migration existed have no FTS entry
-- yet, since the triggers above only fire on future writes.
INSERT INTO sons_fts (rowid, title) SELECT rowid, title FROM sons;
