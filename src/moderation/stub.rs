//! Placeholder classifier used until CLIP is wired in.
//!
//! It does NOT detect NSFW content and it does NOT detect sons. It applies a
//! couple of cheap structural sanity checks and otherwise waves things through
//! with neutral scores. Do not expose this to the public internet and believe
//! you have moderation — see the warning it logs at startup.

use image::DynamicImage;

use super::{Moderator, Verdict};

pub struct StubModerator;

impl Moderator for StubModerator {
    fn assess(&self, img: &DynamicImage) -> anyhow::Result<Verdict> {
        let (w, h) = (img.width(), img.height());

        // Absurd aspect ratios are almost never memes; usually banners or
        // accidental screenshots. This is the one real signal available here.
        let ratio = w as f32 / h as f32;
        let plausible_meme = (0.25..=4.0).contains(&ratio) && w >= 100 && h >= 100;

        Ok(Verdict {
            // Just above SON_MIN so plausible images pass, below it so junk
            // shapes are turned away.
            son_score: if plausible_meme { 0.50 } else { 0.05 },
            nsfw_score: 0.0,
            embedding: None,
        })
    }

    fn name(&self) -> &'static str {
        "stub (NO REAL MODERATION)"
    }
}
