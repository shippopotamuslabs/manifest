//! Manifest visual tokens and custom Iced theme.
//!
//! Every color, spacing, radius, and type size in the shell lives here
//! exactly once (contract §5, plan D4). Values seed from
//! `contracts/gospel/PALETTE.md`. Feature and kit code reference these
//! tokens; no hex literal or `Color` constructor appears anywhere else.

use iced::theme::Palette;
use iced::{Color, Theme};

// Items marked `#[allow(dead_code)]` below are D4 palette tokens with no
// M1 consumer yet (later phases: focus rings, deeper surfaces, larger
// rhythm steps). They stay defined once here per contract §5; the allow
// is per-item so any newly dead non-token code still fails the clippy
// gate. Revisit when the next feature lands.

// Background layers.
pub const BG_BASE: Color = Color::from_rgb8(0x0D, 0x0E, 0x11);
pub const BG_SUBTLE: Color = Color::from_rgb8(0x11, 0x12, 0x16);
pub const SURFACE_1: Color = Color::from_rgb8(0x18, 0x1A, 0x20);
pub const SURFACE_2: Color = Color::from_rgb8(0x1F, 0x22, 0x2A);
#[allow(dead_code)]
pub const SURFACE_3: Color = Color::from_rgb8(0x27, 0x2A, 0x33);

// Borders and dividers.
pub const BORDER_DEFAULT: Color = Color::from_rgb8(0x2A, 0x2E, 0x39);
#[allow(dead_code)]
pub const BORDER_STRONG: Color = Color::from_rgb8(0x34, 0x3A, 0x48);
pub const BORDER_SUBTLE: Color = Color::from_rgb8(0x20, 0x22, 0x2E);

// Text ramp.
pub const TEXT_PRIMARY: Color = Color::from_rgb8(0xED, 0xEE, 0xF0);
#[allow(dead_code)]
pub const TEXT_HIGH: Color = Color::from_rgb8(0xC5, 0xC7, 0xDD);
pub const TEXT_SECONDARY: Color = Color::from_rgb8(0xA1, 0xA5, 0xB0);
pub const TEXT_TERTIARY: Color = Color::from_rgb8(0x85, 0x89, 0x94);
pub const TEXT_FAINT: Color = Color::from_rgb8(0x6E, 0x72, 0x80);

// Indigo accent family.
pub const ACCENT_BASE: Color = Color::from_rgb8(0x7A, 0x7C, 0xFF);
pub const ACCENT_HOVER: Color = Color::from_rgb8(0x8D, 0x8F, 0xFF);
pub const ACCENT_ACTIVE: Color = Color::from_rgb8(0x5F, 0x62, 0xB9);
pub const ACCENT_BORDER_SUBTLE: Color = Color::from_rgb8(0x4B, 0x4E, 0xBF);
#[allow(dead_code)]
pub const ACCENT_DEEP: Color = Color::from_rgb8(0x3B, 0x3B, 0xAA);
pub const ACCENT_MUTED_BG: Color = with_alpha(ACCENT_BASE, 0.15);
#[allow(dead_code)]
pub const FOCUS_RING: Color = with_alpha(ACCENT_BASE, 0.40);

// Semantics.
pub const SUCCESS: Color = Color::from_rgb8(0x7D, 0xBE, 0x8C);
pub const WARNING: Color = Color::from_rgb8(0xD9, 0xB0, 0x7A);
pub const DANGER: Color = Color::from_rgb8(0xE0, 0x7A, 0x7A);
#[allow(dead_code)]
pub const INFO: Color = Color::from_rgb8(0x7B, 0xB5, 0xD6);

// Spacing scale in logical pixels.
#[allow(dead_code)]
pub const SPACE_XXS: f32 = 2.0;
pub const SPACE_XS: f32 = 4.0;
pub const SPACE_S: f32 = 8.0;
pub const SPACE_M: f32 = 12.0;
pub const SPACE_L: f32 = 16.0;
pub const SPACE_XL: f32 = 24.0;
#[allow(dead_code)]
pub const SPACE_XXL: f32 = 32.0;

