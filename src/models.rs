use serde::{Deserialize, Serialize};

/// One son. Shared verbatim between the server and the wasm bundle, so this
/// struct must not name any ssr-only type (no chrono, no sqlx) — timestamps
/// travel as RFC3339 strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Son {
    pub id: String,
    pub title: String,
    /// Public URLs, not filesystem paths.
    pub orig_url: String,
    pub thumb_url: String,
    pub width: u32,
    pub height: u32,
    /// How much the classifier believes this is a Son variant. 0.0..=1.0
    pub son_score: f32,
    /// How much the classifier believes this is NSFW. 0.0..=1.0
    pub nsfw_score: f32,
    pub created_at: String,
    pub is_public: bool,
    pub reports: i64,
}

impl Son {
    /// Text for the little badge on the card.
    pub fn sonness_label(&self) -> String {
        format!("{:.0}% son", self.son_score * 100.0)
    }
}

/// A page of sons plus the cursor needed to ask for the next one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SonPage {
    pub sons: Vec<Son>,
    /// `created_at` of the last row, or `None` when the gallery is exhausted.
    pub next_cursor: Option<String>,
}

pub const PAGE_SIZE: i64 = 24;

/// Reply from `POST /api/upload`.
///
/// Lives here rather than beside the handler so the wasm side deserializes the
/// exact type the server serializes — two hand-matched shapes would drift.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UploadResult {
    Ok {
        son: Son,
    },
    Rejected {
        reason: String,
        son_score: f32,
        nsfw_score: f32,
    },
    Error {
        message: String,
    },
}
