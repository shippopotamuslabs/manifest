//! Root shell: the four-region layout, the root reducer, and boot.
//!
//! The top bar is a view function on the root state, not a module with
//! invented state (plan chunk 4): it owns nothing, so it owns no file.
//! Child features own their state; this module holds the child states,
//! routes messages down, and maps child messages up (UI contract §7).

use std::collections::{HashMap, HashSet};

use iced::{Element, Length, Task};
use manifest_domain::{
    Connection, EngineEvent, Fixture, ProjectId, SessionId, StreamEvent, TodoItem,
};

use crate::engine;
use crate::kit;
use crate::theme::{
    FONT_SIZE_BODY, FONT_SIZE_SMALL, FONT_SIZE_TITLE, PANEL_WIDTH_DEFAULT, PANEL_WIDTH_MAX,
    PANEL_WIDTH_MIN, SPACE_M, SPACE_S, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
    WINDOW_RESIZE_HANDLE,
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

/// A session being composed before it exists: entered from a project's
/// "+" button, resolved into a real session on the next send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSessionDraft {
    /// Folder the spawned session will run in, from the project.
    pub workspace: String,
    /// Precomputed "project · New session" title the topbar reads.
    pub title: String,
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
    /// A spawned session awaiting its first list load to select it.
    pub select_on_load: Option<SessionId>,
    /// A spawn command is in flight and the visible transcript holds
    /// its optimistic prompt; cleared on `SessionSpawned` (which
    /// adopts the transcript) or `CommandFailed`.
    pub awaiting_spawn: bool,
    /// A new session being composed in a project's workspace; set by
    /// the project's "+" button, cleared on send (which spawns it) or
    /// on session select. While set, the visible transcript is the
    /// draft's, and list syncs must not steal it.
    pub new_session: Option<NewSessionDraft>,
    /// Draft input cluster.
    pub composer: composer::State,
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
            select_on_load: None,
            awaiting_spawn: false,
            new_session: None,
            composer: composer::State::new(),
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
        };
        app.refresh_send_gate();
        app
    }

    /// Production boot: connects to the real server. Starts empty (no
    /// sessions, no transcript entries, no todos) with the connection
    /// at `Connecting`, then connects on the first subscription tick;
    /// sessions arrive through the engine stream.
    pub fn live(config: manifest_mcp::ServerConfig) -> Self {
        let mut app = Self::from_fixture(manifest_domain::fixture());
        app.sessions = sessions::State::from_projects(Vec::new(), Vec::new());
        app.transcript = transcript::State::default();
        app.todos = Vec::new();
        app.todo_summary = String::from("0 of 0 done");
        app.current_todo = None;
        app.connection = Connection::Connecting;
        app.engine_config = Some(config);
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
        self.new_session = Some(NewSessionDraft { workspace, title });
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
    /// single writer of [`composer::State::send_enabled`] outside
    /// [`composer::State::new`]: send needs a server config, a live
    /// connection with a handle, no spawn in flight, and no turn in
    /// flight on the listed selection (when one is listed).
    fn refresh_send_gate(&mut self) {
        let selected_running = self
            .sessions
            .selected_summary()
            .is_some_and(|summary| self.running.contains(&summary.id));
        // A draft is a spawn, not a follow-up: the selected session's
        // running state does not gate it.
        let gated_by_selection = selected_running && self.new_session.is_none();
        self.composer.send_enabled = self.engine_config.is_some()
            && self.connection == Connection::Connected
            && self.engine.is_some()
            && !self.awaiting_spawn
            && !gated_by_selection;
    }

    /// Spawns a session in the given workspace with the draft prompt:
    /// the shared tail of the draft spawn and the no-selection cwd
    /// fallback. Echoes the prompt optimistically, clears the draft,
    /// and marks the spawn in flight until `SessionSpawned` adopts it.
    fn spawn_session(&mut self, workspace: String, prompt: String) -> Task<Message> {
        if let Some(engine) = self.engine.clone() {
            engine.spawn_session(workspace, prompt.clone());
        }
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
    /// follow-up turn unless one is already in flight; otherwise the
    /// draft spawns a session in the process working directory.
    /// Sends echo the prompt optimistically, clear the draft, and
    /// snap the transcript to the end.
    fn send_from_composer(&mut self) -> Task<Message> {
        if self.engine_config.is_none() {
            return Task::none();
        }
        let text = self.composer.content.text();
        if text.trim().is_empty() {
            return Task::none();
        }
        if self.connection != Connection::Connected || self.engine.is_none() {
            let appended = self.transcript.push_error(not_connected_message());
            return scroll_transcript_to_end(appended);
        }
        if let Some(draft) = self.new_session.take() {
            return self.spawn_session(draft.workspace, text);
        }
        if let Some(summary) = self.sessions.selected_summary() {
            let session = summary.id.clone();
            if self.running.contains(&session) {
                return Task::none();
            }
            if let Some(engine) = self.engine.clone() {
                engine.send_turn(session.clone(), text.clone());
            }
            self.transcript.push_user(text);
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
        self.spawn_session(workspace, text)
    }

    /// Folds one stream event into the owning session's transcript:
    /// the visible state when the event names it, else the stored one.
    /// Returns whether the visible transcript gained an entry, so the
    /// scroll can snap to the end. Deltas never scroll.
    fn apply_stream_event(&mut self, event: StreamEvent) -> bool {
        match event {
            StreamEvent::TurnStarted { session, .. } => {
                self.running.insert(session);
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
            // Usage reports stay out of the transcript in this slice.
            StreamEvent::Usage { .. } => false,
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
                self.refresh_send_gate();
                Task::none()
            }
            Message::Composer(composer::Message::SendPressed) => {
                let task = self.send_from_composer();
                self.refresh_send_gate();
                task
            }
            Message::Composer(child) => {
                self.composer.update(child);
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
                                    engine.list_sessions();
                                }
                            } else if matches!(self.connection, Connection::Failed(_)) {
                                self.engine = None;
                                self.running.clear();
                            }
                        }
                        EngineEvent::SessionsLoaded { projects, sessions } => {
                            self.sessions.replace(projects, sessions);
                            if let Some(spawned) = self.select_on_load.clone()
                                && self.session_is_listed(&spawned)
                            {
                                self.sessions.update(sessions::Message::Select(spawned));
                                self.select_on_load = None;
                            }
                            self.sync_visible_transcript();
                        }
                        EngineEvent::SessionsLoadFailed(message) => {
                            appended = self.transcript.push_error(message);
                        }
                        EngineEvent::CommandFailed(message) => {
                            appended = self.transcript.push_error(message);
                            if let Some(visible) = self.visible_session.clone() {
                                self.running.remove(&visible);
                            }
                            self.awaiting_spawn = false;
                        }
                        EngineEvent::SessionSpawned { session, .. } => {
                            self.running.insert(session.clone());
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
                            } else {
                                self.sync_visible_transcript();
                            }
                            if let Some(engine) = &self.engine {
                                engine.list_sessions();
                            }
                        }
                        EngineEvent::TurnAccepted { session, .. } => {
                            self.running.insert(session);
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
                if appended {
                    transcript::scroll_to_end().map(Message::Transcript)
                } else {
                    Task::none()
                }
            }
            Message::Reconnect => {
                if self.engine_config.is_some() {
                    // Dropping the handle shuts the old worker down; the
                    // subscription restarts under the new attempt.
                    self.engine = None;
                    self.connection = Connection::Connecting;
                    self.connection_attempt += 1;
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

/// User-facing note when send is pressed while the engine is not
/// connected yet. Matches the worker's wording for the same refusal.
fn not_connected_message() -> String {
    String::from("The Muse MCP server isn't connected yet. Try again once it connects.")
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
    #[derive(Clone)]
    struct FakeEngine {
        calls: std::sync::Arc<std::sync::Mutex<usize>>,
        spawns: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
        turns: std::sync::Arc<std::sync::Mutex<Vec<(manifest_domain::SessionId, String)>>>,
    }

    impl manifest_domain::SessionEngine for FakeEngine {
        fn list_sessions(&self) {
            *self.calls.lock().unwrap() += 1;
        }

        fn spawn_session(&self, workspace: String, prompt: String) {
            self.spawns.lock().unwrap().push((workspace, prompt));
        }

        fn send_turn(&self, session: manifest_domain::SessionId, prompt: String) {
            self.turns.lock().unwrap().push((session, prompt));
        }
    }

    /// Production boot with the handle already delivered, as the
    /// subscription's first item would.
    fn live_app() -> (App, FakeEngine) {
        let fake = FakeEngine {
            calls: std::sync::Arc::new(std::sync::Mutex::new(0)),
            spawns: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            turns: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        let mut app = App::live(manifest_mcp::ServerConfig::new("fake"));
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

    fn draft_project() -> manifest_domain::ProjectSummary {
        manifest_domain::ProjectSummary {
            id: manifest_domain::ProjectId::new("project-live"),
            name: String::from("live"),
            workspace: Some(String::from("/live/workspace")),
        }
    }

    fn live_session(id: &str) -> manifest_domain::SessionSummary {
        manifest_domain::SessionSummary {
            id: manifest_domain::SessionId::new(id),
            title: String::from(id),
            status: manifest_domain::SessionStatus::Idle,
            project: Some(manifest_domain::ProjectId::new("project-live")),
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
                projects: vec![live_project()],
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
                projects: vec![live_project()],
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
        let app = App::live(manifest_mcp::ServerConfig::new("fake"));
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
        assert!(!app.composer.send_enabled);
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
            vec![(workspace, String::from("start me"))]
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

    /// Loads one project carrying a workspace, for draft tests.
    fn loaded_with_workspace(app: &mut App) {
        deliver(
            app,
            engine::Input::event(manifest_domain::EngineEvent::SessionsLoaded {
                projects: vec![draft_project()],
                sessions: vec![live_session("s-1"), live_session("s-2")],
            }),
        );
    }

    /// Presses the project's "+" button through the root reducer.
    fn new_session_pressed(app: &mut App) {
        let _ = app.update(Message::Sessions(sessions::Message::NewSession(
            manifest_domain::ProjectId::new("project-live"),
        )));
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
                title: String::from("live · New session"),
            })
        );
        assert_eq!(
            app.sessions.expanded,
            Some(manifest_domain::ProjectId::new("project-live"))
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
            vec![(String::from("/live/workspace"), String::from("fresh idea"))]
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
        assert!(!app.composer.send_enabled);
        new_session_pressed(&mut app);
        assert!(app.new_session.is_some());
        assert!(app.composer.send_enabled);
    }

    #[test]
    fn new_session_without_workspace_expands_without_draft() {
        let (mut app, _) = live_app();
        deliver_connected(&mut app);
        loaded(&mut app, &["s-1"]);
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
            Some(manifest_domain::ProjectId::new("project-live"))
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
}
