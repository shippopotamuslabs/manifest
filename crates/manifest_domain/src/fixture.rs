//! Session, transcript, and todo domain values plus the fixture shell renders.

use crate::engine::{
    ApprovalChoiceSummary, ApprovalRequest, PendingRequest, PendingResolution, Question,
    QuestionOption, QuestionRequest,
};

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

    /// Borrow the stored handle for wire calls.
    ///
    /// Callers still compare and match by the whole handle.
    pub fn as_str(&self) -> &str {
        &self.0
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
    /// The server has the record but no live host; resumes on next use.
    Orphaned,
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

    /// Build the project handle for a workspace.
    ///
    /// The single owner of the `workspace:` handle format: every
    /// workspace-derived project id is built here.
    pub fn for_workspace(workspace: &str) -> Self {
        Self(format!("workspace:{workspace}"))
    }

    /// Borrow the stored handle for comparisons.
    ///
    /// Callers still compare and match by the whole handle.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One project grouping sessions in the left panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    /// Opaque project handle.
    pub id: ProjectId,
    /// Short human name, e.g. "checkout-api".
    pub name: String,
    /// Folder sessions in this project run in; `None` when unknown,
    /// as in the fixture.
    pub workspace: Option<String>,
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
    /// Folder the session runs in as the server reported it; empty in the fixture.
    pub workspace: String,
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

/// Lifecycle status of one checklist entry, matching the four
/// statuses of the `write_todos` tool schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoStatus {
    /// Waiting to start.
    Pending,
    /// Actively being worked on.
    InProgress,
    /// Finished.
    Done,
    /// Dropped without finishing.
    Cancelled,
}

impl TodoStatus {
    /// Whether the step counts as finished.
    pub fn is_done(self) -> bool {
        matches!(self, TodoStatus::Done)
    }
}

