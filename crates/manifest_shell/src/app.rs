//! Root shell: the four-region layout, the root reducer, and boot.
//!
//! The top bar is a view function on the root state, not a module with
//! invented state (plan chunk 4): it owns nothing, so it owns no file.
//! Child features own their state; this module holds the child states,
//! routes messages down, and maps child messages up (UI contract §7).

use std::collections::{HashMap, HashSet};

use iced::{Element, Length, Task};
use manifest_domain::{
    Connection, EffortChoice, EngineEvent, Fixture, ModelChoice, PermissionMode, ProjectId,
    ProjectSummary, SessionId, SessionStatus, SessionSummary, SpawnChoices, StreamEvent, TodoItem,
    TurnId, UsageSnapshot, WorktreeChoice,
};

use crate::engine;
use crate::kit;
use crate::theme::{
    DIVIDER_HIT_WIDTH, FONT_SIZE_BODY, FONT_SIZE_SMALL, FONT_SIZE_TITLE, MODEL_MENU_BOTTOM_OFFSET,
    PANEL_WIDTH_DEFAULT, PANEL_WIDTH_MAX, PANEL_WIDTH_MIN, SPACE_M, SPACE_S, TEXT_PRIMARY,
    TEXT_SECONDARY, TEXT_TERTIARY, WINDOW_RESIZE_HANDLE,
};
use crate::{composer, sessions, transcript};

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
/// "+" button, resolved into a real session on the next send.
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
    /// adopts the transcript) or `CommandFailed`.
    pub awaiting_spawn: bool,
    /// The in-flight spawn, set together with [`App::awaiting_spawn`]
    /// whenever a spawn is issued; `Some` means the spawn can be
    /// restored on failure.
    pub pending_spawn: Option<PendingSpawn>,
    /// Orphaned sessions with a resume in flight; gates duplicate
    /// resumes and the send button.
    pub resuming: HashSet<SessionId>,
    /// A new session being composed in a project's workspace; set by
    /// the project's "+" button, cleared on send (which spawns it) or
    /// on session select. While set, the visible transcript is the
    /// draft's, and list syncs must not steal it.
    pub new_session: Option<NewSessionDraft>,
    /// Draft input cluster.
    pub composer: composer::State,
    /// A model-catalog request is in flight. Set on each request,
    /// cleared on `ModelsLoaded` (or on the `CommandFailed` that marks
    /// the catalog failed, or on disconnect).
    pub models_pending: bool,
    /// Checklist shown in the drawer, from the fixture.
    pub todos: Vec<TodoItem>,
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
        let done = fixture.todos.iter().filter(|todo| todo.done).count();
        let total = fixture.todos.len();
        let todo_summary = format!("{done} of {total} done");
        let current_todo = fixture
            .todos
            .iter()
            .find(|todo| !todo.done)
            .map(|todo| todo.label.clone());
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
            new_session: None,
            composer: composer::State::new(),
            models_pending: false,
            todos: fixture.todos,
            todo_drawer: TodoDrawer::Peek,
            todo_summary,
            current_todo,
            panel_width: PANEL_WIDTH_DEFAULT,
            panel_drag: PanelDrag::Idle,
            window_id: None,
            connection: Connection::Disconnected,
            connection_attempt: 0,
            engine: None,
            engine_config: None,
            project_store: None,
            projects: Vec::new(),
            projects_ready: false,
            projects_missing: false,
        };
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
        app.todos = Vec::new();
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
        self.composer.running_visible = running_visible;
        self.composer.send_enabled = self.engine_config.is_some()
            && self.connection == Connection::Connected
            && self.engine.is_some()
            && !self.awaiting_spawn
            && !gated_by_resume;
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

    /// Points the composer's usage meter at the visible session's
    /// snapshot. Called after usage events and transcript syncs; the
    /// composer stores the strings, so the view formats nothing.
    fn sync_composer_usage(&mut self) {
        let snapshot = self
            .visible_session
            .as_ref()
            .and_then(|session| self.usage.get(session))
            .cloned();
        self.composer.set_usage(snapshot);
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
            self.sessions.expanded = Some(existing);
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
        self.sessions.expanded = Some(id);
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

    /// Spawns a session in the given workspace with the draft prompt:
    /// the shared tail of the draft spawn and the no-selection cwd
    /// fallback. Echoes the prompt optimistically, clears the draft,
    /// and marks the spawn in flight until `SessionSpawned` adopts it.
    /// Keeps the taken draft (when the spawn came from one) and the
    /// prompt so a failure can restore them for retry.
    fn spawn_session(
        &mut self,
        draft: Option<NewSessionDraft>,
        workspace: String,
        prompt: String,
        choices: SpawnChoices,
    ) -> Task<Message> {
        if let Some(engine) = self.engine.clone() {
            engine.spawn_session(workspace, prompt.clone(), choices);
        }
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
    /// friendly note and the draft stays; a new-session draft spawns
    /// into its project workspace; a listed selection sends a
    /// follow-up turn unless one is already in flight (steering the
    /// running turn there instead); otherwise the draft spawns a
    /// session in the process working directory with the shown picker
    /// state. Every send carries the composed prompt: the draft text
    /// with attached path refs appended as ref lines.
    /// Sends echo the prompt optimistically, clear the draft, and
    /// snap the transcript to the end.
    fn send_from_composer(&mut self) -> Task<Message> {
        if self.engine_config.is_none() {
            return Task::none();
        }
        if self.composer.content.text().trim().is_empty() {
            return Task::none();
        }
        let prompt = self.composer.composed_prompt();
        if self.connection != Connection::Connected || self.engine.is_none() {
            let appended = self.transcript.push_error(not_connected_message());
            return scroll_transcript_to_end(appended);
        }
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
            return self.spawn_session(Some(draft), workspace, prompt, choices);
        }
        if let Some(summary) = self.sessions.selected_summary() {
            let session = summary.id.clone();
            if self.running.contains(&session) {
                // Steer: the visible session is mid-turn, so the draft
                // joins it instead of starting a new one.
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
                engine.send_turn(session.clone(), prompt.clone());
            }
            self.transcript.push_user(prompt);
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
        self.spawn_session(None, workspace, prompt, choices)
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
            StreamEvent::Pending { .. } => {
                if self.connection == Connection::Connected
                    && let Some(engine) = &self.engine
                {
                    engine.list_sessions();
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
                        self.resuming.insert(id);
                    }
                }
                self.refresh_send_gate();
                Task::none()
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
            Message::Transcript(child) => match child {
                // Clipboard writes are tasks, which state cannot produce.
                transcript::Message::Copy(text) => iced::clipboard::write(text),
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
                            }
                        }
                        EngineEvent::SessionsLoaded { sessions } => {
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
                            }
                            self.sync_visible_transcript();
                            // Status reconciles: snapshots for unlisted
                            // sessions drop with their rows.
                            let listed: std::collections::HashSet<SessionId> =
                                self.current_sessions().into_iter().map(|s| s.id).collect();
                            self.usage.retain(|id, _| listed.contains(id));
                            // Cold start names no host until a session is
                            // listed: request the catalog now, with the
                            // selected session as its context.
                            self.request_models_for_selection();
                        }
                        EngineEvent::SessionsLoadFailed(message) => {
                            appended = self.transcript.push_error(message);
                        }
                        EngineEvent::ModelsLoaded { models } => {
                            self.models_pending = false;
                            self.composer
                                .set_catalog(composer::ModelCatalog::Ready(models));
                        }
                        EngineEvent::CommandFailed(message) => {
                            if self.models_pending {
                                // The catalog request failed: attribute
                                // it to the catalog while keeping the
                                // transcript error below.
                                self.models_pending = false;
                                self.composer
                                    .set_catalog(composer::ModelCatalog::Failed(message.clone()));
                            }
                            appended = self.transcript.push_error(message);
                            if self.awaiting_spawn {
                                if let Some(pending) = self.pending_spawn.take() {
                                    if let Some(draft) = pending.draft {
                                        self.new_session = Some(draft);
                                    }
                                    self.composer.restore(pending.prompt);
                                }
                            } else {
                                // Failures release resume gates so the next select retries.
                                self.resuming.clear();
                            }
                            if let Some(visible) = self.visible_session.clone() {
                                self.running.remove(&visible);
                                self.running_turns.remove(&visible);
                            }
                            self.awaiting_spawn = false;
                        }
                        EngineEvent::SessionSpawned { session, turn } => {
                            self.running.insert(session.clone());
                            if let Some(turn) = turn {
                                self.running_turns.insert(session.clone(), turn);
                            }
                            self.select_on_load = Some(session.clone());
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
                            } else {
                                self.sync_visible_transcript();
                            }
                            if let Some(engine) = &self.engine {
                                engine.list_sessions();
                            }
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
                            if self.connection == Connection::Connected
                                && let Some(engine) = &self.engine
                            {
                                engine.list_sessions();
                            }
                        }
                        EngineEvent::Diagnostic(message) => {
                            appended = self.transcript.push_error(message);
                        }
                        EngineEvent::Stream(event) => {
                            appended = self.apply_stream_event(event);
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
        iced::Subscription::batch([drag, engine])
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
            iced::widget::container(self.transcript.view().map(Message::Transcript))
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .spacing(SPACE_S)
        .width(Length::Fill)
        .height(Length::Fill);
        if !self.todos.is_empty() {
            let current = self.current_todo.as_deref().unwrap_or("All done");
            let drawer: Element<'_, Message> = match self.todo_drawer {
                TodoDrawer::Peek => {
                    kit::peek_strip(current, &self.todo_summary, Message::ToggleTodoDrawer)
                }
                TodoDrawer::Open => todo_drawer_view(&self.todos, current, &self.todo_summary),
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
    // The two Fill regions flanking the title share the leftover
    // width equally, so the title stays centered no matter how wide
    // the window controls run.
    let title_row = iced::widget::row![
        drag_region(
            iced::widget::container(iced::widget::text("").size(FONT_SIZE_BODY))
                .width(Length::Fill),
        ),
        drag_region(iced::widget::container(
            iced::widget::text(selected_title)
                .size(FONT_SIZE_BODY)
                .color(TEXT_SECONDARY),
        )),
        iced::widget::container({
            let mut controls = iced::widget::row![];
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
                    "−",
                    kit::IconButtonKind::Subtle,
                    Some(Message::MinimizeWindow),
                ))
                .push(kit::icon_button(
                    "□",
                    kit::IconButtonKind::Subtle,
                    Some(Message::ToggleMaximizeWindow),
                ))
                .push(kit::icon_button(
                    "✕",
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

/// The open TODO drawer: a panel whose first row is the shared bar
/// header, followed by one line per todo with finished steps in the
/// tertiary tone. Plain text lines — no extra widget kinds.
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
        // No string building in the view (UI contract §2): the done
        // marker is a static label beside the borrowed todo text.
        let (marker, color) = if todo.done {
            ("Done", TEXT_TERTIARY)
        } else {
            ("Todo", TEXT_PRIMARY)
        };
        list = list.push(
            iced::widget::container(
                iced::widget::row![
                    iced::widget::text(marker)
                        .size(FONT_SIZE_SMALL)
                        .color(color),
                    iced::widget::text(todo.label.as_str())
                        .size(FONT_SIZE_SMALL)
                        .color(color),
                ]
                .spacing(SPACE_S),
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

    #[derive(Clone)]
    struct FakeEngine {
        calls: std::sync::Arc<std::sync::Mutex<usize>>,
        spawns:
            std::sync::Arc<std::sync::Mutex<Vec<(String, String, manifest_domain::SpawnChoices)>>>,
        turns: std::sync::Arc<std::sync::Mutex<Vec<(manifest_domain::SessionId, String)>>>,
        resumes: std::sync::Arc<std::sync::Mutex<Vec<manifest_domain::SessionId>>>,
        steers: std::sync::Arc<std::sync::Mutex<Vec<SteerCall>>>,
        cancels: std::sync::Arc<std::sync::Mutex<Vec<CancelCall>>>,
        models: std::sync::Arc<std::sync::Mutex<Vec<ModelsCall>>>,
    }

    impl manifest_domain::SessionEngine for FakeEngine {
        fn list_sessions(&self) {
            *self.calls.lock().unwrap() += 1;
        }

        fn spawn_session(
            &self,
            workspace: String,
            prompt: String,
            choices: manifest_domain::SpawnChoices,
        ) {
            self.spawns
                .lock()
                .unwrap()
                .push((workspace, prompt, choices));
        }

        fn send_turn(&self, session: manifest_domain::SessionId, prompt: String) {
            self.turns.lock().unwrap().push((session, prompt));
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
    fn pending_hint_refreshes_sessions() {
        let (mut app, fake) = live_app();
        deliver_connected(&mut app);
        assert_eq!(*fake.calls.lock().unwrap(), 1);
        deliver(
            &mut app,
            engine::Input::event(manifest_domain::EngineEvent::Stream(
                manifest_domain::StreamEvent::Pending {
                    session: manifest_domain::SessionId::new("s-1"),
                    turn: None,
                    request_id: String::from("req-1"),
                    kind: manifest_domain::PendingKind::Approval,
                    phase: manifest_domain::PendingPhase::Requested,
                },
            )),
        );
        assert_eq!(*fake.calls.lock().unwrap(), 2);
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

    /// Loads a live session list with the given ids.
    fn loaded(app: &mut App, ids: &[&str]) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                sessions: ids.iter().map(|id| live_session(id)).collect(),
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
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed(String::from(
                "nope",
            ))),
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
        assert!(app.todos.is_empty());
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
            vec![(SessionId::new("s-1"), String::from("hello"))]
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
                String::from("start me"),
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

    /// Presses the first listed project's "+" button through the root
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
        assert_eq!(
            app.sessions.expanded,
            Some(manifest_domain::ProjectId::for_workspace("/live/workspace"))
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
                String::from("fresh idea"),
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
        assert_eq!(
            app.sessions.expanded,
            Some(manifest_domain::ProjectId::new("project-live"))
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
        assert_eq!(
            app.sessions.expanded,
            Some(manifest_domain::ProjectId::for_workspace("/live/workspace"))
        );
    }

    #[test]
    fn fixture_new_session_expands_without_draft() {
        let mut app = App::boot();
        let _ = app.update(Message::Sessions(sessions::Message::NewSession(
            manifest_domain::ProjectId::new("project-checkout-api"),
        )));
        assert_eq!(app.new_session, None);
        assert_eq!(
            app.sessions.expanded,
            Some(manifest_domain::ProjectId::new("project-checkout-api"))
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
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded { sessions }),
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
        assert_eq!(
            app.sessions.expanded,
            Some(manifest_domain::ProjectId::for_workspace(&dir))
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
        assert_eq!(app.sessions.expanded, Some(existing.id));
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
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed(message.clone())),
        );
        assert!(transcript_errors(&app).contains(&message.as_str()));
        // The gate released: reselecting retries the resume.
        select(&mut app, "sess-orphan");
        assert_eq!(fake.resumes.lock().unwrap().len(), 2);
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
                String::from("build it"),
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
                String::from("build it"),
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
                String::from("build it"),
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
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed(message.clone())),
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
                    String::from("build it"),
                    manifest_domain::SpawnChoices {
                        worktree: manifest_domain::WorktreeChoice::Existing {
                            path: String::from("/tmp/wt-x"),
                        },
                        ..Default::default()
                    }
                ),
                (
                    String::from("/live/workspace"),
                    String::from("build it"),
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
                String::from("start me"),
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
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed(String::from(
                "spawn broke",
            ))),
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
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed(message.clone())),
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
            engine::Input::event(manifest_domain::EngineEvent::CommandFailed(String::from(
                "models down",
            ))),
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
                String::from("fresh idea"),
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
        assert_eq!(app.composer.context_label, "Context —");
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
        assert_eq!(app.composer.context_label, "Context —");
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
        assert_eq!(app.composer.context_label, "7 tokens");
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
        assert_eq!(app.composer.context_label, "7 tokens");
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
                String::from("fresh idea\n[ref: /work/notes.md]"),
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
                String::from("hello\n[ref: /work/notes.md]")
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
}