// Radii in logical pixels. Pills read as fully rounded at any height
// the shell uses; chunk 3 confirms against `Border::rounded`.
pub const RADIUS_PANEL: f32 = 8.0;
pub const RADIUS_PILL: f32 = 999.0;

// Type sizes in logical pixels.
// Dense panel sizes for the left session list: single-line rows and
// quiet headers (the panel restyle). Transcript, composer, and topbar
// keep the body sizes below.
pub const FONT_SIZE_PANEL: f32 = 14.0;
pub const FONT_SIZE_PANEL_SMALL: f32 = 12.0;
pub const FONT_SIZE_SMALL: f32 = 15.0;
pub const FONT_SIZE_BODY: f32 = 18.0;
pub const FONT_SIZE_HEADING: f32 = 20.0;
pub const FONT_SIZE_TITLE: f32 = 25.0;
// Lucide icon glyph size; single rescale point for all kit icons.
pub const ICON_SIZE: f32 = 20.0;

// Modal dim over the workspace while a popup is open.
pub const SCRIM: Color = with_alpha(BG_BASE, 0.65);
// Composer input geometry: visible line cap, per-line height, and the
// fixed square of the inside-bar icon buttons.
pub const COMPOSER_MAX_LINES: usize = 5;
pub const COMPOSER_LINE_HEIGHT: f32 = 27.0;
pub const ICON_BUTTON_SIZE: f32 = 36.0;
// Context ring geometry: the clickable usage ring at the sub-bar's
// right end. A fixed square with a thin track circle plus a progress
// arc; kit reads both tokens, call sites name no value.
pub const CONTEXT_RING_SIZE: f32 = 18.0;
pub const CONTEXT_RING_STROKE: f32 = 2.0;
// Composer input text padding: triple the bar's tight step, so the text
// breathes inside the bar. Kit reads this token; call sites name no value.
pub const COMPOSER_INPUT_PADDING: f32 = 12.0;
// Fixed width of the add-project popup.
pub const POPUP_WIDTH: f32 = 480.0;
// Fixed width of the floating permission popup. The widest row is the
// "Yolo" pill beside its one-line description ("Runs tools without
// asking. The sandbox stays on.") at FONT_SIZE_SMALL: the short pill,
// the row spacing, and that description fit on one line inside the
// card padding at the same scale as POPUP_WIDTH.
pub const PERMISSION_POPUP_WIDTH: f32 = 480.0;
// Fixed widths of the composer model picker cards: the model card is
// narrow and anchored at the right edge, and the effort card sits to
// its left at a smaller width.
pub const MODEL_POPUP_WIDTH: f32 = 340.0;
pub const EFFORT_POPUP_WIDTH: f32 = 210.0;
// Bottom offset of the floating model menu from the window's bottom
// edge: the main column's bottom padding (SPACE_M) below the composer,
// plus the sub-bar pill height (its SPACE_S top and bottom padding plus
// one body text line), plus a SPACE_S gap between the sub-bar and the
// floating card. The App anchors the overlay with this token so the
// card sits just above the "model · effort" pill without reflowing
// the composer beneath it.
pub const MODEL_MENU_BOTTOM_OFFSET: f32 = SPACE_M + (SPACE_S * 2.0 + FONT_SIZE_BODY) + SPACE_S;
// Fixed width of the floating attach popup. The card only carries
// icon-and-short-label rows ("Files", "Photos") plus a one-line note,
// so it stays narrower than the permission and model cards.
pub const ATTACH_POPUP_WIDTH: f32 = 260.0;
// Fixed width of the floating context popup. It is on the same scale
// as the model card (MODEL_POPUP_WIDTH): it carries the context meter
// plus its one-line breakdown.
pub const CONTEXT_POPUP_WIDTH: f32 = 340.0;

