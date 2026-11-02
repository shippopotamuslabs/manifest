//! Manifest widget kit: every styled widget the shell composes from.
//!
//! All widget styling decisions live here (or in [`crate::theme`]);
//! feature call sites use these constructors and never style widgets
//! themselves (UI contract §5, plan D5). Colors, spacing, radii, and
//! type sizes come from [`crate::theme`] tokens only — no literals here.
//!
use iced::widget::{button, container, text, text_input};
use iced::{Background, Border, Color, Element, Length, Theme};

use crate::theme::{
    ACCENT_ACTIVE, ACCENT_BASE, ACCENT_BORDER_SUBTLE, ACCENT_HOVER, ACCENT_MUTED_BG, BG_BASE,
    BORDER_DEFAULT, BORDER_SUBTLE, FONT_SIZE_BODY, FONT_SIZE_SMALL, RADIUS_PANEL, RADIUS_PILL,
    SPACE_M, SPACE_S, SPACE_XS, SURFACE_1, SURFACE_2, TEXT_FAINT, TEXT_HIGH, TEXT_PRIMARY,
    TEXT_SECONDARY, TEXT_TERTIARY,
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
        border: Border::default()
            .color(background)
            .width(1.0)
            .rounded(RADIUS_PILL),
        ..button::Style::default()
    }
}

/// Status-aware style for [`ButtonKind::Ghost`].
fn ghost_style(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = button_colors(ButtonKind::Ghost, status);
    let border_color = match status {
        button::Status::Disabled => BORDER_SUBTLE,
        _ => BORDER_DEFAULT,
    };
    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: Border::default()
            .color(border_color)
            .width(1.0)
            .rounded(RADIUS_PILL),
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

/// A panel: surface-1 fill, default border, panel radius, medium padding.
fn panel_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE_1)),
        border: Border::default()
            .color(BORDER_DEFAULT)
            .width(1.0)
            .rounded(RADIUS_PANEL),
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

/// A [`panel`] that stretches to the full height of its parent column.
/// Used for full-column panels (e.g. the session list); item panels
/// inside scrollables must keep shrink height, so they stay on [`panel`].
pub(crate) fn panel_fill<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .padding(SPACE_M)
        .width(Length::Fill)
        .height(Length::Fill)
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
        let border_color = if selected {
            ACCENT_BORDER_SUBTLE
        } else {
            BORDER_DEFAULT
        };
        button::Style {
            background: background.map(Background::Color),
            text_color: TEXT_PRIMARY,
            border: Border::default()
                .color(border_color)
                .width(1.0)
                .rounded(RADIUS_PANEL),
            ..button::Style::default()
        }
    }
}

/// A full-width session row: title plus a small subtitle. Selected rows
/// sit on the muted accent wash; unselected rows are transparent until
/// hovered.
pub(crate) fn list_row<'a, Message: Clone + 'a>(
    title: &'a str,
    subtitle: &'a str,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let title_color = if selected { TEXT_HIGH } else { TEXT_PRIMARY };
    let content = iced::widget::column![
        text(title).size(FONT_SIZE_BODY).color(title_color),
        text(subtitle).size(FONT_SIZE_SMALL).color(TEXT_TERTIARY),
    ]
    .spacing(SPACE_XS)
    .width(Length::Fill);
    button(content)
        .width(Length::Fill)
        .padding(SPACE_S)
        .on_press(on_press)
        .style(row_style(selected))
        .into()
}

/// Status-aware style for a toggle chip.
fn chip_style(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme: &Theme, status: button::Status| {
        let (background, border_color, text_color) = if on {
            (
                Some(ACCENT_MUTED_BG),
                ACCENT_BORDER_SUBTLE,
                TEXT_HIGH,
            )
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
            (background, BORDER_DEFAULT, text_color)
        };
        button::Style {
            background: background.map(Background::Color),
            text_color,
            border: Border::default()
                .color(border_color)
                .width(1.0)
                .rounded(RADIUS_PILL),
            ..button::Style::default()
        }
    }
}

/// A pill toggle chip (a button, not a switch): the on state carries the
/// muted accent wash with an accent border.
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
