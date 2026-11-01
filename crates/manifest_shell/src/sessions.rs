//! Session list: the left panel's state, messages, and view.
//!
//! Sessions render grouped under their projects in fixture order,
//! with project-less sessions under a static "No project" header.
//! Selection is owned here: [`State::selected`] is the single owner of
//! which session is selected, and the root reads it for the transcript
//! header. `update` changes the selection only for ids already in the
//! list; an unknown id is a no-op.

use iced::{Element, Length};
use manifest_domain::{ProjectId, ProjectSummary, SessionId, SessionStatus, SessionSummary};

use crate::kit;
use crate::theme::{
    FONT_SIZE_BODY, FONT_SIZE_HEADING, FONT_SIZE_SMALL, POPUP_WIDTH, SPACE_M, SPACE_S, SPACE_XS,
    TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};

/// Stand-in for the OS folder picker while the form is a fixture: the
/// directory a "choose folder" press fills in.
const FIXTURE_DIRECTORY: &str = "/home/sean/Code/billing-service";

/// Messages the session list can receive, enumerated before the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Select the session with this id, if it is in the list.
    Select(SessionId),
    /// Expand the project with this id, or collapse it if open.
    ExpandProject(ProjectId),
    /// Open the add-project popup with empty fields.
    OpenAddProject,
    /// Close the add-project popup, discarding its fields.
    CloseAddProject,
    /// A directory was typed (or picked) into the popup form.
    DirectoryChanged(String),
    /// A name was typed into the popup form.
    NameChanged(String),
    /// Fill the popup directory from the fixture picker stand-in.
    PickDirectory,
    /// Accept the popup form. Fixture form: registers nothing.
    SubmitAddProject,
}

/// One project's sessions in display order.
#[derive(Debug, Clone)]
pub struct ProjectGroup {
    /// The project heading this group.
    pub project: ProjectSummary,
    /// The project's sessions, in input order.
    pub sessions: Vec<SessionSummary>,
}

/// The add-project popup has two real situations, so it is an enum,
/// not an option of strings (UI contract §1). Fixture form: nothing
/// registers or persists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AddProject {
    /// No popup is showing.
    #[default]
    Closed,
    /// The popup is showing with its form fields.
    Open {
        /// The typed or picked directory path.
        directory: String,
        /// The project name; autofilled from the directory until edited.
        name: String,
        /// Whether the name was typed by hand (stops autofill).
        name_edited: bool,
    },
}

/// Session-list state: grouped rows plus selection and popup.
#[derive(Debug, Clone)]
pub struct State {
    /// One group per project, in the given project order.
    pub groups: Vec<ProjectGroup>,
    /// Sessions with no project (or an unknown one), in input order.
    pub unprojected: Vec<SessionSummary>,
    /// The selected session. Always the id of a listed session when
    /// built from the fixture; see [`State::from_projects`].
    pub selected: SessionId,
    /// The expanded project, if any.
    pub expanded: Option<ProjectId>,
    /// The add-project popup situation.
    pub add_project: AddProject,
}

impl State {
    /// Builds list state from domain rows: one group per project in
    /// the given order, each session placed into its project's group
    /// preserving input order. Sessions with no project, or one the
    /// project list does not name, land in [`State::unprojected`].
    /// The default selection is the first session in display order
    /// (first group's first session, else the first unprojected one);
    /// an empty input falls back to a blank placeholder id so boot
    /// stays total (no `unwrap` / panic paths). The default expanded
    /// project is the selected session's project.
    pub fn from_projects(projects: Vec<ProjectSummary>, sessions: Vec<SessionSummary>) -> Self {
        let mut groups: Vec<ProjectGroup> = projects
            .into_iter()
            .map(|project| ProjectGroup {
                project,
                sessions: Vec::new(),
            })
            .collect();
        let mut unprojected = Vec::new();
        for session in sessions {
            let placed = match &session.project {
                Some(id) => groups.iter_mut().find(|group| group.project.id == *id),
                None => None,
            };
            match placed {
                Some(group) => group.sessions.push(session),
                None => unprojected.push(session),
            }
        }
        let selected = groups
            .iter()
            .flat_map(|group| group.sessions.iter())
            .chain(unprojected.iter())
            .next()
            .map(|summary| summary.id.clone())
            .unwrap_or_else(|| SessionId::new(String::new()));
        let expanded = State::find_in(&groups, &unprojected, &selected)
            .and_then(|summary| summary.project.clone());
        State {
            groups,
            unprojected,
            selected,
            expanded,
            add_project: AddProject::Closed,
        }
    }

