//! Zero-shot moderation with CLIP (ViT-B/32) via candle.
//!
//! One model answers both questions the site needs:
//!
//! - is this NSFW? → similarity against explicit-content prompts
//! - is this actually a son? → similarity against Son-meme prompts
//!
//! and it hands back the image embedding as a side effect, which is the whole
//! reason CLIP was chosen over a dedicated NSFW classifier. Those 512-dim
//! vectors accumulate from the first upload and become the dataset for dedupe,
//! "similar sons", and eventually a generator.
//!
//! Scoring is contrastive, not absolute. A raw cosine similarity to "porn" is
//! meaningless on its own, so each question is posed as a softmax over a set of
//! competing captions and the score is the probability mass landing on the
//! positive captions. That makes the thresholds in the parent module meaningful.

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::clip::{ClipConfig, ClipModel};
use image::DynamicImage;
use tokenizers::Tokenizer;

use super::{Moderator, Verdict};

const MODEL_REPO: &str = "openai/clip-vit-base-patch32";
const IMAGE_SIZE: usize = 224;

/// CLIP's published channel statistics. Wrong values here silently degrade
/// every score, so they are spelled out rather than inlined.
const MEAN: [f32; 3] = [0.481_454_67, 0.457_827_5, 0.408_210_73];
const STD: [f32; 3] = [0.268_629_54, 0.261_302_6, 0.275_777_1];

/// Captions asserting explicit content, followed by competing benign captions.
/// `NSFW_POSITIVE` marks how many leading entries count as "unsafe".
/// The benign side is deliberately much larger than the explicit side.
///
/// With only a handful of narrow benign captions, an image resembling none of
/// them (a solid colour, a gradient, noise, flat graphics) had nowhere for its
/// probability mass to go and landed on the explicit captions by default — a
/// plain yellow square scored 0.57 NSFW. Broad coverage of the harmless half of
/// image space is what makes the remaining mass on explicit captions mean
/// something.
const NSFW_POSITIVE: usize = 5;
const NSFW_PROMPTS: &[&str] = &[
    "an explicit pornographic photograph",
    "a photograph of nude genitalia",
    "a photograph of explicit sexual activity",
    "a photograph of a naked person",
    "a graphic photograph of gore, blood and mutilation",
    // Competing benign captions — broad on purpose.
    "a funny internet meme image",
    "a photograph of a fully clothed person",
    "a screenshot of text on a screen",
    "a photograph of an ordinary everyday object",
    "a plain solid colour background",
    "an abstract pattern or gradient",
    "random visual noise or static",
    "a cartoon or digital drawing",
    "a logo, icon or piece of flat graphic design",
    "a photograph of an animal",
    "a photograph of food",
    "a landscape or outdoor scene",
    "a photograph of a building or street",
    "a chart, diagram or user interface",
];

/// Captions describing the Son meme and its variants, then competing captions
/// for the things people upload that are simply not sons.
const SON_POSITIVE: usize = 5;
const SON_PROMPTS: &[&str] = &[
    "the Son crying emoji meme with Anthony Mackie",
    "an internet meme image captioned with the word son",
    "a meme image with crying laughing emoji overlaid",
    "a funny image macro with bold caption text",
    "a photoshopped meme of a face merged with an object",
    // Competing captions. An advertising banner also carries bold caption text,
    // so several variants of "commercial graphic" appear here — one caption was
    // not enough to keep ad creative out.
    "a plain corporate stock photograph",
    "a screenshot of a spreadsheet or document",
    "an advertising banner graphic",
    "a promotional sale advertisement with large text",
    "a website header or marketing banner",
    "a blurry accidental camera photograph",
    "a plain solid colour background",
    "random visual noise or static",
    "an ordinary snapshot with no caption text",
];

/// Default location for a checked-out copy of the weights.
const DEFAULT_MODEL_DIR: &str = "models/clip-vit-base-patch32";

/// Locate the weights and tokenizer on local disk.
///
/// Deliberately does NOT download. An earlier version used `hf-hub`, which
/// dragged in ureq → rustls 0.21 → rustls-webpki 0.101.7 and a HIGH advisory,
/// for a code path only ever used to populate a dev machine once. Fetching
/// 600MB lazily at boot was never a deployment story either, so the download is
/// a documented `curl` in the README instead.
fn locate_model() -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf)> {
    let dir = std::env::var("CLIP_MODEL_DIR").unwrap_or_else(|_| DEFAULT_MODEL_DIR.to_string());
    let dir = std::path::Path::new(&dir);
    let weights = dir.join("pytorch_model.bin");
    let tokenizer = dir.join("tokenizer.json");

    if weights.is_file() && tokenizer.is_file() {
        tracing::info!("CLIP weights from {}", dir.display());
        return Ok((weights, tokenizer));
    }

    anyhow::bail!(
        "CLIP weights not found in {}. Fetch them from {MODEL_REPO}:\n  \
         mkdir -p {} && cd $_ && for f in pytorch_model.bin tokenizer.json; do \
         curl -sLO \"https://huggingface.co/{MODEL_REPO}/resolve/main/$f\"; done",
        dir.display(),
        dir.display()
    )
}

