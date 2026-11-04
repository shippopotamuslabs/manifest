//! Manifest widget kit: every styled widget the shell composes from.
//!
//! All widget styling decisions live here (or in [`crate::theme`]);
//! feature call sites use these constructors and never style widgets
//! themselves (UI contract §5, plan D5). Colors, spacing, radii, and
//! type sizes come from [`crate::theme`] tokens only — no literals here.
//!
//! Surfaces are borderless: panels, rows, chips, and buttons keep
//! their rounded corners but carry no border width, so state reads
//! through flat background fills alone. Three exceptions: the
//! single-line text field, whose border marks where typing lands,
//! the floating popup cards, whose thin border lifts them off the
//! surfaces they overlay, and the goal inline editor, whose stepped
//! surface and border mark where typing lands on its card.
//!
use iced::widget::{button, container, text, text_editor, text_input};
use iced::{Background, Border, Color, Element, Length, Rectangle, Theme};
use manifest_domain::{GoalStatus, TodoStatus};

use crate::theme::{
    ACCENT_ACTIVE, ACCENT_BASE, ACCENT_BORDER_SUBTLE, ACCENT_HOVER, ACCENT_MUTED_BG, BG_BASE,
    BG_SUBTLE, BORDER_DEFAULT, BORDER_SUBTLE, COMPOSER_INPUT_PADDING, COMPOSER_LINE_HEIGHT,
    CONTEXT_RING_SIZE, CONTEXT_RING_STROKE, DANGER, DIVIDER_HIT_WIDTH, FONT_SIZE_BODY,
    FONT_SIZE_PANEL, FONT_SIZE_PANEL_SMALL, FONT_SIZE_SMALL, GOAL_EDITOR_MAX_LINES,
    ICON_BUTTON_SIZE, ICON_SIZE, MENU_ROW_HEIGHT, RADIUS_PANEL, RADIUS_PILL, SCRIM, SPACE_L,
    SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXS, SUCCESS, SURFACE_1, SURFACE_2, TEXT_FAINT,
    TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};

/// The pill-button treatments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    Primary,
    Ghost,
    /// Filled danger treatment: the palette error red as the fill.
    Danger,
    /// Ghost treatment whose label stays in the palette error red.
    DangerGhost,
}

/// Pure data mapping behind the pill-button styles: background and text
/// color for a kind in a status. Kept as data (not `Style`) so the
/// mapping is unit-testable without constructing widgets.
pub(crate) fn button_colors(kind: ButtonKind, status: button::Status) -> (Option<Color>, Color) {
    match kind {
        ButtonKind::Primary => {
            let background = match status {
                button::Status::Active => ACCENT_BASE,
                button::Status::Hovered => ACCENT_HOVER,
                button::Status::Pressed => ACCENT_ACTIVE,
                button::Status::Disabled => SURFACE_2,
            };
            let text = match status {
                button::Status::Disabled => TEXT_FAINT,
                _ => BG_BASE,
            };
            (Some(background), text)
        }
        ButtonKind::Ghost => {
            let background = match status {
                button::Status::Hovered => Some(SURFACE_1),
                button::Status::Pressed => Some(SURFACE_2),
                button::Status::Active | button::Status::Disabled => None,
            };
            let text = match status {
                button::Status::Active => TEXT_SECONDARY,
                button::Status::Hovered | button::Status::Pressed => TEXT_PRIMARY,
                button::Status::Disabled => TEXT_FAINT,
            };
            (background, text)
        }
        ButtonKind::Danger => {
            let background = match status {
                button::Status::Active | button::Status::Hovered | button::Status::Pressed => {
                    DANGER
                }
                button::Status::Disabled => SURFACE_2,
            };
            let text = match status {
                button::Status::Disabled => TEXT_FAINT,
                _ => BG_BASE,
            };
            (Some(background), text)
        }
        ButtonKind::DangerGhost => {
            let background = match status {
                button::Status::Hovered => Some(SURFACE_1),
                button::Status::Pressed => Some(SURFACE_2),
                button::Status::Active | button::Status::Disabled => None,
            };
            let text = match status {
                button::Status::Disabled => TEXT_FAINT,
                _ => DANGER,
            };
            (background, text)
        }
    }
}

/// Status-aware style for [`ButtonKind::Primary`].
fn primary_style(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = button_colors(ButtonKind::Primary, status);
    // The primary mapping always yields a background; the fallback keeps
    // styling total without a panic path.
    let background = match background {
        Some(color) => color,
        None => SURFACE_2,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border::default().width(0.0).rounded(RADIUS_PILL),
        ..button::Style::default()
    }
}

/// Status-aware style for [`ButtonKind::Danger`].
fn danger_style(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = button_colors(ButtonKind::Danger, status);
    // The danger mapping always yields a background; the fallback keeps
    // styling total without a panic path.
    let background = match background {
        Some(color) => color,
        None => SURFACE_2,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border::default().width(0.0).rounded(RADIUS_PILL),
        ..button::Style::default()
    }
}

/// Status-aware style for [`ButtonKind::DangerGhost`].
fn danger_ghost_style(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = button_colors(ButtonKind::DangerGhost, status);
    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: Border::default().width(0.0).rounded(RADIUS_PILL),
        ..button::Style::default()
    }
}

/// Status-aware style for [`ButtonKind::Ghost`].
fn ghost_style(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = button_colors(ButtonKind::Ghost, status);
    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: Border::default().width(0.0).rounded(RADIUS_PILL),
        ..button::Style::default()
    }
}

/// A pill button. `on_press: None` renders disabled (the framework
/// reports [`button::Status::Disabled`], so the muted colors apply) and
/// emits no message.
pub(crate) fn pill_button<'a, Message: Clone + 'a>(
    label: &'a str,
    kind: ButtonKind,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let style = match kind {
        ButtonKind::Primary => primary_style,
        ButtonKind::Ghost => ghost_style,
        ButtonKind::Danger => danger_style,
        ButtonKind::DangerGhost => danger_ghost_style,
    };
    button(text(label))
        .padding(SPACE_S)
        .on_press_maybe(on_press)
        .style(style)
        .into()
}

/// A pill button with a leading Lucide icon: icon and label centered
/// in a row, reusing the pill treatments.
pub(crate) fn icon_pill_button<'a, Message: Clone + 'a>(
    icon: lucide_icons::Icon,
    label: &'a str,
    kind: ButtonKind,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let style = match kind {
        ButtonKind::Primary => primary_style,
        ButtonKind::Ghost => ghost_style,
        ButtonKind::Danger => danger_style,
        ButtonKind::DangerGhost => danger_ghost_style,
    };
    let content = iced::widget::row![
        text(char::from(icon).to_string())
            .font(iced::Font::with_name("lucide"))
            .size(ICON_SIZE),
        text(label),
    ]
    .spacing(SPACE_XS)
    .align_y(iced::alignment::Vertical::Center);
    button(content)
        .padding(SPACE_S)
        .on_press_maybe(on_press)
        .style(style)
        .into()
}

/// Sweep fraction behind the context ring: `None` draws the bare
/// track, `Some(f)` sweeps `f` of the circle from the top. Pure so
/// the meter math is unit-testable without constructing widgets.
pub(crate) fn ring_fraction(percent: Option<f32>) -> Option<f32> {
    percent.map(|p| (p / 100.0).clamp(0.0, 1.0))
}

/// The canvas program behind [`context_ring`] and the goal ring: a
/// faint track circle plus a progress arc starting at the top. The arc
/// color is a parameter now, so the goal ring can reuse this drawing
/// with its own tone while the context ring keeps the accent.
#[derive(Debug, Clone, Copy)]
struct ContextRingProgram {
    fraction: Option<f32>,
    /// Arc color: the accent for context, the goal tone for goals.
    color: Color,
}

