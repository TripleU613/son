//! Plain, stable routes meant for consumption outside this site's own
//! frontend: `/api/v1/*` (a documented public API), `/oembed` (the oEmbed
//! spec, so third-party embedders don't have to scrape OG tags), `/embed/:id`
//! (a framable card for one son) and the same-origin download proxy the
//! detail page's download button needs.
//!
//! Deliberately not Leptos server functions: those live at hashed paths that
//! change on every rebuild (`/api/list_sons4217581579200484497`), which is
//! fine for this site's own wasm bundle but useless as a stable public
//! contract for anyone else to integrate against.
//!
//! `/embed/:id` is a plain Axum route for the same reason and one more: it is
//! a whole document, not a page inside the app shell. A Leptos route would
//! drag the nav, the wasm bundle and the hydration island into an iframe that
//! wants none of them.

use axum::extract::{Path, Query};
use axum::http::{header, HeaderName, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::seo::{attr_escape, embed_iframe, fit_within, EMBED_BAR_H, EMBED_DEFAULT_W};

/// `X-Robots-Tag`, which `http` has no constant for.
const X_ROBOTS_TAG: HeaderName = HeaderName::from_static("x-robots-tag");

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
    /// The image itself for a `photo`; absent for a `rich`, where `html` is
    /// the resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    width: u32,
    height: u32,
    /// Only on a `rich` response. `skip_serializing_if` rather than a `null`:
    /// consumers that branch on the presence of the key exist, and some reject
    /// a null outright.
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    /// Seconds. Matches `/embed/:id`'s own `max-age`, so a consumer that
    /// honours this refetches on the same clock the edge does -- which matters
    /// because the three-report auto-hide is the primary moderation mechanism
    /// and a stale embed outlives the decision.
    cache_age: &'static str,
    /// oEmbed requires all three thumbnail fields or none of them, so these
    /// are set and cleared together.
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail_height: Option<u32>,
}

#[derive(Deserialize)]
pub struct OEmbedQuery {
    url: String,
    /// `json` or absent. The spec says a provider that cannot supply the
    /// requested format must answer 501, and answering JSON to `format=xml`
    /// (which is what this did) leaves the consumer parsing HTML-ish noise as
    /// XML instead of seeing a clear "not supported".
    format: Option<String>,
    maxwidth: Option<u32>,
    maxheight: Option<u32>,
}

