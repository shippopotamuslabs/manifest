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
    FONT_SIZE_BODY, FONT_SIZE_SMALL, FONT_SIZE_TITLE, SPACE_M, SPACE_S, TEXT_PRIMARY,
    TEXT_SECONDARY, TEXT_TERTIARY,
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

/// Root messages, enumerated before the view (UI contract §1).
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// A session-list message, routed to [`sessions::State`].
    Sessions(sessions::Message),
    /// A composer message, routed to [`composer::State`].
    Composer(composer::Message),
    /// Toggles the TODO drawer between peek strip and open.
    ToggleTodoDrawer,
}

impl From<sessions::Message> for Message {
    fn from(message: sessions::Message) -> Self {
        Message::Sessions(message)
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
    /// Selected-session transcript (read-only in M1).
    pub transcript: transcript::State,
    /// Draft input cluster.
    pub composer: composer::State,
    /// Checklist shown in the drawer, from the fixture.
    pub todos: Vec<TodoItem>,
    /// Whether the TODO drawer is a peek strip or open.
    pub todo_drawer: TodoDrawer,
    /// Precomputed "<done> of <total> done" line for the drawer bar.
    pub todo_summary: String,
    /// Label of the first unfinished todo, for the drawer bar.
    pub current_todo: Option<String>,
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
        }
    }

    /// Boots the shell from [`manifest_domain::fixture()`].
    pub fn boot() -> Self {
        Self::from_fixture(manifest_domain::fixture())
    }

    /// Root reducer. M1 performs no asynchronous work, so every branch
    /// returns [`Task::none()`]; no `Subscription` exists either.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Sessions(child) => self.sessions.update(child),
            Message::Composer(child) => self.composer.update(child),
            Message::ToggleTodoDrawer => {
                self.todo_drawer = match self.todo_drawer {
                    TodoDrawer::Peek => TodoDrawer::Open,
                    TodoDrawer::Open => TodoDrawer::Peek,
                };
            }
        }
        Task::none()
    }

    /// Renders the four regions: top bar, then a row splitting the
    /// session panel (`FillPortion(1)`) from the main column
    /// (`FillPortion(4)`) of transcript, drawer (peek or open),
    /// composer. The drawer always renders while todos exist.
    pub fn view(&self) -> Element<'_, Message> {
        let mut main = iced::widget::column![
            iced::widget::container(self.transcript.view().map(|never| match never {}),)
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .spacing(SPACE_S)
        .width(Length::FillPortion(4))
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
                iced::widget::container(self.sessions.view().map(Message::Sessions))
                    .width(Length::FillPortion(1))
                    .height(Length::Fill),
                iced::widget::container(main)
                    .width(Length::FillPortion(4))
                    .height(Length::Fill),
            ]
            .spacing(SPACE_M)
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .spacing(SPACE_S)
        .padding(SPACE_M)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        if let Some(popup) = self.sessions.add_project_view() {
            return iced::widget::stack![base, kit::scrim(popup.map(Message::Sessions))].into();
        }
        base
    }
}

impl Default for App {
    fn default() -> Self {
        Self::boot()
    }
}

/// The top bar: app title, selected session title, and the Todos chip.
/// A plain function on the root state — no state of its own, no module.
fn topbar(state: &App) -> Element<'_, Message> {
    let selected_title: &str = state
        .sessions
        .selected_summary()
        .map(|summary| summary.title.as_str())
        .unwrap_or("No session");
    iced::widget::row![
        iced::widget::text("Manifest")
            .size(FONT_SIZE_TITLE)
            .color(TEXT_PRIMARY),
        iced::widget::container(
            iced::widget::text(selected_title)
                .size(FONT_SIZE_BODY)
                .color(TEXT_SECONDARY),
        )
        .width(Length::Fill),
        kit::toggle_chip(
            "Todos",
            state.todo_drawer == TodoDrawer::Open,
            Message::ToggleTodoDrawer,
        ),
    ]
    .spacing(SPACE_M)
    .width(Length::Fill)
    .into()
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
}
