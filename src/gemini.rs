//! Talks to the Gemini sidecar (see `sidecar/gemini_service.py`).
//!
//! Two calls, `/judge` then `/square`, deliberately separate: judging takes a few
//! seconds and squaring takes most of a minute, and the upload page reports which
//! one it is waiting on. Behind a single endpoint the whole wait had to be
//! labelled "scanning", which is a progress list that lies.
//!
//! The sidecar is a separate process because the Gemini web client is Python;
//! this module is the whole of the Rust side.
//!
//! Dormant unless `GEMINI_URL` is set, in the same way Google sign-in is dormant
//! without its client id: the app runs and uploads work, they just are not
//! screened or squared.

use image::DynamicImage;

/// Generous, because `/square` waits on an image generation. Measured at ~50s
/// locally and ~85s through production, so this is roughly 2x the slow case: an
/// upload waiting is better than an upload lost.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// What Gemini said about an image. Not a decision -- `Verdict::acceptable`
/// applies the policy, so the rule lives in one place rather than in the
/// sidecar's HTTP status codes.
#[derive(Debug, serde::Deserialize)]
pub struct Verdict {
    /// `PASS` or `FAIL`, or empty if the model did not answer in the shape asked.
    pub verdict: String,
    /// `SON` or `NOTSON`, likewise.
    pub topic: String,
}

impl Verdict {
    /// `Ok(())` to continue, `Err(reason)` to refuse with a line fit to show a
    /// visitor.
    ///
    /// Fails closed: anything that is not an explicit PASS is a refusal,
    /// including an empty answer, a refusal to look at the image, or the model
    /// deciding to be chatty. The reasons are written here rather than echoed
    /// from the model, so nothing it generates is ever rendered on the page.
    pub fn acceptable(&self) -> Result<(), String> {
        if !self.verdict.starts_with("PASS") {
            return Err("This image was not accepted.".into());
        }
        if self.topic.starts_with("NOTSON") {
            return Err("That doesn't look like a son.".into());
        }
        Ok(())
    }
}

/// Why a call could not produce an answer. Distinct from a refusal on purpose:
/// nothing was decided about the image, so the caller keeps the original rather
/// than throwing away a good upload over an outage.
pub struct Unavailable(pub String);

/// `None` when screening is switched off, so callers can tell "not configured"
/// apart from "configured and it failed".
pub fn url() -> Option<String> {
    std::env::var("GEMINI_URL").ok().filter(|u| !u.is_empty())
}

fn form(bytes: Vec<u8>) -> reqwest::multipart::Form {
    reqwest::multipart::Form::new().part(
        "image",
        reqwest::multipart::Part::bytes(bytes)
            .file_name("upload.png")
            .mime_str("image/png")
            .expect("image/png is a valid mime type"),
    )
}

async fn post(path: &str, bytes: Vec<u8>) -> Result<(reqwest::StatusCode, Vec<u8>), Unavailable> {
    let base = url().ok_or_else(|| Unavailable("GEMINI_URL not set".into()))?;
    let resp = reqwest::Client::new()
        .post(format!("{}{path}", base.trim_end_matches('/')))
        .timeout(TIMEOUT)
        .multipart(form(bytes))
        .send()
        .await
        .map_err(|e| Unavailable(format!("sidecar unreachable: {e}")))?;
    let status = resp.status();
    let body = resp
        .bytes()
        .await
        .map_err(|e| Unavailable(format!("sidecar body unreadable: {e}")))?;
    Ok((status, body.to_vec()))
}

fn reason(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_owned))
        .unwrap_or_else(|| "screening failed".to_string())
}

