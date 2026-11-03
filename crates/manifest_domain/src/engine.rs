//! Engine seam types: connection, stream, and turn values the shell renders.
//!
//! The engine seam is the boundary between the presentation layer and the
//! agent backend (the Muse MCP implementation in the data crate; BYOK
//! engines later). These types carry everything an engine reports to the
//! presentation layer and perform no I/O.

use crate::fixture::{SessionId, SessionSummary, TodoItem};

/// Opaque handle for one agent turn.
///
/// Mirrors [`SessionId`]: callers compare and match by the whole handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TurnId(String);

impl TurnId {
    /// Build a turn id from its stored handle.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the stored handle for wire calls.
    ///
    /// Callers still compare and match by the whole handle.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How far a transcript item got on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemStatus {
    /// The item is still streaming.
    InProgress,
    /// The item finished cleanly.
    Completed,
    /// The item finished with an error; details ride on the item.
    Failed,
    /// Any other wire status, carried verbatim for forward compatibility.
    ///
    /// Holds the raw status text, or an empty string when the wire sent
    /// none. Unknown statuses stay values, never parse failures.
    Other(String),
}

/// One transcript item, typed from the wire object.
///
/// The mapper classifies every item the host streams; the shell renders
/// from these variants instead of raw JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamItem {
    /// Something the user wrote; the prompt echo.
    UserMessage {
        /// Host item handle; joins deltas to this item.
        item_id: String,
        /// Prompt text; empty when the wire sent none.
        text: String,
    },
    /// Something the agent wrote.
    AgentMessage {
        /// Host item handle; joins deltas to this item.
        item_id: String,
        /// Assistant markdown; empty at started, authoritative at completed.
        text: String,
        /// Raw message phase (only `finalAnswer` observed); other phases
        /// stay forward-compatible values.
        phase: Option<String>,
    },
    /// The host's abstract of the turn's plan, not raw thinking text.
    Reasoning {
        /// Host item handle; joins deltas to this item.
        item_id: String,
        /// Summary sentences; string entries only, empty when none sent.
        summary: Vec<String>,
    },
    /// One tool invocation with its headline and result.
    ToolCall {
        /// Host item handle; joins deltas to this item.
        item_id: String,
        /// Tool name; empty when the wire sent none.
        tool: String,
        /// One-line headline parsed from the args; none when no arg fits.
        headline: Option<String>,
        /// Raw args JSON text, kept for later slices; empty when none sent.
        args_json: String,
        /// Parsed todo list, carried only for the todo tool; `None`
        /// for every other tool and for malformed todo args.
        todos: Option<Vec<TodoItem>>,
        /// How far the call got.
        status: ItemStatus,
        /// Renderable result text after envelope unwrap; none until sent.
        output: Option<String>,
        /// Machine-readable failure detail; present on failed calls.
        failure: Option<String>,
    },
    /// Host bookkeeping with no user-facing content.
    ///
    /// Known kinds (for example `reminderChild`) the shell deliberately
    /// does not render.
    System {
        /// Host item handle; joins deltas to this item.
        item_id: String,
        /// Wire kind, kept so new system kinds stay classifiable.
        kind: String,
    },
    /// A context-compaction turn: the host summarised history to free
    /// the window. Failures after admission arrive as this item's
    /// terminal outcome, never as wire errors, so the outcome and
    /// reason ride here and the shell renders them.
    Compaction {
        /// Host item handle; joins deltas to this item.
        item_id: String,
        /// Terminal outcome (`compacted`, `noop`, `failed`,
        /// `cancelled`); none while the item is still in progress.
        outcome: Option<String>,
        /// Machine reason verbatim (for example
        /// `no_compactable_history`); present on some terminals.
        reason: Option<String>,
        /// Window tokens before compaction; none when unsent.
        tokens_before: Option<u64>,
        /// Window tokens after compaction; none when unsent.
        tokens_after: Option<u64>,
    },
    /// Any other kind; the shell renders a generic note, never a drop.
    Unknown {
        /// Host item handle; joins deltas to this item.
        item_id: String,
        /// Wire kind, kept so the note can name it.
        kind: String,
    },
}

impl StreamItem {
    /// Borrow the host item handle, whatever the variant.
    pub fn item_id(&self) -> &str {
        match self {
            Self::UserMessage { item_id, .. }
            | Self::AgentMessage { item_id, .. }
            | Self::Reasoning { item_id, .. }
            | Self::ToolCall { item_id, .. }
            | Self::System { item_id, .. }
            | Self::Compaction { item_id, .. }
            | Self::Unknown { item_id, .. } => item_id,
        }
    }
}

/// A session goal's lifecycle situation, parsed leniently from the
/// host's free status string.
///
/// Known situations name themselves; anything else stays a value,
/// never a parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalStatus {
    /// The goal is being worked on.
    Active,
    /// The goal is paused; progress is frozen as sent.
    Paused,
    /// The goal is done; progress renders as complete.
    Completed,
    /// Any other status, carried verbatim for forward compatibility.
    ///
    /// Holds the raw status text, or an empty string when the wire
    /// sent none. Unknown statuses stay values, never parse failures.
    Other(String),
}

impl GoalStatus {
    /// Parse the host's free status string: `active`, `paused`, and
    /// `completed` (any case, surrounding whitespace ignored) name
    /// themselves; anything else stays an [`GoalStatus::Other`] value.
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "active" => Self::Active,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            _ => Self::Other(text.to_string()),
        }
    }
}

/// One session goal, translated from the host's goal block.
///
/// The renderer clamps the percent for display; the carried value
/// stays verbatim (it may exceed 100).
#[derive(Debug, Clone, PartialEq)]
pub struct GoalInfo {
    /// The goal objective; never empty (the mapper rejects blocks
    /// without one, mirroring the other required wire fields).
    pub objective: String,
    /// Completion percent, verbatim from the wire; may exceed 100.
    pub percent_complete: f64,
    /// Lifecycle situation, parsed leniently from the free string.
    pub status: GoalStatus,
    /// What the agent is doing now, when reported.
    pub current_work: Option<String>,
    /// What comes next, when reported.
    pub next_work: Option<String>,
}

impl GoalInfo {
    /// Display percent in `0.0..=100.0` for rendering. Non-finite
    /// values read as zero; the carried value is untouched.
    pub fn clamped_percent(&self) -> f32 {
        if !self.percent_complete.is_finite() {
            return 0.0;
        }
        (self.percent_complete as f32).clamp(0.0, 100.0)
    }
}

/// The named connection situations the shell renders.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Connection {
    /// No connection to the engine.
    #[default]
    Disconnected,
    /// A connection attempt is in flight.
    Connecting,
    /// Connected and ready to issue commands.
    Connected,
    /// The connection attempt failed.
    ///
    /// Carries a user-actionable message composed at the data boundary.
    Failed(String),
}

/// Which host frame a usage event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageKind {
    /// Cumulative usage snapshot frame.
    TokenUsage,
    /// Change-notice frame.
    Changed,
}

/// Which kind of pending request a pending event hints at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKind {
    /// The engine asks the user to approve an action.
    Approval,
    /// The engine asks the user a question.
    UserInput,
}

/// Which phase of a pending request a pending event hints at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingPhase {
    /// The request was raised and awaits settlement.
    Requested,
    /// The request was settled.
    Settled,
}

/// Host cap on free-text answers, in characters.
///
/// Mirrors `buildUserInputAnswers` in the server's `answers.ts`:
/// free text is sliced to 500 before it reaches the host.
pub const FREE_TEXT_LIMIT: usize = 500;

/// Truncate free text to the host cap.
///
/// Counts characters, not bytes, so multi-byte text is never split.
pub fn truncate_free_text(text: &str) -> String {
    text.chars().take(FREE_TEXT_LIMIT).collect()
}

/// One offered approval choice, summarised for rendering.
///
/// The full wire choice stays at the data boundary; the shell only
/// needs the label and whether a deny may carry feedback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalChoiceSummary {
    /// Human label for the choice; may be empty on older hosts.
    pub label: String,
    /// Wire decision this choice settles, e.g. `approved` or `denied`.
    pub decision: String,
    /// Wire scope, e.g. `once`.
    pub scope: String,
    /// Whether this choice accepts a free-text reason.
    pub accepts_feedback: bool,
}

impl ApprovalChoiceSummary {
    /// Human name for pickers: the label, or the decision when blank.
    pub fn display_label(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.decision
        } else {
            &self.label
        }
    }
}

/// An approval the engine asks the user to settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    /// Tool the approval gates; empty when the wire sent none.
    pub tool: String,
    /// One-line account of what is being approved.
    pub subject_line: String,
    /// Offered choices, summarised.
    pub choices: Vec<ApprovalChoiceSummary>,
}

impl ApprovalRequest {
    /// Labels of the offered choices, in order.
    pub fn choice_labels(&self) -> Vec<&str> {
        self.choices
            .iter()
            .map(ApprovalChoiceSummary::display_label)
            .collect()
    }
}

