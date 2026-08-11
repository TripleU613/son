//! SQLite access.
//!
//! Uses runtime-checked queries (`sqlx::query`) rather than the `query!` macros
//! on purpose: the macros need a live DATABASE_URL at *compile* time, which
//! would mean `cargo leptos build` fails on a fresh clone.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::models::{Son, SonPage, PAGE_SIZE};

pub type Db = SqlitePool;

/// Process-wide pool, set once at startup.
///
/// A `OnceLock` rather than Leptos context because server functions and plain
/// Axum handlers (the multipart upload) both need it, and threading a context
/// through both entry points buys nothing for a single-database app.
static POOL: std::sync::OnceLock<Db> = std::sync::OnceLock::new();

pub fn set_pool(db: Db) {
    let _ = POOL.set(db);
}

/// Panics if called before `set_pool`, which can only happen if a handler runs
/// before startup finished — not reachable in practice.
pub fn pool() -> &'static Db {
    POOL.get().expect("db pool not initialized")
}

pub async fn connect(url: &str) -> anyhow::Result<Db> {
    let opts: SqliteConnectOptions = url.parse()?;
    let opts = opts
        .create_if_missing(true)
        // WAL lets the gallery keep serving reads during an upload's write.
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

fn row_to_son(row: &sqlx::sqlite::SqliteRow) -> Son {
    Son {
        id: row.get("id"),
        title: row.get("title"),
        orig_url: row.get("orig_url"),
        thumb_url: row.get("thumb_url"),
        width: row.get::<i64, _>("width") as u32,
        height: row.get::<i64, _>("height") as u32,
        son_score: row.get::<f64, _>("son_score") as f32,
        nsfw_score: row.get::<f64, _>("nsfw_score") as f32,
        created_at: row.get("created_at"),
        is_public: row.get::<i64, _>("is_public") != 0,
        reports: row.get("reports"),
    }
}

const COLS: &str = "id, title, orig_url, thumb_url, width, height, \
                    son_score, nsfw_score, created_at, is_public, reports";

/// One page of public sons, newest first.
///
/// Keyset pagination on `created_at` rather than OFFSET: a gallery where new
/// rows land at the top would shift OFFSET windows under the reader, showing
/// duplicates. Ties on an identical timestamp break by `id` to stay total.
pub async fn list_public(db: &Db, cursor: Option<&str>) -> anyhow::Result<SonPage> {
    let sql = format!(
        "SELECT {COLS} FROM sons \
         WHERE is_public = 1 AND ($1 IS NULL OR created_at < $1) \
         ORDER BY created_at DESC, id DESC \
         LIMIT $2"
    );

    let rows = sqlx::query(&sql)
        .bind(cursor)
        .bind(PAGE_SIZE + 1) // one extra row tells us whether more exist
        .fetch_all(db)
        .await?;

    let has_more = rows.len() as i64 > PAGE_SIZE;
    let sons: Vec<Son> = rows
        .iter()
        .take(PAGE_SIZE as usize)
        .map(row_to_son)
        .collect();

    let next_cursor = if has_more {
        sons.last().map(|s| s.created_at.clone())
    } else {
        None
    };

    Ok(SonPage { sons, next_cursor })
}

pub async fn get(db: &Db, id: &str) -> anyhow::Result<Option<Son>> {
    let sql = format!("SELECT {COLS} FROM sons WHERE id = $1");
    let row = sqlx::query(&sql).bind(id).fetch_optional(db).await?;
    Ok(row.as_ref().map(row_to_son))
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

pub async fn insert(db: &Db, new: NewSon<'_>) -> anyhow::Result<Son> {
    let created_at = chrono::Utc::now().to_rfc3339();
    let blob: Option<Vec<u8>> = new
        .embedding
        .map(|e| e.iter().flat_map(|f| f.to_le_bytes()).collect());

    sqlx::query(
        "INSERT INTO sons (id, title, orig_url, thumb_url, width, height, \
                           son_score, nsfw_score, embedding, created_at, is_public, reports) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,1,0)",
    )
    .bind(new.id)
    .bind(new.title)
    .bind(new.orig_url)
    .bind(new.thumb_url)
    .bind(new.width as i64)
    .bind(new.height as i64)
    .bind(new.son_score as f64)
    .bind(new.nsfw_score as f64)
    .bind(blob)
    .bind(&created_at)
    .execute(db)
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
    })
}

/// Flag a son. At `AUTO_HIDE_REPORTS` it pulls itself from the gallery — the
/// safety valve that makes auto-publishing survivable without a review queue.
pub const AUTO_HIDE_REPORTS: i64 = 3;

pub async fn report(db: &Db, id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE sons \
         SET reports = reports + 1, \
             is_public = CASE WHEN reports + 1 >= $2 THEN 0 ELSE is_public END \
         WHERE id = $1",
    )
    .bind(id)
    .bind(AUTO_HIDE_REPORTS)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_public(db: &Db, id: &str, public: bool) -> anyhow::Result<()> {
    sqlx::query("UPDATE sons SET is_public = $2 WHERE id = $1")
        .bind(id)
        .bind(if public { 1i64 } else { 0i64 })
        .execute(db)
        .await?;
    Ok(())
}

pub async fn count_public(db: &Db) -> anyhow::Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM sons WHERE is_public = 1")
        .fetch_one(db)
        .await?;
    Ok(row.get("n"))
}
