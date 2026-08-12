use serde::{Deserialize, Serialize};

/// One son. Shared verbatim between the server and the wasm bundle, so this
/// struct must not name any ssr-only type (no chrono, no reqwest) — timestamps
/// travel as RFC3339 strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Son {
    pub id: String,
    /// URL-safe form of the title, unique across sons. What `/son/:slug` matches
    /// on; falls back to the id for a son whose title had nothing sluggable in
    /// it. Always prefer this over `id` when building a link.
    pub slug: String,
    pub title: String,
    /// Public URLs, not filesystem paths.
    pub orig_url: String,
    pub thumb_url: String,
    pub width: u32,
    pub height: u32,
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
}

/// Where an upload is. Ordered as the pipeline actually runs, which is not the
/// order anyone would guess: the fingerprint is taken first because the
/// duplicate check has to happen before anything expensive.
// Hash so the upload page can key a `<For>` over `Step::ALL` by the step
// itself, which is already a unique, stable identity -- no index needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Bytes arriving from the browser.
    Receiving,
    /// Decode, then SHA-256 of the pixel buffer, then the duplicate lookup.
    Fingerprinting,
    /// Gemini deciding: safe, and actually a son?
    Scanning,
    /// Gemini drawing the square version.
    Regenerating,
    /// Centre crop and scale to the fixed canvas.
    Cropping,
    /// Invisible provenance watermark, PNG re-encode, thumbnail, upload to R2.
    Storing,
}

impl Step {
    /// Shown verbatim in the UI. Present tense, because it names what is
    /// happening right now rather than a stage in an abstract pipeline.
    pub fn label(self) -> &'static str {
        match self {
            Step::Receiving => "Uploading",
            Step::Fingerprinting => "Fingerprinting",
            Step::Scanning => "Scanning",
            Step::Regenerating => "Regenerating",
            Step::Cropping => "Cropping",
            Step::Storing => "Saving",
        }
    }

    /// Every step, in order, so the UI can draw the whole list up front and
    /// light each one as it completes instead of having rows appear one by one.
    pub const ALL: [Step; 6] = [
        Step::Receiving,
        Step::Fingerprinting,
        Step::Scanning,
        Step::Regenerating,
        Step::Cropping,
        Step::Storing,
    ];
}

/// A job's public state, exactly as the browser polls it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Progress {
    /// Still going. `step` is the one currently running.
    Running { step: Step },
    /// Finished. The son is published.
    Done { son: Box<Son> },
    /// Refused, with a line fit to show a visitor.
    Rejected { reason: String },
    /// Broke. Also what a client gets for an id that no longer exists.
    Failed { message: String },
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
    /// A-Z by title.
    Az,
}

impl Sort {
    pub fn as_str(&self) -> &'static str {
        match self {
            Sort::Newest => "newest",
            Sort::MostLiked => "liked",
            Sort::Az => "az",
        }
    }

    pub fn from_str_or_default(s: &str) -> Self {
        match s {
            "liked" => Sort::MostLiked,
            "az" => Sort::Az,
            _ => Sort::Newest,
        }
    }
}

/// Whether screening is actually working, as shown on the admin page.
///
/// `usable` counts accounts answering right now, which is not the same as
/// accounts that started: a session with expired cookies initialises fine and
/// then fails every call.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScreeningStatus {
    /// `false` when GEMINI_URL is unset -- screening is off, not broken.
    pub configured: bool,
    pub usable: u32,
    pub initialised: u32,
    /// Present when the sidecar could not be reached or reported an error.
    pub error: Option<String>,
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
    /// Accepted for processing. The pipeline takes the best part of a minute
    /// because Gemini generates an image in the middle of it, so the response
    /// carries a job id to poll rather than the finished son.
    Queued {
        job: String,
    },
    Ok {
        son: Son,
    },
    /// Refused before anything was stored. The only remaining reason is an
    /// exact duplicate -- there is no content analysis, so nothing here is a
    /// judgement about what the image depicts.
    Rejected {
        reason: String,
    },
    Error {
        message: String,
    },
}