/// One answer option for a question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    /// Option label; also the answer key when selected.
    pub label: String,
    /// Longer account of the option, when the host sent one.
    pub description: Option<String>,
}

impl QuestionOption {
    /// Whether this option carries a non-empty description.
    pub fn has_description(&self) -> bool {
        self.description
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
    }
}

/// One question in a user-input request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// Question handle; answers key on this id.
    pub id: String,
    /// Full question text.
    pub prompt: String,
    /// Offered options; the synthetic Other choice is shell-added
    /// and never appears here.
    pub options: Vec<QuestionOption>,
    /// Whether several options may be picked.
    pub multi: bool,
}

impl Question {
    /// Labels of the offered options, in order.
    pub fn option_labels(&self) -> Vec<&str> {
        self.options
            .iter()
            .map(|option| option.label.as_str())
            .collect()
    }

    /// Whether `label` is one of the offered option labels.
    pub fn has_option(&self, label: &str) -> bool {
        self.options.iter().any(|option| option.label == label)
    }

    /// Classify raw user text the way the server's
    /// `buildUserInputAnswers` does.
    ///
    /// Multiple-choice text is split on commas and only parts that
    /// exactly match an option label are kept; a single exact label
    /// match selects that label; anything else is free text capped
    /// at [`FREE_TEXT_LIMIT`].
    pub fn classify_text(&self, text: &str) -> AnswerValue {
        if self.multi {
            let picked: Vec<String> = text
                .split(',')
                .map(str::trim)
                .filter(|part| self.has_option(part))
                .map(String::from)
                .collect();
            if !picked.is_empty() {
                return AnswerValue::Labels(picked);
            }
        }
        if self.has_option(text) {
            return AnswerValue::Label(String::from(text));
        }
        AnswerValue::free_text(text)
    }
}

/// A user-input request: one or more questions asked together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionRequest {
    /// Questions in pager order.
    pub questions: Vec<Question>,
}

impl QuestionRequest {
    /// Find a question by id.
    pub fn question(&self, id: &str) -> Option<&Question> {
        self.questions.iter().find(|question| question.id == id)
    }
}

/// One pending approval or question, fetched as source of truth.
///
/// Split by kind so the shell renders an approval card or a
/// question card without re-classifying a raw payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingRequest {
    /// An approval awaiting a decision.
    Approval {
        /// Session the request belongs to.
        session: SessionId,
        /// Opaque request handle for decide calls.
        request_id: String,
        /// The approval to render.
        approval: ApprovalRequest,
    },
    /// A question set awaiting answers.
    Question {
        /// Session the request belongs to.
        session: SessionId,
        /// Opaque request handle for answer calls.
        request_id: String,
        /// The questions to render.
        question: QuestionRequest,
    },
}

impl PendingRequest {
    /// Session the request belongs to, whatever the kind.
    pub fn session(&self) -> &SessionId {
        match self {
            Self::Approval { session, .. } | Self::Question { session, .. } => session,
        }
    }

    /// Opaque request handle, whatever the kind.
    pub fn request_id(&self) -> &str {
        match self {
            Self::Approval { request_id, .. } | Self::Question { request_id, .. } => request_id,
        }
    }

    /// Which kind of request this is.
    pub fn kind(&self) -> PendingKind {
        match self {
            Self::Approval { .. } => PendingKind::Approval,
            Self::Question { .. } => PendingKind::UserInput,
        }
    }

    /// Questions to render, when this is a question request.
    pub fn questions(&self) -> Option<&[Question]> {
        match self {
            Self::Question { question, .. } => Some(&question.questions),
            Self::Approval { .. } => None,
        }
    }
}

/// How a pending request settled.
///
/// `Expired` is an approval that timed out and was denied by the
/// server; `Cancelled` is a question the server cancelled on
/// timeout. Both are named terminal states, never a spinner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingResolution {
    /// The user approved the request.
    Approved,
    /// The user denied the request, with an optional reason.
    Denied(Option<String>),
    /// The user answered, summarised in one line.
    Answered(String),
    /// The request expired (approvals deny on expiry).
    Expired,
    /// The request was cancelled before an answer.
    Cancelled,
}

impl PendingResolution {
    /// Short human name for the settled one-line record.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Approved => "Approved",
            Self::Denied(_) => "Denied",
            Self::Answered(_) => "Answered",
            Self::Expired => "Expired",
            Self::Cancelled => "Cancelled",
        }
    }

    /// Carried detail: the deny reason or the answer summary.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Denied(reason) => reason.as_deref(),
            Self::Answered(summary) => Some(summary),
            _ => None,
        }
    }
}

/// The user's decision on an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Approve the gated action.
    Approve,
    /// Deny the gated action.
    Deny,
}

/// Alias matching the pending-request vocabulary.
pub type PendingDecision = ApprovalDecision;

impl ApprovalDecision {
    /// Wire value for `muse_decide`'s `decision`.
    pub fn wire_value(&self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
        }
    }

    /// Short human name.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Approve => "Approve",
            Self::Deny => "Deny",
        }
    }
}

/// Which wire field an [`AnswerValue`] maps to.
///
/// Mirrors the host's exactly-one-of rule: `selectedLabel`,
/// `selectedLabels`, or `freeText`. `Skipped` carries none of
/// them; how a skip is encoded on the wire is the data layer's
/// call, and no skipped value exists in the server's answer map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireAnswerKind {
    /// Maps to `selectedLabel`.
    SelectedLabel,
    /// Maps to `selectedLabels`.
    SelectedLabels,
    /// Maps to `freeText`.
    FreeText,
    /// No wire field; the question was skipped.
    Skipped,
}

/// One question's answer, as a domain value.
///
/// The data layer maps each variant to the wire shape the server's
/// `buildUserInputAnswers` produces: a label that matches an
/// offered option, a label list for multiple choice, or free text
/// capped at [`FREE_TEXT_LIMIT`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerValue {
    /// A single offered option, by label.
    Label(String),
    /// Several offered options, by label (multiple choice).
    Labels(Vec<String>),
    /// Free text (the synthetic Other choice lands here).
    FreeText(String),
    /// The user skipped this question.
    Skipped,
}

impl AnswerValue {
    /// Build a free-text answer, capped at [`FREE_TEXT_LIMIT`].
    pub fn free_text(text: &str) -> Self {
        Self::FreeText(truncate_free_text(text))
    }

    /// Which wire field this answer maps to.
    pub fn wire_kind(&self) -> WireAnswerKind {
        match self {
            Self::Label(_) => WireAnswerKind::SelectedLabel,
            Self::Labels(_) => WireAnswerKind::SelectedLabels,
            Self::FreeText(_) => WireAnswerKind::FreeText,
            Self::Skipped => WireAnswerKind::Skipped,
        }
    }

    /// Whether this answer is a skip.
    pub fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped)
    }

    /// One-line summary for the settled record.
    pub fn summary(&self) -> String {
        match self {
            Self::Label(label) => label.clone(),
            Self::Labels(labels) => labels.join(", "),
            Self::FreeText(text) => text.clone(),
            Self::Skipped => String::from("Skipped"),
        }
    }
}

/// All answers for one user-input request, keyed by question id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingAnswer {
    /// Per-question answers, in question order.
    pub answers: Vec<(String, AnswerValue)>,
}

impl PendingAnswer {
    /// Build from per-question answers.
    pub fn new(answers: Vec<(String, AnswerValue)>) -> Self {
        Self { answers }
    }

    /// Build a single-question answer.
    pub fn single(question_id: impl Into<String>, value: AnswerValue) -> Self {
        Self {
            answers: vec![(question_id.into(), value)],
        }
    }

    /// The answer for one question, when present.
    pub fn get(&self, question_id: &str) -> Option<&AnswerValue> {
        self.answers
            .iter()
            .find(|(id, _)| id == question_id)
            .map(|(_, value)| value)
    }

    /// Whether every question in `request` has an answer entry.
    pub fn is_complete_for(&self, request: &QuestionRequest) -> bool {
        request
            .questions
            .iter()
            .all(|question| self.get(&question.id).is_some())
    }