impl<Message> iced::widget::canvas::Program<Message> for ContextRingProgram {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Path, Stroke, path::Arc};
        use std::f32::consts::PI;
        let mut frame = Frame::new(renderer, bounds.size());
        let radius = (bounds.width.min(bounds.height) - CONTEXT_RING_STROKE).max(0.0) / 2.0;
        let track = Path::circle(frame.center(), radius);
        frame.stroke(
            &track,
            Stroke::default()
                .with_width(CONTEXT_RING_STROKE)
                .with_color(TEXT_FAINT),
        );
        if let Some(fraction) = self.fraction.filter(|f| *f > 0.0) {
            let start = -PI / 2.0;
            let arc = Path::new(|builder| {
                builder.arc(Arc {
                    center: frame.center(),
                    radius,
                    start_angle: start.into(),
                    end_angle: (start + fraction * 2.0 * PI).into(),
                });
            });
            frame.stroke(
                &arc,
                Stroke::default()
                    .with_width(CONTEXT_RING_STROKE)
                    .with_line_cap(iced::widget::canvas::LineCap::Round)
                    .with_color(self.color),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// A clickable context ring for the composer's sub-bar: a small
/// canvas drawing the track circle plus the usage arc (`None` draws
/// the track only), wrapped in a ghost-style button so hover and
/// press read like the neighboring pills. No text, ever.
pub(crate) fn context_ring<'a, Message: Clone + 'a>(
    percent: Option<f32>,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    use iced::widget::canvas::Canvas;
    let ring = Canvas::new(ContextRingProgram {
        fraction: ring_fraction(percent),
        color: ACCENT_BASE,
    })
    .width(Length::Fixed(CONTEXT_RING_SIZE))
    .height(Length::Fixed(CONTEXT_RING_SIZE));
    button(ring)
        .padding(SPACE_XS)
        .on_press_maybe(on_press)
        .style(ghost_style)
        .into()
}

/// Pure data behind the goal ring: the centered glyph plus the arc
/// and glyph colors for a goal status. Non-completed goals carry a
/// target at center (active goals sweep the accent arc under it);
/// paused goals freeze the sent percent under a faint arc and a
/// tertiary target, so the paused distinction is tones only;
/// completed goals read full and green with a check; anything else
/// reads like an active goal. Kept as data (not widgets) so the
/// mapping is unit-testable.
pub(crate) fn goal_ring_presentation(status: &GoalStatus) -> (lucide_icons::Icon, Color, Color) {
    match status {
        GoalStatus::Completed => (
            lucide_icons::Icon::CircleCheck,
            SUCCESS,
            SUCCESS,
        ),
        GoalStatus::Paused => (
            lucide_icons::Icon::Target,
            TEXT_FAINT,
            TEXT_TERTIARY,
        ),
        GoalStatus::Active | GoalStatus::Other(_) => {
            (lucide_icons::Icon::Target, ACCENT_BASE, TEXT_SECONDARY)
        }
    }
}

/// The goal ring drawing without its button: the track circle plus
/// the status-toned arc from [`ContextRingProgram`], with the status
/// glyph centered on top. Button-less so the sub-bar chip and the
/// popover readout can each wrap it in their own single press target
/// instead of nesting buttons.
pub(crate) fn goal_ring_content<'a, Message: 'a>(
    percent: f32,
    status: &GoalStatus,
    size: f32,
) -> Element<'a, Message> {
    use iced::widget::canvas::Canvas;
    let (glyph, arc, glyph_color) = goal_ring_presentation(status);
    let ring = Canvas::new(ContextRingProgram {
        fraction: ring_fraction(Some(percent)),
        color: arc,
    })
    .width(Length::Fixed(size))
    .height(Length::Fixed(size));
    let badge = container(icon(glyph, size * 0.55, glyph_color))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center);
    iced::widget::Stack::new()
        .push(ring)
        .push(badge)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

/// A compact goal pill for the composer's sub-bar: one ghost-style
/// button (the same [`ghost_style`] treatment as [`context_ring`]
/// and the sub-bar pills, so hover and press read alike) whose
/// content is the button-less [`goal_ring_content`] drawing beside
/// the precomputed percent label. The whole pill — ring, percent,
/// and background — is a single click target emitting the press
/// message, so clicking anywhere on it opens the goal popover. The
/// ring stays button-less inside: a button in a button is illegal,
/// so the shared drawing is composed directly into the content row.
/// A paused percent reads tertiary; every other status reads
/// secondary.
pub(crate) fn goal_chip<'a, Message: Clone + 'a>(
    percent: f32,
    percent_label: &'a str,
    status: &GoalStatus,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let paused = matches!(status, GoalStatus::Paused);
    let content = iced::widget::row![
        goal_ring_content(percent, status, CONTEXT_RING_SIZE),
        text(percent_label)
            .size(FONT_SIZE_PANEL)
            .color(if paused { TEXT_TERTIARY } else { TEXT_SECONDARY }),
    ]
    .spacing(SPACE_XXS)
    .align_y(iced::alignment::Vertical::Center);
    button(content)
        .padding(SPACE_XS)
        .on_press_maybe(on_press)
        .style(ghost_style)
        .into()
}

/// A panel: surface-1 fill, no border, panel radius, medium padding.
fn panel_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE_1)),
        border: Border::default().width(0.0).rounded(RADIUS_PANEL),
        ..container::Style::default()
    }
}

/// Wraps content in a [`panel_style`] container.
pub(crate) fn panel<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .padding(SPACE_M)
        .style(panel_style)
        .into()
}

/// Status-aware style for [`popup_card`]: a surface-1 step distinct
/// from the transcript background, panel radius, and a thin 1px
/// border so the floating card reads apart from the input bar and
/// any same-tone surface it overlays.
fn popup_card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE_1)),
        border: Border::default()
            .color(BORDER_DEFAULT)
            .width(1.0)
            .rounded(RADIUS_PANEL),
        ..container::Style::default()
    }
}

/// A compact popup card at a fixed width: surface-1 fill, panel
/// radius, small padding. Callers right-align it in their layout.
pub(crate) fn popup_card<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    width: f32,
) -> Element<'a, Message> {
    container(content)
        .padding(SPACE_S)
        .width(Length::Fixed(width))
        .style(popup_card_style)
        .into()
}

/// Pure data mapping behind the popup menu rows: a selected row is a
/// quiet surface-2 step with primary text; an unselected row is
/// transparent at rest and surface-1 on hover. Kept as data (not
/// `Style`) so the mapping is unit-testable without widgets.
pub(crate) fn menu_row_colors(selected: bool, status: button::Status) -> (Option<Color>, Color) {
    row_colors(selected, status)
}

/// Pure data mapping behind the quiet "Edit" text action on a menu
/// row: transparent at rest, a surface-2 step on hover, tertiary text
/// warming to primary under the pointer.
pub(crate) fn edit_button_colors(status: button::Status) -> (Option<Color>, Color) {
    match status {
        button::Status::Hovered | button::Status::Pressed => (Some(SURFACE_2), TEXT_PRIMARY),
        button::Status::Active => (None, TEXT_TERTIARY),
        button::Status::Disabled => (None, TEXT_FAINT),
    }
}

/// Status-aware style for popup menu and choice rows.
fn menu_row_style(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme: &Theme, status: button::Status| {
        let (background, text_color) = menu_row_colors(selected, status);
        button::Style {
            background: background.map(Background::Color),
            text_color,
            border: Border::default().width(0.0).rounded(RADIUS_PANEL),
            ..button::Style::default()
        }
    }
}

/// Status-aware style for the quiet "Edit" text action.
fn edit_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = edit_button_colors(status);
    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: Border::default().width(0.0).rounded(RADIUS_PANEL),
        ..button::Style::default()
    }
}

