//! `POST /api/upload` — multipart image intake.
//!
//! Order matters and is deliberate: decode → moderate → store → insert. Nothing
//! is written to disk until the classifier has passed it, so a rejected upload
//! leaves no trace to clean up.

use crate::models::UploadResult;
use crate::moderation::Moderator;
use axum::extract::Multipart;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

static MODERATOR: std::sync::OnceLock<Box<dyn Moderator>> = std::sync::OnceLock::new();

pub fn set_moderator(m: Box<dyn Moderator>) {
    let _ = MODERATOR.set(m);
}

fn moderator() -> &'static dyn Moderator {
    MODERATOR.get().expect("moderator not initialized").as_ref()
}

// `HeaderMap` before `Multipart`: axum requires body-consuming extractors
// (Multipart reads the request body) to come last in a handler's arguments.
pub async fn upload(headers: axum::http::HeaderMap, mut mp: Multipart) -> impl IntoResponse {
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok());
    // A cookie lookup failure (D1 unreachable) should not block the upload
    // over something unrelated to it; fall back to anonymous rather than
    // erroring the whole request.
    let uploader = crate::auth::current_user(cookie_header)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("could not resolve uploader from session: {e}");
            None
        });

    let mut bytes: Option<Vec<u8>> = None;
    let mut title = String::new();
    let mut tags_raw = String::new();

    loop {
        let field = match mp.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return bad(StatusCode::BAD_REQUEST, format!("malformed upload: {e}")),
        };

        match field.name() {
            Some("son") => match field.bytes().await {
                Ok(b) => bytes = Some(b.to_vec()),
                Err(e) => return bad(StatusCode::BAD_REQUEST, format!("could not read file: {e}")),
            },
            Some("title") => {
                title = field.text().await.unwrap_or_default();
            }
            Some("tags") => {
                tags_raw = field.text().await.unwrap_or_default();
            }
            _ => {}
        }
    }

    let Some(bytes) = bytes else {
        return bad(StatusCode::BAD_REQUEST, "no file in the 'son' field".into());
    };

    // Decoding is CPU-bound and attacker-influenced; keep it off the async
    // runtime's worker threads so one huge image can't stall request handling.
    let decoded = tokio::task::spawn_blocking(move || crate::storage::decode(&bytes)).await;

    let img = match decoded {
        Ok(Ok(img)) => img,
        Ok(Err(e)) => return bad(StatusCode::BAD_REQUEST, e.to_string()),
        Err(e) => {
            return bad(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("decode panicked: {e}"),
            )
        }
    };

    let verdict = match tokio::task::spawn_blocking(move || {
        let v = moderator().assess(&img);
        (v, img)
    })
    .await
    {
        Ok((Ok(v), img)) => (v, img),
        // Fail closed: a classifier error rejects rather than publishes.
        Ok((Err(e), _)) => {
            tracing::error!("moderation failed: {e}");
            return bad(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not assess this image".into(),
            );
        }
        Err(e) => {
            return bad(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("moderation panicked: {e}"),
            )
        }
    };

    let (verdict, img) = verdict;

    if let Some(reason) = verdict.rejection_reason() {
        tracing::info!(
            son = verdict.son_score,
            nsfw = verdict.nsfw_score,
            "upload rejected"
        );
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(UploadResult::Rejected {
                reason: reason.to_string(),
                son_score: verdict.son_score,
                nsfw_score: verdict.nsfw_score,
            }),
        );
    }

    let stored = match crate::storage::store(&img).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("store failed: {e}");
            return bad(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not save this son".into(),
            );
        }
    };

    let title = clean_title(&title);
    let tag_names = parse_tags(&tags_raw);

    let son = crate::db::insert(crate::db::NewSon {
        id: &stored.id,
        title: &title,
        orig_url: &stored.orig_url,
        thumb_url: &stored.thumb_url,
        width: stored.width,
        height: stored.height,
        son_score: verdict.son_score,
        nsfw_score: verdict.nsfw_score,
        embedding: verdict.embedding.as_deref(),
        uploader_id: uploader.as_ref().map(|u| u.id.as_str()),
        uploader: uploader.as_ref().map(|u| crate::models::Uploader {
            display_name: u.display_name.clone(),
            avatar_url: u.avatar_url.clone(),
        }),
        // Attached after insert (sons_fts's AFTER INSERT trigger needs the
        // row to exist first); filled in below once we have the real id.
        tags: Vec::new(),
    })
    .await;

    let mut son = match son {
        Ok(son) => son,
        Err(e) => {
            // The row failed but the files landed. Remove them so the disk does
            // not fill with images nothing references.
            crate::storage::remove(&stored.id).await;
            tracing::error!("insert failed: {e}");
            return bad(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not record this son".into(),
            );
        }
    };

    if !tag_names.is_empty() {
        match crate::db::attach_tags(&son.id, &tag_names).await {
            Ok(tags) => son.tags = tags,
            // Not fatal: the son itself is already saved correctly, and tags
            // can be added on a later edit -- worth failing loudly in logs,
            // not worth discarding an otherwise-good upload over.
            Err(e) => tracing::error!("attach_tags failed for {}: {e}", son.id),
        }
    }

    (StatusCode::CREATED, Json(UploadResult::Ok { son }))
}

fn bad(code: StatusCode, message: String) -> (StatusCode, Json<UploadResult>) {
    (code, Json(UploadResult::Error { message }))
}

/// Comma-separated free text -> a short, sane list of tag names. Capped at 8
/// tags and 30 characters each so one upload can't turn into an unbounded
/// write (or an unbounded `attach_tags` loop).
fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().take(30).collect::<String>())
        .take(8)
        .collect()
}

/// Titles are rendered as text by Leptos (which escapes), so this is about
/// keeping the gallery legible, not about injection.
fn clean_title(raw: &str) -> String {
    let t: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    if t.is_empty() {
        "untitled son".to_string()
    } else {
        t
    }
}
