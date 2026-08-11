//! Exact duplicate detection: a SHA-256 of the decoded pixel buffer.
//!
//! Hashing the decoded pixels rather than the uploaded bytes is what makes this
//! useful — the same image re-saved as a different format, or with different
//! compression settings, produces different file bytes but identical pixels, and
//! is still the same son.
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
}
