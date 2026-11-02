//! Session list: the left panel's state, messages, and view.
//!
//! Selection is owned here: [`State::selected`] is the single owner of
//! which session is selected, and the root reads it for the transcript
//! header. `update` changes the selection only for ids already in the
//! list; an unknown id is a no-op.

use iced::{Element, Length};
use manifest_domain::{SessionId, SessionStatus, SessionSummary};

use crate::kit;
use crate::theme::{FONT_SIZE_HEADING, SPACE_S, TEXT_SECONDARY};

/// Messages the session list can receive, enumerated before the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Select the session with this id, if it is in the list.
    Select(SessionId),
}

/// Session-list state: the rows in display order plus the selection.
#[derive(Debug, Clone)]
pub struct State {
    /// Sessions in display order.
    pub sessions: Vec<SessionSummary>,
    /// The selected session. Always an id from [`State::sessions`] when
    /// built from the fixture; see [`State::from_sessions`].
    pub selected: SessionId,
}

impl State {
    /// Builds list state from domain rows. The default selection is the
    /// first row's id; an empty input falls back to a blank placeholder
    /// id so boot stays total (no `unwrap` / panic paths). The fixture
    /// always carries sessions, so the fallback never fires in practice.
    pub fn from_sessions(sessions: Vec<SessionSummary>) -> Self {
        let selected = sessions
            .first()
            .map(|summary| summary.id.clone())
            .unwrap_or_else(|| SessionId::new(String::new()));
        State { sessions, selected }
    }

    /// Folds a message into the list state.
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Select(id) => {
                if self.sessions.iter().any(|summary| summary.id == id) {
                    self.selected = id;
                }
            }
        }
    }

    /// The selected row, if the selection names a listed session.
    pub fn selected_summary(&self) -> Option<&SessionSummary> {
        self.sessions
            .iter()
            .find(|summary| summary.id == self.selected)
    }

    /// Renders the panel: a titled column of [`kit::list_row`] entries.
    pub fn view(&self) -> Element<'_, Message> {
        let mut rows = iced::widget::column![
            iced::widget::text("Sessions")
                .size(FONT_SIZE_HEADING)
                .color(TEXT_SECONDARY),
        ]
        .spacing(SPACE_S)
        .width(Length::Fill);
        for summary in &self.sessions {
            rows = rows.push(kit::list_row(
                &summary.title,
                status_label(&summary.status),
                summary.id == self.selected,
                Message::Select(summary.id.clone()),
            ));
        }
        kit::panel_fill(iced::widget::container(rows).width(Length::Fill))
    }
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
        State::from_sessions(fixture().sessions)
    }

    #[test]
    fn select_known_id_changes_selection() {
        let mut got = state();
        let target = got.sessions[1].id.clone();
        got.update(Message::Select(target.clone()));
        assert_eq!(got.selected, target);
    }

    #[test]
    fn select_unknown_id_leaves_selection_unchanged() {
        let mut got = state();
        let before = got.selected.clone();
        got.update(Message::Select(SessionId::new("no-such-session")));
        assert_eq!(got.selected, before);
    }
}