    /// One-line summary of all answers for the settled record.
    pub fn summary(&self) -> String {
        self.answers
            .iter()
            .map(|(_, value)| value.summary())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// One server stream event, translated to domain values.
///
/// `PartialEq` without `Eq`: [`StreamEvent::GoalChanged`] carries a
/// float percent, which has no total equality.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// A turn started.
    TurnStarted {
        /// Session the turn belongs to.
        session: SessionId,
        /// Turn that started, when known.
        turn: Option<TurnId>,
        /// Cursor into the transcript view, when known.
        view_cursor: Option<String>,
    },
    /// A transcript item started, typed from the wire object.
    ItemStarted {
        /// Session the item belongs to.
        session: SessionId,
        /// Turn the item belongs to, when known.
        turn: Option<TurnId>,
        /// Cursor into the transcript view, when known.
        view_cursor: Option<String>,
        /// The typed item.
        item: StreamItem,
    },
    /// A delta appended to one field of a transcript item.
    ItemDelta {
        /// Session the item belongs to.
        session: SessionId,
        /// Turn the item belongs to, when known.
        turn: Option<TurnId>,
        /// Cursor into the transcript view, when known.
        view_cursor: Option<String>,
        /// Item the delta appends to.
        item_id: String,
        /// Dotted field path the delta appends to.
        field: String,
        /// Raw delta text.
        delta: String,
    },
    /// A transcript item completed; authoritative over delta concatenation.
    ItemCompleted {
        /// Session the item belongs to.
        session: SessionId,
        /// Turn the item belongs to, when known.
        turn: Option<TurnId>,
        /// Cursor into the transcript view, when known.
        view_cursor: Option<String>,
        /// The typed item.
        item: StreamItem,
    },
    /// A turn completed.
    TurnCompleted {
        /// Session the turn belongs to.
        session: SessionId,
        /// Turn that completed, when known.
        turn: Option<TurnId>,
        /// Cursor into the transcript view, when known.
        view_cursor: Option<String>,
        /// Terminal state of the turn, when reported.
        terminal: Option<String>,
        /// Final answer text, when reported.
        answer: Option<String>,
        /// Delta events whose send failed during that turn; loss is a value, never silent.
        dropped_deltas: u64,
    },
    /// Token-usage report from the host.
    Usage {
        /// Session the usage belongs to.
        session: SessionId,
        /// Turn the usage belongs to, when known.
        turn: Option<TurnId>,
        /// Cursor into the transcript view, when known.
        view_cursor: Option<String>,
        /// Which host frame the usage came from.
        kind: UsageKind,
        /// Host's raw token-usage params as JSON text.
        usage_json: String,
    },
    /// A hint to refresh approval/question state.
    ///
    /// A hint to refresh, never the source of truth for
    /// approval/question payloads.
    Pending {
        /// Session the request belongs to.
        session: SessionId,
        /// Turn the request belongs to, when known.
        turn: Option<TurnId>,
        /// Opaque request handle.
        request_id: String,
        /// Which kind of request this hints at.
        kind: PendingKind,
        /// Which phase the request is in.
        phase: PendingPhase,
    },
    /// The host skill catalog changed for this session.
    ///
    /// A hint to re-list, never the catalog itself: the shell
    /// re-lists through [`SessionEngine::list_skills`] on this event.
    CatalogChanged {
        /// Session whose catalog changed.
        session: SessionId,
    },
    /// The session's goal changed: the new goal, or `None` when the
    /// host cleared it. Replaces the stored goal wholesale.
    GoalChanged {
        /// Session whose goal changed.
        session: SessionId,
        /// The new goal, or `None` when cleared.
        goal: Option<GoalInfo>,
    },
    /// The session's name changed: the host allocated or renamed it,
    /// or the server's spawn-prompt fallback was superseded. Carries
    /// the new canonical name; replaces the displayed title wholesale.
    NameChanged {
        /// Session whose name changed.
        session: SessionId,
        /// The new name now in effect.
        name: String,
    },
}

/// One Muse approval posture, mirroring the server's `approval_mode`.
///
/// Per-engine so a future BYOK scheme slots in beside it as another
/// [`PermissionMode`] variant; the shell matches on the engine, not on
/// bare strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MusePermission {
    /// Tools run without asking; the sandbox stays on.
    ///
    /// This is the server default for spawned sessions.
    #[default]
    Yolo,
    /// The agent asks before each tool run.
    OnRequest,
    /// A reviewer approves each tool run.
    Judge,
}

impl MusePermission {
    /// Every posture, in picker order.
    pub fn all() -> [Self; 3] {
        [Self::Yolo, Self::OnRequest, Self::Judge]
    }

    /// Short human name for pickers.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Yolo => "Yolo",
            Self::OnRequest => "On request",
            Self::Judge => "Judge",
        }
    }

    /// One-line account of what the posture does.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Yolo => "Runs tools without asking. The sandbox stays on.",
            Self::OnRequest => "Asks you before running each tool.",
            Self::Judge => "A reviewer approves each tool run.",
        }
    }

    /// Wire value for the server's `approval_mode`.
    pub fn wire_value(&self) -> &'static str {
        match self {
            Self::Yolo => "yolo",
            Self::OnRequest => "on-request",
            Self::Judge => "judge",
        }
    }

    /// Parse a wire value; unknown text is not a posture.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "yolo" => Some(Self::Yolo),
            "on-request" => Some(Self::OnRequest),
            "judge" => Some(Self::Judge),
            _ => None,
        }
    }
}

/// Which engine's permission scheme a spawn uses.
///
/// Today only Muse exists; a BYOK scheme arrives as another variant.
/// The default mirrors the server default: Muse yolo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Muse `approval_mode` posture.
    Muse(MusePermission),
}

impl Default for PermissionMode {
    /// Mirrors the server default: Muse yolo.
    fn default() -> Self {
        Self::Muse(MusePermission::default())
    }
}

impl PermissionMode {
    /// Short human name for pickers.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Muse(inner) => inner.label(),
        }
    }

    /// One-line account of what the posture does.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Muse(inner) => inner.description(),
        }
    }
}

/// Which model a spawned session runs on.
///
/// [`ModelChoice::Default`] omits `model` so the server (or its
/// environment) decides; [`ModelChoice::Named`] pins a catalog id from
/// [`EngineEvent::ModelsLoaded`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ModelChoice {
    /// Use the server default model.
    #[default]
    Default,
    /// Pin this catalog model id.
    Named(String),
}

impl ModelChoice {
    /// Catalog id to send; `None` omits `model` (default or blank).
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Default => None,
            Self::Named(id) if id.trim().is_empty() => None,
            Self::Named(id) => Some(id),
        }
    }

    /// Short human name for pickers: `Default` or the catalog id.
    pub fn display(&self) -> &str {
        match self {
            Self::Default => "Default",
            Self::Named(id) => id,
        }
    }
}

/// Reasoning effort for a spawned session, mirroring the server's
/// `reasoning_effort` values.
///
/// [`EffortChoice::Default`] omits the key so the server (or its
/// environment) decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EffortChoice {
    /// Use the server default effort.
    #[default]
    Default,
    /// No reasoning effort.
    None,
    /// Minimal reasoning effort.
    Minimal,
    /// Low reasoning effort.
    Low,
    /// Medium reasoning effort.
    Medium,
    /// High reasoning effort.
    High,
    /// Extra-high reasoning effort.
    XHigh,
    /// Maximum reasoning effort.
    Max,
    /// Ultra reasoning effort.
    Ultra,
}

impl EffortChoice {
    /// Every choice, in picker order.
    pub fn all() -> [Self; 9] {
        [
            Self::Default,
            Self::None,
            Self::Minimal,
            Self::Low,
            Self::Medium,
            Self::High,
            Self::XHigh,
            Self::Max,
            Self::Ultra,
        ]
    }

    /// Wire value for `reasoning_effort`; `None` for [`EffortChoice::Default`]
    /// (omit the key).
    pub fn wire_value(&self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::None => Some("none"),
            Self::Minimal => Some("minimal"),
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::XHigh => Some("xhigh"),
            Self::Max => Some("max"),
            Self::Ultra => Some("ultra"),
        }
    }

    /// Parse a wire value (or `"default"`); unknown text is not an effort.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "default" => Some(Self::Default),
            "none" => Some(Self::None),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            "ultra" => Some(Self::Ultra),
            _ => None,
        }
    }
}

/// One row of the host model catalog, mirroring the server's
/// `muse_models` entries; the shell picks from these, never raw JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// Catalog id, sent back as `model`.
    pub id: String,
    /// Human name from the catalog; may be empty on older hosts.
    pub display_label: String,
    /// The host's default model.
    pub is_default: bool,
    /// The model the session (or workspace host) currently runs.
    pub is_active: bool,
}

impl ModelInfo {
    /// Human name for pickers: the display label, or the id when blank.
    pub fn label(&self) -> &str {
        if self.display_label.trim().is_empty() {
            &self.id
        } else {
            &self.display_label
        }
    }
}

/// One row of the host skill catalog, mirroring the server's
/// `muse_skills` entries; the shell picks from these, never raw JSON.
///
/// Selectors are slash-menu handles sent back as the skill part's
/// `selector`; rows without one never reach this type (the mapper
/// skips them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInfo {
    /// Slash-menu handle, sent back as `selector`.
    pub selector: String,
    /// Human name from the catalog; falls back to `selector`.
    pub display_name: String,
    /// One-line account of what the skill does; empty when unsent.
    pub description: String,
    /// Catalog scope (`bundled`, `user`, `project`, or `plugin`).
    pub source: String,
    /// Plugin that provides the skill, when the host named one.
    pub plugin_id: Option<String>,
    /// Argument hint shown in the slash menu, when the host sent one.
    pub argument_hint: Option<String>,
}

impl SkillInfo {
    /// Human name for pickers: the display name, or the selector when blank.
    pub fn label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.selector
        } else {
            &self.display_name
        }
    }
}