/// One checklist entry the agent works through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    /// Short human description of the step.
    pub label: String,
    /// Lifecycle status of the step.
    pub status: TodoStatus,
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
    /// Unsettled pending requests for the awaiting session, in card
    /// order. Fixture mode seeds its pending store from these so the
    /// cards render without an engine; live mode never reads them.
    pub pending: Vec<PendingRequest>,
    /// Already-settled pending requests with their resolutions, in
    /// card order. Fixture mode collapses each to its one-line record.
    pub settled_pending: Vec<(PendingRequest, PendingResolution)>,
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
                workspace: None,
            },
            ProjectSummary {
                id: ProjectId::new("project-storefront-web"),
                name: String::from("storefront-web"),
                workspace: None,
            },
        ],
        sessions: vec![
            SessionSummary {
                id: SessionId::new("session-checkout-sync"),
                title: String::from("Fix checkout sync retries"),
                status: SessionStatus::Running,
                project: Some(ProjectId::new("project-checkout-api")),
                workspace: String::new(),
                relative_time: String::from("20h"),
                stream_dropped_deltas: 0,
            },
            SessionSummary {
                id: SessionId::new("session-migration-cleanup"),
                title: String::from("Clean up old migration scripts"),
                status: SessionStatus::Idle,
                project: Some(ProjectId::new("project-checkout-api")),
                workspace: String::new(),
                relative_time: String::from("1d"),
                stream_dropped_deltas: 0,
            },
            SessionSummary {
                id: SessionId::new("session-settings-copy"),
                title: String::from("Review new settings screen copy"),
                status: SessionStatus::AwaitingApproval,
                project: Some(ProjectId::new("project-storefront-web")),
                workspace: String::new(),
                relative_time: String::from("2d"),
                stream_dropped_deltas: 0,
            },
            SessionSummary {
                id: SessionId::new("session-empty-cart"),
                title: String::from("Audit empty-cart states"),
                status: SessionStatus::Idle,
                project: Some(ProjectId::new("project-storefront-web")),
                workspace: String::new(),
                relative_time: String::from("4d"),
                stream_dropped_deltas: 0,
            },
            SessionSummary {
                id: SessionId::new("session-cron-scratch"),
                title: String::from("Cron expression help"),
                status: SessionStatus::Idle,
                project: None,
                workspace: String::new(),
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
                status: TodoStatus::Done,
            },
            TodoItem {
                label: String::from("Move token refresh into the request path"),
                status: TodoStatus::InProgress,
            },
            TodoItem {
                label: String::from("Apply the patch after approval"),
                status: TodoStatus::Pending,
            },
            TodoItem {
                label: String::from("Rerun the checkout sync suite"),
                status: TodoStatus::Pending,
            },
        ],
        pending: vec![
            PendingRequest::Question {
                session: SessionId::new("session-settings-copy"),
                request_id: String::from("fixture-question-1"),
                question: QuestionRequest {
                    questions: vec![
                        Question {
                            id: String::from("q1"),
                            prompt: String::from("Which token refresh should settings sync use?"),
                            options: vec![
                                QuestionOption {
                                    label: String::from("Per-request refresh"),
                                    description: Some(String::from(
                                        "Refresh inside the request path and retry once on expiry.",
                                    )),
                                },
                                QuestionOption {
                                    label: String::from("Refresh on open"),
                                    description: Some(String::from(
                                        "Keep the current read-once behaviour when the screen opens.",
                                    )),
                                },
                                // A real option literally named None: a
                                // plain radio, never the synthetic Other.
                                QuestionOption {
                                    label: String::from("None"),
                                    description: None,
                                },
                            ],
                            multi: false,
                        },
                        Question {
                            id: String::from("q2"),
                            prompt: String::from("Which suites should cover expiry?"),
                            options: vec![
                                QuestionOption {
                                    label: String::from("Checkout sync"),
                                    description: Some(String::from(
                                        "The suite that already caught the two failures.",
                                    )),
                                },
                                QuestionOption {
                                    label: String::from("Settings sync"),
                                    description: None,
                                },
                            ],
                            multi: true,
                        },
                        Question {
                            id: String::from("q3"),
                            prompt: String::from("How should the changelog group this?"),
                            options: vec![
                                QuestionOption {
                                    label: String::from("Both sync fixes"),
                                    description: Some(String::from(
                                        "One entry covering checkout and settings sync.",
                                    )),
                                },
                                QuestionOption {
                                    label: String::from("Separate entries"),
                                    description: None,
                                },
                            ],
                            multi: false,
                        },
                    ],
                },
            },
            PendingRequest::Approval {
                session: SessionId::new("session-settings-copy"),
                request_id: String::from("fixture-approval-1"),
                approval: ApprovalRequest {
                    tool: String::from("edit_file"),
                    subject_line: String::from(
                        "Apply per-request token refresh to settings sync (1 file)",
                    ),
                    choices: vec![
                        ApprovalChoiceSummary {
                            label: String::from("Approve"),
                            decision: String::from("approved"),
                            scope: String::from("once"),
                            accepts_feedback: false,
                        },
                        ApprovalChoiceSummary {
                            label: String::from("Deny"),
                            decision: String::from("denied"),
                            scope: String::from("once"),
                            accepts_feedback: true,
                        },
                    ],
                },
            },
        ],
        settled_pending: vec![(
            PendingRequest::Approval {
                session: SessionId::new("session-settings-copy"),
                request_id: String::from("fixture-settled-1"),
                approval: ApprovalRequest {
                    tool: String::from("bash"),
                    subject_line: String::from("Run the checkout sync suite"),
                    choices: Vec::new(),
                },
            },
            PendingResolution::Approved,
        )],
    }
}

/// Short human name for a workspace: its last non-empty `/`-segment.
///
/// Trailing slashes are ignored; empty or all-slash input falls back
/// to the input unchanged.
pub fn workspace_name(workspace: &str) -> &str {
    workspace
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(workspace)
}

/// Trim trailing slashes for workspace comparison.
fn trimmed_workspace(workspace: &str) -> &str {
    workspace.trim_end_matches('/')
}

/// Compose a session row's display title: `{label} · {title}`, where
/// `label` is the project name when the session belongs to a project,
/// else [`workspace_name`] of its workspace, and `title` is the
/// session's own title — the server's session name, or the short id
/// for unnamed rows.
pub fn compose_session_title(project_name: Option<&str>, workspace: &str, title: &str) -> String {
    match project_name {
        Some(name) => format!("{name} · {title}"),
        None => format!("{} · {title}", workspace_name(workspace)),
    }
}

