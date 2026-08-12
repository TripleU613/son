//! An admin-only window onto the keeper's browser, served from this site.
//!
//! The point is that signing in should need nothing but the deployed site: open a
//! page, see a real Chromium, sign in to Google, done. No Tailscale, no DevTools,
//! no files to copy, and no password handed to anyone. The keeper's own profile is
//! the one being signed into, so the session it saves is the session it will keep
//! refreshing.
//!
//! What sits behind this is `websockify` in the keeper container, serving noVNC
//! over HTTP on port 6080 and relaying a VNC stream over a WebSocket. Neither is
//! exposed publicly; both are reached only through the routes here, which check
//! `is_admin` on every single request.
//!
//! Two routes because noVNC needs both halves:
//!
//! * `/admin/browser/*` — the noVNC page and its assets, proxied over HTTP. Its
//!   own asset paths are relative, so serving it under this prefix makes them
//!   resolve back through the proxy without rewriting anything.
//! * `/admin/browser/websockify` — the VNC stream, a WebSocket relayed frame for
//!   frame. A plain HTTP proxy cannot carry this, which is why it is separate.
//!
//! The keeper always runs that stack, with no mode to switch on: a flag would put
//! a redeploy between "the session died" and "I can fix it", which is the friction
//! this exists to remove.

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};

/// Where websockify is, inside the compose network. Not configurable per request
/// and never taken from user input: this proxy forwards to exactly one host, so
/// there is no way to point it somewhere else.
fn keeper_base() -> String {
    std::env::var("KEEPER_URL").unwrap_or_else(|_| "http://keeper:6080".to_string())
}

/// Admin or nothing.
///
/// Checked on the noVNC page, on every asset, and on the WebSocket upgrade --
/// separately, because they are separate requests and a browser will happily open
/// the socket without having loaded the page. The session cookie is the same one
/// the rest of `/admin` uses.
async fn is_admin(headers: &HeaderMap) -> bool {
    let cookie = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok());
    matches!(
        crate::auth::current_user(cookie).await,
        Ok(Some(u)) if u.is_admin
    )
}

/// A short page that frames noVNC, plus the one instruction that matters.
///
/// Server-rendered here rather than as a Leptos route: it is a frame around a
/// third-party page and a couple of sentences, with no reactivity to justify
/// shipping it through the app's router.
pub async fn page(headers: HeaderMap) -> Response {
    if !is_admin(&headers).await {
        return (StatusCode::FORBIDDEN, "admin only").into_response();
    }

    let reachable = reqwest::Client::new()
        .get(format!("{}/vnc.html", keeper_base()))
        .timeout(std::time::Duration::from_secs(4))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    if !reachable {
        return Html(
            r#"<!doctype html><meta charset=utf-8>
<title>sign-in browser — son collection</title>
<style>body{background:#08090b;color:#f4f4f5;font:15px/1.6 system-ui;margin:0;
display:grid;place-items:center;min-height:100dvh;text-align:center;padding:2rem}
code{background:#171a20;padding:.15em .4em;border-radius:4px}a{color:#ffcc33}</style>
<div><h1 style="font-size:1.3rem">The sign-in browser is not answering</h1>
<p>The keeper container should always have one running. It may still be starting
&mdash; give it a minute and reload.</p>
<p>If it persists, check <code>docker logs son-keeper-1</code>.</p>
<p><a href="/admin">back to admin</a></p></div>"#,
        )
        .into_response();
    }

    // autoconnect + reconnect so the page is usable without touching noVNC's own
    // connection dialog, and resize=remote so the desktop matches the window
    // rather than being scrolled around.
    Html(
        r#"<!doctype html><meta charset=utf-8>
<title>sign-in browser — son collection</title>
<style>html,body{margin:0;height:100%;background:#08090b;color:#f4f4f5;
font:14px/1.5 system-ui}header{display:flex;gap:1rem;align-items:center;
padding:.6rem .9rem;border-bottom:1px solid #292d35}a{color:#ffcc33}
iframe{border:0;width:100%;height:calc(100% - 44px);display:block}</style>
<header><strong>Sign in to Google in this window.</strong>
<span style="color:#a6a8b0">The session is saved automatically and screening starts
working within a minute. Nothing to switch off afterwards.</span>
<a href="/admin" style="margin-left:auto">admin</a></header>
<iframe src="/admin/browser/vnc.html?path=admin/browser/websockify&autoconnect=1&reconnect=1&resize=remote"
        allow="clipboard-read; clipboard-write"></iframe>"#,
    )
    .into_response()
}

/// Proxy noVNC's static assets.
pub async fn asset(headers: HeaderMap, Path(path): Path<String>) -> Response {
    if !is_admin(&headers).await {
        return (StatusCode::FORBIDDEN, "admin only").into_response();
    }

    // The path comes from a wildcard route, so it cannot escape the prefix, and
    // the target host is fixed. Query strings are dropped deliberately: noVNC's
    // assets take none, and forwarding them would be surface for nothing.
    let url = format!("{}/{}", keeper_base(), path.trim_start_matches('/'));
    match reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            // Content-Type carried through, because noVNC serves JavaScript
            // modules and a browser refuses to execute those as text/plain.
            let ct = resp
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            match resp.bytes().await {
                Ok(body) => (
                    status,
                    [(axum::http::header::CONTENT_TYPE, ct)],
                    Body::from(body),
                )
                    .into_response(),
                Err(e) => {
                    (StatusCode::BAD_GATEWAY, format!("keeper read failed: {e}")).into_response()
                }
            }
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("keeper unreachable: {e}")).into_response(),
    }
}

