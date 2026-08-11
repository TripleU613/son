//! Crawler-facing routes: `robots.txt`, `sitemap.xml` (with the Google
//! Images sitemap extension), and `llms.txt` (the emerging convention for an
//! LLM-readable site summary).
//!
//! The image sitemap extension is one of the few Google-documented,
//! structural levers for ranking in Google Images specifically -- alongside
//! descriptive `alt`/title text (already on every `<img>`) and structured
//! data (the `ImageObject` JSON-LD on the detail page). Sitemaps don't
//! guarantee indexing, but they're the difference between "Google might
//! eventually crawl this" and "Google has an explicit, complete list."

use axum::http::header;
use axum::response::IntoResponse;

use crate::public_route::site_origin;

pub async fn robots_txt() -> impl IntoResponse {
    let origin = site_origin();
    let body = format!(
        "User-agent: *\n\
         Allow: /\n\
         Disallow: /admin\n\
         Disallow: /api/\n\
         Disallow: /auth/\n\
         Sitemap: {origin}/sitemap.xml\n"
    );
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
}

pub async fn llms_txt() -> impl IntoResponse {
    let origin = site_origin();
    let body = format!(
        "# son collection\n\
         \n\
         > Every image is somebody's son. A community gallery of the \"son\" \
         meme and its wordplay variants -- Sonion, Capri-Son, Dy-Son, \
         Sonflower, and anything else with a son hidden in it. Free, \
         anonymous-friendly uploads, moderated by report after \
         publishing.\n\
         \n\
         ## Pages\n\
         \n\
         - [Gallery]({origin}/): every public son, sortable newest / most-liked / A-Z / \"sun level\"\n\
         - [Leaderboard]({origin}/leaderboard): top contributors by upload count\n\
         - [Upload]({origin}/upload): contribute a son, no account required\n\
         - [Search]({origin}/search?q=...): full-text search over titles and tags\n\
         \n\
         ## API\n\
         \n\
         - `GET {origin}/api/v1/sons` -- paginated JSON list of public sons\n\
         - `GET {origin}/api/v1/sons/:id` -- a single public son\n\
         - `GET {origin}/oembed?url=...` -- oEmbed 1.0 metadata for any son page\n\
         - `GET {origin}/sitemap.xml` -- full sitemap, including an image sitemap extension\n"
    );
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
}

/// Escapes the handful of characters that are structurally significant in
/// XML text content. Titles and tag names are free text (only control
/// characters are stripped on the way in — see `upload_route::clean_title`),
/// so `&`/`<` in particular can genuinely appear here.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Comfortably under the sitemap protocol's 50,000-URL-per-file limit, with
/// headroom before a sitemap index (multiple files) is worth the added
/// complexity of building one.
const MAX_SITEMAP_SONS: i64 = 45_000;

pub async fn sitemap_xml() -> impl IntoResponse {
    let origin = site_origin();
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\" \
         xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\">\n",
    );

    // Static routes. `/search` and `/admin` are deliberately absent: search
    // results are per-query (nothing stable to point a crawler at, and the
    // page itself carries a noindex meta tag) and `/admin` is disallowed in
    // robots.txt entirely.
    for (path, priority, changefreq) in [
        ("/", "1.0", "hourly"),
        ("/leaderboard", "0.5", "daily"),
        ("/upload", "0.3", "monthly"),
        ("/privacy", "0.1", "yearly"),
        ("/tos", "0.1", "yearly"),
    ] {
        xml.push_str(&format!(
            "  <url><loc>{origin}{path}</loc><changefreq>{changefreq}</changefreq><priority>{priority}</priority></url>\n"
        ));
    }

    match crate::db::sitemap_tags().await {
        Ok(tags) => {
            for tag in tags {
                xml.push_str(&format!(
                    "  <url><loc>{origin}/tag/{}</loc><changefreq>daily</changefreq><priority>0.4</priority></url>\n",
                    xml_escape(&tag.slug)
                ));
            }
        }
        Err(e) => tracing::error!("sitemap.xml: could not load tags: {e}"),
    }

    match crate::db::sitemap_sons(MAX_SITEMAP_SONS).await {
        Ok(sons) => {
            if sons.len() as i64 >= MAX_SITEMAP_SONS {
                tracing::warn!(
                    "sitemap.xml: hit the {MAX_SITEMAP_SONS}-son cap -- older sons are being \
                     omitted. Time to build a sitemap index instead of one file."
                );
            }
            for son in sons {
                xml.push_str(&format!(
                    "  <url>\n    <loc>{origin}/son/{id}</loc>\n    <lastmod>{lastmod}</lastmod>\n    <image:image>\n      <image:loc>{img}</image:loc>\n      <image:title>{title}</image:title>\n    </image:image>\n  </url>\n",
                    id = xml_escape(&son.id),
                    lastmod = xml_escape(&son.created_at),
                    img = xml_escape(&son.orig_url),
                    title = xml_escape(&son.title),
                ));
            }
        }
        Err(e) => tracing::error!("sitemap.xml: could not load sons: {e}"),
    }

    xml.push_str("</urlset>\n");
    (
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        xml,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escape_covers_the_five_predefined_entities() {
        assert_eq!(
            xml_escape("<Capri & Son> \"quoted\" 'son'"),
            "&lt;Capri &amp; Son&gt; &quot;quoted&quot; &apos;son&apos;"
        );
    }
}
