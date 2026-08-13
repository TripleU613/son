//! Image intake: decode, bound, thumbnail, watermark, persist.
//!
//! Persistence sits behind the `Backend` trait so the same intake path works
//! against local disk in development and Cloudflare R2 in production. Object
//! keys are generated UUIDs, never anything the uploader supplies, so there is
//! no path-traversal or key-injection surface either way.
//!
//! Every original is re-encoded to PNG from raw decoded pixels (strips EXIF
//! and anything smuggled after the image data), carries an invisible
//! provenance watermark (see `watermark`), and gets this site's own text
//! metadata written back in rather than whatever the source file had.
//! Duplicate detection (`dedupe`) happens in `upload_route` before any of this
//! runs, so this module only ever sees an image that is not already here.

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

/// Every stored original is exactly this, square. 1024 because that is what
/// Gemini returns, so an accepted image is stored at its native size with no
/// resample at all.
pub const CANVAS: u32 = 1024;

/// Force an image to a `CANVAS`-sized square by cropping to centre and scaling.
///
/// Applied to every upload, including ones Gemini never saw (screening off, or
/// unavailable) -- "every image the same size" has to hold unconditionally, or
/// the grid goes back to having its rows staggered by whichever card is tallest.
///
/// Crops rather than letterboxes: bars would make the stored file contain
/// padding that every downstream consumer -- cards, OG images, the API -- then
/// has to work around. A centre crop loses edges, which for a meme is the part
/// nobody framed anything important in.
pub fn to_square(img: &DynamicImage) -> DynamicImage {
    let (w, h) = (img.width().max(1), img.height().max(1));

    let square = if w == h {
        img.clone()
    } else {
        let edge = w.min(h);
        // Integer division: with an odd difference this leaves the extra pixel
        // on the right/bottom, which is invisible and beats rounding up and
        // cropping outside the image.
        img.crop_imm((w - edge) / 2, (h - edge) / 2, edge, edge)
    };

    if square.width() == CANVAS {
        return square;
    }
    // Lanczos3: these get scaled both up (a 200px meme) and down (a 4000px
    // photo), and it is the only filter here that does not visibly soften on the
    // way up or alias on the way down.
    square.resize_exact(CANVAS, CANVAS, image::imageops::FilterType::Lanczos3)
}

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

/// The prefix embedded in every original's invisible watermark. Kept short:
/// the payload competes with image content for bits, and this is already
/// enough to trace a copy back to a specific upload via `watermark::extract`.
const WATERMARK_PREFIX: &str = "son-collection:v1:";

/// Decode bytes into an image, rejecting anything oversized before it is
/// rasterized. Separate from `store` so the caller can hash the pixels
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

/// Thumbnails are JPEG, and the extension is part of the key because the
/// original is not.
///
/// A son is a photograph with text on it, and PNG is lossless: measured in
/// production, a 480px thumbnail weighed ~300KB, so one 24-tile gallery page
/// pulled roughly **7MB** of images. The same tile as JPEG is 20-40KB. Nothing
/// about the format was the problem -- the caching was already right
/// (`immutable`, `max-age=31536000`, hitting the edge) -- there was simply ten
/// times more of it than there needed to be.
///
/// Losing alpha is fine *here specifically*: the thumbnail is only ever drawn
/// into a grid tile with `object-cover`, so it is cropped edge to edge and
/// nothing behind it can show through. Transparent pixels are flattened onto the
/// surface colour rather than turning black. The original keeps PNG, because
/// that is the copy people download and the one carrying the provenance chunks
/// and the watermark.
pub fn thumb_key(id: &str) -> String {
    format!("thumb/{id}.jpg")
}

/// Quality for thumbnails. 82 is the usual sweet spot where JPEG artefacts stop
/// being visible on photographic content at this size; the emoji and caption
/// text these images carry are the parts that would show ringing first, and they
/// survive it at 480px.
const THUMB_QUALITY: u8 = 82;

/// `bg` is the tile background these are drawn on (`surface-raised`), so a
/// transparent source flattens to the colour it would have appeared to sit on
/// rather than to black.
fn encode_thumb_jpeg(img: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    use image::{Rgba, RgbaImage};

    const BG: Rgba<u8> = Rgba([0x12, 0x14, 0x19, 0xff]);

    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut flat = RgbaImage::from_pixel(w, h, BG);
    image::imageops::overlay(&mut flat, &rgba, 0, 0);

    let rgb = DynamicImage::ImageRgba8(flat).to_rgb8();
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, THUMB_QUALITY).encode(
        rgb.as_raw(),
        w,
        h,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf)
}

