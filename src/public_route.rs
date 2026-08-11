//! Plain, stable JSON routes meant for consumption outside this site's own
//! frontend: `/api/v1/*` (a documented public API), `/oembed` (the oEmbed
//! spec, so third-party embedders don't have to scrape OG tags), and the
//! same-origin download proxy the detail page's download button needs.
//!
//! Deliberately not Leptos server functions: those live at hashed paths that
//! change on every rebuild (`/api/list_sons4217581579200484497`), which is
//! fine for this site's own wasm bundle but useless as a stable public
//! contract for anyone else to integrate against.

use axum::extract::{Path, Query};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct ListQuery {
    cursor: Option<String>,
}

/// `GET /api/v1/sons` — a page of public sons, newest first. No personalization
/// (`liked_by_me` is always `false`): an anonymous API consumer never carries
/// this site's voter cookie, so there is nothing to personalize against.
pub async fn list_sons(Query(q): Query<ListQuery>) -> impl IntoResponse {
    match crate::db::list_public(q.cursor.as_deref(), crate::models::Sort::Newest, None).await {
        Ok(page) => Json(page).into_response(),
        Err(e) => api_error(e),
    }
}

/// `GET /api/v1/sons/:id` — a single public son. 404s for a hidden one, same
/// as the site's own detail page.
pub async fn get_son(Path(id): Path<String>) -> impl IntoResponse {
    match crate::db::get(&id, None).await {
        Ok(Some(son)) if son.is_public => Json(son).into_response(),
        Ok(_) => (StatusCode::NOT_FOUND, "no such son").into_response(),
        Err(e) => api_error(e),
    }
}

fn api_error(e: anyhow::Error) -> axum::response::Response {
    tracing::error!("public API error: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
}

#[derive(Serialize)]
struct OEmbedResponse {
    version: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    title: String,
    author_name: Option<String>,
    provider_name: &'static str,
    provider_url: &'static str,
    url: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
pub struct OEmbedQuery {
    url: String,
}

/// `GET /oembed?url=...` per the oEmbed 1.0 spec (oembed.com): given one of
/// this site's own son URLs, return embeddable metadata. This exists
/// alongside the OG tags already on the detail page because oEmbed is what
/// tools that don't just scrape Open Graph (many wikis, some chat platforms'
/// generic embed handling) look for instead.
pub async fn oembed(Query(q): Query<OEmbedQuery>) -> impl IntoResponse {
    let Some(id) = extract_son_id(&q.url) else {
        return (
            StatusCode::BAD_REQUEST,
            "url must point at a /son/:id page on this site",
        )
            .into_response();
    };

    match crate::db::get(&id, None).await {
        Ok(Some(son)) if son.is_public => Json(OEmbedResponse {
            version: "1.0",
            kind: "photo",
            title: son.title,
            author_name: son.uploader.map(|u| u.display_name),
            provider_name: "son collection",
            provider_url: site_origin(),
            url: son.orig_url,
            width: son.width,
            height: son.height,
        })
        .into_response(),
        Ok(_) => (StatusCode::NOT_FOUND, "no such son").into_response(),
        Err(e) => api_error(e),
    }
}

/// Same-origin download proxy. `<a download>` is silently ignored by browsers
/// when the link target is cross-origin, and R2's public domain
/// (media.soncollection.com) is a different origin from the app -- so this
/// route fetches the object itself and sets `Content-Disposition` on a
/// response that genuinely comes from this site, which is the only way the
/// browser reliably treats the click as "save this file."
pub async fn download(Path(id): Path<String>) -> impl IntoResponse {
    let son = match crate::db::get(&id, None).await {
        Ok(Some(son)) if son.is_public => son,
        Ok(_) => return (StatusCode::NOT_FOUND, "no such son").into_response(),
        Err(e) => return api_error(e),
    };

    let key = crate::storage::orig_key(&son.id);
    let bytes = match crate::storage::backend().get(&key).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("download fetch failed for {id}: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not fetch this son",
            )
                .into_response();
        }
    };

    let filename = filename_for(&son.title);
    (
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// A son's title, filesystem-safe. Titles are free text (see
/// `upload_route::clean_title`) and can contain characters invalid in a
/// filename on some platforms, or nothing usable at all.
fn filename_for(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "son.png".to_string()
    } else {
        format!("{trimmed}.png")
    }
}

/// Pull the id out of `.../son/<id>` or `.../son/<id>/`, from any host --
/// oEmbed consumers pass back exactly the URL the site published, so this
/// only needs to parse our own path shape, not validate an arbitrary URL.
fn extract_son_id(url: &str) -> Option<String> {
    let path = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split_once('/'))
        .map(|(_, p)| p)
        .unwrap_or(url);
    let mut segments = path.trim_matches('/').split('/');
    if segments.next()? != "son" {
        return None;
    }
    let id = segments.next()?;
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Cached once, not re-leaked per call: `SITE_ORIGIN` is fixed for the
/// process's lifetime, so this only ever allocates a single `String`.
fn site_origin() -> &'static str {
    static ORIGIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ORIGIN.get_or_init(|| {
        std::env::var("SITE_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3100".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_id_from_a_normal_url() {
        assert_eq!(
            extract_son_id("https://soncollection.com/son/abc-123"),
            Some("abc-123".to_string())
        );
    }

    #[test]
    fn extracts_id_with_trailing_slash() {
        assert_eq!(
            extract_son_id("https://soncollection.com/son/abc-123/"),
            Some("abc-123".to_string())
        );
    }

    #[test]
    fn rejects_urls_that_are_not_a_son_page() {
        assert_eq!(extract_son_id("https://soncollection.com/upload"), None);
        assert_eq!(extract_son_id("https://evil.example/son/"), None);
    }

    #[test]
    fn filename_strips_unsafe_characters() {
        assert_eq!(
            filename_for("Capri/Son: the *best*"),
            "Capri_Son_ the _best_.png"
        );
        assert_eq!(filename_for("   "), "son.png");
    }
}
