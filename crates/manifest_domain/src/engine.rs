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
    },
    /// A transcript item started; the full item object as raw JSON text.
    ItemStarted {
        /// Session the item belongs to.
        session: SessionId,
        /// Turn the item belongs to, when known.
        turn: Option<TurnId>,
        /// Cursor into the transcript view, when known.
        view_cursor: Option<String>,
        /// Full item object as raw JSON text, passed through uninterpreted in M2.
        item_json: String,
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
        /// Full item object as raw JSON text.
        item_json: String,
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
}