/// A popup menu row: a full-width label button, a separate quiet
/// "Edit" text button beside it (a sibling, never nested), and a
/// check slot at the right that shows a check icon only on the selected row.
pub(crate) fn menu_row<'a, Message: Clone + 'a>(
    label: &'a str,
    selected: bool,
    on_press: Message,
    on_edit: Message,
) -> Element<'a, Message> {
    let title_color = if selected {
        TEXT_PRIMARY
    } else {
        TEXT_SECONDARY
    };
    let check: Element<'a, Message> = if selected {
        icon(
            lucide_icons::Icon::Check,
            FONT_SIZE_PANEL_SMALL,
            TEXT_SECONDARY,
        )
    } else {
        text("").size(FONT_SIZE_PANEL_SMALL).into()
    };
    iced::widget::row![
        button(
            container(
                text(label)
                    .size(FONT_SIZE_PANEL)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .color(title_color),
            )
            .width(Length::Fill)
            .clip(true),
        )
        .width(Length::Fill)
        .height(Length::Fixed(MENU_ROW_HEIGHT))
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_press)
        .style(menu_row_style(selected)),
        button(text("Edit").size(FONT_SIZE_PANEL_SMALL))
            .height(Length::Fixed(MENU_ROW_HEIGHT))
            .padding([SPACE_XS, SPACE_S])
            .on_press(on_edit)
            .style(edit_button_style),
        container(check)
            .width(Length::Fixed(SPACE_L))
            .align_x(iced::alignment::Horizontal::Center),
    ]
    .spacing(SPACE_XXS)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill)
    .into()
}

/// Icon for a TODO drawer row: check-in-circle when done, play
/// button in a circle while in progress, empty circle when pending
/// or cancelled.
pub(crate) fn todo_status_icon(status: TodoStatus) -> lucide_icons::Icon {
    match status {
        TodoStatus::Done => lucide_icons::Icon::CircleCheck,
        TodoStatus::InProgress => lucide_icons::Icon::CirclePlay,
        TodoStatus::Pending | TodoStatus::Cancelled => lucide_icons::Icon::Circle,
    }
}

/// Icon for a selection row: filled dot/check when selected.
pub(crate) fn selection_icon(selected: bool, multi: bool) -> lucide_icons::Icon {
    match (multi, selected) {
        (true, true) => lucide_icons::Icon::SquareCheck,
        (true, false) => lucide_icons::Icon::Square,
        (false, true) => lucide_icons::Icon::CircleDot,
        (false, false) => lucide_icons::Icon::Circle,
    }
}

/// Whether pager chrome shows: hidden for a single question.
pub(crate) fn pager_visible(total: usize) -> bool {
    total > 1
}

/// Precomputed pager label, e.g. `2 of 3`; empty when hidden.
pub(crate) fn pager_label(index: usize, total: usize) -> String {
    if pager_visible(total) {
        format!("{} of {}", index + 1, total)
    } else {
        String::new()
    }
}

/// Badge label for a pending count; `None` hides the badge.
pub(crate) fn pending_badge_label(count: u64) -> Option<String> {
    if count == 0 {
        None
    } else {
        Some(format!("{count} pending"))
    }
}

/// A radio (single) or checkbox (multi) row: selection icon, label,
/// and a secondary description line when present.
pub(crate) fn selection_row<'a, Message: Clone + 'a>(
    label: &'a str,
    description: Option<&'a str>,
    selected: bool,
    multi: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let title_color = if selected {
        TEXT_PRIMARY
    } else {
        TEXT_SECONDARY
    };
    let mut text_col = iced::widget::column![text(label).size(FONT_SIZE_PANEL).color(title_color)]
        .spacing(SPACE_XXS);
    if let Some(description) = description
        && !description.is_empty()
    {
        text_col = text_col.push(
            text(description)
                .size(FONT_SIZE_PANEL_SMALL)
                .color(TEXT_TERTIARY),
        );
    }
    let content = iced::widget::row![
        icon(
            selection_icon(selected, multi),
            FONT_SIZE_PANEL,
            title_color
        ),
        container(text_col).width(Length::Fill),
    ]
    .spacing(SPACE_S)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill);
    button(content)
        .width(Length::Fill)
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_press)
        .style(menu_row_style(selected))
        .into()
}

/// A monospace block for a raw command/args line.
pub(crate) fn code_block<'a, Message: Clone + 'a>(value: &'a str) -> Element<'a, Message> {
    container(
        text(value)
            .size(FONT_SIZE_PANEL)
            .color(TEXT_PRIMARY)
            .font(iced::Font::MONOSPACE),
    )
    .width(Length::Fill)
    .padding(SPACE_S)
    .style(panel_style)
    .into()
}

/// A pending-count badge pill.
pub(crate) fn pending_badge<'a, Message: Clone + 'a>(count: u64) -> Option<Element<'a, Message>> {
    pending_badge_label(count).map(|label| {
        container(text(label).size(FONT_SIZE_PANEL_SMALL).color(TEXT_PRIMARY))
            .padding([SPACE_XS, SPACE_S])
            .style(panel_flat_pill_style)
            .into()
    })
}

/// A simple popup choice row: a full-width label button with a check
/// slot at the right that shows a check icon only on the selected row.
pub(crate) fn choice_row<'a, Message: Clone + 'a>(
    label: &'a str,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let title_color = if selected {
        TEXT_PRIMARY
    } else {
        TEXT_SECONDARY
    };
    let check: Element<'a, Message> = if selected {
        icon(
            lucide_icons::Icon::Check,
            FONT_SIZE_PANEL_SMALL,
            TEXT_SECONDARY,
        )
    } else {
        text("").size(FONT_SIZE_PANEL_SMALL).into()
    };
    let content = iced::widget::row![
        container(
            text(label)
                .size(FONT_SIZE_PANEL)
                .wrapping(iced::widget::text::Wrapping::None)
                .color(title_color),
        )
        .width(Length::Fill)
        .clip(true),
        container(check)
            .width(Length::Fixed(SPACE_L))
            .align_x(iced::alignment::Horizontal::Center),
    ]
    .spacing(SPACE_S)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill);
    button(content)
        .width(Length::Fill)
        .height(Length::Fixed(MENU_ROW_HEIGHT))
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_press)
        .style(menu_row_style(selected))
        .into()
}

/// A quiet one-line note inside a popup card, for loading and
/// unavailable states: small tertiary text, never a button.
pub(crate) fn popup_note<'a, Message: Clone + 'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(FONT_SIZE_PANEL_SMALL).color(TEXT_TERTIARY))
        .padding([SPACE_XS, SPACE_S])
        .width(Length::Fill)
        .into()
}

/// A de-emphasized section label above a panel section.
pub(crate) fn section_header<'a, Message: Clone + 'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(FONT_SIZE_PANEL_SMALL).color(TEXT_TERTIARY))
        .padding(iced::Padding {
            top: SPACE_M,
            right: SPACE_S,
            bottom: 0.0,
            left: SPACE_S,
        })
        .width(Length::Fill)
        .into()
}

/// A popup section header: small tertiary text above a card's rows.
pub(crate) fn popup_header<'a, Message: Clone + 'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(FONT_SIZE_PANEL_SMALL).color(TEXT_TERTIARY))
        .padding([SPACE_XS, SPACE_S])
        .width(Length::Fill)
        .into()
}

/// Status-aware style for [`side_panel`]: the subtle background step,
/// edge to edge with no border and no radius.
fn side_panel_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_SUBTLE)),
        ..container::Style::default()
    }
}

/// The left panel: the subtle background fill running edge to edge,
/// medium padding, mirroring [`panel`]'s shape without its radius.
pub(crate) fn side_panel<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .padding(SPACE_M)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(side_panel_style)
        .into()
}

/// Status-aware style for [`vertical_divider`]: the subtle border step.
fn vertical_divider_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BORDER_SUBTLE)),
        ..container::Style::default()
    }
}

/// The thin divider between the side panel and the main chat: a 1px
/// fixed-width, fill-height container in the subtle border tone.
/// Only used as the visible line inside [`panel_divider`].
fn vertical_divider<'a, Message: Clone + 'a>() -> Element<'a, Message> {
    container(text(""))
        .width(Length::Fixed(1.0))
        .height(Length::Fill)
        .style(vertical_divider_style)
        .into()
}

/// The draggable divider between the side panel and the main chat: a
/// fixed-width, fill-height hit zone with the 1px divider line
/// centered horizontally inside it, so the grab target is wider than
/// the visible line. The caller wraps this in a mouse area.
pub(crate) fn panel_divider<'a, Message: Clone + 'a>() -> Element<'a, Message> {
    container(vertical_divider())
        .width(Length::Fixed(DIVIDER_HIT_WIDTH))
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Center)
        .into()
}

