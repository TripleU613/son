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
use crate::models::{Son, SonPage, Sort, PAGE_SIZE};

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

const COLS: &str = "id, title, orig_url, thumb_url, width, height, \
                    son_score, nsfw_score, created_at, is_public, reports, likes";

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
            // Depends on who is asking; filled in by the caller.
            liked_by_me: false,
        }
    }
}

/// Cursors are opaque to the client but encode the full sort key, because
/// keyset pagination needs a total order. `likes` alone is not one — many sons
/// share a count — so the most-liked cursor carries `created_at` too.
fn encode_cursor(s: &Son, sort: Sort) -> String {
    match sort {
        Sort::Newest => s.created_at.clone(),
        Sort::MostLiked => format!("{}|{}", s.likes, s.created_at),
    }
}

fn decode_cursor(cursor: &str, sort: Sort) -> (i64, String) {
    match sort {
        Sort::Newest => (0, cursor.to_string()),
        Sort::MostLiked => match cursor.split_once('|') {
            Some((likes, created)) => (likes.parse().unwrap_or(i64::MAX), created.to_string()),
            // Malformed cursor: start from the top rather than erroring at the
            // user, since a cursor only ever comes from us.
            None => (i64::MAX, String::new()),
        },
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
                "SELECT {COLS} FROM sons \
                 WHERE is_public = 1 AND (?1 IS NULL OR created_at < ?1) \
                 ORDER BY created_at DESC, id DESC \
                 LIMIT ?2"
            );
            client()
                .query(&sql, vec![json!(cursor), json!(PAGE_SIZE + 1)])
                .await?
        }
        Sort::MostLiked => {
            let (likes, created) = cursor
                .map(|c| decode_cursor(c, sort))
                .unwrap_or((i64::MAX, String::new()));
            let sql = format!(
                "SELECT {COLS} FROM sons \
                 WHERE is_public = 1 \
                   AND (?1 = 0 OR likes < ?2 OR (likes = ?2 AND created_at < ?3)) \
                 ORDER BY likes DESC, created_at DESC, id DESC \
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

pub async fn get(id: &str, voter: Option<&str>) -> anyhow::Result<Option<Son>> {
    let sql = format!("SELECT {COLS} FROM sons WHERE id = ?1");
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
    Ok(Some(son))
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
                               son_score, nsfw_score, embedding, created_at, is_public, reports) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,0)",
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
    })
}

/// Flag a son. At `AUTO_HIDE_REPORTS` it pulls itself from the gallery — the
/// safety valve that makes auto-publishing survivable without a review queue.
pub const AUTO_HIDE_REPORTS: i64 = 3;

pub async fn report(id: &str) -> anyhow::Result<()> {
    client()
        .exec(
            "UPDATE sons \
             SET reports = reports + 1, \
                 is_public = CASE WHEN reports + 1 >= ?2 THEN 0 ELSE is_public END \
             WHERE id = ?1",
            vec![json!(id), json!(AUTO_HIDE_REPORTS)],
        )
        .await?;
    Ok(())
}

pub async fn set_public(id: &str, public: bool) -> anyhow::Result<()> {
    client()
        .exec(
            "UPDATE sons SET is_public = ?2 WHERE id = ?1",
            vec![json!(id), json!(i64::from(public))],
        )
        .await?;
    Ok(())
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
