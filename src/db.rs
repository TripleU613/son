//! Data access, over Cloudflare D1 (see `d1.rs` for the transport and what its
//! HTTP API does and does not make atomic).
//!
//! There is no local database at all — this app runs on a server explicitly
//! provisioned with no persistent local storage, so every read and write is a
//! network call to D1. Local dev talks to the same D1 database as production;
//! that is a deliberate simplification (see README) to avoid two divergent
//! code paths — a SQLite-shaped dev path and a D1-shaped prod path — that could
//! silently behave differently at exactly the places that matter most
//! (`toggle_like`'s lack of cross-call transactions, in particular).

use std::collections::HashSet;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::d1::D1;
use crate::models::{LeaderboardEntry, Son, SonPage, Sort, Uploader, User, PAGE_SIZE};

/// Process-wide client, set once at startup.
static D1_CLIENT: std::sync::OnceLock<D1> = std::sync::OnceLock::new();

pub fn set_client(d1: D1) {
    let _ = D1_CLIENT.set(d1);
}

/// Panics if called before `set_client`, which can only happen if a handler
/// runs before startup finished — not reachable in practice.
pub fn client() -> &'static D1 {
    D1_CLIENT.get().expect("D1 client not initialized")
}

// A LEFT JOIN against users, not a separate per-row query: this is the hot
// path (every gallery page, every detail view), and joining costs nothing D1
// doesn't already pay for a single indexed lookup.
const SON_SELECT: &str = "SELECT s.id, s.title, s.orig_url, s.thumb_url, s.width, s.height, \
     s.son_score, s.nsfw_score, s.created_at, s.is_public, s.reports, s.likes, \
     u.display_name AS uploader_name, u.avatar_url AS uploader_avatar \
     FROM sons s LEFT JOIN users u ON u.id = s.uploader_id";

/// Shape of a row as D1 returns it: JSON, with SQLite's native types (no bool,
/// no fixed-width ints/floats). Converted into `Son` below.
#[derive(Deserialize)]
struct SonRow {
    id: String,
    title: String,
    orig_url: String,
    thumb_url: String,
    width: i64,
    height: i64,
    son_score: f64,
    nsfw_score: f64,
    created_at: String,
    is_public: i64,
    reports: i64,
    likes: i64,
    uploader_name: Option<String>,
    uploader_avatar: Option<String>,
}

impl From<SonRow> for Son {
    fn from(r: SonRow) -> Self {
        Son {
            id: r.id,
            title: r.title,
            orig_url: r.orig_url,
            thumb_url: r.thumb_url,
            width: r.width as u32,
            height: r.height as u32,
            son_score: r.son_score as f32,
            nsfw_score: r.nsfw_score as f32,
            created_at: r.created_at,
            is_public: r.is_public != 0,
            reports: r.reports,
            likes: r.likes,
            // Both depend on who is asking / a follow-up query; filled in by
            // the caller (mark_liked, attach_tags).
            liked_by_me: false,
            tags: Vec::new(),
            uploader: r.uploader_name.map(|display_name| Uploader {
                display_name,
                avatar_url: r.uploader_avatar,
            }),
        }
    }
}

/// Cursors are opaque to the client but encode the full sort key, because
/// keyset pagination needs a total order. `likes`/`son_score` alone are not
/// one — many sons share a value — so those cursors carry a tie-breaker too.
/// Titles can't contain control characters (`upload_route::clean_title`
/// strips them), so NUL is a safe separator there even though `|` could
/// theoretically appear in free-text titles.
fn encode_cursor(s: &Son, sort: Sort) -> String {
    match sort {
        Sort::Newest => s.created_at.clone(),
        Sort::MostLiked => format!("{}|{}", s.likes, s.created_at),
        Sort::Az => format!("{}\u{0}{}", s.title, s.id),
        Sort::SonScore => format!("{}|{}", s.son_score, s.id),
    }
}

fn decode_liked_cursor(cursor: &str) -> (i64, String) {
    match cursor.split_once('|') {
        Some((likes, created)) => (likes.parse().unwrap_or(i64::MAX), created.to_string()),
        // Malformed cursor: start from the top rather than erroring at the
        // user, since a cursor only ever comes from us.
        None => (i64::MAX, String::new()),
    }
}

