//! Transcript: scrollable history of the selected session.
//!
//! The transcript renders domain items and tracks its scroll position
//! so the current user prompt sticks to the top while scrolling. Item
//! heights are estimated in [`State::on_viewport`] from the same
//! constants the view uses; the view itself stays a pure projection
//! of state (UI contract §§1-2).

use iced::{Element, Length};
use manifest_domain::TranscriptItem;

use crate::kit;
use crate::theme::{FONT_SIZE_BODY, SPACE_M, SPACE_S, TEXT_PRIMARY, TEXT_TERTIARY};

/// Default line spacing for transcript text. The transcript text
/// widgets set no explicit line height, so they use the framework
/// default of [`iced::widget::text::LineHeight::Relative`] 1.3.
const LINE_HEIGHT: f32 = FONT_SIZE_BODY * 1.3;
/// Mean glyph advance for the height estimator. Body-size proportional
/// type averages near half an em; tune against the live build if the
/// sticky timing ever drifts from the real text.
const ESTIMATED_CHAR_WIDTH: f32 = FONT_SIZE_BODY * 0.52;

/// Transcript state: the visible items plus scroll-derived sticky data.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// Transcript entries for the selected session.
    pub items: Vec<TranscriptItem>,
    /// Absolute vertical scroll offset in px, from the last viewport.
    pub scroll_offset: f32,
    /// Scrollable content width in px, from the last viewport.
    pub content_width: f32,
    /// Index into [`items`](State::items) of the prompt stuck to the top.
    pub sticky: Option<usize>,
    /// Px the sticky bubble is pushed up by the incoming prompt.
    pub sticky_push: f32,
    /// Full height of the sticky bubble in px; sizes the overlay window.
    pub sticky_height: f32,
}

/// Transcript messages, enumerated before the view (UI contract §1).
/// `Viewport` is `Debug + Clone` but not `PartialEq`, so neither is
/// this enum.
#[derive(Debug, Clone)]
pub enum Message {
    /// The scrollable published a new viewport after scrolling or a
    /// bounds change.
    Scrolled(iced::widget::scrollable::Viewport),
}

impl State {
    /// Builds transcript state from domain items. Scroll-derived fields
    /// start at zero / none, matching `Default`.
    pub fn from_items(items: Vec<TranscriptItem>) -> Self {
        State {
            items,
            scroll_offset: 0.0,
            content_width: 0.0,
            sticky: None,
            sticky_push: 0.0,
            sticky_height: 0.0,
        }
    }

    /// Reducer: folds a viewport sample into scroll and sticky state.
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Scrolled(viewport) => {
                let offset_y = viewport.absolute_offset().y;
                let width = viewport.content_bounds().width;
                self.on_viewport(offset_y, width);
            }
        }
    }

    /// Folds one viewport sample into scroll and sticky state. Tests in
    /// this module drive the reducer through this fn directly: a
    /// `Viewport` has no public constructor, so `update` itself is a
    /// trivial delegation that headless tests cannot feed.
    fn on_viewport(&mut self, offset_y: f32, content_width: f32) {
        self.scroll_offset = offset_y;
        self.content_width = content_width;
        if content_width <= 0.0 {
            self.sticky = None;
            self.sticky_push = 0.0;
            self.sticky_height = 0.0;
            return;
        }
        let tops = item_tops(&self.items, content_width);
        // Rule: nothing sticks while the offset sits at or above the top
        // of the first user prompt. There the overlay would exactly cover
        // the natural bubble, so leaving it out is cleaner.
        let first_top = self
            .items
            .iter()
            .enumerate()
            .find(|(_, item)| matches!(item, TranscriptItem::UserTurn { .. }))
            .map(|(index, _)| tops[index]);
        let mut current = None;
        if first_top.is_none_or(|top| offset_y > top) {
            for (index, item) in self.items.iter().enumerate() {
                if matches!(item, TranscriptItem::UserTurn { .. }) && tops[index] <= offset_y {
                    current = Some(index);
                }
            }
        }
        match current {
            None => {
                self.sticky = None;
                self.sticky_push = 0.0;
                self.sticky_height = 0.0;
            }
            Some(index) => {
                let height = item_height(&self.items[index], content_width);
                let next_top = self
                    .items
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .find(|(_, item)| matches!(item, TranscriptItem::UserTurn { .. }))
                    .map(|(next, _)| tops[next]);
                let push = match next_top {
                    Some(top) => {
                        let gap = top - offset_y;
                        if gap < height {
                            (height - gap).clamp(0.0, height)
                        } else {
                            0.0
                        }
                    }
                    None => 0.0,
                };
                self.sticky = Some(index);
                self.sticky_push = push;
                self.sticky_height = height;
            }
        }
    }

    /// Renders the scrollable transcript with the sticky prompt overlay.
    /// User prompts render as filled bubbles; assistant and tool text
    /// sits unadorned in a fill-width container so long lines wrap
    /// (UI contract §4). The overlay is display-only: containers and the
    /// pin forward events without capturing them, so wheel and pointer
    /// input still reach the scrollable beneath.
    pub fn view(&self) -> Element<'_, Message> {
        let mut list = iced::widget::column![].spacing(SPACE_M).width(Length::Fill);
        for item in &self.items {
            list = list.push(render_item(item));
        }
        let base: Element<'_, Message> = iced::widget::scrollable(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .on_scroll(Message::Scrolled)
            .into();
        let text = match self.sticky.and_then(|index| self.items.get(index)) {
            Some(TranscriptItem::UserTurn { text }) => text.as_str(),
            _ => return base,
        };
        // Top-anchored window onto the stuck bubble: the pin lifts the
        // identical bubble up by `sticky_push` px and the clipped fixed
        // container cuts whatever slides past the transcript's top edge,
        // so the overlay can never paint over the top bar.
        let overlay: Element<'_, Message> = iced::widget::container(
            iced::widget::pin(kit::prompt_bubble(text)).y(-self.sticky_push),
        )
        .width(Length::Fill)
        .height(Length::Fixed(
            (self.sticky_height - self.sticky_push).max(0.0),
        ))
        .clip(true)
        .into();
        iced::widget::stack![base, overlay]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// Greedy word wrap by character count. `max_chars` is the widest line
