//! Root shell: the four-region layout, the root reducer, and boot.
//!
//! The top bar is a view function on the root state, not a module with
//! invented state (plan chunk 4): it owns nothing, so it owns no file.
//! Child features own their state; this module holds the child states,
//! routes messages down, and maps child messages up (UI contract §7).

use std::collections::{HashMap, HashSet};

use iced::{Element, Length, Task};
use manifest_domain::{
    Connection, EffortChoice, EngineEvent, Fixture, GoalInfo, ModelChoice, PermissionMode,
    ProjectId, ProjectSummary, SessionId, SessionStatus, SessionSummary, SpawnChoices, StreamEvent,
    StreamItem, TodoItem, TodoStatus, TurnId, UsageSnapshot, WorktreeChoice,
};

use crate::engine;
use crate::kit;
use crate::theme::{
    DIVIDER_HIT_WIDTH, FONT_SIZE_BODY, FONT_SIZE_SMALL, FONT_SIZE_TITLE, MODEL_MENU_BOTTOM_OFFSET,
    PANEL_WIDTH_DEFAULT, PANEL_WIDTH_MAX, PANEL_WIDTH_MIN, SPACE_M, SPACE_S, SPACE_XS,
    TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, WINDOW_RESIZE_HANDLE, attach_menu_bottom_offset,
};
use crate::{composer, pending, sessions, transcript};

/// The TODO drawer has two real situations, so it is an enum, not a bool
/// (UI contract §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TodoDrawer {
    /// The drawer is a slim peek strip above the composer.
    #[default]
    Peek,
    /// The drawer lists fixture todos between transcript and composer.
    Open,
}

/// The side-panel divider drag has two real situations, so it is an
/// enum, not a bool (UI contract §1).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PanelDrag {
    /// No divider drag is in flight; panel events are not subscribed.
    #[default]
    Idle,
    /// A divider drag is in flight. `offset` is the grab point: cursor x
    /// minus panel width, captured on the first move after press so the
    /// panel does not jump to the cursor.
    Dragging {
        /// Cursor-x minus panel-width at grab time; `None` until the
        /// first move after press captures it.
        offset: Option<f32>,
    },
}

/// Root messages, enumerated before the view (UI contract §1).
/// No `PartialEq`: [`transcript::Message`] carries a scroll viewport,
/// which is `Debug + Clone` but not `PartialEq`.
#[derive(Debug, Clone)]
pub enum Message {
    /// A session-list message, routed to [`sessions::State`].
    Sessions(sessions::Message),
    /// A composer message, routed to [`composer::State`].
    Composer(composer::Message),
    /// A transcript scroll message, routed to [`transcript::State`].
    Transcript(transcript::Message),
    /// A pending-card message, routed to [`pending::State`]; its
    /// effect (fetch/decide/answer) runs against the engine here.
    /// Kept as the direct routing spelling; inline cards arrive via
    /// `Transcript(Pending(..))`.
    #[allow(dead_code)]
    Pending(pending::Message),
    /// Ctrl+Enter pressed: submits/advances the visible awaiting
    /// question card, mirroring its Next/Submit button.
    SubmitPendingShortcut,
    /// Toggles the TODO drawer between peek strip and open.
    ToggleTodoDrawer,
    /// A press landed on the panel divider; the next moves resize.
    PanelDragStarted,
    /// The cursor moved; while a divider drag is in flight the panel
    /// width tracks it.
    PanelDragMoved(iced::Point),
    /// The left button released; any divider drag ends.
    PanelDragEnded,
    /// Resizes the window from one corner while the handle is pressed.
    ResizeWindow(iced::window::Direction),
    /// The runtime reported the latest window id at boot (or none).
    WindowIdentified(Option<iced::window::Id>),
    /// A window resize happened; the maximized state should be rechecked.
    WindowResized,
    /// The runtime reported the window's maximized state.
    WindowMaximizedChecked(bool),
    /// Starts dragging the (undecorated) window.
    DragWindow,
    /// Minimizes the window.
    MinimizeWindow,
    /// Toggles the window between maximized and restored.
    ToggleMaximizeWindow,
    /// Closes the window.
    CloseWindow,
    /// One engine-stream item, stamped with the connection attempt that
    /// produced it; items from older attempts are stale and ignored.
    Engine {
        /// The connection attempt the item belongs to.
        attempt: u64,
        /// The handle (sent once, first) or one engine event.
        input: engine::Input,
    },
    /// The user pressed Retry after a connection failure; drops the old
    /// handle and starts a fresh connection attempt.
    Reconnect,
    /// The project registry finished loading. `Ok(Present)` replaces the
    /// registry and regroups; `Ok(Missing)` seeds from the loaded sessions
    /// (or waits for the first load when empty); `Err` keeps the derived
    /// fallback and shows the message. Store errors convert to strings
    /// once here, at the boundary into the reducer.
    ProjectsLoaded(Result<manifest_store::Loaded, String>),
    /// A registry save finished. `Ok` needs no state change; `Err` shows
    /// the message verbatim. Local state is never rolled back: a failed
    /// save is partial success carried as a value.
    ProjectsSaved(Result<(), String>),
    /// The draft's worktree mode changed; no-op while no draft is open.
    DraftWorktreeModeSelected(DraftWorktreeMode),
    /// The draft's create-mode base ref changed; no-op while no draft.
    DraftWorktreeBaseChanged(String),
    /// The draft's existing-mode path changed; no-op while no draft.
    DraftWorktreePathChanged(String),
    /// The OS folder picker returned. `Some` types the path into the open
    /// popup; `None` (dismissed) is a no-op.
    DirectoryPicked(Option<String>),
    /// The add-project form validation finished. `Ok` carries the
    /// canonical directory and the trimmed name; `Err` carries the
    /// message shown under the form.
    AddProjectChecked(Result<(String, String), String>),
    /// A git branch read for a workspace finished. Applied only when
    /// the workspace still names the visible one; any I/O failure
    /// arrives as `None`, an expected display state that never shows
    /// a transcript error.
    BranchLoaded {
        /// Workspace the read ran for; the staleness guard.
        workspace: String,
        /// Display branch (`None` while unread or unreadable).
        branch: Option<String>,
    },
}

impl From<sessions::Message> for Message {
    fn from(message: sessions::Message) -> Self {
        Message::Sessions(message)
    }
}

impl From<transcript::Message> for Message {
    fn from(message: transcript::Message) -> Self {
        Message::Transcript(message)
    }
}

impl From<composer::Message> for Message {
    fn from(message: composer::Message) -> Self {
        Message::Composer(message)
    }
}

/// How the draft's spawned session places its working directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DraftWorktreeMode {
    /// Spawn directly in the project workspace.
    #[default]
    Off,
    /// Create a worktree for the session, optionally from a base ref.
    Create,
    /// Use an existing worktree folder.
    Existing,
}

/// A session being composed before it exists: entered from a project's
/// Plus icon button, resolved into a real session on the next send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSessionDraft {
    /// Folder the spawned session will run in, from the project.
    pub workspace: String,
    /// Precomputed "project · New session" title the topbar reads.
    pub title: String,
    /// How the spawned session places its working directory.
    pub worktree_mode: DraftWorktreeMode,
    /// Base ref for create mode; blank means no base.
    pub worktree_base: String,
    /// Folder for existing mode.
    pub worktree_path: String,
    /// The model the spawned session runs on.
    pub model: ModelChoice,
    /// The reasoning effort the spawned session runs with.
    pub effort: EffortChoice,
    /// The permission posture the spawned session runs with.
    pub permission: PermissionMode,
}

impl NewSessionDraft {
    /// Maps the draft's worktree fields to engine spawn choices: `Off`
    /// spawns in place, `Create` trims the base (`None` when blank),
    /// `Existing` carries the trimmed path. The composer picker state
    /// rides along so the first send spawns with the shown choices.
    pub fn choices(&self) -> SpawnChoices {
        let worktree = match self.worktree_mode {
            DraftWorktreeMode::Off => WorktreeChoice::Off,
            DraftWorktreeMode::Create => {
                let base = self.worktree_base.trim();
                WorktreeChoice::Create {
                    base: if base.is_empty() {
                        None
                    } else {
                        Some(base.to_string())
                    },
                }
            }
            DraftWorktreeMode::Existing => WorktreeChoice::Existing {
                path: self.worktree_path.trim().to_string(),
            },
        };
        SpawnChoices {
            worktree,
            model: self.model.clone(),
            effort: self.effort,
            permission: self.permission,
        }
    }
}

/// A spawn in flight, kept so a failure can restore it for retry: the
/// taken draft (when the spawn came from one) plus the failed prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSpawn {
    /// The draft taken by the spawn, if it came from one.
    pub draft: Option<NewSessionDraft>,
    /// The prompt the spawn carried.
    pub prompt: String,
}

/// Root state: the child states plus the drawer.
///
/// `transcript::State` holds incrementally parsed markdown, which is not
/// `Clone`, so neither is the app: switch sessions by moving states.
#[derive(Debug)]
pub struct App {
    /// Left-panel session list; owns the selection.
    pub sessions: sessions::State,
    /// Visible-session transcript; inactive sessions rest in
    /// [`transcripts`](App::transcripts).
    pub transcript: transcript::State,
    /// Inactive sessions' transcripts, keyed by session.
    pub transcripts: HashMap<SessionId, transcript::State>,
    /// Which session [`transcript`](App::transcript) belongs to; `None`
    /// in fixture mode and while the selection names no listed session.
    pub visible_session: Option<SessionId>,
    /// Sessions with a turn in flight.
    pub running: HashSet<SessionId>,
    /// The running turn per session, when the server named it. Steer
    /// and Stop address this turn; the entry clears with the turn.
    pub running_turns: HashMap<SessionId, TurnId>,
    /// Latest usage snapshot per session; the composer's meter reads
    /// the visible session's entry.
    pub usage: HashMap<SessionId, UsageSnapshot>,
    /// A spawned session awaiting its first list load to select it.
    pub select_on_load: Option<SessionId>,
    /// A spawn command is in flight and the visible transcript holds
    /// its optimistic prompt; cleared on `SessionSpawned` (which
    /// adopts the transcript) or a spawn-tagged `CommandFailed`.
    pub awaiting_spawn: bool,
    /// The in-flight spawn, set together with [`App::awaiting_spawn`]
    /// whenever a spawn is issued; `Some` means the spawn can be
    /// restored on failure.
    pub pending_spawn: Option<PendingSpawn>,
    /// Orphaned sessions with a resume in flight; gates duplicate
    /// resumes and the send button.
    pub resuming: HashSet<SessionId>,
    /// Sessions whose stored history was requested; kept once
    /// requested (even when the fetch returns empty) so a session
    /// never refetches in a loop. A history-tagged `CommandFailed`
    /// releases the entry so the next select retries.
    pub history_requested: HashSet<SessionId>,
    /// Sessions spawned this run; their transcripts start with the
    /// optimistic echo, so no history backfill is ever requested
    /// for them.
    pub spawned_sessions: HashSet<SessionId>,
    /// A new session being composed in a project's workspace; set by
    /// the project's Plus icon button, cleared on send (which spawns it) or
    /// on session select. While set, the visible transcript is the
    /// draft's, and list syncs must not steal it.
    pub new_session: Option<NewSessionDraft>,
    /// Pending approval/question cards, by session. Transcript
    /// entries hold only request ids; this owns payloads/phases.
    pub pending: pending::State,
    /// Draft input cluster.
    pub composer: composer::State,
    /// A model-catalog request is in flight. Set on each request,
    /// cleared on `ModelsLoaded` (or on the models-tagged
    /// `CommandFailed` that marks the catalog failed, or on disconnect).
    pub models_pending: bool,
    /// A skill-catalog request for the visible session is in flight.
    /// Set on each request, cleared on the visible session's
    /// `SkillsLoaded` (or on its skills-tagged `CommandFailed` that
    /// marks the catalog failed, or on disconnect).
    pub skills_pending: bool,
    /// Branch shown in the top bar for the visible workspace;
    /// `None` while unread or unreadable. Display only: never used to
    /// switch branches.
    pub branch: Option<String>,
    /// Workspace [`App::branch`] was read for; the staleness guard.
    pub branch_workspace: Option<String>,
    /// Todo list per session, replaced wholesale by each `write_todos`
    /// stream item; the drawer renders the visible session's entry.
    pub todo_lists: HashMap<SessionId, Vec<TodoItem>>,
    /// Goal per session, replaced wholesale by each `goal-changed`
    /// stream event (`None` when the host cleared it); the composer
    /// strip renders the visible session's entry.
    pub goals: HashMap<SessionId, Option<GoalInfo>>,
    /// Whether the TODO drawer is a peek strip or open.
    pub todo_drawer: TodoDrawer,
    /// Current width of the side panel; the divider drag rewrites it.
    pub panel_width: f32,
    /// Whether a divider drag is in flight.
    pub panel_drag: PanelDrag,
    /// Precomputed "<done> of <total> done" line for the drawer bar.
    pub todo_summary: String,
    /// Label of the first unfinished todo, for the drawer bar.
    pub current_todo: Option<String>,
    /// The window id from boot, for the title-bar window controls.
    pub window_id: Option<iced::window::Id>,
    /// Whether the window is maximized; drives the maximize icon.
    pub window_maximized: bool,
    /// The named connection situation the top bar renders.
    pub connection: Connection,
    /// Counts connection attempts; engine-stream items carry the attempt
    /// that produced them so stale streams are ignored.
    pub connection_attempt: u64,
    /// The live engine handle, present once the stream delivers it and
    /// cleared on failure or retry. `None` in fixture mode.
    pub engine: Option<engine::EngineHandle>,
    /// The server config to connect with; `None` in fixture mode, where
    /// no server is ever spawned.
    pub engine_config: Option<manifest_mcp::ServerConfig>,
    /// The persisted project registry store; `None` in fixture mode,
    /// where nothing is ever read from or written to disk.
    pub project_store: Option<manifest_store::ProjectStore>,
    /// The authoritative project registry once loaded. Empty until the
    /// load completes; every mutation sets this to the new list before
    /// its save task is created, so the task always saves the new list.
    pub projects: Vec<ProjectSummary>,
    /// Whether `projects` is authoritative. `false` (load pending or
    /// failed, or no store) falls back to derived grouping; `true` means
    /// the list renders from `projects`.
    pub projects_ready: bool,
    /// Whether the authoritative load said `Missing` while no sessions
    /// were listed yet: the registry stays empty until the first session
    /// load seeds it. Cleared by any explicit registry content.
    pub projects_missing: bool,
}

impl App {
    /// Builds shell state from domain values. Pure: no Iced runtime, so
    /// headless tests drive it directly. Default selection is the first
    /// session; the drawer starts as a peek strip. The drawer bar
    /// strings are precomputed here so the view formats nothing
    /// (UI contract §2). Empty session lists fall back to the
    /// sessions-state placeholder (no `unwrap` / panic paths).
    pub fn from_fixture(fixture: Fixture) -> Self {
        // Fixture pending data is seeded below, after the literal, so
        // the cards exist without an engine. Live boot clears it.
        let fixture_pending = fixture.pending.clone();
        let fixture_settled = fixture.settled_pending.clone();
        let seed_fallback = fixture.sessions.first().map(|summary| summary.id.clone());
        let done = fixture
            .todos
            .iter()
            .filter(|todo| todo.status.is_done())
            .count();
        let total = fixture.todos.len();
        let todo_summary = format!("{done} of {total} done");
        let current_todo = current_todo_label(&fixture.todos);
        let mut app = App {
            sessions: sessions::State::from_projects(fixture.projects, fixture.sessions),
            transcript: transcript::State::from_items(fixture.transcript),
            transcripts: HashMap::new(),
            visible_session: None,
            running: HashSet::new(),
            running_turns: HashMap::new(),
            usage: HashMap::new(),
            select_on_load: None,
            awaiting_spawn: false,
            pending_spawn: None,
            resuming: HashSet::new(),
            history_requested: HashSet::new(),
            spawned_sessions: HashSet::new(),
            new_session: None,
            pending: pending::State::default(),
            composer: composer::State::new(),
            models_pending: false,
            skills_pending: false,
            branch: None,
            branch_workspace: None,
            todo_lists: HashMap::new(),
            goals: HashMap::new(),
            todo_drawer: TodoDrawer::Peek,
            todo_summary,
            current_todo,
            panel_width: PANEL_WIDTH_DEFAULT,
            panel_drag: PanelDrag::Idle,
            window_id: None,
            window_maximized: false,
            connection: Connection::Disconnected,
            connection_attempt: 0,
            engine: None,
            engine_config: None,
            project_store: None,
            projects: Vec::new(),
            projects_ready: false,
            projects_missing: false,
        };
        // Fixture mode only: seed the pending store from the fixture
        // and point the visible transcript at the awaiting session, so
        // The transcript pause-point entries render the cards inline
        // with no engine attached. Settled
        // fixture requests load as cards, then collapse in place to
        // their one-line records.
        let fixture_owner = fixture_pending
            .first()
            .map(|request| request.session().clone())
            .or_else(|| {
                fixture_settled
                    .first()
                    .map(|(request, _)| request.session().clone())
            });
        if let Some(owner) = fixture_owner {
            app.visible_session = Some(owner);
            let mut all = fixture_pending;
            all.extend(fixture_settled.iter().map(|(request, _)| request.clone()));
            app.apply_pending_loaded(all);
            for (request, resolution) in fixture_settled {
                app.apply_pending_settled(
                    request.session().clone(),
                    request.request_id().to_string(),
                    resolution,
                );
            }
        }
        // Fixture mode only: the fixture list belongs to the visible
        // session (else the first listed one), so the drawer keeps its
        // meaning; live boot below starts with no lists.
        let seed_owner = app.visible_session.clone().or(seed_fallback);
        if let Some(owner) = seed_owner {
            app.todo_lists.insert(owner, fixture.todos);
        }
        app.sync_visible_todos();
        app.refresh_send_gate();
        app
    }

    /// Production boot: connects to the real server. Starts empty (no
    /// sessions, no transcript entries, no todos) with the connection
    /// at `Connecting`, then connects on the first subscription tick;
    /// sessions arrive through the engine stream. The registry starts
    /// empty and unauthoritative; [`App::project_load_task`] loads it.
    pub fn live(
        config: manifest_mcp::ServerConfig,
        project_store: Option<manifest_store::ProjectStore>,
    ) -> Self {
        let mut app = Self::from_fixture(manifest_domain::fixture());
        app.sessions = sessions::State::from_projects(Vec::new(), Vec::new());
        app.transcript = transcript::State::default();
        // Live starts with no cards: the fixture seed is fixture-only.
        app.pending = pending::State::default();
        app.visible_session = None;
        app.todo_lists = HashMap::new();
        app.goals = HashMap::new();
        app.todo_summary = String::from("0 of 0 done");
        app.current_todo = None;
        app.connection = Connection::Connecting;
        app.engine_config = Some(config);
        app.project_store = project_store;
        app.projects = Vec::new();
        app.projects_ready = false;
        app.projects_missing = false;
        app.refresh_send_gate();
        app
    }

    /// Boots the shell from [`manifest_domain::fixture()`].
    pub fn boot() -> Self {
        Self::from_fixture(manifest_domain::fixture())
    }

    /// Root reducer. Child messages fold into child states; window
    /// messages run against the boot-reported window id, and fall back
    /// to [`Task::none()`] while it is unknown. Panel-drag moves only
    /// fold while a drag is in flight; anything else is a no-op.
    /// [`App::subscription`] feeds those moves while dragging.
    /// Keeps the visible transcript on the selected session in live
    /// mode: parks the current state under its owner and loads the new
    /// selection's stored state (or a default). A selection naming no
    /// listed session parks the visible state and shows an empty
    /// transcript. Fixture mode shows the boot transcript and never syncs.
    fn sync_visible_transcript(&mut self) {
        // Draft mode owns the visible transcript; background list
        // loads must not steal it.
        if self.new_session.is_some() {
            return;
        }
        if self.engine_config.is_none() {
            return;
        }
        let selected = self.sessions.selected.clone();
        if !self.session_is_listed(&selected) {
            // Park the visible state under its owner when one is showing;
            // before the first load there is no owner and the boot
            // transcript stays until a listed selection arrives.
            if let Some(owner) = self.visible_session.take() {
                self.transcripts
                    .insert(owner, std::mem::take(&mut self.transcript));
                self.transcript = transcript::State::default();
            }
            self.visible_session = None;
            self.sync_composer_usage();
            self.sync_composer_goal();
            self.sync_visible_todos();
            return;
        }
        if self.visible_session.as_ref() == Some(&selected) {
            return;
        }
        if let Some(owner) = self.visible_session.replace(selected.clone()) {
            self.transcripts
                .insert(owner, std::mem::take(&mut self.transcript));
        }
        self.transcript = self.transcripts.remove(&selected).unwrap_or_default();
        self.sync_composer_usage();
        self.sync_composer_goal();
        self.sync_visible_todos();
    }

    /// Enters new-session draft mode for the named project: parks the
    /// visible transcript under its owner (when one is showing) and
    /// shows an empty draft transcript with the precomputed title.
    /// Live mode only, never while a spawn is in flight, and only when
    /// the project is listed with a known workspace; anything else
    /// leaves the list expansion (done by the child) as the whole
    /// effect.
    fn enter_new_session_draft(&mut self, project: &ProjectId) {
        if self.engine_config.is_none() || self.awaiting_spawn {
            return;
        }
        let Some(group) = self
            .sessions
            .groups
            .iter()
            .find(|group| group.project.id == *project)
        else {
            return;
        };
        let Some(workspace) = group.project.workspace.clone() else {
            return;
        };
        let title = format!("{} · New session", group.project.name);
        if let Some(owner) = self.visible_session.take() {
            self.transcripts
                .insert(owner, std::mem::take(&mut self.transcript));
        }
        self.transcript = transcript::State::default();
        // The draft spawns with the shown picker state.
        let composer = &self.composer;
        self.new_session = Some(NewSessionDraft {
            workspace,
            title,
            worktree_mode: DraftWorktreeMode::Off,
            worktree_base: String::new(),
            worktree_path: String::new(),
            model: composer.model.clone(),
            effort: composer.effort,
            permission: composer.permission,
        });
        // The draft names the host the catalog lists from: request it
        // now while the catalog is still unresolved and no request is
        // in flight. A bare request would fail while no host is live,
        // so the shell never sends one.
        if !matches!(self.composer.catalog, composer::ModelCatalog::Ready(_))
            && !self.models_pending
            && self.connection == Connection::Connected
        {
            let workspace = self
                .new_session
                .as_ref()
                .map(|draft| draft.workspace.clone());
            if let (Some(workspace), Some(engine)) = (workspace, &self.engine) {
                engine.list_models(Some(workspace), None);
                self.models_pending = true;
            }
        }
    }

    /// Whether the id names a session in the current list.
    fn session_is_listed(&self, id: &SessionId) -> bool {
        self.sessions
            .groups
            .iter()
            .flat_map(|group| group.sessions.iter())
            .chain(self.sessions.unprojected.iter())
            .any(|summary| summary.id == *id)
    }

    /// Whether the session owns the visible transcript.
    fn is_visible(&self, session: &SessionId) -> bool {
        self.visible_session.as_ref() == Some(session)
    }

    /// The stored transcript for a background session, defaulting one.
    fn stored_transcript(&mut self, session: &SessionId) -> &mut transcript::State {
        self.transcripts.entry(session.clone()).or_default()
    }

    /// Recomputes whether the composer send button is live. The
    /// single writer of [`composer::State::send_enabled`] and
    /// [`composer::State::running_visible`] outside
    /// [`composer::State::new`]: send needs a server config, a live
    /// connection with a handle, no spawn in flight, and no resume in
    /// flight on the listed selection (when one is listed). A running
    /// selection no longer gates the button: Send becomes Steer there,
    /// and the Stop control appears.
    fn refresh_send_gate(&mut self) {
        let selected = self
            .sessions
            .selected_summary()
            .map(|summary| summary.id.clone());
        let running_visible = self.new_session.is_none()
            && selected
                .as_ref()
                .is_some_and(|id| self.running.contains(id));
        let gated_by_resume = !running_visible
            && selected
                .as_ref()
                .is_some_and(|id| self.resuming.contains(id));
        // A visible awaiting card blocks Send (the turn is paused
        // for input); Stop stays live through running_visible, which
        // this gate never clears.
        let blocked_by_pending = self
            .visible_session
            .as_ref()
            .is_some_and(|id| self.pending.has_awaiting(id));
        self.composer.pending_hint =
            blocked_by_pending.then(|| String::from("Answer the pending card to continue"));
        self.sessions
            .set_pending_counts(self.pending.pending_counts());
        self.composer.running_visible = running_visible;
        self.composer.send_enabled = self.engine_config.is_some()
            && self.connection == Connection::Connected
            && self.engine.is_some()
            && !self.awaiting_spawn
            && !gated_by_resume
            && !blocked_by_pending;
    }

    /// The transcript owning `session`: visible or parked.
    fn target_transcript(&mut self, session: &SessionId) -> &mut transcript::State {
        if self.is_visible(session) {
            &mut self.transcript
        } else {
            self.stored_transcript(session)
        }
    }

    /// Reconcile transcripts with a wholesale pending fetch: insert
    /// entries for new cards at their pause point, collapse entries
    /// whose cards vanished unsettled to the expired record.
    /// Returns whether the visible transcript changed (for scroll).
    fn apply_pending_loaded(&mut self, requests: Vec<manifest_domain::PendingRequest>) -> bool {
        let diff = self.pending.apply_loaded(requests);
        let mut visible_changed = false;
        for (session, request_id) in &diff.inserted {
            let visible = self.is_visible(session);
            if self
                .target_transcript(session)
                .push_pending(request_id.clone())
                && visible
            {
                visible_changed = true;
            }
        }
        for (session, request_id, line) in &diff.expired {
            let visible = self.is_visible(session);
            if self
                .target_transcript(session)
                .collapse_pending(request_id, line.clone())
                && visible
            {
                visible_changed = true;
            }
        }
        visible_changed
    }

    /// Collapse one settled card's entry to its record. Returns
    /// whether the visible transcript changed.
    fn apply_pending_settled(
        &mut self,
        session: SessionId,
        request_id: String,
        resolution: manifest_domain::PendingResolution,
    ) -> bool {
        let visible = self.is_visible(&session);
        match self
            .pending
            .apply_settled(&session, &request_id, resolution)
        {
            Some(line) => {
                self.target_transcript(&session)
                    .collapse_pending(&request_id, line)
                    && visible
            }
            None => false,
        }
    }

    /// Route one pending-card message through the reducer and engine.
    fn apply_pending(&mut self, child: pending::Message) {
        let effect = self.pending.update(child);
        self.run_pending_effect(effect);
        self.refresh_send_gate();
    }

    /// Perform a pending reducer effect against the engine.
    fn run_pending_effect(&self, effect: pending::Effect) {
        let Some(engine) = &self.engine else {
            return;
        };
        match effect {
            pending::Effect::None => {}
            pending::Effect::FetchPending => engine.fetch_pending(),
            pending::Effect::Decide {
                session,
                request_id,
                decision,
                reason,
            } => engine.decide(session, request_id, decision, reason),
            pending::Effect::Answer {
                session,
                request_id,
                answer,
            } => engine.answer(session, request_id, answer),
        }
    }

    /// Requests the skill catalog for the visible session when no
    /// request is in flight. Called on session-list loads and session
    /// selects, alongside the model-catalog request: the catalog
    /// belongs to the visible session, and re-selecting re-lists it.
    /// The single writer of [`App::skills_pending`] beside the event
    /// arms and the catalog-changed hint.
    fn request_skills_for_visible(&mut self) {
        if self.skills_pending || self.connection != Connection::Connected {
            return;
        }
        if let (Some(session), Some(engine)) = (self.visible_session.clone(), &self.engine) {
            engine.list_skills(session);
            self.skills_pending = true;
        }
    }

    /// Whether `session`'s transcript holds no host content: no
    /// entries, or only local error notes. Errors never stand in for
    /// history, so a failure still leaves the session refetchable.
    fn transcript_is_empty(&self, session: &SessionId) -> bool {
        let entries = if self.is_visible(session) {
            &self.transcript.entries
        } else {
            match self.transcripts.get(session) {
                Some(state) => &state.entries,
                None => return true,
            }
        };
        entries
            .iter()
            .all(|entry| matches!(entry.kind, transcript::EntryKind::Error { .. }))
    }