/// Bottom offset of the floating attach menu from the window's bottom
/// edge for a composer showing `visible_lines` input lines. The attach
/// trigger (Plus) sits in the input bar, which is above the sub-bar,
/// so the card must float higher than the model/permission cards:
/// their [`MODEL_MENU_BOTTOM_OFFSET`] (bottom padding, sub-bar height,
/// and the gap above the sub-bar), plus the input-bar height — the
/// input itself (`visible_lines` line heights plus its top and bottom
/// padding, as `kit::composer_input` computes it) plus the input bar's
/// own top and bottom padding (`SPACE_XS` each side, from
/// `kit::input_bar`) — plus a `SPACE_S` gap between the input bar and
/// the floating card.
pub fn attach_menu_bottom_offset(visible_lines: usize) -> f32 {
    MODEL_MENU_BOTTOM_OFFSET
        + (visible_lines as f32 * COMPOSER_LINE_HEIGHT
            + 2.0 * COMPOSER_INPUT_PADDING
            + 2.0 * SPACE_XS)
        + SPACE_S
}
// Fixed width of the floating slash-menu card above the composer.
// The card carries a source icon, the selector, and a one-line
// description per row, so it runs wider than the model card.
pub const SLASH_POPUP_WIDTH: f32 = 420.0;
// Fixed max height of the scrollable catalog list inside the model
// card, so a long catalog never grows the popup unbounded.
pub const MODEL_LIST_MAX_HEIGHT: f32 = 240.0;
// Fixed height of a popup menu row (model and effort rows), so the
// catalog list height can be derived from its row count.
pub const MENU_ROW_HEIGHT: f32 = 28.0;
// Draggable side-panel geometry: default width with hard bounds, plus
// the invisible grab width of the divider hit zone.
pub const PANEL_WIDTH_DEFAULT: f32 = 260.0;
pub const PANEL_WIDTH_MIN: f32 = 180.0;
pub const PANEL_WIDTH_MAX: f32 = 480.0;
pub const DIVIDER_HIT_WIDTH: f32 = 9.0;
// Square edge of the window resize corner handles.
pub const WINDOW_RESIZE_HANDLE: f32 = 12.0;

/// Returns `base` with its alpha channel replaced.
pub const fn with_alpha(base: Color, alpha: f32) -> Color {
    Color {
        r: base.r,
        g: base.g,
        b: base.b,
        a: alpha,
    }
}

/// The Manifest dark theme: the canonical palette as an Iced theme.
pub fn manifest_theme() -> Theme {
    Theme::custom(
        "Manifest",
        Palette {
            background: BG_BASE,
            text: TEXT_PRIMARY,
            primary: ACCENT_BASE,
            success: SUCCESS,
            warning: WARNING,
            danger: DANGER,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{
        COMPOSER_INPUT_PADDING, COMPOSER_LINE_HEIGHT, MODEL_MENU_BOTTOM_OFFSET, SPACE_S, SPACE_XS,
        attach_menu_bottom_offset, with_alpha,
    };
    use iced::Color;

    #[test]
    fn alpha_derivation_keeps_rgb_and_sets_alpha() {
        let base = Color::from_rgb8(0x7A, 0x7C, 0xFF);
        let derived = with_alpha(base, 0.15);
        assert_eq!(derived.r, base.r);
        assert_eq!(derived.g, base.g);
        assert_eq!(derived.b, base.b);
        assert_eq!(derived.a, 0.15);
    }

    #[test]
    fn attach_offset_floats_above_the_input_bar() {
        let input_bar = |lines: usize| {
            lines as f32 * COMPOSER_LINE_HEIGHT + 2.0 * COMPOSER_INPUT_PADDING + 2.0 * SPACE_XS
        };
        assert_eq!(
            attach_menu_bottom_offset(1),
            MODEL_MENU_BOTTOM_OFFSET + input_bar(1) + SPACE_S
        );
        assert_eq!(
            attach_menu_bottom_offset(3),
            MODEL_MENU_BOTTOM_OFFSET + input_bar(3) + SPACE_S
        );
    }
}
