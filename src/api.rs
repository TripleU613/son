//! Server functions — the typed RPC the wasm bundle calls.
//!
//! The upload itself is NOT here: multipart file bodies go through a plain Axum
//! route (`src/upload_route.rs`) because server functions would have to buffer
//! the whole encoded body through a serde round-trip.

use leptos::prelude::*;

use crate::models::SonPage;

/// One page of the gallery. `cursor` is the previous page's `next_cursor`.
#[server(ListSons, "/api")]
pub async fn list_sons(cursor: Option<String>) -> Result<SonPage, ServerFnError> {
    crate::db::list_public(crate::db::pool(), cursor.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(GetSon, "/api")]
pub async fn get_son(id: String) -> Result<Option<crate::models::Son>, ServerFnError> {
    let son = crate::db::get(crate::db::pool(), &id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // A hidden son must 404 by direct link too, not just vanish from the grid.
    // Direct links are how these spread, so leaving them reachable would make
    // the auto-hide safety valve decorative.
    Ok(son.filter(|s| s.is_public))
}

#[server(TotalSons, "/api")]
pub async fn total_sons() -> Result<i64, ServerFnError> {
    crate::db::count_public(crate::db::pool())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Flag a son for review. Unauthenticated by design — the cost of a false
/// report is one hidden meme, and requiring accounts would mean nobody reports
/// anything. At `AUTO_HIDE_REPORTS` the son hides itself.
#[server(ReportSon, "/api")]
pub async fn report_son(id: String) -> Result<(), ServerFnError> {
    crate::db::report(crate::db::pool(), &id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
