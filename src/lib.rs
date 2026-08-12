// SonDetail's view! (Title/Meta/Link/script siblings for OG, Twitter, JSON-LD,
// and oEmbed discovery, plus the article body) generates deeply nested
// tachys types -- release-mode monomorphization hits rustc's default query
// depth limit (128) here specifically, even though debug builds never do.
// Found via a real CI failure across multiple pushes, not a hypothetical.
#![recursion_limit = "512"]

pub mod api;
pub mod app;
pub mod components;
pub mod models;
pub mod seo;

#[cfg(feature = "ssr")]
pub mod auth;
/// Admin-only proxy to the keeper's sign-in browser.
#[cfg(feature = "ssr")]
pub mod browser_proxy;
#[cfg(feature = "ssr")]
pub mod d1;
#[cfg(feature = "ssr")]
pub mod db;
#[cfg(feature = "ssr")]
pub mod dedupe;
/// Gemini screening + squaring, via the Python sidecar (see sidecar/).
#[cfg(feature = "ssr")]
pub mod gemini;
/// In-memory upload progress, polled by the upload page.
#[cfg(feature = "ssr")]
pub mod jobs;

#[cfg(feature = "ssr")]
pub mod oauth_route;
#[cfg(feature = "ssr")]
pub mod public_route;
#[cfg(feature = "ssr")]
pub mod seo_route;
#[cfg(feature = "ssr")]
pub mod storage;
#[cfg(feature = "ssr")]
pub mod upload_route;
#[cfg(feature = "ssr")]
pub mod watermark;

/// Wasm entry point. `cargo-leptos` wires this into the generated JS loader.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
