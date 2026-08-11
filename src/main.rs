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

    // Said out loud at startup, every start, because it is the kind of thing
    // that is easy to forget is true: nothing inspects an upload's contents.
    tracing::warn!(
        "content moderation: NONE — uploads publish unscreened; only /admin reports can remove them"
    );

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
        .with_state(leptos_options);

    tracing::info!("son collection listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

/// `cargo-leptos` also builds the lib for wasm, where there is no main to run.
#[cfg(not(feature = "ssr"))]
fn main() {}
