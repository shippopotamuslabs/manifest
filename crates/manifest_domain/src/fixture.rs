//! Session, transcript, and todo domain values plus the fixture shell renders.

/// Opaque handle for one agent session.
///
/// The inner value is never inspected by callers; sessions compare and
/// match by the whole handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// Build a session id from its stored handle.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// The situation a session is in. One variant per real situation;
/// callers match exhaustively instead of combining flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    /// No work running; waiting for the user.
    Idle,
    /// The agent is working on a turn.
    Running,
    /// The agent paused for the user to approve or answer.
    AwaitingApproval,
}

/// One row in the session list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// Opaque session handle.
    pub id: SessionId,
    /// Short human title, e.g. "Refresh expired API tokens".
    pub title: String,
    /// What the session is doing now.
    pub status: SessionStatus,
}

/// One entry in a session transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptItem {
    /// Something the user wrote.
    UserTurn { text: String },
    /// Something the agent wrote.
    AssistantTurn { text: String },
    /// A tool result worth showing inline, e.g. a test run summary.
    ToolNote { text: String },
}

/// One checklist entry the agent works through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    /// Short human description of the step.
    pub label: String,
    /// Whether the step is finished.
    pub done: bool,
}

/// The whole fixture: sessions, one visible transcript, and todos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixture {
    /// Sessions for the left panel, in display order.
    pub sessions: Vec<SessionSummary>,
    /// Transcript of the selected session.
    pub transcript: Vec<TranscriptItem>,
    /// Checklist for the open drawer.
    pub todos: Vec<TodoItem>,
}

/// Deterministic fixture the shell boots from.
///
/// Pure constructor: every call returns the same value, so tests and
/// the shell always start from identical state.
pub fn fixture() -> Fixture {
    Fixture {
        sessions: vec![
            SessionSummary {
                id: SessionId::new("session-checkout-sync"),
                title: String::from("Fix checkout sync retries"),
                status: SessionStatus::Running,
            },
            SessionSummary {
                id: SessionId::new("session-settings-copy"),
                title: String::from("Review new settings screen copy"),
                status: SessionStatus::AwaitingApproval,
            },
            SessionSummary {
                id: SessionId::new("session-migration-cleanup"),
                title: String::from("Clean up old migration scripts"),
                status: SessionStatus::Idle,
            },
        ],
        transcript: vec![
            TranscriptItem::UserTurn {
                text: String::from(
                    "Why does the checkout sync fail when the token expires mid-run?",
                ),
            },
            TranscriptItem::AssistantTurn {
                text: String::from(
                    "The refresh only happens at startup. I will move it into \
                     the request path so an expired token retries once before it errors.",
                ),
            },
            TranscriptItem::ToolNote {
                text: String::from(
                    "Ran checkout sync suite: 14 passed, 2 failed on expired-token cases.",
                ),
            },
            TranscriptItem::AssistantTurn {
                text: String::from(
                    "Both failures match the expired-token path. I have a patch \
                     ready for your review.",
                ),
            },
            TranscriptItem::UserTurn {
                text: String::from("Go ahead and apply it, then rerun the suite."),
            },
        ],
        todos: vec![
            TodoItem {
                label: String::from("Reproduce the expired-token failure"),
                done: true,
            },
            TodoItem {
                label: String::from("Move token refresh into the request path"),
                done: false,
            },
            TodoItem {
                label: String::from("Apply the patch after approval"),
                done: false,
            },
            TodoItem {
                label: String::from("Rerun the checkout sync suite"),
                done: false,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_deterministic() {
        assert_eq!(fixture(), fixture());
    }

    #[test]
    fn fixture_has_sessions_with_work_to_show() {
        let got = fixture();
        assert!(got.sessions.len() >= 3);
        assert!(
            got.sessions
                .iter()
                .any(|s| s.status != SessionStatus::Idle)
        );
    }

    #[test]
    fn fixture_has_transcript_and_mixed_todos() {
        let got = fixture();
        assert!(got.transcript.len() >= 4);
        assert!(got.todos.len() >= 3);
        assert!(got.todos.iter().any(|t| t.done));
        assert!(got.todos.iter().any(|t| !t.done));
    }
}
