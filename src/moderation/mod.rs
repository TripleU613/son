//! Upload gating.
//!
//! The site auto-publishes anything that clears the thresholds, with no human
//! review queue, so this module is the only thing standing between an upload
//! and the front page. Two consequences shape the design:
//!
//! 1. `Verdict` carries an `embedding`. Even before anything consumes it, every
//!    upload gets one stored — it is the dataset for dedupe, "similar sons",
//!    and eventually a generator. You cannot backfill uploads you never
//!    embedded.
//! 2. Failing closed is the default. If a classifier errors, the upload is
//!    rejected rather than waved through.

use image::DynamicImage;

pub mod stub;

/// Reject anything at or above this NSFW confidence.
pub const NSFW_MAX: f32 = 0.5;

/// Accept anything at or above this "is it really a son" confidence.
/// Deliberately permissive while the gallery is small — the failure mode of a
/// too-strict gate is an empty site, which is worse than a few loose variants.
pub const SON_MIN: f32 = 0.15;

#[derive(Clone, Debug)]
pub struct Verdict {
    pub son_score: f32,
    pub nsfw_score: f32,
    /// CLIP image embedding, when the backend produces one.
    pub embedding: Option<Vec<f32>>,
}

impl Verdict {
    pub fn passes(&self) -> bool {
        self.nsfw_score < NSFW_MAX && self.son_score >= SON_MIN
    }

    /// Why this upload was turned away, for showing the uploader.
    pub fn rejection_reason(&self) -> Option<&'static str> {
        if self.nsfw_score >= NSFW_MAX {
            Some("This one reads as NSFW. Son collection is a family establishment.")
        } else if self.son_score < SON_MIN {
            Some("Can't find the son in this. Needs more son.")
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(son: f32, nsfw: f32) -> Verdict {
        Verdict {
            son_score: son,
            nsfw_score: nsfw,
            embedding: None,
        }
    }

    #[test]
    fn nsfw_blocks_even_a_perfect_son() {
        let verdict = v(1.0, NSFW_MAX);
        assert!(!verdict.passes(), "NSFW_MAX must be exclusive");
        assert!(verdict.rejection_reason().unwrap().contains("NSFW"));
    }

    #[test]
    fn son_min_is_inclusive() {
        assert!(v(SON_MIN, 0.0).passes());
        assert!(!v(SON_MIN - 0.01, 0.0).passes());
    }

    /// NSFW is reported ahead of low-sonness: telling someone their image was
    /// "not son enough" when it was actually rejected as explicit would be
    /// actively misleading.
    #[test]
    fn nsfw_reason_wins_when_both_fail() {
        let reason = v(0.0, 0.99).rejection_reason().unwrap();
        assert!(reason.contains("NSFW"));
    }

    #[test]
    fn passing_verdict_has_no_reason() {
        assert!(v(0.9, 0.01).rejection_reason().is_none());
    }
}

pub trait Moderator: Send + Sync + 'static {
    fn assess(&self, img: &DynamicImage) -> anyhow::Result<Verdict>;

    /// Human-readable backend name, surfaced at startup so it is obvious in the
    /// logs when the real classifier is not actually running.
    fn name(&self) -> &'static str;
}
