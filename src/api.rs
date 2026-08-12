//! Server functions — the typed RPC the wasm bundle calls.
//!
//! The upload itself is NOT here: multipart file bodies go through a plain Axum
//! route (`src/upload_route.rs`) because server functions would have to buffer
//! the whole encoded body through a serde round-trip.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::models::{Son, SonPage, User};
// Sort::from_str_or_default only runs server-side (the client sends the sort
// as a plain string over the wire), so the type itself is ssr-only here.
#[cfg(feature = "ssr")]
use crate::models::Sort;

/// Who is acting on this request, for everything keyed to a person: which sons
/// come back marked as already liked, and whether a like or a report is allowed
/// at all.
///
/// This used to be an anonymous ID in a `son_voter` cookie, minted on the first
/// like. It is now the signed-in user's id, because an identity the holder can
/// reissue at will is not one a write can be attributed to: clearing that cookie
/// between clicks let one person like a son unboundedly, and — worse, since
/// auto-hide is the *primary* moderation mechanism here — let one person file
/// the three reports that pull any son out of the gallery.
///
/// Reads use the same identity, so `liked_by_me` describes the account and not
/// the browser. A signed-out visitor therefore sees every son as un-liked, which
/// is the honest answer now that they cannot un-like one: the alternative is a
/// filled-in tear that does nothing when clicked.
///
/// Rows already written against anonymous voter ids stay exactly where they are.
/// They keep counting — `sons.likes` and `sons.reports` are recomputed from
/// `COUNT(*)` rather than incremented, so nothing double-counts and no total
/// moves — they are simply no longer attributable to anyone, and so nobody can
/// un-like them. Deleting them would quietly discard real likes to make the data
/// model tidier, which is the wrong way round.
#[cfg(feature = "ssr")]
async fn current_actor() -> Option<String> {
    crate::auth::session_user_id(cookie_header().await.as_deref())
}

/// Where to send someone who has to sign in before an action will work.
///
/// The same shape as the header's sign-in link, so both land the visitor back on
/// the page they were reading. `return_to` is always a same-origin path and is
/// re-checked as one server-side in `oauth_route::login` — the encoding here is
/// only to stop a path with a `?`, `&` or `#` in it truncating the query value.
pub fn sign_in_href(return_to: &str) -> String {
    let encoded: String = return_to
        .chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect();
    format!("/auth/google/login?return_to={encoded}")
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
    let actor = current_actor().await;

    crate::db::list_public(cursor.as_deref(), sort, actor.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server(GetSon, "/api")]
pub async fn get_son(id: String) -> Result<Option<Son>, ServerFnError> {
    let actor = current_actor().await;
    let son = crate::db::get(&id, actor.as_deref())
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
    let actor = current_actor().await;
    crate::db::search_sons(&query, actor.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// The sons either side of this one, as `(newer, older)` slugs, for stepping
/// between detail pages without going back to the grid.
///
/// A plain tuple rather than a named struct: it needs no new shared type, and
/// the two positions are named at every call site by the `let (newer, older)`
/// that receives them. Takes the same slug-or-id string the route carries, like
/// `get_son`. Nothing here is per-visitor, so unlike `get_son` it is identical
/// for everyone and cheap to serve.
#[server(SonNeighbours, "/api")]
pub async fn son_neighbours(id: String) -> Result<(Option<String>, Option<String>), ServerFnError> {
    crate::db::neighbours(&id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// What a like attempt actually did.
///
/// `SignInRequired` is a variant rather than an `Err`, because the caller has to
/// tell "you need an account" apart from "that request failed" and those are
/// different UI: the first is a sign-in link, the second is a rollback and a log
/// line. Folded into one `ServerFnError` they can only be told apart by matching
/// on message text, and whichever way that match goes wrong, a signed-out
/// visitor ends up with a button that appears to do nothing — the exact dead
/// control requiring sign-in was meant to remove.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LikeOutcome {
    /// Recorded. `count` is re-read from the database, not the client's guess.
    Toggled { count: i64, liked: bool },
    /// Nobody is signed in. Nothing was written.
    SignInRequired,
}

/// Toggle a like. Signed-in visitors only.
///
/// Enforced here rather than by hiding the button, because a server function is
/// a plain HTTP endpoint that anything can POST to: a control no signed-out
/// visitor sees is not the same thing as a request no signed-out visitor can
/// make, and only the second one is a gate.
#[server(LikeSon, "/api")]
pub async fn like_son(id: String) -> Result<LikeOutcome, ServerFnError> {
    let Some(actor) = current_actor().await else {
        return Ok(LikeOutcome::SignInRequired);
    };

    let (count, liked) = crate::db::toggle_like(&id, &actor)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(LikeOutcome::Toggled { count, liked })
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

/// Same two states as `LikeOutcome`, for the same reason: the form has to show
/// a sign-in link, not "Flagged. Someone will look." over a report nobody filed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportOutcome {
    Recorded,
    SignInRequired,
}
/// Flag a son for review. Signed-in visitors only, as of this change.
///
/// It used to be open, on the reasoning that a false report only costs one
/// hidden meme and that requiring accounts means nobody reports anything. What
/// changed is the other side of that trade: `/admin` and the three-report
/// auto-hide are now the primary moderation mechanism rather than a backstop, so
/// `reports`' one-per-voter primary key is the only thing standing between one
/// annoyed visitor and any son they like being pulled from the gallery — and
/// against a self-issued cookie that key is worth nothing, since three clears of
/// `son_voter` were three distinct voters. Against an account it is worth what
/// it claims to be.
#[server(ReportSon, "/api")]
pub async fn report_son(
    id: String,
    reason: String,
    message: Option<String>,
) -> Result<ReportOutcome, ServerFnError> {
    let Some(actor) = current_actor().await else {
        return Ok(ReportOutcome::SignInRequired);
    };
    let reason = crate::models::ReportReason::from_str_or_default(&reason);
    // A blank textarea should store as absent, not as an empty string forever
    // shown in the queue.
    let message = message
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());

    crate::db::report(&id, &actor, reason.as_str(), message.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(ReportOutcome::Recorded)
}

#[server(AdminFlaggedSons, "/api")]
pub async fn admin_flagged_sons() -> Result<Vec<crate::models::FlaggedSon>, ServerFnError> {
    require_admin().await?;
    crate::db::flagged_sons()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// How screening is doing, for the admin page. Admin-only: it reports the state
/// of a credential, which is nobody else's business.
#[server(AdminScreeningStatus, "/api")]
pub async fn admin_screening_status() -> Result<crate::models::ScreeningStatus, ServerFnError> {
    require_admin().await?;
    Ok(crate::gemini::status().await)
}

/// Hand the sidecar fresh Gemini cookies, live.
///
/// The alternative is editing a GitHub secret and waiting out a full CI deploy to
/// restore screening, which is ~12 minutes of uploads piling up in the held
/// queue. This is seconds. The sidecar refuses cookies that cannot authenticate,
/// so a bad paste cannot take working screening down.
#[server(AdminSetGeminiCookies, "/api")]
pub async fn admin_set_gemini_cookies(cookies: String) -> Result<u32, ServerFnError> {
    require_admin().await?;
    crate::gemini::set_cookies(&cookies)
        .await
        .map_err(ServerFnError::new)
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
