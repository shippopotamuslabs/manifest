//! Root shell: the four-region layout, the root reducer, and boot.
//!
//! The top bar is a view function on the root state, not a module with
//! invented state (plan chunk 4): it owns nothing, so it owns no file.
//! Child features own their state; this module holds the child states,
//! routes messages down, and maps child messages up (UI contract §7).

use iced::{Element, Length, Task};
use manifest_domain::{Connection, EngineEvent, Fixture, StreamEvent, TodoItem};

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

/// Root state: the child states plus the drawer.
#[derive(Debug, Clone)]
pub struct App {
    /// Left-panel session list; owns the selection.
    pub sessions: sessions::State,
    /// Selected-session transcript; only scroll state changes in M1.
    pub transcript: transcript::State,
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
    /// The latest session-load failure or diagnostic message, shown in
    /// the side panel until a retry clears it.
    pub engine_notice: Option<String>,
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
        App {
            sessions: sessions::State::from_projects(fixture.projects, fixture.sessions),
            transcript: transcript::State::from_items(fixture.transcript),
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
            engine_notice: None,
        }
    }

    /// Production boot: connects to the real server. Starts from the
    /// fixture (transcript and todos stay fixture until later slices own
    /// them) with an empty session list, then connects on the first
    /// subscription tick; sessions arrive through the engine stream.
    pub fn live(config: manifest_mcp::ServerConfig) -> Self {
        let mut app = Self::from_fixture(manifest_domain::fixture());
        app.sessions = sessions::State::from_projects(Vec::new(), Vec::new());
        app.connection = Connection::Connecting;
        app.engine_config = Some(config);
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
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Sessions(child) => {
                self.sessions.update(child);
                Task::none()
            }
            Message::Composer(child) => {
                self.composer.update(child);
                Task::none()
            }
            Message::Transcript(child) => {
                self.transcript.update(child);
                Task::none()
            }
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
                match input {
                    engine::Input::Ready(handle) => {
                        self.engine = Some(handle);
                    }
                    engine::Input::Event(EngineEvent::ConnectionChanged(next)) => {
                        self.connection = next;
                        if self.connection == Connection::Connected {
                            if let Some(engine) = &self.engine {
                                engine.list_sessions();
                            }
                        } else if matches!(self.connection, Connection::Failed(_)) {
                            self.engine = None;
                        }
                    }
                    engine::Input::Event(EngineEvent::SessionsLoaded { projects, sessions }) => {
                        self.sessions.replace(projects, sessions);
                    }
                    engine::Input::Event(EngineEvent::SessionsLoadFailed(message)) => {
                        self.engine_notice = Some(message);
                    }
                    engine::Input::Event(EngineEvent::Diagnostic(message)) => {
                        self.engine_notice = Some(message);
                    }
                    engine::Input::Event(EngineEvent::Stream(StreamEvent::Pending { .. })) => {
                        if self.connection == Connection::Connected
                            && let Some(engine) = &self.engine
                        {
                            engine.list_sessions();
                        }
                    }
                    // M2 owns no transcript: other stream events are
                    // consumed as no-ops until M3 renders them.
                    engine::Input::Event(EngineEvent::Stream(_)) => {}
                }
                Task::none()
            }
            Message::Reconnect => {
                if self.engine_config.is_some() {
                    // Dropping the handle shuts the old worker down; the
                    // subscription restarts under the new attempt.
                    self.engine = None;
                    self.connection = Connection::Connecting;
                    self.engine_notice = None;
                    self.connection_attempt += 1;
                }
                Task::none()
            }
        }
    }

    /// Drag subscription plus the engine stream: idle produces nothing
    /// for the drag; while a divider drag is in flight, cursor moves
    /// resize the panel and releasing the left button ends the drag.
    /// The engine stream runs only while connecting or connected with a
    /// config; a finished stream whose recipe is still returned gets
    /// restarted, which would respawn against a dead server in a loop,
    /// so failure and fixture mode return no engine recipe until the
    /// user retries.
    pub fn subscription(&self) -> iced::Subscription<Message> {
        let drag = match self.panel_drag {
            PanelDrag::Idle => iced::Subscription::none(),
            PanelDrag::Dragging { .. } => iced::event::listen_with(drag_event),
        };
        let engine = match (&self.engine_config, &self.connection) {
            (Some(config), Connection::Connecting | Connection::Connected) => {
                let attempt = self.connection_attempt;
                engine::subscription(attempt, config)
                    .map(move |(attempt, input)| Message::Engine { attempt, input })
            }
            _ => iced::Subscription::none(),
        };
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
        let mut panel = iced::widget::column![];
        if let Some(notice) = side_notice(self) {
            panel = panel.push(
                iced::widget::text(notice)
                    .size(FONT_SIZE_SMALL)
                    .color(TEXT_TERTIARY),
            );
        }
        let panel: Element<'_, Message> = panel
            .push(self.sessions.view().map(Message::Sessions))
            .spacing(SPACE_S)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
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

/// Static label for the connection state. The view performs no
/// formatting: this match returns a static label, keeping string
/// building out of the view (UI contract §2).
fn connection_label(connection: &Connection) -> &'static str {
    match connection {
        Connection::Disconnected => "Offline",
        Connection::Connecting => "Connecting...",
        Connection::Connected => "Connected",
        Connection::Failed(_) => "Connection failed",
    }
}

