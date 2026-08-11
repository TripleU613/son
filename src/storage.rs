//! Image intake: decode, bound, thumbnail, persist.
//!
//! Persistence sits behind the `Backend` trait so the same intake path works
//! against local disk in development and Cloudflare R2 in production. Object
//! keys are generated UUIDs, never anything the uploader supplies, so there is
//! no path-traversal or key-injection surface either way.

use std::path::PathBuf;
use std::sync::Arc;

use image::DynamicImage;
use uuid::Uuid;

pub mod local;
pub mod r2;

pub const UPLOAD_ROOT: &str = "uploads";
pub const MAX_UPLOAD_BYTES: usize = 12 * 1024 * 1024;
pub const THUMB_MAX_EDGE: u32 = 480;

/// Hard cap on decoded pixel count, checked before allocating the full image.
/// Blocks decompression bombs: a few-KB PNG can claim 50000x50000.
const MAX_PIXELS: u64 = 40_000_000;

/// Where the bytes actually live.
#[async_trait::async_trait]
pub trait Backend: Send + Sync + 'static {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> anyhow::Result<()>;

    /// Fetch an object's bytes back. Used only for the same-origin download
    /// route (`Content-Disposition: attachment` has to come from a response
    /// this server controls -- browsers ignore an `<a download>` attribute
    /// pointed at a cross-origin URL, and R2's public domain is a different
    /// origin from the app itself). Normal viewing never calls this; the
    /// gallery links straight to the CDN.
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>>;

    /// Best-effort: a missing object is not an error, since the goal of calling
    /// this is for the object to be gone.
    async fn delete(&self, key: &str);

    /// Publicly reachable URL for a stored key.
    fn public_url(&self, key: &str) -> String;

    fn name(&self) -> String;
}

static BACKEND: std::sync::OnceLock<Arc<dyn Backend>> = std::sync::OnceLock::new();

pub fn set_backend(b: Arc<dyn Backend>) {
    let _ = BACKEND.set(b);
}

pub fn backend() -> &'static Arc<dyn Backend> {
    BACKEND.get().expect("storage backend not initialized")
}

/// Pick a backend from the environment: R2 when fully configured, local disk
/// otherwise. Falling back rather than failing keeps `cargo leptos watch`
/// working with no cloud credentials present.
pub async fn backend_from_env() -> Arc<dyn Backend> {
    match r2::R2::from_env().await {
        Ok(Some(r2)) => match r2.check().await {
            Ok(()) => Arc::new(r2),
            Err(e) => {
                tracing::error!("R2 credentials rejected ({e}); falling back to local disk");
                Arc::new(local::LocalDisk::new(UPLOAD_ROOT, "/uploads"))
            }
        },
        Ok(None) => Arc::new(local::LocalDisk::new(UPLOAD_ROOT, "/uploads")),
        Err(e) => {
            tracing::error!("R2 is configured but unusable ({e}); falling back to local disk");
            Arc::new(local::LocalDisk::new(UPLOAD_ROOT, "/uploads"))
        }
    }
}

pub struct Stored {
    pub id: String,
    pub orig_url: String,
    pub thumb_url: String,
    pub width: u32,
    pub height: u32,
}

/// Decode bytes into an image, rejecting anything oversized before it is
/// rasterized. Separate from `store` so moderation can inspect the pixels
/// before anything is persisted.
pub fn decode(bytes: &[u8]) -> anyhow::Result<DynamicImage> {
    if bytes.len() > MAX_UPLOAD_BYTES {
        anyhow::bail!(
            "too big: {:.1}MB, limit is {}MB",
            bytes.len() as f64 / 1_048_576.0,
            MAX_UPLOAD_BYTES / 1_048_576
        );
    }

    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| anyhow::anyhow!("unreadable image: {e}"))?;

    // Check the header's claimed dimensions before decoding the body.
    if let Ok((w, h)) = reader.into_dimensions() {
        let pixels = w as u64 * h as u64;
        if pixels > MAX_PIXELS {
            anyhow::bail!("{w}x{h} is too many pixels");
        }
    }

    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| anyhow::anyhow!("unreadable image: {e}"))?;

    reader
        .decode()
        .map_err(|e| anyhow::anyhow!("could not decode: {e}"))
}

pub fn orig_key(id: &str) -> String {
    format!("orig/{id}.png")
}

pub fn thumb_key(id: &str) -> String {
    format!("thumb/{id}.png")
}

/// Persist the original (re-encoded to PNG, which strips EXIF and any payload
/// smuggled after the image data) plus a thumbnail. Call only after moderation
/// has passed.
pub async fn store(img: &DynamicImage) -> anyhow::Result<Stored> {
    let id = Uuid::new_v4().to_string();
    let (width, height) = (img.width(), img.height());

    let thumb = img.thumbnail(THUMB_MAX_EDGE, THUMB_MAX_EDGE);
    let orig_bytes = encode_png(img)?;
    let thumb_bytes = encode_png(&thumb)?;

    let be = backend();
    let (ok, tk) = (orig_key(&id), thumb_key(&id));

    be.put(&ok, orig_bytes, "image/png").await?;

    // If the thumbnail fails, drop the original too rather than leaving a son
    // that the gallery cannot render.
    if let Err(e) = be.put(&tk, thumb_bytes, "image/png").await {
        be.delete(&ok).await;
        return Err(e);
    }

    Ok(Stored {
        orig_url: be.public_url(&ok),
        thumb_url: be.public_url(&tk),
        id,
        width,
        height,
    })
}

fn encode_png(img: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

/// Delete both objects for a son.
pub async fn remove(id: &str) {
    let be = backend();
    be.delete(&orig_key(id)).await;
    be.delete(&thumb_key(id)).await;
}

/// Local path for a key, used only by the disk backend.
fn local_path(root: &str, key: &str) -> PathBuf {
    PathBuf::from(root).join(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_derived_from_id_only() {
        assert_eq!(orig_key("abc"), "orig/abc.png");
        assert_eq!(thumb_key("abc"), "thumb/abc.png");
    }

    #[test]
    fn oversized_input_is_rejected_before_decode() {
        let huge = vec![0u8; MAX_UPLOAD_BYTES + 1];
        let err = decode(&huge).unwrap_err().to_string();
        assert!(err.contains("too big"), "got: {err}");
    }

    #[test]
    fn garbage_is_not_an_image() {
        assert!(decode(b"definitely not a png").is_err());
    }
}
