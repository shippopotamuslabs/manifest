//! Transcript: read-only history of the selected session.
//!
//! The transcript carries no messages in M1: it renders domain items
//! and nothing else. State is the item list; items read typographically
//! with no headers and no panel around them.

use iced::{Element, Length};
use manifest_domain::TranscriptItem;

use crate::kit;
use crate::theme::{FONT_SIZE_BODY, SPACE_M, TEXT_PRIMARY, TEXT_TERTIARY};

/// Transcript state: the visible items in display order.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// Transcript entries for the selected session.
    pub items: Vec<TranscriptItem>,
}

impl State {
    /// Builds transcript state from domain items.
    pub fn from_items(items: Vec<TranscriptItem>) -> Self {
        State { items }
    }

    /// Renders the scrollable transcript. User prompts render as
    /// filled bubbles; assistant and tool text sits unadorned in a
    /// fill-width container so long lines wrap (UI contract §4).
    pub fn view(&self) -> Element<'_, Message> {
        let mut list = iced::widget::column![].spacing(SPACE_M).width(Length::Fill);
        for item in &self.items {
            list = list.push(render_item(item));
        }
        iced::widget::scrollable(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// The transcript publishes no messages in M1. This uninhabited marker
/// keeps the module's message position explicit for the root enum.
#[derive(Debug, Clone)]
pub enum Message {}

/// Renders one transcript item, distinct by kind. Body text is borrowed,
/// never cloned: the view reads `&self` (UI contract §2).
fn render_item<'a>(item: &'a TranscriptItem) -> Element<'a, Message> {
    match item {
        TranscriptItem::UserTurn { text } => kit::prompt_bubble(text.as_str()),
        TranscriptItem::AssistantTurn { text } => iced::widget::container(
            iced::widget::text(text.as_str())
                .size(FONT_SIZE_BODY)
                .color(TEXT_PRIMARY),
        )
        .width(Length::Fill)
        .into(),
        TranscriptItem::ToolNote { text } => iced::widget::container(
            iced::widget::text(text.as_str())
                .size(FONT_SIZE_BODY)
                .color(TEXT_TERTIARY),
        )
        .width(Length::Fill)
        .into(),
    }
}