    /// Request `session`'s stored history once: skipped when already
    /// requested, when the session spawned this run (fresh
    /// transcripts need no backfill), or when its transcript already
    /// holds host content. Marks the session before sending so the
    /// loaded event cannot refetch.
    fn maybe_request_history(&mut self, session: &SessionId) {
        if self.history_requested.contains(session)
            || self.spawned_sessions.contains(session)
            || !self.transcript_is_empty(session)
        {
            return;
        }
        if self.connection != Connection::Connected {
            return;
        }
        if let Some(engine) = &self.engine {
            engine.load_history(session.clone());
            self.history_requested.insert(session.clone());
        }
    }

    /// Requests the model catalog for the selected session when the
    /// catalog is still loading and no request is in flight. Called on
    /// session-list loads: cold start has no host until a session (or a
    /// draft workspace) names one, so a bare request would fail. The
    /// single writer of [`App::models_pending`] beside the draft entry
    /// point and the event arms.
    fn request_models_for_selection(&mut self) {
        if !matches!(self.composer.catalog, composer::ModelCatalog::Loading)
            || self.models_pending
            || self.connection != Connection::Connected
        {
            return;
        }
        let selected = self
            .sessions
            .selected_summary()
            .map(|summary| summary.id.clone());
        if let (Some(session), Some(engine)) = (selected, &self.engine) {
            engine.list_models(None, Some(session));
            self.models_pending = true;
        }
    }

    /// Workspace the top bar reports a branch for: the draft's in
    /// new-session draft mode (the draft owns the surface then, as in
    /// [`App::topbar_project_name`]), else the selected session's,
    /// else none. Empty strings name no workspace.
    fn visible_workspace(&self) -> Option<String> {
        if let Some(draft) = &self.new_session
            && !draft.workspace.is_empty()
        {
            return Some(draft.workspace.clone());
        }
        if let Some(workspace) = self
            .sessions
            .selected_summary()
            .map(|summary| summary.workspace.as_str())
            && !workspace.is_empty()
        {
            return Some(workspace.to_string());
        }
        None
    }

    /// Reads `<workspace>/.git` off the reducer: a directory reads
    /// HEAD inside it, a `gitdir: <path>` pointer file (a worktree
    /// checkout) resolves through the domain helpers first. Any
    /// failure yields `None`: an expected display state, never an
    /// error the transcript shows.
    fn read_branch_blocking(workspace: &str) -> Option<String> {
        let dot_git = format!("{workspace}/.git");
        let git_dir = match std::fs::metadata(&dot_git) {
            Ok(meta) if meta.is_dir() => dot_git,
            Ok(_) => {
                let content = std::fs::read_to_string(&dot_git).ok()?;
                let pointer = manifest_domain::parse_gitdir_pointer(&content)?;
                manifest_domain::resolve_git_dir(workspace, pointer)
            }
            Err(_) => return None,
        };
        let head = std::fs::read_to_string(format!("{git_dir}/HEAD")).ok()?;
        manifest_domain::parse_head(&head)
    }

    /// Starts a branch read for one workspace, or no task for none.
    /// The result returns as [`Message::BranchLoaded`] carrying its
    /// workspace, so the reducer can drop stale reads.
    fn branch_task_for(workspace: Option<String>) -> Task<Message> {
        match workspace {
            Some(workspace) => Task::perform(
                async move {
                    let branch = Self::read_branch_blocking(&workspace);
                    Message::BranchLoaded { workspace, branch }
                },
                |message| message,
            ),
            None => Task::none(),
        }
    }

    /// Starts a branch read for the visible workspace, or no task
    /// when no workspace is visible.
    fn branch_task_for_visible(&self) -> Task<Message> {
        Self::branch_task_for(self.visible_workspace())
    }

    /// Folds a finished branch read into state. Applies only when the
    /// carried workspace still names the visible one; a stale read
    /// (the selection moved on while it ran) drops silently.
    fn apply_branch_loaded(&mut self, workspace: String, branch: Option<String>) {
        if self.visible_workspace().as_deref() == Some(workspace.as_str()) {
            self.branch = branch;
            self.branch_workspace = Some(workspace);
        }
    }

    /// The visible session's todo list, empty when no session is
    /// showing or the session has no list yet. The drawer renders from
    /// this, never from another session's list.
    fn visible_todos(&self) -> &[TodoItem] {
        self.visible_session
            .as_ref()
            .and_then(|session| self.todo_lists.get(session))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Recomputes the drawer bar strings from the visible session's
    /// list (UI contract §2: the view formats nothing). No visible
    /// list shows "0 of 0 done" with no current todo, which hides the
    /// drawer; a finished list leaves no current todo, which the view
    /// renders as "All done".
    fn sync_visible_todos(&mut self) {
        let (done, total, current) = match self
            .visible_session
            .as_ref()
            .and_then(|session| self.todo_lists.get(session))
        {
            Some(todos) => {
                let done = todos.iter().filter(|todo| todo.status.is_done()).count();
                let current = current_todo_label(todos);
                (done, todos.len(), current)
            }
            None => (0, 0, None),
        };
        self.todo_summary = format!("{done} of {total} done");
        self.current_todo = current;
    }

    /// Folds a todo-carrying stream item into per-session state: a
    /// `Some` list replaces the session's list wholesale (started may
    /// set it earlier; completed is authoritative), and the visible
    /// projection refreshes when the session is visible. Items without
    /// todos leave every list untouched.
    fn apply_todo_item(&mut self, session: &SessionId, item: &StreamItem) {
        if let StreamItem::ToolCall {
            todos: Some(todos), ..
        } = item
        {
            self.todo_lists.insert(session.clone(), todos.clone());
            if self.is_visible(session) {
                self.sync_visible_todos();
            }
        }
    }

    /// The visible session's goal, when the store holds one. A
    /// missing entry and a stored clear both read as no goal.
    fn visible_goal(&self) -> Option<&GoalInfo> {
        self.visible_session
            .as_ref()
            .and_then(|session| self.goals.get(session))
            .and_then(|goal| goal.as_ref())
    }

    /// Points the composer's goal strip at the visible session's
    /// stored goal. Called after goal events and transcript syncs;
    /// the composer precomputes the strip strings, so the view reads
    /// without formatting.
    fn sync_composer_goal(&mut self) {
        let goal = self.visible_goal().cloned();
        self.composer.set_goal(goal.as_ref());
    }

    /// Points the composer's usage meter at the visible session's
    /// snapshot. Called after usage events, status reconciles, and
    /// transcript syncs; the composer stores the snapshot, so the
    /// views format nothing. The top bar carries no usage text: the
    /// composer's ring and popover are the only meter surfaces.
    fn sync_composer_usage(&mut self) {
        let snapshot = self
            .visible_session
            .as_ref()
            .and_then(|session| self.usage.get(session))
            .cloned();
        self.composer.set_usage(snapshot);
    }

    /// Project name for the top bar's left cluster: the draft's
    /// project while a draft is open (the group whose workspace
    /// matches the draft's, else the draft workspace's short name),
    /// else the selected session's project group name, else the
    /// selected workspace's short name, else nothing. A lookup over
    /// stored state, so the view reads without formatting.
    fn topbar_project_name(&self) -> Option<String> {
        if let Some(draft) = &self.new_session {
            if let Some(group) =
                self.sessions.groups.iter().find(|group| {
                    group.project.workspace.as_deref() == Some(draft.workspace.as_str())
                })
            {
                return Some(group.project.name.clone());
            }
            let name = manifest_domain::workspace_name(&draft.workspace);
            return (!name.is_empty()).then(|| name.to_string());
        }
        let summary = self.sessions.selected_summary()?;
        if let Some(project) = summary.project.as_ref()
            && let Some(group) = self
                .sessions
                .groups
                .iter()
                .find(|group| &group.project.id == project)
        {
            return Some(group.project.name.clone());
        }
        let name = manifest_domain::workspace_name(&summary.workspace);
        (!name.is_empty()).then(|| name.to_string())
    }

    /// Sessions in display order: grouped rows then unprojected, cloned.
    /// The mutation base for registry updates and reseeding.
    fn current_sessions(&self) -> Vec<SessionSummary> {
        self.sessions
            .groups
            .iter()
            .flat_map(|group| group.sessions.iter().cloned())
            .chain(self.sessions.unprojected.iter().cloned())
            .collect()
    }

    /// The project list mutations start from: the authoritative registry
    /// once loaded, else a fresh seed from the listed sessions (which
    /// also recovers after a failed load).
    fn registry_projects(&self) -> Vec<ProjectSummary> {
        if self.projects_ready {
            self.projects.clone()
        } else {
            manifest_domain::seed_projects(&self.current_sessions())
        }
    }

    /// Saves the current registry list to disk. Callers set `projects`
    /// (and `projects_ready`) to the new list before creating the task,
    /// so the task saves the new list. No store means no task.
    fn save_projects_task(&self) -> Task<Message> {
        let Some(store) = &self.project_store else {
            return Task::none();
        };
        let path = store.path().to_path_buf();
        let list = self.projects.clone();
        Task::perform(
            async move { manifest_store::ProjectStore::new(path).save(&list) },
            |result| Message::ProjectsSaved(result.map_err(|error| error.to_string())),
        )
    }

    /// Loads the persisted registry; the result arrives as
    /// [`Message::ProjectsLoaded`]. No store means no task.
    pub(crate) fn project_load_task(&self) -> Task<Message> {
        let Some(store) = &self.project_store else {
            return Task::none();
        };
        let path = store.path().to_path_buf();
        Task::perform(
            async move { manifest_store::ProjectStore::new(path).load() },
            |result| Message::ProjectsLoaded(result.map_err(|error| error.to_string())),
        )
    }

    /// Folds a registry load into state: replaces the registry, regroups
    /// the listed sessions under it, and keeps the visible transcript on
    /// the selection. Returns a save task when a `Missing` load seeds
    /// from listed sessions, else no task.
    fn apply_projects_loaded(
        &mut self,
        result: Result<manifest_store::Loaded, String>,
    ) -> Task<Message> {
        match result {
            Ok(manifest_store::Loaded::Present(list)) => {
                self.projects = list;
                self.projects_ready = true;
                self.projects_missing = false;
                self.regroup_under_registry();
                self.refresh_send_gate();
                Task::none()
            }
            Ok(manifest_store::Loaded::Missing) => {
                if self.current_sessions().is_empty() {
                    self.projects = Vec::new();
                    self.projects_ready = true;
                    self.projects_missing = true;
                    self.refresh_send_gate();
                    Task::none()
                } else {
                    self.projects = manifest_domain::seed_projects(&self.current_sessions());
                    self.projects_ready = true;
                    self.projects_missing = false;
                    self.regroup_under_registry();
                    self.refresh_send_gate();
                    self.save_projects_task()
                }
            }
            Err(message) => {
                let appended = self.transcript.push_error(message);
                self.refresh_send_gate();
                scroll_transcript_to_end(appended)
            }
        }
    }

    /// Rebuilds the list groups from the authoritative registry: assigns
    /// the listed sessions under `projects` and replaces the list state,
    /// then keeps the visible transcript on the selection.
    fn regroup_under_registry(&mut self) {
        let current = self.current_sessions();
        let assigned = manifest_domain::assign_projects(&self.projects, current);
        let projects = self.projects.clone();
        self.sessions.replace(projects, assigned);
        self.sync_visible_transcript();
    }

    /// Handles the intercepted add-project submit: fixture (or any store
    /// less) mode closes the popup and registers nothing; live mode
    /// validates the form asynchronously and routes the outcome back as
    /// [`Message::AddProjectChecked`], leaving the popup open meanwhile.
    fn submit_add_project(&mut self) -> Task<Message> {
        let (directory, name) = match &self.sessions.add_project {
            sessions::AddProject::Open {
                directory, name, ..
            } => (directory.clone(), name.clone()),
            sessions::AddProject::Closed => return Task::none(),
        };
        if self.project_store.is_none() {
            self.sessions.update(sessions::Message::CloseAddProject);
            return Task::none();
        }
        if directory.trim().is_empty() || name.trim().is_empty() {
            self.sessions
                .update(sessions::Message::AddProjectFailed(String::from(
                    "Choose a project directory and name.",
                )));
            return Task::none();
        }
        Task::perform(
            async move { validate_project_dir(directory, name) },
            Message::AddProjectChecked,
        )
    }

    /// Folds a finished add-project validation into state. A known
    /// workspace expands its existing group; a new one appends to the
    /// registry, regroups, expands, and saves. A closed popup still
    /// registers the project; only the close and expand steps no-op.
    fn apply_add_project_checked(
        &mut self,
        checked: Result<(String, String), String>,
    ) -> Task<Message> {
        let (directory, name) = match checked {
            Ok(valid) => valid,
            Err(message) => {
                self.sessions
                    .update(sessions::Message::AddProjectFailed(message));
                return Task::none();
            }
        };
        let list = self.registry_projects();
        if let Some(existing) = list
            .iter()
            .find(|project| {
                project
                    .workspace
                    .as_deref()
                    .is_some_and(|workspace| workspace.trim() == directory.trim())
            })
            .map(|project| project.id.clone())
        {
            self.sessions.update(sessions::Message::CloseAddProject);
            let assigned = manifest_domain::assign_projects(&list, self.current_sessions());
            self.sessions.replace(list, assigned);
            self.sessions.expanded.insert(existing);
            return Task::none();
        }
        let id = ProjectId::for_workspace(&directory);
        let mut projects = list;
        projects.push(ProjectSummary {
            id: id.clone(),
            name,
            workspace: Some(directory),
        });
        self.projects = projects;
        self.projects_ready = true;
        self.projects_missing = false;
        let assigned = manifest_domain::assign_projects(&self.projects, self.current_sessions());
        let snapshot = self.projects.clone();
        self.sessions.replace(snapshot, assigned);
        self.sessions.expanded.insert(id);
        self.sessions.update(sessions::Message::CloseAddProject);
        self.save_projects_task()
    }

    /// Handles the intercepted project removal: the child moves the rows
    /// first, then the live path drops the project from the registry
    /// (matching by id, and by workspace so a not-ready seed cannot
    /// bring it back), cancels a draft in its workspace, regroups to
    /// recompute row ownership, and saves. Fixture mode keeps the child
    /// regroup only.
    fn remove_project(&mut self, id: ProjectId) -> Task<Message> {
        let removed_workspace = self
            .sessions
            .groups
            .iter()
            .find(|group| group.project.id == id)
            .and_then(|group| group.project.workspace.clone());
        self.sessions
            .update(sessions::Message::RemoveProject(id.clone()));
        if self.project_store.is_none() {
            return Task::none();
        }
        let mut list = self.registry_projects();
        let before = list.len();
        list.retain(|project| project.id != id);
        if let Some(workspace) = &removed_workspace {
            list.retain(|project| project.workspace.as_deref() != Some(workspace.as_str()));
        }
        if list.len() == before {
            return Task::none();
        }
        self.projects = list;
        self.projects_ready = true;
        self.projects_missing = false;
        if self
            .new_session
            .as_ref()
            .is_some_and(|draft| Some(draft.workspace.as_str()) == removed_workspace.as_deref())
        {
            self.new_session = None;
        }
        let assigned = manifest_domain::assign_projects(&self.projects, self.current_sessions());
        let snapshot = self.projects.clone();
        self.sessions.replace(snapshot, assigned);
        self.sync_visible_transcript();
        self.refresh_send_gate();
        self.save_projects_task()
    }

    /// Spawns a session in the given workspace with the draft input:
    /// the shared tail of the draft spawn and the no-selection cwd
    /// fallback. Echoes the input optimistically, clears the draft,
    /// and marks the spawn in flight until `SessionSpawned` adopts it.
    /// Keeps the taken draft (when the spawn came from one) and the
    /// prompt so a failure can restore them for retry.
    fn spawn_session(
        &mut self,
        draft: Option<NewSessionDraft>,
        workspace: String,
        input: manifest_domain::TurnInput,
        choices: SpawnChoices,
    ) -> Task<Message> {
        if let Some(engine) = self.engine.clone() {
            engine.spawn_session(workspace, input.clone(), choices);
        }
        let prompt = input.display_text();
        self.pending_spawn = Some(PendingSpawn {
            draft,
            prompt: prompt.clone(),
        });
        self.awaiting_spawn = true;
        self.transcript.push_user(prompt);
        self.composer.clear();
        transcript::scroll_to_end().map(Message::Transcript)
    }

    /// Routes the composer send button press. In order: fixture mode
    /// is a no-op with the draft untouched; a blank draft sends
    /// nothing; while not connected the visible transcript gains a
    /// friendly note and the draft stays; a slash draft folds through
    /// [`manifest_domain::parse_slash_draft`] first (builtins open the
    /// model menu, compact the session, or set its goal instead of
    /// sending; a ready catalog skill sends as a skill turn); a
    /// new-session draft spawns into its project workspace; a listed
    /// selection sends a follow-up turn unless one is already in
    /// flight (steering the running turn there instead, always as
    /// plain text); otherwise the draft spawns a session in the
    /// process working directory with the shown picker state. Every
    /// send carries the composed prompt: the draft text with attached
    /// path refs appended as ref lines.
    /// Sends echo the prompt optimistically, clear the draft, and
    /// snap the transcript to the end.
    fn send_from_composer(&mut self) -> Task<Message> {
        if self.engine_config.is_none() {
            return Task::none();
        }
        if self.composer.content.text().trim().is_empty() {
            return Task::none();
        }
        if self.connection != Connection::Connected || self.engine.is_none() {
            let appended = self.transcript.push_error(not_connected_message());
            return scroll_transcript_to_end(appended);
        }
        // Slash behavior resolves before spawn/turn/steer routing.
        let draft = self.composer.content.text();
        let skills: Vec<manifest_domain::SkillInfo> = match &self.composer.skill_catalog {
            composer::SkillCatalog::Ready(skills) => skills.clone(),
            composer::SkillCatalog::Loading | composer::SkillCatalog::Failed(_) => Vec::new(),
        };
        match manifest_domain::parse_slash_draft(&draft, &skills) {
            manifest_domain::DraftAction::Text => {}
            manifest_domain::DraftAction::Skill(invocation) => {
                return self.send_turn_input(skill_input_with_refs(
                    invocation,
                    &self.composer.attachments,
                ));
            }
            manifest_domain::DraftAction::OpenModelMenu => {
                self.composer.model_menu = composer::ModelMenu::Open;
                return Task::none();
            }
            manifest_domain::DraftAction::Compact => {
                return self.dispatch_compact();
            }
            manifest_domain::DraftAction::SetGoal(objective) => {
                return self.dispatch_goal(objective);
            }
            manifest_domain::DraftAction::KeepDraft => {
                // `/goal` with no objective: send nothing, keep the
                // draft so the objective can be typed.
                return Task::none();
            }
        }
        let prompt = self.composer.composed_prompt();
        self.send_turn_input(manifest_domain::TurnInput::Text(prompt))
    }

    /// Sends one resolved turn input through the spawn/turn/steer
    /// routing: a new-session draft spawns into its project workspace;
    /// a listed selection sends a follow-up turn unless one is already
    /// in flight (steering the running turn there instead, always as
    /// the composed plain-text prompt); otherwise the input spawns a
    /// session in the process working directory with the shown picker
    /// state.
    fn send_turn_input(&mut self, input: manifest_domain::TurnInput) -> Task<Message> {
        if self.new_session.is_some() {
            let choices = self
                .new_session
                .as_ref()
                .expect("draft checked above")
                .choices();
            if matches!(&choices.worktree, WorktreeChoice::Existing { path } if path.is_empty()) {
                let appended = self.transcript.push_error(String::from(
                    "Choose an existing worktree folder, or switch the worktree choice off.",
                ));
                return scroll_transcript_to_end(appended);
            }
            let draft = self.new_session.take().expect("draft checked above");
            let workspace = draft.workspace.clone();
            return self.spawn_session(Some(draft), workspace, input, choices);
        }
        if let Some(summary) = self.sessions.selected_summary() {
            let session = summary.id.clone();
            if self.running.contains(&session) {
                // Steer: the visible session is mid-turn, so the draft
                // joins it instead of starting a new one. Steers stay
                // plain text, even for slash drafts.
                let prompt = self.composer.composed_prompt();
                let Some(turn) = self.running_turns.get(&session).cloned() else {
                    let appended = self.transcript.push_error(no_running_turn_message());
                    return scroll_transcript_to_end(appended);
                };
                if let Some(engine) = self.engine.clone() {
                    engine.steer_session(session, prompt.clone(), turn);
                }
                self.transcript.push_user(prompt);
                self.composer.clear();
                return transcript::scroll_to_end().map(Message::Transcript);
            }
            if let Some(engine) = self.engine.clone() {
                engine.send_turn(session.clone(), input.clone());
            }
            self.transcript.push_user(input.display_text());
            self.running.insert(session);
            self.composer.clear();
            return transcript::scroll_to_end().map(Message::Transcript);
        }
        let workspace = match std::env::current_dir() {
            Ok(path) => path.display().to_string(),
            Err(_) => {
                let appended = self.transcript.push_error(String::from(
                    "Could not determine the folder to start the session in.",
                ));
                return scroll_transcript_to_end(appended);
            }
        };
        // No listed selection: spawn in the working directory, but with
        // the shown picker state, not the server defaults.
        let choices = SpawnChoices {
            worktree: WorktreeChoice::Off,
            model: self.composer.model.clone(),
            effort: self.composer.effort,
            permission: self.composer.permission,
        };
        self.spawn_session(None, workspace, input, choices)
    }

    /// Compacts the visible session (`/compact` follow-through).
    /// Without a visible session the draft is kept and the transcript
    /// explains; otherwise the engine compacts and the draft clears
    /// like a send. Failures arrive as `CommandFailed(Compact)` on the
    /// transcript channel every command failure already uses.
    fn dispatch_compact(&mut self) -> Task<Message> {
        let Some(session) = self.visible_session.clone() else {
            let appended = self
                .transcript
                .push_error(String::from("There is no visible session to compact yet."));
            return scroll_transcript_to_end(appended);
        };
        if let Some(engine) = self.engine.clone() {
            engine.compact_session(session);
        }
        self.composer.clear();
        Task::none()
    }

    /// Sets the visible session's goal (`/goal` follow-through).
    /// Without a visible session the draft is kept and the transcript
    /// explains; otherwise the engine sets the goal and the draft
    /// clears like a send. Failures arrive as `CommandFailed(Goal)` on
    /// the transcript channel every command failure already uses.
    fn dispatch_goal(&mut self, objective: String) -> Task<Message> {
        let Some(session) = self.visible_session.clone() else {
            let appended = self.transcript.push_error(String::from(
                "There is no visible session to set a goal for yet.",
            ));
            return scroll_transcript_to_end(appended);
        };
        if let Some(engine) = self.engine.clone() {
            engine.set_goal(session, objective);
        }
        self.composer.clear();
        Task::none()
    }

    /// Stops the running turn of the selected session: cancels it on
    /// the engine with the tracked turn id. A no-op without a running
    /// selection; the Stop control only shows while one runs.
    fn stop_running_turn(&mut self) -> Task<Message> {
        if self.new_session.is_some() {
            return Task::none();
        }
        let Some(summary) = self.sessions.selected_summary() else {
            return Task::none();
        };
        if !self.running.contains(&summary.id) {
            return Task::none();
        }
        if let Some(engine) = self.engine.clone() {
            engine.cancel_turn(
                summary.id.clone(),
                self.running_turns.get(&summary.id).cloned(),
            );
        }
        Task::none()
    }

    /// Folds one stream event into the owning session's transcript:
    /// the visible state when the event names it, else the stored one.
    /// Returns whether the visible transcript gained an entry, so the
    /// scroll can snap to the end. Deltas never scroll. Turn lifecycles
    /// track the running turn id; usage reports refresh the meter.
    fn apply_stream_event(&mut self, event: StreamEvent) -> bool {
        match event {
            StreamEvent::TurnStarted { session, turn, .. } => {
                self.running.insert(session.clone());
                if let Some(turn) = turn {
                    self.running_turns.insert(session, turn);
                }
                self.refresh_send_gate();
                false
            }
            StreamEvent::ItemStarted { session, item, .. } => {
                self.apply_todo_item(&session, &item);
                if self.is_visible(&session) {
                    self.transcript.item_started(item);
                    true
                } else {
                    self.stored_transcript(&session).item_started(item);
                    false
                }
            }
            StreamEvent::ItemDelta {
                session,
                item_id,
                field,
                delta,
                ..
            } => {
                if self.is_visible(&session) {
                    self.transcript.item_delta(&item_id, &field, &delta);
                } else {
                    self.stored_transcript(&session)
                        .item_delta(&item_id, &field, &delta);
                }
                false
            }
            StreamEvent::ItemCompleted { session, item, .. } => {
                self.apply_todo_item(&session, &item);
                if self.is_visible(&session) {
                    self.transcript.item_completed(item);
                    true
                } else {
                    self.stored_transcript(&session).item_completed(item);
                    false
                }
            }
            StreamEvent::TurnCompleted {
                session,
                dropped_deltas,
                ..
            } => {
                self.running.remove(&session);
                self.running_turns.remove(&session);
                self.refresh_send_gate();
                let mut appended = false;
                if dropped_deltas > 0 {
                    let message = dropped_deltas_message(dropped_deltas);
                    if self.is_visible(&session) {
                        appended = self.transcript.push_error(message);
                    } else {
                        appended = self.stored_transcript(&session).push_error(message);
                    }
                }
                if self.connection == Connection::Connected
                    && let Some(engine) = &self.engine
                {
                    engine.list_sessions();
                }
                appended
            }
            StreamEvent::Usage {
                session,
                usage_json,
                ..
            } => {
                // The meter reads the parse, never raw JSON: unknown
                // payloads leave the previous snapshot in place.
                if let Some(snapshot) = UsageSnapshot::parse(&usage_json) {
                    self.usage.insert(session.clone(), snapshot);
                    if self.is_visible(&session) {
                        self.sync_composer_usage();
                    }
                }
                false
            }
            StreamEvent::Pending {
                session,
                request_id,
                kind,
                phase,
                ..
            } => {
                // Hint only: the fetch is the source of truth, for
                // both Requested and Settled hints. A Requested hint
                // is remembered so a failed fetch can still name
                // the request with a placeholder card; Settled
                // hints never add.
                if matches!(phase, manifest_domain::PendingPhase::Requested) {
                    self.pending.record_hint(session, request_id, kind);
                }
                if self.connection == Connection::Connected
                    && let Some(engine) = &self.engine
                {
                    engine.fetch_pending();
                }
                false
            }
            StreamEvent::CatalogChanged { session } => {
                // Hint only: the list is the source of truth. Re-list
                // the visible session's catalog when it reports a
                // change; other sessions' hints are ignored, and a
                // request already in flight is never doubled.
                if self.is_visible(&session)
                    && !self.skills_pending
                    && self.connection == Connection::Connected
                    && let Some(engine) = &self.engine
                {
                    engine.list_skills(session);
                    self.skills_pending = true;
                }
                false
            }
            StreamEvent::GoalChanged { session, goal } => {
                // Source of truth, not a hint: replaces the session's
                // stored goal wholesale (`None` clears it). Only the
                // visible session's strip refreshes; background
                // sessions' goals wait for their select sync.
                self.goals.insert(session.clone(), goal);
                if self.is_visible(&session) {
                    self.sync_composer_goal();
                }
                false
            }
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // The App owns the OS picker, the async validation, and the
            // registry: these child messages never reach the child
            // reducer directly.
            Message::Sessions(sessions::Message::PickDirectory) => {
                if self.engine_config.is_some() {
                    Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .pick_folder()
                                .await
                                .map(|handle| handle.path().display().to_string())
                        },
                        Message::DirectoryPicked,
                    )
                } else {
                    Task::none()
                }
            }
            Message::Sessions(sessions::Message::SubmitAddProject) => self.submit_add_project(),
            Message::Sessions(sessions::Message::RemoveProject(id)) => self.remove_project(id),
            Message::Sessions(child) => {
                let select = matches!(child, sessions::Message::Select(_));
                let draft_project = match &child {
                    sessions::Message::NewSession(project) => Some(project.clone()),
                    _ => None,
                };
                self.sessions.update(child);
                if let Some(project) = draft_project {
                    self.enter_new_session_draft(&project);
                } else if select {
                    // Leaving the draft restores the selected session's
                    // parked transcript through the usual sync below.
                    self.new_session = None;
                }
                self.sync_visible_transcript();
                if select && let Some(summary) = self.sessions.selected_summary() {
                    let id = summary.id.clone();
                    let status = summary.status.clone();
                    if status == SessionStatus::Orphaned
                        && self.engine_config.is_some()
                        && self.connection == Connection::Connected
                        && !self.resuming.contains(&id)
                        && let Some(engine) = &self.engine
                    {
                        engine.resume_session(id.clone());
                        self.resuming.insert(id.clone());
                    }
                    // Reopening a session backfills its stored
                    // transcript once; new spawns never fetch.
                    self.maybe_request_history(&id);
                }
                if select {
                    // The sync above moved the visible transcript: the
                    // composer's catalog belongs to the new session.
                    self.request_skills_for_visible();
                }
                self.refresh_send_gate();
                if select {
                    self.branch_task_for_visible()
                } else {
                    Task::none()
                }
            }
            Message::DraftWorktreeModeSelected(mode) => {
                if let Some(draft) = &mut self.new_session {
                    draft.worktree_mode = mode;
                }
                self.refresh_send_gate();
                Task::none()
            }
            Message::DraftWorktreeBaseChanged(base) => {
                if let Some(draft) = &mut self.new_session {
                    draft.worktree_base = base;
                }
                self.refresh_send_gate();
                Task::none()
            }
            Message::DraftWorktreePathChanged(path) => {
                if let Some(draft) = &mut self.new_session {
                    draft.worktree_path = path;
                }
                self.refresh_send_gate();
                Task::none()
            }
            Message::ProjectsLoaded(result) => self.apply_projects_loaded(result),
            Message::ProjectsSaved(result) => match result {
                Ok(()) => Task::none(),
                Err(message) => {
                    let appended = self.transcript.push_error(message);
                    self.refresh_send_gate();
                    scroll_transcript_to_end(appended)
                }
            },
            Message::DirectoryPicked(picked) => {
                if let Some(path) = picked {
                    self.sessions
                        .update(sessions::Message::DirectoryChanged(path));
                }
                Task::none()
            }
            Message::AddProjectChecked(checked) => self.apply_add_project_checked(checked),
            Message::BranchLoaded { workspace, branch } => {
                self.apply_branch_loaded(workspace, branch);
                Task::none()
            }
            Message::Composer(composer::Message::SendPressed) => {
                let task = self.send_from_composer();
                self.refresh_send_gate();
                task
            }
            Message::Composer(composer::Message::StopPressed) => {
                let task = self.stop_running_turn();
                self.refresh_send_gate();
                task
            }
            Message::Composer(composer::Message::AttachFilesPressed) => {
                if self.engine_config.is_some() {
                    Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .pick_file()
                                .await
                                .map(|handle| handle.path().display().to_string())
                        },
                        |picked| Message::Composer(composer::Message::AttachPicked(picked)),
                    )
                } else {
                    Task::none()
                }
            }
            Message::Composer(composer::Message::AttachPicked(picked)) => {
                if let Some(path) = picked {
                    self.composer.insert_attachment(path);
                } else {
                    self.composer.attach_popup = composer::AttachPopup::Closed;
                }
                self.refresh_send_gate();
                Task::none()
            }
            Message::Composer(child) => {
                // The menus edit the draft's spawn choices while one is
                // open, so the first send spawns with the shown picks.
                let syncs_draft = matches!(
                    child,
                    composer::Message::PermissionSelected(_)
                        | composer::Message::ModelSelected(_)
                        | composer::Message::ModelEditPressed(_)
                        | composer::Message::EffortSelected(_)
                );
                self.composer.update(child);
                if syncs_draft && let Some(draft) = &mut self.new_session {
                    draft.model = self.composer.model.clone();
                    draft.effort = self.composer.effort;
                    draft.permission = self.composer.permission;
                }
                self.refresh_send_gate();
                Task::none()
            }
            Message::Pending(child) => {
                self.apply_pending(child);
                Task::none()
            }
            Message::SubmitPendingShortcut => {
                if let Some(request) = self
                    .visible_session
                    .as_ref()
                    .and_then(|id| self.pending.first_awaiting_question(id))
                {
                    let effect = self
                        .pending
                        .update(pending::Message::SubmitAnswers { request });
                    self.run_pending_effect(effect);
                }
                self.refresh_send_gate();
                Task::none()
            }
            Message::Transcript(child) => match child {
                // Clipboard writes are tasks, which state cannot produce.
                transcript::Message::Copy(text) => iced::clipboard::write(text),
                transcript::Message::Pending(child) => {
                    self.apply_pending(child);
                    Task::none()
                }
                other => {
                    self.transcript.update(other);
                    Task::none()
                }
            },
            Message::ToggleTodoDrawer => {
                self.todo_drawer = match self.todo_drawer {
                    TodoDrawer::Peek => TodoDrawer::Open,
                    TodoDrawer::Open => TodoDrawer::Peek,
                };
                Task::none()
            }
            Message::PanelDragStarted => {
                self.panel_drag = PanelDrag::Dragging { offset: None };
                Task::none()
            }
            Message::PanelDragMoved(point) => match self.panel_drag {
                PanelDrag::Dragging { offset } => {
                    let grab = offset.unwrap_or(point.x - self.panel_width);
                    self.panel_drag = PanelDrag::Dragging { offset: Some(grab) };
                    self.panel_width = (point.x - grab).clamp(PANEL_WIDTH_MIN, PANEL_WIDTH_MAX);
                    Task::none()
                }
                PanelDrag::Idle => Task::none(),
            },
            Message::PanelDragEnded => {
                self.panel_drag = PanelDrag::Idle;
                Task::none()
            }
            Message::ResizeWindow(direction) => match self.window_id {
                Some(id) => iced::window::drag_resize(id, direction),
                None => Task::none(),
            },
            Message::WindowIdentified(id) => {
                self.window_id = id;
                match id {
                    Some(id) => iced::window::is_maximized(id).map(Message::WindowMaximizedChecked),
                    None => Task::none(),
                }
            }
            Message::WindowResized => match self.window_id {
                Some(id) => iced::window::is_maximized(id).map(Message::WindowMaximizedChecked),
                None => Task::none(),
            },
            Message::WindowMaximizedChecked(maximized) => {
                self.window_maximized = maximized;
                Task::none()
            }
            Message::DragWindow => match self.window_id {
                Some(id) => iced::window::drag(id),
                None => Task::none(),
            },
            Message::MinimizeWindow => match self.window_id {
                Some(id) => iced::window::minimize(id, true),
                None => Task::none(),
            },
            Message::ToggleMaximizeWindow => match self.window_id {
                Some(id) => iced::window::toggle_maximize(id),
                None => Task::none(),
            },
            Message::CloseWindow => match self.window_id {
                Some(id) => iced::window::close(id),
                None => Task::none(),
            },
            Message::Engine { attempt, input } => {
                if attempt != self.connection_attempt || self.engine_config.is_none() {
                    return Task::none();
                }
                let mut appended = false;
                let mut extra_task: Task<Message> = Task::none();
                match input {
                    engine::Input::Ready(handle) => {
                        self.engine = Some(handle);
                    }
                    // The event rides boxed so the hot message enum stays
                    // small; unwrap it before matching.
                    engine::Input::Event(event) => match *event {
                        EngineEvent::ConnectionChanged(next) => {
                            if let Connection::Failed(message) = &next {
                                appended = self.transcript.push_error(message.clone());
                            }
                            self.connection = next;
                            if self.connection == Connection::Connected {
                                if let Some(engine) = &self.engine {
                                    // The catalog request needs a host
                                    // context, which only a loaded
                                    // session (or draft workspace)
                                    // names: it follows the list load,
                                    // never the bare connection.
                                    engine.list_sessions();
                                }
                            } else if matches!(self.connection, Connection::Failed(_)) {
                                self.engine = None;
                                self.running.clear();
                                self.running_turns.clear();
                            }
                            if self.connection != Connection::Connected {
                                // An in-flight catalog request dies with
                                // the connection; the next list load
                                // re-requests it with fresh context.
                                self.models_pending = false;
                                self.skills_pending = false;
                            }
                        }
                        EngineEvent::SessionsLoaded { sessions, usage } => {
                            if self.project_store.is_some() && self.projects_missing {
                                let seeded = manifest_domain::seed_projects(&sessions);
                                if !seeded.is_empty() {
                                    self.projects = seeded;
                                    self.projects_missing = false;
                                    self.projects_ready = true;
                                    extra_task = self.save_projects_task();
                                }
                            }
                            let projects = if self.projects_ready {
                                self.projects.clone()
                            } else {
                                manifest_domain::seed_projects(&sessions)
                            };
                            let sessions = manifest_domain::assign_projects(&projects, sessions);
                            self.resuming.retain(|id| {
                                sessions
                                    .iter()
                                    .any(|s| &s.id == id && s.status == SessionStatus::Orphaned)
                            });
                            self.sessions.replace(projects, sessions);
                            if let Some(spawned) = self.select_on_load.clone()
                                && self.session_is_listed(&spawned)
                            {
                                self.sessions.update(sessions::Message::Select(spawned));
                                self.select_on_load = None;
                                // The direct update above bypasses the
                                // select arm, so the fresh selection's
                                // catalog is listed here (usually a
                                // no-op: the adoption already asked).
                                self.request_skills_for_visible();
                            }
                            self.sync_visible_transcript();
                            // A list load can leave a dormant session
                            // visible by default (first row after
                            // replace, no Select and no resume): the
                            // select arm's backfill never runs for it,
                            // so backfill here too. The request gates
                            // on already-requested, spawned-this-run,
                            // and stored content, so refreshes and the
                            // select_on_load adoption above never
                            // double-fetch; only a History failure
                            // releases the gate for a retry.
                            if let Some(visible) = self.visible_session.clone() {
                                self.maybe_request_history(&visible);
                            }
                            // Usage reconciles against the same
                            // cumulative fold the stream carries: a
                            // carried snapshot replaces the stored one
                            // for its session, and sessions absent from
                            // the carried vec keep theirs. Snapshots for
                            // unlisted sessions still drop with their
                            // rows.
                            for (id, snapshot) in usage {
                                self.usage.insert(id, snapshot);
                            }
                            let listed: std::collections::HashSet<SessionId> =
                                self.current_sessions().into_iter().map(|s| s.id).collect();
                            self.usage.retain(|id, _| listed.contains(id));
                            self.sync_composer_usage();
                            // Cold start names no host until a session is
                            // listed: request the catalog now, with the
                            // selected session as its context.
                            self.request_models_for_selection();
                            // The skill catalog follows the visible
                            // session, which the sync above just set.
                            // Only while it never loaded: routine list
                            // refreshes must not re-list a ready
                            // catalog, and navigation re-lists through
                            // the select arm below.
                            if matches!(
                                self.composer.skill_catalog,
                                composer::SkillCatalog::Loading
                            ) {
                                self.request_skills_for_visible();
                            }
                        }
                        EngineEvent::SessionsLoadFailed(message) => {
                            appended = self.transcript.push_error(message);
                        }
                        EngineEvent::ModelsLoaded { models } => {
                            self.models_pending = false;
                            self.composer
                                .set_catalog(composer::ModelCatalog::Ready(models));
                        }
                        EngineEvent::SkillsLoaded { session, skills } => {
                            // The catalog belongs to the session it was
                            // listed for: only the visible session's
                            // load lands in the composer. A background
                            // session's load is ignored (re-listed on
                            // select), so it never clobbers the
                            // visible rows.
                            if self.is_visible(&session) {
                                self.skills_pending = false;
                                self.composer
                                    .set_skill_catalog(composer::SkillCatalog::Ready(skills));
                            }
                        }
                        EngineEvent::CommandFailed { command, message } => {
                            // Every failure shows in the transcript;
                            // state is consumed only by its own command.
                            appended = self.transcript.push_error(message.clone());
                            match command {
                                manifest_domain::FailedCommand::Models => {
                                    if self.models_pending {
                                        self.models_pending = false;
                                        self.composer
                                            .set_catalog(composer::ModelCatalog::Failed(message));
                                    }
                                }
                                manifest_domain::FailedCommand::Spawn => {
                                    if self.awaiting_spawn
                                        && let Some(pending) = self.pending_spawn.take()
                                    {
                                        if let Some(draft) = pending.draft {
                                            self.new_session = Some(draft);
                                        }
                                        self.composer.restore(pending.prompt);
                                    }
                                    self.awaiting_spawn = false;
                                }
                                manifest_domain::FailedCommand::Resume(session) => {
                                    // Only this session's gate releases,
                                    // so the next select retries it.
                                    self.resuming.remove(&session);
                                }
                                manifest_domain::FailedCommand::History(session) => {
                                    // Only this session's history gate
                                    // releases, so the next select
                                    // retries it. The transcript line
                                    // above carries the failure; the
                                    // session stays sendable.
                                    self.history_requested.remove(&session);
                                }
                                manifest_domain::FailedCommand::Turn(session) => {
                                    // The optimistically started turn
                                    // never began: end it for this
                                    // session only.
                                    self.running.remove(&session);
                                    self.running_turns.remove(&session);
                                }
                                manifest_domain::FailedCommand::Steer(_)
                                | manifest_domain::FailedCommand::Cancel(_)
                                | manifest_domain::FailedCommand::Compact(_)
                                | manifest_domain::FailedCommand::Goal(_) => {
                                    // The turn is still running (or no
                                    // turn state was taken); its stream
                                    // (TurnCompleted) is what clears
                                    // running. The transcript line above
                                    // carries the failure.
                                }
                                manifest_domain::FailedCommand::Skills(session) => {
                                    // Only the visible session owns the
                                    // composer's catalog: a background
                                    // session's failure never marks it.
                                    if self.is_visible(&session) && self.skills_pending {
                                        self.skills_pending = false;
                                        self.composer.set_skill_catalog(
                                            composer::SkillCatalog::Failed(message),
                                        );
                                    }
                                }
                                manifest_domain::FailedCommand::PendingFetch => {
                                    // No loaded card owns a global fetch
                                    // failure (the error shows above), but
                                    // a hinted request with no card must
                                    // not hang silently: materialize a
                                    // Failed placeholder with a Retry.
                                    for (session, request_id, kind) in
                                        self.pending.hinted_requests()
                                    {
                                        if self.pending.mark_hint_failed(
                                            &session,
                                            &request_id,
                                            kind,
                                            message.clone(),
                                        ) {
                                            let visible = self.is_visible(&session);
                                            if self
                                                .target_transcript(&session)
                                                .push_pending(request_id)
                                                && visible
                                            {
                                                appended = true;
                                            }
                                        }
                                    }
                                }
                                manifest_domain::FailedCommand::Decide(session, request_id)
                                | manifest_domain::FailedCommand::Answer(session, request_id) => {
                                    // The card stays as Failed with
                                    // Retry; retry re-fetches truth.
                                    self.pending.mark_failed(
                                        &session,
                                        &request_id,
                                        message.clone(),
                                    );
                                }
                            }
                        }
                        EngineEvent::SessionSpawned { session, turn } => {
                            self.spawned_sessions.insert(session.clone());
                            self.running.insert(session.clone());
                            if let Some(turn) = turn {
                                self.running_turns.insert(session.clone(), turn);
                            }
                            self.select_on_load = Some(session.clone());
                            // The spawn's workspace rides along for the
                            // branch read: the list has not reloaded yet,
                            // so the spawned session names no workspace.
                            let spawned_workspace =
                                self.pending_spawn.as_ref().and_then(|pending| {
                                    pending.draft.as_ref().map(|draft| draft.workspace.clone())
                                });
                            if self.awaiting_spawn {
                                // The visible transcript holds this
                                // spawn's optimistic prompt: adopt it for
                                // the spawned session so the prompt and
                                // its stream share one state. The pending
                                // selection aligns on the next list load;
                                // skipping the sync here keeps it from
                                // parking the echo away first.
                                self.visible_session = Some(session);
                                self.awaiting_spawn = false;
                                self.pending_spawn = None;
                                self.sync_composer_usage();
                                self.sync_composer_goal();
                                // The fresh session owns a catalog: list
                                // it now that the visible session names
                                // a host. Any in-flight load belongs to
                                // the previous session and lands
                                // ignored, so its gate resets here.
                                self.skills_pending = false;
                                self.request_skills_for_visible();
                            } else {
                                self.sync_visible_transcript();
                            }
                            if let Some(engine) = &self.engine {
                                engine.list_sessions();
                            }
                            extra_task = Task::batch([
                                extra_task,
                                Self::branch_task_for(
                                    spawned_workspace.or_else(|| self.visible_workspace()),
                                ),
                            ]);
                        }
                        EngineEvent::TurnAccepted { session, turn } => {
                            self.running.insert(session.clone());
                            if let Some(turn) = turn {
                                self.running_turns.insert(session, turn);
                            }
                            self.refresh_send_gate();
                        }
                        EngineEvent::SessionResumed { session } => {
                            self.resuming.remove(&session);
                            // A resumed session shows its stored
                            // transcript: backfill it when the select
                            // did not already (or could not yet).
                            self.maybe_request_history(&session);
                            if self.connection == Connection::Connected
                                && let Some(engine) = &self.engine
                            {
                                engine.list_sessions();
                            }
                        }
                        EngineEvent::HistoryLoaded { session, items } => {
                            // Seeds the owning transcript (visible or
                            // parked) through the same paths live
                            // stream events use. An empty fetch still
                            // counts as requested: no refetch loop.
                            if self.is_visible(&session) {
                                appended = self.transcript.seed_history(items);
                            } else {
                                self.stored_transcript(&session).seed_history(items);
                            }
                        }
                        EngineEvent::PendingLoaded { requests } => {
                            appended = self.apply_pending_loaded(requests);
                        }
                        EngineEvent::PendingSettled {
                            session,
                            request_id,
                            resolution,
                        } => {
                            let settled = self.apply_pending_settled(
                                session.clone(),
                                request_id.clone(),
                                resolution,
                            );
                            appended = settled;
                            if !settled && self.pending.card(&request_id).is_none() {
                                // Unknown card: reconcile from truth.
                                if let Some(engine) = &self.engine {
                                    engine.fetch_pending();
                                }
                            }
                        }
                        EngineEvent::Diagnostic(message) => {
                            appended = self.transcript.push_error(message);
                        }
                        EngineEvent::Stream(event) => {
                            // A completed visible turn may have switched
                            // branches: re-read alongside the list sync.
                            let refresh_branch = matches!(
                                &event,
                                manifest_domain::StreamEvent::TurnCompleted { session, .. }
                                if self.is_visible(session)
                            );
                            appended = self.apply_stream_event(event);
                            if refresh_branch {
                                extra_task =
                                    Task::batch([extra_task, self.branch_task_for_visible()]);
                            }
                        }
                    },
                }
                self.refresh_send_gate();
                let scrolled = if appended {
                    transcript::scroll_to_end().map(Message::Transcript)
                } else {
                    Task::none()
                };
                Task::batch([scrolled, extra_task])
            }
            Message::Reconnect => {
                if self.engine_config.is_some() {
                    // Dropping the handle shuts the old worker down; the
                    // subscription restarts under the new attempt. Any
                    // in-flight catalog request dies with it; the next
                    // list load re-requests with fresh context.
                    self.engine = None;
                    self.connection = Connection::Connecting;
                    self.connection_attempt += 1;
                    self.models_pending = false;
                    self.skills_pending = false;
                }
                self.refresh_send_gate();
                Task::none()
            }
        }
    }

    /// Drag subscription plus the engine stream: idle produces nothing
    /// for the drag; while a divider drag is in flight, cursor moves
    /// resize the panel and releasing the left button ends the drag.
    /// The engine stream is gated in
    /// [`engine::subscription_for`](crate::engine::subscription_for).
    pub fn subscription(&self) -> iced::Subscription<Message> {
        let drag = match self.panel_drag {
            PanelDrag::Idle => iced::Subscription::none(),
            PanelDrag::Dragging { .. } => iced::event::listen_with(drag_event),
        };
        let engine = engine::subscription_for(
            self.connection_attempt,
            self.engine_config.as_ref(),
            &self.connection,
        )
        .map(move |(attempt, input)| Message::Engine { attempt, input });
        let window = iced::event::listen_with(window_event);
        iced::Subscription::batch([drag, engine, window])
    }

    /// Renders the title bar over a content row splitting the fixed-width
    /// side panel from the fill-width padded main column of transcript,
    /// drawer (peek or open), composer. The drawer always renders while
    /// todos exist. The panel background and divider run the full height
    /// of the row: the root column carries no outer padding. Corner
    /// resize handles float above the base layer, below any popup scrim.
    pub fn view(&self) -> Element<'_, Message> {
        // The stack root is unconditional so base keeps its widget state across popup changes.
        let mut main = iced::widget::column![
            iced::widget::container(
                self.transcript
                    .view_with_pending(&self.pending, self.visible_session.as_ref())
                    .map(Message::Transcript),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .spacing(SPACE_S)
        .width(Length::Fill)
        .height(Length::Fill);
        let visible_todos: &[TodoItem] = self.visible_todos();
        if !visible_todos.is_empty() {
            let current = self.current_todo.as_deref().unwrap_or("All done");
            let drawer: Element<'_, Message> = match self.todo_drawer {
                TodoDrawer::Peek => {
                    kit::peek_strip(current, &self.todo_summary, Message::ToggleTodoDrawer)
                }
                TodoDrawer::Open => todo_drawer_view(visible_todos, current, &self.todo_summary),
            };
            main = main.push(drawer);
        }
        if let Some(draft) = &self.new_session {
            main = main.push(worktree_bar_view(draft));
        }
        let main: Element<'_, Message> = main
            .push(
                iced::widget::container(self.composer.view().map(Message::Composer))
                    .width(Length::Fill),
            )
            .into();
        let panel: Element<'_, Message> = self.sessions.view().map(Message::Sessions);
        let base: Element<'_, Message> = iced::widget::column![
            topbar(self),
            iced::widget::row![
                iced::widget::container(kit::side_panel(panel))
                    .width(Length::Fixed(self.panel_width))
                    .height(Length::Fill),
                iced::widget::mouse_area(kit::panel_divider())
                    .on_press(Message::PanelDragStarted)
                    .interaction(iced::mouse::Interaction::ResizingHorizontally),
                iced::widget::container(main)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(SPACE_M),
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        let mut stack = iced::widget::Stack::new().push(base);
        // Window corner handles: NW-SE corners use the down-diagonal
        // cursor, NE-SW corners the up-diagonal one.
        stack = stack.push(resize_corner(
            iced::window::Direction::NorthWest,
            iced::mouse::Interaction::ResizingDiagonallyDown,
            iced::alignment::Horizontal::Left,
            iced::alignment::Vertical::Top,
        ));
        stack = stack.push(resize_corner(
            iced::window::Direction::NorthEast,
            iced::mouse::Interaction::ResizingDiagonallyUp,
            iced::alignment::Horizontal::Right,
            iced::alignment::Vertical::Top,
        ));
        stack = stack.push(resize_corner(
            iced::window::Direction::SouthWest,
            iced::mouse::Interaction::ResizingDiagonallyUp,
            iced::alignment::Horizontal::Left,
            iced::alignment::Vertical::Bottom,
        ));
        stack = stack.push(resize_corner(
            iced::window::Direction::SouthEast,
            iced::mouse::Interaction::ResizingDiagonallyDown,
            iced::alignment::Horizontal::Right,
            iced::alignment::Vertical::Bottom,
        ));
        if let Some(menu) = self.composer.model_menu_overlay() {
            stack = stack.push(model_menu_overlay_layer(menu.map(Message::Composer)));
        }
        if let Some(menu) = self.composer.permission_menu_overlay() {
            stack = stack.push(permission_menu_overlay_layer(
                menu.map(Message::Composer),
                self.panel_width,
            ));
        }
        if let Some(menu) = self.composer.attach_menu_overlay() {
            stack = stack.push(attach_menu_overlay_layer(
                menu.map(Message::Composer),
                self.panel_width,
                self.composer.visible_lines,
            ));
        }
        if let Some(popup) = self.composer.context_popover_overlay() {
            stack = stack.push(context_popover_overlay_layer(popup.map(Message::Composer)));
        }
        if let Some(menu) = self.composer.slash_menu_overlay() {
            stack = stack.push(slash_menu_overlay_layer(
                menu.map(Message::Composer),
                self.panel_width,
                self.composer.visible_lines,
            ));
        }
        if let Some(popup) = self.sessions.add_project_view() {
            stack = stack.push(kit::scrim(popup.map(Message::Sessions)));
        }
        stack.into()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::boot()
    }
}