/// One skill invocation: a slash-menu pick plus its raw arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInvocation {
    /// Slash-menu handle of the skill to run.
    pub selector: String,
    /// Catalog scope, when known.
    pub source: Option<String>,
    /// Raw argument text after the selector; empty when none was typed.
    pub arguments: String,
}

/// What one turn sends: plain text or a skill invocation.
///
/// The data layer maps `Text` to the server's `prompt` field and
/// `Skill` to a `parts` array carrying one `skill` part; the shell
/// builds this from the composer and echoes
/// [`TurnInput::display_text`] optimistically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnInput {
    /// Plain prompt text, sent as the server's `prompt` field.
    Text(String),
    /// A slash-menu skill pick, sent as one `skill` part.
    Skill(SkillInvocation),
}

impl TurnInput {
    /// Transcript echo: text as-is; skills as `/selector` plus the
    /// arguments when any were typed.
    pub fn display_text(&self) -> String {
        match self {
            Self::Text(prompt) => prompt.clone(),
            Self::Skill(invocation) => {
                if invocation.arguments.trim().is_empty() {
                    format!("/{}", invocation.selector)
                } else {
                    format!("/{} {}", invocation.selector, invocation.arguments)
                }
            }
        }
    }
}

/// One hard-coded composer slash command, listed ahead of the skill
/// catalog rows. Builtins never reach the data layer as turns: the
/// shell folds each into its own behavior on send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinCommand {
    /// Open the model menu instead of sending a turn.
    Model,
    /// Compact the visible session's context through `muse_raw`.
    Compact,
    /// Set the visible session's goal through `muse_raw`.
    Goal,
}

impl BuiltinCommand {
    /// Every builtin, in slash-menu order.
    pub fn all() -> [Self; 3] {
        [Self::Model, Self::Compact, Self::Goal]
    }

    /// Slash-menu handle, typed after `/`.
    pub fn selector(&self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Compact => "compact",
            Self::Goal => "goal",
        }
    }

    /// Short human name for the slash menu.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::Compact => "Compact",
            Self::Goal => "Goal",
        }
    }

    /// One-line account of what accepting the row does.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Model => "Switch the model new sessions spawn with.",
            Self::Compact => "Compact the visible session's context.",
            Self::Goal => "Set the visible session's goal.",
        }
    }

    /// Classify a first-token selector; unknown text is not a builtin.
    pub fn parse(selector: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|builtin| builtin.selector() == selector)
    }
}

/// What the shell does with one composer draft on send.
///
/// Pure data: the shell owns the follow-through (engine calls, menu
/// opens, draft clears), so this stays unit-testable with no engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftAction {
    /// Plain text: send as [`TurnInput::Text`], exactly as typed.
    Text,
    /// A ready catalog skill: send as [`TurnInput::Skill`].
    Skill(SkillInvocation),
    /// Open the model menu instead of sending; the draft is kept.
    OpenModelMenu,
    /// Compact the visible session; the draft clears like a send.
    Compact,
    /// Set the visible session's goal; the draft clears like a send.
    SetGoal(String),
    /// A builtin that needs more input (`/goal` with no objective):
    /// send nothing and keep the draft.
    KeepDraft,
}

/// Parse one composer draft for slash handling.
///
/// Only a draft whose first whitespace-delimited token starts with
/// `/` is a slash draft; anything else is [`DraftAction::Text`].
/// Builtins win over catalog rows on a selector clash (they list
/// first in the menu). A catalog match needs the ready catalog and
/// an exact selector; unknown selectors stay [`DraftAction::Text`].
/// `/goal` with an empty objective is [`DraftAction::KeepDraft`];
/// `/model` (with any trailing text) opens the menu.
pub fn parse_slash_draft(draft: &str, skills: &[SkillInfo]) -> DraftAction {
    // Leading whitespace never owns the first token: the token after
    // it does, so the rest slices from the trimmed draft.
    let trimmed = draft.trim_start();
    let mut tokens = trimmed.split_whitespace();
    let Some(first) = tokens.next() else {
        return DraftAction::Text;
    };
    let Some(selector) = first.strip_prefix('/') else {
        return DraftAction::Text;
    };
    if selector.is_empty() {
        return DraftAction::Text;
    }
    // The rest of the draft after the first token, trimmed; the
    // skill arguments and the goal objective both read from here.
    let rest = trimmed[first.len()..].trim();
    if let Some(builtin) = BuiltinCommand::parse(selector) {
        return match builtin {
            BuiltinCommand::Model => DraftAction::OpenModelMenu,
            BuiltinCommand::Compact => DraftAction::Compact,
            BuiltinCommand::Goal => {
                if rest.is_empty() {
                    DraftAction::KeepDraft
                } else {
                    DraftAction::SetGoal(rest.to_string())
                }
            }
        };
    }
    if let Some(skill) = skills.iter().find(|skill| skill.selector == selector) {
        return DraftAction::Skill(SkillInvocation {
            selector: skill.selector.clone(),
            source: (!skill.source.trim().is_empty()).then(|| skill.source.clone()),
            arguments: rest.to_string(),
        });
    }
    DraftAction::Text
}

/// Read one flat numeric field from raw JSON text.
///
/// Best-effort scan for a crate with no JSON dependency: finds the
/// first `"key": <digits>` pair anywhere in the text. A longer key
/// containing this one (for example `totalTokens` for `total`) never
/// matches, because the closing quote must follow the key.
fn usage_number(raw: &str, key: &str) -> Option<u64> {
    const WS: [char; 4] = [' ', '\t', '\n', '\r'];
    let bytes = raw.as_bytes();
    let key = key.as_bytes();
    let mut i = 0;
    while i + key.len() + 2 <= bytes.len() {
        if bytes[i] == b'"'
            && &bytes[i + 1..i + 1 + key.len()] == key
            && bytes[i + 1 + key.len()] == b'"'
        {
            let mut j = i + 1 + key.len() + 1;
            while j < bytes.len() && WS.contains(&(bytes[j] as char)) {
                j += 1;
            }
            if bytes.get(j) == Some(&b':') {
                j += 1;
                while j < bytes.len() && WS.contains(&(bytes[j] as char)) {
                    j += 1;
                }
                let start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if start < j
                    && let Ok(found) = raw[start..j].parse()
                {
                    return Some(found);
                }
            }
        }
        i += 1;
    }
    None
}

/// First hit across several candidate keys, in order.
fn usage_first(raw: &str, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| usage_number(raw, key))
}

/// Renderable token-usage snapshot parsed from a [`StreamEvent::Usage`]
/// payload.
///
/// The host defines the payload shape, so [`UsageSnapshot::parse`] reads
/// cumulative counts from known flat fields and returns `None` when the
/// text is not a JSON object or carries no token count. Unknown fields
/// stay ignored, never parse failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSnapshot {
    /// Cumulative tokens used.
    pub used: u64,
    /// Context window size, when the payload states one.
    pub window: Option<u64>,
    /// One-line human detail, e.g. `57064 prompt + 16 output`.
    pub detail: String,
}

impl UsageSnapshot {
    /// Parse the raw `usage_json` of a [`StreamEvent::Usage`].
    ///
    /// Pure constructor: no I/O, no panics. Returns `None` when the
    /// text is not a JSON object or carries no token count.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if !(trimmed.starts_with('{') && trimmed.ends_with('}')) {
            return None;
        }
        let total = usage_first(raw, &["total", "totalTokens", "total_tokens"]);
        let input = usage_first(
            raw,
            &[
                "prompt",
                "promptTokens",
                "prompt_tokens",
                "inputTokens",
                "input_tokens",
            ],
        );
        let output = usage_first(
            raw,
            &[
                "output",
                "outputTokens",
                "output_tokens",
                "completionTokens",
                "completion_tokens",
            ],
        );
        let used = match total {
            Some(total) => total,
            None => match (input, output) {
                (Some(input), Some(output)) => input.checked_add(output)?,
                (Some(input), None) => input,
                (None, Some(output)) => output,
                (None, None) => return None,
            },
        };
        let window = usage_first(
            raw,
            &[
                "contextLimit",
                "context_limit",
                "contextWindow",
                "context_window",
                "limit",
            ],
        );
        let detail = match (input, output) {
            (Some(input), Some(output)) => format!("{input} prompt + {output} output"),
            _ => format!("{used} tokens"),
        };
        Some(Self {
            used,
            window,
            detail,
        })
    }
}

/// How to place a spawned session's working directory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WorktreeChoice {
    /// Spawn directly in the given workspace.
    #[default]
    Off,
    /// Create a worktree for the session, optionally from a base.
    Create {
        /// Base to create the worktree from, when set.
        base: Option<String>,
    },
    /// Use an existing worktree at this path.
    Existing {
        /// Path of the existing worktree.
        path: String,
    },
}