fn decode_az_cursor(cursor: &str) -> (String, String) {
    match cursor.split_once('\u{0}') {
        Some((title, id)) => (title.to_string(), id.to_string()),
        None => (String::new(), String::new()),
    }
}

fn decode_sonscore_cursor(cursor: &str) -> (f64, String) {
    match cursor.split_once('|') {
        Some((score, id)) => (score.parse().unwrap_or(f64::MAX), id.to_string()),
        None => (f64::MAX, String::new()),
    }
}

/// One page of public sons.
pub async fn list_public(
    cursor: Option<&str>,
    sort: Sort,
    voter: Option<&str>,
) -> anyhow::Result<SonPage> {
    let rows: Vec<SonRow> = match sort {
        Sort::Newest => {
            let sql = format!(
                "{SON_SELECT} \
                 WHERE s.is_public = 1 AND (?1 IS NULL OR s.created_at < ?1) \
                 ORDER BY s.created_at DESC, s.id DESC \
                 LIMIT ?2"
            );
            client()
                .query(&sql, vec![json!(cursor), json!(PAGE_SIZE + 1)])
                .await?
        }
        Sort::MostLiked => {
            let (likes, created) = cursor
                .map(decode_liked_cursor)
                .unwrap_or((i64::MAX, String::new()));
            let sql = format!(
                "{SON_SELECT} \
                 WHERE s.is_public = 1 \
                   AND (?1 = 0 OR s.likes < ?2 OR (s.likes = ?2 AND s.created_at < ?3)) \
                 ORDER BY s.likes DESC, s.created_at DESC, s.id DESC \
                 LIMIT ?4"
            );
            client()
                .query(
                    &sql,
                    vec![
                        json!(i64::from(cursor.is_some())),
                        json!(likes),
                        json!(created),
                        json!(PAGE_SIZE + 1),
                    ],
                )
                .await?
        }
        Sort::Az => {
            let (title, id) = cursor.map(decode_az_cursor).unwrap_or_default();
            let sql = format!(
                "{SON_SELECT} \
                 WHERE s.is_public = 1 \
                   AND (?1 = 0 OR s.title COLLATE NOCASE > ?2 \
                        OR (s.title COLLATE NOCASE = ?2 AND s.id > ?3)) \
                 ORDER BY s.title COLLATE NOCASE ASC, s.id ASC \
                 LIMIT ?4"
            );
            client()
                .query(
                    &sql,
                    vec![
                        json!(i64::from(cursor.is_some())),
                        json!(title),
                        json!(id),
                        json!(PAGE_SIZE + 1),
                    ],
                )
                .await?
        }
        Sort::SonScore => {
            let (score, id) = cursor
                .map(decode_sonscore_cursor)
                .unwrap_or((f64::MAX, String::new()));
            let sql = format!(
                "{SON_SELECT} \
                 WHERE s.is_public = 1 \
                   AND (?1 = 0 OR s.son_score < ?2 OR (s.son_score = ?2 AND s.id < ?3)) \
                 ORDER BY s.son_score DESC, s.id DESC \
                 LIMIT ?4"
            );
            client()
                .query(
                    &sql,
                    vec![
                        json!(i64::from(cursor.is_some())),
                        json!(score),
                        json!(id),
                        json!(PAGE_SIZE + 1),
                    ],
                )
                .await?
        }
    };

    let has_more = rows.len() as i64 > PAGE_SIZE;
    let mut sons: Vec<Son> = rows
        .into_iter()
        .take(PAGE_SIZE as usize)
        .map(Son::from)
        .collect();

    let next_cursor = if has_more {
        sons.last().map(|s| encode_cursor(s, sort))
    } else {
        None
    };

    mark_liked(&mut sons, voter).await?;
    attach_tags_to(&mut sons).await?;
    Ok(SonPage { sons, next_cursor })
}

