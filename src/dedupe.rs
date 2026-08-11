//! Duplicate detection, in two layers:
//!
//! 1. **Exact**: a SHA-256 of the decoded pixel buffer. Catches the same
//!    file (or the same image re-saved in a different container) uploaded
//!    twice, with a single indexed lookup.
//! 2. **Near**: cosine similarity between CLIP embeddings. Catches a resize,
//!    recompress, or minor crop of something already here -- cases where the
//!    exact hash differs but the image is unmistakably the same son.
//!
//! Near-duplicate detection is a full scan over every stored embedding,
//! which is fine at this site's current scale (uploads are not a hot path,
//! and there is no index structure here for approximate nearest-neighbor
//! search over embeddings). Revisit with an ANN index if the collection
//! grows enough for this to show up as real upload latency.

use sha2::{Digest, Sha256};

/// Cosine similarity above this is treated as "the same son," not just a
/// similar one. CLIP embeddings for genuinely different images this site
/// would plausibly receive (different son variants, different photos)
/// generally sit well below this; a resize/recompress/light crop of the same
/// source image sits close to 1.0. Starting point, not a calibrated
/// constant -- worth revisiting once there's a real corpus of near-duplicate
/// reports to check it against.
pub const NEAR_DUPLICATE_THRESHOLD: f32 = 0.97;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `None` if either vector is empty or they aren't the same length --
/// callers only ever compare embeddings from the same CLIP model, so a
/// length mismatch means something is wrong upstream, not a valid
/// "dissimilar" answer.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return None;
    }
    Some(dot / (norm_a * norm_b))
}

/// Scans every stored embedding for one within `NEAR_DUPLICATE_THRESHOLD` of
/// `embedding`. Returns the id of the closest match found, if any is close
/// enough to count.
pub async fn find_near_duplicate(embedding: &[f32]) -> anyhow::Result<Option<String>> {
    let existing = crate::db::all_embeddings().await?;
    let mut best: Option<(String, f32)> = None;

    for (id, other) in existing {
        let Some(sim) = cosine_similarity(embedding, &other) else {
            continue;
        };
        if sim >= NEAR_DUPLICATE_THRESHOLD && best.as_ref().is_none_or(|(_, b)| sim > *b) {
            best = Some((id, sim));
        }
    }

    Ok(best.map(|(id, _)| id))
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

    #[test]
    fn hash_is_64_hex_chars() {
        let h = sha256_hex(b"anything");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn identical_vectors_are_maximally_similar() {
        let v = vec![0.5, 0.2, -0.3, 0.8];
        assert!((cosine_similarity(&v, &v).unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn opposite_vectors_are_minimally_similar() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_similarity(&a, &b).unwrap() - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn mismatched_lengths_have_no_defined_similarity() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), None);
    }
}
