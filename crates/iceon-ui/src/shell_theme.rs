//! One theme write path for Iceon's native-material chrome.
//!
//! Call `apply` instead of `Theme::change` at startup and on preference changes.
//! Keep the window/root layout transparent and paint only the main content with
//! `theme.background`; otherwise an opaque root hides the native material.

use gpui_kit::component::theme::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{App, Hsla, Window, WindowBackgroundAppearance, rgb};

const CHROME_ALPHA_LIGHT: f32 = 0.48;
const CHROME_ALPHA_DARK: f32 = 0.38;
const SELECTED_ALPHA: f32 = 0.12;
pub(crate) const PRIMARY_RGB: u32 = 0x1a43bf;

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
    if mode.is_dark() {
        colors.sidebar = neutral(0x121214, CHROME_ALPHA_DARK);
        colors.title_bar = colors.sidebar;
        colors.sidebar_foreground = neutral(0xededed, 1.0);
        colors.sidebar_border = neutral(0xffffff, 0.08);
        colors.title_bar_border = colors.sidebar_border;
        colors.secondary_active = neutral(0xffffff, SELECTED_ALPHA);

        colors.background = neutral(0x090909, 1.0);
        colors.foreground = colors.sidebar_foreground;
        colors.muted = neutral(0x151515, 1.0);
        colors.muted_foreground = neutral(0x969696, 1.0);
        colors.popover = neutral(0x090909, 1.0);
        colors.popover_foreground = colors.foreground;
        colors.accent = colors.secondary_active;
        colors.accent_foreground = colors.foreground;
        colors.secondary = neutral(0x151515, 1.0);
        colors.secondary_foreground = colors.foreground;
        colors.secondary_hover = neutral(0xffffff, 0.08);
        colors.border = neutral(0xffffff, 0.10);
    } else {
        colors.sidebar = neutral(0xf6f6f7, CHROME_ALPHA_LIGHT);
        colors.title_bar = colors.sidebar;
        colors.sidebar_foreground = neutral(0x18181b, 1.0);
        colors.sidebar_border = neutral(0x000000, 0.08);
        colors.title_bar_border = colors.sidebar_border;
        colors.secondary_active = neutral(0x000000, 0.06);
        colors.secondary_hover = neutral(0x000000, 0.05);
    }

    // Brand interactive tokens in both modes without tinting the glass surfaces
    // or changing semantic success, warning, info, and danger colors.
    colors.primary = neutral(PRIMARY_RGB, 1.0);
    colors.primary_foreground = neutral(0xffffff, 1.0);
    colors.primary_hover = neutral(0x163aa6, 1.0);
    colors.primary_active = neutral(0x12308a, 1.0);
    colors.button_primary = colors.primary;
    colors.button_primary_foreground = colors.primary_foreground;
    colors.button_primary_hover = colors.primary_hover;
    colors.button_primary_active = colors.primary_active;
    colors.ring = colors.primary;
    colors.link = colors.primary;
    colors.link_hover = colors.primary_hover;
    colors.link_active = colors.primary_active;
    colors.selection = colors.primary.alpha(0.18);
    colors.sidebar_primary = colors.primary;
    colors.sidebar_primary_foreground = colors.primary_foreground;
    colors.sidebar_accent = colors.primary.alpha(SELECTED_ALPHA);
    colors.sidebar_accent_foreground = if mode.is_dark() {
        neutral(0x9bb4ff, 1.0)
    } else {
        colors.primary
    };
}