    /// Replaces the list with fresh domain rows, preserving UI state.
    ///
    /// Groups rebuild exactly as [`State::from_projects`] does. The
    /// current selection survives when its id still exists in the new
    /// data; otherwise the selection falls back to `from_projects`'
    /// default (first in display order, placeholder when empty). The
    /// expanded project becomes the (possibly preserved) selection's
    /// project, and the add-project popup state carries over untouched.
    pub fn replace(&mut self, projects: Vec<ProjectSummary>, sessions: Vec<SessionSummary>) {
        let mut fresh = State::from_projects(projects, sessions);
        if fresh.find(&self.selected).is_some() {
            let selected = self.selected.clone();
            let expanded = fresh
                .find(&selected)
                .and_then(|summary| summary.project.clone());
            fresh.selected = selected;
            fresh.expanded = expanded;
        }
        fresh.add_project = std::mem::take(&mut self.add_project);
        *self = fresh;
    }

    /// Finds the row with this id across groups, then unprojected rows.
    fn find_in<'a>(
        groups: &'a [ProjectGroup],
        unprojected: &'a [SessionSummary],
        id: &SessionId,
    ) -> Option<&'a SessionSummary> {
        groups
            .iter()
            .flat_map(|group| group.sessions.iter())
            .chain(unprojected.iter())
            .find(|summary| summary.id == *id)
    }

    /// Finds the row with this id in this state.
    fn find(&self, id: &SessionId) -> Option<&SessionSummary> {
        Self::find_in(&self.groups, &self.unprojected, id)
    }

    /// Shared directory rule for typed and picked directories: store
    /// the path, then autofill the name from its last segment unless
    /// the name was typed by hand and still holds text.
    fn set_directory(&mut self, directory: String) {
        if let AddProject::Open {
            directory: current,
            name,
            name_edited,
        } = &mut self.add_project
        {
            *current = directory;
            if !*name_edited || name.is_empty() {
                *name = directory_name(current).to_owned();
            }
        }
    }

    /// Folds a message into the list state.
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Select(id) => {
                let project = self.find(&id).map(|summary| summary.project.clone());
                if let Some(project) = project {
                    self.selected = id;
                    if project.is_some() {
                        self.expanded = project;
                    }
                }
            }
            Message::ExpandProject(id) => {
                let known = self.groups.iter().any(|group| group.project.id == id);
                if known {
                    self.expanded = match &self.expanded {
                        Some(current) if *current == id => None,
                        _ => Some(id),
                    };
                }
            }
            Message::OpenAddProject => {
                self.add_project = AddProject::Open {
                    directory: String::new(),
                    name: String::new(),
                    name_edited: false,
                };
            }
            Message::CloseAddProject | Message::SubmitAddProject => {
                self.add_project = AddProject::Closed;
            }
            Message::DirectoryChanged(directory) => self.set_directory(directory),
            Message::PickDirectory => self.set_directory(String::from(FIXTURE_DIRECTORY)),
            Message::NameChanged(name) => {
                if let AddProject::Open {
                    name: current,
                    name_edited,
                    ..
                } = &mut self.add_project
                {
                    *current = name;
                    *name_edited = true;
                }
            }
        }
    }

    /// The selected row, if the selection names a listed session.
    pub fn selected_summary(&self) -> Option<&SessionSummary> {
        self.find(&self.selected)
    }

    /// Renders the panel: the add row, one collapsible section per
    /// project, then the always-visible "No project" rows.
    pub fn view(&self) -> Element<'_, Message> {
        let mut list =
            iced::widget::column![kit::action_row("+ New Project", Message::OpenAddProject),]
                .spacing(SPACE_S)
                .width(Length::Fill);
        for group in &self.groups {
            list = list.push(kit::section_header(
                group.project.name.as_str(),
                Message::ExpandProject(group.project.id.clone()),
            ));
            if self.expanded.as_ref() == Some(&group.project.id) {
                list = list.push(indented(session_rows(&group.sessions, &self.selected)));
            }
        }
        if !self.unprojected.is_empty() {
            list = list.push(no_project_header());
            list = list.push(indented(session_rows(&self.unprojected, &self.selected)));
        }
        iced::widget::scrollable(iced::widget::container(list).width(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Renders the add-project popup, or [`None`] while it is closed.
    pub fn add_project_view(&self) -> Option<Element<'_, Message>> {
        let AddProject::Open {
            directory, name, ..
        } = &self.add_project
        else {
            return None;
        };
        let submittable = !directory.trim().is_empty() && !name.trim().is_empty();
        let on_press = if submittable {
            Some(Message::SubmitAddProject)
        } else {
            None
        };
        let form = iced::widget::column![
            iced::widget::text("New project")
                .size(FONT_SIZE_HEADING)
                .color(TEXT_PRIMARY),
            iced::widget::text("Directory")
                .size(FONT_SIZE_SMALL)
                .color(TEXT_SECONDARY),
            iced::widget::row![
                iced::widget::container(kit::text_field(
                    "Choose a directory...",
                    directory,
                    Message::DirectoryChanged,
                ))
                .width(Length::Fill),
                kit::pill_button(
                    "Choose folder...",
                    kit::ButtonKind::Ghost,
                    Some(Message::PickDirectory),
                ),
            ]
            .spacing(SPACE_S),
            iced::widget::text("Name")
                .size(FONT_SIZE_SMALL)
                .color(TEXT_SECONDARY),
            kit::text_field("Project name", name, Message::NameChanged),
            iced::widget::row![
                iced::widget::container(iced::widget::text("").size(FONT_SIZE_BODY),)
                    .width(Length::Fill),
                kit::pill_button(
                    "Cancel",
                    kit::ButtonKind::Ghost,
                    Some(Message::CloseAddProject),
                ),
                kit::pill_button("Add project", kit::ButtonKind::Primary, on_press),
            ]
            .spacing(SPACE_S),
        ]
        .spacing(SPACE_S)
        .width(Length::Fill);
        Some(
            iced::widget::container(kit::panel(form))
                .width(Length::Fixed(POPUP_WIDTH))
                .into(),
        )
    }
}