/// Persist the original (re-encoded to PNG, which strips EXIF and any payload
/// smuggled after the image data) plus a thumbnail. Call only after the
/// duplicate check has passed -- this writes files, and a rejected upload
/// should leave none behind.
///
/// `title`/`uploader_name` are written back into the original's PNG text
/// chunks -- provenance metadata the site controls, replacing whatever
/// (or nothing) the source file carried before the EXIF strip.
pub async fn store(
    img: &DynamicImage,
    title: &str,
    uploader_name: Option<&str>,
) -> anyhow::Result<Stored> {
    let id = Uuid::new_v4().to_string();
    let (width, height) = (img.width(), img.height());
    let page_url = crate::seo::absolute(&format!("/son/{id}"));

    let thumb = img.thumbnail(THUMB_MAX_EDGE, THUMB_MAX_EDGE);

    // Only the original carries the watermark: a thumbnail this small
    // (THUMB_MAX_EDGE) has nowhere near enough pixels for the payload to
    // survive being meaningful, and it isn't the copy anyone would trace
    // provenance from anyway.
    let watermarked = crate::watermark::embed(img, format!("{WATERMARK_PREFIX}{id}").as_bytes());

    let meta = PngMeta {
        title,
        uploader_name,
        page_url: &page_url,
    };
    let orig_bytes = encode_png(&watermarked, &meta)?;
    // The thumbnail deliberately does NOT go through `encode_png`, so it carries
    // none of the iTXt provenance chunks -- JPEG has nowhere to put them, and it
    // is not the copy anyone traces provenance from. See `thumb_key`.
    let thumb_bytes = encode_thumb_jpeg(&thumb)?;

    let be = backend();
    let (ok, tk) = (orig_key(&id), thumb_key(&id));

    be.put(&ok, orig_bytes, "image/png").await?;

    // If the thumbnail fails, drop the original too rather than leaving a son
    // that the gallery cannot render.
    if let Err(e) = be.put(&tk, thumb_bytes, "image/jpeg").await {
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

struct PngMeta<'a> {
    title: &'a str,
    uploader_name: Option<&'a str>,
    page_url: &'a str,
}

/// Encodes via the `png` crate directly rather than `DynamicImage::write_to`:
/// `image`'s generic encoder has no way to attach text chunks, and provenance
/// metadata is the whole point of this function. `iTXt`, not `tEXt`: titles
/// are free text and can contain characters `tEXt`'s Latin-1 encoding can't
/// represent.
fn encode_png(img: &DynamicImage, meta: &PngMeta) -> anyhow::Result<Vec<u8>> {
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.add_itxt_chunk("Title".to_string(), meta.title.to_string())?;
        encoder.add_itxt_chunk(
            "Software".to_string(),
            "son collection (soncollection.com)".to_string(),
        )?;
        encoder.add_itxt_chunk("Description".to_string(), meta.page_url.to_string())?;
        if let Some(author) = meta.uploader_name {
            encoder.add_itxt_chunk("Author".to_string(), author.to_string())?;
        }
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba.as_raw())?;
    }
    Ok(buf)
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
        // The two extensions differ on purpose and are not interchangeable: the
        // original stays PNG because it carries the watermark and the iTXt
        // provenance chunks, and the thumbnail is JPEG because it is a
        // photograph shown at 480px and PNG made it ten times bigger than it
        // needed to be. Changing either string orphans every object already in
        // R2 under the old key.
        assert_eq!(thumb_key("abc"), "thumb/abc.jpg");
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

    /// Every stored son is the same square, whatever came in -- the whole point
    /// of the fixed canvas. Landscape, portrait and already-square all land on
    /// exactly CANVAS x CANVAS.
    #[test]
    fn every_shape_becomes_the_same_square() {
        for (w, h) in [
            (640, 360),
            (360, 640),
            (1024, 1024),
            (37, 5),
            (5, 37),
            (1, 1),
        ] {
            let src = DynamicImage::new_rgba8(w, h);
            let out = to_square(&src);
            assert_eq!(
                (out.width(), out.height()),
                (CANVAS, CANVAS),
                "{w}x{h} did not become {CANVAS}x{CANVAS}"
            );
        }
    }

    /// The crop is centred, so a subject in the middle survives. Checked by
    /// colour rather than by geometry: a red stripe down the centre of a wide
    /// image must still be red at the centre of the square.
    #[test]
    fn crop_keeps_the_middle() {
        let mut src = image::RgbaImage::from_pixel(600, 200, image::Rgba([0, 0, 0, 255]));
        for y in 0..200 {
            for x in 280..320 {
                src.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let out = to_square(&DynamicImage::ImageRgba8(src)).to_rgba8();
        let centre = out.get_pixel(CANVAS / 2, CANVAS / 2);
        assert!(
            centre[0] > 200 && centre[1] < 60,
            "centre pixel was not the red stripe: {centre:?}"
        );
    }

    /// A one-pixel-wide input still produces a valid square rather than
    /// dividing by zero or panicking on a zero-sized crop.
    #[test]
    fn extreme_aspect_does_not_panic() {
        let out = to_square(&DynamicImage::new_rgba8(1, 4000));
        assert_eq!((out.width(), out.height()), (CANVAS, CANVAS));
    }
}
