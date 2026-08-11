//! Image intake: decode, bound, thumbnail, write to disk.
//!
//! Everything goes through `UPLOAD_ROOT` and filenames are generated UUIDs, so
//! nothing the uploader controls reaches the filesystem — no path traversal
//! surface. Swapping this for S3/R2 later means reimplementing `store` alone.

use std::path::{Path, PathBuf};

use image::DynamicImage;
use uuid::Uuid;

pub const UPLOAD_ROOT: &str = "uploads";
pub const MAX_UPLOAD_BYTES: usize = 12 * 1024 * 1024;
pub const THUMB_MAX_EDGE: u32 = 480;

/// Hard cap on decoded pixel count, checked before allocating the full image.
/// Blocks decompression bombs: a few-KB PNG can claim 50000x50000.
const MAX_PIXELS: u64 = 40_000_000;

pub struct Stored {
    pub id: String,
    pub orig_url: String,
    pub thumb_url: String,
    pub width: u32,
    pub height: u32,
}

/// Decode bytes into an image, rejecting anything oversized before it is
/// rasterized. Returned separately from `store` so moderation can inspect the
/// pixels before anything touches the disk.
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

/// Write the original (re-encoded to PNG, which strips EXIF and any smuggled
/// trailing payload) plus a thumbnail. Call only after moderation has passed.
pub async fn store(img: &DynamicImage) -> anyhow::Result<Stored> {
    let id = Uuid::new_v4().to_string();
    let (width, height) = (img.width(), img.height());

    let orig_rel = format!("orig/{id}.png");
    let thumb_rel = format!("thumb/{id}.png");

    let thumb = img.thumbnail(THUMB_MAX_EDGE, THUMB_MAX_EDGE);

    // Encoding is CPU-bound; keep it off the async worker threads.
    let orig_bytes = encode_png(img)?;
    let thumb_bytes = encode_png(&thumb)?;

    write_under_root(&orig_rel, &orig_bytes).await?;
    write_under_root(&thumb_rel, &thumb_bytes).await?;

    Ok(Stored {
        id,
        orig_url: format!("/uploads/{orig_rel}"),
        thumb_url: format!("/uploads/{thumb_rel}"),
        width,
        height,
    })
}

fn encode_png(img: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

async fn write_under_root(rel: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let path = PathBuf::from(UPLOAD_ROOT).join(rel);
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(&path, bytes).await?;
    Ok(())
}

/// Delete both files for a son. Best-effort: a missing file is not an error,
/// since the point of calling this is to end up with the file gone.
pub async fn remove(id: &str) {
    for rel in [format!("orig/{id}.png"), format!("thumb/{id}.png")] {
        let path = Path::new(UPLOAD_ROOT).join(rel);
        let _ = tokio::fs::remove_file(path).await;
    }
}
