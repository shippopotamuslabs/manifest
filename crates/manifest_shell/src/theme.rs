//! Manifest visual tokens and custom Iced theme.
//!
//! Every color, spacing, radius, and type size in the shell lives here
//! exactly once (contract §5, plan D4). Values seed from
//! `contracts/gospel/PALETTE.md`. Feature and kit code reference these
//! tokens; no hex literal or `Color` constructor appears anywhere else.

use iced::{Color, Theme};
use iced::theme::Palette;

// Items marked `#[allow(dead_code)]` below are D4 palette tokens with no
// M1 consumer yet (later phases: focus rings, deeper surfaces, larger
// rhythm steps). They stay defined once here per contract §5; the allow
// is per-item so any newly dead non-token code still fails the clippy
// gate. Revisit when the next feature lands.

// Background layers.
pub const BG_BASE: Color = Color::from_rgb8(0x0D, 0x0E, 0x11);
#[allow(dead_code)]
pub const BG_SUBTLE: Color = Color::from_rgb8(0x11, 0x12, 0x16);
pub const SURFACE_1: Color = Color::from_rgb8(0x18, 0x1A, 0x20);
pub const SURFACE_2: Color = Color::from_rgb8(0x1F, 0x22, 0x2A);
#[allow(dead_code)]
pub const SURFACE_3: Color = Color::from_rgb8(0x27, 0x2A, 0x33);

// Borders and dividers.
pub const BORDER_DEFAULT: Color = Color::from_rgb8(0x2A, 0x2E, 0x39);
#[allow(dead_code)]
pub const BORDER_STRONG: Color = Color::from_rgb8(0x34, 0x3A, 0x48);
#[allow(dead_code)]
pub const BORDER_SUBTLE: Color = Color::from_rgb8(0x20, 0x22, 0x2E);

// Text ramp.
pub const TEXT_PRIMARY: Color = Color::from_rgb8(0xED, 0xEE, 0xF0);
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
#[allow(dead_code)]
pub const SPACE_L: f32 = 16.0;
#[allow(dead_code)]
pub const SPACE_XL: f32 = 24.0;
#[allow(dead_code)]
pub const SPACE_XXL: f32 = 32.0;

// Radii in logical pixels. Pills read as fully rounded at any height
// the shell uses; chunk 3 confirms against `Border::rounded`.
pub const RADIUS_PANEL: f32 = 8.0;
pub const RADIUS_PILL: f32 = 999.0;

// Type sizes in logical pixels.
pub const FONT_SIZE_SMALL: f32 = 12.0;
pub const FONT_SIZE_BODY: f32 = 14.0;
pub const FONT_SIZE_HEADING: f32 = 16.0;
pub const FONT_SIZE_TITLE: f32 = 20.0;

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
    use super::with_alpha;
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
}