/// Status-aware style for [`title_bar`]: the same subtle background
/// step as the side panel, so header and panel read as one L.
fn title_bar_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_SUBTLE)),
        ..container::Style::default()
    }
}

/// The themed title bar replacing the OS decorations: full width with
/// bar padding on the subtle background fill.
pub(crate) fn title_bar<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding([SPACE_S, SPACE_M])
        .style(title_bar_style)
        .into()
}

/// Status-aware style for the single-line input.
fn field_style(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Focused { .. } => ACCENT_BORDER_SUBTLE,
        _ => BORDER_DEFAULT,
    };
    text_input::Style {
        background: Background::Color(SURFACE_1),
        border: Border::default()
            .color(border_color)
            .width(1.0)
            .rounded(RADIUS_PANEL),
        icon: TEXT_SECONDARY,
        placeholder: TEXT_FAINT,
        value: TEXT_PRIMARY,
        selection: ACCENT_MUTED_BG,
    }
}

/// A single-line text input on surface-1 with an accent focus border.
pub(crate) fn text_field<'a, Message: Clone + 'a>(
    placeholder: &'a str,
    value: &str,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    text_input::TextInput::new(placeholder, value)
        .on_input(on_input)
        .size(FONT_SIZE_BODY)
        .padding(SPACE_S)
        .style(field_style)
        .into()
}

/// Pure data mapping behind the session-row styles: background and
/// title color for a selection state in a button status. Kept as data
/// (not `Style`) so the mapping is unit-testable without constructing
/// widgets. Selection is a quiet surface-2 step, not the accent wash.
pub(crate) fn row_colors(selected: bool, status: button::Status) -> (Option<Color>, Color) {
    let background = if selected {
        Some(SURFACE_2)
    } else {
        match status {
            button::Status::Hovered | button::Status::Pressed => Some(SURFACE_1),
            button::Status::Active | button::Status::Disabled => None,
        }
    };
    let title = if selected {
        TEXT_PRIMARY
    } else {
        TEXT_SECONDARY
    };
    (background, title)
}

/// Status-aware style for a session list row.
fn row_style(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme: &Theme, status: button::Status| {
        let (background, text_color) = row_colors(selected, status);
        button::Style {
            background: background.map(Background::Color),
            text_color,
            border: Border::default().width(0.0).rounded(RADIUS_PANEL),
            ..button::Style::default()
        }
    }
}

/// A dense single-line session row: the title clips on one line with
/// a faint trailing label pinned to the right edge. Selected titles
/// read primary on surface-2; unselected titles read secondary and sit
/// transparent until hovered.
#[allow(dead_code)]
pub(crate) fn list_row<'a, Message: Clone + 'a>(
    title: &'a str,
    trailing: &'a str,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
    list_row_with_badge(title, trailing, 0, selected, on_press)
}

/// Like [`list_row`], with a pending-count badge before the trailing
/// label when `pending_count > 0`.
pub(crate) fn list_row_with_badge<'a, Message: Clone + 'a>(
    title: &'a str,
    trailing: &'a str,
    pending_count: u64,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let title_color = if selected {
        TEXT_PRIMARY
    } else {
        TEXT_SECONDARY
    };
    let mut row = iced::widget::row![
        container(
            text(title)
                .size(FONT_SIZE_PANEL)
                .wrapping(iced::widget::text::Wrapping::None)
                .color(title_color)
        )
        .width(Length::Fill)
        .clip(true)
    ]
    .spacing(SPACE_S)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill);
    if let Some(badge) = pending_badge::<Message>(pending_count) {
        row = row.push(badge);
    }
    row = row.push(text(trailing).size(FONT_SIZE_PANEL_SMALL).color(TEXT_FAINT));
    button(row)
        .width(Length::Fill)
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_press)
        .style(row_style(selected))
        .into()
}

/// Status-aware style for label rows (section headers, add-row): clear
/// at rest, surface-1 on hover and press, with the label tone shifting
/// from `rest` to `hot` under the pointer.
fn label_row_style(rest: Color, hot: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme: &Theme, status: button::Status| {
        let (background, text_color) = match status {
            button::Status::Hovered | button::Status::Pressed => (Some(SURFACE_1), hot),
            button::Status::Active | button::Status::Disabled => (None, rest),
        };
        button::Style {
            background: background.map(Background::Color),
            text_color,
            border: Border::default().width(0.0).rounded(RADIUS_PANEL),
            ..button::Style::default()
        }
    }
}

/// A project header row: the expand/collapse button fills the row with
/// a small subtle Plus icon button for starting a new session in that
/// project and a small subtle X icon button for removing the project
/// from the registry, pinned to its right end. The whole header stays
/// pressable for expand; the Plus and X are distinct buttons.
pub(crate) fn project_header<'a, Message: Clone + 'a>(
    label: &'a str,
    expanded: bool,
    on_expand: Message,
    on_new: Message,
    on_remove: Message,
) -> Element<'a, Message> {
    iced::widget::row![
        button(
            iced::widget::row![
                icon(folder_icon(expanded), FONT_SIZE_PANEL, TEXT_SECONDARY,),
                container(
                    text(label)
                        .size(FONT_SIZE_PANEL)
                        .wrapping(iced::widget::text::Wrapping::None),
                )
                .width(Length::Fill)
                .clip(true),
            ]
            .spacing(SPACE_XS)
            .align_y(iced::alignment::Vertical::Center)
            .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_expand)
        .style(label_row_style(TEXT_SECONDARY, TEXT_PRIMARY)),
        icon_button(
            lucide_icons::Icon::Plus,
            IconButtonKind::Subtle,
            Some(on_new),
        ),
        icon_button(
            lucide_icons::Icon::X,
            IconButtonKind::Subtle,
            Some(on_remove)
        ),
    ]
    .spacing(SPACE_XXS)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill)
    .into()
}

/// A full-width add row: same shape as the project header with a body
/// label tone, for actions like "New Project". A leading icon renders
/// when supplied.
pub(crate) fn action_row<'a, Message: Clone + 'a>(
    label: &'a str,
    leading_icon: Option<lucide_icons::Icon>,
    on_press: Message,
) -> Element<'a, Message> {
    let content: Element<'a, Message> = match leading_icon {
        Some(leading) => iced::widget::row![
            icon(leading, FONT_SIZE_PANEL, TEXT_SECONDARY),
            text(label).size(FONT_SIZE_PANEL),
        ]
        .spacing(SPACE_XS)
        .align_y(iced::alignment::Vertical::Center)
        .into(),
        None => text(label).size(FONT_SIZE_PANEL).into(),
    };
    button(content)
        .width(Length::Fill)
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_press)
        .style(label_row_style(TEXT_SECONDARY, TEXT_PRIMARY))
        .into()
}

/// A full-width transcript group header: a chevron glyph plus the
/// precomputed headline, expanding or collapsing the run below it.
/// Same shape as the project header: the whole row is the press target.
pub(crate) fn group_header<'a, Message: Clone + 'a>(
    headline: &'a str,
    expanded: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let content = iced::widget::row![
        icon(drawer_chevron(expanded), FONT_SIZE_SMALL, TEXT_TERTIARY),
        text(headline).size(FONT_SIZE_SMALL).color(TEXT_SECONDARY),
    ]
    .spacing(SPACE_S)
    .width(Length::Fill);
    button(content)
        .width(Length::Fill)
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_press)
        .style(label_row_style(TEXT_TERTIARY, TEXT_SECONDARY))
        .into()
}

/// Status-aware style for the modal dim: the scrim token, no border.
fn scrim_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SCRIM)),
        ..container::Style::default()
    }
}

/// Dims the workspace and centers a popup: a full Fill by Fill
/// container carrying the popup in the middle of both axes.
pub(crate) fn scrim<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .center(Length::Fill)
        .style(scrim_style)
        .into()
}

/// Maps the drawer situation to its chevron icon: the drawer grows
/// upward when opened, so the collapsed peek strip shows an up chevron
/// (expand) and the open header shows a down chevron (collapse).
pub(crate) fn drawer_chevron(expanded: bool) -> lucide_icons::Icon {
    if expanded {
        lucide_icons::Icon::ChevronDown
    } else {
        lucide_icons::Icon::ChevronUp
    }
}

