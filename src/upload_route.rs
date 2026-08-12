//! `POST /api/upload` — multipart image intake.
//!
//! Order: decode+hash → duplicate check → store (watermark + provenance
//! metadata) → insert. Nothing is written until the duplicate check clears, so a
//! rejected upload leaves no files to clean up.
//!
//! Screening and squaring happen in Gemini, through the sidecar (see
//! `sidecar/gemini_service.py` and `crate::gemini`): it decides whether an image
//! is safe and whether it is actually a son, and returns a square version. No
//! model runs in this process.
//!
//! With `GEMINI_URL` unset the whole step is skipped and uploads publish
//! unscreened, which is also what happens when Gemini is unreachable -- an
//! outage must not stop people contributing. `storage::to_square` runs either
//! way, so every stored image is the same size regardless.

use crate::models::UploadResult;
use axum::extract::Multipart;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

/// `POST /api/upload` -- parses the multipart, then answers 202 with a job id and
/// does the slow part in the background.
///
/// It used to hold the connection for the whole pipeline. With Gemini in the
/// middle that is ~50 seconds of a form looking frozen, and any proxy with a
/// 30-second read timeout in between would kill an upload that was going to
/// succeed. The browser polls `/api/upload/status/:id` instead.
///
/// `HeaderMap` before `Multipart`: axum requires body-consuming extractors
/// (Multipart reads the request body) to come last in a handler's arguments.
pub async fn upload(headers: axum::http::HeaderMap, mut mp: Multipart) -> impl IntoResponse {
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok());
    // A cookie lookup failure (D1 unreachable) should not block the upload
    // over something unrelated to it; fall back to anonymous rather than
    // erroring the whole request.
    let uploader = crate::auth::current_user(cookie_header)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("could not resolve uploader from session: {e}");
            None
        });

    let mut bytes: Option<Vec<u8>> = None;
    let mut title = String::new();

    loop {
        let field = match mp.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return bad(StatusCode::BAD_REQUEST, format!("malformed upload: {e}")),
        };

        match field.name() {
            Some("son") => match field.bytes().await {
                Ok(b) => bytes = Some(b.to_vec()),
                Err(e) => return bad(StatusCode::BAD_REQUEST, format!("could not read file: {e}")),
            },
            Some("title") => {
                title = field.text().await.unwrap_or_default();
            }
            _ => {}
        }
    }

    let Some(bytes) = bytes else {
        return bad(StatusCode::BAD_REQUEST, "no file in the 'son' field".into());
    };

    // Everything past here is slow, so it moves off the request. The multipart
    // had to be drained first: it borrows the request body, which does not
    // outlive this handler.
    let job = crate::jobs::start();
    let job_id = job.clone();
    tokio::spawn(async move { run(job_id, bytes, title, uploader).await });

    (StatusCode::ACCEPTED, Json(UploadResult::Queued { job }))
}

