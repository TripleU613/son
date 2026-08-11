//! Server entry point.

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use axum::routing::post;
    use axum::Router;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use tower_http::services::ServeDir;

    use soncollection::app::{shell, App};
    use soncollection::moderation::stub::StubModerator;
    use soncollection::moderation::Moderator;

    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://sons.db".into());
    let db = soncollection::db::connect(&db_url).await?;
    let existing = soncollection::db::count_public(&db).await.unwrap_or(0);
    soncollection::db::set_pool(db);
    tracing::info!("database ready at {db_url} ({existing} sons collected)");

    // R2 when configured, local disk otherwise. Logged either way so it is never
    // a mystery which one served a given upload.
    let storage = soncollection::storage::backend_from_env().await;
    tracing::info!("storage backend: {}", storage.name());
    soncollection::storage::set_backend(storage);

    let moderator: Box<dyn Moderator> = Box::new(StubModerator);
    if moderator.name().contains("NO REAL MODERATION") {
        tracing::warn!(
            "moderation backend is {} — uploads are auto-published with only \
             structural checks. Do not expose this publicly until CLIP is wired in.",
            moderator.name()
        );
    }
    soncollection::upload_route::set_moderator(moderator);

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