/// in characters; words longer than that occupy the ceiling of their
/// length over `max_chars`. Empty text is one line. Counts characters
/// with `.chars().count()`, never bytes.
fn wrapped_lines(text: &str, text_width: f32) -> usize {
    let max_chars = ((text_width / ESTIMATED_CHAR_WIDTH) as usize).max(1);
    if text.is_empty() {
        return 1;
    }
    let mut lines = 1;
    let mut used = 0;
    for word in text.split_whitespace() {
        let len = word.chars().count();
        if len > max_chars {
            if used > 0 {
                lines += 1;
            }
            lines += len.div_ceil(max_chars) - 1;
            used = match len % max_chars {
                0 => max_chars,
                rest => rest,
            };
        } else if used == 0 {
            used = len;
        } else if used + 1 + len <= max_chars {
            used += 1 + len;
        } else {
            lines += 1;
            used = len;
        }
    }
    lines
}

/// Estimated item height in px. Assistant and tool text spans the full
/// content width; a user bubble is narrower by its horizontal padding
/// (see [`kit::prompt_bubble`]) and taller by its vertical padding.
/// These constants must match that constructor and the column spacing.
fn item_height(item: &TranscriptItem, content_width: f32) -> f32 {
    let (text, is_user) = match item {
        TranscriptItem::UserTurn { text } => (text.as_str(), true),
        TranscriptItem::AssistantTurn { text } => (text.as_str(), false),
        TranscriptItem::ToolNote { text } => (text.as_str(), false),
    };
    let text_width = if is_user {
        content_width - 2.0 * SPACE_M
    } else {
        content_width
    }
    .max(ESTIMATED_CHAR_WIDTH);
    let height = wrapped_lines(text, text_width) as f32 * LINE_HEIGHT;
    if is_user {
        height + 2.0 * SPACE_S
    } else {
        height
    }
}

/// Top edge of every item in px. Items stack with `SPACE_M` between
/// them, no spacing before the first, and the content carries no other
/// padding.
fn item_tops(items: &[TranscriptItem], content_width: f32) -> Vec<f32> {
    let mut tops = Vec::with_capacity(items.len());
    let mut top = 0.0;
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            top += SPACE_M;
        }
        tops.push(top);
        top += item_height(item, content_width);
    }
    tops
}

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

#[cfg(test)]
mod tests {
    use super::{State, item_height, item_tops, wrapped_lines};
    use crate::theme::{SPACE_M, SPACE_S};
    use manifest_domain::{TranscriptItem, fixture};

