//! Refuses every upload.
//!
//! Used when the real classifier cannot be loaded. The alternative — quietly
//! dropping back to the stub — would turn a model-loading failure into an open
//! door on a site that auto-publishes. The gallery keeps serving; only
//! contributions pause.

use image::DynamicImage;

use super::{Moderator, Verdict};

pub struct DenyAll;

impl Moderator for DenyAll {
    fn assess(&self, _img: &DynamicImage) -> anyhow::Result<Verdict> {
        // Scores that fail both gates, so `rejection_reason` has something to say.
        Ok(Verdict {
            son_score: 0.0,
            nsfw_score: 1.0,
            embedding: None,
        })
    }

    fn name(&self) -> &'static str {
        "deny-all (classifier unavailable)"
    }
}
