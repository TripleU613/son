-- Search that actually finds things.
--
-- The previous index (sons_fts, migration 0005) tokenised titles into whole
-- words, which made three common searches return nothing at all:
--
--   "cap"    -> no match, because "capri" is one token and there is no prefix
--               matching, so nothing appears until you finish typing a token
--   "flower" -> no match for "Sonflower", because a token only matches from
--               its start
--   "logic"  -> no match, because tag names were never indexed (0005 says so
--               in its own comment and defers it)
--
-- The middle one matters most on this site specifically: the entire joke is
-- words with "son" buried inside them, so the substring nobody can search for
-- is exactly the substring everybody will type.
--
-- The trigram tokeniser fixes all three. It indexes every 3-character run, so
-- a query matches anywhere inside the text, prefix or infix, case-insensitively.
-- Verified against this D1 database before writing this migration: "flower"
-- finds Sonflower, "apri" finds Capri-Son, "cap" finds Capri-Son, and an OR of
-- a misspelling's trigrams ranks the right son far above the noise
-- (sonflwer -> Sonflower at bm25 -1.43, everything else at -1e-6), which is
-- what search_sons() uses as its fuzzy fallback.
--
-- Two deliberate differences from 0005:
--
-- 1. This is a regular FTS5 table, not external-content (content='sons').
--    External content mode maps each FTS column onto a column of one table, and
--    tag names live in son_tags/tags behind a join, so they cannot be projected
--    that way. A regular table stores its own copy; titles are capped at 80
--    characters and tags are short, so the duplication is negligible. It also
--    means a plain DELETE works here, instead of the 'delete' command form that
--    external-content mode requires.
--
-- 2. sons_fts is left in place, unused. Search moves over by code change alone,
--    so rolling back is reverting a commit rather than restoring an index.
CREATE VIRTUAL TABLE IF NOT EXISTS sons_search USING fts5(
    title,
    tags,
    tokenize = 'trigram'
);

-- rowid is kept equal to sons.rowid so results join straight back to the sons
-- table, the same way the old index did.
CREATE TRIGGER IF NOT EXISTS sons_search_ai AFTER INSERT ON sons BEGIN
    INSERT INTO sons_search (rowid, title, tags) VALUES (new.rowid, new.title, '');
END;

CREATE TRIGGER IF NOT EXISTS sons_search_ad AFTER DELETE ON sons BEGIN
    DELETE FROM sons_search WHERE rowid = old.rowid;
END;

-- Re-reads the tags on update rather than blanking them: a title edit must not
-- silently drop a son out of tag search.
CREATE TRIGGER IF NOT EXISTS sons_search_au AFTER UPDATE ON sons BEGIN
    DELETE FROM sons_search WHERE rowid = old.rowid;
    INSERT INTO sons_search (rowid, title, tags)
    SELECT new.rowid,
           new.title,
           COALESCE((SELECT group_concat(t.name, ' ')
                     FROM son_tags st JOIN tags t ON t.id = st.tag_id
                     WHERE st.son_id = new.id), '');
END;

-- Tags are attached after the son row exists, so the son's indexed tag text has
-- to be rebuilt whenever the join table changes. Full recompute rather than
-- appending one name: removing a tag has to shrink the text too.
CREATE TRIGGER IF NOT EXISTS son_tags_search_ai AFTER INSERT ON son_tags BEGIN
    DELETE FROM sons_search WHERE rowid = (SELECT rowid FROM sons WHERE id = new.son_id);
    INSERT INTO sons_search (rowid, title, tags)
    SELECT s.rowid,
           s.title,
           COALESCE((SELECT group_concat(t.name, ' ')
                     FROM son_tags st JOIN tags t ON t.id = st.tag_id
                     WHERE st.son_id = s.id), '')
    FROM sons s WHERE s.id = new.son_id;
END;

CREATE TRIGGER IF NOT EXISTS son_tags_search_ad AFTER DELETE ON son_tags BEGIN
    DELETE FROM sons_search WHERE rowid = (SELECT rowid FROM sons WHERE id = old.son_id);
    INSERT INTO sons_search (rowid, title, tags)
    SELECT s.rowid,
           s.title,
           COALESCE((SELECT group_concat(t.name, ' ')
                     FROM son_tags st JOIN tags t ON t.id = st.tag_id
                     WHERE st.son_id = s.id), '')
    FROM sons s WHERE s.id = old.son_id;
END;

-- Backfill every existing son, titles and tags together. DELETE first so this
-- migration is safe to re-run against a database that already has the table.
DELETE FROM sons_search;

INSERT INTO sons_search (rowid, title, tags)
SELECT s.rowid,
       s.title,
       COALESCE((SELECT group_concat(t.name, ' ')
                 FROM son_tags st JOIN tags t ON t.id = st.tag_id
                 WHERE st.son_id = s.id), '')
FROM sons s;
