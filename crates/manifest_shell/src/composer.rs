//! Composer: the draft input cluster at the foot of the main column.
//!
//! The input grows as the user types, the attach and send buttons sit
//! inside the bar, and the sub-bar shows static fixture labels. There
//! is no attach flow. Send is wired at the App level: the view gates
//! the button on [`State::send_enabled`], and `App` intercepts
//! `SendPressed`; [`State::update`] still treats it as a no-op.

use iced::widget::text_editor;
use iced::{Element, Length};

use crate::kit::{self, IconButtonKind};
use crate::theme::{COMPOSER_MAX_LINES, SPACE_S, SPACE_XS};

/// Static fixture value naming the active mode.
pub const MODE_LABEL: &str = "Code";
/// Static fixture value naming the active model.
pub const MODEL_LABEL: &str = "Muse Spark · High";
/// Static fixture value naming the context usage.
pub const CONTEXT_LABEL: &str = "Context 18%";

/// Messages the composer can receive, enumerated before the view.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// The editor performed an editing action.
    Edit(text_editor::Action),
    /// The attach button was pressed; a no-op in this slice.
    AttachPressed,
    /// The send button was pressed; a no-op in this slice.
    SendPressed,
}

/// Composer state: the draft content plus the derived values the view
/// reads without computing them itself (UI contract §2).
#[derive(Debug, Clone)]
pub struct State {
    /// Text the user has typed but not sent.
    pub content: text_editor::Content,
    /// Visible input lines, grown from the content and clamped to
    /// [`COMPOSER_MAX_LINES`].
    pub visible_lines: usize,
    /// Whether the content holds any text; gates the send button.
    pub has_text: bool,
    /// Whether the send button is live. Owned by the App, which
    /// rewrites it as connection, selection, and in-flight state
    /// change; defaults to true so the button works before the App
    /// runs its first gate refresh.
    pub send_enabled: bool,
}

impl State {
    /// Builds composer state with empty content, one visible line,
    /// and the send button enabled.
    pub fn new() -> Self {
        State {
            content: text_editor::Content::new(),
            visible_lines: 1,
            has_text: false,
            send_enabled: true,
        }
    }

    /// Clears the draft after a send, restoring the [`State::new`]
    /// shape while preserving the App-owned [`send_enabled`](State::send_enabled)
    /// gate.
    pub fn clear(&mut self) {
        self.content = text_editor::Content::new();
        self.visible_lines = 1;
        self.has_text = false;
    }

    /// Restores a failed send's prompt into the draft: the inverse of
    /// [`State::clear`], recomputing the derived values from the text
    /// while preserving the App-owned
    /// [`send_enabled`](State::send_enabled) gate.
    pub fn restore(&mut self, text: String) {
        self.content = text_editor::Content::with_text(&text);
        self.has_text = !self.content.text().is_empty();
        self.visible_lines = self.content.line_count().clamp(1, COMPOSER_MAX_LINES);
    }