/// `GET /oembed?url=...` per the oEmbed 1.0 spec (oembed.com): given one of
/// this site's own URLs, return embeddable metadata. This exists alongside the
/// OG tags already on the detail page because oEmbed is what tools that don't
/// just scrape Open Graph (many wikis, some chat platforms' generic embed
/// handling) look for instead.
///
/// Two URL shapes, two resource types, on purpose:
///
/// - `/son/<slug>` -> `type: photo`, the answer the `<link rel="alternate"
///   type="application/json+oembed">` on the detail page has been advertising
///   since before `/embed/` existed. Consumers have that cached. Switching it
///   to `rich` would hand an iframe to everything that currently renders the
///   image, and some of them refuse to frame anything -- a silent unfurl
///   regression with no error anywhere.
/// - `/embed/<slug>` -> `type: rich`, with the iframe markup built by the same
///   `seo::embed_iframe` the copy button uses.
///
/// Both are spec-legal: they are two different resources that happen to
/// describe one son.
pub async fn oembed(Query(q): Query<OEmbedQuery>) -> impl IntoResponse {
    // Absent means "provider's choice" per the spec, and JSON is the choice.
    if !matches!(q.format.as_deref(), None | Some("json")) {
        return (StatusCode::NOT_IMPLEMENTED, "only format=json is supported").into_response();
    }

    let Some((id, wants_embed)) = extract_son_ref(&q.url) else {
        return (
            StatusCode::BAD_REQUEST,
            "url must point at a /son/:id or /embed/:id page on this site",
        )
            .into_response();
    };

    let son = match crate::db::get(&id, None).await {
        Ok(Some(son)) if son.is_public => son,
        Ok(_) => return (StatusCode::NOT_FOUND, "no such son").into_response(),
        Err(e) => return api_error(e),
    };

    // All three or none, per the spec. A zero-sized source (impossible for a
    // stored son, but the columns allow it) drops the group rather than
    // reporting 0x0.
    let (tw, th) = fit_within(son.width, son.height, crate::storage::THUMB_MAX_EDGE);
    let thumb = (tw > 0 && th > 0).then(|| son.thumb_url.clone());

    let mut resp = OEmbedResponse {
        version: "1.0",
        kind: "photo",
        title: son.title.clone(),
        author_name: son.uploader.map(|u| u.display_name),
        provider_name: "son collection",
        provider_url: site_origin(),
        url: None,
        width: 0,
        height: 0,
        html: None,
        cache_age: "300",
        thumbnail_url: thumb.clone(),
        thumbnail_width: thumb.is_some().then_some(tw),
        thumbnail_height: thumb.is_some().then_some(th),
    };

    if wants_embed {
        // The card is a square image area above a fixed-height bar, so its
        // height follows its width -- which means `maxheight` constrains the
        // width too, not just the height.
        let mut w = EMBED_DEFAULT_W;
        if let Some(mw) = q.maxwidth {
            w = w.min(mw);
        }
        if let Some(mh) = q.maxheight {
            w = w.min(mh.saturating_sub(EMBED_BAR_H));
        }
        // Below this the bar has no room for a title and the card is not worth
        // rendering; honouring an absurd maxwidth exactly would be worse than
        // returning the smallest card that works.
        let w = w.max(120);
        let embed_url = crate::seo::absolute(&crate::seo::embed_path(&son.slug));
        resp.kind = "rich";
        resp.width = w;
        resp.height = w + EMBED_BAR_H;
        resp.html = Some(embed_iframe(&embed_url, &son.title, w, w + EMBED_BAR_H));
    } else {
        // A consumer that asked for something no larger than a thumbnail gets
        // the thumbnail: serving a 12MB original into a 480px slot is R2 egress
        // spent on pixels nobody sees.
        let cap = [q.maxwidth, q.maxheight].into_iter().flatten().min();
        let small = cap.is_some_and(|c| c <= crate::storage::THUMB_MAX_EDGE) && tw > 0;
        if small {
            resp.url = Some(son.thumb_url);
            resp.width = tw;
            resp.height = th;
        } else {
            resp.url = Some(son.orig_url);
            resp.width = son.width;
            resp.height = son.height;
        }
    }

    Json(resp).into_response()
}

/// Everything the framable card needs, as a single inline stylesheet.
///
/// Deliberately no reference to `/pkg/soncollection.css`: that filename
/// depends on `hash.txt` plus the `LEPTOS_HASH_FILES` runtime switch, and
/// re-deriving it here would couple third-party embeds to the hashing trap
/// described in CLAUDE.md -- an embed silently losing all styling on every
/// site that uses it, discovered by nobody.
///
/// The colours are copied by hand from `tailwind.config.js` (bg `#08090b`,
/// surface `#0d0f12`, line `#292d35`, ink `#f4f4f5`/`#a6a8b0`, accent
/// `#ffcc33`). That file is the palette's source of truth and this is a real
/// duplication of it; there is no `@apply` available in a string constant, so
/// a palette change has to be made in both places.
///
/// No Tailwind class name appears anywhere in this document. The scanner reads
/// .rs files as raw text, so utility names written here would be emitted into
/// `soncollection.css` -- harmless, since nothing here loads that file, but
/// the reverse is not: a `.card` or `.btn` used *in the markup* would render
/// completely unstyled, because the embed page loads no stylesheet but this
/// one.
///
/// `min-height:0` on `.se-img` is load-bearing. A flex item's default
/// `min-height:auto` sizes it by its content's min-content height, so the
/// image pushes the card past the iframe's box and the bar disappears below
/// the fold -- the same failure CLAUDE.md records for implicit grid tracks.
/// `object-fit:contain` letterboxes, because sons are not all square
/// (`makaaut-queson` is 1919x1080, `sonion-powder` is 1024x1024) and one
/// snippet has to work for every one of them.
const EMBED_CSS: &str = "\
html,body{margin:0;height:100%;background:#08090b;color:#f4f4f5;\
font:400 14px/1.35 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}\
a{color:inherit;text-decoration:none}\
.se-card{display:flex;flex-direction:column;height:100%;box-sizing:border-box;\
background:#0d0f12;border:1px solid #292d35;border-radius:10px;overflow:hidden}\
.se-img{flex:1 1 auto;min-height:0;display:flex;align-items:center;\
justify-content:center;padding:12px}\
.se-img img{max-width:100%;max-height:100%;width:auto;height:auto;\
object-fit:contain;display:block}\
.se-bar{flex:none;display:flex;align-items:center;justify-content:space-between;\
gap:8px;height:44px;padding:0 12px;border-top:1px solid #292d35}\
.se-title{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:500}\
.se-brand{flex:none;color:#ffcc33;font-size:12px}\
.se-miss{display:flex;height:100%;align-items:center;justify-content:center;\
padding:12px;text-align:center;color:#a6a8b0}";