/// Maps a project's expansion state to its folder icon: an open
/// folder when expanded and a closed folder when collapsed.
pub(crate) fn folder_icon(expanded: bool) -> lucide_icons::Icon {
    if expanded {
        lucide_icons::Icon::FolderOpen
    } else {
        lucide_icons::Icon::Folder
    }
}

/// Maps the window maximize state to its icon: a square when restored
/// (maximize affordance) and overlapping squares when maximized
/// (restore affordance).
pub(crate) fn maximize_icon(maximized: bool) -> lucide_icons::Icon {
    if maximized {
        lucide_icons::Icon::Copy
    } else {
        lucide_icons::Icon::Square
    }
}

/// The shared Todos bar row: a small label, the current todo in a
/// fill-width container so it bounds the text, the small summary, and
/// the expand/collapse chevron at the right end. Both the peek strip
/// and the expanded drawer header show this row.
fn todo_bar_row<'a, Message: 'a>(
    current: &'a str,
    summary: &'a str,
    expanded: bool,
) -> iced::widget::Row<'a, Message> {
    iced::widget::row![
        text("Todos").size(FONT_SIZE_SMALL).color(TEXT_TERTIARY),
        iced::widget::container(text(current).size(FONT_SIZE_SMALL).color(TEXT_PRIMARY),)
            .width(Length::Fill),
        text(summary).size(FONT_SIZE_SMALL).color(TEXT_TERTIARY),
        icon(drawer_chevron(expanded), FONT_SIZE_SMALL, TEXT_TERTIARY),
    ]
    .spacing(SPACE_S)
    .width(Length::Fill)
}

/// Status-aware style for the peek strip: flat surface-1 fill,
/// surface-2 on hover and press, no border, no shadow.
fn peek_strip_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => SURFACE_2,
        button::Status::Active | button::Status::Disabled => SURFACE_1,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: TEXT_PRIMARY,
        border: Border::default().width(0.0).rounded(RADIUS_PANEL),
        ..button::Style::default()
    }
}

/// The collapsed Todos drawer: a full-width strip with a flat fill.
/// Always rendered while todos exist; pressing it opens the drawer.
pub(crate) fn peek_strip<'a, Message: Clone + 'a>(
    current: &'a str,
    summary: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(todo_bar_row(current, summary, false))
        .width(Length::Fill)
        .padding([SPACE_S, SPACE_M])
        .on_press(on_press)
        .style(peek_strip_style)
        .into()
}

/// Status-aware style for the expanded drawer header: transparent at
/// rest, surface-2 on hover and press, no border, no shadow.
fn drawer_header_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Some(SURFACE_2),
        button::Status::Active | button::Status::Disabled => None,
    };
    button::Style {
        background: background.map(Background::Color),
        text_color: TEXT_PRIMARY,
        border: Border::default().width(0.0).rounded(RADIUS_PANEL),
        ..button::Style::default()
    }
}

/// The expanded drawer's header: the shared row on a transparent
/// button. Pressing it collapses the drawer back to the peek strip.
pub(crate) fn drawer_header<'a, Message: Clone + 'a>(
    current: &'a str,
    summary: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(todo_bar_row(current, summary, true))
        .width(Length::Fill)
        .padding([SPACE_S, SPACE_M])
        .on_press(on_press)
        .style(drawer_header_style)
        .into()
}

/// Status-aware style for the multi-line composer input: the bar
/// surface, no border, faint placeholder, primary value text, and the
/// muted accent selection wash.
fn composer_input_style(_theme: &Theme, _status: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: Background::Color(SURFACE_1),
        border: Border::default().width(0.0).rounded(RADIUS_PANEL),
        placeholder: TEXT_FAINT,
        value: TEXT_PRIMARY,
        selection: ACCENT_MUTED_BG,
    }
}

/// Key interception for the composer input: maps a key press to an
/// editor binding, replacing the editor defaults while installed.
/// The composer installs one only while its slash menu is open; every
/// other key falls back to [`text_editor::Binding::from_key_press`]
/// inside the composer's closure, so the editor behaves exactly as
/// before when the menu is closed.
pub(crate) type ComposerKeys<'a, Message> =
    Box<dyn Fn(text_editor::KeyPress) -> Option<text_editor::Binding<Message>> + 'a>;

/// The multi-line composer input. Its height grows with
/// `visible_lines` up to [`COMPOSER_MAX_LINES`]; past that cap the
/// caller holds the height fixed and the editor scrolls internally,
/// so later slices inherit the growth contract from this constructor.
/// Every exact `/selector` token on every draft line highlights
/// through [`crate::composer::SlashHighlighter`] (no-op while the
/// settings name no valid token); `key_binding` intercepts keys
/// before the editor consumes them, when the caller installs one.
pub(crate) fn composer_input<'a, Message: Clone + 'a>(
    content: &'a text_editor::Content,
    visible_lines: usize,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
    highlight: &crate::composer::SlashHighlightSettings,
    key_binding: Option<ComposerKeys<'a, Message>>,
) -> Element<'a, Message> {
    // Concrete widget generics: pinning the renderer lets the
    // highlight closure below fill its (unnameable) `Format` by
    // inference, since the expected return type is then concrete.
    let mut editor: text_editor::TextEditor<
        '_,
        crate::composer::SlashHighlighter,
        Message,
        iced::Theme,
        iced::Renderer,
    > = text_editor::TextEditor::new(content)
        .placeholder("Message the agent...")
        .on_action(on_action)
        .size(FONT_SIZE_BODY)
        .line_height(COMPOSER_LINE_HEIGHT / FONT_SIZE_BODY)
        .padding(COMPOSER_INPUT_PADDING)
        .height(Length::Fixed(
            visible_lines as f32 * COMPOSER_LINE_HEIGHT + 2.0 * COMPOSER_INPUT_PADDING,
        ))
        .highlight_with::<crate::composer::SlashHighlighter>(
            highlight.clone(),
            // The highlight `Format` type is not nameable through the
            // `iced` facade (its `core` module is hidden), so the
            // closure below fills it by inference: `Default` plus field
            // writes against the style pair, coerced to the `fn`
            // pointer `highlight_with` expects.
            |_highlight, theme| {
                let (color, font) = crate::composer::slash_token_style(theme);
                iced_core::text::highlighter::Format {
                    color: Some(color),
                    font: Some(font),
                }
            },
        );
    if let Some(keys) = key_binding {
        editor = editor.key_binding(keys);
    }
    editor.style(composer_input_style).into()
}

/// Pure data mapping behind the goal inline editor style: the
/// surface-2 step and the border color for an editor status. Kept
/// as data (not `Style`) so the mapping is unit-testable without
/// constructing widgets. The card behind the editor is surface-1,
/// so the editor needs both a stepped fill and a border to read as
/// a field at rest; focus warms the border to the accent, mirroring
/// [`field_style`].
pub(crate) fn goal_editor_colors(status: text_editor::Status) -> (Color, Color) {
    let border = match status {
        text_editor::Status::Focused { .. } => ACCENT_BORDER_SUBTLE,
        _ => BORDER_DEFAULT,
    };
    (SURFACE_2, border)
}

/// Status-aware style for the multi-line goal inline editor: the
/// surface-2 step over its surface-1 card, a thin border in every
/// state (accent on focus), faint placeholder, primary value text,
/// and the muted accent selection wash.
fn goal_editor_style(_theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let (background, border) = goal_editor_colors(status);
    text_editor::Style {
        background: Background::Color(background),
        border: Border::default()
            .color(border)
            .width(1.0)
            .rounded(RADIUS_PANEL),
        placeholder: TEXT_FAINT,
        value: TEXT_PRIMARY,
        selection: ACCENT_MUTED_BG,
    }
}

