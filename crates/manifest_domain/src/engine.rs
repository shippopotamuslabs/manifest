//! Engine seam types: connection, stream, and turn values the shell renders.
//!
//! The engine seam is the boundary between the presentation layer and the
//! agent backend (the Muse MCP implementation in the data crate; BYOK
//! engines later). These types carry everything an engine reports to the
//! presentation layer and perform no I/O.

use crate::fixture::{SessionId, SessionSummary};

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
            | Self::Unknown { item_id, .. } => item_id,
        }
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

/// One server stream event, translated to domain values.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// Everything an engine reports to the presentation layer.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Spawn a session on `workspace` with `prompt` as its first turn.
    ///
    /// Non-blocking; success arrives as
    /// [`EngineEvent::SessionSpawned`], failure as
    /// [`EngineEvent::CommandFailed`]. Commands submitted while
    /// disconnected fail fast with [`EngineEvent::CommandFailed`]
    /// rather than queueing.
    fn spawn_session(&self, workspace: String, prompt: String, choices: SpawnChoices);
    /// Send a follow-up turn to `session`.
    ///
    /// Non-blocking; success arrives as [`EngineEvent::TurnAccepted`],
    /// failure as [`EngineEvent::CommandFailed`]. Commands submitted
    /// while disconnected fail fast with [`EngineEvent::CommandFailed`]
    /// rather than queueing.
    fn send_turn(&self, session: SessionId, prompt: String);
    /// Resume `session` on the server.
    ///
    /// Non-blocking; success arrives as
    /// [`EngineEvent::SessionResumed`], failure as
    /// [`EngineEvent::CommandFailed`]. Commands submitted while
    /// disconnected fail fast with [`EngineEvent::CommandFailed`]
    /// rather than queueing.
    fn resume_session(&self, session: SessionId);
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
        fn spawn_session(&self, _workspace: String, _prompt: String, _choices: SpawnChoices) {}
        fn send_turn(&self, _session: SessionId, _prompt: String) {}
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
    }
}