/// Group sessions onto projects by workspace.
///
/// For each session with a non-empty workspace, `project` becomes the
/// listed project whose workspace matches (exact strings compared
/// after trimming trailing `/`; a project with `None` or empty
/// workspace never matches), else `None`, and the title recomposes
/// via [`compose_session_title`] from the matched project name (else
/// the workspace name) and the session's incoming title. Sessions
/// with an empty workspace are returned untouched. Input session
/// order is preserved.
pub fn assign_projects(
    projects: &[ProjectSummary],
    sessions: Vec<SessionSummary>,
) -> Vec<SessionSummary> {
    sessions
        .into_iter()
        .map(|mut session| {
            if session.workspace.is_empty() {
                return session;
            }
            let matched = projects.iter().find(|project| match &project.workspace {
                Some(workspace) if !workspace.is_empty() => {
                    trimmed_workspace(workspace) == trimmed_workspace(&session.workspace)
                }
                _ => false,
            });
            match matched {
                Some(project) => {
                    let title = compose_session_title(Some(&project.name), &session.workspace, &session.title);
                    session.title = title;
                    session.project = Some(project.id.clone());
                }
                None => {
                    let title = compose_session_title(None, &session.workspace, &session.title);
                    session.title = title;
                    session.project = None;
                }
            }
            session
        })
        .collect()
}