/// Fill in `liked_by_me` for a page in one query rather than one per card.
async fn mark_liked(sons: &mut [Son], voter: Option<&str>) -> anyhow::Result<()> {
    let Some(voter) = voter else { return Ok(()) };
    if sons.is_empty() {
        return Ok(());
    }

    let holes = (0..sons.len())
        .map(|i| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT son_id FROM likes WHERE voter_id = ?1 AND son_id IN ({holes})");

    let mut params = vec![json!(voter)];
    params.extend(sons.iter().map(|s| json!(s.id)));

    #[derive(Deserialize)]
    struct LikedRow {
        son_id: String,
    }
    let liked: HashSet<String> = client()
        .query::<LikedRow>(&sql, params)
        .await?
        .into_iter()
        .map(|r| r.son_id)
        .collect();

    for s in sons.iter_mut() {
        s.liked_by_me = liked.contains(&s.id);
    }
    Ok(())
}

pub async fn get(id: &str, voter: Option<&str>) -> anyhow::Result<Option<Son>> {
    let sql = format!("{SON_SELECT} WHERE s.id = ?1");
    let rows: Vec<SonRow> = client().query(&sql, vec![json!(id)]).await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let mut son = Son::from(row);

    if let Some(voter) = voter {
        #[derive(Deserialize)]
        struct Hit {
            #[allow(dead_code)]
            x: i64,
        }
        let hit: Vec<Hit> = client()
            .query(
                "SELECT 1 AS x FROM likes WHERE son_id = ?1 AND voter_id = ?2",
                vec![json!(id), json!(voter)],
            )
            .await?;
        son.liked_by_me = !hit.is_empty();
    }
    let mut one = [son];
    attach_tags_to(&mut one).await?;
    let [son] = one;
    Ok(Some(son))
}

/// Exact-duplicate lookup by content hash, across every son regardless of
/// `is_public` -- a hidden son re-uploaded byte-for-identical is still the
/// same file, and catching that closes an easy moderation-evasion loop
/// (hide it, then just upload it again).
pub async fn find_by_hash(hash: &str) -> anyhow::Result<Option<Son>> {
    let sql = format!("{SON_SELECT} WHERE s.content_hash = ?1 LIMIT 1");
    let rows: Vec<SonRow> = client().query(&sql, vec![json!(hash)]).await?;
    Ok(rows.into_iter().next().map(Son::from))
}

/// Every son's CLIP embedding, for the near-duplicate scan in `dedupe`. Rows
/// with no embedding (moderation backend was `stub`/`deny`, or the row
/// predates embeddings existing at all) are skipped, not returned as an
/// empty vector -- an empty embedding would otherwise compare as
/// "maximally dissimilar" to everything, which is a misleading answer for
/// "we don't actually know."
pub async fn all_embeddings() -> anyhow::Result<Vec<(String, Vec<f32>)>> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        embedding: Option<Vec<u8>>,
    }
    let rows: Vec<Row> = client()
        .query(
            "SELECT id, embedding FROM sons WHERE embedding IS NOT NULL",
            vec![],
        )
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let bytes = r.embedding?;
            let floats = bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            Some((r.id, floats))
        })
        .collect())
}

pub struct NewSon<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub orig_url: &'a str,
    pub thumb_url: &'a str,
    pub width: u32,
    pub height: u32,
    pub son_score: f32,
    pub nsfw_score: f32,
    pub embedding: Option<&'a [f32]>,
    /// SHA-256 of the decoded pixel buffer, for exact-duplicate detection.
    /// Always computed for a new upload; only rows inserted before this
    /// column existed lack one.
    pub content_hash: &'a str,
    /// Who uploaded this, if they were signed in. `None` for an anonymous
    /// upload — still fully supported; accounts are additive, not a gate.
    pub uploader_id: Option<&'a str>,
    /// Display info for the same uploader, so the row handed straight back to
    /// the caller after insert already shows attribution correctly, without
    /// an extra round trip to re-fetch what the caller already had in hand.
    pub uploader: Option<Uploader>,
    /// Attached by the caller after linking `son_tags`, for the same reason
    /// as `uploader`: the immediate response should already show what was
    /// just attached, not wait for the next page load.
    pub tags: Vec<crate::models::Tag>,
}

