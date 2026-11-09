//! One theme write path for Iceon's black, native-material chrome.
//!
//! Call `apply` instead of `Theme::change` at startup and on preference changes.
//! Keep the window/root layout transparent and paint only the main content with
//! `theme.background`; otherwise an opaque root hides the native material.
//! TitleBar 0.9 mixes its token with the content background into a gradient:
//! set the standard TitleBar's `.bg(cx.theme().tokens.title_bar)` and
//! `.text_color(cx.theme().sidebar_foreground)` to avoid a light-mode gradient.

use gpui_kit::component::theme::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{App, Hsla, Window, WindowBackgroundAppearance, rgb};

const CHROME_ALPHA: f32 = 0.90;
const SELECTED_ALPHA: f32 = 0.10;

fn neutral(value: u32, alpha: f32) -> Hsla {
    Hsla::from(rgb(value)).alpha(alpha)
}

/// Native blur on macOS; other platforms use compositor transparency when
/// available. No software blur, unsupported native calls, or extra dependency.
pub(crate) fn background_appearance() -> WindowBackgroundAppearance {
    if cfg!(target_os = "macos") {
        WindowBackgroundAppearance::Blurred
    } else {
        WindowBackgroundAppearance::Transparent
    }
}

fn brand_colors(colors: &mut ThemeColor, mode: ThemeMode) {
    colors.sidebar = neutral(0x000000, CHROME_ALPHA);
    colors.title_bar = colors.sidebar;
    colors.sidebar_foreground = neutral(0xe6e6e6, 1.0);
    colors.sidebar_accent = neutral(0xffffff, SELECTED_ALPHA);
    colors.sidebar_accent_foreground = colors.sidebar_foreground;
    colors.sidebar_primary = colors.sidebar_accent;
    colors.sidebar_primary_foreground = colors.sidebar_foreground;
    colors.sidebar_border = neutral(0xffffff, 0.08);
    colors.title_bar_border = colors.sidebar_border;
    // Ghost Button's selected state reads secondary_active, not sidebar_accent.
    colors.secondary_active = colors.sidebar_accent;

    // Preserve the saved light content palette. Only dark content is branded;
    // sidebar/titlebar tokens remain black independently of the content mode.
    if mode.is_dark() {
        colors.background = neutral(0x090909, 1.0);
        colors.foreground = colors.sidebar_foreground;
        colors.muted = neutral(0x151515, 1.0);
        colors.muted_foreground = neutral(0x969696, 1.0);
        colors.popover = neutral(0x090909, 1.0);
        colors.popover_foreground = colors.foreground;
        colors.accent = colors.sidebar_accent;
        colors.accent_foreground = colors.foreground;
        colors.secondary = neutral(0x151515, 1.0);
        colors.secondary_foreground = colors.foreground;
        colors.secondary_hover = neutral(0xffffff, 0.08);
        colors.border = neutral(0xffffff, 0.10);
        colors.ring = neutral(0x969696, 1.0);
    }
}

/// Reload the user's chosen content mode, then brand chrome. Theme::update
/// reconciles legacy colors, renderable ThemeTokens, and GPUI Base tokens in
/// one operation; mutating Theme::global_mut alone would leave stale copies.
///
/// Passing no window is useful before opening one: set WindowOptions'
/// `window_background: background_appearance()` as well. Defaults/native font
/// resolution remain under gpui-kit ownership; this never installs a web font.
pub(crate) fn apply(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| brand_colors(&mut theme.colors, mode));
    if let Some(window) = window {
        window.set_background_appearance(background_appearance());
    }
}

#[cfg(test)]
mod tests {
    use super::{CHROME_ALPHA, SELECTED_ALPHA, brand_colors, neutral};
    use gpui_kit::component::theme::{ThemeColor, ThemeMode};

    #[test]
    fn black_chrome_is_independent_of_content_mode() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let mut colors = ThemeColor::default();
            brand_colors(&mut colors, mode);
            assert_eq!(colors.sidebar, neutral(0x000000, CHROME_ALPHA));
            assert_eq!(colors.title_bar, colors.sidebar);
            assert_eq!(colors.sidebar_accent, neutral(0xffffff, SELECTED_ALPHA));
            assert_eq!(colors.sidebar_foreground, neutral(0xe6e6e6, 1.0));
        }
    }

    #[test]
    fn light_content_palette_is_retained() {
        let mut colors = ThemeColor::default();
        colors.background = neutral(0xfafafa, 1.0);
        colors.foreground = neutral(0x121212, 1.0);
        colors.muted_foreground = neutral(0x666666, 1.0);
        let before = colors;
        brand_colors(&mut colors, ThemeMode::Light);
        assert_eq!(colors.background, before.background);
        assert_eq!(colors.foreground, before.foreground);
        assert_eq!(colors.muted_foreground, before.muted_foreground);
    }

    #[test]
    fn dark_content_is_opaque_and_neutral() {
        let mut colors = ThemeColor::default();
        brand_colors(&mut colors, ThemeMode::Dark);
        assert_eq!(colors.background, neutral(0x090909, 1.0));
        assert_eq!(colors.muted_foreground, neutral(0x969696, 1.0));
    }
}
