pub mod api;
pub mod app;
pub mod components;
pub mod models;
pub mod seo;

#[cfg(feature = "ssr")]
pub mod auth;
#[cfg(feature = "ssr")]
pub mod d1;
#[cfg(feature = "ssr")]
pub mod db;
#[cfg(feature = "ssr")]
pub mod dedupe;
#[cfg(feature = "ssr")]
pub mod moderation;
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