/// Drags the window on press and toggles maximize on double click.
/// Only non-interactive title regions opt into this; the window
/// controls stay out so they never start a drag.
fn drag_region<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    iced::widget::mouse_area(content)
        .on_press(Message::DragWindow)
        .on_double_click(Message::ToggleMaximizeWindow)
        .into()
}

/// Maps window resize events to [`Message::WindowResized`] so the
/// maximized state is rechecked; everything else maps to nothing.
fn window_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::Resized(_)) => Some(Message::WindowResized),
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
            modifiers,
            ..
        }) if modifiers.control() => Some(Message::SubmitPendingShortcut),
        _ => None,
    }
}

/// Maps runtime events to panel-drag messages for [`App::subscription`]:
/// cursor moves track the panel, left-button release ends the drag,
/// everything else maps to nothing.
fn drag_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(Message::PanelDragMoved(position))
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            Some(Message::PanelDragEnded)
        }
        _ => None,
    }
}

/// The floating model menu layer: a transparent click-away catcher
/// covering the window behind the card, and the card itself anchored
/// bottom-right. The card's right edge lines up with the main column's
/// right edge (the window edge minus the main padding), and its bottom
/// edge floats just above the composer sub-bar. The catcher carries no
/// fill, so nothing beneath the card dims, and a press anywhere outside
/// the card dismisses the menu without reflowing anything beneath it.
fn model_menu_overlay_layer<'a>(menu: Element<'a, Message>) -> Element<'a, Message> {
    let catcher = iced::widget::mouse_area(
        iced::widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::Composer(composer::Message::ModelDismissed));
    let card = iced::widget::row![
        iced::widget::column![
            iced::widget::container(menu)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right)
                .align_y(iced::alignment::Vertical::Bottom),
            iced::widget::Space::new().height(Length::Fixed(MODEL_MENU_BOTTOM_OFFSET)),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
        iced::widget::Space::new().width(Length::Fixed(SPACE_M)),
    ]
    .width(Length::Fill)
    .height(Length::Fill);
    iced::widget::Stack::new()
        .push(catcher)
        .push(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// The floating permission menu layer: a transparent click-away
/// catcher covering the window behind the card, and the card itself
/// anchored bottom-left. The card's left edge lines up with the main
/// column's left edge: the side panel, the divider hit width, and the
/// main container's padding. Its bottom edge floats just above the
/// composer sub-bar with the same offset as the model card. The
/// catcher carries no fill, so nothing beneath the card dims, and a
/// press anywhere outside the card dismisses the menu without
/// reflowing anything beneath it.
fn permission_menu_overlay_layer<'a>(
    menu: Element<'a, Message>,
    panel_width: f32,
) -> Element<'a, Message> {
    let catcher = iced::widget::mouse_area(
        iced::widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::Composer(composer::Message::PermissionDismissed));
    let card = iced::widget::row![
        iced::widget::Space::new().width(Length::Fixed(panel_width + DIVIDER_HIT_WIDTH + SPACE_M)),
        iced::widget::column![
            iced::widget::container(menu)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::alignment::Horizontal::Left)
                .align_y(iced::alignment::Vertical::Bottom),
            iced::widget::Space::new().height(Length::Fixed(MODEL_MENU_BOTTOM_OFFSET)),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);
    iced::widget::Stack::new()
        .push(catcher)
        .push(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// The floating attach menu layer: a transparent click-away catcher
/// covering the window behind the card, and the card itself anchored
/// bottom-left. The card's left edge lines up with the main column's
/// left edge: the side panel, the divider hit width, and the main
/// container's padding (the Plus button's column). Its bottom edge
/// floats just above the composer input bar — the attach trigger's
/// bar, which sits above the sub-bar — via
/// [`attach_menu_bottom_offset`] for the composer's visible line
/// count. The catcher carries no fill, so nothing beneath the card
/// dims, and a press anywhere outside the card dismisses the menu
/// without reflowing anything beneath it.
fn attach_menu_overlay_layer<'a>(
    menu: Element<'a, Message>,
    panel_width: f32,
    visible_lines: usize,
) -> Element<'a, Message> {
    let catcher = iced::widget::mouse_area(
        iced::widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::Composer(composer::Message::AttachDismissed));
    let card = iced::widget::row![
        iced::widget::Space::new().width(Length::Fixed(panel_width + DIVIDER_HIT_WIDTH + SPACE_M)),
        iced::widget::column![
            iced::widget::container(menu)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::alignment::Horizontal::Left)
                .align_y(iced::alignment::Vertical::Bottom),
            iced::widget::Space::new()
                .height(Length::Fixed(attach_menu_bottom_offset(visible_lines))),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);
    iced::widget::Stack::new()
        .push(catcher)
        .push(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// A skill invocation carrying the composer's attachment refs: refs
/// append as `[ref: path]` lines after the parsed arguments, mirroring
/// how [`composer::State::composed_prompt`] appends them to plain
/// text. Without attachments the invocation rides unchanged.
fn skill_input_with_refs(
    mut invocation: manifest_domain::SkillInvocation,
    attachments: &[String],
) -> manifest_domain::TurnInput {
    let mut arguments = invocation.arguments;
    for path in attachments {
        if !arguments.is_empty() {
            arguments.push('\n');
        }
        arguments.push_str("[ref: ");
        arguments.push_str(path);
        arguments.push(']');
    }
    invocation.arguments = arguments;
    manifest_domain::TurnInput::Skill(invocation)
}

/// The floating context popup layer: a transparent click-away
/// catcher covering the window behind the card, and the card itself
/// anchored bottom-right. The card's right edge lines up with the
/// main column's right edge (the window edge minus the main padding),
/// and its bottom edge floats just above the context meter pill in
/// the composer sub-bar, with the same offset as the model card —
/// the context trigger sits in that sub-bar, so no input-bar height
/// enters the anchor. The catcher carries no fill, so nothing beneath
/// the card dims, and a press anywhere outside the card dismisses the
/// popup without reflowing anything beneath it.
fn context_popover_overlay_layer<'a>(popup: Element<'a, Message>) -> Element<'a, Message> {
    let catcher = iced::widget::mouse_area(
        iced::widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::Composer(composer::Message::ContextDismissed));
    let card = iced::widget::row![
        iced::widget::column![
            iced::widget::container(popup)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right)
                .align_y(iced::alignment::Vertical::Bottom),
            iced::widget::Space::new().height(Length::Fixed(MODEL_MENU_BOTTOM_OFFSET)),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
        iced::widget::Space::new().width(Length::Fixed(SPACE_M)),
    ]
    .width(Length::Fill)
    .height(Length::Fill);
    iced::widget::Stack::new()
        .push(catcher)
        .push(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// The floating slash-menu layer: a transparent click-away catcher
/// covering the window behind the card, and the card itself anchored
/// bottom-left. The card's left edge lines up with the main column's
/// left edge: the side panel, the divider hit width, and the main
/// container's padding. Its bottom edge floats just above the composer
/// input bar — the slash trigger's bar, which sits above the sub-bar —
/// via [`attach_menu_bottom_offset`] for the composer's visible line
/// count, the same anchor the attach menu uses. The catcher carries no
/// fill, so nothing beneath the card dims, and a press anywhere
/// outside the card dismisses the menu without reflowing anything
/// beneath it.
fn slash_menu_overlay_layer<'a>(
    menu: Element<'a, Message>,
    panel_width: f32,
    visible_lines: usize,
) -> Element<'a, Message> {
    let catcher = iced::widget::mouse_area(
        iced::widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::Composer(composer::Message::SlashDismiss));
    let card = iced::widget::row![
        iced::widget::Space::new().width(Length::Fixed(panel_width + DIVIDER_HIT_WIDTH + SPACE_M)),
        iced::widget::column![
            iced::widget::container(menu)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::alignment::Horizontal::Left)
                .align_y(iced::alignment::Vertical::Bottom),
            iced::widget::Space::new()
                .height(Length::Fixed(attach_menu_bottom_offset(visible_lines))),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);
    iced::widget::Stack::new()
        .push(catcher)
        .push(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// One window corner resize handle: a small press target pinned to a
/// corner of the full window, with the diagonal cursor of its axis.
fn resize_corner<'a>(
    direction: iced::window::Direction,
    interaction: iced::mouse::Interaction,
    horizontal: iced::alignment::Horizontal,
    vertical: iced::alignment::Vertical,
) -> Element<'a, Message> {
    iced::widget::container(
        iced::widget::mouse_area(
            iced::widget::Space::new()
                .width(Length::Fixed(WINDOW_RESIZE_HANDLE))
                .height(Length::Fixed(WINDOW_RESIZE_HANDLE)),
        )
        .on_press(Message::ResizeWindow(direction))
        .interaction(interaction),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(horizontal)
    .align_y(vertical)
    .into()
}

