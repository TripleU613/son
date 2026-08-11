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
use serde_json::json;

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
const SON_SELECT: &str =
    "SELECT s.id, s.slug, s.title, s.orig_url, s.thumb_url, s.width, s.height, \
     s.created_at, s.is_public, s.reports, s.likes, \
     u.display_name AS uploader_name, u.avatar_url AS uploader_avatar \
     FROM sons s LEFT JOIN users u ON u.id = s.uploader_id";

/// Shape of a row as D1 returns it: JSON, with SQLite's native types (no bool,
/// no fixed-width ints/floats). Converted into `Son` below.
#[derive(Deserialize)]
struct SonRow {
    id: String,
    // Nullable in the schema (see migration 0008), so a row written before the
    // column existed still deserializes; the id stands in when it is absent.
    slug: Option<String>,
    title: String,
    orig_url: String,
    thumb_url: String,
    width: i64,
    height: i64,
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
            slug: r.slug.clone().unwrap_or_else(|| r.id.clone()),
            id: r.id,
            title: r.title,
            orig_url: r.orig_url,
            thumb_url: r.thumb_url,
            width: r.width as u32,
            height: r.height as u32,
            created_at: r.created_at,
            is_public: r.is_public != 0,
            reports: r.reports,
            likes: r.likes,
            // Both depend on who is asking / a follow-up query; filled in by
            // the caller (mark_liked).
            liked_by_me: false,
            uploader: r.uploader_name.map(|display_name| Uploader {
                display_name,
                avatar_url: r.uploader_avatar,
            }),
        }
    }
}