pub struct ClipModerator {
    model: ClipModel,
    device: Device,
    /// Pre-computed, L2-normalised text embeddings. Encoding the captions once
    /// at startup keeps per-upload work to a single image forward pass.
    nsfw_text: Tensor,
    son_text: Tensor,
}

impl ClipModerator {
    /// Download (or reuse the cached) weights and encode the caption sets.
    ///
    /// Blocking and slow on a cold cache — roughly 600MB — so call this at
    /// startup, never on a request path.
    pub fn load() -> anyhow::Result<Self> {
        let (weights, tokenizer_path) = locate_model()?;

        let device = Device::Cpu;
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("could not load CLIP tokenizer: {e}"))?;

        let vb = VarBuilder::from_pth(&weights, DType::F32, &device)
            .map_err(|e| anyhow::anyhow!("could not load CLIP weights: {e}"))?;

        let config = ClipConfig::vit_base_patch32();
        let model = ClipModel::new(vb, &config)
            .map_err(|e| anyhow::anyhow!("could not build CLIP: {e}"))?;

        let nsfw_text = encode_prompts(&model, &tokenizer, &device, NSFW_PROMPTS)?;
        let son_text = encode_prompts(&model, &tokenizer, &device, SON_PROMPTS)?;

        Ok(Self {
            model,
            device,
            nsfw_text,
            son_text,
        })
    }
}

/// Tokenise and encode captions, padding to a common length because CLIP's text
/// encoder takes a fixed-width batch.
fn encode_prompts(
    model: &ClipModel,
    tokenizer: &Tokenizer,
    device: &Device,
    prompts: &[&str],
) -> anyhow::Result<Tensor> {
    let mut rows: Vec<Vec<u32>> = Vec::with_capacity(prompts.len());
    for p in prompts {
        let enc = tokenizer
            .encode(*p, true)
            .map_err(|e| anyhow::anyhow!("tokenizing {p:?} failed: {e}"))?;
        rows.push(enc.get_ids().to_vec());
    }

    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let pad = tokenizer.get_padding().map_or(0, |p| p.pad_id);
    for r in rows.iter_mut() {
        r.resize(width, pad);
    }

    let flat: Vec<u32> = rows.concat();
    let ids = Tensor::from_vec(flat, (rows.len(), width), device)?;

    let feats = model
        .get_text_features(&ids)
        .map_err(|e| anyhow::anyhow!("CLIP text encode failed: {e}"))?;

    normalize(&feats)
}

/// L2-normalise along the feature dimension so a dot product is a cosine.
fn normalize(t: &Tensor) -> anyhow::Result<Tensor> {
    let norm = t.sqr()?.sum_keepdim(1)?.sqrt()?;
    Ok(t.broadcast_div(&norm)?)
}

/// Resize to CLIP's input square and apply its channel normalisation.
fn preprocess(img: &DynamicImage, device: &Device) -> anyhow::Result<Tensor> {
    let resized = img
        .resize_exact(
            IMAGE_SIZE as u32,
            IMAGE_SIZE as u32,
            image::imageops::FilterType::Triangle,
        )
        .to_rgb8();

    let data = resized.into_raw();
    // (H, W, C) as bytes, then to (C, H, W) floats.
    let t = Tensor::from_vec(data, (IMAGE_SIZE, IMAGE_SIZE, 3), device)?
        .permute((2, 0, 1))?
        .to_dtype(DType::F32)?
        .affine(1.0 / 255.0, 0.0)?;

    let mean = Tensor::from_vec(MEAN.to_vec(), (3, 1, 1), device)?;
    let std = Tensor::from_vec(STD.to_vec(), (3, 1, 1), device)?;

    Ok(t.broadcast_sub(&mean)?.broadcast_div(&std)?.unsqueeze(0)?) // batch of one
}

/// Softmax over `image · captions`, summing the probability assigned to the
/// leading `positive` captions.
///
/// CLIP's learned temperature (logit_scale = 100) is what makes the softmax
/// decisive rather than mush; without it every score hovers near uniform.
fn positive_mass(image: &Tensor, text: &Tensor, positive: usize) -> anyhow::Result<f32> {
    let logits = (image.matmul(&text.t()?)? * 100.0)?;
    let probs = candle_nn::ops::softmax(&logits, 1)?
        .squeeze(0)?
        .to_vec1::<f32>()?;
    Ok(probs.iter().take(positive).sum())
}

impl Moderator for ClipModerator {
    fn assess(&self, img: &DynamicImage) -> anyhow::Result<Verdict> {
        let pixels = preprocess(img, &self.device)?;
        let image_features = normalize(
            &self
                .model
                .get_image_features(&pixels)
                .map_err(|e| anyhow::anyhow!("CLIP image encode failed: {e}"))?,
        )?;

        let nsfw_score = positive_mass(&image_features, &self.nsfw_text, NSFW_POSITIVE)?;
        let son_score = positive_mass(&image_features, &self.son_text, SON_POSITIVE)?;

        // Kept for the dataset, not for this decision.
        let embedding = image_features.squeeze(0)?.to_vec1::<f32>()?;

        Ok(Verdict {
            son_score,
            nsfw_score,
            embedding: Some(embedding),
        })
    }

    fn name(&self) -> &'static str {
        "CLIP ViT-B/32 (candle, cpu)"
    }
}