    /// Indices of the user prompts in display order.
    fn user_indices(items: &[TranscriptItem]) -> Vec<usize> {
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, TranscriptItem::UserTurn { .. }))
            .map(|(index, _)| index)
            .collect()
    }

    #[test]
    fn short_text_stays_on_one_line() {
        assert_eq!(wrapped_lines("Hello", 600.0), 1);
    }

    #[test]
    fn wrapping_grows_lines_as_width_shrinks() {
        let text = "The refresh now runs inside the request path";
        assert_eq!(wrapped_lines(text, 600.0), 1);
        assert!(wrapped_lines(text, 100.0) > 1);
    }

    #[test]
    fn overlong_word_splits_across_lines() {
        let word = "a".repeat(50);
        assert_eq!(wrapped_lines(&word, 600.0), 1);
        assert_eq!(wrapped_lines(&word, 100.0), 4);
    }

    #[test]
    fn empty_text_is_one_line() {
        assert_eq!(wrapped_lines("", 600.0), 1);
    }

    #[test]
    fn user_bubble_is_taller_by_vertical_padding() {
        let text = String::from("Short line");
        let user = TranscriptItem::UserTurn { text: text.clone() };
        let assistant = TranscriptItem::AssistantTurn { text };
        let width = 600.0;
        let gap = item_height(&user, width) - item_height(&assistant, width);
        assert!((gap - 2.0 * SPACE_S).abs() < 0.001);
    }

    #[test]
    fn tops_accumulate_height_plus_spacing() {
        let state = State::from_items(fixture().transcript);
        let tops = item_tops(&state.items, 600.0);
        assert_eq!(tops.len(), state.items.len());
        assert_eq!(tops[0], 0.0);
        for (index, top) in tops.iter().enumerate().skip(1) {
            let want = tops[index - 1] + item_height(&state.items[index - 1], 600.0) + SPACE_M;
            assert!((top - want).abs() < 0.001);
        }
    }

    #[test]
    fn offset_zero_shows_no_sticky_for_leading_prompt() {
        let mut state = State::from_items(fixture().transcript);
        state.on_viewport(0.0, 600.0);
        assert_eq!(state.scroll_offset, 0.0);
        assert_eq!(state.content_width, 600.0);
        assert_eq!(state.sticky, None);
        assert_eq!(state.sticky_push, 0.0);
        assert_eq!(state.sticky_height, 0.0);
    }

    #[test]
    fn scrolled_past_first_prompt_sticks_it() {
        let mut state = State::from_items(fixture().transcript);
        let tops = item_tops(&state.items, 600.0);
        let users = user_indices(&state.items);
        assert!(users.len() >= 2);
        state.on_viewport(tops[users[0]] + 1.0, 600.0);
        assert_eq!(state.sticky, Some(users[0]));
        assert_eq!(state.sticky_push, 0.0);
        assert!(state.sticky_height > 0.0);
    }

    #[test]
    fn incoming_prompt_pushes_sticky_up() {
        let mut state = State::from_items(fixture().transcript);
        let tops = item_tops(&state.items, 600.0);
        let users = user_indices(&state.items);
        let height = item_height(&state.items[users[0]], 600.0);
        let offset = tops[users[1]] - height / 2.0;
        state.on_viewport(offset, 600.0);
        assert_eq!(state.sticky, Some(users[0]));
        assert!(state.sticky_push > 0.0 && state.sticky_push < state.sticky_height);
    }

    #[test]
    fn arriving_at_next_prompt_switches_sticky() {
        let mut state = State::from_items(fixture().transcript);
        let tops = item_tops(&state.items, 600.0);
        let users = user_indices(&state.items);
        state.on_viewport(tops[users[1]], 600.0);
        assert_eq!(state.sticky, Some(users[1]));
        assert_eq!(state.sticky_push, 0.0);
    }

    #[test]
    fn narrower_width_recomputes_sticky() {
        let mut state = State::from_items(fixture().transcript);
        let users = user_indices(&state.items);
        let tops_wide = item_tops(&state.items, 600.0);
        let height_wide = item_height(&state.items[users[0]], 600.0);
        let offset = tops_wide[users[1]] - height_wide / 2.0;
        state.on_viewport(offset, 600.0);
        assert_eq!(state.sticky, Some(users[0]));
        state.on_viewport(offset, 200.0);
        assert_eq!(state.content_width, 200.0);
        assert_eq!(state.sticky, Some(users[0]));
        assert!(state.sticky_height > height_wide);
    }

    #[test]
    fn zero_width_clears_sticky() {
        let mut state = State::from_items(fixture().transcript);
        state.on_viewport(50.0, 600.0);
        assert!(state.sticky.is_some());
        state.on_viewport(50.0, 0.0);
        assert_eq!(state.sticky, None);
        assert_eq!(state.sticky_push, 0.0);
        assert_eq!(state.sticky_height, 0.0);
    }

    #[test]
    fn empty_transcript_never_sticks() {
        let mut state = State::from_items(vec![]);
        state.on_viewport(100.0, 600.0);
        assert_eq!(state.sticky, None);
        assert_eq!(state.sticky_push, 0.0);
        assert_eq!(state.sticky_height, 0.0);
    }
}
