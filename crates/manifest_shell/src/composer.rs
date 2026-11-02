//! Composer: the draft input cluster at the foot of the main column.
//!
//! The composer owns the draft string. Sending is out of scope in M1
//! (no engine), so the send affordance renders disabled.

use iced::{Element, Length};

use crate::kit::{self, ButtonKind};

/// Messages the composer can receive, enumerated before the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The draft text changed.
    DraftChanged(String),
}

/// Composer state: the current draft.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// Text the user has typed but not sent.
    pub draft: String,
}

impl State {
    /// Builds composer state with an empty draft.
    pub fn new() -> Self {
        State {
            draft: String::new(),
        }
    }

    /// Folds a message into the composer state.
    pub fn update(&mut self, message: Message) {
        match message {
            Message::DraftChanged(next) => {
                self.draft = next;
            }
        }
    }

    /// Renders the input row: a text field plus a disabled Send button.
    pub fn view(&self) -> Element<'_, Message> {
        iced::widget::row![
            iced::widget::container(kit::text_field(
                "Message the agent...",
                &self.draft,
                Message::DraftChanged,
            ))
            .width(Length::Fill),
            kit::pill_button("Send", ButtonKind::Primary, None),
        ]
        .spacing(crate::theme::SPACE_S)
        .width(Length::Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_changed_replaces_draft() {
        let mut got = State {
            draft: String::from("a"),
        };
        got.update(Message::DraftChanged(String::from("ab")));
        assert_eq!(got.draft, "ab");
    }
}