pub async fn insert(new: NewSon<'_>) -> anyhow::Result<Son> {
    let created_at = chrono::Utc::now().to_rfc3339();
    // D1 has no native blob param type; a BLOB column round-trips as a plain
    // JSON array of byte values, verified directly against the API before
    // relying on it here.
    let blob: Value = match new.embedding {
        Some(e) => json!(e.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()),
        None => Value::Null,
    };

    client()
        .exec(
            "INSERT INTO sons (id, title, orig_url, thumb_url, width, height, \
                               son_score, nsfw_score, embedding, created_at, is_public, reports, \
                               uploader_id, content_hash) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,0,?11,?12)",
            vec![
                json!(new.id),
                json!(new.title),
                json!(new.orig_url),
                json!(new.thumb_url),
                json!(new.width),
                json!(new.height),
                json!(new.son_score),
                json!(new.nsfw_score),
                blob,
                json!(created_at),
                json!(new.uploader_id),
                json!(new.content_hash),
            ],
        )
        .await?;

    Ok(Son {
        id: new.id.to_string(),
        title: new.title.to_string(),
        orig_url: new.orig_url.to_string(),
        thumb_url: new.thumb_url.to_string(),
        width: new.width,
        height: new.height,
        son_score: new.son_score,
        nsfw_score: new.nsfw_score,
        created_at,
        is_public: true,
        reports: 0,
        likes: 0,
        liked_by_me: false,
        uploader: new.uploader,
        tags: new.tags,
    })
}

/// Record a report and recompute `sons.reports` from `COUNT(*)`, the same
/// self-healing pattern as `sons.likes` — a drift-proof counter beats an
/// incremented one given D1's lack of cross-call transactions (see `d1.rs`).
///
/// One report per voter per son (`reports`'s primary key): a repeat report
/// from the same voter is silently a no-op, so a single visitor cannot force
/// auto-hide alone by resubmitting.
pub async fn report(
    son_id: &str,
    voter: &str,
    reason: &str,
    message: Option<&str>,
) -> anyhow::Result<()> {
    client()
        .exec(
            "INSERT INTO reports (son_id, voter_id, reason, message, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (son_id, voter_id) DO NOTHING",
            vec![
                json!(son_id),
                json!(voter),
                json!(reason),
                json!(message),
                json!(chrono::Utc::now().to_rfc3339()),
            ],
        )
        .await?;

    client()
        .exec(
            "UPDATE sons \
             SET reports = (SELECT COUNT(*) FROM reports WHERE son_id = sons.id), \
                 is_public = CASE \
                     WHEN (SELECT COUNT(*) FROM reports WHERE son_id = sons.id) >= ?2 \
                     THEN 0 ELSE is_public \
                 END \
             WHERE id = ?1",
            vec![json!(son_id), json!(AUTO_HIDE_REPORTS)],
        )
        .await?;
    Ok(())
}

/// Flag a son. At `AUTO_HIDE_REPORTS` it pulls itself from the gallery — the
/// safety valve that makes auto-publishing survivable without a review queue.
pub const AUTO_HIDE_REPORTS: i64 = 3;

pub async fn set_public(id: &str, public: bool) -> anyhow::Result<()> {
    client()
        .exec(
            "UPDATE sons SET is_public = ?2 WHERE id = ?1",
            vec![json!(id), json!(i64::from(public))],
        )
        .await?;
    Ok(())
}

/// Delete a son's row and its reports/likes (both `ON DELETE CASCADE`, though
/// D1's HTTP API executes each statement without a guarantee that
/// `PRAGMA foreign_keys` is on for it, so the cascades are not solely relied
/// on — both child tables are cleared explicitly first).
pub async fn delete_son(id: &str) -> anyhow::Result<()> {
    client()
        .exec("DELETE FROM reports WHERE son_id = ?1", vec![json!(id)])
        .await?;
    client()
        .exec("DELETE FROM likes WHERE son_id = ?1", vec![json!(id)])
        .await?;
    client()
        .exec("DELETE FROM sons WHERE id = ?1", vec![json!(id)])
        .await?;
    Ok(())
}