/// The document shell every `/embed/:id` response uses, hit or miss.
///
/// `noindex, follow` and no `<link rel="canonical">`. The canonical is the
/// missing half on purpose: Google documents `noindex` plus a canonical
/// pointing elsewhere as a mistake, because the `noindex` propagates to the
/// canonical target -- which here would deindex the son's own page. `follow`
/// still lets a crawler walk the link back to it. The matching mistake is
/// disallowing `/embed/` in robots.txt, which would stop the crawler ever
/// fetching the page and reading this tag; see `seo_route::robots_txt`.
fn embed_document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex, follow\">\
         <title>{title} — son collection</title>\
         <style>{EMBED_CSS}</style></head><body>{body}</body></html>",
        title = attr_escape(title),
    )
}

/// `GET /embed/:id` — one son as a standalone, framable document.
///
/// No Leptos, no hydration, no wasm: an iframe on someone else's site should
/// cost them one small HTML document and one image, not this app's bundle.
/// The card fills 100% of whatever box the embedder gives it, so a single
/// snippet works at every width and for every son's aspect ratio.
pub async fn embed(Path(id): Path<String>) -> impl IntoResponse {
    // `db::get` matches slug *or* id, so an embed minted before slugs existed
    // keeps resolving on whatever site it was pasted into.
    let son = match crate::db::get(&id, None).await {
        Ok(Some(son)) if son.is_public => son,
        Ok(_) => return embed_gone(StatusCode::NOT_FOUND, "This son is no longer here."),
        Err(e) => {
            tracing::error!("embed lookup failed for {id}: {e}");
            return embed_gone(StatusCode::INTERNAL_SERVER_ERROR, "This son is unavailable.");
        }
    };

    // The thumbnail is the right file for a default-width card and the
    // original only earns its bytes above that, so let the browser choose. The
    // `w` descriptors have to be the real widths: `fit_within` reports what
    // `storage` actually produced. Skipped when the "thumbnail" is an upscale
    // of a small son, where the original is the smaller file of the two.
    let (tw, _) = fit_within(son.width, son.height, crate::storage::THUMB_MAX_EDGE);
    let srcset = (tw > 0 && tw < son.width)
        .then(|| {
            format!(
                " srcset=\"{thumb} {tw}w, {orig} {ow}w\" sizes=\"100vw\"",
                thumb = attr_escape(&son.thumb_url),
                orig = attr_escape(&son.orig_url),
                ow = son.width,
            )
        })
        .unwrap_or_default();

    let body = format!(
        "<a class=\"se-card\" href=\"{page}\" target=\"_blank\" rel=\"noopener\">\
         <div class=\"se-img\"><img src=\"{orig}\"{srcset} alt=\"{alt}\" width=\"{w}\" \
         height=\"{h}\" loading=\"lazy\"></div>\
         <div class=\"se-bar\"><span class=\"se-title\">{title}</span>\
         <span class=\"se-brand\">son collection</span></div></a>",
        // From the slug, never the id: an embed is the most-copied link this
        // site emits and it should carry the readable form.
        page = attr_escape(&format!("{}/son/{}", site_origin(), son.slug)),
        orig = attr_escape(&son.orig_url),
        alt = attr_escape(&son.title),
        w = son.width,
        h = son.height,
        title = attr_escape(&son.title),
    );

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            // Five minutes is the ceiling, and `stale-while-revalidate` is
            // deliberately absent: /admin and the three-report auto-hide are
            // the primary moderation mechanism, and an edge that keeps serving
            // a removed son past its TTL would outlive the decision on sites
            // this one does not control.
            (header::CACHE_CONTROL, "public, max-age=300"),
            (X_ROBOTS_TAG, "noindex"),
            // Explicit permission to be framed anywhere -- and a marker, so
            // that a blanket `X-Frame-Options: DENY` added later cannot be
            // dropped in without noticing it breaks this route.
            (header::CONTENT_SECURITY_POLICY, "frame-ancestors *"),
        ],
        embed_document(&son.title, &body),
    )
        .into_response()
}