/// Validates the add-project form off the reducer: trims the name, then
/// resolves the directory to its canonical path. Runs inside a
/// [`Task`] so the reducer never touches the filesystem.
fn validate_project_dir(directory: String, name: String) -> Result<(String, String), String> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(String::from("Give the project a name."));
    }
    let canonical = std::fs::canonicalize(directory.trim()).map_err(|_| {
        String::from("That folder couldn't be found. Check the path and try again.")
    })?;
    if !canonical.is_dir() {
        return Err(String::from("That path isn't a folder. Choose a folder."));
    }
    Ok((canonical.display().to_string(), trimmed_name.to_string()))
}

/// User-facing note when send is pressed while the engine is not
/// connected yet. Matches the worker's wording for the same refusal.
fn not_connected_message() -> String {
    String::from("The Muse MCP server isn't connected yet. Try again once it connects.")
}

/// User-facing note when steering finds no tracked turn id: the
/// session runs, but the server never named its turn, so there is
/// nothing to steer into. A value, never silence.
fn no_running_turn_message() -> String {
    String::from(
        "That session is still starting its turn, so there is nothing to steer yet. Try again in a moment.",
    )
}

/// Snaps the transcript to the end when an entry was appended, else
/// yields no task. Mirrors the engine-event appended pattern.
fn scroll_transcript_to_end(appended: bool) -> Task<Message> {
    if appended {
        transcript::scroll_to_end().map(Message::Transcript)
    } else {
        Task::none()
    }
}

/// Names a mid-turn delta loss for the transcript: the count that was
/// dropped, plus the reassurance that the final message is complete.
/// Precomputed here so the view formats nothing (UI contract §2).
fn dropped_deltas_message(dropped: u64) -> String {
    if dropped == 1 {
        String::from("The stream dropped 1 update; the final message is complete.")
    } else {
        format!("The stream dropped {dropped} updates; the final message is complete.")
    }
}

/// The title bar: the app title in the fixed panel share, then the
/// selected session title truly centered in the fill share with the
/// window controls right-aligned. A plain function on the root state
/// — no state of its own, no module.
fn topbar(state: &App) -> Element<'_, Message> {
    // Draft mode shows the precomputed draft title; the view formats
    // nothing (UI contract §2).
    let selected_title: &str = if let Some(draft) = &state.new_session {
        draft.title.as_str()
    } else {
        state
            .sessions
            .selected_summary()
            .map(|summary| summary.title.as_str())
            .unwrap_or("No session")
    };
    // No session and no draft means no context segments: the flanks
    // stay empty spacers around the centered title.
    let has_context = state.new_session.is_some() || state.sessions.selected_summary().is_some();
    // The two Fill regions flanking the title share the leftover
    // width equally, so the title stays centered no matter how wide
    // the window controls run. The left region names the project and
    // branch from stored state; the view formats nothing here, only
    // conditional text widgets (UI contract §2).
    let left: Element<'_, Message> = if has_context {
        let mut segments = iced::widget::row![]
            .spacing(SPACE_S)
            .align_y(iced::alignment::Vertical::Center);
        if let Some(project) = state.topbar_project_name() {
            segments = segments.push(
                iced::widget::row![
                    kit::icon(
                        lucide_icons::Icon::FolderGit2,
                        FONT_SIZE_SMALL,
                        TEXT_SECONDARY,
                    ),
                    iced::widget::text(project)
                        .size(FONT_SIZE_SMALL)
                        .color(TEXT_SECONDARY),
                ]
                .spacing(SPACE_XS)
                .align_y(iced::alignment::Vertical::Center),
            );
        }
        if let Some(branch) = state.branch.as_deref() {
            segments = segments.push(
                iced::widget::row![
                    kit::icon(
                        lucide_icons::Icon::GitBranch,
                        FONT_SIZE_SMALL,
                        TEXT_TERTIARY,
                    ),
                    iced::widget::text(branch)
                        .size(FONT_SIZE_SMALL)
                        .color(TEXT_TERTIARY),
                ]
                .spacing(SPACE_XS)
                .align_y(iced::alignment::Vertical::Center),
            );
        }
        drag_region(iced::widget::container(segments).width(Length::Fill))
    } else {
        drag_region(
            iced::widget::container(iced::widget::text("").size(FONT_SIZE_BODY))
                .width(Length::Fill),
        )
    };
    let title_row = iced::widget::row![
        left,
        drag_region(iced::widget::container(
            iced::widget::text(selected_title)
                .size(FONT_SIZE_BODY)
                .color(TEXT_SECONDARY),
        )),
        iced::widget::container({
            let mut controls = iced::widget::row![];
            // The right cluster names the connection, then the window
            // controls; usage lives in the composer's ring, not here.
            // Fixture mode shows no cluster; live mode names the
            // connection state and offers Retry while failed.
            if state.engine_config.is_some() {
                controls = controls.push(engine::connection_cluster(
                    &state.connection,
                    Message::Reconnect,
                ));
            }
            controls
                .push(kit::icon_button(
                    lucide_icons::Icon::Minus,
                    kit::IconButtonKind::Subtle,
                    Some(Message::MinimizeWindow),
                ))
                .push(kit::icon_button(
                    kit::maximize_icon(state.window_maximized),
                    kit::IconButtonKind::Subtle,
                    Some(Message::ToggleMaximizeWindow),
                ))
                .push(kit::icon_button(
                    lucide_icons::Icon::X,
                    kit::IconButtonKind::Subtle,
                    Some(Message::CloseWindow),
                ))
                .spacing(SPACE_S)
                .align_y(iced::alignment::Vertical::Center)
        })
        .align_right(Length::Fill),
    ]
    .spacing(SPACE_M)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill);
    kit::title_bar(
        iced::widget::row![
            drag_region(
                iced::widget::container(
                    iced::widget::text("Manifest")
                        .size(FONT_SIZE_TITLE)
                        .color(TEXT_PRIMARY),
                )
                .width(Length::Fixed(state.panel_width)),
            ),
            iced::widget::container(title_row).width(Length::Fill),
        ]
        .spacing(SPACE_M)
        .align_y(iced::alignment::Vertical::Center)
        .width(Length::Fill),
    )
}

/// The new-session draft's worktree bar, rendered directly above the
/// composer while a draft is open: the mode pills, then the mode's
/// field (if any). Reads the draft only (UI contract §2).
fn worktree_bar_view(draft: &NewSessionDraft) -> Element<'_, Message> {
    let kind_for = |mode| {
        if draft.worktree_mode == mode {
            kit::ButtonKind::Primary
        } else {
            kit::ButtonKind::Ghost
        }
    };
    let row = iced::widget::row![
        iced::widget::text("Worktree")
            .size(FONT_SIZE_SMALL)
            .color(TEXT_SECONDARY),
        kit::pill_button(
            "Off",
            kind_for(DraftWorktreeMode::Off),
            Some(Message::DraftWorktreeModeSelected(DraftWorktreeMode::Off)),
        ),
        kit::pill_button(
            "Create",
            kind_for(DraftWorktreeMode::Create),
            Some(Message::DraftWorktreeModeSelected(
                DraftWorktreeMode::Create
            )),
        ),
        kit::pill_button(
            "Existing",
            kind_for(DraftWorktreeMode::Existing),
            Some(Message::DraftWorktreeModeSelected(
                DraftWorktreeMode::Existing
            )),
        ),
    ]
    .spacing(SPACE_S)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill);
    let mut bar = iced::widget::column![iced::widget::container(row).width(Length::Fill),]
        .spacing(SPACE_S)
        .width(Length::Fill);
    match draft.worktree_mode {
        DraftWorktreeMode::Create => {
            bar = bar.push(kit::text_field(
                "Base ref (optional)",
                draft.worktree_base.as_str(),
                Message::DraftWorktreeBaseChanged,
            ));
        }
        DraftWorktreeMode::Existing => {
            bar = bar.push(kit::text_field(
                "Existing worktree path",
                draft.worktree_path.as_str(),
                Message::DraftWorktreePathChanged,
            ));
        }
        DraftWorktreeMode::Off => {}
    }
    bar.into()
}

/// The drawer's current step: the first in-progress entry, else
/// the first pending one. Cancelled and done entries are never
/// current; a list with neither in-progress nor pending entries has
/// no current step, which the view renders as "All done".
fn current_todo_label(todos: &[TodoItem]) -> Option<String> {
    todos
        .iter()
        .find(|todo| todo.status == TodoStatus::InProgress)
        .or_else(|| todos.iter().find(|todo| todo.status == TodoStatus::Pending))
        .map(|todo| todo.label.clone())
}