/// Cursors are opaque to the client but encode the full sort key, because
/// keyset pagination needs a total order. `likes` alone is not one — many sons
/// share a value — so that cursor carries a tie-breaker too.
/// Titles can't contain control characters (`upload_route::clean_title`
/// strips them), so NUL is a safe separator there even though `|` could
/// theoretically appear in free-text titles.
fn encode_cursor(s: &Son, sort: Sort) -> String {
    match sort {
        Sort::Newest => s.created_at.clone(),
        Sort::MostLiked => format!("{}|{}", s.likes, s.created_at),
        Sort::Az => format!("{}\u{0}{}", s.title, s.id),
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

/// One son, by slug or by id.
///
/// Both, in one query, because a link with a bare id has to keep working: every
/// son shared before slugs existed used one, and so does anything that grabbed a
/// URL from the API. Slug is matched first so a son can never be shadowed by
/// another's id.
pub async fn get(slug_or_id: &str, voter: Option<&str>) -> anyhow::Result<Option<Son>> {
    let sql = format!(
        "{SON_SELECT} WHERE s.slug = ?1 OR s.id = ?1 \
         ORDER BY CASE WHEN s.slug = ?1 THEN 0 ELSE 1 END \
         LIMIT 1"
    );
    let rows: Vec<SonRow> = client().query(&sql, vec![json!(slug_or_id)]).await?;
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
        // Keyed on the son's real id, not on whatever form the URL used.
        let hit: Vec<Hit> = client()
            .query(
                "SELECT 1 AS x FROM likes WHERE son_id = ?1 AND voter_id = ?2",
                vec![json!(son.id), json!(voter)],
            )
            .await?;
        son.liked_by_me = !hit.is_empty();
    }
    Ok(Some(son))
}

/// A slug for `title` that nothing else is using.
///
/// Appends -2, -3, ... on collision. Racy in principle -- two simultaneous
/// uploads of the same title could both see a free slug -- but the UNIQUE index
/// is the real guard, and the insert failing is preferable to two sons quietly
/// sharing a URL. Uploads are not remotely a hot path.
pub async fn unique_slug(title: &str, id: &str) -> String {
    let base = slugify(title);
    if base.is_empty() {
        return id.to_string();
    }

    #[derive(Deserialize)]
    struct Row {
        slug: String,
    }
    let taken: Vec<Row> = client()
        .query(
            "SELECT slug FROM sons WHERE slug = ?1 OR slug LIKE ?1 || '-%'",
            vec![json!(base)],
        )
        .await
        .unwrap_or_default();
    let taken: std::collections::HashSet<String> = taken.into_iter().map(|r| r.slug).collect();

    if !taken.contains(&base) {
        return base;
    }
    // Bounded: past a few hundred sons with one title, the id is a better URL
    // than "sonion-417" anyway.
    for n in 2..500 {
        let candidate = format!("{base}-{n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    id.to_string()
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

pub struct NewSon<'a> {
    pub id: &'a str,
    pub slug: &'a str,
    pub title: &'a str,
    pub orig_url: &'a str,
    pub thumb_url: &'a str,
    pub width: u32,
    pub height: u32,
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
}

pub async fn insert(new: NewSon<'_>) -> anyhow::Result<Son> {
    let created_at = chrono::Utc::now().to_rfc3339();

    // son_score/nsfw_score are written as literal 0: the columns are NOT NULL
    // from migration 0001 and nothing scores an upload any more. They are left
    // in the schema rather than dropped, so that scores from an external
    // screening API have somewhere to land later and so no existing row loses
    // data. Read as "not assessed" -- nothing displays them, and no ordering or
    // filtering depends on them.
    client()
        .exec(
            "INSERT INTO sons (id, slug, title, orig_url, thumb_url, width, height, \
                               son_score, nsfw_score, created_at, is_public, reports, \
                               uploader_id, content_hash) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,0,0,?8,1,0,?9,?10)",
            vec![
                json!(new.id),
                json!(new.slug),
                json!(new.title),
                json!(new.orig_url),
                json!(new.thumb_url),
                json!(new.width),
                json!(new.height),
                json!(created_at),
                json!(new.uploader_id),
                json!(new.content_hash),
            ],
        )
        .await?;

    Ok(Son {
        id: new.id.to_string(),
        slug: new.slug.to_string(),
        title: new.title.to_string(),
        orig_url: new.orig_url.to_string(),
        thumb_url: new.thumb_url.to_string(),
        width: new.width,
        height: new.height,
        created_at,
        is_public: true,
        reports: 0,
        likes: 0,
        liked_by_me: false,
        uploader: new.uploader,
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
/// Admin comes from `ADMIN_EMAILS` (comma-separated), matched against the email
/// Google just asserted for this login. Deliberately not a hardcoded address:
/// this repo is public, and a list of admin accounts in source is both a
/// disclosure and a thing nobody can change without a deploy.
///
/// Re-evaluated on every login rather than only on insert, so adding an address
/// to the secret promotes that account the next time they sign in, and removing
/// one demotes them. The env var is the single source of truth -- a manual
/// `UPDATE users SET is_admin = 1` would be silently reverted at their next
/// login, which is the intended behaviour.
fn is_admin_email(email: &str) -> bool {
    let email = email.trim().to_ascii_lowercase();
    std::env::var("ADMIN_EMAILS")
        .unwrap_or_default()
        .split(',')
        .map(|e| e.trim().to_ascii_lowercase())
        .any(|allowed| !allowed.is_empty() && allowed == email)
}

pub async fn upsert_user(
    google_sub: &str,
    email: &str,
    display_name: &str,
    avatar_url: Option<&str>,
) -> anyhow::Result<User> {
    let id = uuid::Uuid::new_v4().to_string();
    let admin = i64::from(is_admin_email(email));
    let sql = format!(
        "INSERT INTO users (id, google_sub, email, display_name, avatar_url, is_admin, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?7, ?6) \
         ON CONFLICT (google_sub) DO UPDATE SET \
             email = excluded.email, \
             display_name = excluded.display_name, \
             avatar_url = excluded.avatar_url, \
             is_admin = excluded.is_admin \
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
                json!(admin),
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

/// Lowercase, alphanumerics kept, every other run collapsed to one dash, no
/// leading or trailing dash. Empty for a title with nothing sluggable in it --
/// callers decide the fallback, which for a son is its id.
pub fn slugify(name: &str) -> String {
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
    slug.trim_end_matches('-').to_string()
}

/// The shortest text the trigram tokeniser can index. A one- or two-character
/// term produces no trigrams at all and matches nothing -- silently, with no
/// error -- so those queries take the LIKE path instead.
const MIN_TRIGRAM: usize = 3;

/// How many results the fuzzy fallback may return. Far below `PAGE_SIZE`: past
/// the first few, a row is only there because it shares an incidental trigram
/// with the query, and a page of those reads as broken rather than helpful.
const FUZZY_LIMIT: i64 = 8;

/// Wraps a user's term as an FTS5 phrase. Unavoidable, not defensive: FTS5
/// reads `-` as NOT, so an unquoted "capri-son" fails outright with
/// `no such column: son`. A search box is free text, not a query language.
fn fts_phrase(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// Every 3-character window of `s`, lowercased -- the same units the trigram
/// index stores, so OR-ing them scores a row by how much of the query it
/// actually contains. This is what makes a misspelling still find its son.
fn trigrams(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.to_lowercase().chars().collect();
    if chars.len() < MIN_TRIGRAM {
        return vec![];
    }
    chars
        .windows(MIN_TRIGRAM)
        .map(|w| w.iter().collect::<String>())
        .filter(|t| !t.trim().is_empty())
        .collect()
}

/// Search over titles *and* tag names, via the trigram `sons_search` index.
///
/// Two precise passes and one forgiving one, because a single query cannot be
/// both and a visitor typing into a box has no way to tell us which they meant:
///
/// 1. **Exact-ish:** every term must appear somewhere in the title or tags.
///    Trigram matching means a term hits *inside* a word, not just at its start,
///    so "flower" finds Sonflower and "apri" finds Capri-Son. On a site whose
///    entire joke is words with "son" buried in them, that is the common case,
///    not an edge case. Terms too short to have trigrams are ANDed in as LIKE
///    conditions in the same query, so "dy son" still means both.
/// 2. **Fuzzy**, only if the first pass found nothing: OR the query's trigrams
///    and let bm25 rank by how much of the query each son actually contains, so
///    one wrong keystroke lands on the right son instead of an empty page.
///
/// Capped tighter than a normal page: this tier is a "did you mean", and every
/// row past the first few shares a trigram or two with the query by accident.
///
/// Ordered by relevance, not recency.
pub async fn search_sons(query: &str, voter: Option<&str>) -> anyhow::Result<Vec<Son>> {
    let terms: Vec<&str> = query.split_whitespace().collect();
    if terms.is_empty() {
        return Ok(vec![]);
    }

    // Split by what the index can actually answer. Sending a 1-2 character term
    // to a trigram index matches nothing at all -- silently, with no error -- so
    // those have to be asked a different way, in the same query rather than
    // instead of it. Getting this wrong turned "dy son" from one exact hit into
    // the entire gallery, because the whole query fell through to fuzzy.
    let (long, short): (Vec<&str>, Vec<&str>) =
        terms.iter().partition(|t| t.chars().count() >= MIN_TRIGRAM);

    let mut where_parts = vec!["s.is_public = 1".to_string()];
    let mut params: Vec<serde_json::Value> = vec![];

    if !long.is_empty() {
        params.push(json!(long
            .iter()
            .map(|t| fts_phrase(t))
            .collect::<Vec<_>>()
            .join(" ")));
        where_parts.push(format!("sons_search MATCH ?{}", params.len()));
    }
    for term in &short {
        // `%` and `_` escaped, or a visitor typing "100%" matches every son.
        params.push(json!(format!(
            "%{}%",
            term.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )));
        let i = params.len();
        where_parts.push(format!(
            "(sons_search.title LIKE ?{i} ESCAPE '\\' OR sons_search.tags LIKE ?{i} ESCAPE '\\')"
        ));
    }

    // Only rank by relevance when there is a MATCH to rank by: bm25 is
    // meaningless for a pure-LIKE query, and `ORDER BY rank` without a MATCH is
    // an error rather than a no-op.
    let order = if long.is_empty() {
        "s.created_at DESC"
    } else {
        "rank"
    };
    params.push(json!(PAGE_SIZE));
    let sql = format!(
        "{SON_SELECT} \
         JOIN sons_search ON sons_search.rowid = s.rowid \
         WHERE {} \
         ORDER BY {order} \
         LIMIT ?{}",
        where_parts.join(" AND "),
        params.len(),
    );

    let mut rows: Vec<SonRow> = client().query(&sql, params).await?;

    // Fuzzy fallback. Deliberately drops the AND between terms as well as the
    // requirement to match at all -- a misspelling is usually one term of
    // several, and insisting on the others would keep the result set empty.
    if rows.is_empty() {
        let grams: Vec<String> = terms.iter().flat_map(|t| trigrams(t)).collect();
        if !grams.is_empty() {
            let expr = grams
                .iter()
                .map(|g| fts_phrase(g))
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "{SON_SELECT} \
                 JOIN sons_search ON sons_search.rowid = s.rowid \
                 WHERE s.is_public = 1 AND sons_search MATCH ?1 \
                 ORDER BY rank \
                 LIMIT ?2"
            );
            rows = client()
                .query(&sql, vec![json!(expr), json!(FUZZY_LIMIT)])
                .await?;
        }
    }

    let mut sons: Vec<Son> = rows.into_iter().map(Son::from).collect();
    mark_liked(&mut sons, voter).await?;
    Ok(sons)
}

/// A lightweight row for `sitemap.xml` -- only what the sitemap's image
/// extension needs, not a full `Son` (no likes/tags/uploader join).
pub struct SitemapSon {
    /// The slug, since that is what the sitemap should advertise -- an id URL
    /// works but is the form nobody should be indexing.
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
        slug: Option<String>,
        title: String,
        orig_url: String,
        created_at: String,
    }
    let rows: Vec<Row> = client()
        .query(
            "SELECT id, slug, title, orig_url, created_at FROM sons \
             WHERE is_public = 1 ORDER BY created_at DESC LIMIT ?1",
            vec![json!(limit)],
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| SitemapSon {
            id: r.slug.clone().unwrap_or_else(|| r.id.clone()),
            title: r.title,
            orig_url: r.orig_url,
            created_at: r.created_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_url_safe() {
        assert_eq!(slugify("Capri-Son"), "capri-son");
        assert_eq!(slugify("Son of Man"), "son-of-man");
        assert_eq!(slugify("  Sonion!!  "), "sonion");
        assert_eq!(slugify("Son   of    spaces"), "son-of-spaces");
    }

    /// Empty rather than a placeholder, so the caller can fall back to the id.
    /// A title of pure punctuation or non-Latin script has nothing to slug.
    #[test]
    fn unsluggable_titles_are_empty() {
        assert_eq!(slugify("!!!"), "");
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("😭😭😭"), "");
    }

    /// Trigrams are what the fuzzy fallback ORs together, so a term shorter than
    /// one must produce nothing rather than a partial gram that matches
    /// everything.
    #[test]
    fn trigrams_need_three_characters() {
        assert!(trigrams("dy").is_empty());
        assert!(trigrams("").is_empty());
        assert_eq!(trigrams("sonion"), ["son", "oni", "nio", "ion"]);
    }

    /// FTS5 reads a bare hyphen as NOT and errors with `no such column`, and an
    /// unescaped quote ends the phrase early. Both must survive quoting.
    #[test]
    fn fts_phrases_neutralise_operators() {
        assert_eq!(fts_phrase("capri-son"), "\"capri-son\"");
        assert_eq!(fts_phrase("say \"son\""), "\"say \"\"son\"\"\"");
    }

    #[test]
    fn admin_emails_match_case_insensitively_and_exactly() {
        // SAFETY: single-threaded test, and the var is read only here.
        unsafe { std::env::set_var("ADMIN_EMAILS", "Boss@Example.com, other@example.com") };
        assert!(is_admin_email("boss@example.com"));
        assert!(is_admin_email("  OTHER@EXAMPLE.COM  "));
        assert!(!is_admin_email("boss@example.com.evil.test"));
        assert!(!is_admin_email("nobody@example.com"));
        unsafe { std::env::remove_var("ADMIN_EMAILS") };
        assert!(!is_admin_email("boss@example.com"));
    }
}
