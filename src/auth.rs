//! Google sign-in and session cookies.
//!
//! Two cookies exist, both AES-GCM encrypted via the `cookie` crate's private
//! jar (used directly, not through axum-extra's `PrivateCookieJar` extractor —
//! this matches how `api.rs` already handles the anonymous voter cookie by
//! hand, rather than mixing two different cookie-handling styles):
//!
//! - `oauth_flow`: short-lived, holds the PKCE verifier and CSRF state between
//!   `/auth/google/login` and `/auth/google/callback`. Never sent anywhere but
//!   back to this server.
//! - `session`: long-lived, holds the signed-in user's id. Nothing but the id
//!   lives in it — email/display name/admin status are looked up fresh from
//!   `users` on each request that needs them, so revoking access (or changing
//!   `is_admin`) takes effect immediately rather than waiting out a stale cookie.
//!
//! Google sign-in is optional infrastructure: every function here degrades to
//! "not configured" rather than panicking when `GOOGLE_CLIENT_ID`/
//! `GOOGLE_CLIENT_SECRET` are unset, so the rest of the app works unchanged
//! before those exist.

use cookie::{Cookie, CookieJar, Key, SameSite};
use serde::{Deserialize, Serialize};

use crate::models::User;

const SESSION_COOKIE: &str = "session";
const FLOW_COOKIE: &str = "oauth_flow";

/// Derived once from `SESSION_SECRET` via HKDF, not read directly as a raw
/// key — this lets the env var be any sufficiently long random string rather
/// than requiring an exact 64-byte encoding.
fn session_key() -> Option<Key> {
    let secret = std::env::var("SESSION_SECRET").ok()?;
    if secret.len() < 32 {
        tracing::error!(
            "SESSION_SECRET is only {} bytes; need at least 32. Sessions are disabled until it is longer.",
            secret.len()
        );
        return None;
    }
    Some(Key::derive_from(secret.as_bytes()))
}

pub fn google_configured() -> bool {
    std::env::var("GOOGLE_CLIENT_ID").is_ok() && std::env::var("GOOGLE_CLIENT_SECRET").is_ok()
}

fn secure_cookies() -> bool {
    std::env::var("SITE_ORIGIN")
        .map(|o| o.starts_with("https://"))
        .unwrap_or(false)
}

fn parse_jar(cookie_header: Option<&str>) -> CookieJar {
    let mut jar = CookieJar::new();
    if let Some(header) = cookie_header {
        for part in header.split(';') {
            if let Ok(c) = Cookie::parse(part.trim().to_string()) {
                jar.add_original(c);
            }
        }
    }
    jar
}

/// State carried between `/auth/google/login` and the callback: the PKCE
/// verifier (proves this callback belongs to the request that started it) and
/// a CSRF token (proves Google, not an attacker, sent the callback).
#[derive(Serialize, Deserialize)]
struct FlowState {
    pkce_verifier: String,
    csrf_state: String,
    /// Where to send the user after a successful login — the page they were
    /// on, not always `/`.
    return_to: String,
}

/// Build the redirect to Google's consent screen, plus the `Set-Cookie` value
/// that must accompany it. Returns `None` if Google sign-in isn't configured.
pub fn start_login(return_to: &str) -> Option<(String, String)> {
    let client_id = std::env::var("GOOGLE_CLIENT_ID").ok()?;
    let key = session_key()?;

    let pkce_verifier = random_url_safe(64);
    let csrf_state = random_url_safe(32);
    let challenge = pkce_challenge(&pkce_verifier);

    let flow = FlowState {
        pkce_verifier,
        csrf_state: csrf_state.clone(),
        return_to: return_to.to_string(),
    };
    let flow_json = serde_json::to_string(&flow).ok()?;

    let mut jar = CookieJar::new();
    let mut c = Cookie::new(FLOW_COOKIE, flow_json);
    c.set_path("/");
    c.set_http_only(true);
    c.set_same_site(SameSite::Lax);
    c.set_secure(secure_cookies());
    // Long enough to survive a slow consent screen, short enough that a
    // half-finished login attempt doesn't linger indefinitely.
    c.set_max_age(cookie::time::Duration::minutes(10));
    jar.private_mut(&key).add(c);

    let redirect_uri = callback_url();
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={client_id}&redirect_uri={redirect_uri}&response_type=code&\
         scope=openid%20email%20profile&state={csrf_state}&\
         code_challenge={challenge}&code_challenge_method=S256&\
         access_type=online&prompt=select_account"
    );

    let set_cookie = jar.delta().next()?.to_string();
    Some((auth_url, set_cookie))
}

