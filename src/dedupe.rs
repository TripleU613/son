//! Exact duplicate detection: a SHA-256 of the decoded pixel buffer.
//!
//! Hashing the decoded pixels rather than the uploaded bytes is what makes this
//! useful — the same image re-saved as a different format, or with different
//! compression settings, produces different file bytes but identical pixels, and
//! is still the same son.
//!
//! It also carries the perceptual hash used to check that Gemini *edited* an
//! upload rather than repainting it -- a different question from duplication, but
//! the same tool.
//!
//! This is the only duplicate check that remains. Near-duplicate detection
//! (a resize, recompress, or light crop, which changes the pixels and so changes
//! the hash) used cosine similarity between CLIP embeddings and went out with the
//! rest of the local image analysis. Nothing here looks at *what* an image
//! depicts.

use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Rows and columns of the difference-hash grid. 8x8 comparisons need a 9x8
/// sample, giving a 64-bit hash.
const DHASH_W: u32 = 9;
const DHASH_H: u32 = 8;

/// A 64-bit perceptual hash: is each pixel brighter than the one to its right?
///
/// Structure, not colour, and cheap to compare. Deliberately not a cryptographic
/// hash: the point is that two images that *look* alike hash alike, which is the
/// opposite of what `sha256_hex` above is for.
pub fn dhash(img: &image::DynamicImage) -> u64 {
    let small = img
        .resize_exact(DHASH_W, DHASH_H, image::imageops::FilterType::Triangle)
        .to_luma8();
    let mut hash = 0u64;
    let mut bit = 0;
    for y in 0..DHASH_H {
        for x in 0..(DHASH_W - 1) {
            if small.get_pixel(x, y)[0] > small.get_pixel(x + 1, y)[0] {
                hash |= 1 << bit;
            }
            bit += 1;
        }
    }
    hash
}

/// Differing bits between two hashes: 0 identical, 64 maximally unalike.
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// How far Gemini's edit may differ from the original before it is rejected.
///
/// This exists because a prompt cannot enforce anything. The instruction now says
/// "edit, do not redraw", but two of the three edits it asks for -- painting over a
/// removed caption, extending the background to reach a square -- are synthesis,
/// and the model re-renders the whole canvas to do them. It once returned a
/// different person's face in place of the uploaded one, and nothing downstream
/// looked.
///
/// 16 of 64 bits is deliberately loose. A legitimate edit does change the image:
/// captions vanish, edges get extended, the crop moves. Measured on real cases, a
/// caption removal moves a handful of bits and a genuinely different subject moves
/// 25 or more, so this sits well clear of the first and well below the second.
/// Wrong in the permissive direction on purpose -- rejecting a good edit means the
/// son is published unsquared, which is a worse outcome than a slightly loose
/// threshold.
pub const MAX_EDIT_DISTANCE: u32 = 16;

/// Whether `edited` is plausibly an edit of `original` rather than a new picture.
///
/// Both are compared as squares, because the edit is expected to change the aspect
/// ratio and comparing a wide original against a square edit would flag every
/// single one.
pub fn is_plausible_edit(original: &image::DynamicImage, edited: &image::DynamicImage) -> bool {
    let distance = hamming(
        dhash(&crate::storage::to_square(original)),
        dhash(&crate::storage::to_square(edited)),
    );
    tracing::info!(distance, limit = MAX_EDIT_DISTANCE, "gemini edit distance");
    distance <= MAX_EDIT_DISTANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_bytes_hash_the_same() {
        assert_eq!(sha256_hex(b"a son"), sha256_hex(b"a son"));
    }

    #[test]
    fn different_bytes_hash_differently() {
        assert_ne!(sha256_hex(b"a son"), sha256_hex(b"a different son"));
    }

    fn gradient(w: u32, h: u32, shift: u8) -> image::DynamicImage {
        let mut img = image::RgbaImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let v = ((x * 7 + y * 3) as u8).wrapping_add(shift);
            *px = image::Rgba([v, v.wrapping_mul(2), 90, 255]);
        }
        image::DynamicImage::ImageRgba8(img)
    }

    /// An identical image must be a distance of zero, or the guard would reject
    /// edits that changed nothing at all.
    #[test]
    fn identical_images_have_no_distance() {
        let a = gradient(200, 120, 0);
        assert_eq!(hamming(dhash(&a), dhash(&a)), 0);
        assert!(is_plausible_edit(&a, &a));
    }

    /// A caption-sized overlay is the change the edit is *for*, so it has to pass.
    #[test]
    fn removing_a_caption_still_counts_as_an_edit() {
        let base = gradient(200, 120, 0);
        let mut captioned = base.to_rgba8();
        // A bar across the bottom fifth, roughly what meme text occupies.
        for y in 96..120 {
            for x in 0..200 {
                captioned.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let captioned = image::DynamicImage::ImageRgba8(captioned);
        assert!(
            is_plausible_edit(&captioned, &base),
            "distance was {}",
            hamming(
                dhash(&crate::storage::to_square(&captioned)),
                dhash(&crate::storage::to_square(&base))
            )
        );
    }

    /// The case this guard exists for: a completely different picture coming back.
    #[test]
    fn a_different_picture_is_rejected() {
        let original = gradient(200, 120, 0);
        // Structurally unrelated: vertical bands rather than a diagonal ramp.
        let mut other = image::RgbaImage::new(200, 120);
        for (x, y, px) in other.enumerate_pixels_mut() {
            let v = if (x / 8) % 2 == 0 { 20u8 } else { 235u8 };
            *px = image::Rgba([v, 255 - v, (y as u8).wrapping_mul(3), 255]);
        }
        let other = image::DynamicImage::ImageRgba8(other);
        let d = hamming(
            dhash(&crate::storage::to_square(&original)),
            dhash(&crate::storage::to_square(&other)),
        );
        assert!(
            !is_plausible_edit(&original, &other),
            "distance was only {d}"
        );
    }

    #[test]
    fn hash_is_64_hex_chars() {
        let h = sha256_hex(b"anything");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