/// Relay the VNC WebSocket.
pub async fn websocket(headers: HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    if !is_admin(&headers).await {
        return (StatusCode::FORBIDDEN, "admin only").into_response();
    }
    // The subprotocol matters: websockify only speaks binary once "binary" is
    // negotiated, and noVNC will not talk to it otherwise.
    upgrade
        .protocols(["binary"])
        .on_upgrade(|socket| async move {
            if let Err(e) = relay(socket).await {
                tracing::warn!("vnc relay ended: {e}");
            }
        })
}

async fn relay(client: WebSocket) -> anyhow::Result<()> {
    let target = keeper_base()
        .replace("http://", "ws://")
        .replace("https://", "wss://");
    let url = format!("{target}/websockify");

    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&url)
        .header("Host", url.split("//").nth(1).unwrap_or("keeper:6080"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Protocol", "binary")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())?;

    let (upstream, _) = tokio_tungstenite::connect_async(request).await?;
    let (mut up_tx, mut up_rx) = upstream.split();
    let (mut cl_tx, mut cl_rx) = client.split();

    // Both directions concurrently, and the first to finish ends the other: a
    // closed VNC connection should not leave a half-open socket behind.
    let to_upstream = async {
        while let Some(Ok(msg)) = cl_rx.next().await {
            let out = match msg {
                Message::Binary(b) => {
                    tokio_tungstenite::tungstenite::Message::Binary(b.to_vec().into())
                }
                Message::Text(t) => {
                    tokio_tungstenite::tungstenite::Message::Text(t.as_str().into())
                }
                Message::Close(_) => break,
                // Ping/Pong are handled by each side's own keepalive; forwarding
                // them would double up.
                _ => continue,
            };
            if up_tx.send(out).await.is_err() {
                break;
            }
        }
    };

    let to_client = async {
        while let Some(Ok(msg)) = up_rx.next().await {
            let out = match msg {
                tokio_tungstenite::tungstenite::Message::Binary(b) => {
                    Message::Binary(b.to_vec().into())
                }
                tokio_tungstenite::tungstenite::Message::Text(t) => {
                    Message::Text(t.as_str().into())
                }
                tokio_tungstenite::tungstenite::Message::Close(_) => break,
                _ => continue,
            };
            if cl_tx.send(out).await.is_err() {
                break;
            }
        }
    };

    tokio::select! {
        _ = to_upstream => {}
        _ = to_client => {}
    }
    Ok(())
}