/// Result of a completed callback: the user, a `Set-Cookie` for the real
/// session, a `Set-Cookie` clearing the flow cookie, and where to redirect.
pub struct LoginResult {
    pub user: User,
    pub session_set_cookie: String,
    pub clear_flow_set_cookie: String,
    pub return_to: String,
}

/// Verify the callback, exchange the code, fetch the profile, and upsert the
/// user. Any failure (bad state, expired flow cookie, Google error) is a
/// plain `Err` with a message safe to log — never a panic, since this runs on
/// a request path an attacker can hit directly with arbitrary query params.
pub async fn complete_login(
    cookie_header: Option<&str>,
    query_code: &str,
    query_state: &str,
) -> anyhow::Result<LoginResult> {
    let key = session_key().ok_or_else(|| anyhow::anyhow!("SESSION_SECRET not configured"))?;
    let client_id = std::env::var("GOOGLE_CLIENT_ID")?;
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET")?;

    let jar = parse_jar(cookie_header);
    let flow_cookie = jar
        .private(&key)
        .get(FLOW_COOKIE)
        .ok_or_else(|| anyhow::anyhow!("no oauth_flow cookie — flow expired or cookies blocked"))?;
    let flow: FlowState = serde_json::from_str(flow_cookie.value())?;

    // Constant-time-ish is not the point here (state isn't a secret an
    // attacker gains from timing), but it must match exactly.
    if flow.csrf_state != query_state {
        anyhow::bail!("state mismatch — possible CSRF, aborting login");
    }

    let http = reqwest::Client::new();
    let token_resp: GoogleTokenResponse = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", query_code),
            ("code_verifier", flow.pkce_verifier.as_str()),
            ("redirect_uri", &callback_url()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let profile: GoogleProfile = http
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(&token_resp.access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let user = crate::db::upsert_user(
        &profile.sub,
        &profile.email,
        &profile.name,
        profile.picture.as_deref(),
    )
    .await?;

    let mut session_jar = CookieJar::new();
    let mut sc = Cookie::new(SESSION_COOKIE, user.id.clone());
    sc.set_path("/");
    sc.set_http_only(true);
    sc.set_same_site(SameSite::Lax);
    sc.set_secure(secure_cookies());
    sc.set_max_age(cookie::time::Duration::days(365));
    session_jar.private_mut(&key).add(sc);
    let session_set_cookie = session_jar.delta().next().unwrap().to_string();

    let mut clear_jar = CookieJar::new();
    let mut fc = Cookie::new(FLOW_COOKIE, "");
    fc.set_path("/");
    clear_jar.add_original(fc.clone());
    clear_jar.remove(fc);
    let clear_flow_set_cookie = clear_jar.delta().next().unwrap().to_string();

    Ok(LoginResult {
        user,
        session_set_cookie,
        clear_flow_set_cookie,
        return_to: flow.return_to,
    })
}

/// Read the current session cookie and look up the user, if any. Returns
/// `Ok(None)` for "not logged in" and only errs on a genuine problem (D1
/// unreachable) — an absent or invalid cookie is not an error.
pub async fn current_user(cookie_header: Option<&str>) -> anyhow::Result<Option<User>> {
    let Some(key) = session_key() else {
        return Ok(None);
    };
    let jar = parse_jar(cookie_header);
    let Some(id_cookie) = jar.private(&key).get(SESSION_COOKIE) else {
        return Ok(None);
    };
    crate::db::get_user(id_cookie.value()).await
}

/// `Set-Cookie` value that clears the session, for `/auth/logout`.
pub fn logout_set_cookie() -> String {
    let mut jar = CookieJar::new();
    let c = Cookie::new(SESSION_COOKIE, "");
    jar.add_original(c.clone());
    jar.remove(c);
    jar.delta().next().unwrap().to_string()
}

fn callback_url() -> String {
    let origin =
        std::env::var("SITE_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3100".to_string());
    format!("{}/auth/google/callback", origin.trim_end_matches('/'))
}

fn random_url_safe(bytes: usize) -> String {
    use base64::Engine;
    let mut buf = vec![0u8; bytes];
    rand::fill(buf.as_mut_slice());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// PKCE S256: base64url(sha256(verifier)), no padding.
fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct GoogleProfile {
    sub: String,
    email: String,
    name: String,
    picture: Option<String>,
}