/// One column of session rows, spaced within the group.
fn session_rows<'a>(
    sessions: &'a [SessionSummary],
    selected: &'a SessionId,
) -> Element<'a, Message> {
    let mut rows = iced::widget::column![]
        .spacing(SPACE_XS)
        .width(Length::Fill);
    for summary in sessions {
        rows = rows.push(kit::list_row(
            &summary.title,
            status_label(&summary.status),
            &summary.relative_time,
            summary.id == *selected,
            Message::Select(summary.id.clone()),
        ));
    }
    rows.into()
}

/// Indents a row column to sit under its section header.
fn indented(content: Element<'_, Message>) -> Element<'_, Message> {
    iced::widget::container(content)
        .padding(iced::Padding {
            left: SPACE_M,
            ..iced::Padding::default()
        })
        .into()
}

/// The static header above project-less sessions, padded to match
/// [`kit::section_header`] without being pressable.
fn no_project_header() -> Element<'static, Message> {
    iced::widget::container(
        iced::widget::text("No project")
            .size(FONT_SIZE_SMALL)
            .color(TEXT_TERTIARY),
    )
    .padding([SPACE_XS, SPACE_S])
    .into()
}

/// The last non-empty slash-separated segment of a path: trailing
/// slashes are ignored, and empty or all-slash input yields "".
fn directory_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    let mut start = 0;
    for (index, byte) in trimmed.as_bytes().iter().enumerate() {
        if *byte == b'/' {
            start = index + 1;
        }
    }
    &trimmed[start..]
}

