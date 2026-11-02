//! Manifest widget kit: every styled widget the shell composes from.
//!
//! All widget styling decisions live here (or in [`crate::theme`]);
//! feature call sites use these constructors and never style widgets
//! themselves (UI contract §5, plan D5). Colors, spacing, radii, and
//! type sizes come from [`crate::theme`] tokens only — no literals here.
//!
//! Surfaces are borderless: panels, rows, chips, and buttons keep
//! their rounded corners but carry no border width, so state reads
//! through flat background fills alone. The single-line text field
//! is the one exception — its border marks where typing lands,
//! an affordance rather than a card edge.
//!
use iced::widget::{button, container, text, text_editor, text_input};
use iced::{Background, Border, Color, Element, Length, Theme};

use crate::theme::{
    ACCENT_ACTIVE, ACCENT_BASE, ACCENT_BORDER_SUBTLE, ACCENT_HOVER, ACCENT_MUTED_BG, BG_BASE,
    BG_SUBTLE, BORDER_DEFAULT, BORDER_SUBTLE, COMPOSER_LINE_HEIGHT, DIVIDER_HIT_WIDTH,
    FONT_SIZE_BODY, FONT_SIZE_HEADING, FONT_SIZE_PANEL, FONT_SIZE_PANEL_SMALL, FONT_SIZE_SMALL,
    ICON_BUTTON_SIZE, RADIUS_PANEL, RADIUS_PILL, SCRIM, SPACE_L, SPACE_M, SPACE_S, SPACE_XL,
    SPACE_XS, SPACE_XXS, SURFACE_1, SURFACE_2, TEXT_FAINT, TEXT_PRIMARY, TEXT_SECONDARY,
    TEXT_TERTIARY,
};

/// The two pill-button treatments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    Primary,
    Ghost,
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
    };
    button(text(label))
        .padding(SPACE_S)
        .on_press_maybe(on_press)
        .style(style)
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
pub(crate) fn list_row<'a, Message: Clone + 'a>(
    title: &'a str,
    trailing: &'a str,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let title_color = if selected {
        TEXT_PRIMARY
    } else {
        TEXT_SECONDARY
    };
    let content = iced::widget::row![
        container(
            text(title)
                .size(FONT_SIZE_PANEL)
                .wrapping(iced::widget::text::Wrapping::None)
                .color(title_color)
        )
        .width(Length::Fill)
        .clip(true),
        text(trailing).size(FONT_SIZE_PANEL_SMALL).color(TEXT_FAINT),
    ]
    .spacing(SPACE_S)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill);
    button(content)
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
/// a small subtle "+" button pinned to its right end for starting a
/// new session in that project. The whole header stays pressable for
/// expand; the "+" is a distinct button.
pub(crate) fn project_header<'a, Message: Clone + 'a>(
    label: &'a str,
    on_expand: Message,
    on_new: Message,
) -> Element<'a, Message> {
    iced::widget::row![
        button(
            container(
                text(label)
                    .size(FONT_SIZE_PANEL)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .width(Length::Fill)
            .clip(true),
        )
        .width(Length::Fill)
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_expand)
        .style(label_row_style(TEXT_SECONDARY, TEXT_PRIMARY)),
        icon_button("+", IconButtonKind::Subtle, Some(on_new)),
    ]
    .spacing(SPACE_XXS)
    .align_y(iced::alignment::Vertical::Center)
    .width(Length::Fill)
    .into()
}

/// A full-width add row: same shape as the project header with a body
/// label tone, for actions like "+ New Project".
pub(crate) fn action_row<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(text(label).size(FONT_SIZE_PANEL))
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
        text(drawer_chevron(expanded))
            .size(FONT_SIZE_SMALL)
            .color(TEXT_TERTIARY),
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

/// Maps the drawer situation to its chevron glyph: the drawer grows
/// upward when opened, so the collapsed peek strip shows an up chevron
/// (expand) and the open header shows a down chevron (collapse).
pub(crate) fn drawer_chevron(expanded: bool) -> &'static str {
    if expanded { "▾" } else { "▴" }
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
        text(drawer_chevron(expanded))
            .size(FONT_SIZE_SMALL)
            .color(TEXT_TERTIARY),
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

/// The multi-line composer input. Its height grows with
/// `visible_lines` up to [`COMPOSER_MAX_LINES`]; past that cap the
/// caller holds the height fixed and the editor scrolls internally,
/// so later slices inherit the growth contract from this constructor.
pub(crate) fn composer_input<'a, Message: Clone + 'a>(
    content: &'a text_editor::Content,
    visible_lines: usize,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
) -> Element<'a, Message> {
    text_editor::TextEditor::new(content)
        .placeholder("Message the agent...")
        .on_action(on_action)
        .size(FONT_SIZE_BODY)
        .line_height(COMPOSER_LINE_HEIGHT / FONT_SIZE_BODY)
        .padding(SPACE_XS)
        .height(Length::Fixed(
            visible_lines as f32 * COMPOSER_LINE_HEIGHT + 2.0 * SPACE_XS,
        ))
        .style(composer_input_style)
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

/// A circular icon button: a fixed-square button with the glyph
/// centered. Accent follows the primary pill; Subtle is transparent
/// at rest since it sits on the bar surface. `on_press: None` renders
/// disabled and emits no message.
pub(crate) fn icon_button<'a, Message: Clone + 'a>(
    glyph: &'a str,
    kind: IconButtonKind,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let style = match kind {
        IconButtonKind::Accent => accent_icon_style,
        IconButtonKind::Subtle => subtle_icon_style,
    };
    button(container(text(glyph).size(FONT_SIZE_HEADING)).center(Length::Fixed(ICON_BUTTON_SIZE)))
        .width(Length::Fixed(ICON_BUTTON_SIZE))
        .height(Length::Fixed(ICON_BUTTON_SIZE))
        .padding(0.0)
        .on_press_maybe(on_press)
        .style(style)
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
        ButtonKind, IconButtonKind, button_colors, drawer_chevron, icon_button_colors, row_colors,
    };
    use crate::theme::{
        ACCENT_ACTIVE, ACCENT_BASE, ACCENT_HOVER, BG_BASE, SURFACE_1, SURFACE_2, TEXT_FAINT,
        TEXT_PRIMARY, TEXT_SECONDARY,
    };
    use iced::widget::button;

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
    fn drawer_chevron_points_up_when_collapsed_and_down_when_open() {
        assert_eq!(drawer_chevron(false), "▴");
        assert_eq!(drawer_chevron(true), "▾");
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
}