/// Ask whether the image is safe and on-topic. Seconds, not a minute.
///
/// Retried once, because Gemini's transient failures are common enough to have
/// hit the first production upload: "Unknown API error code: 1100. This might be
/// a temporary Google service issue." A judge call is cheap, and the alternative
/// to retrying is an upload that skips screening over a blip.
pub async fn judge(bytes: Vec<u8>) -> Result<Verdict, Unavailable> {
    match judge_once(bytes.clone()).await {
        Ok(v) => Ok(v),
        Err(Unavailable(first)) => {
            tracing::warn!(%first, "gemini judge failed; retrying once");
            judge_once(bytes).await.map_err(|Unavailable(second)| {
                Unavailable(format!("{second} (first attempt: {first})"))
            })
        }
    }
}

async fn judge_once(bytes: Vec<u8>) -> Result<Verdict, Unavailable> {
    let (status, body) = post("/judge", bytes).await?;
    if !status.is_success() {
        return Err(Unavailable(reason(&body)));
    }
    serde_json::from_slice(&body).map_err(|e| Unavailable(format!("unreadable verdict: {e}")))
}

/// Ask for the square version. This is the slow half.
pub async fn square(bytes: Vec<u8>) -> Result<DynamicImage, Unavailable> {
    let (status, body) = post("/square", bytes).await?;
    if !status.is_success() {
        return Err(Unavailable(reason(&body)));
    }
    // Decoded by content rather than by any declared type: Gemini returns JPEG
    // today and that can change without this needing to know.
    image::load_from_memory(&body)
        .map_err(|e| Unavailable(format!("sidecar returned undecodable image: {e}")))
}

/// The shared secret the sidecar requires on `/cookies`. Absent means the swap
/// endpoint refuses everything, which is the safe direction for a
/// misconfiguration.
fn sidecar_key() -> String {
    std::env::var("SIDECAR_KEY").unwrap_or_default()
}

/// Ask the sidecar how it is doing, for the admin page.
///
/// Distinguishes "not configured" from "configured and broken", because those
/// call for completely different actions and a single "screening is off" would
/// hide which one is true.
pub async fn status() -> crate::models::ScreeningStatus {
    let mut out = crate::models::ScreeningStatus::default();
    let Some(base) = url() else {
        return out;
    };
    out.configured = true;

    #[derive(serde::Deserialize)]
    struct Health {
        accounts: u32,
        initialised: u32,
    }

    // Short timeout: this runs on an admin page load, and a hung sidecar should
    // report as broken rather than hang the page.
    match reqwest::Client::new()
        .get(format!("{}/health", base.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        // 503 is the sidecar's way of saying "up, but no account works", so the
        // body is still the answer -- an error branch here would throw it away.
        Ok(resp) => match resp.json::<Health>().await {
            Ok(h) => {
                out.usable = h.accounts;
                out.initialised = h.initialised;
            }
            Err(e) => out.error = Some(format!("unreadable health: {e}")),
        },
        Err(e) => out.error = Some(format!("sidecar unreachable: {e}")),
    }
    out
}

/// Replace the sidecar's cookies at runtime. Returns how many accounts came up.
pub async fn set_cookies(cookies: &str) -> Result<u32, String> {
    let base = url().ok_or_else(|| "GEMINI_URL not set".to_string())?;

    #[derive(serde::Deserialize)]
    struct Accepted {
        accounts: u32,
    }
    #[derive(serde::Deserialize)]
    struct Refused {
        reason: String,
    }

    let resp = reqwest::Client::new()
        .post(format!("{}/cookies", base.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(120))
        .header("X-Sidecar-Key", sidecar_key())
        .json(&serde_json::json!({ "cookies": cookies }))
        .send()
        .await
        .map_err(|e| format!("sidecar unreachable: {e}"))?;

    let status = resp.status();
    let body = resp.bytes().await.map_err(|e| e.to_string())?;
    if status.is_success() {
        return serde_json::from_slice::<Accepted>(&body)
            .map(|a| a.accounts)
            .map_err(|e| format!("unreadable reply: {e}"));
    }
    Err(serde_json::from_slice::<Refused>(&body)
        .map(|r| r.reason)
        .unwrap_or_else(|_| format!("sidecar refused ({status})")))
}