/// Plain status subtitle for a session row. The view performs no
/// formatting: this match returns a static label, keeping string
/// building out of the view (UI contract §2).
pub fn status_label(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Idle => "Idle",
        SessionStatus::Running => "Running",
        SessionStatus::AwaitingApproval => "Awaiting approval",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use manifest_domain::fixture;

    fn state() -> State {
        let got = fixture();
        State::from_projects(got.projects, got.sessions)
    }

    #[test]
    fn groups_follow_fixture_order_and_sizes() {
        let got = state();
        assert_eq!(got.groups.len(), 2);
        assert_eq!(got.groups[0].project.name, "checkout-api");
        assert_eq!(got.groups[0].sessions.len(), 2);
        assert_eq!(got.groups[1].project.name, "storefront-web");
        assert_eq!(got.groups[1].sessions.len(), 2);
        assert_eq!(got.unprojected.len(), 1);
    }

    #[test]
    fn boot_selects_first_session_with_its_project_expanded() {
        let got = state();
        let fixture = fixture();
        assert_eq!(got.selected, fixture.sessions[0].id);
        assert_eq!(got.expanded, Some(ProjectId::new("project-checkout-api")));
    }

    #[test]
    fn select_in_collapsed_project_expands_it() {
        let mut got = state();
        got.update(Message::ExpandProject(ProjectId::new(
            "project-storefront-web",
        )));
        got.update(Message::ExpandProject(ProjectId::new(
            "project-storefront-web",
        )));
        assert_eq!(got.expanded, None);
        let target = got.groups[1].sessions[0].id.clone();
        got.update(Message::Select(target.clone()));
        assert_eq!(got.selected, target);
        assert_eq!(got.expanded, Some(ProjectId::new("project-storefront-web")));
    }

    #[test]
    fn select_unprojected_leaves_expanded_unchanged() {
        let mut got = state();
        let before = got.expanded.clone();
        let target = got.unprojected[0].id.clone();
        got.update(Message::Select(target.clone()));
        assert_eq!(got.selected, target);
        assert_eq!(got.expanded, before);
    }

    #[test]
    fn select_unknown_id_changes_neither_selection_nor_expansion() {
        let mut got = state();
        let selected = got.selected.clone();
        let expanded = got.expanded.clone();
        got.update(Message::Select(SessionId::new("no-such-session")));
        assert_eq!(got.selected, selected);
        assert_eq!(got.expanded, expanded);
    }

    #[test]
    fn expand_project_switches_and_retoggles_to_none() {
        let mut got = state();
        got.update(Message::ExpandProject(ProjectId::new(
            "project-storefront-web",
        )));
        assert_eq!(got.expanded, Some(ProjectId::new("project-storefront-web")));
        got.update(Message::ExpandProject(ProjectId::new(
            "project-storefront-web",
        )));
        assert_eq!(got.expanded, None);
    }

    #[test]
    fn expand_unknown_project_is_noop() {
        let mut got = state();
        let before = got.expanded.clone();
        got.update(Message::ExpandProject(ProjectId::new("no-such-project")));
        assert_eq!(got.expanded, before);
    }

    #[test]
    fn open_popup_resets_fields() {
        let mut got = state();
        got.update(Message::DirectoryChanged(String::from("/x/y/old")));
        got.update(Message::OpenAddProject);
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::new(),
                name: String::new(),
                name_edited: false,
            }
        );
    }

    #[test]
    fn directory_autofills_name_from_last_segment() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::DirectoryChanged(String::from(
            "/x/y/billing-service",
        )));
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::from("/x/y/billing-service"),
                name: String::from("billing-service"),
                name_edited: false,
            }
        );
    }

    #[test]
    fn directory_with_trailing_slash_autofills_trimmed_name() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::DirectoryChanged(String::from(
            "/x/y/billing-service/",
        )));
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::from("/x/y/billing-service/"),
                name: String::from("billing-service"),
                name_edited: false,
            }
        );
    }

    #[test]
    fn manual_name_survives_later_directory() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::NameChanged(String::from("custom")));
        got.update(Message::DirectoryChanged(String::from("/x/y/other")));
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::from("/x/y/other"),
                name: String::from("custom"),
                name_edited: true,
            }
        );
    }

    #[test]
    fn cleared_name_refills_from_directory() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::NameChanged(String::from("custom")));
        got.update(Message::NameChanged(String::new()));
        got.update(Message::DirectoryChanged(String::from("/x/y/other")));
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::from("/x/y/other"),
                name: String::from("other"),
                name_edited: true,
            }
        );
    }

    #[test]
    fn pick_directory_fills_fixture_path_and_name() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::PickDirectory);
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::from(FIXTURE_DIRECTORY),
                name: String::from("billing-service"),
                name_edited: false,
            }
        );
    }

    #[test]
    fn close_and_submit_both_close_the_popup() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::CloseAddProject);
        assert_eq!(got.add_project, AddProject::Closed);
        got.update(Message::OpenAddProject);
        got.update(Message::SubmitAddProject);
        assert_eq!(got.add_project, AddProject::Closed);
    }

    #[test]
    fn form_messages_are_noops_while_closed() {
        let mut got = state();
        got.update(Message::DirectoryChanged(String::from("/x/y/z")));
        got.update(Message::NameChanged(String::from("z")));
        got.update(Message::PickDirectory);
        assert_eq!(got.add_project, AddProject::Closed);
    }

    #[test]
    fn directory_name_edge_cases() {
        assert_eq!(directory_name("name"), "name");
        assert_eq!(directory_name(""), "");
        assert_eq!(directory_name("/"), "");
        assert_eq!(directory_name("a/b/c"), "c");
    }

    fn project(id: &str, name: &str) -> ProjectSummary {
        ProjectSummary {
            id: ProjectId::new(id),
            name: String::from(name),
        }
    }

    fn row(id: &str, project: Option<&str>) -> SessionSummary {
        SessionSummary {
            id: SessionId::new(id),
            title: String::from(id),
            status: SessionStatus::Idle,
            project: project.map(ProjectId::new),
            relative_time: String::from("now"),
            stream_dropped_deltas: 0,
        }
    }

    #[test]
    fn replace_preserves_selection_and_expansion_when_session_remains() {
        let mut got = State::from_projects(
            vec![project("p-a", "a"), project("p-b", "b")],
            vec![row("s-1", Some("p-a")), row("s-2", Some("p-b"))],
        );
        got.update(Message::Select(SessionId::new("s-2")));
        assert_eq!(got.expanded, Some(ProjectId::new("p-b")));
        got.replace(
            vec![project("p-b", "b"), project("p-a", "a")],
            vec![row("s-2", Some("p-b")), row("s-3", Some("p-a"))],
        );
        assert_eq!(got.selected, SessionId::new("s-2"));
        assert_eq!(got.expanded, Some(ProjectId::new("p-b")));
        assert_eq!(got.groups.len(), 2);
    }

    #[test]
    fn replace_falls_back_to_first_when_selection_gone() {
        let mut got = State::from_projects(
            vec![project("p-a", "a")],
            vec![row("s-1", Some("p-a")), row("s-2", Some("p-a"))],
        );
        got.replace(vec![project("p-b", "b")], vec![row("s-9", Some("p-b"))]);
        assert_eq!(got.selected, SessionId::new("s-9"));
        assert_eq!(got.expanded, Some(ProjectId::new("p-b")));
    }

    #[test]
    fn replace_with_empty_uses_placeholder_selection() {
        let mut got = state();
        got.replace(Vec::new(), Vec::new());
        assert!(got.groups.is_empty());
        assert!(got.unprojected.is_empty());
        assert!(got.selected_summary().is_none());
        assert_eq!(got.expanded, None);
    }

    #[test]
    fn replace_preserves_open_add_project_popup() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::DirectoryChanged(String::from(
            "/x/y/billing-service",
        )));
        got.replace(Vec::new(), Vec::new());
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::from("/x/y/billing-service"),
                name: String::from("billing-service"),
                name_edited: false,
            }
        );
    }
}