/// The pipeline. Reports each step into the job registry as it starts it, so the
/// browser's poll is describing real work rather than a timed animation.
async fn run(job: String, bytes: Vec<u8>, title: String, uploader: Option<crate::models::User>) {
    use crate::jobs::set;
    use crate::models::{Progress, Step};

    macro_rules! step {
        ($s:expr) => {
            set(&job, Progress::Running { step: $s })
        };
    }
    macro_rules! fail {
        ($msg:expr) => {{
            set(&job, Progress::Failed { message: $msg });
            return;
        }};
    }
    macro_rules! reject {
        ($reason:expr) => {{
            set(&job, Progress::Rejected { reason: $reason });
            return;
        }};
    }

    step!(Step::Fingerprinting);

    // Decoding is CPU-bound and attacker-influenced; keep it off the async
    // runtime's worker threads so one huge image can't stall request handling.
    // The content hash rides along in the same blocking call: it's a hash of
    // the decoded pixel buffer (not the raw upload bytes), computed here
    // because this is the one place that buffer exists before storage
    // consumes it.
    let decoded = tokio::task::spawn_blocking(move || {
        let img = crate::storage::decode(&bytes)?;
        let hash = crate::dedupe::sha256_hex(img.to_rgba8().as_raw());
        Ok::<_, anyhow::Error>((img, hash))
    })
    .await;

    let (img, content_hash) = match decoded {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => fail!(e.to_string()),
        Err(e) => fail!(format!("decode panicked: {e}")),
    };

    // Exact duplicate: same decoded pixels as something already here,
    // regardless of container format. A single indexed lookup.
    match crate::db::find_by_hash(&content_hash).await {
        Ok(Some(existing)) => {
            tracing::info!(existing = existing.id, "upload rejected: exact duplicate");
            reject!(format!(
                "this exact image is already in the collection: /son/{}",
                existing.slug
            ));
        }
        Ok(None) => {}
        // A dedupe-check outage shouldn't block an otherwise-good upload;
        // log loudly and let it through rather than fail the whole request.
        Err(e) => tracing::error!("hash dedupe check failed: {e}"),
    }

    // Screening and squaring, in Gemini. After the duplicate check so a re-upload
    // costs nothing, and before storage so a refused image is never written.
    //
    // Two calls, reported as two steps: judging takes seconds, squaring takes
    // most of a minute, and one label over both would be a progress list that
    // lies about what it is waiting for.
    //
    // When screening cannot run at all, the upload is *held* rather than either
    // dropped or published. Publishing it would mean a Gemini blip silently
    // producing unscreened public content -- which happened on the first
    // production upload, via a transient "API error code: 1100" -- and refusing
    // it would mean an outage stopping contributions. Held keeps the upload,
    // keeps it out of the gallery, and puts it in the admin queue.
    let mut held_reason: Option<String> = None;

    let img = if crate::gemini::url().is_some() {
        let bytes = match encode_png(&img) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("could not re-encode for screening: {e}");
                fail!("could not process this son".into());
            }
        };

        step!(Step::Scanning);
        match crate::gemini::judge(bytes.clone()).await {
            Ok(verdict) => {
                if let Err(reason) = verdict.acceptable() {
                    tracing::info!(?verdict, "upload refused by gemini");
                    reject!(reason);
                }
                step!(Step::Regenerating);
                match crate::gemini::square(bytes).await {
                    // Checked, not trusted. The prompt says "edit, do not redraw",
                    // but painting over a removed caption and extending the
                    // background to reach a square are both synthesis, and the
                    // model re-renders the whole canvas either way -- it once
                    // returned a different person's face in place of the uploaded
                    // one. A prompt cannot enforce this; a comparison can.
                    Ok(square) if crate::dedupe::is_plausible_edit(&img, &square) => {
                        tracing::info!("gemini edited and squared this upload");
                        square
                    }
                    Ok(_) => {
                        // Publish the original rather than the redraw, and rather
                        // than refusing: the upload was screened and is fine, it
                        // just did not come back recognisable. to_square below
                        // still gives it the right shape.
                        tracing::warn!(
                            "gemini returned something too unlike the upload; keeping the original"
                        );
                        img
                    }
                    // Judged safe, only the redraw failed. Publish the original:
                    // it has been screened, and `to_square` below still makes it
                    // the right shape.
                    Err(crate::gemini::Unavailable(why)) => {
                        tracing::warn!(%why, "gemini could not square it; publishing the original");
                        img
                    }
                }
            }
            Err(crate::gemini::Unavailable(why)) => {
                tracing::error!(%why, "gemini could not screen this upload; holding it for review");
                held_reason = Some(why);
                img
            }
        }
    } else {
        img
    };

    // Unconditional, so the gallery's tiles are uniform whether Gemini ran or
    // not. A no-op when Gemini already returned 1024x1024.
    step!(Step::Cropping);
    let img = crate::storage::to_square(&img);

    let title = clean_title(&title);

    step!(Step::Storing);
    let stored = match crate::storage::store(
        &img,
        &title,
        uploader.as_ref().map(|u| u.display_name.as_str()),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("store failed: {e}");
            fail!("could not save this son".into());
        }
    };

    // After the title is cleaned, so the slug matches what will be displayed.
    let slug = crate::db::unique_slug(&title, &stored.id).await;

    let son = crate::db::insert(crate::db::NewSon {
        id: &stored.id,
        slug: &slug,
        is_public: held_reason.is_none(),
        title: &title,
        orig_url: &stored.orig_url,
        thumb_url: &stored.thumb_url,
        width: stored.width,
        height: stored.height,
        content_hash: &content_hash,
        uploader_id: uploader.as_ref().map(|u| u.id.as_str()),
        uploader: uploader.as_ref().map(|u| crate::models::Uploader {
            display_name: u.display_name.clone(),
            avatar_url: u.avatar_url.clone(),
        }),
    })
    .await;

    let son = match son {
        Ok(son) => son,
        Err(e) => {
            // The row failed but the files landed. Remove them so the disk does
            // not fill with images nothing references.
            crate::storage::remove(&stored.id).await;
            tracing::error!("insert failed: {e}");
            fail!("could not record this son".into());
        }
    };

    if let Some(why) = held_reason {
        tracing::warn!(son = %son.id, %why, "son held, awaiting admin review");
    }
    set(&job, Progress::Done { son: Box::new(son) });
}

/// `GET /api/upload/status/:id`.
///
/// An unknown id answers `Failed` rather than 404: from the browser's side an
/// expired job and a job that never existed are the same situation -- nothing
/// further is coming -- and giving it one shape to handle keeps the polling loop
/// from needing a special case.
pub async fn status(axum::extract::Path(id): axum::extract::Path<String>) -> impl IntoResponse {
    match crate::jobs::get(&id) {
        Some(p) => Json(p),
        None => Json(crate::jobs::Progress::Failed {
            message: "this upload is no longer being tracked".into(),
        }),
    }
}

/// Re-encode the decoded image as PNG for the trip to the sidecar.
///
/// Deliberately not `storage::encode_png`, which also writes the provenance
/// iTXt chunks and applies the watermark: none of that should exist on a copy
/// that only travels to Gemini and back, and the version that gets stored is
/// the one Gemini returns, not this one.
fn encode_png(img: &image::DynamicImage) -> anyhow::Result<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

fn bad(code: StatusCode, message: String) -> (StatusCode, Json<UploadResult>) {
    (code, Json(UploadResult::Error { message }))
}

/// Titles are rendered as text by Leptos (which escapes), so this is about
/// keeping the gallery legible, not about injection.
fn clean_title(raw: &str) -> String {
    let t: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    if t.is_empty() {
        "untitled son".to_string()
    } else {
        t
    }
}
