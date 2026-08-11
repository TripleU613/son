//! Server functions — the typed RPC the wasm bundle calls.
//!
//! The upload itself is NOT here: multipart file bodies go through a plain Axum
//! route (`src/upload_route.rs`) because server functions would have to buffer
//! the whole encoded body through a serde round-trip.

use leptos::prelude::*;

use crate::models::{Son, SonPage};
// Sort::from_str_or_default only runs server-side (the client sends the sort
// as a plain string over the wire), so the type itself is ssr-only here.
#[cfg(feature = "ssr")]
use crate::models::Sort;

/// Name of the cookie holding the anonymous voter ID.
#[cfg(feature = "ssr")]
pub const VOTER_COOKIE: &str = "son_voter";

/// Read the caller's voter ID, if they have one.
///
/// Deliberately does not mint one: a plain page view should not set a cookie.
/// The ID is issued on the first like, so read-only visitors stay cookie-free.
#[cfg(feature = "ssr")]
async fn current_voter() -> Option<String> {
    use axum::http::header::COOKIE;

    // Request parts reach a server fn through leptos_axum::extract, not context.
    let headers: axum::http::HeaderMap = leptos_axum::extract().await.ok()?;
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    raw.split(';')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| k.trim() == VOTER_COOKIE)
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Issue a voter ID and set it on the response.
///
/// HttpOnly because nothing client-side needs to read it, and Lax so the cookie
/// survives someone arriving from a shared link.
#[cfg(feature = "ssr")]
fn issue_voter() -> String {
    use axum::http::header::SET_COOKIE;
    use axum::http::HeaderValue;

    let id = uuid::Uuid::new_v4().to_string();
    let secure = std::env::var("SITE_ORIGIN")
        .map(|o| o.starts_with("https://"))
        .unwrap_or(false);

    let mut cookie =
        format!("{VOTER_COOKIE}={id}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax");
    if secure {
        cookie.push_str("; Secure");
    }

    if let Some(resp) = use_context::<leptos_axum::ResponseOptions>() {
        if let Ok(v) = HeaderValue::from_str(&cookie) {
            resp.append_header(SET_COOKIE, v);
        }
    }
    id
}

/// One page of the gallery. `cursor` is the previous page's `next_cursor`.
#[server(ListSons, "/api")]
pub async fn list_sons(
    cursor: Option<String>,
    sort: Option<String>,
) -> Result<SonPage, ServerFnError> {
    let sort = sort
        .as_deref()
        .map(Sort::from_str_or_default)
        .unwrap_or_default();
    let voter = current_voter().await;

    crate::db::list_public(crate::db::pool(), cursor.as_deref(), sort, voter.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(GetSon, "/api")]
pub async fn get_son(id: String) -> Result<Option<Son>, ServerFnError> {
    let voter = current_voter().await;
    let son = crate::db::get(crate::db::pool(), &id, voter.as_deref())
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

/// Toggle a like. Returns `(new_count, liked_now)`.
///
/// Anonymous: identity is a cookie, minted here on first use. Bypassable by
/// clearing cookies, which is the accepted trade for not storing visitor IPs.
#[server(LikeSon, "/api")]
pub async fn like_son(id: String) -> Result<(i64, bool), ServerFnError> {
    let voter = match current_voter().await {
        Some(v) => v,
        None => issue_voter(),
    };

    crate::db::toggle_like(crate::db::pool(), &id, &voter)
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