/// The multi-line goal inline editor: the composer input's line
/// height and padding with an objective placeholder and no slash
/// highlighting, on its own stepped [`goal_editor_style`] surface.
/// Typed sentences soft-wrap at the card width instead of running
/// one line until Enter. The height tracks the laid-out (wrapped)
/// content itself: it grows line by line to
/// [`GOAL_EDITOR_MAX_LINES`], then holds and scrolls internally.
/// `key_binding` intercepts keys before the editor consumes them
/// (Cmd/Ctrl+Enter to save, Esc to cancel).
pub(crate) fn goal_editor<'a, Message: Clone + 'a>(
    content: &'a text_editor::Content,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
    key_binding: ComposerKeys<'a, Message>,
) -> Element<'a, Message> {
    text_editor::TextEditor::new(content)
        .placeholder("Objective")
        .on_action(on_action)
        .size(FONT_SIZE_BODY)
        .line_height(COMPOSER_LINE_HEIGHT / FONT_SIZE_BODY)
        .padding(COMPOSER_INPUT_PADDING)
        .wrapping(iced::widget::text::Wrapping::Word)
        .height(Length::Shrink)
        .min_height(COMPOSER_LINE_HEIGHT + 2.0 * COMPOSER_INPUT_PADDING)
        .max_height(
            GOAL_EDITOR_MAX_LINES as f32 * COMPOSER_LINE_HEIGHT + 2.0 * COMPOSER_INPUT_PADDING,
        )
        .key_binding(key_binding)
        .style(goal_editor_style)
        .into()
}

/// One slash-menu row: a source icon, the `/selector` in strong text,
/// and the description on one truncated line. Selected rows read as
/// the quiet surface-2 step, mirroring [`menu_row`].
pub(crate) fn slash_row<'a, Message: Clone + 'a>(
    source_icon: lucide_icons::Icon,
    selector: &'a str,
    description: &'a str,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let title_color = if selected {
        TEXT_PRIMARY
    } else {
        TEXT_SECONDARY
    };
    let content = iced::widget::row![
        icon(source_icon, FONT_SIZE_PANEL_SMALL, TEXT_TERTIARY),
        container(
            text(selector)
                .size(FONT_SIZE_PANEL)
                .wrapping(iced::widget::text::Wrapping::None)
                .color(title_color),
        )
        .width(Length::Shrink)
        .clip(true),
        container(
            text(description)
                .size(FONT_SIZE_PANEL_SMALL)
                .wrapping(iced::widget::text::Wrapping::None)
                .color(TEXT_TERTIARY),
        )
        .width(Length::Fill)
        .clip(true),
    ]
    .spacing(SPACE_S)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill);
    button(content)
        .width(Length::Fill)
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_press)
        .style(menu_row_style(selected))
        .into()
}

/// The two icon-button treatments for the inside-bar buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconButtonKind {
    Accent,
    Subtle,
}

/// Pure data mapping behind the icon-button styles: background and
/// glyph color for a kind in a status. Kept as data (not `Style`) so
/// the mapping is unit-testable without constructing widgets.
pub(crate) fn icon_button_colors(
    kind: IconButtonKind,
    status: button::Status,
) -> (Option<Color>, Color) {
    match kind {
        IconButtonKind::Accent => {
            let background = match status {
                button::Status::Active => ACCENT_BASE,
                button::Status::Hovered => ACCENT_HOVER,
                button::Status::Pressed => ACCENT_ACTIVE,
                button::Status::Disabled => SURFACE_2,
            };
            let glyph = match status {
                button::Status::Disabled => TEXT_FAINT,
                _ => BG_BASE,
            };
            (Some(background), glyph)
        }
        IconButtonKind::Subtle => {
            let background = match status {
                button::Status::Hovered | button::Status::Pressed => Some(SURFACE_2),
                button::Status::Active | button::Status::Disabled => None,
            };
            let glyph = match status {
                button::Status::Active => TEXT_SECONDARY,
                button::Status::Hovered | button::Status::Pressed => TEXT_PRIMARY,
                button::Status::Disabled => TEXT_FAINT,
            };
            (background, glyph)
        }
    }
}

/// Status-aware style for [`IconButtonKind::Accent`].
fn accent_icon_style(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = icon_button_colors(IconButtonKind::Accent, status);
    // The accent mapping always yields a background; the fallback keeps
    // styling total without a panic path.
    let background = match background {
        Some(color) => color,
        None => SURFACE_2,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border::default().width(0.0).rounded(RADIUS_PILL),
        ..button::Style::default()
    }
}

/// Status-aware style for [`IconButtonKind::Subtle`].
fn subtle_icon_style(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = icon_button_colors(IconButtonKind::Subtle, status);
    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: Border::default().width(0.0).rounded(RADIUS_PILL),
        ..button::Style::default()
    }
}

/// A non-button Lucide icon at a caller-named size and color.
pub(crate) fn icon<'a, Message: 'a>(
    icon: lucide_icons::Icon,
    size: f32,
    color: Color,
) -> Element<'a, Message> {
    text(char::from(icon).to_string())
        .font(iced::Font::with_name("lucide"))
        .size(size)
        .color(color)
        .into()
}

/// A circular icon button: a fixed-square button with the icon
/// centered. Accent follows the primary pill; Subtle is transparent
/// at rest since it sits on the bar surface. `on_press: None` renders
/// disabled and emits no message.
pub(crate) fn icon_button<'a, Message: Clone + 'a>(
    icon: lucide_icons::Icon,
    kind: IconButtonKind,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let style = match kind {
        IconButtonKind::Accent => accent_icon_style,
        IconButtonKind::Subtle => subtle_icon_style,
    };
    button(
        container(
            text(char::from(icon).to_string())
                .font(iced::Font::with_name("lucide"))
                .size(ICON_SIZE),
        )
        .center(Length::Fixed(ICON_BUTTON_SIZE)),
    )
    .width(Length::Fixed(ICON_BUTTON_SIZE))
    .height(Length::Fixed(ICON_BUTTON_SIZE))
    .padding(0.0)
    .on_press_maybe(on_press)
    .style(style)
    .into()
}

/// An icon button with a small tooltip shown above it on hover.
pub(crate) fn tooltip_icon_button<'a, Message: Clone + 'a>(
    icon: lucide_icons::Icon,
    kind: IconButtonKind,
    tooltip: &'a str,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    iced::widget::tooltip(
        icon_button(icon, kind, on_press),
        container(
            text(tooltip)
                .size(FONT_SIZE_PANEL_SMALL)
                .color(TEXT_PRIMARY),
        )
        .padding(SPACE_XS)
        .style(popup_card_style),
        iced::widget::tooltip::Position::Top,
    )
    .into()
}

/// Fixed-height slot centering a composer icon button on one input
/// line, so the button stays centered as the input grows.
pub(crate) fn composer_icon_slot<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .height(Length::Fixed(
            COMPOSER_LINE_HEIGHT + 2.0 * COMPOSER_INPUT_PADDING,
        ))
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

/// A fixture label: static display only, deliberately not a button,
/// for the composer sub-bar values in this slice.
pub(crate) fn static_pill<'a, Message: Clone + 'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(FONT_SIZE_SMALL).color(TEXT_SECONDARY))
        .padding([SPACE_XS, SPACE_S])
        .style(panel_flat_pill_style)
        .into()
}

/// Status-aware style for [`prompt_bubble`]: flat surface-2 fill,
/// no border, panel radius, no shadow.
fn prompt_bubble_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE_2)),
        border: Border::default().width(0.0).rounded(RADIUS_PANEL),
        ..container::Style::default()
    }
}

