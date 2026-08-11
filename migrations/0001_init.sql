CREATE TABLE IF NOT EXISTS sons (
    id          TEXT    PRIMARY KEY NOT NULL,
    title       TEXT    NOT NULL,
    orig_url    TEXT    NOT NULL,
    thumb_url   TEXT    NOT NULL,
    width       INTEGER NOT NULL,
    height      INTEGER NOT NULL,
    son_score   REAL    NOT NULL,
    nsfw_score  REAL    NOT NULL,
    -- CLIP image embedding as raw little-endian f32s. Stored from day one even
    -- though nothing reads it yet: it is the dataset for dedupe, similarity,
    -- and a future generator, and it cannot be backfilled.
    embedding   BLOB,
    created_at  TEXT    NOT NULL,
    -- Auto-publish means this starts at 1. A bad upload is one UPDATE from gone.
    is_public   INTEGER NOT NULL DEFAULT 1,
    reports     INTEGER NOT NULL DEFAULT 0
);

-- The gallery's only hot query: public sons, newest first, keyset-paginated.
CREATE INDEX IF NOT EXISTS idx_sons_public_created
    ON sons (is_public, created_at DESC);

-- Surfacing what needs a look, since nothing is held for review up front.
CREATE INDEX IF NOT EXISTS idx_sons_reports
    ON sons (reports DESC) WHERE reports > 0;