/// Every son with at least one report, each with its full report history —
/// the admin queue's unit of review. Two queries, not N+1: one for the sons,
/// one for every report against any of them.
pub async fn flagged_sons() -> anyhow::Result<Vec<crate::models::FlaggedSon>> {
    use crate::models::{FlaggedSon, ReportDetail};

    let sql =
        format!("{SON_SELECT} WHERE s.reports > 0 ORDER BY s.reports DESC, s.created_at DESC");
    let sons: Vec<Son> = client()
        .query::<SonRow>(&sql, vec![])
        .await?
        .into_iter()
        .map(Son::from)
        .collect();
    if sons.is_empty() {
        return Ok(vec![]);
    }

    let holes = (0..sons.len())
        .map(|i| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(",");
    let sql2 =
        format!("SELECT son_id, reason, message, created_at FROM reports WHERE son_id IN ({holes}) ORDER BY created_at DESC");

    #[derive(Deserialize)]
    struct Row {
        son_id: String,
        reason: String,
        message: Option<String>,
        created_at: String,
    }
    let rows: Vec<Row> = client()
        .query(&sql2, sons.iter().map(|s| json!(s.id)).collect())
        .await?;

    let mut by_son: std::collections::HashMap<String, Vec<ReportDetail>> =
        std::collections::HashMap::new();
    for r in rows {
        by_son.entry(r.son_id).or_default().push(ReportDetail {
            reason: r.reason,
            message: r.message,
            created_at: r.created_at,
        });
    }

    Ok(sons
        .into_iter()
        .map(|son| {
            let reports = by_son.remove(&son.id).unwrap_or_default();
            FlaggedSon { son, reports }
        })
        .collect())
}

pub async fn count_public() -> anyhow::Result<i64> {
    #[derive(Deserialize)]
    struct Count {
        n: i64,
    }
    let rows: Vec<Count> = client()
        .query("SELECT COUNT(*) AS n FROM sons WHERE is_public = 1", vec![])
        .await?;
    Ok(rows.first().map(|r| r.n).unwrap_or(0))
}

/// Toggle a like and return `(new_count, liked_now)`.
///
/// D1 gives no transaction spanning separate HTTP calls (see `d1.rs`), so this
/// cannot be "read whether liked, then branch" the way a local sqlx
/// transaction could. Instead:
///
/// 1. Try to insert the like row, ignoring a conflict, and use `RETURNING` to
///    learn — atomically, in this one statement — whether the insert actually
///    happened. This *is* the read: no separate SELECT needed to know the
///    prior state, so nothing can change between checking and acting.
/// 2. If it didn't happen (the row already existed), delete it instead — the
///    unlike path. If a concurrent request already deleted it in the gap
///    between these two calls, this is a no-op, which is fine: the row is
///    gone either way and that is the outcome we want.
/// 3. Recompute `sons.likes` from `COUNT(*)` on the source-of-truth `likes`
///    table, rather than incrementing/decrementing a counter. This makes the
///    counter self-healing: any drift from a request that failed partway
///    through gets corrected by the next successful toggle, rather than
///    compounding.
pub async fn toggle_like(son_id: &str, voter: &str) -> anyhow::Result<(i64, bool)> {
    #[derive(Deserialize)]
    struct SonIdRow {
        #[allow(dead_code)]
        son_id: String,
    }

    let inserted: Vec<SonIdRow> = client()
        .query(
            "INSERT INTO likes (son_id, voter_id, created_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (son_id, voter_id) DO NOTHING \
             RETURNING son_id",
            vec![
                json!(son_id),
                json!(voter),
                json!(chrono::Utc::now().to_rfc3339()),
            ],
        )
        .await?;

    let now_liked = if inserted.is_empty() {
        client()
            .exec(
                "DELETE FROM likes WHERE son_id = ?1 AND voter_id = ?2",
                vec![json!(son_id), json!(voter)],
            )
            .await?;
        false
    } else {
        true
    };

    #[derive(Deserialize)]
    struct Likes {
        likes: i64,
    }
    let recomputed: Vec<Likes> = client()
        .query(
            "UPDATE sons SET likes = (SELECT COUNT(*) FROM likes WHERE son_id = sons.id) \
             WHERE id = ?1 \
             RETURNING likes",
            vec![json!(son_id)],
        )
        .await?;

    let count = recomputed.first().map(|r| r.likes).unwrap_or(0);
    Ok((count, now_liked))
}

const USER_COLS: &str = "id, email, display_name, avatar_url, is_admin";

#[derive(Deserialize)]
struct UserRow {
    id: String,
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    is_admin: i64,
}

impl From<UserRow> for User {
    fn from(r: UserRow) -> Self {
        // email intentionally dropped: User is the client-facing shape (see
        // its doc comment), and email has no reason to leave the server.
        let _ = r.email;
        User {
            id: r.id,
            display_name: r.display_name,
            avatar_url: r.avatar_url,
            is_admin: r.is_admin != 0,
        }
    }
}

pub async fn get_user(id: &str) -> anyhow::Result<Option<User>> {
    let sql = format!("SELECT {USER_COLS} FROM users WHERE id = ?1");
    let rows: Vec<UserRow> = client().query(&sql, vec![json!(id)]).await?;
    Ok(rows.into_iter().next().map(User::from))
}

/// Create or update a user from their Google profile, keyed on `google_sub`
/// (Google's stable subject id — the only field from a Google profile that is
/// guaranteed never to change or be reused for a different person).
///
/// `is_admin` is untouched on conflict: it starts at 0 for a new row, and an
/// existing admin does not get silently reset just because they logged in
/// again with a since-changed display name or photo.
pub async fn upsert_user(
    google_sub: &str,
    email: &str,
    display_name: &str,
    avatar_url: Option<&str>,
) -> anyhow::Result<User> {
    let id = uuid::Uuid::new_v4().to_string();
    let sql = format!(
        "INSERT INTO users (id, google_sub, email, display_name, avatar_url, is_admin, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6) \
         ON CONFLICT (google_sub) DO UPDATE SET \
             email = excluded.email, \
             display_name = excluded.display_name, \
             avatar_url = excluded.avatar_url \
         RETURNING {USER_COLS}"
    );
    let rows: Vec<UserRow> = client()
        .query(
            &sql,
            vec![
                json!(id),
                json!(google_sub),
                json!(email),
                json!(display_name),
                json!(avatar_url),
                json!(chrono::Utc::now().to_rfc3339()),
            ],
        )
        .await?;

    rows.into_iter()
        .next()
        .map(User::from)
        .ok_or_else(|| anyhow::anyhow!("upsert_user: RETURNING produced no row"))
}

/// Ranked by upload count among accounts with at least one public upload —
/// an account that has never uploaded doesn't clutter a leaderboard entry at
/// zero.
pub async fn leaderboard(limit: i64) -> anyhow::Result<Vec<LeaderboardEntry>> {
    #[derive(Deserialize)]
    struct Row {
        display_name: String,
        avatar_url: Option<String>,
        upload_count: i64,
    }
    let rows: Vec<Row> = client()
        .query(
            "SELECT u.display_name, u.avatar_url, COUNT(*) AS upload_count \
             FROM sons s JOIN users u ON u.id = s.uploader_id \
             WHERE s.is_public = 1 \
             GROUP BY u.id \
             ORDER BY upload_count DESC, u.display_name ASC \
             LIMIT ?1",
            vec![json!(limit)],
        )
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| LeaderboardEntry {
            display_name: r.display_name,
            avatar_url: r.avatar_url,
            upload_count: r.upload_count,
        })
        .collect())
}