/// A user prompt bubble: body text on a flat surface-2 fill. Renders
/// both the natural prompt in the scroll list and the sticky header
/// copy, so it takes text only.
pub(crate) fn prompt_bubble<'a, Message: Clone + 'a>(text: &'a str) -> Element<'a, Message> {
    container(
        iced::widget::text(text)
            .size(FONT_SIZE_BODY)
            .color(TEXT_PRIMARY),
    )
    .width(Length::Fill)
    .padding([SPACE_L, SPACE_XL])
    .style(prompt_bubble_style)
    .into()
}

/// Status-aware style for [`sticky_bubble`]: the same flat surface-2
/// fill as [`prompt_bubble_style`], no border, no shadow. While the
/// bubble is pushed (`square_top`), its real top edge sits above the
/// window, so the top corners are square and only the bottom corners
/// keep the panel radius; once settled the full panel radius returns.
///
/// Pinned/settled at the top (`push == 0.0`) the bubble looks exactly
/// like a natural prompt bubble with rounded top corners, clean against
/// the background. While being pushed under the top bar (`push > 0.0`)
/// the top corners are square: the rounded corners are conceptually
/// already hidden under the bar, so the visible top edge is a straight
/// cut.
fn sticky_bubble_style(square_top: bool) -> impl Fn(&Theme) -> container::Style {
    move |_theme: &Theme| container::Style {
        background: Some(Background::Color(SURFACE_2)),
        border: Border::default().width(0.0).rounded(if square_top {
            iced::border::Radius {
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: RADIUS_PANEL,
                bottom_left: RADIUS_PANEL,
            }
        } else {
            RADIUS_PANEL.into()
        }),
        ..container::Style::default()
    }
}

/// Status-aware style for the sticky bubble's notch backdrop: a flat
/// [`BG_BASE`] fill and nothing else, so the layer is invisible against
/// the window background.
fn sticky_backdrop_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_BASE)),
        ..container::Style::default()
    }
}

/// The sticky prompt window: a container sized to exactly the visible
/// slice of the stuck bubble, with its text slid up inside by `push`.
///
/// A styled container quad is never scissored to a clip rect in iced
/// (`container::draw` paints `draw_background` with the container's
/// full bounds whenever they intersect the viewport; only the text
/// clips). So the bubble itself must be the window: its own bounds are
/// the visible slice (`window_height`), and a negative top padding of
/// `SPACE_L - push` performs the slide — the text glyphs clip at the
/// window's top edge via the container's clipped viewport while the
/// background can never paint above it. `push` is 0.0 when settled;
/// positive while the incoming prompt pushes the sticky bubble out.
///
/// The bubble sits in a [`Stack`](iced::widget::Stack) over the
/// scrollable transcript, so its rounded-corner notches are transparent
/// and would show scrolling text slivers through them. An opaque
/// [`BG_BASE`] backdrop layer behind the bubble keeps the corner notches
/// clean over the scrolling list, in both corner modes.
/// Returned as a small [`Stack`](iced::widget::Stack): layer 1 is the
/// backdrop sized to the same window, layer 2 is the bubble itself.
pub(crate) fn sticky_bubble<'a, Message: Clone + 'a>(
    text: &'a str,
    window_height: f32,
    push: f32,
) -> Element<'a, Message> {
    let backdrop = container(iced::widget::text(""))
        .width(Length::Fill)
        .height(Length::Fixed(window_height.max(0.0)))
        .style(sticky_backdrop_style);
    let bubble = container(
        iced::widget::text(text)
            .size(FONT_SIZE_BODY)
            .color(TEXT_PRIMARY),
    )
    .width(Length::Fill)
    .height(Length::Fixed(window_height.max(0.0)))
    .padding(iced::Padding {
        top: SPACE_L - push,
        right: SPACE_XL,
        bottom: SPACE_L,
        left: SPACE_XL,
    })
    .clip(true)
    .style(sticky_bubble_style(push > 0.0));
    iced::widget::Stack::new()
        .push(backdrop)
        .push(bubble)
        .width(Length::Fill)
        .into()
}

/// Status-aware style for [`static_pill`]: flat surface fill, no
/// border, pill radius.
fn panel_flat_pill_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE_1)),
        border: Border::default().width(0.0).rounded(RADIUS_PILL),
        ..container::Style::default()
    }
}

/// The composer input bar: a flat panel-surface container holding the
/// input row, full width with bar padding.
pub(crate) fn input_bar<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding([SPACE_XS, SPACE_S])
        .style(panel_style)
        .into()
}

#[cfg(test)]
mod tests {
    use super::{
        ButtonKind, IconButtonKind, button_colors, drawer_chevron, edit_button_colors, folder_icon,
        goal_editor_colors, icon_button_colors, maximize_icon, menu_row_colors, row_colors,
    };
    use crate::theme::{
        ACCENT_ACTIVE, ACCENT_BASE, ACCENT_BORDER_SUBTLE, ACCENT_HOVER, BG_BASE, BORDER_DEFAULT,
        DANGER, SURFACE_1, SURFACE_2, TEXT_FAINT, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
    };
    use iced::widget::{button, text_editor};

    #[test]
    fn primary_colors_follow_status() {
        assert_eq!(
            button_colors(ButtonKind::Primary, button::Status::Active),
            (Some(ACCENT_BASE), BG_BASE)
        );
        assert_eq!(
            button_colors(ButtonKind::Primary, button::Status::Hovered),
            (Some(ACCENT_HOVER), BG_BASE)
        );
        assert_eq!(
            button_colors(ButtonKind::Primary, button::Status::Pressed),
            (Some(ACCENT_ACTIVE), BG_BASE)
        );
        assert_eq!(
            button_colors(ButtonKind::Primary, button::Status::Disabled),
            (Some(SURFACE_2), TEXT_FAINT)
        );
    }

    #[test]
    fn accent_icon_colors_follow_primary() {
        assert_eq!(
            icon_button_colors(IconButtonKind::Accent, button::Status::Active),
            (Some(ACCENT_BASE), BG_BASE)
        );
        assert_eq!(
            icon_button_colors(IconButtonKind::Accent, button::Status::Hovered),
            (Some(ACCENT_HOVER), BG_BASE)
        );
        assert_eq!(
            icon_button_colors(IconButtonKind::Accent, button::Status::Pressed),
            (Some(ACCENT_ACTIVE), BG_BASE)
        );
        assert_eq!(
            icon_button_colors(IconButtonKind::Accent, button::Status::Disabled),
            (Some(SURFACE_2), TEXT_FAINT)
        );
    }

    #[test]
    fn subtle_icon_colors_stay_transparent_at_rest() {
        assert_eq!(
            icon_button_colors(IconButtonKind::Subtle, button::Status::Active),
            (None, TEXT_SECONDARY)
        );
        assert_eq!(
            icon_button_colors(IconButtonKind::Subtle, button::Status::Hovered),
            (Some(SURFACE_2), TEXT_PRIMARY)
        );
        assert_eq!(
            icon_button_colors(IconButtonKind::Subtle, button::Status::Pressed),
            (Some(SURFACE_2), TEXT_PRIMARY)
        );
        assert_eq!(
            icon_button_colors(IconButtonKind::Subtle, button::Status::Disabled),
            (None, TEXT_FAINT)
        );
    }

    #[test]
    fn row_colors_select_with_a_quiet_surface_step() {
        assert_eq!(
            row_colors(true, button::Status::Active),
            (Some(SURFACE_2), TEXT_PRIMARY)
        );
        assert_eq!(
            row_colors(true, button::Status::Hovered),
            (Some(SURFACE_2), TEXT_PRIMARY)
        );
        assert_eq!(
            row_colors(false, button::Status::Active),
            (None, TEXT_SECONDARY)
        );
        assert_eq!(
            row_colors(false, button::Status::Hovered),
            (Some(SURFACE_1), TEXT_SECONDARY)
        );
        assert_eq!(
            row_colors(false, button::Status::Disabled),
            (None, TEXT_SECONDARY)
        );
    }

    #[test]
    fn menu_row_colors_select_with_a_quiet_surface_step() {
        assert_eq!(
            menu_row_colors(true, button::Status::Active),
            (Some(SURFACE_2), TEXT_PRIMARY)
        );
        assert_eq!(
            menu_row_colors(true, button::Status::Hovered),
            (Some(SURFACE_2), TEXT_PRIMARY)
        );
        assert_eq!(
            menu_row_colors(false, button::Status::Active),
            (None, TEXT_SECONDARY)
        );
        assert_eq!(
            menu_row_colors(false, button::Status::Hovered),
            (Some(SURFACE_1), TEXT_SECONDARY)
        );
        assert_eq!(
            menu_row_colors(false, button::Status::Disabled),
            (None, TEXT_SECONDARY)
        );
    }

