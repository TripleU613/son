-- Denormalised counter on the son itself. The gallery sorts and renders by this
-- on every page load, so counting rows in `likes` per card would be the hot
-- query. Kept in step with `likes` inside one transaction.
ALTER TABLE sons ADD COLUMN likes INTEGER NOT NULL DEFAULT 0;

-- One row per (son, voter). Voters are anonymous cookie IDs, never IP
-- addresses: no visitor IPs stored, and no collisions between everyone behind
-- one NAT. Trivially bypassable by clearing cookies, which is an acceptable
-- trade for meme likes.
CREATE TABLE IF NOT EXISTS likes (
    son_id     TEXT NOT NULL REFERENCES sons(id) ON DELETE CASCADE,
    voter_id   TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (son_id, voter_id)
);

-- "Which sons has this visitor already liked", for rendering button state.
CREATE INDEX IF NOT EXISTS idx_likes_voter ON likes (voter_id);

-- The "most liked" ordering, restricted to what is actually shown.
CREATE INDEX IF NOT EXISTS idx_sons_public_likes
    ON sons (is_public, likes DESC, created_at DESC);