/// The side-panel notice line, if there is something true to show, in
/// priority order: the failure message, then the latest load or
/// diagnostic notice, then the connecting line. Borrows stored text so
/// the view formats nothing (UI contract §2).
fn side_notice(state: &App) -> Option<&str> {
    if let Connection::Failed(message) = &state.connection {
        Some(message.as_str())
    } else if let Some(notice) = state.engine_notice.as_deref() {
        Some(notice)
    } else if state.engine_config.is_some() && state.connection == Connection::Connecting {
        Some("Connecting to the MCP server...")
    } else {
        None
    }
}

/// The title bar: the app title in the fixed panel share, then the
/// selected session title truly centered in the fill share with the
/// window controls right-aligned. A plain function on the root state
/// — no state of its own, no module.
fn topbar(state: &App) -> Element<'_, Message> {
    let selected_title: &str = state
        .sessions
        .selected_summary()
        .map(|summary| summary.title.as_str())
        .unwrap_or("No session");
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
                controls = controls.push(
                    iced::widget::text(connection_label(&state.connection))
                        .size(FONT_SIZE_SMALL)
                        .color(TEXT_TERTIARY),
                );
                if matches!(state.connection, Connection::Failed(_)) {
                    controls = controls.push(kit::pill_button(
                        "Retry",
                        kit::ButtonKind::Ghost,
                        Some(Message::Reconnect),
                    ));
                }
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
    /// would serve them, without spawning anything.
    struct FakeEngine {
        calls: std::sync::Mutex<usize>,
    }

    impl manifest_domain::SessionEngine for FakeEngine {
        fn list_sessions(&self) {
            *self.calls.lock().unwrap() += 1;
        }
    }

    /// Production boot with the handle already delivered, as the
    /// subscription's first item would.
    fn live_app() -> (App, std::sync::Arc<FakeEngine>) {
        let fake = std::sync::Arc::new(FakeEngine {
            calls: std::sync::Mutex::new(0),
        });
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
            engine::Input::Event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Connected,
            )),
        );
    }

    fn live_project() -> manifest_domain::ProjectSummary {
        manifest_domain::ProjectSummary {
            id: manifest_domain::ProjectId::new("project-live"),
            name: String::from("live"),
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
            engine::Input::Event(manifest_domain::EngineEvent::SessionsLoaded {
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
            engine::Input::Event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Failed(String::from("boom")),
            )),
        );
        assert_eq!(
            app.connection,
            manifest_domain::Connection::Failed(String::from("boom"))
        );
        assert!(app.engine.is_none());
    }

    #[test]
    fn reconnect_bumps_attempt_and_resets_state() {
        let (mut app, _) = live_app();
        deliver(
            &mut app,
            engine::Input::Event(manifest_domain::EngineEvent::ConnectionChanged(
                manifest_domain::Connection::Failed(String::from("boom")),
            )),
        );
        deliver(
            &mut app,
            engine::Input::Event(manifest_domain::EngineEvent::SessionsLoadFailed(
                String::from("load broke"),
            )),
        );
        let before = app.connection_attempt;
        let _ = app.update(Message::Reconnect);
        assert_eq!(app.connection_attempt, before + 1);
        assert_eq!(app.connection, manifest_domain::Connection::Connecting);
        assert!(app.engine_notice.is_none());
        assert!(app.engine.is_none());
    }

    #[test]
    fn stale_attempt_inputs_are_ignored() {
        let (mut app, _) = live_app();
        let stale = app.connection_attempt;
        let _ = app.update(Message::Reconnect);
        let _ = app.update(Message::Engine {
            attempt: stale,
            input: engine::Input::Event(manifest_domain::EngineEvent::ConnectionChanged(
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
            engine::Input::Event(manifest_domain::EngineEvent::Stream(
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
    fn load_failed_and_diagnostic_set_notice() {
        let (mut app, _) = live_app();
        deliver(
            &mut app,
            engine::Input::Event(manifest_domain::EngineEvent::SessionsLoadFailed(
                String::from("x"),
            )),
        );
        assert_eq!(app.engine_notice, Some(String::from("x")));
        deliver(
            &mut app,
            engine::Input::Event(manifest_domain::EngineEvent::Diagnostic(String::from("y"))),
        );
        assert_eq!(app.engine_notice, Some(String::from("y")));
    }

    #[test]
    fn fixture_boot_stays_offline() {
        let app = App::boot();
        assert_eq!(app.connection, manifest_domain::Connection::Disconnected);
        assert!(app.engine_config.is_none());
        assert!(app.engine.is_none());
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
}
