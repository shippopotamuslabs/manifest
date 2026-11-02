//! Transcript: read-only history of the selected session.
//!
//! The transcript carries no messages in M1: it renders domain items
//! and nothing else. State is the item list; each item renders distinctly
//! by kind inside a [`kit::panel`] group.

use iced::{Element, Length};
use manifest_domain::TranscriptItem;

use crate::kit;
use crate::theme::{
    FONT_SIZE_BODY, FONT_SIZE_SMALL, SPACE_S, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};

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

    /// Renders the scrollable transcript. Each item keeps its kind's
    /// header ("You", "Assistant", "Tool note") and body text inside a
    /// fill-width container so long lines wrap (UI contract §4).
    pub fn view(&self) -> Element<'_, Message> {
        let mut list = iced::widget::column![].spacing(SPACE_S).width(Length::Fill);
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
    let (header, body) = match item {
        TranscriptItem::UserTurn { text } => ("You", text.as_str()),
        TranscriptItem::AssistantTurn { text } => ("Assistant", text.as_str()),
        TranscriptItem::ToolNote { text } => ("Tool note", text.as_str()),
    };
    let header_color = match item {
        TranscriptItem::UserTurn { .. } => TEXT_SECONDARY,
        TranscriptItem::AssistantTurn { .. } => TEXT_SECONDARY,
        TranscriptItem::ToolNote { .. } => TEXT_TERTIARY,
    };
    kit::panel(
        iced::widget::column![
            iced::widget::text(header)
                .size(FONT_SIZE_SMALL)
                .color(header_color),
            iced::widget::container(
                iced::widget::text(body)
                    .size(FONT_SIZE_BODY)
                    .color(TEXT_PRIMARY),
            )
            .width(Length::Fill),
        ]
        .spacing(SPACE_S)
        .width(Length::Fill),
    )
}