/// The open TODO drawer: a panel whose first row is the shared bar
/// header, followed by one line per todo: a leading status icon
/// (check-in-circle for finished steps, play-in-circle while in
/// progress, empty circle for pending or cancelled ones) beside the
/// label in full width. Finished and cancelled rows read in the
/// tertiary tone; pending and in-progress rows read in the primary
/// tone.
fn todo_drawer_view<'a>(
    todos: &'a [TodoItem],
    current: &'a str,
    summary: &'a str,
) -> Element<'a, Message> {
    let mut list = iced::widget::column![kit::drawer_header(
        current,
        summary,
        Message::ToggleTodoDrawer
    ),]
    .spacing(SPACE_S)
    .width(Length::Fill);
    for todo in todos {
        // No string building in the view (UI contract §2): the status
        // icon is a static mapping beside the borrowed todo text.
        let color = match todo.status {
            TodoStatus::Done | TodoStatus::Cancelled => TEXT_TERTIARY,
            TodoStatus::Pending | TodoStatus::InProgress => TEXT_PRIMARY,
        };
        list = list.push(
            iced::widget::container(
                iced::widget::row![
                    kit::icon(kit::todo_status_icon(todo.status), FONT_SIZE_SMALL, color),
                    iced::widget::text(todo.label.as_str())
                        .size(FONT_SIZE_SMALL)
                        .color(color)
                        .width(Length::Fill),
                ]
                .spacing(SPACE_S)
                .align_y(iced::alignment::Vertical::Center),
            )
            .width(Length::Fill),
        );
    }
    kit::panel(iced::widget::container(list).width(Length::Fill))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{PANEL_WIDTH_DEFAULT, PANEL_WIDTH_MAX, PANEL_WIDTH_MIN};
    use iced::widget::text_editor;

    /// Types text into the composer through edit actions.
    fn type_composer_text(app: &mut App, text: &str) {
        for next in text.chars() {
            let edit = if next == '\n' {
                text_editor::Edit::Enter
            } else {
                text_editor::Edit::Insert(next)
            };
            let _ = app.update(Message::Composer(composer::Message::Edit(
                text_editor::Action::Edit(edit),
            )));
        }
    }

    #[test]
    fn window_identified_none_leaves_window_id_none() {
        let mut got = App::boot();
        assert_eq!(got.window_id, None);
        let _ = got.update(Message::WindowIdentified(None));
        assert_eq!(got.window_id, None);
    }

    #[test]
    fn window_messages_without_id_leave_state_unchanged() {
        let mut got = App::boot();
        let _ = got.update(Message::ToggleTodoDrawer);
        assert_eq!(got.todo_drawer, TodoDrawer::Open);
        let target = got.sessions.groups[0].sessions[0].id.clone();
        let _ = got.update(Message::Sessions(sessions::Message::Select(target.clone())));
        let _ = got.update(Message::DragWindow);
        let _ = got.update(Message::MinimizeWindow);
        let _ = got.update(Message::ToggleMaximizeWindow);
        let _ = got.update(Message::CloseWindow);
        let _ = got.update(Message::ResizeWindow(iced::window::Direction::SouthEast));
        assert_eq!(got.window_id, None);
        assert_eq!(got.todo_drawer, TodoDrawer::Open);
        assert_eq!(got.sessions.selected, target);
        assert_eq!(got.panel_width, PANEL_WIDTH_DEFAULT);
        assert_eq!(got.panel_drag, PanelDrag::Idle);
    }

    #[test]
    fn window_maximized_checked_stores_value() {
        let mut got = App::boot();
        assert!(!got.window_maximized);
        let _ = got.update(Message::WindowMaximizedChecked(true));
        assert!(got.window_maximized);
        let _ = got.update(Message::WindowMaximizedChecked(false));
        assert!(!got.window_maximized);
    }

    #[test]
    fn window_resized_without_id_is_noop() {
        let mut got = App::boot();
        assert_eq!(got.window_id, None);
        let _ = got.update(Message::WindowResized);
        assert_eq!(got.window_id, None);
        assert!(!got.window_maximized);
    }

    #[test]
    fn window_identified_stores_unique_id() {
        let mut got = App::boot();
        let id = iced::window::Id::unique();
        let _ = got.update(Message::WindowIdentified(Some(id)));
        assert_eq!(got.window_id, Some(id));
    }

    #[test]
    fn drawer_toggle_opens_from_peek() {
        let mut got = App::boot();
        assert_eq!(got.todo_drawer, TodoDrawer::Peek);
        let _ = got.update(Message::ToggleTodoDrawer);
        assert_eq!(got.todo_drawer, TodoDrawer::Open);
    }

    #[test]
    fn drawer_toggle_peeks_from_open() {
        let mut got = App::boot();
        let _ = got.update(Message::ToggleTodoDrawer);
        assert_eq!(got.todo_drawer, TodoDrawer::Open);
        let _ = got.update(Message::ToggleTodoDrawer);
        assert_eq!(got.todo_drawer, TodoDrawer::Peek);
    }

    #[test]
    fn boot_precomputes_drawer_bar_strings() {
        let got = App::boot();
        assert_eq!(got.todo_summary, "1 of 4 done");
        assert_eq!(
            got.current_todo,
            Some(String::from("Move token refresh into the request path")),
        );
    }

    fn todo_entry(label: &str, status: TodoStatus) -> TodoItem {
        TodoItem {
            label: String::from(label),
            status,
        }
    }

    /// A completed `write_todos` item for `session`, as the mapper
    /// classifies it.
    fn todo_completed(session: &str, item_id: &str, todos: Vec<TodoItem>) -> StreamEvent {
        StreamEvent::ItemCompleted {
            session: SessionId::new(session),
            turn: None,
            view_cursor: None,
            item: manifest_domain::StreamItem::ToolCall {
                item_id: String::from(item_id),
                tool: String::from("write_todos"),
                headline: None,
                args_json: String::new(),
                todos: Some(todos),
                status: manifest_domain::ItemStatus::Completed,
                output: None,
                failure: None,
            },
        }
    }

    /// A started `write_todos` item for `session`, as the mapper
    /// classifies it.
    fn todo_started(session: &str, item_id: &str, todos: Vec<TodoItem>) -> StreamEvent {
        StreamEvent::ItemStarted {
            session: SessionId::new(session),
            turn: None,
            view_cursor: None,
            item: manifest_domain::StreamItem::ToolCall {
                item_id: String::from(item_id),
                tool: String::from("write_todos"),
                headline: None,
                args_json: String::new(),
                todos: Some(todos),
                status: manifest_domain::ItemStatus::InProgress,
                output: None,
                failure: None,
            },
        }
    }

    #[test]
    fn todo_completed_replaces_visible_list_wholesale() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        assert_eq!(app.todo_summary, "0 of 0 done");
        assert_eq!(app.current_todo, None);
        let _ = app.apply_stream_event(todo_completed(
            "s-1",
            "todo-1",
            vec![
                todo_entry("first", TodoStatus::Pending),
                todo_entry("second", TodoStatus::Pending),
            ],
        ));
        assert_eq!(app.todo_summary, "0 of 2 done");
        assert_eq!(app.current_todo.as_deref(), Some("first"));
        // A second item replaces the first wholesale; no entry survives.
        let _ = app.apply_stream_event(todo_completed(
            "s-1",
            "todo-2",
            vec![todo_entry("third", TodoStatus::Done)],
        ));
        assert_eq!(
            app.todo_lists.get(&SessionId::new("s-1")),
            Some(&vec![todo_entry("third", TodoStatus::Done)])
        );
        assert_eq!(app.todo_summary, "1 of 1 done");
        assert_eq!(app.current_todo, None);
    }

    #[test]
    fn todo_started_sets_list_before_completion_replaces_it() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let _ = app.apply_stream_event(todo_started(
            "s-1",
            "todo-1",
            vec![todo_entry("first", TodoStatus::Pending)],
        ));
        assert_eq!(app.todo_summary, "0 of 1 done");
        assert_eq!(app.current_todo.as_deref(), Some("first"));
        let _ = app.apply_stream_event(todo_completed(
            "s-1",
            "todo-1",
            vec![
                todo_entry("first", TodoStatus::Done),
                todo_entry("second", TodoStatus::Pending),
            ],
        ));
        assert_eq!(app.todo_summary, "1 of 2 done");
        assert_eq!(app.current_todo.as_deref(), Some("second"));
    }

    #[test]
    fn todo_update_in_background_session_waits_for_selection() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1", "s-2"]);
        let _ = app.apply_stream_event(todo_completed(
            "s-2",
            "todo-1",
            vec![
                todo_entry("first", TodoStatus::Done),
                todo_entry("second", TodoStatus::Pending),
            ],
        ));
        // The background list stores without touching the visible drawer.
        assert_eq!(
            app.todo_lists.get(&SessionId::new("s-2")).map(Vec::len),
            Some(2)
        );
        assert_eq!(app.todo_summary, "0 of 0 done");
        assert_eq!(app.current_todo, None);
        assert!(app.visible_todos().is_empty());
        // Selecting the session projects its stored list.
        let _ = app.update(Message::Sessions(sessions::Message::Select(
            SessionId::new("s-2"),
        )));
        assert_eq!(app.sessions.selected, SessionId::new("s-2"));
        assert_eq!(app.todo_summary, "1 of 2 done");
        assert_eq!(app.current_todo.as_deref(), Some("second"));
        assert_eq!(app.visible_todos().len(), 2);
    }

    #[test]
    fn todo_all_done_clears_current_todo() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let _ = app.apply_stream_event(todo_completed(
            "s-1",
            "todo-1",
            vec![
                todo_entry("first", TodoStatus::Done),
                todo_entry("second", TodoStatus::Done),
            ],
        ));
        assert_eq!(app.todo_summary, "2 of 2 done");
        // No unfinished step: the view renders this as "All done".
        assert_eq!(app.current_todo, None);
        assert_eq!(app.visible_todos().len(), 2);
    }

    #[test]
    fn todo_in_progress_is_current_ahead_of_pending() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let _ = app.apply_stream_event(todo_completed(
            "s-1",
            "todo-1",
            vec![
                todo_entry("waiting", TodoStatus::Pending),
                todo_entry("working", TodoStatus::InProgress),
                todo_entry("dropped", TodoStatus::Cancelled),
            ],
        ));
        assert_eq!(app.todo_summary, "0 of 3 done");
        assert_eq!(app.current_todo.as_deref(), Some("working"));
    }

    #[test]
    fn todo_cancelled_never_becomes_current() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let _ = app.apply_stream_event(todo_completed(
            "s-1",
            "todo-1",
            vec![
                todo_entry("finished", TodoStatus::Done),
                todo_entry("dropped", TodoStatus::Cancelled),
            ],
        ));
        assert_eq!(app.todo_summary, "1 of 2 done");
        // No in-progress or pending step: the view renders "All done".
        assert_eq!(app.current_todo, None);
    }

    #[test]
    fn todo_empty_list_hides_drawer_state() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let _ = app.apply_stream_event(todo_completed(
            "s-1",
            "todo-1",
            vec![todo_entry("first", TodoStatus::Pending)],
        ));
        assert!(!app.visible_todos().is_empty());
        let _ = app.apply_stream_event(todo_completed("s-1", "todo-2", Vec::new()));
        assert!(app.visible_todos().is_empty());
        assert_eq!(app.todo_summary, "0 of 0 done");
        assert_eq!(app.current_todo, None);
    }

    #[test]
    fn boot_selects_first_session_with_drawer_peeking() {
        let fixture = manifest_domain::fixture();
        let got = App::from_fixture(fixture.clone());
        let total: usize = got
            .sessions
            .groups
            .iter()
            .map(|group| group.sessions.len())
            .sum::<usize>()
            + got.sessions.unprojected.len();
        assert!(total >= 3);
        assert_eq!(got.todo_drawer, TodoDrawer::Peek);
        let first_id = fixture.sessions.first().map(|summary| summary.id.clone());
        assert!(first_id.is_some());
        assert_eq!(got.sessions.selected, first_id.unwrap());
    }

    #[test]
    fn selection_survives_composer_and_drawer_messages() {
        let mut got = App::boot();
        let target = got.sessions.groups[0].sessions[1].id.clone();
        let _ = got.update(Message::Sessions(sessions::Message::Select(target.clone())));
        assert_eq!(got.sessions.selected, target);
        type_composer_text(&mut got, "draft text");
        let _ = got.update(Message::ToggleTodoDrawer);
        assert_eq!(got.sessions.selected, target);
        assert_eq!(got.composer.content.text(), "draft text");
        assert_eq!(got.todo_drawer, TodoDrawer::Open);
    }

    #[test]
    fn draft_preserved_across_drawer_toggles() {
        let mut got = App::boot();
        type_composer_text(&mut got, "keep me");
        let _ = got.update(Message::ToggleTodoDrawer);
        assert_eq!(got.todo_drawer, TodoDrawer::Open);
        let _ = got.update(Message::ToggleTodoDrawer);
        assert_eq!(got.todo_drawer, TodoDrawer::Peek);
        assert_eq!(got.composer.content.text(), "keep me");
    }

    #[test]
    fn transcript_scroll_state_survives_other_messages() {
        // A scroll viewport has no public constructor outside iced, so
        // no headless test can feed `Message::Transcript` directly; the
        // reducer itself is covered through `transcript::State` tests.
        // This pins the routing contract instead: other messages leave
        // the transcript scroll state alone.
        let mut got = App::boot();
        assert_eq!(got.transcript.sticky, None);
        assert_eq!(got.transcript.scroll_offset, 0.0);
        let target = got.sessions.groups[0].sessions[1].id.clone();
        let _ = got.update(Message::Sessions(sessions::Message::Select(target.clone())));
        type_composer_text(&mut got, "hello");
        let _ = got.update(Message::ToggleTodoDrawer);
        assert_eq!(got.sessions.selected, target);
        assert_eq!(got.composer.content.text(), "hello");
        assert_eq!(got.transcript.sticky, None);
        assert_eq!(got.transcript.scroll_offset, 0.0);
    }

    #[test]
    fn drawer_state_survives_selection_and_draft_messages() {
        let mut got = App::boot();
        let _ = got.update(Message::ToggleTodoDrawer);
        let target = got.sessions.groups[0].sessions[1].id.clone();
        let _ = got.update(Message::Sessions(sessions::Message::Select(target)));
        type_composer_text(&mut got, "hello");
        assert_eq!(got.todo_drawer, TodoDrawer::Open);
        assert_eq!(got.composer.content.text(), "hello");
    }

    #[test]
    fn popup_name_rule_flows_through_root_update() {
        let mut got = App::boot();
        let _ = got.update(Message::Sessions(sessions::Message::OpenAddProject));
        let _ = got.update(Message::Sessions(sessions::Message::DirectoryChanged(
            String::from("/x/y/billing-service"),
        )));
        assert_eq!(
            got.sessions.add_project,
            sessions::AddProject::Open {
                directory: String::from("/x/y/billing-service"),
                name: String::from("billing-service"),
                name_edited: false,
                error: None,
            }
        );
        assert!(got.sessions.add_project_view().is_some());
    }

    #[test]
    fn popup_survives_drawer_and_composer_messages() {
        let mut got = App::boot();
        let _ = got.update(Message::Sessions(sessions::Message::OpenAddProject));
        let _ = got.update(Message::ToggleTodoDrawer);
        type_composer_text(&mut got, "draft text");
        assert_eq!(got.todo_drawer, TodoDrawer::Open);
        assert_eq!(got.composer.content.text(), "draft text");
        assert_eq!(
            got.sessions.add_project,
            sessions::AddProject::Open {
                directory: String::new(),
                name: String::new(),
                name_edited: false,
                error: None,
            }
        );
    }

    #[test]
    fn panel_drag_captures_offset_then_tracks() {
        let mut got = App::boot();
        assert_eq!(got.panel_width, PANEL_WIDTH_DEFAULT);
        assert_eq!(got.panel_width, 260.0);
        let _ = got.update(Message::PanelDragStarted);
        assert_eq!(got.panel_drag, PanelDrag::Dragging { offset: None });
        // Grab 40px into the hit zone: the panel must not jump.
        let _ = got.update(Message::PanelDragMoved(iced::Point::new(300.0, 0.0)));
        assert_eq!(got.panel_width, 260.0);
        assert_eq!(got.panel_drag, PanelDrag::Dragging { offset: Some(40.0) });
        let _ = got.update(Message::PanelDragMoved(iced::Point::new(340.0, 0.0)));
        assert_eq!(got.panel_width, 300.0);
        assert_eq!(got.panel_drag, PanelDrag::Dragging { offset: Some(40.0) });
    }

    #[test]
    fn panel_drag_clamps_to_bounds() {
        let mut got = App::boot();
        let _ = got.update(Message::PanelDragStarted);
        let _ = got.update(Message::PanelDragMoved(iced::Point::new(260.0, 0.0)));
        assert_eq!(got.panel_width, 260.0);
        let _ = got.update(Message::PanelDragMoved(iced::Point::new(100.0, 0.0)));
        assert_eq!(got.panel_width, PANEL_WIDTH_MIN);
        let _ = got.update(Message::PanelDragMoved(iced::Point::new(900.0, 0.0)));
        assert_eq!(got.panel_width, PANEL_WIDTH_MAX);
    }

    /// Test engine: counts `list_sessions` calls like the real worker
    /// would serve them, and records spawn/send arguments, without
    /// spawning anything. The counters are shared, so clones observe
    /// the same calls.
    /// One recorded steer call: session, prompt, and steered turn.
    type SteerCall = (manifest_domain::SessionId, String, manifest_domain::TurnId);
    /// One recorded cancel call: session and the canceled turn, if known.
    type CancelCall = (manifest_domain::SessionId, Option<manifest_domain::TurnId>);
    /// One recorded catalog call: workspace and session context.
    type ModelsCall = (Option<String>, Option<manifest_domain::SessionId>);
    /// One recorded decide call: session, request, decision, reason.
    type DecideCall = (
        manifest_domain::SessionId,
        String,
        manifest_domain::ApprovalDecision,
        Option<String>,
    );
    /// One recorded answer call: session, request, assembled answer.
    type AnswerCall = (
        manifest_domain::SessionId,
        String,
        manifest_domain::PendingAnswer,
    );

    #[derive(Clone)]
    struct FakeEngine {
        calls: std::sync::Arc<std::sync::Mutex<usize>>,
        spawns: std::sync::Arc<
            std::sync::Mutex<
                Vec<(
                    String,
                    manifest_domain::TurnInput,
                    manifest_domain::SpawnChoices,
                )>,
            >,
        >,
        turns: std::sync::Arc<
            std::sync::Mutex<Vec<(manifest_domain::SessionId, manifest_domain::TurnInput)>>,
        >,
        resumes: std::sync::Arc<std::sync::Mutex<Vec<manifest_domain::SessionId>>>,
        steers: std::sync::Arc<std::sync::Mutex<Vec<SteerCall>>>,
        cancels: std::sync::Arc<std::sync::Mutex<Vec<CancelCall>>>,
        models: std::sync::Arc<std::sync::Mutex<Vec<ModelsCall>>>,
        skills: std::sync::Arc<std::sync::Mutex<Vec<manifest_domain::SessionId>>>,
        histories: std::sync::Arc<std::sync::Mutex<Vec<manifest_domain::SessionId>>>,
        compacts: std::sync::Arc<std::sync::Mutex<Vec<manifest_domain::SessionId>>>,
        goals: std::sync::Arc<std::sync::Mutex<Vec<(manifest_domain::SessionId, String)>>>,
        fetches: std::sync::Arc<std::sync::Mutex<usize>>,
        decides: std::sync::Arc<std::sync::Mutex<Vec<DecideCall>>>,
        answers: std::sync::Arc<std::sync::Mutex<Vec<AnswerCall>>>,
    }

    impl manifest_domain::SessionEngine for FakeEngine {
        fn list_sessions(&self) {
            *self.calls.lock().unwrap() += 1;
        }

        fn spawn_session(
            &self,
            workspace: String,
            input: manifest_domain::TurnInput,
            choices: manifest_domain::SpawnChoices,
        ) {
            self.spawns
                .lock()
                .unwrap()
                .push((workspace, input, choices));
        }

        fn send_turn(
            &self,
            session: manifest_domain::SessionId,
            input: manifest_domain::TurnInput,
        ) {
            self.turns.lock().unwrap().push((session, input));
        }

        fn resume_session(&self, session: manifest_domain::SessionId) {
            self.resumes.lock().unwrap().push(session);
        }

        fn steer_session(
            &self,
            session: manifest_domain::SessionId,
            prompt: String,
            turn: manifest_domain::TurnId,
        ) {
            self.steers.lock().unwrap().push((session, prompt, turn));
        }

        fn cancel_turn(
            &self,
            session: manifest_domain::SessionId,
            turn: Option<manifest_domain::TurnId>,
        ) {
            self.cancels.lock().unwrap().push((session, turn));
        }

        fn list_models(
            &self,
            workspace: Option<String>,
            session: Option<manifest_domain::SessionId>,
        ) {
            self.models.lock().unwrap().push((workspace, session));
        }

        fn list_skills(&self, session: manifest_domain::SessionId) {
            self.skills.lock().unwrap().push(session);
        }

        fn load_history(&self, session: manifest_domain::SessionId) {
            self.histories.lock().unwrap().push(session);
        }

        fn compact_session(&self, session: manifest_domain::SessionId) {
            self.compacts.lock().unwrap().push(session);
        }

        fn set_goal(&self, session: manifest_domain::SessionId, objective: String) {
            self.goals.lock().unwrap().push((session, objective));
        }

        fn fetch_pending(&self) {
            *self.fetches.lock().unwrap() += 1;
        }

        fn decide(
            &self,
            session: manifest_domain::SessionId,
            request_id: String,
            decision: manifest_domain::ApprovalDecision,
            reason: Option<String>,
        ) {
            self.decides
                .lock()
                .unwrap()
                .push((session, request_id, decision, reason));
        }

        fn answer(
            &self,
            session: manifest_domain::SessionId,
            request_id: String,
            answer: manifest_domain::PendingAnswer,
        ) {
            self.answers
                .lock()
                .unwrap()
                .push((session, request_id, answer));
        }
    }

    /// A fresh fake engine with empty call logs.
    fn fake_engine() -> FakeEngine {
        FakeEngine {
            calls: std::sync::Arc::new(std::sync::Mutex::new(0)),
            spawns: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            turns: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            resumes: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            steers: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            cancels: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            models: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            skills: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            histories: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            compacts: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            goals: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            fetches: std::sync::Arc::new(std::sync::Mutex::new(0)),
            decides: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            answers: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Production boot with the handle already delivered, as the
    /// subscription's first item would.
    fn live_app() -> (App, FakeEngine) {
        let fake = fake_engine();
        let mut app = App::live(manifest_mcp::ServerConfig::new("fake"), None);
        let attempt = app.connection_attempt;
        let _ = app.update(Message::Engine {
            attempt,
            input: engine::Input::Ready(engine::EngineHandle::new(fake.clone())),
        });
        (app, fake)
    }

    /// Delivers one engine input stamped with the current attempt, as
    /// the subscription maps it.
    fn deliver(app: &mut App, input: engine::Input) {
        let attempt = app.connection_attempt;
        let _ = app.update(Message::Engine { attempt, input });
    }

    /// Delivers the connected state change.
    fn deliver_connected(app: &mut App) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Connected,
            )),
        );
    }

    fn live_project() -> manifest_domain::ProjectSummary {
        manifest_domain::ProjectSummary {
            id: manifest_domain::ProjectId::new("project-live"),
            name: String::from("live"),
            workspace: None,
        }
    }

    fn live_session(id: &str) -> manifest_domain::SessionSummary {
        manifest_domain::SessionSummary {
            id: manifest_domain::SessionId::new(id),
            title: String::from(id),
            status: manifest_domain::SessionStatus::Idle,
            project: None,
            workspace: String::from("/live/workspace"),
            relative_time: String::from("now"),
            stream_dropped_deltas: 0,
        }
    }

    #[test]
    fn connected_requests_sessions_once() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        assert_eq!(app.connection, manifest_domain::Connection::Connected);
        assert_eq!(*fake.calls.lock().unwrap(), 1);
    }

    #[test]
    fn sessions_loaded_replaces_the_list() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![live_session("s-1"), live_session("s-2")],
                usage: Vec::new(),
            }),
        );
        assert_eq!(app.sessions.groups.len(), 1);
        assert_eq!(app.sessions.groups[0].sessions.len(), 2);
        assert_eq!(
            app.sessions.selected,
            manifest_domain::SessionId::new("s-1")
        );
    }

    #[test]
    fn failed_clears_engine_and_keeps_message() {
        let (mut app, _) = live_app();
        assert!(app.engine.is_some());
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Failed(String::from("boom")),
            )),
        );
        assert_eq!(
            app.connection,
            manifest_domain::Connection::Failed(String::from("boom"))
        );
        assert!(app.engine.is_none());
        assert!(app.transcript.entries.iter().any(|entry| matches!(
            &entry.kind,
            crate::transcript::EntryKind::Error { text } if text == "boom"
        )));
    }

    #[test]
    fn reconnect_bumps_attempt_and_resets_state() {
        let (mut app, _) = live_app();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Failed(String::from("boom")),
            )),
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoadFailed(
                String::from("load broke"),
            )),
        );
        let before = app.connection_attempt;
        let _ = app.update(Message::Reconnect);
        assert_eq!(app.connection_attempt, before + 1);
        assert_eq!(app.connection, manifest_domain::Connection::Connecting);
        assert!(app.engine.is_none());
    }

    #[test]
    fn stale_attempt_inputs_are_ignored() {
        let (mut app, _) = live_app();
        let stale = app.connection_attempt;
        let _ = app.update(Message::Reconnect);
        let _ = app.update(Message::Engine {
            attempt: stale,
            input: engine::Input::event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Connected,
            )),
        });
        assert_eq!(app.connection, manifest_domain::Connection::Connecting);
        assert!(app.engine.is_none());
    }

    #[test]
    fn pending_hint_fetches_pending_not_sessions() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        assert_eq!(*fake.calls.lock().unwrap(), 1);
        assert_eq!(*fake.fetches.lock().unwrap(), 0);
        for phase in [
            manifest_domain::PendingPhase::Requested,
            manifest_domain::PendingPhase::Settled,
        ] {
            deliver(
                &mut app,
                engine::Input::event(manifest_domain::EngineEvent::Stream(
                    manifest_domain::StreamEvent::Pending {
                        session: manifest_domain::SessionId::new("s-1"),
                        turn: None,
                        request_id: String::from("req-1"),
                        kind: manifest_domain::PendingKind::Approval,
                        phase,
                    },
                )),
            );
        }
        // Both hints fetch pending; neither re-lists sessions.
        assert_eq!(*fake.fetches.lock().unwrap(), 2);
        assert_eq!(*fake.calls.lock().unwrap(), 1);
    }

    fn pending_approval(session: &str, id: &str) -> manifest_domain::PendingRequest {
        manifest_domain::PendingRequest::Approval {
            session: manifest_domain::SessionId::new(session),
            request_id: String::from(id),
            approval: manifest_domain::ApprovalRequest {
                tool: String::from("bash"),
                subject_line: String::from("ls"),
                choices: Vec::new(),
            },
        }
    }

    fn pending_questions(session: &str, id: &str) -> manifest_domain::PendingRequest {
        manifest_domain::PendingRequest::Question {
            session: manifest_domain::SessionId::new(session),
            request_id: String::from(id),
            question: manifest_domain::QuestionRequest {
                questions: vec![
                    manifest_domain::Question {
                        id: String::from("q1"),
                        prompt: String::from("Colour?"),
                        options: vec![manifest_domain::QuestionOption {
                            label: String::from("Red"),
                            description: None,
                        }],
                        multi: false,
                    },
                    manifest_domain::Question {
                        id: String::from("q2"),
                        prompt: String::from("File?"),
                        options: vec![manifest_domain::QuestionOption {
                            label: String::from("alpha"),
                            description: None,
                        }],
                        multi: false,
                    },
                ],
            },
        }
    }

    /// Connected app with two listed sessions; s-1 visible.
    fn live_app_two_sessions() -> (App, FakeEngine) {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![live_session("s-1"), live_session("s-2")],
                usage: Vec::new(),
            }),
        );
        (app, fake)
    }

    #[test]
    fn hint_then_loaded_inserts_card_and_gates_send() {
        let (mut app, fake) = live_app_two_sessions();
        assert!(app.composer.send_enabled);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::Stream(
                manifest_domain::StreamEvent::Pending {
                    session: manifest_domain::SessionId::new("s-1"),
                    turn: None,
                    request_id: String::from("r1"),
                    kind: manifest_domain::PendingKind::Approval,
                    phase: manifest_domain::PendingPhase::Requested,
                },
            )),
        );
        assert_eq!(*fake.fetches.lock().unwrap(), 1);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingLoaded {
                requests: vec![pending_approval("s-1", "r1")],
            }),
        );
        assert!(app.transcript.has_pending("r1"));
        assert!(!app.composer.send_enabled, "awaiting card gates send");
        assert_eq!(
            app.composer.pending_hint.as_deref(),
            Some("Answer the pending card to continue")
        );
        assert_eq!(
            app.sessions
                .pending_count(&manifest_domain::SessionId::new("s-1")),
            1
        );
        // Stop path is untouched: running state still drives it.
        assert_eq!(
            app.pending
                .pending_count(&manifest_domain::SessionId::new("s-1")),
            1
        );
    }

    #[test]
    fn approve_flow_decides_then_settles_to_record() {
        let (mut app, fake) = live_app_two_sessions();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingLoaded {
                requests: vec![pending_approval("s-1", "r1")],
            }),
        );
        let _ = app.update(Message::Pending(crate::pending::Message::Approve {
            request: String::from("r1"),
        }));
        assert_eq!(fake.decides.lock().unwrap().len(), 1);
        assert_eq!(
            fake.decides.lock().unwrap()[0].2,
            manifest_domain::ApprovalDecision::Approve
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingSettled {
                session: manifest_domain::SessionId::new("s-1"),
                request_id: String::from("r1"),
                resolution: manifest_domain::PendingResolution::Approved,
            }),
        );
        assert!(!app.transcript.has_pending("r1"));
        assert!(app.transcript.entries.iter().any(|entry| matches!(
            &entry.kind,
            crate::transcript::EntryKind::Note { text } if text.contains("Approved by you")
        )));
        assert!(app.composer.send_enabled, "settled card releases gate");
    }

    #[test]
    fn transcript_pending_approve_decides_like_direct_pending() {
        let (mut app, fake) = live_app_two_sessions();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingLoaded {
                requests: vec![pending_approval("s-1", "r1")],
            }),
        );
        let _ = app.update(Message::Transcript(crate::transcript::Message::Pending(
            crate::pending::Message::Approve {
                request: String::from("r1"),
            },
        )));
        assert_eq!(fake.decides.lock().unwrap().len(), 1);
        assert_eq!(
            fake.decides.lock().unwrap()[0].2,
            manifest_domain::ApprovalDecision::Approve
        );
        assert_eq!(
            app.pending.card("r1").unwrap().phase,
            crate::pending::Phase::Deciding
        );
    }

    #[test]
    fn decide_failure_marks_card_failed_and_retry_refetches() {
        let (mut app, fake) = live_app_two_sessions();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingLoaded {
                requests: vec![pending_approval("s-1", "r1")],
            }),
        );
        let _ = app.update(Message::Pending(crate::pending::Message::Approve {
            request: String::from("r1"),
        }));
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::Decide(
                    manifest_domain::SessionId::new("s-1"),
                    String::from("r1"),
                ),
                message: String::from("nope"),
            }),
        );
        assert!(matches!(
            app.pending.card("r1").unwrap().phase,
            crate::pending::Phase::Failed(ref m) if m == "nope"
        ));
        assert!(app.transcript.has_pending("r1"), "failed card stays");
        let before = *fake.fetches.lock().unwrap();
        let _ = app.update(Message::Pending(crate::pending::Message::RetryFetch {
            request: String::from("r1"),
        }));
        assert_eq!(*fake.fetches.lock().unwrap(), before + 1);
    }

    #[test]
    fn hinted_fetch_failure_materializes_placeholder_and_retry_revives() {
        let (mut app, fake) = live_app_two_sessions();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::Stream(
                manifest_domain::StreamEvent::Pending {
                    session: manifest_domain::SessionId::new("s-1"),
                    turn: None,
                    request_id: String::from("r1"),
                    kind: manifest_domain::PendingKind::Approval,
                    phase: manifest_domain::PendingPhase::Requested,
                },
            )),
        );
        assert_eq!(*fake.fetches.lock().unwrap(), 1);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::PendingFetch,
                message: String::from("write EPIPE"),
            }),
        );
        let card = app
            .pending
            .card("r1")
            .expect("failed fetch must materialize a placeholder card");
        assert!(matches!(
            card.phase,
            crate::pending::Phase::Failed(ref m) if m == "write EPIPE"
        ));
        assert!(
            app.transcript.has_pending("r1"),
            "placeholder has a transcript entry"
        );
        assert!(
            app.composer.send_enabled,
            "a Failed placeholder must not gate send"
        );
        // Retry re-fetches truth.
        let before = *fake.fetches.lock().unwrap();
        let _ = app.update(Message::Pending(crate::pending::Message::RetryFetch {
            request: String::from("r1"),
        }));
        assert_eq!(*fake.fetches.lock().unwrap(), before + 1);
        // The successful fetch revives the placeholder with real payload.
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingLoaded {
                requests: vec![pending_approval("s-1", "r1")],
            }),
        );
        let card = app.pending.card("r1").unwrap();
        assert_eq!(card.phase, crate::pending::Phase::Awaiting);
        match &card.request {
            manifest_domain::PendingRequest::Approval { approval, .. } => {
                assert_eq!(approval.tool, "bash");
                assert_eq!(approval.subject_line, "ls");
            }
            other => panic!("expected revived approval payload, got {other:?}"),
        }
        assert!(app.transcript.has_pending("r1"));
        assert!(!app.composer.send_enabled, "revived awaiting card gates");
        // Hint was cleared by the load: a later global fetch failure
        // must not fail the now-loaded awaiting card.
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::PendingFetch,
                message: String::from("write EPIPE"),
            }),
        );
        assert_eq!(
            app.pending.card("r1").unwrap().phase,
            crate::pending::Phase::Awaiting
        );
    }

    #[test]
    fn settled_hint_fetch_failure_creates_no_placeholder() {
        let (mut app, _fake) = live_app_two_sessions();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::Stream(
                manifest_domain::StreamEvent::Pending {
                    session: manifest_domain::SessionId::new("s-1"),
                    turn: None,
                    request_id: String::from("r1"),
                    kind: manifest_domain::PendingKind::Approval,
                    phase: manifest_domain::PendingPhase::Settled,
                },
            )),
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::PendingFetch,
                message: String::from("write EPIPE"),
            }),
        );
        assert!(app.pending.card("r1").is_none());
        assert!(!app.transcript.has_pending("r1"));
    }

    #[test]
    fn hinted_fetch_failure_parks_placeholder_for_background_session() {
        let (mut app, _fake) = live_app_two_sessions();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::Stream(
                manifest_domain::StreamEvent::Pending {
                    session: manifest_domain::SessionId::new("s-2"),
                    turn: None,
                    request_id: String::from("r9"),
                    kind: manifest_domain::PendingKind::UserInput,
                    phase: manifest_domain::PendingPhase::Requested,
                },
            )),
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::PendingFetch,
                message: String::from("write EPIPE"),
            }),
        );
        assert!(matches!(
            app.pending.card("r9").unwrap().phase,
            crate::pending::Phase::Failed(_)
        ));
        assert!(!app.transcript.has_pending("r9"), "visible is s-1");
        let _ = app.update(Message::Sessions(crate::sessions::Message::Select(
            manifest_domain::SessionId::new("s-2"),
        )));
        assert!(
            app.transcript.has_pending("r9"),
            "parked placeholder entry shows on switch"
        );
    }

    #[test]
    fn answer_flow_sends_assembled_answer() {
        let (mut app, fake) = live_app_two_sessions();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingLoaded {
                requests: vec![pending_questions("s-1", "r2")],
            }),
        );
        let _ = app.update(Message::Pending(crate::pending::Message::SelectOption {
            request: String::from("r2"),
            question: String::from("q1"),
            label: String::from("Red"),
        }));
        let _ = app.update(Message::Pending(crate::pending::Message::PagerNext {
            request: String::from("r2"),
        }));
        let _ = app.update(Message::Pending(crate::pending::Message::SelectOption {
            request: String::from("r2"),
            question: String::from("q2"),
            label: String::from("alpha"),
        }));
        let _ = app.update(Message::Pending(crate::pending::Message::SubmitAnswers {
            request: String::from("r2"),
        }));
        let answers = fake.answers.lock().unwrap();
        assert_eq!(answers.len(), 1);
        assert_eq!(
            answers[0].2.get("q1"),
            Some(&manifest_domain::AnswerValue::Label(String::from("Red")))
        );
        assert_eq!(
            answers[0].2.get("q2"),
            Some(&manifest_domain::AnswerValue::Label(String::from("alpha")))
        );
    }

    #[test]
    fn expiry_vanish_collapses_to_expired_record() {
        let (mut app, _) = live_app_two_sessions();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingLoaded {
                requests: vec![pending_approval("s-1", "r1")],
            }),
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingLoaded {
                requests: Vec::new(),
            }),
        );
        assert!(!app.transcript.has_pending("r1"));
        assert!(app.transcript.entries.iter().any(|entry| matches!(
            &entry.kind,
            crate::transcript::EntryKind::Note { text } if text.contains("Expired")
        )));
        assert!(app.composer.send_enabled);
    }

    #[test]
    fn background_count_and_session_switch_preserves_pager() {
        let (mut app, _) = live_app_two_sessions();
        // Background session s-2 gets a question card while s-1 shows.
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::PendingLoaded {
                requests: vec![pending_questions("s-2", "r2")],
            }),
        );
        assert_eq!(
            app.pending
                .pending_count(&manifest_domain::SessionId::new("s-2")),
            1
        );
        assert!(app.composer.send_enabled, "background card does not gate");
        // Switch to s-2, advance pager, switch away and back.
        let _ = app.update(Message::Sessions(crate::sessions::Message::Select(
            manifest_domain::SessionId::new("s-2"),
        )));
        assert!(app.transcript.has_pending("r2"));
        assert!(!app.composer.send_enabled);
        let _ = app.update(Message::Pending(crate::pending::Message::PagerNext {
            request: String::from("r2"),
        }));
        let _ = app.update(Message::Sessions(crate::sessions::Message::Select(
            manifest_domain::SessionId::new("s-1"),
        )));
        let _ = app.update(Message::Sessions(crate::sessions::Message::Select(
            manifest_domain::SessionId::new("s-2"),
        )));
        assert_eq!(
            app.pending.card("r2").unwrap().interaction.pager_index,
            1,
            "pager survives session switches (state is per-session)"
        );
        assert!(app.transcript.has_pending("r2"));
    }

    #[test]
    fn load_failed_and_diagnostic_append_error_notes() {
        let (mut app, _) = live_app();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoadFailed(
                String::from("x"),
            )),
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::Diagnostic(String::from("y"))),
        );
        let tail: Vec<&str> = app
            .transcript
            .entries
            .iter()
            .rev()
            .take(2)
            .rev()
            .filter_map(|entry| match &entry.kind {
                crate::transcript::EntryKind::Error { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        // Both events append; the tail holds exactly the two texts.
        assert!(app.transcript.entries.len() >= 2);
        assert_eq!(tail, vec!["x", "y"]);
    }

    #[test]
    fn connected_and_connecting_append_no_error_notes() {
        let (mut app, _) = live_app();
        let before = app.transcript.entries.len();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Connecting,
            )),
        );
        deliver_connected(&mut app);
        assert_eq!(app.transcript.entries.len(), before);
    }

    #[test]
    fn fixture_boot_stays_offline() {
        let app = App::boot();
        assert_eq!(app.connection, manifest_domain::Connection::Disconnected);
        assert!(app.engine_config.is_none());
        assert!(app.engine.is_none());
    }

    #[test]
    fn fixture_mode_renders_pending_cards_without_engine() {
        let app = App::boot();
        let awaiting = manifest_domain::SessionId::new("session-settings-copy");
        assert!(app.engine.is_none(), "cards render without an engine");
        assert_eq!(app.visible_session, Some(awaiting.clone()));
        // Question card (3 questions) and approval card stay pending;
        // the settled fixture approval collapsed to its record, so the
        // 14 fixture transcript items gain exactly 2 pending entries
        // and 1 settled note.
        assert_eq!(app.pending.pending_count(&awaiting), 2);
        assert_eq!(
            app.pending
                .card("fixture-question-1")
                .and_then(|card| card.current_question())
                .map(|q| q.prompt.as_str()),
            Some("Which token refresh should settings sync use?")
        );
        assert!(app.transcript.has_pending("fixture-question-1"));
        assert!(app.transcript.has_pending("fixture-approval-1"));
        assert!(!app.transcript.has_pending("fixture-settled-1"));
        assert_eq!(app.transcript.entries.len(), 14 + 3);
        assert!(app.transcript.entries.iter().any(|entry| matches!(
            &entry.kind,
            crate::transcript::EntryKind::Note { text }
                if text == "Approval · Run the checkout sync suite — Approved by you"
        )));
        // Background badge, hint, and the inline card all read the seed.
        assert_eq!(app.sessions.pending_count(&awaiting), 2);
        assert_eq!(
            app.composer.pending_hint.as_deref(),
            Some("Answer the pending card to continue")
        );
        assert!(
            crate::transcript::pending_element(
                &app.pending,
                app.visible_session.as_ref(),
                "fixture-question-1"
            )
            .is_some()
        );
        let _ = app.view();
    }

    #[test]
    fn live_boot_clears_fixture_pending_seed() {
        let (app, _) = live_app();
        let awaiting = manifest_domain::SessionId::new("session-settings-copy");
        assert_eq!(app.pending.pending_count(&awaiting), 0);
        assert!(app.pending.card("fixture-question-1").is_none());
        assert_eq!(app.visible_session, None);
        assert!(app.transcript.entries.is_empty());
    }

    /// Loads a live session list with the given ids.
    fn loaded(app: &mut App, ids: &[&str]) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: ids.iter().map(|id| live_session(id)).collect(),
                usage: Vec::new(),
            }),
        );
    }

    /// Delivers one stream event to the app.
    fn stream(app: &mut App, event: manifest_domain::StreamEvent) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::Stream(event)),
        );
    }

    /// A completed assistant item event for the given session.
    fn agent_completed(session: &str, item: &str, text: &str) -> manifest_domain::StreamEvent {
        manifest_domain::StreamEvent::ItemCompleted {
            session: manifest_domain::SessionId::new(session),
            turn: None,
            view_cursor: None,
            item: manifest_domain::StreamItem::AgentMessage {
                item_id: String::from(item),
                text: String::from(text),
                phase: None,
            },
        }
    }

    /// Entries in a background session's stored transcript.
    fn stored_len(app: &App, session: &str) -> usize {
        app.transcripts
            .get(&manifest_domain::SessionId::new(session))
            .map(|state| state.entries.len())
            .unwrap_or(usize::MAX)
    }

    #[test]
    fn background_stream_leaves_visible_transcript_untouched() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1", "s-2"]);
        assert_eq!(app.visible_session, Some(SessionId::new("s-1")));
        stream(
            &mut app,
            manifest_domain::StreamEvent::TurnStarted {
                session: manifest_domain::SessionId::new("s-2"),
                turn: None,
                view_cursor: None,
            },
        );
        assert!(app.running.contains(&SessionId::new("s-2")));
        stream(&mut app, agent_completed("s-2", "a-1", "background reply"));
        assert!(app.transcript.entries.is_empty());
        assert_eq!(stored_len(&app, "s-2"), 1);
    }

    #[test]
    fn switching_selection_parks_and_restores_transcripts() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1", "s-2"]);
        stream(&mut app, agent_completed("s-1", "a-1", "one"));
        assert_eq!(app.transcript.entries.len(), 1);
        let second = SessionId::new("s-2");
        let _ = app.update(Message::Sessions(sessions::Message::Select(second.clone())));
        // The s-1 transcript parks; s-2 shows empty.
        assert_eq!(app.visible_session, Some(second));
        assert!(app.transcript.entries.is_empty());
        assert_eq!(stored_len(&app, "s-1"), 1);
        stream(&mut app, agent_completed("s-2", "a-2", "two"));
        assert_eq!(app.transcript.entries.len(), 1);
        // Switching back restores s-1 without loss.
        let first = SessionId::new("s-1");
        let _ = app.update(Message::Sessions(sessions::Message::Select(first.clone())));
        assert_eq!(app.visible_session, Some(first));
        assert_eq!(app.transcript.entries.len(), 1);
        assert!(matches!(
            &app.transcript.entries[0].kind,
            crate::transcript::EntryKind::Assistant { text, .. } if text == "one"
        ));
        assert_eq!(stored_len(&app, "s-2"), 1);
    }

    #[test]
    fn turn_completed_with_dropped_deltas_adds_error_and_clears_running() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1", "s-2"]);
        let before = *fake.calls.lock().unwrap();
        let first = SessionId::new("s-1");
        stream(
            &mut app,
            manifest_domain::StreamEvent::TurnStarted {
                session: first.clone(),
                turn: None,
                view_cursor: None,
            },
        );
        assert!(app.running.contains(&first));
        stream(
            &mut app,
            manifest_domain::StreamEvent::TurnCompleted {
                session: first.clone(),
                turn: None,
                view_cursor: None,
                terminal: None,
                answer: None,
                dropped_deltas: 2,
            },
        );
        assert!(!app.running.contains(&first));
        assert!(matches!(
            app.transcript.entries.last(),
            Some(entry) if matches!(
                &entry.kind,
                crate::transcript::EntryKind::Error { text }
                    if text == "The stream dropped 2 updates; the final message is complete."
            )
        ));
        // A completed turn refreshes the session list while connected.
        assert_eq!(*fake.calls.lock().unwrap(), before + 1);
        // The singular names one update on a background session.
        let second = SessionId::new("s-2");
        stream(
            &mut app,
            manifest_domain::StreamEvent::TurnCompleted {
                session: second.clone(),
                turn: None,
                view_cursor: None,
                terminal: None,
                answer: None,
                dropped_deltas: 1,
            },
        );
        assert!(matches!(
            app.transcripts.get(&second).and_then(|state| state.entries.last()),
            Some(entry) if matches!(
                &entry.kind,
                crate::transcript::EntryKind::Error { text }
                    if text == "The stream dropped 1 update; the final message is complete."
            )
        ));
    }

    #[test]
    fn spawned_session_selects_on_load() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let before = *fake.calls.lock().unwrap();
        let spawned = SessionId::new("s-9");
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionSpawned {
                session: spawned.clone(),
                turn: None,
            }),
        );
        assert!(app.running.contains(&spawned));
        assert_eq!(app.select_on_load, Some(spawned.clone()));
        assert_eq!(*fake.calls.lock().unwrap(), before + 1);
        loaded(&mut app, &["s-1", "s-9"]);
        assert_eq!(app.sessions.selected, spawned);
        assert_eq!(app.select_on_load, None);
        assert_eq!(app.visible_session, Some(spawned));
    }

    #[test]
    fn command_failed_clears_visible_running() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let first = SessionId::new("s-1");
        stream(
            &mut app,
            manifest_domain::StreamEvent::TurnStarted {
                session: first.clone(),
                turn: None,
                view_cursor: None,
            },
        );
        assert!(app.running.contains(&first));
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::Turn(first.clone()),
                message: String::from("nope"),
            }),
        );
        assert!(!app.running.contains(&first));
        assert!(matches!(
            app.transcript.entries.last(),
            Some(entry) if matches!(
                &entry.kind,
                crate::transcript::EntryKind::Error { text } if text == "nope"
            )
        ));
    }

    /// Delivers a tagged command failure, as the worker now emits it.
    fn command_failed(app: &mut App, command: manifest_domain::FailedCommand, message: &str) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command,
                message: String::from(message),
            }),
        );
    }

    #[test]
    fn steer_failure_keeps_running_and_catalog() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        deliver(&mut app, models_event());
        assert!(matches!(
            app.composer.catalog,
            composer::ModelCatalog::Ready(_)
        ));
        assert!(!app.models_pending);
        start_turn(&mut app, "s-1", "turn-7");
        let first = SessionId::new("s-1");
        assert!(app.running.contains(&first));
        assert!(app.composer.running_visible);
        command_failed(
            &mut app,
            manifest_domain::FailedCommand::Steer(first.clone()),
            "steer broke",
        );
        // The turn is still running: nothing about it is consumed.
        assert!(app.running.contains(&first));
        assert_eq!(
            app.running_turns.get(&first),
            Some(&manifest_domain::TurnId::new("turn-7"))
        );
        assert!(app.composer.running_visible);
        // The ready catalog is untouched by another command's failure.
        assert!(matches!(
            app.composer.catalog,
            composer::ModelCatalog::Ready(_)
        ));
        assert!(!app.models_pending);
        assert!(transcript_errors(&app).contains(&"steer broke"));
    }

    #[test]
    fn cancel_failure_keeps_running() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        start_turn(&mut app, "s-1", "turn-7");
        let first = SessionId::new("s-1");
        assert!(app.running.contains(&first));
        command_failed(
            &mut app,
            manifest_domain::FailedCommand::Cancel(first.clone()),
            "cancel broke",
        );
        assert!(app.running.contains(&first));
        assert_eq!(
            app.running_turns.get(&first),
            Some(&manifest_domain::TurnId::new("turn-7"))
        );
        assert!(app.composer.running_visible);
        assert!(transcript_errors(&app).contains(&"cancel broke"));
    }

    #[test]
    fn turn_failure_clears_only_its_session() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1", "s-2"]);
        start_turn(&mut app, "s-1", "turn-1");
        start_turn(&mut app, "s-2", "turn-2");
        let first = SessionId::new("s-1");
        let second = SessionId::new("s-2");
        assert!(app.running.contains(&first));
        assert!(app.running.contains(&second));
        command_failed(
            &mut app,
            manifest_domain::FailedCommand::Turn(first.clone()),
            "turn broke",
        );
        assert!(!app.running.contains(&first));
        assert!(!app.running_turns.contains_key(&first));
        // The other session's turn is untouched.
        assert!(app.running.contains(&second));
        assert_eq!(
            app.running_turns.get(&second),
            Some(&manifest_domain::TurnId::new("turn-2"))
        );
        assert!(transcript_errors(&app).contains(&"turn broke"));
    }

    #[test]
    fn non_spawn_failure_leaves_awaiting_spawn_intact() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        assert!(app.sessions.selected_summary().is_none());
        type_composer_text(&mut app, "start me");
        send_pressed(&mut app);
        assert!(app.awaiting_spawn);
        assert!(app.pending_spawn.is_some());
        command_failed(
            &mut app,
            manifest_domain::FailedCommand::Turn(SessionId::new("s-1")),
            "unrelated broke",
        );
        // Another command's failure consumes neither the in-flight
        // spawn nor its restorable prompt.
        assert!(app.awaiting_spawn);
        let pending = app.pending_spawn.as_ref().expect("spawn still pending");
        assert_eq!(pending.prompt, "start me");
        assert_eq!(app.composer.content.text(), "");
        assert!(transcript_errors(&app).contains(&"unrelated broke"));
        // A later spawn failure still restores the prompt.
        command_failed(
            &mut app,
            manifest_domain::FailedCommand::Spawn,
            "spawn broke",
        );
        assert!(!app.awaiting_spawn);
        assert!(app.pending_spawn.is_none());
        assert_eq!(app.composer.content.text(), "start me");
        assert!(transcript_errors(&app).contains(&"spawn broke"));
    }

    #[test]
    fn failed_connection_clears_running() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let first = SessionId::new("s-1");
        stream(
            &mut app,
            manifest_domain::StreamEvent::TurnStarted {
                session: first.clone(),
                turn: None,
                view_cursor: None,
            },
        );
        assert!(app.running.contains(&first));
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Failed(String::from("down")),
            )),
        );
        assert!(app.running.is_empty());
    }

    #[test]
    fn panel_drag_ends_and_idle_moves_are_noops() {
        let mut got = App::boot();
        let _ = got.update(Message::PanelDragMoved(iced::Point::new(500.0, 0.0)));
        assert_eq!(got.panel_width, PANEL_WIDTH_DEFAULT);
        assert_eq!(got.panel_drag, PanelDrag::Idle);
        let _ = got.update(Message::PanelDragStarted);
        let _ = got.update(Message::PanelDragMoved(iced::Point::new(260.0, 0.0)));
        let _ = got.update(Message::PanelDragMoved(iced::Point::new(320.0, 0.0)));
        assert_eq!(got.panel_width, 320.0);
        let _ = got.update(Message::PanelDragEnded);
        assert_eq!(got.panel_drag, PanelDrag::Idle);
        let _ = got.update(Message::PanelDragMoved(iced::Point::new(200.0, 0.0)));
        assert_eq!(got.panel_width, 320.0);
        assert_eq!(got.panel_drag, PanelDrag::Idle);
    }

    /// Presses the composer send button through the root reducer, as
    /// the view's send button would.
    fn send_pressed(app: &mut App) {
        let _ = app.update(Message::Composer(composer::Message::SendPressed));
    }

    /// A completed userMessage event, for the echo-claim path.
    fn user_completed(session: &str, item: &str, text: &str) -> manifest_domain::StreamEvent {
        manifest_domain::StreamEvent::ItemCompleted {
            session: manifest_domain::SessionId::new(session),
            turn: None,
            view_cursor: None,
            item: manifest_domain::StreamItem::UserMessage {
                item_id: String::from(item),
                text: String::from(text),
            },
        }
    }

    /// Prompt texts of the user entries in the visible transcript.
    fn user_entries(app: &App) -> Vec<&str> {
        app.transcript
            .entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                crate::transcript::EntryKind::User { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn live_boot_is_empty_and_connecting() {
        let app = App::live(manifest_mcp::ServerConfig::new("fake"), None);
        assert!(app.transcript.entries.is_empty());
        assert!(app.todo_lists.is_empty());
        assert!(app.sessions.groups.is_empty());
        assert!(app.sessions.unprojected.is_empty());
        assert_eq!(app.connection, manifest_domain::Connection::Connecting);
        assert!(!app.composer.send_enabled);
    }

    #[test]
    fn send_to_selected_session_sends_turn_and_clears_draft() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        assert!(app.composer.send_enabled);
        type_composer_text(&mut app, "hello");
        send_pressed(&mut app);
        assert_eq!(
            *fake.turns.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                manifest_domain::TurnInput::Text(String::from("hello"))
            )]
        );
        assert!(fake.spawns.lock().unwrap().is_empty());
        assert_eq!(user_entries(&app), vec!["hello"]);
        assert_eq!(app.composer.content.text(), "");
        assert!(!app.composer.has_text);
        assert!(app.running.contains(&SessionId::new("s-1")));
        // Steer mode: Send stays live while the session runs.
        assert!(app.composer.send_enabled);
        assert!(app.composer.running_visible);
    }

    #[test]
    fn second_send_while_running_records_nothing() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        type_composer_text(&mut app, "first");
        send_pressed(&mut app);
        assert_eq!(fake.turns.lock().unwrap().len(), 1);
        type_composer_text(&mut app, "second");
        send_pressed(&mut app);
        assert_eq!(fake.turns.lock().unwrap().len(), 1);
        assert_eq!(user_entries(&app), vec!["first"]);
    }

    #[test]
    fn send_while_disconnected_notes_error_and_keeps_draft() {
        let (mut app, fake) = live_app();
        type_composer_text(&mut app, "draft");
        send_pressed(&mut app);
        assert!(fake.turns.lock().unwrap().is_empty());
        assert!(fake.spawns.lock().unwrap().is_empty());
        assert!(matches!(
            app.transcript.entries.last(),
            Some(entry) if matches!(
                &entry.kind,
                crate::transcript::EntryKind::Error { text }
                    if text == "The Muse MCP server isn't connected yet. Try again once it connects."
            )
        ));
        assert_eq!(app.composer.content.text(), "draft");
        assert!(!app.composer.send_enabled);
    }

    /// One catalog skill for the slash tests.
    fn review_skill() -> manifest_domain::SkillInfo {
        manifest_domain::SkillInfo {
            selector: String::from("review"),
            display_name: String::from("Review"),
            description: String::from("Reviews code"),
            source: String::from("user"),
            plugin_id: None,
            argument_hint: None,
        }
    }

    /// A connected app with one listed session and its skill catalog
    /// loaded, as the subscription would deliver it.
    fn slash_app() -> (App, FakeEngine) {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        assert_eq!(
            *fake.skills.lock().unwrap(),
            vec![SessionId::new("s-1")],
            "the list load requests the visible session's catalog"
        );
        assert!(app.skills_pending);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SkillsLoaded {
                session: SessionId::new("s-1"),
                skills: vec![review_skill()],
            }),
        );
        (app, fake)
    }

    #[test]
    fn skills_loaded_for_visible_sets_ready() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        assert!(app.skills_pending);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SkillsLoaded {
                session: SessionId::new("s-1"),
                skills: vec![review_skill()],
            }),
        );
        assert!(!app.skills_pending);
        assert_eq!(
            app.composer.skill_catalog,
            composer::SkillCatalog::Ready(vec![review_skill()])
        );
    }

    #[test]
    fn skills_loaded_for_other_session_does_not_clobber() {
        let (mut app, fake) = slash_app();
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SkillsLoaded {
                session: SessionId::new("s-2"),
                skills: Vec::new(),
            }),
        );
        assert_eq!(
            app.composer.skill_catalog,
            composer::SkillCatalog::Ready(vec![review_skill()])
        );
        assert!(!app.skills_pending);
        assert_eq!(
            *fake.skills.lock().unwrap(),
            vec![SessionId::new("s-1")],
            "no re-list follows an ignored load"
        );
    }

    #[test]
    fn skills_failed_for_visible_marks_failed() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        assert!(app.skills_pending);
        command_failed(
            &mut app,
            manifest_domain::FailedCommand::Skills(SessionId::new("s-1")),
            "skills broke",
        );
        assert!(!app.skills_pending);
        assert_eq!(
            app.composer.skill_catalog,
            composer::SkillCatalog::Failed(String::from("skills broke"))
        );
    }

    #[test]
    fn send_slash_skill_records_skill_turn_and_clears_draft() {
        let (mut app, fake) = slash_app();
        type_composer_text(&mut app, "/review fix it");
        send_pressed(&mut app);
        assert_eq!(
            *fake.turns.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                manifest_domain::TurnInput::Skill(manifest_domain::SkillInvocation {
                    selector: String::from("review"),
                    source: Some(String::from("user")),
                    arguments: String::from("fix it"),
                })
            )]
        );
        assert_eq!(user_entries(&app), vec!["/review fix it"]);
        assert_eq!(app.composer.content.text(), "");
    }

    #[test]
    fn plain_send_with_ready_catalog_stays_text() {
        let (mut app, fake) = slash_app();
        type_composer_text(&mut app, "hello");
        send_pressed(&mut app);
        assert_eq!(
            *fake.turns.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                manifest_domain::TurnInput::Text(String::from("hello"))
            )]
        );
    }

    #[test]
    fn unknown_slash_sends_text() {
        let (mut app, fake) = slash_app();
        type_composer_text(&mut app, "/nope fix it");
        send_pressed(&mut app);
        assert_eq!(
            *fake.turns.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                manifest_domain::TurnInput::Text(String::from("/nope fix it"))
            )]
        );
    }

    #[test]
    fn goal_changed_shows_visible_goal_in_composer() {
        let (mut app, _) = slash_app();
        assert!(app.composer.goal_display.is_none());
        stream(
            &mut app,
            manifest_domain::StreamEvent::GoalChanged {
                session: SessionId::new("s-1"),
                goal: Some(manifest_domain::GoalInfo {
                    objective: String::from("Ship the slice"),
                    percent_complete: 42.0,
                    status: manifest_domain::GoalStatus::Active,
                    current_work: Some(String::from("Wiring the mapper")),
                    next_work: None,
                }),
            },
        );
        let display = app.composer.goal_display.as_ref().expect("strip shows");
        assert_eq!(display.objective, "Ship the slice");
        assert_eq!(display.percent, 42.0);
        assert_eq!(display.percent_label, "42%");
        assert_eq!(
            display.current_work,
            Some(String::from("Wiring the mapper"))
        );
    }

    #[test]
    fn goal_changed_for_other_session_does_not_clobber_visible() {
        let (mut app, _) = slash_app();
        stream(
            &mut app,
            manifest_domain::StreamEvent::GoalChanged {
                session: SessionId::new("s-9"),
                goal: Some(manifest_domain::GoalInfo {
                    objective: String::from("Other work"),
                    percent_complete: 10.0,
                    status: manifest_domain::GoalStatus::Active,
                    current_work: None,
                    next_work: None,
                }),
            },
        );
        assert!(app.composer.goal_display.is_none());
        assert_eq!(
            app.goals
                .get(&SessionId::new("s-9"))
                .and_then(|goal| goal.as_ref())
                .map(|goal| goal.objective.as_str()),
            Some("Other work")
        );
    }

    #[test]
    fn goal_changed_clear_removes_the_strip() {
        let (mut app, _) = slash_app();
        stream(
            &mut app,
            manifest_domain::StreamEvent::GoalChanged {
                session: SessionId::new("s-1"),
                goal: Some(manifest_domain::GoalInfo {
                    objective: String::from("Ship the slice"),
                    percent_complete: 42.0,
                    status: manifest_domain::GoalStatus::Active,
                    current_work: None,
                    next_work: None,
                }),
            },
        );
        assert!(app.composer.goal_display.is_some());
        stream(
            &mut app,
            manifest_domain::StreamEvent::GoalChanged {
                session: SessionId::new("s-1"),
                goal: None,
            },
        );
        assert!(app.composer.goal_display.is_none());
    }

    #[test]
    fn catalog_changed_relits_visible_once() {
        let (mut app, fake) = slash_app();
        assert_eq!(fake.skills.lock().unwrap().len(), 1);
        stream(
            &mut app,
            manifest_domain::StreamEvent::CatalogChanged {
                session: SessionId::new("s-1"),
            },
        );
        assert_eq!(
            *fake.skills.lock().unwrap(),
            vec![SessionId::new("s-1"), SessionId::new("s-1")]
        );
        assert!(app.skills_pending);
        // A second hint while the re-list is in flight doubles nothing.
        stream(
            &mut app,
            manifest_domain::StreamEvent::CatalogChanged {
                session: SessionId::new("s-1"),
            },
        );
        assert_eq!(fake.skills.lock().unwrap().len(), 2);
        // Other sessions' hints are ignored.
        stream(
            &mut app,
            manifest_domain::StreamEvent::CatalogChanged {
                session: SessionId::new("s-9"),
            },
        );
        assert_eq!(fake.skills.lock().unwrap().len(), 2);
    }

    #[test]
    fn send_model_opens_menu_without_sending() {
        let (mut app, fake) = slash_app();
        type_composer_text(&mut app, "/model");
        send_pressed(&mut app);
        assert!(fake.turns.lock().unwrap().is_empty());
        assert!(fake.spawns.lock().unwrap().is_empty());
        assert_eq!(app.composer.model_menu, composer::ModelMenu::Open);
        assert_eq!(app.composer.content.text(), "/model");
    }

    #[test]
    fn send_compact_compacts_visible_and_clears_draft() {
        let (mut app, fake) = slash_app();
        type_composer_text(&mut app, "/compact");
        send_pressed(&mut app);
        assert_eq!(*fake.compacts.lock().unwrap(), vec![SessionId::new("s-1")]);
        assert!(fake.turns.lock().unwrap().is_empty());
        assert_eq!(app.composer.content.text(), "");
    }

    #[test]
    fn send_goal_sets_objective_and_clears_draft() {
        let (mut app, fake) = slash_app();
        type_composer_text(&mut app, "/goal ship the slice");
        send_pressed(&mut app);
        assert_eq!(
            *fake.goals.lock().unwrap(),
            vec![(SessionId::new("s-1"), String::from("ship the slice"))]
        );
        assert!(fake.turns.lock().unwrap().is_empty());
        assert_eq!(app.composer.content.text(), "");
    }

    #[test]
    fn send_bare_goal_keeps_draft_and_sends_nothing() {
        let (mut app, fake) = slash_app();
        type_composer_text(&mut app, "/goal");
        send_pressed(&mut app);
        assert!(fake.goals.lock().unwrap().is_empty());
        assert!(fake.turns.lock().unwrap().is_empty());
        assert_eq!(app.composer.content.text(), "/goal");
    }

    #[test]
    fn selecting_another_session_relits_its_catalog() {
        let (mut app, fake) = slash_app();
        // A routine list refresh never re-lists a ready catalog.
        loaded(&mut app, &["s-1", "s-2"]);
        assert_eq!(
            *fake.skills.lock().unwrap(),
            vec![SessionId::new("s-1")],
            "no re-list on refresh"
        );
        let _ = app.update(Message::Sessions(crate::sessions::Message::Select(
            SessionId::new("s-2"),
        )));
        assert_eq!(
            *fake.skills.lock().unwrap(),
            vec![SessionId::new("s-1"), SessionId::new("s-2")]
        );
        assert!(app.skills_pending);
    }

    #[test]
    fn steer_with_slash_draft_stays_text() {
        let (mut app, fake) = slash_app();
        start_turn(&mut app, "s-1", "turn-7");
        type_composer_text(&mut app, "/review fix it");
        send_pressed(&mut app);
        assert_eq!(
            *fake.steers.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                String::from("/review fix it"),
                manifest_domain::TurnId::new("turn-7")
            )]
        );
        assert!(fake.turns.lock().unwrap().is_empty());
    }

    #[test]
    fn compact_and_goal_failures_surface_on_transcript() {
        let (mut app, _) = slash_app();
        command_failed(
            &mut app,
            manifest_domain::FailedCommand::Compact(SessionId::new("s-1")),
            "compact broke",
        );
        command_failed(
            &mut app,
            manifest_domain::FailedCommand::Goal(SessionId::new("s-1")),
            "goal broke",
        );
        let errors: Vec<&str> = app
            .transcript
            .entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                crate::transcript::EntryKind::Error { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(errors.contains(&"compact broke"));
        assert!(errors.contains(&"goal broke"));
    }

    #[test]
    fn spawn_path_records_workspace_and_adopts_on_spawned() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        assert!(app.sessions.selected_summary().is_none());
        type_composer_text(&mut app, "start me");
        send_pressed(&mut app);
        let workspace = std::env::current_dir().unwrap().display().to_string();
        assert_eq!(
            *fake.spawns.lock().unwrap(),
            vec![(
                workspace,
                manifest_domain::TurnInput::Text(String::from("start me")),
                manifest_domain::SpawnChoices::default()
            )]
        );
        assert!(fake.turns.lock().unwrap().is_empty());
        assert!(app.awaiting_spawn);
        assert_eq!(user_entries(&app), vec!["start me"]);
        assert_eq!(app.composer.content.text(), "");
        let spawned = SessionId::new("s-9");
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionSpawned {
                session: spawned.clone(),
                turn: None,
            }),
        );
        assert!(!app.awaiting_spawn);
        assert_eq!(app.visible_session, Some(spawned.clone()));
        assert_eq!(user_entries(&app), vec!["start me"]);
        stream(&mut app, user_completed("s-9", "item-1", "start me"));
        assert_eq!(user_entries(&app), vec!["start me"]);
        assert!(app.transcript.entries[0].completed);
        loaded(&mut app, &["s-1", "s-9"]);
        assert_eq!(app.sessions.selected, spawned);
        assert_eq!(app.visible_session, Some(spawned));
        assert_eq!(user_entries(&app), vec!["start me"]);
    }

    #[test]
    fn whitespace_only_draft_sends_nothing() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        type_composer_text(&mut app, "   ");
        send_pressed(&mut app);
        assert!(fake.turns.lock().unwrap().is_empty());
        assert!(fake.spawns.lock().unwrap().is_empty());
        assert!(app.transcript.entries.is_empty());
        assert_eq!(app.composer.content.text(), "   ");
    }

    #[test]
    fn fixture_mode_send_is_noop_keeping_draft() {
        let mut app = App::boot();
        let entries = app.transcript.entries.len();
        type_composer_text(&mut app, "hi");
        send_pressed(&mut app);
        assert_eq!(app.transcript.entries.len(), entries);
        assert_eq!(app.composer.content.text(), "hi");
        assert!(!app.composer.send_enabled);
    }

    #[test]
    fn send_gate_false_in_fixture_true_when_live_connected_idle() {
        let fixture = App::boot();
        assert!(!fixture.composer.send_enabled);
        let (mut app, _) = live_app();
        assert!(!app.composer.send_enabled);
        deliver_connected(&mut app);
        // No listed session yet, but the spawn path is available.
        assert!(app.composer.send_enabled);
        loaded(&mut app, &["s-1"]);
        assert!(app.composer.send_enabled);
    }

    /// Loads sessions sharing one workspace, for draft tests: the
    /// derived project carries that workspace.
    fn loaded_with_workspace(app: &mut App) {
        loaded(app, &["s-1", "s-2"]);
    }

    /// Presses the first listed project's Plus icon button through the root
    /// reducer.
    fn new_session_pressed(app: &mut App) {
        let project = app
            .sessions
            .groups
            .first()
            .expect("a listed project")
            .project
            .id
            .clone();
        let _ = app.update(Message::Sessions(sessions::Message::NewSession(project)));
    }

    #[test]
    fn new_session_enters_draft_mode_with_parked_transcript() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        stream(&mut app, agent_completed("s-1", "a-1", "one"));
        assert_eq!(app.transcript.entries.len(), 1);
        new_session_pressed(&mut app);
        assert_eq!(
            app.new_session,
            Some(NewSessionDraft {
                workspace: String::from("/live/workspace"),
                title: String::from("workspace · New session"),
                worktree_mode: DraftWorktreeMode::Off,
                worktree_base: String::new(),
                worktree_path: String::new(),
                model: manifest_domain::ModelChoice::Default,
                effort: manifest_domain::EffortChoice::Default,
                permission: manifest_domain::PermissionMode::default(),
            })
        );
        assert!(
            app.sessions
                .expanded
                .contains(&(manifest_domain::ProjectId::for_workspace("/live/workspace")))
        );
        // The s-1 transcript parks; the draft shows empty.
        assert!(app.transcript.entries.is_empty());
        assert_eq!(app.visible_session, None);
        assert_eq!(stored_len(&app, "s-1"), 1);
    }

    #[test]
    fn draft_send_spawns_into_project_workspace_and_clears_draft() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        new_session_pressed(&mut app);
        assert!(app.new_session.is_some());
        type_composer_text(&mut app, "fresh idea");
        send_pressed(&mut app);
        assert_eq!(
            *fake.spawns.lock().unwrap(),
            vec![(
                String::from("/live/workspace"),
                manifest_domain::TurnInput::Text(String::from("fresh idea")),
                manifest_domain::SpawnChoices::default()
            )]
        );
        assert!(fake.turns.lock().unwrap().is_empty());
        assert!(app.new_session.is_none());
        assert!(app.awaiting_spawn);
        assert_eq!(user_entries(&app), vec!["fresh idea"]);
        assert_eq!(app.composer.content.text(), "");
    }

    #[test]
    fn select_exits_draft_mode_and_restores_parked_transcript() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        stream(&mut app, agent_completed("s-1", "a-1", "one"));
        new_session_pressed(&mut app);
        assert!(app.new_session.is_some());
        let first = SessionId::new("s-1");
        let _ = app.update(Message::Sessions(sessions::Message::Select(first.clone())));
        assert_eq!(app.new_session, None);
        assert_eq!(app.visible_session, Some(first));
        assert_eq!(app.transcript.entries.len(), 1);
        assert!(matches!(
            &app.transcript.entries[0].kind,
            crate::transcript::EntryKind::Assistant { text, .. } if text == "one"
        ));
    }

    #[test]
    fn sessions_loaded_during_draft_keeps_the_draft_view() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        new_session_pressed(&mut app);
        loaded_with_workspace(&mut app);
        assert!(app.new_session.is_some());
        assert_eq!(app.visible_session, None);
        assert!(app.transcript.entries.is_empty());
    }

    #[test]
    fn send_gate_stays_enabled_in_draft_while_selection_runs() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        stream(
            &mut app,
            manifest_domain::StreamEvent::TurnStarted {
                session: SessionId::new("s-1"),
                turn: None,
                view_cursor: None,
            },
        );
        // Steer mode: the gate stays live while the selection runs.
        assert!(app.composer.send_enabled);
        assert!(app.composer.running_visible);
        new_session_pressed(&mut app);
        assert!(app.new_session.is_some());
        assert!(app.composer.send_enabled);
        assert!(!app.composer.running_visible);
    }

    #[test]
    fn new_session_without_workspace_expands_without_draft() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        // A listed project with no workspace: built through replace
        // directly, since derived projects always carry one.
        let mut session = live_session("s-1");
        session.workspace = String::new();
        session.project = Some(manifest_domain::ProjectId::new("project-live"));
        app.sessions.replace(vec![live_project()], vec![session]);
        new_session_pressed(&mut app);
        assert!(
            app.sessions
                .expanded
                .contains(&(manifest_domain::ProjectId::new("project-live")))
        );
        assert_eq!(app.new_session, None);
    }

    #[test]
    fn new_session_while_awaiting_spawn_expands_without_draft() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        new_session_pressed(&mut app);
        type_composer_text(&mut app, "fresh idea");
        send_pressed(&mut app);
        assert!(app.awaiting_spawn);
        new_session_pressed(&mut app);
        assert_eq!(app.new_session, None);
        assert!(
            app.sessions
                .expanded
                .contains(&(manifest_domain::ProjectId::for_workspace("/live/workspace")))
        );
    }

    #[test]
    fn fixture_new_session_expands_without_draft() {
        let mut app = App::boot();
        let _ = app.update(Message::Sessions(sessions::Message::NewSession(
            manifest_domain::ProjectId::new("project-checkout-api"),
        )));
        assert_eq!(app.new_session, None);
        assert!(
            app.sessions
                .expanded
                .contains(&(manifest_domain::ProjectId::new("project-checkout-api")))
        );
    }

    /// Unique temp dir per test: temp dir plus process id, millis, and an
    /// atomic counter, so parallel tests never share a registry file.
    /// Std only. Best-effort cleanup after the test.
    static STORE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_registry_path(suffix: &str) -> std::path::PathBuf {
        let unique = STORE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "manifest-shell-test-{}-{millis}-{unique}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create unique temp dir");
        dir.join("projects.json")
    }

    fn cleanup_registry(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    /// Production boot with a registry store, handle already delivered.
    fn live_app_with_store(path: std::path::PathBuf) -> (App, FakeEngine) {
        let fake = fake_engine();
        let mut app = App::live(
            manifest_mcp::ServerConfig::new("fake"),
            Some(manifest_store::ProjectStore::new(path)),
        );
        let attempt = app.connection_attempt;
        let _ = app.update(Message::Engine {
            attempt,
            input: engine::Input::Ready(engine::EngineHandle::new(fake.clone())),
        });
        (app, fake)
    }

    fn workspace_session(id: &str, workspace: &str) -> manifest_domain::SessionSummary {
        manifest_domain::SessionSummary {
            id: SessionId::new(id),
            title: String::from(id),
            status: manifest_domain::SessionStatus::Idle,
            project: None,
            workspace: String::from(workspace),
            relative_time: String::from("now"),
            stream_dropped_deltas: 0,
        }
    }

    fn workspace_project(name: &str, workspace: &str) -> manifest_domain::ProjectSummary {
        manifest_domain::ProjectSummary {
            id: manifest_domain::ProjectId::for_workspace(workspace),
            name: String::from(name),
            workspace: Some(String::from(workspace)),
        }
    }

    fn load_sessions(app: &mut App, sessions: Vec<manifest_domain::SessionSummary>) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions,
                usage: Vec::new(),
            }),
        );
    }

    fn transcript_errors(app: &App) -> Vec<&str> {
        app.transcript
            .entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                crate::transcript::EntryKind::Error { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn projects_loaded_present_groups_sessions_under_custom_name() {
        let path = temp_registry_path("present");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(
            &mut app,
            vec![
                workspace_session("s-1", "/work/alpha"),
                workspace_session("s-9", "/work/elsewhere"),
            ],
        );
        let renamed = workspace_project("Custom", "/work/alpha");
        let _ = app.update(Message::ProjectsLoaded(Ok(
            manifest_store::Loaded::Present(vec![renamed.clone()]),
        )));
        assert!(app.projects_ready);
        assert!(!app.projects_missing);
        assert_eq!(app.projects, vec![renamed.clone()]);
        assert_eq!(app.sessions.groups.len(), 1);
        assert_eq!(app.sessions.groups[0].project.name, "Custom");
        assert_eq!(app.sessions.groups[0].sessions.len(), 1);
        assert_eq!(app.sessions.groups[0].sessions[0].id, SessionId::new("s-1"));
        assert_eq!(app.sessions.unprojected.len(), 1);
        assert_eq!(app.sessions.unprojected[0].id, SessionId::new("s-9"));
        cleanup_registry(&path);
    }

    #[test]
    fn projects_loaded_missing_seeds_from_loaded_sessions() {
        let path = temp_registry_path("missing-seeds");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(
            &mut app,
            vec![
                workspace_session("s-1", "/work/alpha"),
                workspace_session("s-2", "/work/alpha"),
                workspace_session("s-3", "/work/beta"),
            ],
        );
        let seeded = manifest_domain::seed_projects(&[
            workspace_session("s-1", "/work/alpha"),
            workspace_session("s-2", "/work/alpha"),
            workspace_session("s-3", "/work/beta"),
        ]);
        let _ = app.update(Message::ProjectsLoaded(Ok(manifest_store::Loaded::Missing)));
        assert_eq!(app.projects, seeded);
        assert!(app.projects_ready);
        assert!(!app.projects_missing);
        assert_eq!(app.sessions.groups.len(), 2);
        // The save task is not executed by `update`: persistence itself
        // is covered by `manifest_store` tests. State is the assertion.
        cleanup_registry(&path);
    }

    #[test]
    fn sessions_loaded_after_missing_seeds_and_saves() {
        let path = temp_registry_path("missing-then-load");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        let _ = app.update(Message::ProjectsLoaded(Ok(manifest_store::Loaded::Missing)));
        assert!(app.projects_ready);
        assert!(app.projects_missing);
        assert!(app.projects.is_empty());
        load_sessions(
            &mut app,
            vec![
                workspace_session("s-1", "/work/alpha"),
                workspace_session("s-2", "/work/beta"),
            ],
        );
        assert!(!app.projects_missing);
        assert!(app.projects_ready);
        assert_eq!(app.projects.len(), 2);
        assert_eq!(app.sessions.groups.len(), 2);
        cleanup_registry(&path);
    }

    #[test]
    fn projects_loaded_error_keeps_derived_grouping_and_shows_error() {
        let path = temp_registry_path("load-error");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(
            &mut app,
            vec![
                workspace_session("s-1", "/work/alpha"),
                workspace_session("s-2", "/work/beta"),
            ],
        );
        let groups_before = app.sessions.groups.len();
        let before = app.transcript.entries.len();
        let _ = app.update(Message::ProjectsLoaded(Err(String::from("disk broke"))));
        assert!(!app.projects_ready);
        assert_eq!(app.sessions.groups.len(), groups_before);
        assert_eq!(app.sessions.groups.len(), 2);
        assert_eq!(app.transcript.entries.len(), before + 1);
        assert!(transcript_errors(&app).contains(&"disk broke"));
        cleanup_registry(&path);
    }

    #[test]
    fn projects_saved_error_reports_verbatim_without_rollback() {
        let path = temp_registry_path("save-error");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(&mut app, vec![workspace_session("s-1", "/work/alpha")]);
        let _ = app.update(Message::ProjectsLoaded(Ok(manifest_store::Loaded::Missing)));
        let projects_before = app.projects.clone();
        assert!(!projects_before.is_empty());
        let before = app.transcript.entries.len();
        let _ = app.update(Message::ProjectsSaved(Err(String::from(
            "Couldn't save the project list: disk broke",
        ))));
        assert_eq!(app.projects, projects_before);
        assert_eq!(app.transcript.entries.len(), before + 1);
        assert!(transcript_errors(&app).contains(&"Couldn't save the project list: disk broke"));
        cleanup_registry(&path);
    }

    /// Opens the popup and fills the form through the real message path.
    fn open_and_fill(app: &mut App, directory: &str, name: &str) {
        let _ = app.update(Message::Sessions(sessions::Message::OpenAddProject));
        let _ = app.update(Message::Sessions(sessions::Message::DirectoryChanged(
            String::from(directory),
        )));
        let _ = app.update(Message::Sessions(sessions::Message::NameChanged(
            String::from(name),
        )));
    }

    #[test]
    fn add_project_happy_path_closes_groups_and_expands() {
        let workspace = temp_registry_path("happy-ws");
        let dir = workspace
            .parent()
            .expect("temp dir has a parent")
            .display()
            .to_string();
        let path = temp_registry_path("happy");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(&mut app, vec![workspace_session("s-1", &dir)]);
        let _ = app.update(Message::ProjectsLoaded(Ok(
            manifest_store::Loaded::Present(Vec::new()),
        )));
        assert_eq!(app.sessions.groups.len(), 0);
        open_and_fill(&mut app, &dir, "Custom");
        let _ = app.update(Message::AddProjectChecked(Ok((
            dir.clone(),
            String::from("Custom"),
        ))));
        assert_eq!(app.sessions.add_project, sessions::AddProject::Closed);
        assert_eq!(app.projects.len(), 1);
        assert_eq!(app.projects[0].workspace, Some(dir.clone()));
        assert_eq!(app.projects[0].name, "Custom");
        assert_eq!(app.sessions.groups.len(), 1);
        assert_eq!(app.sessions.groups[0].project.name, "Custom");
        assert_eq!(app.sessions.groups[0].sessions.len(), 1);
        assert_eq!(app.sessions.groups[0].sessions[0].id, SessionId::new("s-1"));
        assert!(
            app.sessions
                .expanded
                .contains(&(manifest_domain::ProjectId::for_workspace(&dir)))
        );
        cleanup_registry(&path);
        cleanup_registry(&workspace);
    }

    #[test]
    fn add_project_duplicate_expands_without_duplicating() {
        let dir = String::from("/work/alpha");
        let path = temp_registry_path("duplicate");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(&mut app, vec![workspace_session("s-1", &dir)]);
        let existing = workspace_project("Old", &dir);
        let _ = app.update(Message::ProjectsLoaded(Ok(
            manifest_store::Loaded::Present(vec![existing.clone()]),
        )));
        open_and_fill(&mut app, &dir, "New name");
        let _ = app.update(Message::AddProjectChecked(Ok((
            dir.clone(),
            String::from("New name"),
        ))));
        assert_eq!(app.sessions.add_project, sessions::AddProject::Closed);
        assert_eq!(app.projects, vec![existing.clone()]);
        assert_eq!(app.sessions.groups.len(), 1);
        assert_eq!(app.sessions.groups[0].project.name, "Old");
        assert!(app.sessions.expanded.contains(&(existing.id)));
        cleanup_registry(&path);
    }

    #[test]
    fn add_project_validation_error_keeps_popup_with_error() {
        let path = temp_registry_path("validation-error");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(&mut app, vec![workspace_session("s-1", "/work/alpha")]);
        let _ = app.update(Message::ProjectsLoaded(Ok(
            manifest_store::Loaded::Present(Vec::new()),
        )));
        open_and_fill(&mut app, "/work/alpha", "Alpha");
        let _ = app.update(Message::AddProjectChecked(Err(String::from(
            "That path isn't a folder. Choose a folder.",
        ))));
        assert_eq!(
            app.sessions.add_project,
            sessions::AddProject::Open {
                directory: String::from("/work/alpha"),
                name: String::from("Alpha"),
                name_edited: true,
                error: Some(String::from("That path isn't a folder. Choose a folder.")),
            }
        );
        // Typing clears the error through the child contract.
        let _ = app.update(Message::Sessions(sessions::Message::DirectoryChanged(
            String::from("/work/other"),
        )));
        assert!(matches!(
            &app.sessions.add_project,
            sessions::AddProject::Open { error: None, .. }
        ));
        cleanup_registry(&path);
    }

    #[test]
    fn submit_in_fixture_mode_closes_popup() {
        let mut app = App::boot();
        let _ = app.update(Message::Sessions(sessions::Message::OpenAddProject));
        let _ = app.update(Message::Sessions(sessions::Message::DirectoryChanged(
            String::from("/x/y/billing-service"),
        )));
        let _ = app.update(Message::Sessions(sessions::Message::SubmitAddProject));
        assert_eq!(app.sessions.add_project, sessions::AddProject::Closed);
        assert!(app.projects.is_empty());
    }

    #[test]
    fn submit_blank_form_in_live_mode_reports_error() {
        let path = temp_registry_path("blank-form");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(&mut app, vec![workspace_session("s-1", "/work/alpha")]);
        let _ = app.update(Message::ProjectsLoaded(Ok(
            manifest_store::Loaded::Present(Vec::new()),
        )));
        let _ = app.update(Message::Sessions(sessions::Message::OpenAddProject));
        let _ = app.update(Message::Sessions(sessions::Message::SubmitAddProject));
        assert!(matches!(
            &app.sessions.add_project,
            sessions::AddProject::Open { error: Some(_), .. }
        ));
        assert!(app.projects.is_empty());
        cleanup_registry(&path);
    }

    #[test]
    fn remove_project_regroups_sessions_and_keeps_them_listed() {
        let workspace_dir = std::env::temp_dir().join(format!(
            "manifest-shell-ws-{}-{}",
            std::process::id(),
            STORE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
        let marker = workspace_dir.join("marker.txt");
        std::fs::write(&marker, "keep me").expect("write marker");
        let dir = workspace_dir.display().to_string();
        let path = temp_registry_path("remove");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(
            &mut app,
            vec![
                workspace_session("s-1", &dir),
                workspace_session("s-2", "/work/other"),
            ],
        );
        let kept = workspace_project("Kept", "/work/other");
        let removed = workspace_project("Gone", &dir);
        let _ = app.update(Message::ProjectsLoaded(Ok(
            manifest_store::Loaded::Present(vec![kept.clone(), removed.clone()]),
        )));
        assert_eq!(app.sessions.groups.len(), 2);
        let _ = app.update(Message::Sessions(sessions::Message::RemoveProject(
            removed.id.clone(),
        )));
        assert_eq!(app.sessions.groups.len(), 1);
        assert_eq!(app.sessions.groups[0].project, kept);
        assert_eq!(app.projects, vec![kept]);
        // Both sessions stay listed; the removed project's rows land in
        // unprojected with recomputed ownership.
        let listed: Vec<SessionId> = app
            .sessions
            .groups
            .iter()
            .flat_map(|group| group.sessions.iter())
            .chain(app.sessions.unprojected.iter())
            .map(|summary| summary.id.clone())
            .collect();
        assert!(listed.contains(&SessionId::new("s-1")));
        assert!(listed.contains(&SessionId::new("s-2")));
        assert!(
            app.sessions
                .unprojected
                .iter()
                .any(|summary| summary.id == SessionId::new("s-1"))
        );
        // Removal never touches the project folder on disk.
        assert!(workspace_dir.is_dir());
        assert!(marker.is_file());
        let _ = std::fs::remove_dir_all(&workspace_dir);
        cleanup_registry(&path);
    }

    #[test]
    fn remove_project_cancels_draft_in_that_project() {
        let dir = String::from("/work/alpha");
        let path = temp_registry_path("remove-draft");
        let (mut app, _) = live_app_with_store(path.clone());
        deliver_connected(&mut app);
        load_sessions(&mut app, vec![workspace_session("s-1", &dir)]);
        let project = workspace_project("Alpha", &dir);
        let _ = app.update(Message::ProjectsLoaded(Ok(
            manifest_store::Loaded::Present(vec![project.clone()]),
        )));
        let _ = app.update(Message::Sessions(sessions::Message::NewSession(
            project.id.clone(),
        )));
        assert!(app.new_session.is_some());
        let _ = app.update(Message::Sessions(sessions::Message::RemoveProject(
            project.id.clone(),
        )));
        assert_eq!(app.new_session, None);
        assert!(app.projects.is_empty());
        cleanup_registry(&path);
    }

    #[test]
    fn remove_in_fixture_mode_removes_group_only() {
        let mut app = App::boot();
        let before: usize = app
            .sessions
            .groups
            .iter()
            .map(|group| group.sessions.len())
            .sum::<usize>()
            + app.sessions.unprojected.len();
        let _ = app.update(Message::Sessions(sessions::Message::RemoveProject(
            manifest_domain::ProjectId::new("project-checkout-api"),
        )));
        assert_eq!(app.sessions.groups.len(), 1);
        let after: usize = app
            .sessions
            .groups
            .iter()
            .map(|group| group.sessions.len())
            .sum::<usize>()
            + app.sessions.unprojected.len();
        assert_eq!(before, after);
        assert!(app.project_store.is_none());
    }

    #[test]
    fn directory_picked_some_types_into_popup() {
        let mut app = App::boot();
        let _ = app.update(Message::Sessions(sessions::Message::OpenAddProject));
        let _ = app.update(Message::DirectoryPicked(Some(String::from("/x/y/picked"))));
        assert_eq!(
            app.sessions.add_project,
            sessions::AddProject::Open {
                directory: String::from("/x/y/picked"),
                name: String::from("picked"),
                name_edited: false,
                error: None,
            }
        );
    }

    #[test]
    fn directory_picked_none_is_noop() {
        let mut app = App::boot();
        let _ = app.update(Message::Sessions(sessions::Message::OpenAddProject));
        let _ = app.update(Message::DirectoryPicked(None));
        assert_eq!(
            app.sessions.add_project,
            sessions::AddProject::Open {
                directory: String::new(),
                name: String::new(),
                name_edited: false,
                error: None,
            }
        );
    }

    #[test]
    fn validate_project_dir_accepts_a_real_directory() {
        let dir = std::env::temp_dir().join(format!(
            "manifest-shell-validate-{}-{}",
            std::process::id(),
            STORE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("create dir");
        let got = validate_project_dir(dir.display().to_string(), String::from("  Pretty  "))
            .expect("real dir validates");
        let canonical = std::fs::canonicalize(&dir)
            .expect("canonicalize")
            .display()
            .to_string();
        assert_eq!(got, (canonical, String::from("Pretty")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_project_dir_rejects_file_and_missing_paths() {
        let dir = std::env::temp_dir().join(format!(
            "manifest-shell-validate-file-{}-{}",
            std::process::id(),
            STORE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("create dir");
        let file = dir.join("note.txt");
        std::fs::write(&file, "x").expect("write file");
        assert_eq!(
            validate_project_dir(file.display().to_string(), String::from("n")),
            Err(String::from("That path isn't a folder. Choose a folder."))
        );
        assert_eq!(
            validate_project_dir(
                dir.join("no-such-dir").display().to_string(),
                String::from("n")
            ),
            Err(String::from(
                "That folder couldn't be found. Check the path and try again."
            ))
        );
        assert_eq!(
            validate_project_dir(dir.display().to_string(), String::from("   ")),
            Err(String::from("Give the project a name."))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session with an explicit status, for resume tests.
    fn status_session(
        id: &str,
        status: manifest_domain::SessionStatus,
    ) -> manifest_domain::SessionSummary {
        let mut summary = live_session(id);
        summary.status = status;
        summary
    }

    /// Loads one orphaned and one idle session.
    fn loaded_orphan_and_idle(app: &mut App) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![
                    status_session("sess-orphan", manifest_domain::SessionStatus::Orphaned),
                    live_session("s-idle"),
                ],
                usage: Vec::new(),
            }),
        );
    }

    fn select(app: &mut App, id: &str) {
        let _ = app.update(Message::Sessions(sessions::Message::Select(
            SessionId::new(id),
        )));
    }

    #[test]
    fn orphaned_select_resumes_once() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_orphan_and_idle(&mut app);
        select(&mut app, "sess-orphan");
        assert_eq!(
            *fake.resumes.lock().unwrap(),
            vec![SessionId::new("sess-orphan")]
        );
        // Still in flight: a second select issues no duplicate resume.
        select(&mut app, "sess-orphan");
        assert_eq!(
            *fake.resumes.lock().unwrap(),
            vec![SessionId::new("sess-orphan")]
        );
        // An idle selection resumes nothing further.
        select(&mut app, "s-idle");
        assert_eq!(
            *fake.resumes.lock().unwrap(),
            vec![SessionId::new("sess-orphan")]
        );
    }

    #[test]
    fn orphaned_select_while_connecting_does_not_resume() {
        let (mut app, fake) = live_app();
        loaded_orphan_and_idle(&mut app);
        select(&mut app, "sess-orphan");
        assert!(fake.resumes.lock().unwrap().is_empty());
    }

    #[test]
    fn resumed_then_loaded_idle_releases_gate_and_send_enabled() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_orphan_and_idle(&mut app);
        select(&mut app, "sess-orphan");
        assert_eq!(fake.resumes.lock().unwrap().len(), 1);
        // The orphaned selection with a resume in flight is busy.
        assert!(!app.composer.send_enabled);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionResumed {
                session: SessionId::new("sess-orphan"),
            }),
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![
                    status_session("sess-orphan", manifest_domain::SessionStatus::Idle),
                    live_session("s-idle"),
                ],
                usage: Vec::new(),
            }),
        );
        assert!(app.composer.send_enabled);
        select(&mut app, "sess-orphan");
        assert_eq!(fake.resumes.lock().unwrap().len(), 1);
    }

    #[test]
    fn resume_failure_releases_gate_for_retry() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_orphan_and_idle(&mut app);
        select(&mut app, "sess-orphan");
        assert_eq!(fake.resumes.lock().unwrap().len(), 1);
        let message = String::from(
            "session sess-orphan is in use by another client (sessionInUse: already attached). Muse MCP will not attach to it.",
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::Resume(SessionId::new("sess-orphan")),
                message: message.clone(),
            }),
        );
        assert!(transcript_errors(&app).contains(&message.as_str()));
        // The gate released: reselecting retries the resume.
        select(&mut app, "sess-orphan");
        assert_eq!(fake.resumes.lock().unwrap().len(), 2);
    }

    /// Two-turn stored history, oldest first.
    fn history_items() -> Vec<manifest_domain::StreamItem> {
        vec![
            manifest_domain::StreamItem::UserMessage {
                item_id: String::from("hist-user-1"),
                text: String::from("First question"),
            },
            manifest_domain::StreamItem::AgentMessage {
                item_id: String::from("hist-agent-1"),
                text: String::from("First answer."),
                phase: None,
            },
        ]
    }

    /// User and assistant texts of the visible transcript, in order.
    fn visible_texts(app: &App) -> Vec<String> {
        app.transcript
            .entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                crate::transcript::EntryKind::User { text } => Some(format!("user:{text}")),
                crate::transcript::EntryKind::Assistant { text, .. } => {
                    Some(format!("agent:{text}"))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn resumed_requests_history_once() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_orphan_and_idle(&mut app);
        select(&mut app, "sess-orphan");
        assert_eq!(
            *fake.histories.lock().unwrap(),
            vec![SessionId::new("sess-orphan")]
        );
        // The resume completing requests nothing further.
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionResumed {
                session: SessionId::new("sess-orphan"),
            }),
        );
        assert_eq!(fake.histories.lock().unwrap().len(), 1);
        // Reselecting the backfilled session requests nothing further.
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::HistoryLoaded {
                session: SessionId::new("sess-orphan"),
                items: history_items(),
            }),
        );
        select(&mut app, "sess-orphan");
        assert_eq!(fake.histories.lock().unwrap().len(), 1);
    }

    #[test]
    fn reopened_session_restores_history_and_send_gate() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_orphan_and_idle(&mut app);
        select(&mut app, "sess-orphan");
        assert_eq!(fake.histories.lock().unwrap().len(), 1);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionResumed {
                session: SessionId::new("sess-orphan"),
            }),
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![
                    status_session("sess-orphan", manifest_domain::SessionStatus::Idle),
                    live_session("s-idle"),
                ],
                usage: Vec::new(),
            }),
        );
        assert!(app.composer.send_enabled);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::HistoryLoaded {
                session: SessionId::new("sess-orphan"),
                items: history_items(),
            }),
        );
        assert_eq!(
            visible_texts(&app),
            vec![
                String::from("user:First question"),
                String::from("agent:First answer."),
            ]
        );
        assert!(app.composer.send_enabled);
        // A duplicate load seeds nothing twice.
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::HistoryLoaded {
                session: SessionId::new("sess-orphan"),
                items: history_items(),
            }),
        );
        assert_eq!(visible_texts(&app).len(), 2);
    }

    #[test]
    fn history_loaded_for_background_renders_after_switch() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_orphan_and_idle(&mut app);
        select(&mut app, "sess-orphan");
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::HistoryLoaded {
                session: SessionId::new("s-idle"),
                items: history_items(),
            }),
        );
        // The background seed leaves the visible transcript alone.
        assert!(visible_texts(&app).is_empty());
        select(&mut app, "s-idle");
        assert_eq!(
            visible_texts(&app),
            vec![
                String::from("user:First question"),
                String::from("agent:First answer."),
            ]
        );
        // The switch found stored content, so it fetched nothing more.
        assert_eq!(
            *fake.histories.lock().unwrap(),
            vec![SessionId::new("sess-orphan")]
        );
    }

    #[test]
    fn history_failure_keeps_session_sendable_and_retries_on_reselect() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_orphan_and_idle(&mut app);
        select(&mut app, "sess-orphan");
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionResumed {
                session: SessionId::new("sess-orphan"),
            }),
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![
                    status_session("sess-orphan", manifest_domain::SessionStatus::Idle),
                    live_session("s-idle"),
                ],
                usage: Vec::new(),
            }),
        );
        assert!(app.composer.send_enabled);
        let message = String::from("history blew up");
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::History(SessionId::new("sess-orphan")),
                message: message.clone(),
            }),
        );
        assert!(transcript_errors(&app).contains(&message.as_str()));
        // The failure shows but the session stays sendable.
        assert!(app.composer.send_enabled);
        // The history gate released: reselecting retries the fetch.
        select(&mut app, "sess-orphan");
        assert_eq!(fake.histories.lock().unwrap().len(), 2);
    }

    #[test]
    fn spawned_session_never_fetches_history() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionSpawned {
                session: SessionId::new("s-new"),
                turn: None,
            }),
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![live_session("s-new")],
                usage: Vec::new(),
            }),
        );
        select(&mut app, "s-new");
        assert!(fake.histories.lock().unwrap().is_empty());
    }

    #[test]
    fn sessions_loaded_backfills_visible_session_without_select() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        // No Select and no resume: replace falls back to the
        // first listed row, so the dormant orphan is visible by
        // default (the app-reopen shape) and the load backfills
        // its history once.
        loaded_orphan_and_idle(&mut app);
        assert_eq!(app.visible_session, Some(SessionId::new("sess-orphan")));
        assert_eq!(
            *fake.histories.lock().unwrap(),
            vec![SessionId::new("sess-orphan")]
        );
        // A second list refresh requests nothing further.
        loaded_orphan_and_idle(&mut app);
        assert_eq!(fake.histories.lock().unwrap().len(), 1);
    }

    /// Enters a draft, sets its worktree mode and field, and types a
    /// prompt through the root reducer.
    fn draft_with_worktree(app: &mut App, mode: DraftWorktreeMode, field: &str, prompt: &str) {
        loaded_with_workspace(app);
        new_session_pressed(app);
        let _ = app.update(Message::DraftWorktreeModeSelected(mode));
        match mode {
            DraftWorktreeMode::Create => {
                let _ = app.update(Message::DraftWorktreeBaseChanged(String::from(field)));
            }
            DraftWorktreeMode::Existing => {
                let _ = app.update(Message::DraftWorktreePathChanged(String::from(field)));
            }
            DraftWorktreeMode::Off => {}
        }
        type_composer_text(app, prompt);
    }

    #[test]
    fn draft_existing_path_reaches_spawn() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        draft_with_worktree(
            &mut app,
            DraftWorktreeMode::Existing,
            "/tmp/wt-x",
            "build it",
        );
        send_pressed(&mut app);
        assert_eq!(
            *fake.spawns.lock().unwrap(),
            vec![(
                String::from("/live/workspace"),
                manifest_domain::TurnInput::Text(String::from("build it")),
                manifest_domain::SpawnChoices {
                    worktree: manifest_domain::WorktreeChoice::Existing {
                        path: String::from("/tmp/wt-x"),
                    },
                    ..Default::default()
                }
            )]
        );
    }

    #[test]
    fn draft_create_base_reaches_spawn() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        draft_with_worktree(&mut app, DraftWorktreeMode::Create, "main", "build it");
        send_pressed(&mut app);
        assert_eq!(
            *fake.spawns.lock().unwrap(),
            vec![(
                String::from("/live/workspace"),
                manifest_domain::TurnInput::Text(String::from("build it")),
                manifest_domain::SpawnChoices {
                    worktree: manifest_domain::WorktreeChoice::Create {
                        base: Some(String::from("main")),
                    },
                    ..Default::default()
                }
            )]
        );
    }

    #[test]
    fn draft_create_blank_base_spawns_without_base() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        draft_with_worktree(&mut app, DraftWorktreeMode::Create, "   ", "build it");
        send_pressed(&mut app);
        assert_eq!(
            *fake.spawns.lock().unwrap(),
            vec![(
                String::from("/live/workspace"),
                manifest_domain::TurnInput::Text(String::from("build it")),
                manifest_domain::SpawnChoices {
                    worktree: manifest_domain::WorktreeChoice::Create { base: None },
                    ..Default::default()
                }
            )]
        );
    }

    #[test]
    fn existing_worktree_without_path_blocks_send() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        draft_with_worktree(&mut app, DraftWorktreeMode::Existing, "", "build it");
        send_pressed(&mut app);
        assert!(fake.spawns.lock().unwrap().is_empty());
        let draft = app.new_session.as_ref().expect("draft stays");
        assert_eq!(draft.worktree_mode, DraftWorktreeMode::Existing);
        assert_eq!(draft.worktree_path, "");
        assert_eq!(app.composer.content.text(), "build it");
        assert!(
            transcript_errors(&app).contains(
                &"Choose an existing worktree folder, or switch the worktree choice off."
            )
        );
    }

    #[test]
    fn spawn_failure_restores_draft_choices_and_prompt() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        draft_with_worktree(
            &mut app,
            DraftWorktreeMode::Existing,
            "/tmp/wt-x",
            "build it",
        );
        send_pressed(&mut app);
        assert_eq!(fake.spawns.lock().unwrap().len(), 1);
        assert!(app.new_session.is_none());
        let message = String::from(
            "Repository '/untrusted' is not trusted; worktree create, attach, and list require a trusted repository.",
        );
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::Spawn,
                message: message.clone(),
            }),
        );
        assert!(!app.awaiting_spawn);
        let draft = app.new_session.as_ref().expect("draft restored");
        assert_eq!(draft.worktree_mode, DraftWorktreeMode::Existing);
        assert_eq!(draft.worktree_path, "/tmp/wt-x");
        assert_eq!(app.composer.content.text(), "build it");
        assert!(transcript_errors(&app).contains(&message.as_str()));
        // Retry after switching the choice off: the restored prompt
        // spawns again with no worktree.
        let _ = app.update(Message::DraftWorktreeModeSelected(DraftWorktreeMode::Off));
        send_pressed(&mut app);
        assert_eq!(
            *fake.spawns.lock().unwrap(),
            vec![
                (
                    String::from("/live/workspace"),
                    manifest_domain::TurnInput::Text(String::from("build it")),
                    manifest_domain::SpawnChoices {
                        worktree: manifest_domain::WorktreeChoice::Existing {
                            path: String::from("/tmp/wt-x"),
                        },
                        ..Default::default()
                    }
                ),
                (
                    String::from("/live/workspace"),
                    manifest_domain::TurnInput::Text(String::from("build it")),
                    manifest_domain::SpawnChoices::default()
                ),
            ]
        );
    }

    #[test]
    fn cwd_spawn_carries_composer_picks() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        assert!(app.sessions.selected_summary().is_none());
        let _ = app.update(Message::Composer(composer::Message::ModelSelected(
            manifest_domain::ModelChoice::Named(String::from("muse-spark")),
        )));
        let _ = app.update(Message::Composer(composer::Message::EffortSelected(
            manifest_domain::EffortChoice::High,
        )));
        let _ = app.update(Message::Composer(composer::Message::PermissionSelected(
            manifest_domain::MusePermission::Judge,
        )));
        type_composer_text(&mut app, "start me");
        send_pressed(&mut app);
        let workspace = std::env::current_dir().unwrap().display().to_string();
        assert_eq!(
            *fake.spawns.lock().unwrap(),
            vec![(
                workspace,
                manifest_domain::TurnInput::Text(String::from("start me")),
                manifest_domain::SpawnChoices {
                    worktree: manifest_domain::WorktreeChoice::Off,
                    model: manifest_domain::ModelChoice::Named(String::from("muse-spark")),
                    effort: manifest_domain::EffortChoice::High,
                    permission: manifest_domain::PermissionMode::Muse(
                        manifest_domain::MusePermission::Judge
                    ),
                }
            )]
        );
    }

    #[test]
    fn cwd_spawn_failure_restores_prompt_only() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        assert!(app.sessions.selected_summary().is_none());
        type_composer_text(&mut app, "start me");
        send_pressed(&mut app);
        assert_eq!(fake.spawns.lock().unwrap().len(), 1);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::Spawn,
                message: String::from("spawn broke"),
            }),
        );
        assert!(!app.awaiting_spawn);
        assert_eq!(app.new_session, None);
        assert_eq!(app.composer.content.text(), "start me");
        assert!(transcript_errors(&app).contains(&"spawn broke"));
    }

    #[test]
    fn worktree_messages_without_draft_are_noops() {
        let mut app = App::boot();
        type_composer_text(&mut app, "hi");
        let _ = app.update(Message::DraftWorktreeModeSelected(
            DraftWorktreeMode::Existing,
        ));
        let _ = app.update(Message::DraftWorktreeBaseChanged(String::from("main")));
        let _ = app.update(Message::DraftWorktreePathChanged(String::from("/tmp/wt-x")));
        assert_eq!(app.new_session, None);
        assert_eq!(app.composer.content.text(), "hi");
    }

    /// Starts a turn on the selected session with a named turn id.
    fn start_turn(app: &mut App, session: &str, turn: &str) {
        stream(
            app,
            manifest_domain::StreamEvent::TurnStarted {
                session: SessionId::new(session),
                turn: Some(manifest_domain::TurnId::new(turn)),
                view_cursor: None,
            },
        );
    }

    #[test]
    fn steer_sends_prompt_to_running_turn() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        start_turn(&mut app, "s-1", "turn-7");
        assert!(app.composer.running_visible);
        type_composer_text(&mut app, "go faster");
        send_pressed(&mut app);
        assert_eq!(
            *fake.steers.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                String::from("go faster"),
                manifest_domain::TurnId::new("turn-7")
            )]
        );
        assert!(fake.turns.lock().unwrap().is_empty());
        assert_eq!(user_entries(&app), vec!["go faster"]);
        assert_eq!(app.composer.content.text(), "");
    }

    #[test]
    fn steer_without_turn_id_notes_error_and_keeps_draft() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        // Running with no named turn: the server never sent one.
        stream(
            &mut app,
            manifest_domain::StreamEvent::TurnStarted {
                session: SessionId::new("s-1"),
                turn: None,
                view_cursor: None,
            },
        );
        type_composer_text(&mut app, "nudge");
        send_pressed(&mut app);
        assert!(fake.steers.lock().unwrap().is_empty());
        assert!(fake.turns.lock().unwrap().is_empty());
        assert_eq!(app.composer.content.text(), "nudge");
        assert!(transcript_errors(&app).contains(
            &"That session is still starting its turn, so there is nothing to steer yet. Try again in a moment."
        ));
    }

    #[test]
    fn stop_cancels_running_turn_with_tracked_id() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        start_turn(&mut app, "s-1", "turn-7");
        let _ = app.update(Message::Composer(composer::Message::StopPressed));
        assert_eq!(
            *fake.cancels.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                Some(manifest_domain::TurnId::new("turn-7"))
            )]
        );
        assert!(app.running.contains(&SessionId::new("s-1")));
    }

    #[test]
    fn stop_without_running_session_records_nothing() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let _ = app.update(Message::Composer(composer::Message::StopPressed));
        assert!(fake.cancels.lock().unwrap().is_empty());
    }

    #[test]
    fn turn_completed_clears_turn_id_and_steer_mode() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        start_turn(&mut app, "s-1", "turn-7");
        assert!(app.composer.running_visible);
        stream(
            &mut app,
            manifest_domain::StreamEvent::TurnCompleted {
                session: SessionId::new("s-1"),
                turn: Some(manifest_domain::TurnId::new("turn-7")),
                view_cursor: None,
                terminal: None,
                answer: None,
                dropped_deltas: 0,
            },
        );
        assert!(!app.running.contains(&SessionId::new("s-1")));
        assert!(app.running_turns.is_empty());
        assert!(!app.composer.running_visible);
    }

    #[test]
    fn turn_accepted_tracks_turn_id_for_steer() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        type_composer_text(&mut app, "hello");
        send_pressed(&mut app);
        assert_eq!(fake.turns.lock().unwrap().len(), 1);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::TurnAccepted {
                session: SessionId::new("s-1"),
                turn: Some(manifest_domain::TurnId::new("turn-9")),
            }),
        );
        type_composer_text(&mut app, "adjust");
        send_pressed(&mut app);
        assert_eq!(
            *fake.steers.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                String::from("adjust"),
                manifest_domain::TurnId::new("turn-9")
            )]
        );
    }

    /// A one-row catalog event, as the worker would map it.
    fn models_event() -> engine::Input {
        engine::Input::event(manifest_domain::EngineEvent::ModelsLoaded {
            models: vec![manifest_domain::ModelInfo {
                id: String::from("muse-spark"),
                display_label: String::from("Muse Spark"),
                is_default: true,
                is_active: false,
            }],
        })
    }

    #[test]
    fn connected_sends_no_bare_catalog_request() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        // Cold start names no host: the bare request would fail, so the
        // shell sends none until a session (or draft workspace) names one.
        assert!(fake.models.lock().unwrap().is_empty());
        assert!(!app.models_pending);
        assert_eq!(app.composer.catalog, composer::ModelCatalog::Loading);
    }

    #[test]
    fn sessions_loaded_requests_catalog_with_selected_session() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1", "s-2"]);
        assert_eq!(
            *fake.models.lock().unwrap(),
            vec![(None, Some(SessionId::new("s-1")))]
        );
        assert!(app.models_pending);
        // A second load while the request is in flight sends no duplicate.
        loaded(&mut app, &["s-1", "s-2"]);
        assert_eq!(fake.models.lock().unwrap().len(), 1);
        deliver(&mut app, models_event());
        assert!(!app.models_pending);
        assert!(matches!(
            app.composer.catalog,
            composer::ModelCatalog::Ready(_)
        ));
        // The catalog arrives without changing the selection.
        assert_eq!(app.composer.model_effort_label, "Default · Default");
        // A later load with a ready catalog requests nothing further.
        loaded(&mut app, &["s-1", "s-2"]);
        assert_eq!(fake.models.lock().unwrap().len(), 1);
    }

    #[test]
    fn catalog_failure_marks_catalog_failed_and_keeps_transcript_error() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        assert!(app.models_pending);
        let message = String::from("models requires workspace or session_id");
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::Models,
                message: message.clone(),
            }),
        );
        assert!(!app.models_pending);
        assert_eq!(
            app.composer.catalog,
            composer::ModelCatalog::Failed(message.clone())
        );
        assert!(transcript_errors(&app).contains(&message.as_str()));
        // The menu still names the default choice without a catalog.
        assert_eq!(app.composer.model_effort_label, "Default · Default");
        assert_eq!(fake.models.lock().unwrap().len(), 1);
    }

    #[test]
    fn failed_connection_clears_pending_catalog_request() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        assert!(app.models_pending);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Failed(String::from("down")),
            )),
        );
        assert!(!app.models_pending);
    }

    #[test]
    fn draft_entry_requests_catalog_with_draft_workspace() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        // The list load requested with the selected session.
        assert_eq!(
            *fake.models.lock().unwrap(),
            vec![(None, Some(SessionId::new("s-1")))]
        );
        // The request fails, so the catalog stays unresolved.
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed {
                command: manifest_domain::FailedCommand::Models,
                message: String::from("models down"),
            }),
        );
        assert!(matches!(
            app.composer.catalog,
            composer::ModelCatalog::Failed(_)
        ));
        new_session_pressed(&mut app);
        assert!(app.new_session.is_some());
        // Entering the draft retries with the draft workspace.
        assert_eq!(
            *fake.models.lock().unwrap(),
            vec![
                (None, Some(SessionId::new("s-1"))),
                (Some(String::from("/live/workspace")), None),
            ]
        );
        assert!(app.models_pending);
    }

    #[test]
    fn draft_entry_with_ready_catalog_requests_nothing() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        deliver(&mut app, models_event());
        new_session_pressed(&mut app);
        assert!(app.new_session.is_some());
        // The catalog is ready: entering the draft requests nothing.
        assert_eq!(fake.models.lock().unwrap().len(), 1);
    }

    #[test]
    fn menu_picks_sync_into_open_draft_choices() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        new_session_pressed(&mut app);
        assert!(app.new_session.is_some());
        let _ = app.update(Message::Composer(composer::Message::PermissionSelected(
            manifest_domain::MusePermission::Judge,
        )));
        let _ = app.update(Message::Composer(composer::Message::ModelSelected(
            manifest_domain::ModelChoice::Named(String::from("muse-spark")),
        )));
        let _ = app.update(Message::Composer(composer::Message::EffortSelected(
            manifest_domain::EffortChoice::High,
        )));
        let draft = app.new_session.as_ref().expect("open draft");
        assert_eq!(
            draft.permission,
            manifest_domain::PermissionMode::Muse(manifest_domain::MusePermission::Judge)
        );
        assert_eq!(
            draft.model,
            manifest_domain::ModelChoice::Named(String::from("muse-spark"))
        );
        assert_eq!(draft.effort, manifest_domain::EffortChoice::High);
        type_composer_text(&mut app, "fresh idea");
        send_pressed(&mut app);
        assert_eq!(
            *fake.spawns.lock().unwrap(),
            vec![(
                String::from("/live/workspace"),
                manifest_domain::TurnInput::Text(String::from("fresh idea")),
                manifest_domain::SpawnChoices {
                    worktree: manifest_domain::WorktreeChoice::Off,
                    model: manifest_domain::ModelChoice::Named(String::from("muse-spark")),
                    effort: manifest_domain::EffortChoice::High,
                    permission: manifest_domain::PermissionMode::Muse(
                        manifest_domain::MusePermission::Judge
                    ),
                }
            )]
        );
    }

    #[test]
    fn usage_event_updates_visible_meter_only() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1", "s-2"]);
        assert_eq!(app.composer.usage, None);
        assert_eq!(app.composer.context_percent, None);
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-2"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::TokenUsage,
                usage_json: String::from(r#"{"total": 42}"#),
            },
        );
        // Background usage stores without touching the visible meter.
        assert_eq!(app.composer.usage, None);
        assert_eq!(app.composer.context_percent, None);
        assert_eq!(app.usage.len(), 1);
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-1"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::TokenUsage,
                usage_json: String::from(r#"{"total": 7}"#),
            },
        );
        assert_eq!(
            app.composer.usage.as_ref().map(|snapshot| snapshot.used),
            Some(7)
        );
        assert_eq!(app.composer.context_percent, None);
        // Unknown payloads leave the snapshot in place.
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-1"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::Changed,
                usage_json: String::from("nope"),
            },
        );
        assert_eq!(
            app.composer.usage.as_ref().map(|snapshot| snapshot.used),
            Some(7)
        );
    }

    #[test]
    fn sessions_loaded_prunes_usage_of_unlisted_sessions() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1", "s-2"]);
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-2"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::TokenUsage,
                usage_json: String::from(r#"{"total": 9}"#),
            },
        );
        assert_eq!(app.usage.len(), 1);
        loaded(&mut app, &["s-1"]);
        assert!(app.usage.is_empty());
    }

    /// Loads two listed sessions with carried usage for the first.
    fn loaded_with_carried_usage(app: &mut App) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![live_session("s-1"), live_session("s-2")],
                usage: vec![(
                    SessionId::new("s-1"),
                    manifest_domain::UsageSnapshot::parse(r#"{"total": 42, "limit": 200}"#)
                        .expect("carried usage parses"),
                )],
            }),
        );
    }

    #[test]
    fn sessions_loaded_usage_replaces_stored_snapshot() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1", "s-2"]);
        // Stream snapshots land first; the status reconcile replaces them.
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-1"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::TokenUsage,
                usage_json: String::from(r#"{"total": 7}"#),
            },
        );
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-2"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::TokenUsage,
                usage_json: String::from(r#"{"total": 9}"#),
            },
        );
        assert_eq!(app.usage[&SessionId::new("s-1")].used, 7);
        loaded_with_carried_usage(&mut app);
        // The carried snapshot replaces the stale stream value, and the
        // visible meter follows through the existing sync path.
        assert_eq!(app.usage[&SessionId::new("s-1")].used, 42);
        assert_eq!(app.usage[&SessionId::new("s-1")].window, Some(200));
        assert_eq!(
            app.composer.usage.as_ref().map(|snapshot| snapshot.used),
            Some(42)
        );
        assert_eq!(app.composer.context_percent, Some(21.0));
        // Sessions absent from the carried vec keep their snapshot.
        assert_eq!(app.usage[&SessionId::new("s-2")].used, 9);
    }

    #[test]
    fn sessions_loaded_without_usage_keeps_meter() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-1"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::TokenUsage,
                usage_json: String::from(r#"{"total": 7}"#),
            },
        );
        assert_eq!(
            app.composer.usage.as_ref().map(|snapshot| snapshot.used),
            Some(7)
        );
        // A row without token_usage carries no entry, so the stored
        // snapshot and the meter stay put.
        loaded(&mut app, &["s-1"]);
        assert_eq!(app.usage[&SessionId::new("s-1")].used, 7);
        assert_eq!(
            app.composer.usage.as_ref().map(|snapshot| snapshot.used),
            Some(7)
        );
        assert_eq!(app.composer.context_percent, None);
    }

    #[test]
    fn composer_meter_reconciles_with_and_without_window() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        assert_eq!(app.composer.usage, None);
        assert_eq!(app.composer.context_percent, None);
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-1"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::TokenUsage,
                usage_json: String::from(r#"{"total": 7}"#),
            },
        );
        assert_eq!(
            app.composer.usage.as_ref().map(|snapshot| snapshot.used),
            Some(7)
        );
        assert_eq!(app.composer.context_percent, None);
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-1"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::TokenUsage,
                usage_json: String::from(r#"{"total": 42, "limit": 200}"#),
            },
        );
        assert_eq!(
            app.composer.usage.as_ref().map(|snapshot| snapshot.used),
            Some(42)
        );
        assert_eq!(app.composer.context_percent, Some(21.0));
        // A zero window is not a window: no meter fill.
        stream(
            &mut app,
            manifest_domain::StreamEvent::Usage {
                session: SessionId::new("s-1"),
                turn: None,
                view_cursor: None,
                kind: manifest_domain::UsageKind::TokenUsage,
                usage_json: String::from(r#"{"total": 5, "limit": 0}"#),
            },
        );
        assert_eq!(
            app.composer.usage.as_ref().map(|snapshot| snapshot.used),
            Some(5)
        );
        assert_eq!(app.composer.context_percent, None);
    }

    #[test]
    fn topbar_segments_follow_selection_draft_and_branch() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        // No listed session and no draft: no segments.
        assert_eq!(app.topbar_project_name(), None);
        assert_eq!(app.branch, None);
        loaded(&mut app, &["s-1", "s-2"]);
        // The selected session's project group names the segment; the
        // branch follows the finished read.
        assert_eq!(app.topbar_project_name().as_deref(), Some("workspace"));
        assert_eq!(app.branch, None);
        let _ = app.update(Message::BranchLoaded {
            workspace: String::from("/live/workspace"),
            branch: Some(String::from("feature/login")),
        });
        assert_eq!(app.branch.as_deref(), Some("feature/login"));
        // Draft mode names the draft's project instead.
        new_session_pressed(&mut app);
        assert!(app.new_session.is_some());
        assert_eq!(app.topbar_project_name().as_deref(), Some("workspace"));
        // A selected session with an empty workspace names nothing.
        let _ = app.update(Message::Sessions(sessions::Message::Select(
            SessionId::new("s-1"),
        )));
        assert!(app.new_session.is_none());
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![workspace_session("s-9", "")],
                usage: Vec::new(),
            }),
        );
        assert_eq!(app.topbar_project_name(), None);
    }

    #[test]
    fn draft_send_appends_attachment_refs_to_prompt() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded_with_workspace(&mut app);
        new_session_pressed(&mut app);
        app.composer
            .insert_attachment(String::from("/work/notes.md"));
        type_composer_text(&mut app, "fresh idea");
        send_pressed(&mut app);
        assert_eq!(
            *fake.spawns.lock().unwrap(),
            vec![(
                String::from("/live/workspace"),
                manifest_domain::TurnInput::Text(
                    String::from("fresh idea\n[ref: /work/notes.md]",)
                ),
                manifest_domain::SpawnChoices::default()
            )]
        );
        assert_eq!(
            user_entries(&app),
            vec!["fresh idea\n[ref: /work/notes.md]"]
        );
        // Sending clears the attachments with the draft.
        assert!(app.composer.attachments.is_empty());
    }

    #[test]
    fn follow_up_send_appends_attachment_refs_to_prompt() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        app.composer
            .insert_attachment(String::from("/work/notes.md"));
        type_composer_text(&mut app, "hello");
        send_pressed(&mut app);
        assert_eq!(
            *fake.turns.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                manifest_domain::TurnInput::Text(String::from("hello\n[ref: /work/notes.md]"))
            )]
        );
    }

    #[test]
    fn steer_send_appends_attachment_refs_to_prompt() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        start_turn(&mut app, "s-1", "turn-7");
        app.composer
            .insert_attachment(String::from("/work/notes.md"));
        type_composer_text(&mut app, "go faster");
        send_pressed(&mut app);
        assert_eq!(
            *fake.steers.lock().unwrap(),
            vec![(
                SessionId::new("s-1"),
                String::from("go faster\n[ref: /work/notes.md]"),
                manifest_domain::TurnId::new("turn-7")
            )]
        );
    }

    #[test]
    fn attach_picked_inserts_path_and_dismissed_closes() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        let _ = app.update(Message::Composer(composer::Message::AttachPressed));
        assert_eq!(app.composer.attach_popup, composer::AttachPopup::Open);
        let _ = app.update(Message::Composer(composer::Message::AttachPicked(Some(
            String::from("/work/notes.md"),
        ))));
        assert_eq!(
            app.composer.attachments,
            vec![String::from("/work/notes.md")]
        );
        assert_eq!(app.composer.attach_popup, composer::AttachPopup::Closed);
        let _ = app.update(Message::Composer(composer::Message::AttachPressed));
        let _ = app.update(Message::Composer(composer::Message::AttachPicked(None)));
        assert_eq!(app.composer.attach_popup, composer::AttachPopup::Closed);
        assert_eq!(app.composer.attachments.len(), 1);
    }

    #[test]
    fn context_dismissed_through_root_closes_context_popover() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        // Dismissing a closed popover is harmless.
        let _ = app.update(Message::Composer(composer::Message::ContextDismissed));
        assert_eq!(
            app.composer.context_popover,
            composer::ContextPopover::Closed
        );
        let _ = app.update(Message::Composer(composer::Message::ContextPressed));
        assert_eq!(app.composer.context_popover, composer::ContextPopover::Open);
        let _ = app.update(Message::Composer(composer::Message::ContextDismissed));
        assert_eq!(
            app.composer.context_popover,
            composer::ContextPopover::Closed
        );
    }

    #[test]
    fn attach_dismissed_through_root_closes_attach_menu() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
        // Dismissing a closed menu is harmless.
        let _ = app.update(Message::Composer(composer::Message::AttachDismissed));
        assert_eq!(app.composer.attach_popup, composer::AttachPopup::Closed);
        let _ = app.update(Message::Composer(composer::Message::AttachPressed));
        assert_eq!(app.composer.attach_popup, composer::AttachPopup::Open);
        let _ = app.update(Message::Composer(composer::Message::AttachDismissed));
        assert_eq!(app.composer.attach_popup, composer::AttachPopup::Closed);
    }

    /// A scratch workspace dir under the system temp dir, unique per
    /// test name. Removed first so reruns start clean.
    fn temp_workspace(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "manifest-branch-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch workspace");
        dir
    }

    /// Writes `.git/HEAD` content for a plain (non-worktree) checkout.
    fn write_head(workspace: &std::path::Path, content: &str) {
        let git = workspace.join(".git");
        std::fs::create_dir_all(&git).expect("scratch .git");
        std::fs::write(git.join("HEAD"), content).expect("scratch HEAD");
    }

    /// Loads two listed sessions in distinct workspaces; s-1 visible.
    fn loaded_workspaces(app: &mut App, first: &str, second: &str) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: vec![
                    workspace_session("s-1", first),
                    workspace_session("s-2", second),
                ],
                usage: Vec::new(),
            }),
        );
    }

    #[test]
    fn branch_loaded_stores_for_current_workspace() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded_workspaces(&mut app, "/live/first", "/live/second");
        assert_eq!(app.visible_workspace().as_deref(), Some("/live/first"));
        let _ = app.update(Message::BranchLoaded {
            workspace: String::from("/live/first"),
            branch: Some(String::from("feature/login")),
        });
        assert_eq!(app.branch.as_deref(), Some("feature/login"));
        assert_eq!(app.branch_workspace.as_deref(), Some("/live/first"));
    }

    #[test]
    fn branch_loaded_drops_stale_workspace() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded_workspaces(&mut app, "/live/first", "/live/second");
        let _ = app.update(Message::BranchLoaded {
            workspace: String::from("/live/second"),
            branch: Some(String::from("other")),
        });
        assert_eq!(app.branch, None);
        assert_eq!(app.branch_workspace, None);
        // A read failure for the current workspace stores `None`
        // without touching the transcript.
        let before = app.transcript.entries.len();
        let _ = app.update(Message::BranchLoaded {
            workspace: String::from("/live/first"),
            branch: None,
        });
        assert_eq!(app.branch, None);
        assert_eq!(app.branch_workspace.as_deref(), Some("/live/first"));
        assert_eq!(app.transcript.entries.len(), before);
    }

    #[test]
    fn select_moves_visible_workspace_and_branch_follows() {
        let first = temp_workspace("select-first");
        let second = temp_workspace("select-second");
        write_head(&first, "ref: refs/heads/main\n");
        write_head(&second, "ref: refs/heads/feature/x\n");
        let first = first.display().to_string();
        let second = second.display().to_string();

        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded_workspaces(&mut app, &first, &second);
        assert_eq!(app.visible_workspace().as_deref(), Some(first.as_str()));

        // Selecting s-2 moves the visible workspace; the branch task
        // the select fired runs `read_branch_blocking`, whose result
        // the `BranchLoaded` arm then stores.
        let _ = app.update(Message::Sessions(crate::sessions::Message::Select(
            manifest_domain::SessionId::new("s-2"),
        )));
        assert_eq!(app.visible_workspace().as_deref(), Some(second.as_str()));
        let branch = App::read_branch_blocking(&second);
        assert_eq!(branch.as_deref(), Some("feature/x"));
        let _ = app.update(Message::BranchLoaded {
            workspace: second.clone(),
            branch,
        });
        assert_eq!(app.branch.as_deref(), Some("feature/x"));
        assert_eq!(app.branch_workspace.as_deref(), Some(second.as_str()));

        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    #[test]
    fn read_branch_blocking_covers_git_layouts() {
        // Normal branch checkout.
        let plain = temp_workspace("read-plain");
        write_head(&plain, "ref: refs/heads/main\n");
        let plain = plain.display().to_string();
        assert_eq!(App::read_branch_blocking(&plain).as_deref(), Some("main"));

        // Detached HEAD shortens the hash.
        let detached = temp_workspace("read-detached");
        write_head(&detached, "9d3a1f2c4b5e6a7890abcdef1234567890abcdef1\n");
        let detached = detached.display().to_string();
        assert_eq!(
            App::read_branch_blocking(&detached).as_deref(),
            Some("9d3a1f2")
        );

        // Worktree checkout: `.git` is a pointer file, absolute target.
        let target = temp_workspace("read-worktree-target");
        write_head(&target, "ref: refs/heads/fix/retry\n");
        let worktree = temp_workspace("read-worktree");
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}/.git\n", target.display()),
        )
        .expect("scratch gitdir file");
        let worktree_str = worktree.display().to_string();
        assert_eq!(
            App::read_branch_blocking(&worktree_str).as_deref(),
            Some("fix/retry")
        );

        // Worktree checkout with a target relative to the workspace.
        let outer = temp_workspace("read-relative");
        let real = outer.join("real");
        std::fs::create_dir_all(real.join(".git")).expect("scratch real .git");
        std::fs::write(real.join(".git").join("HEAD"), "ref: refs/heads/rel\n")
            .expect("scratch HEAD");
        let linked = outer.join("linked");
        std::fs::create_dir_all(&linked).expect("scratch linked");
        std::fs::write(linked.join(".git"), "gitdir: ../real/.git\n").expect("scratch gitdir file");
        let linked_str = linked.display().to_string();
        assert_eq!(
            App::read_branch_blocking(&linked_str).as_deref(),
            Some("rel")
        );

        // Missing `.git` and malformed HEAD both read as `None`.
        let missing = temp_workspace("read-missing");
        let missing = missing.display().to_string();
        assert_eq!(App::read_branch_blocking(&missing), None);
        let bad = temp_workspace("read-bad");
        write_head(&bad, "definitely not a head\n");
        let bad = bad.display().to_string();
        assert_eq!(App::read_branch_blocking(&bad), None);

        for dir in [
            &plain,
            &detached,
            &worktree_str,
            &linked_str,
            &missing,
            &bad,
        ] {
            let _ = std::fs::remove_dir_all(dir);
        }
        let _ = std::fs::remove_dir_all(outer.display().to_string());
        let _ = std::fs::remove_dir_all(target.display().to_string());
    }
}