/// Extra choices for a session spawn.
///
/// The default omits every optional key, matching what the shell sent
/// before these choices existed: in-place worktree, server model and
/// effort, server permission posture.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpawnChoices {
    /// How to place the session's working directory.
    pub worktree: WorktreeChoice,
    /// Which model the session runs on.
    pub model: ModelChoice,
    /// Reasoning effort for the session.
    pub effort: EffortChoice,
    /// Which engine's permission scheme the session uses.
    pub permission: PermissionMode,
}

/// Which command a [`EngineEvent::CommandFailed`] failure belongs to.
///
/// Named so the shell consumes only the state that command owned: a
/// steer failure must not end a running turn, and a models failure
/// must not restore a spawn draft. Session-carrying variants name
/// the session the command addressed, so one session's failure never
/// releases another session's gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailedCommand {
    /// Spawning a new session failed.
    Spawn,
    /// Sending a follow-up turn to this session failed.
    Turn(SessionId),
    /// Resuming this session failed.
    Resume(SessionId),
    /// Steering the running turn of this session failed.
    Steer(SessionId),
    /// Cancelling the running turn of this session failed.
    Cancel(SessionId),
    /// Listing the model catalog failed.
    Models,
    /// Listing the skill catalog of this session failed.
    Skills(SessionId),
    /// Compacting this session failed.
    Compact(SessionId),
    /// Setting this session's goal failed.
    Goal(SessionId),
    /// Fetching this session's stored history failed.
    History(SessionId),
    /// Fetching pending approvals/questions failed.
    ///
    /// The fetch is global (`muse_pending` takes no session), so
    /// no session rides along.
    PendingFetch,
    /// Deciding this session's pending approval failed.
    Decide(SessionId, String),
    /// Answering this session's pending question failed.
    Answer(SessionId, String),
}

/// Everything an engine reports to the presentation layer.
///
/// `PartialEq` without `Eq`: [`StreamEvent`] (carried by
/// [`EngineEvent::Stream`]) holds a float goal percent, which has no
/// total equality.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// The engine connection state changed.
    ConnectionChanged(Connection),
    /// A session-list refresh succeeded.
    ///
    /// Grouping onto projects is the shell's job via
    /// [`crate::assign_projects`].
    SessionsLoaded {
        /// Sessions for the left panel, in display order.
        sessions: Vec<SessionSummary>,
        /// Cumulative usage per session, parsed from each row's
        /// `token_usage`; rows without usable token usage carry no
        /// entry, and the shell keeps its stored snapshot for those.
        usage: Vec<(SessionId, UsageSnapshot)>,
    },
    /// A session-list refresh failed while the connection stays up.
    ///
    /// Carries the boundary-composed message.
    SessionsLoadFailed(String),
    /// A model-catalog refresh succeeded.
    ///
    /// Selection stays the shell's job; ids go back as `model`.
    ModelsLoaded {
        /// Catalog rows in the host's order.
        models: Vec<ModelInfo>,
    },
    /// A skill-catalog refresh succeeded.
    ///
    /// Selection stays the shell's job; selectors go back as the
    /// skill part's `selector`.
    SkillsLoaded {
        /// Session the catalog belongs to.
        session: SessionId,
        /// Catalog rows in the host's order.
        skills: Vec<SkillInfo>,
    },
    /// A session spawn succeeded; the first turn streams next, when sent.
    SessionSpawned {
        /// Spawned session.
        session: SessionId,
        /// First turn, when the server returned one.
        turn: Option<TurnId>,
    },
    /// A follow-up turn was accepted; its events stream next.
    TurnAccepted {
        /// Session the turn belongs to.
        session: SessionId,
        /// Accepted turn, when the server returned one.
        turn: Option<TurnId>,
    },
    /// A session resume succeeded; its stream refreshes next.
    SessionResumed {
        /// Resumed session.
        session: SessionId,
    },
    /// A stored-history fetch succeeded; the shell seeds the
    /// session's transcript from these items in host order.
    HistoryLoaded {
        /// Session the history belongs to.
        session: SessionId,
        /// Stored items, oldest first; system items ride along and
        /// the shell drops them while seeding.
        items: Vec<StreamItem>,
    },
    /// A command failed while the connection stays up.
    ///
    /// Carries the boundary-composed or server-authored friendly message,
    /// mirroring [`EngineEvent::SessionsLoadFailed`], tagged with the
    /// command that failed so the shell consumes only that command's
    /// state.
    CommandFailed {
        /// The command that failed.
        command: FailedCommand,
        /// The friendly failure message.
        message: String,
    },
    /// A pending fetch succeeded; this is the source of truth for
    /// approval/question cards, replacing any hint-only state.
    PendingLoaded {
        /// All pending requests across sessions, in server order.
        requests: Vec<PendingRequest>,
    },
    /// One pending request settled.
    PendingSettled {
        /// Session the request belonged to.
        session: SessionId,
        /// Opaque request handle that settled.
        request_id: String,
        /// How it settled.
        resolution: PendingResolution,
    },
    /// One stream event arrived.
    Stream(StreamEvent),
    /// Partial-success diagnostic: something was skipped or degraded.
    ///
    /// For example a session row the mapper could not translate; the
    /// shell shows it rather than hiding the gap.
    Diagnostic(String),
}