/// A styled miss, not a bare status line. An empty 404 body renders as the
/// browser's own error page *inside the iframe*, which looks like the
/// embedder's site is broken rather than like a son that went away.
///
/// `no-store`, so a removal takes effect immediately everywhere and a
/// re-publish is not masked by an edge-cached miss.
fn embed_gone(status: StatusCode, message: &str) -> axum::response::Response {
    let body = format!("<div class=\"se-miss\">{}</div>", attr_escape(message));
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (X_ROBOTS_TAG, "noindex"),
            (header::CONTENT_SECURITY_POLICY, "frame-ancestors *"),
        ],
        embed_document("Not found", &body),
    )
        .into_response()
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

/// Pull the id out of `.../son/<id>` or `.../embed/<id>` (with or without a
/// trailing slash, from any host), and say which of the two it was.
///
/// oEmbed consumers pass back exactly the URL the site published, so this only
/// needs to parse our own path shapes, not validate an arbitrary URL. The
/// second half of the return value is what selects `photo` versus `rich` --
/// the two are different resources, so the URL is the only thing that can
/// decide.
fn extract_son_ref(url: &str) -> Option<(String, bool)> {
    let path = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split_once('/'))
        .map(|(_, p)| p)
        .unwrap_or(url);
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let mut segments = path.trim_matches('/').split('/');
    let is_embed = match segments.next()? {
        "son" => false,
        "embed" => true,
        _ => return None,
    };
    let id = segments.next()?;
    if id.is_empty() {
        None
    } else {
        Some((id.to_string(), is_embed))
    }
}

/// Cached once, not re-leaked per call: `SITE_ORIGIN` is fixed for the
/// process's lifetime, so this only ever allocates a single `String`.
/// `pub(crate)`: `seo_route` needs the same origin for `robots.txt`/
/// `sitemap.xml`/`llms.txt`, and there is exactly one process-wide value to
/// agree on -- not something to look up twice.
pub(crate) fn site_origin() -> &'static str {
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
            extract_son_ref("https://soncollection.com/son/abc-123"),
            Some(("abc-123".to_string(), false))
        );
    }

    #[test]
    fn extracts_id_with_trailing_slash() {
        assert_eq!(
            extract_son_ref("https://soncollection.com/son/abc-123/"),
            Some(("abc-123".to_string(), false))
        );
    }

    /// The flag, not just the id: this is what makes `/embed/<slug>` answer
    /// `rich` while the son page keeps answering `photo`.
    #[test]
    fn extracts_id_from_an_embed_url_and_flags_it() {
        assert_eq!(
            extract_son_ref("https://soncollection.com/embed/abc-123"),
            Some(("abc-123".to_string(), true))
        );
        assert_eq!(
            extract_son_ref("http://127.0.0.1:3100/embed/abc-123/"),
            Some(("abc-123".to_string(), true))
        );
    }

    #[test]
    fn rejects_urls_that_are_not_a_son_page() {
        assert_eq!(extract_son_ref("https://soncollection.com/upload"), None);
        assert_eq!(extract_son_ref("https://evil.example/son/"), None);
        assert_eq!(extract_son_ref("https://soncollection.com/embed/"), None);
        assert_eq!(extract_son_ref("https://soncollection.com/"), None);
    }

    /// The embed page's own CSS must never grow a Tailwind class name: it
    /// loads no stylesheet but its own, so one would render unstyled, and the
    /// scanner would emit the rule into `soncollection.css` for nothing.
    #[test]
    fn embed_document_is_self_contained_and_noindex() {
        let html = embed_document("Sonion Powder", "<div class=\"se-miss\">x</div>");
        assert!(html.contains("<meta name=\"robots\" content=\"noindex, follow\">"));
        assert!(!html.contains("rel=\"canonical\""));
        assert!(!html.contains("soncollection.css"));
        assert!(html.contains(".se-img{flex:1 1 auto;min-height:0"));
    }

    /// A title is free text and lands in `<title>` and in three attributes.
    #[test]
    fn embed_document_escapes_a_hostile_title() {
        let html = embed_document("</title><script>alert(1)</script>", "");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;/title&gt;"));
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
