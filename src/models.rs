use serde::{Deserialize, Serialize};

/// One son. Shared verbatim between the server and the wasm bundle, so this
/// struct must not name any ssr-only type (no chrono, no reqwest) — timestamps
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
    pub likes: i64,
    /// Whether the current visitor has already liked this one. Populated per
    /// request from their anonymous cookie ID; not a property of the son.
    pub liked_by_me: bool,
    /// `None` for an anonymous upload — still the common case, since logging
    /// in is additive, not required.
    pub uploader: Option<Uploader>,
    pub tags: Vec<Tag>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub slug: String,
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

/// Gallery ordering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    #[default]
    Newest,
    MostLiked,
}

impl Sort {
    pub fn as_str(&self) -> &'static str {
        match self {
            Sort::Newest => "newest",
            Sort::MostLiked => "liked",
        }
    }

    pub fn from_str_or_default(s: &str) -> Self {
        match s {
            "liked" => Sort::MostLiked,
            _ => Sort::Newest,
        }
    }
}

/// A signed-in user, as far as the UI is concerned.
///
/// Deliberately thin: no email, no Google subject id. Those have no reason to
/// reach the client, and keeping them out of this struct means they can never
/// leak through a server function response by accident.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
}

/// The uploader shown on a card/detail page. Thin on purpose, same reasoning
/// as `User`: no id, no email, nothing beyond what's actually displayed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Uploader {
    pub display_name: String,
    pub avatar_url: Option<String>,
}

/// One row of `/leaderboard`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub upload_count: i64,
}

/// Why a son was reported. A plain string on the wire (see `reports.reason`'s
/// comment for why it isn't an enforced DB constraint), but typed here so the
/// UI can't submit something the report queue doesn't know how to label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportReason {
    NotSon,
    Spam,
    Porn,
    Stolen,
}

impl ReportReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReportReason::NotSon => "not_son",
            ReportReason::Spam => "spam",
            ReportReason::Porn => "porn",
            ReportReason::Stolen => "stolen",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ReportReason::NotSon => "not a son",
            ReportReason::Spam => "spam",
            ReportReason::Porn => "porn / NSFW",
            ReportReason::Stolen => "stolen / not theirs to post",
        }
    }

    pub fn all() -> [ReportReason; 4] {
        [
            ReportReason::NotSon,
            ReportReason::Spam,
            ReportReason::Porn,
            ReportReason::Stolen,
        ]
    }

    pub fn from_str_or_default(s: &str) -> Self {
        match s {
            "spam" => ReportReason::Spam,
            "porn" => ReportReason::Porn,
            "stolen" => ReportReason::Stolen,
            _ => ReportReason::NotSon,
        }
    }
}

/// One report against a son, as shown in the admin queue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportDetail {
    pub reason: String,
    pub message: Option<String>,
    pub created_at: String,
}

/// A son with at least one report, plus every report against it — the admin
/// queue's unit of review.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlaggedSon {
    pub son: Son,
    pub reports: Vec<ReportDetail>,
}

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
