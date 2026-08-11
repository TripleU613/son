//! `POST /api/upload` — multipart image intake.
//!
//! Order: decode+hash → duplicate check → store (watermark + provenance
//! metadata) → insert. Nothing is written until the duplicate check clears, so a
//! rejected upload leaves no files to clean up.
//!
//! **No content analysis happens here.** Uploads are published as they arrive.
//! There is no classifier deciding whether an image is a son, or whether it is
//! explicit — the local CLIP model that used to make both calls was removed, and
//! screening is expected to move to an external API. Until it does, the only
//! things standing between an upload and the public gallery are the format and
//! size limits in `storage::decode` and the human report queue in `/admin`.

use crate::models::UploadResult;
use axum::extract::Multipart;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

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
    // The content hash rides along in the same blocking call: it's a hash of
    // the decoded pixel buffer (not the raw upload bytes), computed here
    // because this is the one place that buffer exists before storage
    // consumes it.
    let decoded = tokio::task::spawn_blocking(move || {
        let img = crate::storage::decode(&bytes)?;
        let hash = crate::dedupe::sha256_hex(img.to_rgba8().as_raw());
        Ok::<_, anyhow::Error>((img, hash))
    })
    .await;

    let (img, content_hash) = match decoded {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => return bad(StatusCode::BAD_REQUEST, e.to_string()),
        Err(e) => {
            return bad(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("decode panicked: {e}"),
            )
        }
    };

    // Exact duplicate: same decoded pixels as something already here,
    // regardless of container format. A single indexed lookup.
    match crate::db::find_by_hash(&content_hash).await {
        Ok(Some(existing)) => {
            tracing::info!(existing = existing.id, "upload rejected: exact duplicate");
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(UploadResult::Rejected {
                    reason: format!(
                        "this exact image is already in the collection: /son/{}",
                        existing.id
                    ),
                }),
            );
        }
        Ok(None) => {}
        // A dedupe-check outage shouldn't block an otherwise-good upload;
        // log loudly and let it through rather than fail the whole request.
        Err(e) => tracing::error!("hash dedupe check failed: {e}"),
    }

    let title = clean_title(&title);

    let stored = match crate::storage::store(
        &img,
        &title,
        uploader.as_ref().map(|u| u.display_name.as_str()),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("store failed: {e}");
            return bad(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not save this son".into(),
            );
        }
    };

    let tag_names = parse_tags(&tags_raw);

    let son = crate::db::insert(crate::db::NewSon {
        id: &stored.id,
        title: &title,
        orig_url: &stored.orig_url,
        thumb_url: &stored.thumb_url,
        width: stored.width,
        height: stored.height,
        content_hash: &content_hash,
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
