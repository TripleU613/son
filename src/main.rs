//! Server entry point.

// See the matching attribute + comment in lib.rs: SonDetail's view! hits
// rustc's default query depth limit in release mode, and this binary crate
// hits the same wall independently of the lib crate.
#![recursion_limit = "512"]

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use axum::routing::{get, post};
    use axum::Router;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use tower_http::services::ServeDir;

    use soncollection::app::{shell, App};

    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // D1 only — no local database. Local dev talks to the same D1 database as
    // production rather than falling back to a divergent local store, so there
    // is exactly one code path to trust (see db.rs).
    let d1 = soncollection::d1::D1::from_env()
        .map_err(|e| anyhow::anyhow!("D1 is not configured: {e}"))?;
    soncollection::db::set_client(d1);
    let existing = soncollection::db::count_public().await?;
    tracing::info!("database ready (D1): {existing} sons collected");

    // R2 when configured, local disk otherwise. Logged either way so it is never
    // a mystery which one served a given upload.
    let storage = soncollection::storage::backend_from_env().await;
    tracing::info!("storage backend: {}", storage.name());
    soncollection::storage::set_backend(storage);

    // Said out loud at every start, because "are uploads being screened?" is
    // exactly the thing that is easy to be wrong about. WARN when they are not:
    // that state is survivable but should never be a surprise.
    match soncollection::gemini::url() {
        Some(url) => tracing::info!("content screening: Gemini sidecar at {url}"),
        None => tracing::warn!(
            "content screening: NONE (GEMINI_URL unset) — uploads publish unscreened; \
             only /admin reports can remove them"
        ),
    }

    if soncollection::auth::google_configured() {
        tracing::info!("Google sign-in: configured");
    } else {
        tracing::info!("Google sign-in: not configured (GOOGLE_CLIENT_ID/SECRET unset) — /auth/google/login will redirect back with an error");
    }

    // cargo-leptos supplies site-addr and site-root through Cargo.toml metadata.
    let conf = get_configuration(None)?;
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(App);

    let app = Router::new()
        // Server functions need their own mount; `.leptos_routes` only registers
        // page routes. Without this, SSR still works (it calls the fns directly
        // in-process) but every client-side call 404s.
        //
        // Declared before the wildcard so the static path wins the match.
        .route("/api/upload", post(soncollection::upload_route::upload))
        .route(
            "/api/upload/status/{id}",
            get(soncollection::upload_route::status),
        )
        // The admin sign-in browser. Every one of these checks is_admin itself --
        // they are plain Axum routes, not Leptos ones, so they get no gating for
        // free. Order matters: the WebSocket path must be declared before the
        // wildcard, or the wildcard swallows it.
        .route("/admin/browser", get(soncollection::browser_proxy::page))
        .route(
            "/admin/browser/websockify",
            get(soncollection::browser_proxy::websocket),
        )
        .route(
            "/admin/browser/{*path}",
            get(soncollection::browser_proxy::asset),
        )
        .route("/auth/google/login", get(soncollection::oauth_route::login))
        .route(
            "/auth/google/callback",
            get(soncollection::oauth_route::callback),
        )
        .route("/auth/logout", post(soncollection::oauth_route::logout))
        // A stable, documented public API -- unlike the server fns below,
        // whose paths are hashed and change on every rebuild.
        .route("/api/v1/sons", get(soncollection::public_route::list_sons))
        .route(
            "/api/v1/sons/{id}",
            get(soncollection::public_route::get_son),
        )
        .route("/oembed", get(soncollection::public_route::oembed))
        // A framable card for one son. A plain route, not a Leptos one:
        // `generate_route_list(App)` knows nothing about `/embed`, so without
        // this line it falls through to `file_and_error_handler` and 404s.
        .route("/embed/{id}", get(soncollection::public_route::embed))
        .route(
            "/son/{id}/download",
            get(soncollection::public_route::download),
        )
        .route("/robots.txt", get(soncollection::seo_route::robots_txt))
        .route("/sitemap.xml", get(soncollection::seo_route::sitemap_xml))
        .route("/llms.txt", get(soncollection::seo_route::llms_txt))
        .route(
            "/api/{*fn_name}",
            axum::routing::any(leptos_axum::handle_server_fns),
        )
        // Uploaded images are served straight off disk; swap for a CDN origin later.
        .nest_service(
            "/uploads",
            ServeDir::new(soncollection::storage::UPLOAD_ROOT),
        )
        .leptos_routes(&leptos_options, routes, {
            let opts = leptos_options.clone();
            move || shell(opts.clone())
        })
        .fallback(leptos_axum::file_and_error_handler(shell))
        // After `.fallback`, before `.with_state`. `Router::layer` wraps every
        // route registered *so far*, fallback included -- and the fallback is
        // what serves `/pkg/` and everything in `public/`, i.e. exactly the
        // responses this layer exists to put a `Cache-Control` on.
        // `.route_layer` would skip the fallback and silently cache nothing.
        //
        // The hashing flag comes from `leptos_options`, never from a second
        // `std::env::var("LEPTOS_HASH_FILES")` read. One value decides both
        // which filenames the HTML asks for and whether those filenames are
        // safe to cache forever; splitting it is the two-switch trap in
        // CLAUDE.md, and getting it wrong in this direction is unrecoverable
        // from the origin, because an `immutable` client never revalidates.
        .layer(axum::middleware::from_fn_with_state(
            leptos_options.hash_files,
            cache::headers,
        ))
        .with_state(leptos_options);

    tracing::info!("son collection listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

/// `cargo-leptos` also builds the lib for wasm, where there is no main to run.
#[cfg(not(feature = "ssr"))]
fn main() {}

/// One `Cache-Control` decision per route class, in one place.
///
/// Before this existed, `curl -sI /` and `curl -sI /pkg/soncollection.css`
/// both came back with no `Cache-Control` at all -- every response was at the
/// mercy of whatever the CDN in front chose to do, which is how a four-hour
/// edge TTL on `/pkg/` once served yesterday's CSS against today's HTML.
///
/// Cloudflare sits in front of this and can still override it: `s-maxage`
/// only binds if the zone is configured to respect origin headers, so the
/// header that actually has to be verified at the edge (not just here) is
/// `private, no-cache` on HTML.
#[cfg(feature = "ssr")]
mod cache {
    use axum::extract::State;
    use axum::http::{header, HeaderName, HeaderValue, Method, Request};
    use axum::middleware::Next;
    use axum::response::Response;

    /// Content-hashed under `hash-files = true`, so the filename changes
    /// whenever the bytes do and the old one is never requested again. A year
    /// plus `immutable` (no revalidation at all, even on reload) is the whole
    /// point of paying for hashed filenames.
    const IMMUTABLE: &str = "public, max-age=31536000, s-maxage=31536000, immutable";

    /// `X-Robots-Tag`, which `http` has no constant for.
    const X_ROBOTS_TAG: HeaderName = HeaderName::from_static("x-robots-tag");

    /// The class of a path, as a `Cache-Control` value.
    ///
    /// `hashed` is `LeptosOptions::hash_files`, i.e. the same value that
    /// decides which filenames the HTML asks for. With it off -- which is what
    /// `cargo leptos watch` does, deliberately -- `/pkg/soncollection.css` is a
    /// fixed name whose contents change on every edit, and `immutable` there
    /// would freeze a stale stylesheet in every visitor's browser for a year
    /// with no way to reach it from the origin.
    pub fn cache_control(path: &str, hashed: bool) -> &'static str {
        if path.starts_with("/pkg/") {
            return if hashed { IMMUTABLE } else { "no-cache" };
        }
        // Local-disk storage only (R2 serves its own origin in production).
        // An upload's bytes never change: the id is in the key and a re-upload
        // is a new son.
        if path.starts_with("/uploads/") {
            return "public, max-age=3600";
        }
        if path == "/robots.txt" || path == "/llms.txt" {
            return "public, max-age=3600";
        }
        if path == "/sitemap.xml" {
            return "public, max-age=600";
        }
        // Anonymous by construction: `list_sons` passes `None` for the voter,
        // so `liked_by_me` is always false and there is nothing per-visitor in
        // the body to leak between users of a shared cache.
        if path == "/oembed" || path.starts_with("/api/v1/") {
            return "public, max-age=60";
        }
        if path.starts_with("/son/") && path.ends_with("/download") {
            return "public, max-age=3600";
        }
        // Sessions, moderation and the upload form. `no-store` rather than
        // `no-cache`: not "revalidate", but "do not write this to disk."
        if path.starts_with("/admin")
            || path.starts_with("/upload")
            || path.starts_with("/auth/")
            || path.starts_with("/api/")
        {
            return "no-store";
        }
        // Unhashed static files served out of `public/`: favicon-32.png,
        // apple-touch-icon.png, logo.png, logo-large.png. A day, not a year --
        // without a hash in the name, `immutable` would make replacing one
        // impossible.
        if [".png", ".ico", ".svg", ".webp", ".woff2"]
            .iter()
            .any(|ext| path.ends_with(ext))
        {
            return "public, max-age=86400";
        }
        // Everything else is server-rendered HTML.
        //
        // `private` is not cosmetic. With `SsrMode::Async` on `/`, the entire
        // document is rendered before the first flush, `AccountAction`
        // included -- a signed-out `/` already contains `aria-label="Sign in"`
        // in the served HTML, which means a signed-in visitor's display name
        // and Google avatar URL are in the body of theirs. A shared cache must
        // never store that. Do not relax this to `public, s-maxage=...` for
        // gallery performance without first moving the account control out of
        // the SSR'd body.
        "private, no-cache"
    }

    /// Paths that should never appear in a search index: byte-identical copies
    /// of images already crawlable at their media origin, machine-readable
    /// duplicates of pages that are indexable in their own right, and the
    /// embed card, which is a thin duplicate of the son page it links to.
    ///
    /// A header rather than (only) a robots.txt `Disallow`, because these are
    /// not all HTML and a `<meta name="robots">` has nowhere to live in a PNG
    /// or a JSON body.
    pub fn noindex(path: &str) -> bool {
        path.starts_with("/api/")
            || path == "/oembed"
            || path.starts_with("/uploads/")
            || path.starts_with("/embed/")
            || (path.starts_with("/son/") && path.ends_with("/download"))
    }

    /// Applies the above to every response, without ever overwriting a header
    /// a handler set for itself.
    ///
    /// `/embed/:id` sets its own `Cache-Control` (300 on a hit, `no-store` on
    /// a miss, so a moderation removal is not masked by an edge-cached copy)
    /// and `/son/:id/download` depends on its `Content-Disposition`. Insert
    /// only when absent; never touch `Content-Type` or `Content-Disposition`.
    pub async fn headers(
        State(hashed): State<bool>,
        req: Request<axum::body::Body>,
        next: Next,
    ) -> Response {
        let path = req.uri().path().to_string();
        let idempotent = matches!(*req.method(), Method::GET | Method::HEAD);

        let mut resp = next.run(req).await;
        let h = resp.headers_mut();

        // Two overrides, both deliberate. A response that sets a cookie is
        // handing out an identity, and a response to a mutating request is not
        // a resource anyone should keep -- in both cases a handler's own
        // `Cache-Control` would be the bug, not the thing to preserve.
        if !idempotent || h.contains_key(header::SET_COOKIE) {
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        } else if !h.contains_key(header::CACHE_CONTROL) {
            h.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(cache_control(&path, hashed)),
            );
        }

        if noindex(&path) && !h.contains_key(&X_ROBOTS_TAG) {
            h.insert(X_ROBOTS_TAG, HeaderValue::from_static("noindex"));
        }

        // Global, because user-uploaded PNGs are proxied through
        // `/son/:id/download` on this origin: without it a browser that
        // disagrees with the declared type gets to guess, and a guess of
        // `text/html` on attacker-supplied bytes is same-origin script.
        if !h.contains_key(header::X_CONTENT_TYPE_OPTIONS) {
            h.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
        }

        resp
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hashed_pkg_assets_are_immutable_and_unhashed_ones_are_not() {
            assert_eq!(cache_control("/pkg/soncollection.abc123.css", true), IMMUTABLE);
            // The `cargo leptos watch` case: one fixed filename, new contents
            // every edit. A year here is unrecoverable.
            assert_eq!(cache_control("/pkg/soncollection.css", false), "no-cache");
            assert_eq!(cache_control("/pkg/soncollection.wasm", false), "no-cache");
        }

        #[test]
        fn html_is_private_and_never_stored_by_a_shared_cache() {
            for path in ["/", "/son/sonion-powder", "/leaderboard", "/search", "/tos"] {
                assert_eq!(cache_control(path, true), "private, no-cache", "{path}");
            }
        }

        #[test]
        fn identity_and_moderation_routes_are_no_store() {
            for path in [
                "/admin",
                "/admin/browser",
                "/upload",
                "/api/upload",
                "/auth/google/callback",
                "/api/like_son1234",
            ] {
                assert_eq!(cache_control(path, true), "no-store", "{path}");
            }
        }

        #[test]
        fn public_data_routes_are_shareable_but_short_lived() {
            assert_eq!(cache_control("/api/v1/sons", true), "public, max-age=60");
            assert_eq!(cache_control("/oembed", true), "public, max-age=60");
            assert_eq!(cache_control("/sitemap.xml", true), "public, max-age=600");
            assert_eq!(cache_control("/robots.txt", true), "public, max-age=3600");
            assert_eq!(cache_control("/llms.txt", true), "public, max-age=3600");
            assert_eq!(
                cache_control("/son/sonion-powder/download", true),
                "public, max-age=3600"
            );
            assert_eq!(
                cache_control("/uploads/orig/x.png", true),
                "public, max-age=3600"
            );
        }

        /// `/api/v1/` has to be matched before the `/api/` no-store rule, and
        /// an unhashed icon before the HTML fallthrough.
        #[test]
        fn the_specific_class_wins_over_the_general_one() {
            assert_eq!(cache_control("/api/v1/sons/x", true), "public, max-age=60");
            assert_eq!(cache_control("/favicon-32.png", true), "public, max-age=86400");
            assert_eq!(
                cache_control("/apple-touch-icon.png", false),
                "public, max-age=86400"
            );
            // Inside /pkg/ the hashing rule wins, extension notwithstanding.
            assert_eq!(cache_control("/pkg/logo.svg", false), "no-cache");
        }

        #[test]
        fn noindex_covers_the_duplicates_and_nothing_else() {
            assert!(noindex("/embed/sonion-powder"));
            assert!(noindex("/son/sonion-powder/download"));
            assert!(noindex("/api/v1/sons"));
            assert!(noindex("/oembed"));
            assert!(noindex("/uploads/orig/x.png"));
            // The pages that must stay indexable.
            assert!(!noindex("/"));
            assert!(!noindex("/son/sonion-powder"));
            assert!(!noindex("/leaderboard"));
        }
    }
}