/// Reload the user's chosen content mode, then brand chrome. Theme::update
/// reconciles legacy colors, renderable ThemeTokens, and GPUI Base tokens in
/// one operation; mutating Theme::global_mut alone would leave stale copies.
///
/// Passing no window is useful before opening one: set WindowOptions'
/// `window_background: background_appearance()` as well. Defaults/native font
/// resolution remain under gpui-kit ownership; this never installs a web font.
pub(crate) fn apply(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    // The settings window's legacy caller maps System to Dark. Resolve from
    // persisted policy here so every write path follows the native appearance.
    let mode = cx
        .try_global::<crate::preferences_view::GlobalPreferencesState>()
        .map(|state| {
            crate::preference_runtime::theme_mode(
                &state.preferences,
                matches!(
                    cx.window_appearance(),
                    gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark
                ),
            )
        })
        .unwrap_or(mode);
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| brand_colors(&mut theme.colors, mode));
    if let Some(window) = window {
        window.set_background_appearance(background_appearance());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CHROME_ALPHA_DARK, CHROME_ALPHA_LIGHT, PRIMARY_RGB, SELECTED_ALPHA, brand_colors, neutral,
    };
    use gpui_kit::component::theme::{ThemeColor, ThemeMode};

    #[test]
    fn chrome_colors_match_mode_palette() {
        let mut dark_colors = ThemeColor::default();
        brand_colors(&mut dark_colors, ThemeMode::Dark);
        assert_eq!(dark_colors.sidebar, neutral(0x121214, CHROME_ALPHA_DARK));
        assert_eq!(dark_colors.title_bar, dark_colors.sidebar);
        assert_eq!(
            dark_colors.sidebar_accent,
            neutral(PRIMARY_RGB, SELECTED_ALPHA)
        );
        assert_eq!(dark_colors.sidebar_foreground, neutral(0xededed, 1.0));
        assert_eq!(
            dark_colors.sidebar_accent_foreground,
            neutral(0x9bb4ff, 1.0)
        );

        let mut light_colors = ThemeColor::default();
        brand_colors(&mut light_colors, ThemeMode::Light);
        assert_eq!(light_colors.sidebar, neutral(0xf6f6f7, CHROME_ALPHA_LIGHT));
        assert_eq!(light_colors.title_bar, light_colors.sidebar);
        assert_eq!(
            light_colors.sidebar_accent,
            neutral(PRIMARY_RGB, SELECTED_ALPHA)
        );
        assert_eq!(light_colors.sidebar_foreground, neutral(0x18181b, 1.0));
        assert_eq!(light_colors.sidebar_accent_foreground, light_colors.primary);
    }

    #[test]
    fn primary_brand_and_interactive_aliases_match_in_both_modes() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let mut colors = ThemeColor::default();
            brand_colors(&mut colors, mode);
            assert_eq!(colors.primary, neutral(0x1a43bf, 1.0));
            assert_eq!(colors.primary_foreground, neutral(0xffffff, 1.0));
            assert_eq!(colors.primary_hover, neutral(0x163aa6, 1.0));
            assert_eq!(colors.primary_active, neutral(0x12308a, 1.0));
            assert_eq!(colors.button_primary, colors.primary);
            assert_eq!(colors.button_primary_foreground, colors.primary_foreground);
            assert_eq!(colors.button_primary_hover, colors.primary_hover);
            assert_eq!(colors.button_primary_active, colors.primary_active);
            assert_eq!(colors.ring, colors.primary);
            assert_eq!(colors.link, colors.primary);
            assert_eq!(colors.link_hover, colors.primary_hover);
            assert_eq!(colors.link_active, colors.primary_active);
            assert_eq!(colors.selection, neutral(PRIMARY_RGB, 0.18));
            assert_eq!(colors.sidebar_primary, colors.primary);
            assert_eq!(colors.sidebar_primary_foreground, colors.primary_foreground);
        }
    }

    #[test]
    fn semantic_status_colors_and_tab_background_are_retained() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let mut colors = ThemeColor::default();
            colors.success = neutral(0x16a34a, 1.0);
            colors.warning = neutral(0xf59e0b, 1.0);
            colors.danger = neutral(0xdc2626, 1.0);
            colors.info = neutral(0x0284c7, 1.0);
            colors.tab_active = neutral(0x27272a, 1.0);
            let before = colors;
            brand_colors(&mut colors, mode);
            assert_eq!(colors.success, before.success);
            assert_eq!(colors.warning, before.warning);
            assert_eq!(colors.danger, before.danger);
            assert_eq!(colors.info, before.info);
            assert_eq!(colors.tab_active, before.tab_active);
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