    /// Folds a message into the composer state. Editing actions apply
    /// to the content, then the derived values are recomputed and
    /// stored; the button presses are no-ops here because the App
    /// intercepts `SendPressed` before it reaches this reducer.
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Edit(action) => {
                self.content.perform(action);
                self.has_text = !self.content.text().is_empty();
                self.visible_lines = self.content.line_count().clamp(1, COMPOSER_MAX_LINES);
            }
            Message::AttachPressed | Message::SendPressed => {}
        }
    }

    /// Renders the input bar plus the fixture sub-bar below it.
    pub fn view(&self) -> Element<'_, Message> {
        let mut row = iced::widget::row![
            kit::icon_button("+", IconButtonKind::Subtle, Some(Message::AttachPressed)),
            iced::widget::container(kit::composer_input(
                &self.content,
                self.visible_lines,
                Message::Edit,
            ))
            .width(Length::Fill),
        ]
        .spacing(SPACE_XS)
        .align_y(iced::alignment::Vertical::Bottom);
        if self.has_text {
            row = row.push(kit::icon_button(
                "↑",
                IconButtonKind::Accent,
                self.send_enabled.then_some(Message::SendPressed),
            ));
        }
        let sub = iced::widget::row![
            kit::static_pill(MODE_LABEL),
            iced::widget::container(iced::widget::text("")).width(Length::Fill),
            kit::static_pill(MODEL_LABEL),
            kit::static_pill(CONTEXT_LABEL),
        ]
        .spacing(SPACE_S)
        .width(Length::Fill);
        iced::widget::column![kit::input_bar(row), sub]
            .spacing(SPACE_XS)
            .width(Length::Fill)
            .into()
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types text through edit actions, mapping newlines to Enter.
    fn type_text(state: &mut State, text: &str) {
        for next in text.chars() {
            let edit = if next == '\n' {
                text_editor::Edit::Enter
            } else {
                text_editor::Edit::Insert(next)
            };
            state.update(Message::Edit(text_editor::Action::Edit(edit)));
        }
    }

    #[test]
    fn default_matches_new() {
        let got = State::default();
        let want = State::new();
        assert_eq!(got.visible_lines, 1);
        assert_eq!(got.visible_lines, want.visible_lines);
        assert_eq!(got.has_text, want.has_text);
        assert!(!got.has_text);
        assert_eq!(got.send_enabled, want.send_enabled);
        assert!(got.send_enabled);
        assert_eq!(got.content.text(), want.content.text());
        assert_eq!(got.content.text(), "");
    }

    #[test]
    fn clear_resets_draft_but_preserves_send_enabled() {
        let mut got = State::new();
        type_text(&mut got, "draft\ntext");
        got.send_enabled = false;
        got.clear();
        assert_eq!(got.content.text(), "");
        assert!(!got.has_text);
        assert_eq!(got.visible_lines, 1);
        assert!(!got.send_enabled);
    }

    #[test]
    fn typing_inserts_text_and_shows_send() {
        let mut got = State::new();
        assert!(!got.has_text);
        type_text(&mut got, "hi");
        assert_eq!(got.content.text(), "hi");
        assert!(got.has_text);
    }

    #[test]
    fn newlines_grow_visible_lines() {
        let mut got = State::new();
        assert_eq!(got.visible_lines, 1);
        type_text(&mut got, "a\nb");
        assert_eq!(got.visible_lines, 2);
        type_text(&mut got, "\nc");
        assert_eq!(got.visible_lines, 3);
    }

    #[test]
    fn visible_lines_clamp_at_max() {
        let mut got = State::new();
        type_text(&mut got, "\n\n\n\n\n\n");
        assert_eq!(got.visible_lines, COMPOSER_MAX_LINES);
    }

    #[test]
    fn deleting_all_text_resets_derived_state() {
        let mut got = State::new();
        type_text(&mut got, "hello\nworld");
        assert!(got.has_text);
        got.update(Message::Edit(text_editor::Action::SelectAll));
        got.update(Message::Edit(text_editor::Action::Edit(
            text_editor::Edit::Backspace,
        )));
        assert_eq!(got.content.text(), "");
        assert!(!got.has_text);
        assert_eq!(got.visible_lines, 1);
    }

    #[test]
    fn restore_sets_text_and_derived_state_preserving_send_enabled() {
        let mut got = State::new();
        got.send_enabled = false;
        got.restore(String::from("hello\nworld"));
        assert_eq!(got.content.text(), "hello\nworld");
        assert!(got.has_text);
        assert_eq!(got.visible_lines, 2);
        assert!(!got.send_enabled);
        got.send_enabled = true;
        got.restore(String::new());
        assert_eq!(got.content.text(), "");
        assert!(!got.has_text);
        assert_eq!(got.visible_lines, 1);
        assert!(got.send_enabled);
    }

    #[test]
    fn button_presses_leave_state_unchanged() {
        let mut got = State::new();
        type_text(&mut got, "draft\ntext");
        let text = got.content.text();
        let lines = got.visible_lines;
        let has_text = got.has_text;
        got.update(Message::SendPressed);
        got.update(Message::AttachPressed);
        assert_eq!(got.content.text(), text);
        assert_eq!(got.visible_lines, lines);
        assert_eq!(got.has_text, has_text);
    }
}
