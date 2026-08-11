//! Talks to the Gemini sidecar (see `sidecar/gemini_service.py`).
//!
//! The sidecar judges an upload and hands back a square version of it. It lives
//! in a separate process because the Gemini web client is Python; this module is
//! the whole of the Rust side.
//!
//! Dormant unless `GEMINI_URL` is set, in the same way Google sign-in is dormant
//! without its client id: the app runs and uploads work, they just are not
//! screened or squared.

use image::DynamicImage;

/// What the sidecar said about an upload.
pub enum Outcome {
    /// Judged safe and on-topic. The image is Gemini's square version.
    Accepted(DynamicImage),
    /// Judged unacceptable. The string is fit to show a visitor verbatim: it is
    /// written by the sidecar, never echoed from the model, so a jailbroken or
    /// chatty reply cannot end up rendered on the page.
    Rejected(String),
    /// Gemini or the sidecar failed. Distinct from `Rejected` on purpose --
    /// nothing was decided about the image, so the caller keeps the original
    /// rather than throwing away a good upload over an outage.
    Unavailable(String),
}

/// `None` when screening is switched off, so callers can tell "not configured"
/// apart from "configured and it failed".
pub fn url() -> Option<String> {
    std::env::var("GEMINI_URL").ok().filter(|u| !u.is_empty())
}

/// Round trip one image through the sidecar.
///
/// Two Gemini calls happen on the far side of this, one of which generates an
/// image, so this is slow by nature -- tens of seconds. The timeout is generous
/// for that reason; an upload waiting is better than an upload lost.
pub async fn process(bytes: Vec<u8>) -> Outcome {
    let Some(base) = url() else {
        return Outcome::Unavailable("GEMINI_URL not set".into());
    };

    let form = reqwest::multipart::Form::new().part(
        "image",
        reqwest::multipart::Part::bytes(bytes)
            .file_name("upload.png")
            .mime_str("image/png")
            .expect("image/png is a valid mime type"),
    );

    let resp = reqwest::Client::new()
        .post(format!("{}/process", base.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(180))
        .multipart(form)
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Outcome::Unavailable(format!("sidecar unreachable: {e}")),
    };

    let status = resp.status();
    let body = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Outcome::Unavailable(format!("sidecar body unreadable: {e}")),
    };

    if status.is_success() {
        // Gemini returns JPEG today. Decoding by content rather than by any
        // declared type means that can change without breaking this.
        return match image::load_from_memory(&body) {
            Ok(img) => Outcome::Accepted(img),
            Err(e) => Outcome::Unavailable(format!("sidecar returned undecodable image: {e}")),
        };
    }

    let reason = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_owned))
        .unwrap_or_else(|| "screening failed".to_string());

    // 422 is a decision about the image; anything else is a malfunction.
    if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
        Outcome::Rejected(reason)
    } else {
        Outcome::Unavailable(reason)
    }
}