    #[test]
    fn edit_button_colors_stay_quiet() {
        assert_eq!(
            edit_button_colors(button::Status::Active),
            (None, TEXT_TERTIARY)
        );
        assert_eq!(
            edit_button_colors(button::Status::Hovered),
            (Some(SURFACE_2), TEXT_PRIMARY)
        );
        assert_eq!(
            edit_button_colors(button::Status::Pressed),
            (Some(SURFACE_2), TEXT_PRIMARY)
        );
        assert_eq!(
            edit_button_colors(button::Status::Disabled),
            (None, TEXT_FAINT)
        );
    }

    #[test]
    fn drawer_chevron_points_up_when_collapsed_and_down_when_open() {
        assert!(matches!(
            drawer_chevron(false),
            lucide_icons::Icon::ChevronUp
        ));
        assert!(matches!(
            drawer_chevron(true),
            lucide_icons::Icon::ChevronDown
        ));
    }

    #[test]
    fn folder_icon_is_closed_when_collapsed_and_open_when_expanded() {
        assert!(matches!(folder_icon(false), lucide_icons::Icon::Folder));
        assert!(matches!(folder_icon(true), lucide_icons::Icon::FolderOpen));
    }

    #[test]
    fn maximize_icon_is_square_when_restored_and_copy_when_maximized() {
        assert!(matches!(maximize_icon(false), lucide_icons::Icon::Square));
        assert!(matches!(maximize_icon(true), lucide_icons::Icon::Copy));
    }

    #[test]
    fn danger_colors_fill_with_the_theme_danger_token() {
        assert_eq!(
            button_colors(ButtonKind::Danger, button::Status::Active),
            (Some(DANGER), BG_BASE)
        );
        assert_eq!(
            button_colors(ButtonKind::Danger, button::Status::Hovered),
            (Some(DANGER), BG_BASE)
        );
        assert_eq!(
            button_colors(ButtonKind::Danger, button::Status::Pressed),
            (Some(DANGER), BG_BASE)
        );
        assert_eq!(
            button_colors(ButtonKind::Danger, button::Status::Disabled),
            (Some(SURFACE_2), TEXT_FAINT)
        );
    }

    #[test]
    fn danger_ghost_colors_keep_danger_text_on_ghost_backgrounds() {
        assert_eq!(
            button_colors(ButtonKind::DangerGhost, button::Status::Active),
            (None, DANGER)
        );
        assert_eq!(
            button_colors(ButtonKind::DangerGhost, button::Status::Hovered),
            (Some(SURFACE_1), DANGER)
        );
        assert_eq!(
            button_colors(ButtonKind::DangerGhost, button::Status::Pressed),
            (Some(SURFACE_2), DANGER)
        );
        assert_eq!(
            button_colors(ButtonKind::DangerGhost, button::Status::Disabled),
            (None, TEXT_FAINT)
        );
    }

    #[test]
    fn pager_chrome_hidden_for_single_question() {
        assert!(!super::pager_visible(0));
        assert!(!super::pager_visible(1));
        assert!(super::pager_visible(2));
        assert_eq!(super::pager_label(0, 1), "");
        assert_eq!(super::pager_label(1, 3), "2 of 3");
    }

    #[test]
    fn pending_badge_label_hides_at_zero() {
        assert_eq!(super::pending_badge_label(0), None);
        assert_eq!(super::pending_badge_label(2).as_deref(), Some("2 pending"));
    }

    #[test]
    fn todo_status_icon_maps_each_status() {
        use manifest_domain::TodoStatus;
        assert!(matches!(
            super::todo_status_icon(TodoStatus::Done),
            lucide_icons::Icon::CircleCheck
        ));
        assert!(matches!(
            super::todo_status_icon(TodoStatus::InProgress),
            lucide_icons::Icon::CirclePlay
        ));
        assert!(matches!(
            super::todo_status_icon(TodoStatus::Pending),
            lucide_icons::Icon::Circle
        ));
        assert!(matches!(
            super::todo_status_icon(TodoStatus::Cancelled),
            lucide_icons::Icon::Circle
        ));
    }

    #[test]
    fn selection_icon_distinguishes_radio_and_checkbox() {
        assert!(matches!(
            super::selection_icon(true, false),
            lucide_icons::Icon::CircleDot
        ));
        assert!(matches!(
            super::selection_icon(false, false),
            lucide_icons::Icon::Circle
        ));
        assert!(matches!(
            super::selection_icon(true, true),
            lucide_icons::Icon::SquareCheck
        ));
        assert!(matches!(
            super::selection_icon(false, true),
            lucide_icons::Icon::Square
        ));
    }

    #[test]
    fn ring_fraction_clamps_sweep() {
        assert_eq!(super::ring_fraction(None), None);
        assert_eq!(super::ring_fraction(Some(0.0)), Some(0.0));
        assert_eq!(super::ring_fraction(Some(29.0)), Some(0.29));
        assert_eq!(super::ring_fraction(Some(100.0)), Some(1.0));
        assert_eq!(super::ring_fraction(Some(140.0)), Some(1.0));
        assert_eq!(super::ring_fraction(Some(-5.0)), Some(0.0));
    }

    #[test]
    fn goal_ring_presentation_tones_each_status() {
        use manifest_domain::GoalStatus;
        let (icon, arc, glyph) = super::goal_ring_presentation(&GoalStatus::Active);
        assert!(matches!(icon, lucide_icons::Icon::Target));
        assert_eq!(arc, crate::theme::ACCENT_BASE);
        assert_eq!(glyph, crate::theme::TEXT_SECONDARY);
        let (icon, arc, glyph) = super::goal_ring_presentation(&GoalStatus::Paused);
        assert!(matches!(icon, lucide_icons::Icon::Target));
        assert_eq!(arc, crate::theme::TEXT_FAINT);
        assert_eq!(glyph, crate::theme::TEXT_TERTIARY);
        let (icon, arc, glyph) = super::goal_ring_presentation(&GoalStatus::Completed);
        assert!(matches!(icon, lucide_icons::Icon::CircleCheck));
        assert_eq!(arc, crate::theme::SUCCESS);
        assert_eq!(glyph, crate::theme::SUCCESS);
        let (icon, arc, glyph) =
            super::goal_ring_presentation(&GoalStatus::Other(String::from("Reviewing")));
        assert!(matches!(icon, lucide_icons::Icon::Target));
        assert_eq!(arc, crate::theme::ACCENT_BASE);
        assert_eq!(glyph, crate::theme::TEXT_SECONDARY);
        let (icon, _, _) = super::goal_ring_presentation(&GoalStatus::Other(String::new()));
        assert!(matches!(icon, lucide_icons::Icon::Target));
    }

    #[test]
    fn ghost_colors_follow_status() {
        assert_eq!(
            button_colors(ButtonKind::Ghost, button::Status::Active),
            (None, TEXT_SECONDARY)
        );
        assert_eq!(
            button_colors(ButtonKind::Ghost, button::Status::Hovered),
            (Some(SURFACE_1), TEXT_PRIMARY)
        );
        assert_eq!(
            button_colors(ButtonKind::Ghost, button::Status::Disabled),
            (None, TEXT_FAINT)
        );
    }

    #[test]
    fn goal_editor_reads_as_a_field_at_rest() {
        // The card behind the editor is surface-1, so the editor
        // steps to surface-2 with a border in every state — it never
        // blends into the card, even unfocused.
        assert_eq!(
            goal_editor_colors(text_editor::Status::Active),
            (SURFACE_2, BORDER_DEFAULT)
        );
        assert_eq!(
            goal_editor_colors(text_editor::Status::Hovered),
            (SURFACE_2, BORDER_DEFAULT)
        );
        assert_eq!(
            goal_editor_colors(text_editor::Status::Focused { is_hovered: false }),
            (SURFACE_2, ACCENT_BORDER_SUBTLE)
        );
        assert_eq!(
            goal_editor_colors(text_editor::Status::Focused { is_hovered: true }),
            (SURFACE_2, ACCENT_BORDER_SUBTLE)
        );
    }
}
