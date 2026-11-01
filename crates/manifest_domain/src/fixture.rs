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

/// Opaque handle for one project.
///
/// Mirrors [`SessionId`]: callers compare and match by the whole handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectId(String);

impl ProjectId {
    /// Build a project id from its stored handle.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// One project grouping sessions in the left panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    /// Opaque project handle.
    pub id: ProjectId,
    /// Short human name, e.g. "checkout-api".
    pub name: String,
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
    /// Owning project, if the session belongs to one.
    pub project: Option<ProjectId>,
    /// Precomputed relative-time label for the row's right edge, e.g. '20h'.
    pub relative_time: String,
    /// Cumulative stream delta events whose send failed for this session (0 in the fixture; live values come from muse_status).
    pub stream_dropped_deltas: u64,
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
    /// An error shown in the transcript like an assistant response.
    /// The shell renders it in the danger tone so failures read as
    /// part of the conversation, per the product's error presentation.
    ErrorNote { text: String },
}

/// One checklist entry the agent works through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    /// Short human description of the step.
    pub label: String,
    /// Whether the step is finished.
    pub done: bool,
}

/// The whole fixture: projects, sessions, one visible transcript, and todos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixture {
    /// Projects for the left panel, in display order.
    pub projects: Vec<ProjectSummary>,
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
        projects: vec![
            ProjectSummary {
                id: ProjectId::new("project-checkout-api"),
                name: String::from("checkout-api"),
            },
            ProjectSummary {
                id: ProjectId::new("project-storefront-web"),
                name: String::from("storefront-web"),
            },
        ],
        sessions: vec![
            SessionSummary {
                id: SessionId::new("session-checkout-sync"),
                title: String::from("Fix checkout sync retries"),
                status: SessionStatus::Running,
                project: Some(ProjectId::new("project-checkout-api")),
                relative_time: String::from("20h"),
                stream_dropped_deltas: 0,
            },
            SessionSummary {
                id: SessionId::new("session-migration-cleanup"),
                title: String::from("Clean up old migration scripts"),
                status: SessionStatus::Idle,
                project: Some(ProjectId::new("project-checkout-api")),
                relative_time: String::from("1d"),
                stream_dropped_deltas: 0,
            },
            SessionSummary {
                id: SessionId::new("session-settings-copy"),
                title: String::from("Review new settings screen copy"),
                status: SessionStatus::AwaitingApproval,
                project: Some(ProjectId::new("project-storefront-web")),
                relative_time: String::from("2d"),
                stream_dropped_deltas: 0,
            },
            SessionSummary {
                id: SessionId::new("session-empty-cart"),
                title: String::from("Audit empty-cart states"),
                status: SessionStatus::Idle,
                project: Some(ProjectId::new("project-storefront-web")),
                relative_time: String::from("4d"),
                stream_dropped_deltas: 0,
            },
            SessionSummary {
                id: SessionId::new("session-cron-scratch"),
                title: String::from("Cron expression help"),
                status: SessionStatus::Idle,
                project: None,
                relative_time: String::from("6d"),
                stream_dropped_deltas: 0,
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
            TranscriptItem::AssistantTurn {
                text: String::from(
                    "Patch applied. The refresh now runs inside the request path, and an \
                     expired token retries once before the sync reports an error.",
                ),
            },
            TranscriptItem::ToolNote {
                text: String::from("Ran checkout sync suite: 16 passed, 0 failed."),
            },
            TranscriptItem::AssistantTurn {
                text: String::from(
                    "All sixteen pass, including the two expired-token cases. The change \
                     stays inside the sync client: no callers need updates, and the \
                     startup refresh still runs as a warmup.",
                ),
            },
            TranscriptItem::UserTurn {
                text: String::from(
                    "Good. Now check whether the settings sync has the same problem.",
                ),
            },
            TranscriptItem::AssistantTurn {
                text: String::from(
                    "It does. Settings sync reads the token once when the screen opens \
                     and never refreshes it, so a long-lived screen fails the same way \
                     after expiry.",
                ),
            },
            TranscriptItem::ToolNote {
                text: String::from(
                    "Searched token reads: settings sync reads at open (1 site), \
                     checkout sync reads per request (1 site).",
                ),
            },
            TranscriptItem::AssistantTurn {
                text: String::from(
                    "One read site to change. I can apply the same per-request refresh \
                     there and add the expired-token case to the settings suite.",
                ),
            },
            TranscriptItem::UserTurn {
                text: String::from("Do it, then summarize both changes for the changelog."),
            },
            TranscriptItem::AssistantTurn {
                text: String::from(
                    "Done. Checkout sync and settings sync both refresh expired tokens \
                     inside the request path and retry once. Both suites pass with the \
                     new expired-token cases covered.",
                ),
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
        assert!(got.sessions.iter().any(|s| s.status != SessionStatus::Idle));
    }

    #[test]
    fn fixture_projects_cover_sessions() {
        let got = fixture();
        assert!(got.projects.len() >= 2);
        let ids: Vec<_> = got.projects.iter().map(|p| &p.id).collect();
        for session in &got.sessions {
            if let Some(project) = &session.project {
                assert!(ids.contains(&project));
            }
        }
        assert!(got.sessions.iter().any(|s| s.project.is_none()));
    }

    #[test]
    fn fixture_relative_times_are_labels() {
        let got = fixture();
        assert!(!got.sessions.is_empty());
        for session in &got.sessions {
            assert!(!session.relative_time.is_empty());
        }
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
