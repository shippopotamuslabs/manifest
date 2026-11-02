//! Engine seam types: connection, stream, and turn values the shell renders.
//!
//! The engine seam is the boundary between the presentation layer and the
//! agent backend (the Muse MCP implementation in the data crate; BYOK
//! engines later). These types carry everything an engine reports to the
//! presentation layer and perform no I/O.

use crate::fixture::{ProjectSummary, SessionId, SessionSummary};

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

/// Everything an engine reports to the presentation layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    /// The engine connection state changed.
    ConnectionChanged(Connection),
    /// A session-list refresh succeeded.
    SessionsLoaded {
        /// Projects for the left panel, in display order.
        projects: Vec<ProjectSummary>,
        /// Sessions for the left panel, in display order.
        sessions: Vec<SessionSummary>,
    },
    /// A session-list refresh failed while the connection stays up.
    ///
    /// Carries the boundary-composed message.
    SessionsLoadFailed(String),
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
    /// A spawn or send-turn command failed while the connection stays up.
    ///
    /// Carries the boundary-composed or server-authored friendly message,
    /// mirroring [`EngineEvent::SessionsLoadFailed`].
    CommandFailed(String),
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
    fn spawn_session(&self, workspace: String, prompt: String);
    /// Send a follow-up turn to `session`.
    ///
    /// Non-blocking; success arrives as [`EngineEvent::TurnAccepted`],
    /// failure as [`EngineEvent::CommandFailed`]. Commands submitted
    /// while disconnected fail fast with [`EngineEvent::CommandFailed`]
    /// rather than queueing.
    fn send_turn(&self, session: SessionId, prompt: String);
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
}
