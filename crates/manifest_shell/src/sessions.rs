//! Session list: the left panel's state, messages, and view.
//!
//! Sessions render grouped under their projects in registry order,
//! with project-less sessions under a static "No project" header.
//!
//! The add-project popup is real in live mode and registers nothing in
//! fixture mode: the App owns the OS folder picker, validates the
//! form asynchronously, and persists the registry through
//! manifest_store, routing the outcome back down as child messages.
//! Selection is owned here: [`State::selected`] is the single owner of
//! which session is selected, and the root reads it for the transcript
//! header. `update` changes the selection only for ids already in the
//! list; an unknown id is a no-op.

use std::collections::{HashMap, HashSet};

use iced::{Element, Length};
use manifest_domain::{
    ProjectId, ProjectSummary, SessionId, SessionStatus, SessionSummary, compose_session_title,
};

use crate::kit;
use crate::theme::{
    DANGER, FONT_SIZE_BODY, FONT_SIZE_HEADING, FONT_SIZE_PANEL_SMALL, FONT_SIZE_SMALL, POPUP_WIDTH,
    SPACE_M, SPACE_S, SPACE_XS, SPACE_XXS, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};

/// Messages the session list can receive, enumerated before the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Select the session with this id, if it is in the list.
    Select(SessionId),
    /// Expand the project with this id, or collapse it if open.
    ExpandProject(ProjectId),
    /// Start composing a new session in this project's workspace.
    /// `State::update` handles this locally by expanding the project;
    /// the App enters draft mode on top, mirroring the composer
    /// `SendPressed` split (a local no-op there, acted on by the App).
    NewSession(ProjectId),
    /// Open the add-project popup with empty fields.
    OpenAddProject,
    /// Close the add-project popup, discarding its fields.
    CloseAddProject,
    /// A directory was typed (or picked) into the popup form.
    DirectoryChanged(String),
    /// A name was typed into the popup form.
    NameChanged(String),
    /// The App owns the OS folder picker: this is a child-level no-op
    /// (mirroring the composer `SendPressed` split). The App intercepts
    /// it and routes the picked path back as [`Message::DirectoryChanged`].
    PickDirectory,
    /// The App validates the form asynchronously: this is a child-level
    /// no-op (same `SendPressed` split). The App intercepts it and
    /// routes the outcome back as [`Message::AddProjectFailed`] or by
    /// closing the popup.
    SubmitAddProject,
    /// Record a validation failure on the open popup; no-op while closed.
    AddProjectFailed(String),
    /// Remove the project with this id: its sessions move (order
    /// preserved) onto the end of unprojected, the id leaves the
    /// expanded set, and the selection is unchanged. Unknown id is a no-op.
    RemoveProject(ProjectId),
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
/// not an option of strings (UI contract §1). The popup is real in
/// live mode; in fixture mode it opens and closes but registers
/// nothing.
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
        /// Precomputed validation error shown under the form, if any.
        error: Option<String>,
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
    /// The expanded projects.
    pub expanded: HashSet<ProjectId>,
    /// The add-project popup situation.
    pub add_project: AddProject,
    /// Precomputed unsettled pending counts by session, written by
    /// the App on every pending load (UI contract §2).
    pub pending_counts: HashMap<SessionId, u64>,
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
        let expanded: HashSet<ProjectId> = State::find_in(&groups, &unprojected, &selected)
            .and_then(|summary| summary.project.clone())
            .into_iter()
            .collect();
        State {
            groups,
            unprojected,
            selected,
            expanded,
            add_project: AddProject::Closed,
            pending_counts: HashMap::new(),
        }
    }

    /// Retitle one listed session from a new bare name (MFST-34): the
    /// stored title recomposes with the row's group label — its
    /// project name, or its workspace name when unprojected. Unknown
    /// ids are a no-op; the App-level name overlay still records the
    /// name for the next list refresh.
    pub fn retitle(&mut self, id: &SessionId, name: &str) {
        for group in &mut self.groups {
            if group.sessions.iter().any(|session| &session.id == id) {
                let label = group.project.name.clone();
                let session = group
                    .sessions
                    .iter_mut()
                    .find(|session| &session.id == id)
                    .expect("presence checked above");
                session.title = compose_session_title(Some(&label), &session.workspace, name);
                return;
            }
        }
        if let Some(session) = self.unprojected.iter_mut().find(|session| &session.id == id) {
            session.title = compose_session_title(None, &session.workspace, name);
        }
    }

    /// Replace the precomputed pending counts (App is the writer).
    pub fn set_pending_counts(&mut self, counts: HashMap<SessionId, u64>) {
        self.pending_counts = counts;
    }

    /// Pending count for one session's badge; 0 hides it.
    #[allow(dead_code)]
    pub fn pending_count(&self, session: &SessionId) -> u64 {
        self.pending_counts.get(session).copied().unwrap_or(0)
    }

    /// Replaces the list with fresh domain rows, preserving UI state.
    ///
    /// Groups rebuild exactly as [`State::from_projects`] does. The
    /// current selection survives when its id still exists in the new
    /// data; otherwise the selection falls back to `from_projects`'
    /// default (first in display order, placeholder when empty). When
    /// the selection survives, the expanded set is the previous set
    /// intersected with the fresh group ids (the selected project is
    /// not force-expanded); when it falls back, the expanded set is
    /// the fresh default. The add-project popup state carries over
    /// untouched.
    pub fn replace(&mut self, projects: Vec<ProjectSummary>, sessions: Vec<SessionSummary>) {
        let previous_expanded = self.expanded.clone();
        let mut fresh = State::from_projects(projects, sessions);
        if fresh.find(&self.selected).is_some() {
            fresh.selected = self.selected.clone();
            fresh.expanded = previous_expanded
                .into_iter()
                .filter(|id| fresh.groups.iter().any(|group| &group.project.id == id))
                .collect();
        }
        fresh.add_project = std::mem::take(&mut self.add_project);
        fresh.pending_counts = std::mem::take(&mut self.pending_counts);
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
    /// the name was typed by hand and still holds text. Clears any
    /// validation error: the user is fixing the form.
    fn set_directory(&mut self, directory: String) {
        if let AddProject::Open {
            directory: current,
            name,
            name_edited,
            error,
        } = &mut self.add_project
        {
            *current = directory;
            if !*name_edited || name.is_empty() {
                *name = directory_name(current).to_owned();
            }
            *error = None;
        }
    }

    /// Removes the project with this id: its sessions move (order
    /// preserved) onto the end of unprojected, the id leaves the
    /// expanded set, and the selection is unchanged. Unknown id is a no-op.
    fn remove_project(&mut self, id: &ProjectId) {
        let Some(index) = self.groups.iter().position(|group| group.project.id == *id) else {
            return;
        };
        let group = self.groups.remove(index);
        self.unprojected.extend(group.sessions);
        self.expanded.remove(id);
    }

    /// Folds a message into the list state.
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Select(id) => {
                let project = self.find(&id).map(|summary| summary.project.clone());
                if let Some(project) = project {
                    self.selected = id;
                    if let Some(project_id) = project {
                        self.expanded.insert(project_id);
                    }
                }
            }
            Message::ExpandProject(id) => {
                let known = self.groups.iter().any(|group| group.project.id == id);
                if known && !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Message::NewSession(id) => {
                // Expansion only; the App enters draft mode on top.
                if self.groups.iter().any(|group| group.project.id == id) {
                    self.expanded.insert(id);
                }
            }
            Message::OpenAddProject => {
                self.add_project = AddProject::Open {
                    directory: String::new(),
                    name: String::new(),
                    name_edited: false,
                    error: None,
                };
            }
            Message::CloseAddProject => {
                self.add_project = AddProject::Closed;
            }
            // Owned by the App (OS picker / async validation): no-op here.
            Message::PickDirectory | Message::SubmitAddProject => {}
            Message::DirectoryChanged(directory) => self.set_directory(directory),
            Message::NameChanged(name) => {
                if let AddProject::Open {
                    name: current,
                    name_edited,
                    error,
                    ..
                } = &mut self.add_project
                {
                    *current = name;
                    *name_edited = true;
                    *error = None;
                }
            }
            Message::AddProjectFailed(error) => {
                if let AddProject::Open { error: current, .. } = &mut self.add_project {
                    *current = Some(error);
                }
            }
            Message::RemoveProject(id) => self.remove_project(&id),
        }
    }

    /// The selected row, if the selection names a listed session.
    pub fn selected_summary(&self) -> Option<&SessionSummary> {
        self.find(&self.selected)
    }

    /// Renders the panel: the add row, one collapsible section per
    /// project, then the always-visible "No project" rows.
    pub fn view(&self) -> Element<'_, Message> {
        let mut list = iced::widget::column![kit::action_row(
            "New Project",
            Some(lucide_icons::Icon::FolderPlus),
            Message::OpenAddProject,
        ),]
        .spacing(SPACE_S)
        .width(Length::Fill);
        if !self.groups.is_empty() {
            list = list.push(kit::section_header("Projects"));
        }
        for group in &self.groups {
            let expanded = self.expanded.contains(&group.project.id);
            list = list.push(kit::project_header(
                group.project.name.as_str(),
                expanded,
                Message::ExpandProject(group.project.id.clone()),
                Message::NewSession(group.project.id.clone()),
                Message::RemoveProject(group.project.id.clone()),
            ));
            if expanded {
                list = list.push(indented(session_rows(
                    &group.sessions,
                    &self.selected,
                    &self.pending_counts,
                )));
            }
        }
        if !self.unprojected.is_empty() {
            list = list.push(no_project_header());
            list = list.push(indented(session_rows(
                &self.unprojected,
                &self.selected,
                &self.pending_counts,
            )));
        }
        iced::widget::scrollable(iced::widget::container(list).width(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Renders the add-project popup, or [`None`] while it is closed.
    pub fn add_project_view(&self) -> Option<Element<'_, Message>> {
        let AddProject::Open {
            directory,
            name,
            error,
            ..
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
                kit::tooltip_icon_button(
                    lucide_icons::Icon::FolderOpen,
                    kit::IconButtonKind::Subtle,
                    "Choose folder...",
                    Some(Message::PickDirectory),
                ),
            ]
            .spacing(SPACE_S),
            iced::widget::text("Name")
                .size(FONT_SIZE_SMALL)
                .color(TEXT_SECONDARY),
            kit::text_field("Project name", name, Message::NameChanged),
            match error {
                Some(text) => iced::widget::text(text.as_str())
                    .size(FONT_SIZE_SMALL)
                    .color(DANGER),
                None => iced::widget::text("").size(FONT_SIZE_SMALL),
            },
            iced::widget::row![
                iced::widget::container(iced::widget::text("").size(FONT_SIZE_BODY),)
                    .width(Length::Fill),
                kit::pill_button(
                    "Cancel",
                    kit::ButtonKind::Ghost,
                    Some(Message::CloseAddProject),
                ),
                kit::icon_pill_button(
                    lucide_icons::Icon::FolderPlus,
                    "Add project",
                    kit::ButtonKind::Primary,
                    on_press,
                ),
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
    pending_counts: &'a HashMap<SessionId, u64>,
) -> Element<'a, Message> {
    let mut rows = iced::widget::column![]
        .spacing(SPACE_XXS)
        .width(Length::Fill);
    for summary in sessions {
        // The trailing meta is the relative time when the row has one,
        // else the status label; both are precomputed, never formatted
        // here (UI contract §2).
        let trailing = if summary.relative_time.is_empty() {
            status_label(&summary.status)
        } else {
            summary.relative_time.as_str()
        };
        rows = rows.push(kit::list_row_with_badge(
            &summary.title,
            trailing,
            pending_counts.get(&summary.id).copied().unwrap_or(0),
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
/// the project headers without being pressable. It carries no Plus icon button:
/// a new session needs a project workspace to spawn in.
fn no_project_header() -> Element<'static, Message> {
    iced::widget::container(
        iced::widget::text("No project")
            .size(FONT_SIZE_PANEL_SMALL)
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
        SessionStatus::Orphaned => "Orphaned",
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
    fn pending_counts_badge_reads_precomputed_map() {
        let mut got = state();
        assert_eq!(got.pending_count(&got.selected), 0);
        let mut counts = std::collections::HashMap::new();
        counts.insert(got.selected.clone(), 2_u64);
        got.set_pending_counts(counts);
        assert_eq!(got.pending_count(&got.selected), 2);
        // Counts survive a list replace (App rewrites them on load).
        let fixture = fixture();
        got.replace(fixture.projects, fixture.sessions);
        assert_eq!(got.pending_count(&got.selected), 2);
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
        assert!(
            got.expanded
                .contains(&ProjectId::new("project-checkout-api"))
        );
        assert_eq!(got.expanded.len(), 1);
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
        assert!(
            !got.expanded
                .contains(&ProjectId::new("project-storefront-web"))
        );
        assert!(
            got.expanded
                .contains(&ProjectId::new("project-checkout-api"))
        );
        let target = got.groups[1].sessions[0].id.clone();
        got.update(Message::Select(target.clone()));
        assert_eq!(got.selected, target);
        assert!(
            got.expanded
                .contains(&ProjectId::new("project-storefront-web"))
        );
        assert!(
            got.expanded
                .contains(&ProjectId::new("project-checkout-api"))
        );
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
    fn expand_project_adds_and_retoggle_removes_only_it() {
        let mut got = state();
        got.update(Message::ExpandProject(ProjectId::new(
            "project-storefront-web",
        )));
        assert!(
            got.expanded
                .contains(&ProjectId::new("project-storefront-web"))
        );
        assert!(
            got.expanded
                .contains(&ProjectId::new("project-checkout-api"))
        );
        got.update(Message::ExpandProject(ProjectId::new(
            "project-storefront-web",
        )));
        assert!(
            !got.expanded
                .contains(&ProjectId::new("project-storefront-web"))
        );
        assert!(
            got.expanded
                .contains(&ProjectId::new("project-checkout-api"))
        );
    }

    #[test]
    fn expand_unknown_project_is_noop() {
        let mut got = state();
        let before = got.expanded.clone();
        got.update(Message::ExpandProject(ProjectId::new("no-such-project")));
        assert_eq!(got.expanded, before);
    }

    #[test]
    fn new_session_expands_its_project() {
        let mut got = state();
        got.update(Message::ExpandProject(ProjectId::new(
            "project-checkout-api",
        )));
        assert!(got.expanded.is_empty());
        got.update(Message::NewSession(ProjectId::new(
            "project-storefront-web",
        )));
        assert!(
            got.expanded
                .contains(&ProjectId::new("project-storefront-web"))
        );
        assert_eq!(got.expanded.len(), 1);
    }

    #[test]
    fn new_session_unknown_project_is_noop() {
        let mut got = state();
        let before = got.expanded.clone();
        let selected = got.selected.clone();
        got.update(Message::NewSession(ProjectId::new("no-such-project")));
        assert_eq!(got.expanded, before);
        assert_eq!(got.selected, selected);
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
                error: None,
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
                error: None,
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
                error: None,
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
                error: None,
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
                error: None,
            }
        );
    }

    #[test]
    fn pick_directory_is_a_child_noop_owned_by_the_app() {
        // The App owns the OS folder picker: the child never fills the
        // form itself. The App intercepts PickDirectory and routes the
        // picked path back as DirectoryChanged.
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::PickDirectory);
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::new(),
                name: String::new(),
                name_edited: false,
                error: None,
            }
        );
    }

    #[test]
    fn submit_is_a_child_noop_owned_by_the_app() {
        // The App validates asynchronously: the child never closes the
        // popup itself. The App intercepts SubmitAddProject and routes
        // the outcome back as AddProjectFailed or CloseAddProject.
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::DirectoryChanged(String::from("/x/y/z")));
        got.update(Message::SubmitAddProject);
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::from("/x/y/z"),
                name: String::from("z"),
                name_edited: false,
                error: None,
            }
        );
    }

    #[test]
    fn close_closes_the_popup() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::CloseAddProject);
        assert_eq!(got.add_project, AddProject::Closed);
    }

    #[test]
    fn form_messages_are_noops_while_closed() {
        let mut got = state();
        got.update(Message::DirectoryChanged(String::from("/x/y/z")));
        got.update(Message::NameChanged(String::from("z")));
        got.update(Message::PickDirectory);
        got.update(Message::SubmitAddProject);
        got.update(Message::AddProjectFailed(String::from("nope")));
        assert_eq!(got.add_project, AddProject::Closed);
    }

    #[test]
    fn add_project_failed_sets_error_while_open() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::AddProjectFailed(String::from(
            "That folder couldn't be found.",
        )));
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::new(),
                name: String::new(),
                name_edited: false,
                error: Some(String::from("That folder couldn't be found.")),
            }
        );
    }

    #[test]
    fn typing_clears_the_error() {
        let mut got = state();
        got.update(Message::OpenAddProject);
        got.update(Message::AddProjectFailed(String::from("bad")));
        got.update(Message::DirectoryChanged(String::from("/x/y/other")));
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::from("/x/y/other"),
                name: String::from("other"),
                name_edited: false,
                error: None,
            }
        );
        got.update(Message::AddProjectFailed(String::from("bad")));
        got.update(Message::NameChanged(String::from("typed")));
        assert_eq!(
            got.add_project,
            AddProject::Open {
                directory: String::from("/x/y/other"),
                name: String::from("typed"),
                name_edited: true,
                error: None,
            }
        );
    }

    #[test]
    fn remove_project_moves_sessions_to_unprojected_end() {
        let mut got = State::from_projects(
            vec![project("p-a", "a"), project("p-b", "b")],
            vec![
                row("s-1", Some("p-a")),
                row("s-2", Some("p-b")),
                row("s-3", Some("p-a")),
                row("s-0", None),
            ],
        );
        let selected = got.selected.clone();
        got.update(Message::RemoveProject(ProjectId::new("p-a")));
        assert_eq!(got.groups.len(), 1);
        assert_eq!(got.groups[0].project.id, ProjectId::new("p-b"));
        let unprojected: Vec<&str> = got
            .unprojected
            .iter()
            .map(|summary| summary.id.as_str())
            .collect();
        assert_eq!(unprojected, vec!["s-0", "s-1", "s-3"]);
        // Selection is unchanged even when its group is removed.
        assert_eq!(got.selected, selected);
    }

    #[test]
    fn remove_project_clears_expansion_only_for_the_removed_id() {
        let mut got = State::from_projects(
            vec![project("p-a", "a"), project("p-b", "b")],
            vec![row("s-1", Some("p-a")), row("s-2", Some("p-b"))],
        );
        got.update(Message::ExpandProject(ProjectId::new("p-b")));
        got.update(Message::RemoveProject(ProjectId::new("p-a")));
        assert!(got.expanded.contains(&ProjectId::new("p-b")));
        assert!(!got.expanded.contains(&ProjectId::new("p-a")));
        got.update(Message::RemoveProject(ProjectId::new("p-b")));
        assert!(got.expanded.is_empty());
    }

    #[test]
    fn remove_unknown_project_is_noop() {
        let mut got = state();
        let groups = got.groups.len();
        let unprojected = got.unprojected.len();
        let selected = got.selected.clone();
        let expanded = got.expanded.clone();
        got.update(Message::RemoveProject(ProjectId::new("no-such-project")));
        assert_eq!(got.groups.len(), groups);
        assert_eq!(got.unprojected.len(), unprojected);
        assert_eq!(got.selected, selected);
        assert_eq!(got.expanded, expanded);
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
            workspace: None,
        }
    }

    fn row(id: &str, project: Option<&str>) -> SessionSummary {
        SessionSummary {
            id: SessionId::new(id),
            title: String::from(id),
            status: SessionStatus::Idle,
            project: project.map(ProjectId::new),
            workspace: String::new(),
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
        assert!(got.expanded.contains(&ProjectId::new("p-b")));
        got.replace(
            vec![project("p-b", "b"), project("p-a", "a")],
            vec![row("s-2", Some("p-b")), row("s-3", Some("p-a"))],
        );
        assert_eq!(got.selected, SessionId::new("s-2"));
        assert!(got.expanded.contains(&ProjectId::new("p-b")));
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
        assert!(got.expanded.contains(&ProjectId::new("p-b")));
        assert_eq!(got.expanded.len(), 1);
    }

    #[test]
    fn replace_with_empty_uses_placeholder_selection() {
        let mut got = state();
        got.replace(Vec::new(), Vec::new());
        assert!(got.groups.is_empty());
        assert!(got.unprojected.is_empty());
        assert!(got.selected_summary().is_none());
        assert!(got.expanded.is_empty());
    }

    #[test]
    fn expanding_two_projects_keeps_both_and_collapsing_one_leaves_other() {
        let mut got = State::from_projects(
            vec![project("p-a", "a"), project("p-b", "b")],
            vec![row("s-1", Some("p-a")), row("s-2", Some("p-b"))],
        );
        // Boot expands only p-a.
        assert!(got.expanded.contains(&ProjectId::new("p-a")));
        assert!(!got.expanded.contains(&ProjectId::new("p-b")));
        got.update(Message::ExpandProject(ProjectId::new("p-b")));
        assert!(got.expanded.contains(&ProjectId::new("p-a")));
        assert!(got.expanded.contains(&ProjectId::new("p-b")));
        got.update(Message::ExpandProject(ProjectId::new("p-a")));
        assert!(!got.expanded.contains(&ProjectId::new("p-a")));
        assert!(got.expanded.contains(&ProjectId::new("p-b")));
    }

    #[test]
    fn selecting_session_in_another_project_adds_it_without_collapsing_first() {
        let mut got = State::from_projects(
            vec![project("p-a", "a"), project("p-b", "b")],
            vec![row("s-1", Some("p-a")), row("s-2", Some("p-b"))],
        );
        assert!(got.expanded.contains(&ProjectId::new("p-a")));
        got.update(Message::Select(SessionId::new("s-2")));
        assert_eq!(got.selected, SessionId::new("s-2"));
        assert!(got.expanded.contains(&ProjectId::new("p-a")));
        assert!(got.expanded.contains(&ProjectId::new("p-b")));
    }

    #[test]
    fn replace_with_surviving_selection_preserves_multiple_and_drops_removed() {
        let mut got = State::from_projects(
            vec![
                project("p-a", "a"),
                project("p-b", "b"),
                project("p-c", "c"),
            ],
            vec![
                row("s-1", Some("p-a")),
                row("s-2", Some("p-b")),
                row("s-3", Some("p-c")),
            ],
        );
        got.update(Message::ExpandProject(ProjectId::new("p-b")));
        got.update(Message::ExpandProject(ProjectId::new("p-c")));
        assert_eq!(got.expanded.len(), 3);
        // Selection (s-1) survives; p-c is gone from the fresh groups.
        got.replace(
            vec![project("p-b", "b"), project("p-a", "a")],
            vec![row("s-1", Some("p-a")), row("s-2", Some("p-b"))],
        );
        assert_eq!(got.selected, SessionId::new("s-1"));
        assert!(got.expanded.contains(&ProjectId::new("p-a")));
        assert!(got.expanded.contains(&ProjectId::new("p-b")));
        assert!(!got.expanded.contains(&ProjectId::new("p-c")));
        assert_eq!(got.expanded.len(), 2);
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
                error: None,
            }
        );
    }
}
