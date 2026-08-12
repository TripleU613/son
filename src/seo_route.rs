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
use crate::seo::w3c_lastmod;

/// One group, `User-agent: *`.
///
/// There is deliberately no AI-crawler group (GPTBot, CCBot, ClaudeBot,
/// Google-Extended). That is a decision, not an omission: this site publishes
/// an `llms.txt` describing itself for exactly those readers, and the content
/// is a meme collection that is meant to spread. Someone re-reading this file
/// should know it was considered.
///
/// Two things are pointedly *not* disallowed, and both would be mistakes:
///
/// - `/embed/` — a `Disallow` stops a crawler fetching the URL at all, so it
///   never reads the `noindex, follow` the embed page serves, and the URL
///   stays eligible for indexing as a thin duplicate on the strength of
///   inbound links alone. Blocking and noindexing the same path are mutually
///   exclusive; the page carries the tag, so robots must let it be read.
/// - `/search` — same mechanism (it already serves `noindex, follow`), and
///   blocking it would additionally throw away the `follow` that lets a
///   crawler reach individual sons through result links.
///
/// `/oembed`, `/uploads/` and `/son/*/download` *are* disallowed even though
/// they also carry `X-Robots-Tag: noindex`, and that is not the same
/// contradiction. The aim there is not to be fetched at all: the download
/// proxy is a byte-identical copy of an image already crawlable at its
/// media.soncollection.com URL, and serving it twice costs real R2 egress.
/// The header is only a fallback for a crawler that ignores this file.
///
/// `*` wildcards are honoured by Google and Bing and ignored (harmlessly, as a
/// literal path) by everything else.
pub async fn robots_txt() -> impl IntoResponse {
    let origin = site_origin();
    let body = format!(
        "User-agent: *\n\
         Allow: /\n\
         Disallow: /admin\n\
         Disallow: /api/\n\
         Disallow: /auth/\n\
         Disallow: /oembed\n\
         Disallow: /uploads/\n\
         Disallow: /son/*/download\n\
         Sitemap: {origin}/sitemap.xml\n"
    );
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            // The middleware in main.rs would set the same value; stated here
            // because this handler already owns its header list and one line
            // is cheaper than making a reader go and check.
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
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
         - `GET {origin}/embed/:slug` -- a standalone, framable card for one son (iframe it; noindex, no JavaScript)\n\
         - `GET {origin}/oembed?url=...` -- oEmbed 1.0 metadata. A `/son/` URL returns `type: photo`; an `/embed/` URL returns `type: rich` with ready-made iframe HTML\n\
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

/// A `<lastmod>` element, or nothing at all.
///
/// `w3c_lastmod` returns `None` for a timestamp it cannot normalise, and an
/// invalid `<lastmod>` is strictly worse than an absent one: Search Console
/// reports the whole URL entry as an error rather than ignoring the date.
fn lastmod_tag(raw: &str) -> String {
    w3c_lastmod(raw)
        .map(|d| format!("<lastmod>{}</lastmod>", xml_escape(&d)))
        .unwrap_or_default()
}

pub async fn sitemap_xml() -> impl IntoResponse {
    let origin = site_origin();

    // Loaded first, because the two collection-driven static pages take their
    // `<lastmod>` from the newest son. `sitemap_sons` is already ordered
    // `created_at DESC`, so that is `first()` -- no second query.
    let sons = match crate::db::sitemap_sons(MAX_SITEMAP_SONS).await {
        Ok(sons) => {
            if sons.len() as i64 >= MAX_SITEMAP_SONS {
                tracing::warn!(
                    "sitemap.xml: hit the {MAX_SITEMAP_SONS}-son cap -- older sons are being \
                     omitted. Time to build a sitemap index instead of one file."
                );
            }
            sons
        }
        Err(e) => {
            tracing::error!("sitemap.xml: could not load sons: {e}");
            Vec::new()
        }
    };
    let newest = sons.first().map(|s| lastmod_tag(&s.created_at));

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\" \
         xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\">\n",
    );

    // Static routes. `/search`, `/admin` and `/embed/` are deliberately
    // absent: search results are per-query (nothing stable to point a crawler
    // at, and the page itself carries a noindex meta tag), `/admin` is
    // disallowed in robots.txt entirely, and an embed card is a noindex
    // duplicate of the son page that already has its own entry below.
    //
    // `<changefreq>` and `<priority>` are ignored by Google outright. They are
    // kept because they cost nothing, other consumers of the protocol still
    // read them, and removing them is churn with no upside.
    //
    // The bool is "does this page change when the collection does" -- only
    // those two get the newest son's timestamp. `/tos` did not change because
    // somebody uploaded a son, and claiming otherwise trains a crawler to
    // ignore `<lastmod>` on this site.
    for (path, priority, changefreq, tracks_collection) in [
        ("/", "1.0", "hourly", true),
        ("/leaderboard", "0.5", "daily", true),
        ("/upload", "0.3", "monthly", false),
        ("/privacy", "0.1", "yearly", false),
        ("/tos", "0.1", "yearly", false),
    ] {
        let lastmod = if tracks_collection {
            newest.clone().unwrap_or_default()
        } else {
            String::new()
        };
        xml.push_str(&format!(
            "  <url><loc>{origin}{path}</loc>{lastmod}<changefreq>{changefreq}</changefreq><priority>{priority}</priority></url>\n"
        ));
    }

    for son in sons {
        // `SitemapSon::id` is the slug where there is one (`db::sitemap_sons`
        // maps it), so this `<loc>` matches the canonical the detail page
        // emits. Two different URLs for one son in a sitemap and a canonical
        // is how a page gets crawled twice and indexed once, at random.
        //
        // `<image:title>` is kept, but do not add `image:caption` or
        // `image:license` expecting an effect: Google deprecated all three in
        // 2022 and now reads only `<image:loc>` from this extension. It stays
        // for the non-Google consumers that still parse it.
        let lastmod = match lastmod_tag(&son.created_at) {
            tag if tag.is_empty() => String::new(),
            tag => format!("    {tag}\n"),
        };
        xml.push_str(&format!(
            "  <url>\n    <loc>{origin}/son/{id}</loc>\n{lastmod}    <image:image>\n      <image:loc>{img}</image:loc>\n      <image:title>{title}</image:title>\n    </image:image>\n  </url>\n",
            id = xml_escape(&son.id),
            img = xml_escape(&son.orig_url),
            title = xml_escape(&son.title),
        ));
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

    /// The whole point of routing through `w3c_lastmod`: a timestamp the
    /// protocol would reject produces no tag rather than a broken one.
    #[test]
    fn lastmod_tag_is_empty_rather_than_invalid() {
        assert_eq!(
            lastmod_tag("2026-08-12T00:15:42.835017700+00:00"),
            "<lastmod>2026-08-12T00:15:42+00:00</lastmod>"
        );
        assert_eq!(
            lastmod_tag("2025-01-09 13:04:55"),
            "<lastmod>2025-01-09T13:04:55Z</lastmod>"
        );
        assert_eq!(lastmod_tag(""), "");
        assert_eq!(lastmod_tag("whenever"), "");
    }
}
