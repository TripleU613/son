//! Server functions — the typed RPC the wasm bundle calls.
//!
//! The upload itself is NOT here: multipart file bodies go through a plain Axum
//! route (`src/upload_route.rs`) because server functions would have to buffer
//! the whole encoded body through a serde round-trip.

use leptos::prelude::*;

use crate::models::{Son, SonPage, User};
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

    crate::db::list_public(cursor.as_deref(), sort, voter.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(GetSon, "/api")]
pub async fn get_son(id: String) -> Result<Option<Son>, ServerFnError> {
    let voter = current_voter().await;
    let son = crate::db::get(&id, voter.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    // A hidden son must 404 by direct link too, not just vanish from the grid.
    // Direct links are how these spread, so leaving them reachable would make
    // the auto-hide safety valve decorative.
    Ok(son.filter(|s| s.is_public))
}

#[server(TotalSons, "/api")]
pub async fn total_sons() -> Result<i64, ServerFnError> {
    crate::db::count_public()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(Leaderboard, "/api")]
pub async fn leaderboard() -> Result<Vec<crate::models::LeaderboardEntry>, ServerFnError> {
    crate::db::leaderboard(50)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(SonOfTheDay, "/api")]
pub async fn son_of_the_day() -> Result<Option<Son>, ServerFnError> {
    crate::db::son_of_the_day()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(SearchSons, "/api")]
pub async fn search_sons(query: String) -> Result<Vec<Son>, ServerFnError> {
    let voter = current_voter().await;
    crate::db::search_sons(&query, voter.as_deref())
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

    crate::db::toggle_like(&id, &voter)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// The raw Cookie header for the in-flight request. Shared by every server fn
/// that needs to read a cookie (voter, session), so the `leptos_axum::extract`
/// dance lives in exactly one place.
#[cfg(feature = "ssr")]
async fn cookie_header() -> Option<String> {
    let headers: axum::http::HeaderMap = leptos_axum::extract().await.ok()?;
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Who, if anyone, is signed in — for the nav to show a sign-in link or an
/// avatar. `Ok(None)` covers both "never signed in" and "Google sign-in isn't
/// configured yet"; the nav doesn't need to tell those apart.
#[server(CurrentUser, "/api")]
pub async fn current_user() -> Result<Option<User>, ServerFnError> {
    crate::auth::current_user(cookie_header().await.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Require an admin session, for every admin-only server fn below. Checked
/// here, not just hidden behind a UI element — a button no ordinary visitor
/// sees is not the same thing as a request no ordinary visitor can make, and
/// the admin route is otherwise a plain server fn like any other.
#[cfg(feature = "ssr")]
async fn require_admin() -> Result<User, ServerFnError> {
    let user = crate::auth::current_user(cookie_header().await.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    match user {
        Some(u) if u.is_admin => Ok(u),
        _ => Err(ServerFnError::new("admin access required")),
    }
}

/// Flag a son for review. Unauthenticated by design — the cost of a false
/// report is one hidden meme, and requiring accounts would mean nobody reports
/// anything. Identity is the same anonymous voter cookie likes already use, so
/// one visitor can't spam-report the same son to force auto-hide alone.
#[server(ReportSon, "/api")]
pub async fn report_son(
    id: String,
    reason: String,
    message: Option<String>,
) -> Result<(), ServerFnError> {
    let voter = match current_voter().await {
        Some(v) => v,
        None => issue_voter(),
    };
    let reason = crate::models::ReportReason::from_str_or_default(&reason);
    // A blank textarea should store as absent, not as an empty string forever
    // shown in the queue.
    let message = message
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());

    crate::db::report(&id, &voter, reason.as_str(), message.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(AdminFlaggedSons, "/api")]
pub async fn admin_flagged_sons() -> Result<Vec<crate::models::FlaggedSon>, ServerFnError> {
    require_admin().await?;
    crate::db::flagged_sons()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(AdminSetPublic, "/api")]
pub async fn admin_set_public(id: String, public: bool) -> Result<(), ServerFnError> {
    require_admin().await?;
    crate::db::set_public(&id, public)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Removes the row and its R2 objects. There is no undo — the confirmation
/// happens client-side (see the admin page), not here, since a server fn has
/// no way to ask "are you sure" mid-request.
#[server(AdminDeleteSon, "/api")]
pub async fn admin_delete_son(id: String) -> Result<(), ServerFnError> {
    require_admin().await?;
    crate::storage::remove(&id).await;
    crate::db::delete_son(&id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
