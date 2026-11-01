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
use iced::widget::{button, container, text, text_input};
use iced::{Background, Border, Color, Element, Length, Theme};

use crate::theme::{
    ACCENT_ACTIVE, ACCENT_BASE, ACCENT_BORDER_SUBTLE, ACCENT_HOVER, ACCENT_MUTED_BG, BG_BASE,
    BORDER_DEFAULT, FONT_SIZE_BODY, FONT_SIZE_SMALL, RADIUS_PANEL, RADIUS_PILL, SCRIM, SPACE_M,
    SPACE_S, SPACE_XS, SURFACE_1, SURFACE_2, TEXT_FAINT, TEXT_HIGH, TEXT_PRIMARY, TEXT_SECONDARY,
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

/// Status-aware style for a session list row.
fn row_style(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme: &Theme, status: button::Status| {
        let background = if selected {
            Some(ACCENT_MUTED_BG)
        } else {
            match status {
                button::Status::Hovered | button::Status::Pressed => Some(SURFACE_1),
                button::Status::Active | button::Status::Disabled => None,
            }
        };
        button::Style {
            background: background.map(Background::Color),
            text_color: TEXT_PRIMARY,
            border: Border::default().width(0.0).rounded(RADIUS_PANEL),
            ..button::Style::default()
        }
    }
}

/// A full-width session row: title plus a small subtitle, with a faint
/// trailing label pinned to the right edge. Selected rows sit on the
/// muted accent wash; unselected rows are transparent until hovered.
pub(crate) fn list_row<'a, Message: Clone + 'a>(
    title: &'a str,
    subtitle: &'a str,
    trailing: &'a str,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let title_color = if selected { TEXT_HIGH } else { TEXT_PRIMARY };
    let body = iced::widget::column![
        text(title).size(FONT_SIZE_BODY).color(title_color),
        text(subtitle).size(FONT_SIZE_SMALL).color(TEXT_TERTIARY),
    ]
    .spacing(SPACE_XS)
    .width(Length::Fill);
    let content = iced::widget::row![body, text(trailing).size(FONT_SIZE_SMALL).color(TEXT_FAINT),]
        .spacing(SPACE_S)
        .width(Length::Fill);
    button(content)
        .width(Length::Fill)
        .padding(SPACE_S)
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

/// A full-width project section header: a small label button that
/// expands or collapses the group below it.
pub(crate) fn section_header<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(text(label).size(FONT_SIZE_SMALL))
        .width(Length::Fill)
        .padding([SPACE_XS, SPACE_S])
        .on_press(on_press)
        .style(label_row_style(TEXT_TERTIARY, TEXT_SECONDARY))
        .into()
}

/// A full-width add row: same shape as [`section_header`] with a body
/// label tone, for actions like "+ New Project".
pub(crate) fn action_row<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(text(label).size(FONT_SIZE_BODY))
        .width(Length::Fill)
        .padding([SPACE_S, SPACE_S])
        .on_press(on_press)
        .style(label_row_style(TEXT_SECONDARY, TEXT_PRIMARY))
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

/// Status-aware style for a toggle chip.
fn chip_style(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme: &Theme, status: button::Status| {
        let (background, text_color) = if on {
            (Some(ACCENT_MUTED_BG), TEXT_HIGH)
        } else {
            let background = match status {
                button::Status::Hovered | button::Status::Pressed => Some(SURFACE_1),
                button::Status::Active | button::Status::Disabled => None,
            };
            let text_color = match status {
                button::Status::Hovered | button::Status::Pressed => TEXT_PRIMARY,
                button::Status::Active => TEXT_SECONDARY,
                button::Status::Disabled => TEXT_FAINT,
            };
            (background, text_color)
        };
        button::Style {
            background: background.map(Background::Color),
            text_color,
            border: Border::default().width(0.0).rounded(RADIUS_PILL),
            ..button::Style::default()
        }
    }
}

/// A pill toggle chip (a button, not a switch): the on state carries the
/// muted accent wash fill and nothing else.
pub(crate) fn toggle_chip<'a, Message: Clone + 'a>(
    label: &'a str,
    on: bool,
    on_toggle: Message,
) -> Element<'a, Message> {
    button(text(label).size(FONT_SIZE_SMALL))
        .padding(SPACE_S)
        .on_press(on_toggle)
        .style(chip_style(on))
        .into()
}

/// The shared Todos bar row: a small label, the current todo in a
/// fill-width container so it bounds the text, and the small summary.
/// Both the peek strip and the expanded drawer header show this row.
fn todo_bar_row<'a, Message: 'a>(
    current: &'a str,
    summary: &'a str,
) -> iced::widget::Row<'a, Message> {
    iced::widget::row![
        text("Todos").size(FONT_SIZE_SMALL).color(TEXT_TERTIARY),
        iced::widget::container(text(current).size(FONT_SIZE_SMALL).color(TEXT_PRIMARY),)
            .width(Length::Fill),
        text(summary).size(FONT_SIZE_SMALL).color(TEXT_TERTIARY),
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
    button(todo_bar_row(current, summary))
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
    button(todo_bar_row(current, summary))
        .width(Length::Fill)
        .padding([SPACE_S, SPACE_M])
        .on_press(on_press)
        .style(drawer_header_style)
        .into()
}

#[cfg(test)]
mod tests {
    use super::{ButtonKind, button_colors};
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