/// Derive one project per distinct session workspace, in first-seen order.
///
/// Workspaces compare after trimming trailing `/` (as in
/// [`assign_projects`]), so `/w` and `/w/` seed one project; the
/// project keeps the first-seen workspace string verbatim for its
/// id, [`workspace_name`] name, and workspace.
/// Sessions with empty workspaces contribute no project.
pub fn seed_projects(sessions: &[SessionSummary]) -> Vec<ProjectSummary> {
    let mut seen: Vec<&str> = Vec::new();
    for session in sessions {
        if session.workspace.is_empty() {
            continue;
        }
        let trimmed = trimmed_workspace(&session.workspace);
        if !seen.iter().any(|prior| trimmed_workspace(prior) == trimmed) {
            seen.push(session.workspace.as_str());
        }
    }
    seen.into_iter()
        .map(|workspace| ProjectSummary {
            id: ProjectId::for_workspace(workspace),
            name: workspace_name(workspace).to_string(),
            workspace: Some(workspace.to_string()),
        })
        .collect()
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
    fn fixture_pending_demonstrates_cards_for_the_awaiting_session() {
        let got = fixture();
        let awaiting = SessionId::new("session-settings-copy");
        assert!(
            got.sessions
                .iter()
                .any(|s| s.id == awaiting && s.status == SessionStatus::AwaitingApproval)
        );
        // Two unsettled cards: a 3-question card, then an approval card.
        assert_eq!(got.pending.len(), 2);
        assert!(got.pending.iter().all(|r| r.session() == &awaiting));
        let questions = got.pending[0].questions().expect("question card first");
        assert_eq!(questions.len(), 3);
        assert!(!questions[0].multi && questions[1].multi && !questions[2].multi);
        assert!(questions[0].options.iter().any(|o| o.label == "None"));
        assert!(questions[0].options[0].has_description());
        assert!(matches!(got.pending[1], PendingRequest::Approval { .. }));
        // One settled record, approved by the user.
        assert_eq!(got.settled_pending.len(), 1);
        assert_eq!(got.settled_pending[0].0.session(), &awaiting);
        assert_eq!(got.settled_pending[0].1, PendingResolution::Approved);
    }

    #[test]
    fn fixture_has_transcript_and_mixed_todos() {
        let got = fixture();
        assert!(got.transcript.len() >= 4);
        assert!(got.todos.len() >= 3);
        assert!(got.todos.iter().any(|t| t.status == TodoStatus::Done));
        assert!(got.todos.iter().any(|t| t.status == TodoStatus::InProgress));
        assert!(got.todos.iter().any(|t| t.status == TodoStatus::Pending));
    }

    #[test]
    fn workspace_name_takes_last_non_empty_segment() {
        assert_eq!(workspace_name("/home/dev/beta"), "beta");
        assert_eq!(workspace_name("/home/dev/beta/"), "beta");
        assert_eq!(workspace_name("name"), "name");
        assert_eq!(workspace_name(""), "");
        assert_eq!(workspace_name("/"), "/");
    }

    #[test]
    fn project_id_for_workspace_owns_handle_format() {
        assert_eq!(
            ProjectId::for_workspace("/home/dev/beta"),
            ProjectId::new("workspace:/home/dev/beta")
        );
        assert_eq!(
            ProjectId::for_workspace("/home/dev/beta").as_str(),
            "workspace:/home/dev/beta"
        );
    }

    fn helper_session(id: &str, workspace: &str) -> SessionSummary {
        SessionSummary {
            id: SessionId::new(id),
            title: String::from("stale"),
            status: SessionStatus::Idle,
            project: None,
            workspace: String::from(workspace),
            relative_time: String::from("now"),
            stream_dropped_deltas: 0,
        }
    }

    fn helper_project(name: &str, workspace: Option<&str>) -> ProjectSummary {
        ProjectSummary {
            id: ProjectId::new(format!("project-{name}")),
            name: String::from(name),
            workspace: workspace.map(String::from),
        }
    }

    #[test]
    fn assign_matches_trims_slashes_and_recomposes_titles() {
        let projects = vec![
            helper_project("beta", Some("/home/dev/beta/")),
            helper_project("mystery", None),
            helper_project("empty", Some("")),
        ];
        // Incoming titles are what the mapper produced: a short id for
        // unnamed rows, the server's session name for named ones.
        let mut named = helper_session("sess-fresh-2", "/home/dev/gamma");
        named.title = String::from("Fix the login bug");
        let got = assign_projects(
            &projects,
            vec![
                helper_session("sess-run-1", "/home/dev/beta"),
                named,
                helper_session("ééééééééXYZ", "/w/long-name"),
                helper_session("untouched", ""),
            ],
        );
        assert_eq!(
            got[0].project,
            Some(ProjectId::new("project-beta")),
            "trailing-slash project workspace still matches"
        );
        assert_eq!(got[0].title, "beta · stale");
        assert_eq!(got[1].project, None);
        assert_eq!(got[1].title, "gamma · Fix the login bug");
        assert_eq!(got[2].title, "long-name · stale");
        // Empty workspaces pass through untouched.
        assert_eq!(got[3].title, "stale");
        assert_eq!(got[3].project, None);
        // Input order preserved.
        let ids: Vec<_> = got.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["sess-run-1", "sess-fresh-2", "ééééééééXYZ", "untouched"]
        );
    }

    #[test]
    fn seed_projects_dedups_first_seen_in_order() {
        let sessions = vec![
            helper_session("s1", "/home/dev/beta"),
            helper_session("s2", "/home/dev/alpha"),
            helper_session("s3", "/home/dev/beta"),
            helper_session("s4", ""),
        ];
        let got = seed_projects(&sessions);
        assert_eq!(
            got.iter().map(|p| &p.name).collect::<Vec<_>>(),
            vec![&String::from("beta"), &String::from("alpha")]
        );
        assert_eq!(got[0].id, ProjectId::for_workspace("/home/dev/beta"));
        assert_eq!(got[0].workspace, Some(String::from("/home/dev/beta")));
    }

    #[test]
    fn seed_projects_dedups_trailing_slash_variants() {
        let sessions = vec![
            helper_session("s1", "/w"),
            helper_session("s2", "/w/"),
            helper_session("s3", "/w//"),
        ];
        let got = seed_projects(&sessions);
        assert_eq!(got.len(), 1, "trailing slashes seed one project");
        assert_eq!(got[0].workspace, Some(String::from("/w")));
        assert_eq!(got[0].id, ProjectId::for_workspace("/w"));
        assert_eq!(got[0].name, "w");
        // assign_projects still groups the slashed session under it.
        let assigned = assign_projects(&got, sessions);
        assert!(
            assigned
                .iter()
                .all(|s| s.project == Some(ProjectId::for_workspace("/w")))
        );
    }

    #[test]
    fn seed_then_assign_round_trips_grouping() {
        let sessions = vec![
            helper_session("sess-run-1", "/home/dev/beta"),
            helper_session("sess-idle-2", "/home/dev/alpha"),
        ];
        let projects = seed_projects(&sessions);
        let got = assign_projects(&projects, sessions);
        assert_eq!(
            got[0].project,
            Some(ProjectId::for_workspace("/home/dev/beta"))
        );
        assert_eq!(got[0].title, "beta · stale");
        assert_eq!(
            got[1].project,
            Some(ProjectId::for_workspace("/home/dev/alpha"))
        );
        assert_eq!(got[1].title, "alpha · stale");
    }

    #[test]
    fn compose_session_title_labels_by_project_else_workspace() {
        assert_eq!(
            compose_session_title(Some("beta"), "/home/dev/beta", "Fix the login bug"),
            "beta · Fix the login bug"
        );
        assert_eq!(
            compose_session_title(None, "/home/dev/gamma", "sess-fre"),
            "gamma · sess-fre"
        );
    }
}