/// The most-liked son uploaded in the last 24 hours, falling back to the
/// most-liked public son overall when nothing new has landed recently — a
/// quiet day shouldn't mean the homepage's featured slot goes empty.
pub async fn son_of_the_day() -> anyhow::Result<Option<Son>> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();

    let recent_sql = format!(
        "{SON_SELECT} \
         WHERE s.is_public = 1 AND s.created_at >= ?1 \
         ORDER BY s.likes DESC, s.created_at DESC \
         LIMIT 1"
    );
    let recent: Vec<SonRow> = client().query(&recent_sql, vec![json!(cutoff)]).await?;
    if let Some(row) = recent.into_iter().next() {
        return Ok(Some(Son::from(row)));
    }

    let fallback_sql = format!(
        "{SON_SELECT} WHERE s.is_public = 1 ORDER BY s.likes DESC, s.created_at DESC LIMIT 1"
    );
    let fallback: Vec<SonRow> = client().query(&fallback_sql, vec![]).await?;
    Ok(fallback.into_iter().next().map(Son::from))
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = true; // swallow a leading dash
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = slug.trim_end_matches('-');
    if trimmed.is_empty() {
        "tag".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Free-text tag names (as typed on the upload form) → upserted `tags` rows,
/// linked to `son_id` via `son_tags`. Idempotent: re-uploading with the same
/// tag name reuses the existing row rather than erroring on the UNIQUE name.
pub async fn attach_tags(
    son_id: &str,
    names: &[String],
) -> anyhow::Result<Vec<crate::models::Tag>> {
    use crate::models::Tag;

    let mut tags = Vec::with_capacity(names.len());
    for raw in names {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        let slug = slugify(name);
        let id = uuid::Uuid::new_v4().to_string();

        // Upsert by name; on conflict, this is a no-op that still lets the
        // RETURNING clause hand back the existing row's real slug (which may
        // differ from a fresh slugify of `name` if capitalization varies).
        #[derive(Deserialize)]
        struct Row {
            name: String,
            slug: String,
        }
        let rows: Vec<Row> = client()
            .query(
                "INSERT INTO tags (id, name, slug) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (name) DO UPDATE SET name = excluded.name \
                 RETURNING name, slug",
                vec![json!(id), json!(name), json!(slug)],
            )
            .await?;
        let Some(row) = rows.into_iter().next() else {
            continue;
        };

        client()
            .exec(
                "INSERT INTO son_tags (son_id, tag_id) \
                 SELECT ?1, id FROM tags WHERE name = ?2 \
                 ON CONFLICT (son_id, tag_id) DO NOTHING",
                vec![json!(son_id), json!(name)],
            )
            .await?;

        tags.push(Tag {
            name: row.name,
            slug: row.slug,
        });
    }
    Ok(tags)
}

/// Batch-attach tags to a page of sons in one query, the same pattern as
/// `mark_liked` — not a query per card.
async fn attach_tags_to(sons: &mut [Son]) -> anyhow::Result<()> {
    use crate::models::Tag;

    if sons.is_empty() {
        return Ok(());
    }
    let holes = (0..sons.len())
        .map(|i| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT st.son_id, t.name, t.slug FROM son_tags st \
         JOIN tags t ON t.id = st.tag_id \
         WHERE st.son_id IN ({holes}) \
         ORDER BY t.name"
    );

    #[derive(Deserialize)]
    struct Row {
        son_id: String,
        name: String,
        slug: String,
    }
    let rows: Vec<Row> = client()
        .query(&sql, sons.iter().map(|s| json!(s.id)).collect())
        .await?;

    let mut by_son: std::collections::HashMap<String, Vec<Tag>> = std::collections::HashMap::new();
    for r in rows {
        by_son.entry(r.son_id).or_default().push(Tag {
            name: r.name,
            slug: r.slug,
        });
    }
    for s in sons.iter_mut() {
        if let Some(tags) = by_son.remove(&s.id) {
            s.tags = tags;
        }
    }
    Ok(())
}

/// A public gallery page filtered to one tag, newest first. Its own function
/// rather than a third `Sort` variant: a tag filter composes with sort order
/// conceptually, but that's more than this MVP needs — newest-within-tag
/// covers the actual use case (browse everything under a tag).
pub async fn sons_by_tag(
    slug: &str,
    cursor: Option<&str>,
    voter: Option<&str>,
) -> anyhow::Result<SonPage> {
    let sql = format!(
        "{SON_SELECT} \
         JOIN son_tags st ON st.son_id = s.id \
         JOIN tags t ON t.id = st.tag_id \
         WHERE s.is_public = 1 AND t.slug = ?1 AND (?2 IS NULL OR s.created_at < ?2) \
         ORDER BY s.created_at DESC, s.id DESC \
         LIMIT ?3"
    );
    let rows: Vec<SonRow> = client()
        .query(&sql, vec![json!(slug), json!(cursor), json!(PAGE_SIZE + 1)])
        .await?;

    let has_more = rows.len() as i64 > PAGE_SIZE;
    let mut sons: Vec<Son> = rows
        .into_iter()
        .take(PAGE_SIZE as usize)
        .map(Son::from)
        .collect();
    let next_cursor = if has_more {
        sons.last().map(|s| s.created_at.clone())
    } else {
        None
    };

    mark_liked(&mut sons, voter).await?;
    attach_tags_to(&mut sons).await?;
    Ok(SonPage { sons, next_cursor })
}

/// Full-text search over titles via the `sons_fts` FTS5 index (see the
/// migration for why external-content mode and its sync triggers), not a
/// `LIKE '%term%'` scan.
pub async fn search_sons(query: &str, voter: Option<&str>) -> anyhow::Result<Vec<Son>> {
    let sql = format!(
        "{SON_SELECT} \
         JOIN sons_fts ON sons_fts.rowid = s.rowid \
         WHERE s.is_public = 1 AND sons_fts MATCH ?1 \
         ORDER BY rank \
         LIMIT ?2"
    );
    // FTS5's query syntax treats several characters as operators (", *, -,
    // etc.); a search box is free text, not a query language, so each term is
    // quoted and the quote-escaped, then joined as an implicit AND -- this
    // turns "what the" into a phrase search for both words present, never a
    // syntax error from a stray character a visitor typed.
    let escaped = query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ");
    if escaped.is_empty() {
        return Ok(vec![]);
    }

    let rows: Vec<SonRow> = client()
        .query(&sql, vec![json!(escaped), json!(PAGE_SIZE)])
        .await?;
    let mut sons: Vec<Son> = rows.into_iter().map(Son::from).collect();
    mark_liked(&mut sons, voter).await?;
    attach_tags_to(&mut sons).await?;
    Ok(sons)
}

/// A lightweight row for `sitemap.xml` -- only what the sitemap's image
/// extension needs, not a full `Son` (no likes/tags/uploader join).
pub struct SitemapSon {
    pub id: String,
    pub title: String,
    pub orig_url: String,
    pub created_at: String,
}

/// Every public son, newest first, for the sitemap's `<image:image>`
/// extension. Capped, not paged: the sitemap protocol has no cursor concept
/// within one file -- a sitemap *index* (multiple files) is the real answer
/// once a site outgrows one, not something worth building ahead of actually
/// needing it.
pub async fn sitemap_sons(limit: i64) -> anyhow::Result<Vec<SitemapSon>> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        title: String,
        orig_url: String,
        created_at: String,
    }
    let rows: Vec<Row> = client()
        .query(
            "SELECT id, title, orig_url, created_at FROM sons \
             WHERE is_public = 1 ORDER BY created_at DESC LIMIT ?1",
            vec![json!(limit)],
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| SitemapSon {
            id: r.id,
            title: r.title,
            orig_url: r.orig_url,
            created_at: r.created_at,
        })
        .collect())
}

/// Every tag with at least one public son under it -- an empty or
/// entirely-hidden tag page has nothing worth a crawler's time.
pub async fn sitemap_tags() -> anyhow::Result<Vec<crate::models::Tag>> {
    use crate::models::Tag;

    #[derive(Deserialize)]
    struct Row {
        name: String,
        slug: String,
    }
    let rows: Vec<Row> = client()
        .query(
            "SELECT DISTINCT t.name, t.slug FROM tags t \
             JOIN son_tags st ON st.tag_id = t.id \
             JOIN sons s ON s.id = st.son_id \
             WHERE s.is_public = 1 \
             ORDER BY t.name",
            vec![],
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| Tag {
            name: r.name,
            slug: r.slug,
        })
        .collect())
}
