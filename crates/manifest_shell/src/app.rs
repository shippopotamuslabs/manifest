//! Root shell: the four-region layout, the root reducer, and boot.
//!
//! The top bar is a view function on the root state, not a module with
//! invented state (plan chunk 4): it owns nothing, so it owns no file.
//! Child features own their state; this module holds the child states,
//! routes messages down, and maps child messages up (UI contract §7).

use iced::{Element, Length, Task};
use manifest_domain::{Fixture, TodoItem};

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
        }
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
        }
    }

    /// Drag subscription: idle produces nothing; while a divider drag
    /// is in flight, cursor moves resize the panel and releasing the
    /// left button ends the drag. All other events map to nothing (the
    /// status and window arguments are ignored).
    pub fn subscription(&self) -> iced::Subscription<Message> {
        match self.panel_drag {
            PanelDrag::Idle => iced::Subscription::none(),
            PanelDrag::Dragging { .. } => iced::event::listen_with(drag_event),
        }
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
        let base: Element<'_, Message> = iced::widget::column![
            topbar(self),
            iced::widget::row![
                iced::widget::container(kit::side_panel(
                    self.sessions.view().map(Message::Sessions)
                ))
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
        iced::widget::container(
            iced::widget::row![
                kit::icon_button(
                    "−",
                    kit::IconButtonKind::Subtle,
                    Some(Message::MinimizeWindow),
                ),
                kit::icon_button(
                    "□",
                    kit::IconButtonKind::Subtle,
                    Some(Message::ToggleMaximizeWindow),
                ),
                kit::icon_button("✕", kit::IconButtonKind::Subtle, Some(Message::CloseWindow)),
            ]
            .spacing(SPACE_S)
            .align_y(iced::alignment::Vertical::Center),
        )
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