/// The engine seam (Muse MCP implementation in the data crate; BYOK engines later).
///
/// Command submission is non-blocking and results arrive asynchronously as
/// [`EngineEvent`] values through the presentation layer's subscription;
/// commands submitted while disconnected may be dropped, and the shell
/// re-requests on [`Connection::Connected`].
pub trait SessionEngine: Send + Sync {
    /// Request a session-list refresh; results arrive as [`EngineEvent`] values.
    fn list_sessions(&self);
    /// Spawn a session on `workspace` with `input` as its first turn.
    ///
    /// Non-blocking; success arrives as
    /// [`EngineEvent::SessionSpawned`], failure as
    /// [`EngineEvent::CommandFailed`]. Commands submitted while
    /// disconnected fail fast with [`EngineEvent::CommandFailed`]
    /// rather than queueing.
    fn spawn_session(&self, workspace: String, input: TurnInput, choices: SpawnChoices);
    /// Send a follow-up turn to `session`.
    ///
    /// Non-blocking; success arrives as [`EngineEvent::TurnAccepted`],
    /// failure as [`EngineEvent::CommandFailed`]. Commands submitted
    /// while disconnected fail fast with [`EngineEvent::CommandFailed`]
    /// rather than queueing.
    fn send_turn(&self, session: SessionId, input: TurnInput);
    /// Resume `session` on the server.
    ///
    /// Non-blocking; success arrives as
    /// [`EngineEvent::SessionResumed`], failure as
    /// [`EngineEvent::CommandFailed`]. Commands submitted while
    /// disconnected fail fast with [`EngineEvent::CommandFailed`]
    /// rather than queueing.
    fn resume_session(&self, session: SessionId);
    /// Fetch `session`'s stored transcript history.
    ///
    /// Non-blocking; success arrives as
    /// [`EngineEvent::HistoryLoaded`], failure as
    /// [`EngineEvent::CommandFailed`] with
    /// [`FailedCommand::History`]. Engines without history support
    /// keep the default, which drops the call.
    fn load_history(&self, _session: SessionId) {}
    /// Inject `prompt` into the running `turn` of `session`.
    ///
    /// Non-blocking; failure (idle session, unknown turn) arrives as
    /// [`EngineEvent::CommandFailed`] rather than queueing. Engines
    /// without steering keep the default, which drops the call.
    fn steer_session(&self, session: SessionId, prompt: String, turn: TurnId) {
        let _ = (session, prompt, turn);
    }
    /// Cancel the running turn of `session`, or `turn` when given.
    ///
    /// Non-blocking; failure arrives as [`EngineEvent::CommandFailed`].
    /// Engines without cancellation keep the default, which drops the
    /// call.
    fn cancel_turn(&self, session: SessionId, turn: Option<TurnId>) {
        let _ = (session, turn);
    }
    /// Request a model-catalog refresh; results arrive as
    /// [`EngineEvent::ModelsLoaded`], failures as
    /// [`EngineEvent::CommandFailed`]. The server needs a host to list
    /// from, so the caller passes a workspace, a session id, or both;
    /// a bare request fails while no host is live.
    fn list_models(&self, _workspace: Option<String>, _session: Option<SessionId>) {}
    /// Request a skill-catalog refresh for `session`; results arrive
    /// as [`EngineEvent::SkillsLoaded`], failures as
    /// [`EngineEvent::CommandFailed`] with
    /// [`FailedCommand::Skills`]. Engines without skill support keep
    /// the default, which drops the call.
    fn list_skills(&self, _session: SessionId) {}
    /// Compact `session`'s context through the server's `muse_raw`
    /// (`session/compact`, admitted asynchronously).
    ///
    /// Non-blocking; success emits nothing (the compaction's terminal
    /// view event carries the outcome), failure arrives as
    /// [`EngineEvent::CommandFailed`] with
    /// [`FailedCommand::Compact`]. Engines without compaction keep
    /// the default, which drops the call.
    fn compact_session(&self, _session: SessionId) {}
    /// Set `session`'s goal objective through the server's `muse_raw`
    /// (`goal/set`).
    ///
    /// Non-blocking; success emits nothing, failure arrives as
    /// [`EngineEvent::CommandFailed`] with [`FailedCommand::Goal`].
    /// Engines without goals keep the default, which drops the call.
    fn set_goal(&self, _session: SessionId, _objective: String) {}
    /// Fetch pending approvals/questions; results arrive as
    /// [`EngineEvent::PendingLoaded`], failures as
    /// [`EngineEvent::CommandFailed`] with
    /// [`FailedCommand::PendingFetch`]. Engines without pending
    /// support keep the default, which drops the call.
    fn fetch_pending(&self) {}
    /// Decide `request_id` on `session` (approve or deny, with an
    /// optional reason sent only when the choice accepts feedback
    /// at the data boundary; the caller passes `actor: "user"` on
    /// the wire there). Settlement arrives as
    /// [`EngineEvent::PendingSettled`], failure as
    /// [`EngineEvent::CommandFailed`] with [`FailedCommand::Decide`].
    /// Engines without approvals keep the default, which drops it.
    fn decide(
        &self,
        session: SessionId,
        request_id: String,
        decision: ApprovalDecision,
        reason: Option<String>,
    ) {
        let _ = (session, request_id, decision, reason);
    }
    /// Answer `request_id` on `session` with `answer` (the caller
    /// passes `actor: "user"` on the wire at the data boundary).
    /// Settlement arrives as [`EngineEvent::PendingSettled`],
    /// failure as [`EngineEvent::CommandFailed`] with
    /// [`FailedCommand::Answer`]. Engines without questions keep
    /// the default, which drops the call.
    fn answer(&self, session: SessionId, request_id: String, answer: PendingAnswer) {
        let _ = (session, request_id, answer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_defaults_to_disconnected() {
        assert_eq!(Connection::default(), Connection::Disconnected);
    }

    #[test]
    fn turn_id_equality_is_by_whole_handle() {
        assert_eq!(TurnId::new("turn-1"), TurnId::new("turn-1"));
        assert_ne!(TurnId::new("turn-1"), TurnId::new("turn-2"));
    }

    #[test]
    fn id_handles_borrow_back() {
        assert_eq!(TurnId::new("turn-1").as_str(), "turn-1");
        assert_eq!(SessionId::new("session-1").as_str(), "session-1");
    }

    #[test]
    fn stream_item_id_covers_every_variant() {
        let items = vec![
            (
                StreamItem::UserMessage {
                    item_id: String::from("u"),
                    text: String::new(),
                },
                "u",
            ),
            (
                StreamItem::AgentMessage {
                    item_id: String::from("a"),
                    text: String::new(),
                    phase: None,
                },
                "a",
            ),
            (
                StreamItem::Reasoning {
                    item_id: String::from("r"),
                    summary: Vec::new(),
                },
                "r",
            ),
            (
                StreamItem::ToolCall {
                    item_id: String::from("t"),
                    tool: String::new(),
                    headline: None,
                    args_json: String::new(),
                    todos: None,
                    status: ItemStatus::InProgress,
                    output: None,
                    failure: None,
                },
                "t",
            ),
            (
                StreamItem::System {
                    item_id: String::from("s"),
                    kind: String::from("reminderChild"),
                },
                "s",
            ),
            (
                StreamItem::Compaction {
                    item_id: String::from("c"),
                    outcome: Some(String::from("compacted")),
                    reason: None,
                    tokens_before: Some(100),
                    tokens_after: Some(10),
                },
                "c",
            ),
            (
                StreamItem::Unknown {
                    item_id: String::from("x"),
                    kind: String::from("from-the-future"),
                },
                "x",
            ),
        ];
        for (item, want) in items {
            assert_eq!(item.item_id(), want);
        }
    }

    #[test]
    fn goal_status_parses_known_leniently_and_keeps_unknown() {
        assert_eq!(GoalStatus::parse("active"), GoalStatus::Active);
        assert_eq!(GoalStatus::parse(" Paused "), GoalStatus::Paused);
        assert_eq!(GoalStatus::parse("COMPLETED"), GoalStatus::Completed);
        assert_eq!(
            GoalStatus::parse("archived"),
            GoalStatus::Other(String::from("archived"))
        );
        assert_eq!(GoalStatus::parse(""), GoalStatus::Other(String::from("")));
    }

    #[test]
    fn goal_clamped_percent_bounds_the_range() {
        let goal = |percent_complete: f64| GoalInfo {
            objective: String::from("ship it"),
            percent_complete,
            status: GoalStatus::Active,
            current_work: None,
            next_work: None,
        };
        assert_eq!(goal(42.0).clamped_percent(), 42.0);
        assert_eq!(goal(150.0).clamped_percent(), 100.0);
        assert_eq!(goal(-3.0).clamped_percent(), 0.0);
        assert_eq!(goal(f64::NAN).clamped_percent(), 0.0);
        // The carried value stays verbatim past 100.
        assert_eq!(goal(150.0).percent_complete, 150.0);
    }

    #[test]
    fn spawn_choices_default_omits_everything() {
        let choices = SpawnChoices::default();
        assert_eq!(choices.worktree, WorktreeChoice::Off);
        assert_eq!(choices.model, ModelChoice::Default);
        assert_eq!(choices.model.id(), None);
        assert_eq!(choices.effort, EffortChoice::Default);
        assert_eq!(choices.effort.wire_value(), None);
        assert_eq!(
            choices.permission,
            PermissionMode::Muse(MusePermission::Yolo)
        );
    }

    #[test]
    fn permission_postures_have_distinct_labels_and_descriptions() {
        let mut labels = Vec::new();
        let mut descriptions = Vec::new();
        for posture in MusePermission::all() {
            assert!(!posture.label().is_empty());
            assert!(!posture.description().is_empty());
            labels.push(posture.label());
            descriptions.push(posture.description());
        }
        labels.sort_unstable();
        labels.dedup();
        descriptions.sort_unstable();
        descriptions.dedup();
        assert_eq!(labels.len(), 3);
        assert_eq!(descriptions.len(), 3);
    }

    #[test]
    fn permission_wire_values_round_trip() {
        for posture in MusePermission::all() {
            assert_eq!(MusePermission::parse(posture.wire_value()), Some(posture));
        }
        assert_eq!(MusePermission::parse("yolo"), Some(MusePermission::Yolo));
        assert_eq!(
            MusePermission::parse("on-request"),
            Some(MusePermission::OnRequest)
        );
        assert_eq!(MusePermission::parse("judge"), Some(MusePermission::Judge));
        assert_eq!(MusePermission::parse("allow-all"), None);
        assert_eq!(MusePermission::parse(""), None);
    }

    #[test]
    fn permission_mode_delegates_to_engine_posture() {
        let mode = PermissionMode::Muse(MusePermission::OnRequest);
        assert_eq!(mode.label(), "On request");
        assert_eq!(mode.description(), "Asks you before running each tool.");
        assert_eq!(
            PermissionMode::default(),
            PermissionMode::Muse(MusePermission::Yolo)
        );
    }

    #[test]
    fn effort_wire_values_round_trip() {
        assert_eq!(EffortChoice::all().len(), 9);
        for effort in EffortChoice::all() {
            match effort.wire_value() {
                Some(wire) => assert_eq!(EffortChoice::parse(wire), Some(effort)),
                None => assert_eq!(effort, EffortChoice::Default),
            }
        }
        assert_eq!(EffortChoice::parse("default"), Some(EffortChoice::Default));
        assert_eq!(EffortChoice::parse("xhigh"), Some(EffortChoice::XHigh));
        assert_eq!(EffortChoice::parse("turbo"), None);
        assert_eq!(EffortChoice::parse(""), None);
    }

    #[test]
    fn model_choice_ids_omit_default_and_blank() {
        assert_eq!(ModelChoice::Default.id(), None);
        assert_eq!(ModelChoice::Default.display(), "Default");
        assert_eq!(
            ModelChoice::Named(String::from("muse-spark")).id(),
            Some("muse-spark")
        );
        assert_eq!(
            ModelChoice::Named(String::from("muse-spark")).display(),
            "muse-spark"
        );
        assert_eq!(ModelChoice::Named(String::new()).id(), None);
        assert_eq!(ModelChoice::Named(String::from("  ")).id(), None);
    }

    #[test]
    fn model_info_label_falls_back_to_id() {
        let labeled = ModelInfo {
            id: String::from("muse-spark"),
            display_label: String::from("Muse Spark"),
            is_default: true,
            is_active: false,
        };
        assert_eq!(labeled.label(), "Muse Spark");
        let bare = ModelInfo {
            id: String::from("muse-spark"),
            display_label: String::new(),
            is_default: false,
            is_active: true,
        };
        assert_eq!(bare.label(), "muse-spark");
    }

    #[test]
    fn usage_parse_reads_full_payload() {
        let got =
            UsageSnapshot::parse(r#"{"prompt":57064,"output":16,"total":57080,"limit":200000}"#)
                .expect("full payload parses");
        assert_eq!(got.used, 57080);
        assert_eq!(got.window, Some(200000));
        assert_eq!(got.detail, "57064 prompt + 16 output");
    }

    #[test]
    fn usage_parse_prefers_total_and_reads_camel_case() {
        let got = UsageSnapshot::parse(
            r#"{"promptTokens": 100, "outputTokens": 50, "totalTokens": 999, "contextLimit": 1000}"#,
        )
        .expect("camel-case payload parses");
        assert_eq!(got.used, 999);
        assert_eq!(got.window, Some(1000));
        assert_eq!(got.detail, "100 prompt + 50 output");
    }

    #[test]
    fn usage_parse_sums_sides_without_total() {
        let got = UsageSnapshot::parse(r#"{"prompt_tokens":10,"output_tokens":5}"#)
            .expect("sides without total parse");
        assert_eq!(got.used, 15);
        assert_eq!(got.window, None);
        assert_eq!(got.detail, "10 prompt + 5 output");
    }

    #[test]
    fn usage_parse_total_only_has_plain_detail() {
        let got = UsageSnapshot::parse(r#"{"total": 42}"#).expect("bare total parses");
        assert_eq!(got.used, 42);
        assert_eq!(got.window, None);
        assert_eq!(got.detail, "42 tokens");
    }

    #[test]
    fn usage_parse_ignores_longer_keys_and_rejects_junk() {
        // `totalTokens` must not satisfy a `total` lookup by prefix.
        let got = UsageSnapshot::parse(r#"{"totalTokens": 7}"#).expect("parses");
        assert_eq!(got.used, 7);
        for junk in [
            "",
            "nope",
            "[1, 2]",
            "{}",
            "{\"foo\": 1}",
            "{\"total\": \"lots\"}",
            "{\"total\":",
        ] {
            assert_eq!(UsageSnapshot::parse(junk), None, "rejects {junk:?}");
        }
    }

    #[test]
    fn models_loaded_event_carries_catalog() {
        let event = EngineEvent::ModelsLoaded {
            models: vec![ModelInfo {
                id: String::from("muse-spark"),
                display_label: String::from("Muse Spark"),
                is_default: true,
                is_active: true,
            }],
        };
        match event {
            EngineEvent::ModelsLoaded { models } => {
                assert_eq!(models.len(), 1);
                assert!(models[0].is_default && models[0].is_active);
            }
            _ => panic!("expected ModelsLoaded"),
        }
    }

    /// Old implementors keep compiling: new seam methods have defaults.
    struct LegacyEngine;

    impl SessionEngine for LegacyEngine {
        fn list_sessions(&self) {}
        fn spawn_session(&self, _workspace: String, _input: TurnInput, _choices: SpawnChoices) {}
        fn send_turn(&self, _session: SessionId, _input: TurnInput) {}
        fn resume_session(&self, _session: SessionId) {}
    }

    #[test]
    fn new_seam_methods_default_to_dropping() {
        let engine = LegacyEngine;
        engine.steer_session(
            SessionId::new("s"),
            String::from("faster"),
            TurnId::new("t"),
        );
        engine.cancel_turn(SessionId::new("s"), None);
        engine.cancel_turn(SessionId::new("s"), Some(TurnId::new("t")));
        engine.list_models(None, None);
        engine.list_skills(SessionId::new("s"));
        engine.fetch_pending();
        engine.decide(
            SessionId::new("s"),
            String::from("r1"),
            ApprovalDecision::Approve,
            None,
        );
        engine.answer(
            SessionId::new("s"),
            String::from("r1"),
            PendingAnswer::single("q1", AnswerValue::Skipped),
        );
    }

    #[test]
    fn turn_input_display_text_echoes_text_and_skill() {
        assert_eq!(
            TurnInput::Text(String::from("hello")).display_text(),
            "hello"
        );
        let bare = TurnInput::Skill(SkillInvocation {
            selector: String::from("review"),
            source: None,
            arguments: String::new(),
        });
        assert_eq!(bare.display_text(), "/review");
        let blank_args = TurnInput::Skill(SkillInvocation {
            selector: String::from("review"),
            source: Some(String::from("user")),
            arguments: String::from("   "),
        });
        assert_eq!(blank_args.display_text(), "/review");
        let with_args = TurnInput::Skill(SkillInvocation {
            selector: String::from("review"),
            source: Some(String::from("user")),
            arguments: String::from("pr 42"),
        });
        assert_eq!(with_args.display_text(), "/review pr 42");
    }

    #[test]
    fn skill_label_falls_back_to_selector() {
        let named = SkillInfo {
            selector: String::from("review"),
            display_name: String::from("Review"),
            description: String::new(),
            source: String::from("user"),
            plugin_id: None,
            argument_hint: None,
        };
        assert_eq!(named.label(), "Review");
        let blank = SkillInfo {
            display_name: String::from("  "),
            ..named.clone()
        };
        assert_eq!(blank.label(), "review");
        assert_eq!(
            named.clone(),
            SkillInfo {
                selector: String::from("review"),
                ..named
            }
        );
    }

    #[test]
    fn skills_loaded_event_carries_session_and_catalog() {
        let event = EngineEvent::SkillsLoaded {
            session: SessionId::new("s-1"),
            skills: vec![SkillInfo {
                selector: String::from("review"),
                display_name: String::from("Review"),
                description: String::from("Reviews code"),
                source: String::from("user"),
                plugin_id: None,
                argument_hint: Some(String::from("<pr>")),
            }],
        };
        match event {
            EngineEvent::SkillsLoaded { session, skills } => {
                assert_eq!(session, SessionId::new("s-1"));
                assert_eq!(skills.len(), 1);
                assert_eq!(skills[0].label(), "Review");
            }
            _ => panic!("expected SkillsLoaded"),
        }
    }

    fn sample_question(multi: bool) -> Question {
        Question {
            id: String::from("q1"),
            prompt: String::from("Pick one"),
            options: vec![
                QuestionOption {
                    label: String::from("Red"),
                    description: Some(String::from("Warm")),
                },
                QuestionOption {
                    label: String::from("Blue"),
                    description: None,
                },
                QuestionOption {
                    label: String::from("None"),
                    description: None,
                },
            ],
            multi,
        }
    }

    #[test]
    fn classify_exact_label_is_selected_label() {
        let question = sample_question(false);
        assert_eq!(
            question.classify_text("Red"),
            AnswerValue::Label(String::from("Red"))
        );
        // A real option literally labelled None is a plain label:
        // no free-text box is implied.
        assert_eq!(
            question.classify_text("None"),
            AnswerValue::Label(String::from("None"))
        );
        assert_eq!(
            AnswerValue::Label(String::from("Red")).wire_kind(),
            WireAnswerKind::SelectedLabel
        );
    }

    #[test]
    fn classify_unknown_text_is_free_text() {
        let question = sample_question(false);
        assert_eq!(
            question.classify_text("Purple, please"),
            AnswerValue::FreeText(String::from("Purple, please"))
        );
        assert_eq!(
            AnswerValue::free_text("hi").wire_kind(),
            WireAnswerKind::FreeText
        );
    }

    #[test]
    fn classify_multi_splits_on_commas_and_drops_unknown() {
        let question = sample_question(true);
        assert_eq!(
            question.classify_text("Red, Blue, Purple"),
            AnswerValue::Labels(vec![String::from("Red"), String::from("Blue")])
        );
        assert_eq!(
            question.classify_text("Red"),
            AnswerValue::Labels(vec![String::from("Red")])
        );
        // No label survives the filter, so the whole text is free text.
        assert_eq!(
            question.classify_text("Purple"),
            AnswerValue::FreeText(String::from("Purple"))
        );
    }

    #[test]
    fn free_text_is_capped_at_500_chars() {
        let long = "x".repeat(600);
        assert_eq!(truncate_free_text(&long).chars().count(), FREE_TEXT_LIMIT);
        match AnswerValue::free_text(&long) {
            AnswerValue::FreeText(text) => assert_eq!(text.chars().count(), 500),
            other => panic!("expected FreeText, got {other:?}"),
        }
        // Multi-byte text truncates on a char boundary.
        let wide = "é".repeat(600);
        assert_eq!(truncate_free_text(&wide).chars().count(), 500);
    }

    #[test]
    fn skipped_answer_has_no_wire_field() {
        assert!(AnswerValue::Skipped.is_skipped());
        assert_eq!(AnswerValue::Skipped.wire_kind(), WireAnswerKind::Skipped);
        assert_eq!(AnswerValue::Skipped.summary(), "Skipped");
    }

    #[test]
    fn question_label_helpers() {
        let question = sample_question(false);
        assert_eq!(question.option_labels(), vec!["Red", "Blue", "None"]);
        assert!(question.has_option("Blue"));
        assert!(!question.has_option("Purple"));
        assert!(question.options[0].has_description());
        assert!(!question.options[1].has_description());
    }

    #[test]
    fn pending_answer_completeness_and_summary() {
        let request = QuestionRequest {
            questions: vec![
                sample_question(false),
                Question {
                    id: String::from("q2"),
                    ..sample_question(false)
                },
            ],
        };
        let partial = PendingAnswer::single("q1", AnswerValue::Label(String::from("Red")));
        assert!(!partial.is_complete_for(&request));
        let full = PendingAnswer::new(vec![
            (String::from("q1"), AnswerValue::Label(String::from("Red"))),
            (String::from("q2"), AnswerValue::Skipped),
        ]);
        assert!(full.is_complete_for(&request));
        assert_eq!(full.summary(), "Red; Skipped");
        assert_eq!(
            full.get("q1"),
            Some(&AnswerValue::Label(String::from("Red")))
        );
        assert_eq!(request.question("q2").map(|q| q.id.as_str()), Some("q2"));
    }

    #[test]
    fn pending_request_accessors_cover_both_kinds() {
        let approval = PendingRequest::Approval {
            session: SessionId::new("s1"),
            request_id: String::from("r1"),
            approval: ApprovalRequest {
                tool: String::from("Bash"),
                subject_line: String::from("run ls"),
                choices: vec![ApprovalChoiceSummary {
                    label: String::new(),
                    decision: String::from("approved"),
                    scope: String::from("once"),
                    accepts_feedback: false,
                }],
            },
        };
        assert_eq!(approval.session().as_str(), "s1");
        assert_eq!(approval.request_id(), "r1");
        assert_eq!(approval.kind(), PendingKind::Approval);
        assert_eq!(approval.questions(), None);
        let question = PendingRequest::Question {
            session: SessionId::new("s2"),
            request_id: String::from("r2"),
            question: QuestionRequest {
                questions: vec![sample_question(false)],
            },
        };
        assert_eq!(question.kind(), PendingKind::UserInput);
        assert_eq!(question.questions().map(<[Question]>::len), Some(1));
    }

    #[test]
    fn approval_choice_display_label_falls_back_to_decision() {
        let blank = ApprovalChoiceSummary {
            label: String::from("  "),
            decision: String::from("approved"),
            scope: String::from("once"),
            accepts_feedback: false,
        };
        assert_eq!(blank.display_label(), "approved");
        let approval = ApprovalRequest {
            tool: String::from("Bash"),
            subject_line: String::from("run ls"),
            choices: vec![blank],
        };
        assert_eq!(approval.choice_labels(), vec!["approved"]);
    }

    #[test]
    fn resolution_labels_and_details() {
        assert_eq!(PendingResolution::Approved.label(), "Approved");
        assert_eq!(
            PendingResolution::Denied(Some(String::from("no"))).detail(),
            Some("no")
        );
        assert_eq!(PendingResolution::Denied(None).detail(), None);
        assert_eq!(
            PendingResolution::Answered(String::from("Red")).detail(),
            Some("Red")
        );
        assert_eq!(PendingResolution::Expired.label(), "Expired");
        assert_eq!(PendingResolution::Cancelled.label(), "Cancelled");
        assert_eq!(ApprovalDecision::Approve.wire_value(), "approve");
        assert_eq!(ApprovalDecision::Deny.wire_value(), "deny");
    }

    #[test]
    fn pending_events_carry_requests_and_settlement() {
        let loaded = EngineEvent::PendingLoaded {
            requests: vec![PendingRequest::Approval {
                session: SessionId::new("s1"),
                request_id: String::from("r1"),
                approval: ApprovalRequest {
                    tool: String::from("Bash"),
                    subject_line: String::from("run ls"),
                    choices: Vec::new(),
                },
            }],
        };
        match loaded {
            EngineEvent::PendingLoaded { requests } => assert_eq!(requests.len(), 1),
            _ => panic!("expected PendingLoaded"),
        }
        let settled = EngineEvent::PendingSettled {
            session: SessionId::new("s1"),
            request_id: String::from("r1"),
            resolution: PendingResolution::Expired,
        };
        match settled {
            EngineEvent::PendingSettled { resolution, .. } => {
                assert_eq!(resolution, PendingResolution::Expired)
            }
            _ => panic!("expected PendingSettled"),
        }
        // FailedCommand pending variants carry session + request where useful.
        assert_eq!(
            FailedCommand::Decide(SessionId::new("s1"), String::from("r1")),
            FailedCommand::Decide(SessionId::new("s1"), String::from("r1"))
        );
        assert_ne!(
            FailedCommand::Answer(SessionId::new("s1"), String::from("r1")),
            FailedCommand::PendingFetch
        );
    }

    /// One catalog skill for the slash-parse tests.
    fn parse_skills() -> Vec<SkillInfo> {
        vec![
            SkillInfo {
                selector: String::from("review"),
                display_name: String::from("Review"),
                description: String::from("Review code"),
                source: String::from("user"),
                plugin_id: None,
                argument_hint: None,
            },
            SkillInfo {
                selector: String::from("empty-source"),
                display_name: String::new(),
                description: String::new(),
                source: String::new(),
                plugin_id: None,
                argument_hint: None,
            },
        ]
    }

    #[test]
    fn slash_parse_skill_with_args_carries_source_and_arguments() {
        let got = parse_slash_draft("/review pr 42", &parse_skills());
        assert_eq!(
            got,
            DraftAction::Skill(SkillInvocation {
                selector: String::from("review"),
                source: Some(String::from("user")),
                arguments: String::from("pr 42"),
            })
        );
    }

    #[test]
    fn slash_parse_skill_without_args_has_empty_arguments() {
        let got = parse_slash_draft("/review", &parse_skills());
        assert_eq!(
            got,
            DraftAction::Skill(SkillInvocation {
                selector: String::from("review"),
                source: Some(String::from("user")),
                arguments: String::new(),
            })
        );
    }

    #[test]
    fn slash_parse_skill_with_empty_source_sends_no_source() {
        let got = parse_slash_draft("/empty-source x", &parse_skills());
        assert_eq!(
            got,
            DraftAction::Skill(SkillInvocation {
                selector: String::from("empty-source"),
                source: None,
                arguments: String::from("x"),
            })
        );
    }

    #[test]
    fn slash_parse_unknown_selector_is_text() {
        assert_eq!(
            parse_slash_draft("/nope fix it", &parse_skills()),
            DraftAction::Text
        );
    }

    #[test]
    fn slash_parse_plain_text_is_text() {
        assert_eq!(
            parse_slash_draft("hello there", &parse_skills()),
            DraftAction::Text
        );
        assert_eq!(parse_slash_draft("", &parse_skills()), DraftAction::Text);
        assert_eq!(parse_slash_draft("/", &parse_skills()), DraftAction::Text);
    }

    #[test]
    fn slash_parse_prefix_match_is_not_a_skill() {
        // Only an exact selector matches; `/rev` is plain text even
        // though `/review` exists.
        assert_eq!(
            parse_slash_draft("/rev", &parse_skills()),
            DraftAction::Text
        );
    }

    #[test]
    fn slash_parse_goal_with_objective_sets_it() {
        assert_eq!(
            parse_slash_draft("/goal ship the slice", &parse_skills()),
            DraftAction::SetGoal(String::from("ship the slice"))
        );
    }

    #[test]
    fn slash_parse_goal_without_objective_keeps_draft() {
        assert_eq!(
            parse_slash_draft("/goal", &parse_skills()),
            DraftAction::KeepDraft
        );
        assert_eq!(
            parse_slash_draft("/goal   ", &parse_skills()),
            DraftAction::KeepDraft
        );
    }

    #[test]
    fn slash_parse_compact_and_model() {
        assert_eq!(
            parse_slash_draft("/compact", &parse_skills()),
            DraftAction::Compact
        );
        assert_eq!(
            parse_slash_draft("/model", &parse_skills()),
            DraftAction::OpenModelMenu
        );
        assert_eq!(
            parse_slash_draft("/model muse-spark", &parse_skills()),
            DraftAction::OpenModelMenu
        );
    }

    #[test]
    fn slash_parse_builtin_wins_over_catalog_clash() {
        let mut skills = parse_skills();
        skills.push(SkillInfo {
            selector: String::from("model"),
            display_name: String::from("Model skill"),
            description: String::new(),
            source: String::from("user"),
            plugin_id: None,
            argument_hint: None,
        });
        assert_eq!(
            parse_slash_draft("/model", &skills),
            DraftAction::OpenModelMenu
        );
    }

    #[test]
    fn slash_parse_ignores_leading_whitespace() {
        let got = parse_slash_draft("  /review x", &parse_skills());
        assert_eq!(
            got,
            DraftAction::Skill(SkillInvocation {
                selector: String::from("review"),
                source: Some(String::from("user")),
                arguments: String::from("x"),
            })
        );
    }

    #[test]
    fn compact_and_goal_failures_carry_their_session() {
        assert_eq!(
            FailedCommand::Compact(SessionId::new("s1")),
            FailedCommand::Compact(SessionId::new("s1"))
        );
        assert_ne!(
            FailedCommand::Compact(SessionId::new("s1")),
            FailedCommand::Goal(SessionId::new("s1"))
        );
    }
}
