//! One theme write path for Iceon's native-material chrome.
//!
//! Call `apply` instead of `Theme::change` at startup and on preference changes.
//! Keep the window/root layout transparent and paint only the main content with
//! `theme.background`; otherwise an opaque root hides the native material.

use gpui_kit::component::theme::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{App, Hsla, Window, WindowBackgroundAppearance, rgb};
use iceon_core::preferences::AppTheme;

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
    // Filled buttons retain Iceon blue, while text links need a lighter blue
    // against dark content (the brand blue alone is below readable contrast).
    colors.link = if mode.is_dark() {
        neutral(0x9bb4ff, 1.0)
    } else {
        colors.primary
    };
    colors.link_hover = if mode.is_dark() {
        neutral(0xc2d1ff, 1.0)
    } else {
        colors.primary_hover
    };
    colors.link_active = if mode.is_dark() {
        neutral(0xadc2ff, 1.0)
    } else {
        colors.primary_active
    };
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

/// A small set of canonical palette roles, projected onto every component
/// surface rather than leaving tables, inputs and menus in the stock palette.
#[derive(Clone, Copy)]
struct Palette {
    background: u32,
    surface: u32,
    elevated: u32,
    foreground: u32,
    muted: u32,
    border: u32,
    link: u32,
}

fn palette(theme: AppTheme) -> Option<Palette> {
    use AppTheme::*;
    let (background, surface, elevated, foreground, muted, border, link) = match theme {
        System | Light | Dark => return None,
        JetBrainsDarcula => (
            0x2b2b2b, 0x313335, 0x3c3f41, 0xa9b7c6, 0x90989f, 0x55585b, 0x8ab4f8,
        ),
        JetBrainsLight => (
            0xffffff, 0xf2f2f2, 0xe8e8e8, 0x242424, 0x666666, 0xcccccc, 0x174eb6,
        ),
        VsCodeDark => (
            0x1e1e1e, 0x252526, 0x333333, 0xd4d4d4, 0x9d9d9d, 0x454545, 0x75beff,
        ),
        VsCodeLight => (
            0xffffff, 0xf3f3f3, 0xe8e8e8, 0x333333, 0x616161, 0xcecece, 0x005fb8,
        ),
        GitHubDark => (
            0x0d1117, 0x161b22, 0x21262d, 0xe6edf3, 0x9198a1, 0x3d444d, 0x79c0ff,
        ),
        GitHubLight => (
            0xffffff, 0xf6f8fa, 0xeaeef2, 0x1f2328, 0x59636e, 0xd1d9e0, 0x0969da,
        ),
        GruvboxDark => (
            0x282828, 0x32302f, 0x3c3836, 0xebdbb2, 0xbdae93, 0x665c54, 0x83a598,
        ),
        GruvboxDarkHard => (
            0x1d2021, 0x282828, 0x32302f, 0xebdbb2, 0xbdae93, 0x665c54, 0x83a598,
        ),
        GruvboxLight => (
            0xfbf1c7, 0xebdbb2, 0xd5c4a1, 0x3c3836, 0x665c54, 0xbdae93, 0x076678,
        ),
        GruvboxLightHard => (
            0xf9f5d7, 0xf2e5bc, 0xebdbb2, 0x3c3836, 0x665c54, 0xbdae93, 0x076678,
        ),
        CatppuccinLatte => (
            0xeff1f5, 0xe6e9ef, 0xdce0e8, 0x4c4f69, 0x6c6f85, 0xacb0be, 0x1e66f5,
        ),
        CatppuccinFrappe => (
            0x303446, 0x292c3c, 0x414559, 0xc6d0f5, 0xa5adce, 0x626880, 0x8caaee,
        ),
        CatppuccinMacchiato => (
            0x24273a, 0x1e2030, 0x363a4f, 0xcad3f5, 0xa5adcb, 0x5b6078, 0x8aadf4,
        ),
        CatppuccinMocha => (
            0x1e1e2e, 0x181825, 0x313244, 0xcdd6f4, 0xa6adc8, 0x585b70, 0x89b4fa,
        ),
        SolarizedDark => (
            0x002b36, 0x073642, 0x164450, 0x93a1a1, 0x839496, 0x586e75, 0x65b6e3,
        ),
        SolarizedLight => (
            0xfdf6e3, 0xeee8d5, 0xe5dfcb, 0x586e75, 0x657b83, 0xb9b5a6, 0x176b9b,
        ),
    };
    Some(Palette {
        background,
        surface,
        elevated,
        foreground,
        muted,
        border,
        link,
    })
}

fn custom_colors(colors: &mut ThemeColor, theme: AppTheme) {
    let Some(p) = palette(theme) else { return };
    let bg = neutral(p.background, 1.0);
    let surface = neutral(p.surface, 1.0);
    let raised = neutral(p.elevated, 1.0);
    let fg = neutral(p.foreground, 1.0);
    let muted = neutral(p.muted, 1.0);
    let border = neutral(p.border, 1.0);
    let link = neutral(p.link, 1.0);
    colors.background = bg;
    colors.foreground = fg;
    colors.muted = surface;
    colors.muted_foreground = muted;
    colors.border = border;
    colors.window_border = border;
    colors.input = border; // Input background uses `background`; `input` is its border.
    colors.caret = fg;
    colors.popover = surface;
    colors.popover_foreground = fg;
    colors.accent = raised;
    colors.accent_foreground = fg;
    colors.accordion = surface;
    colors.secondary = surface;
    colors.secondary_hover = raised;
    colors.secondary_active = raised;
    colors.secondary_foreground = fg;
    colors.button = surface;
    colors.button_foreground = fg;
    colors.button_hover = raised;
    colors.button_active = raised;
    colors.button_secondary = surface;
    colors.button_secondary_foreground = fg;
    colors.button_secondary_hover = raised;
    colors.button_secondary_active = raised;
    colors.group_box = surface;
    colors.group_box_foreground = fg;
    colors.description_list_label = surface;
    colors.description_list_label_foreground = muted;
    colors.list = bg;
    colors.list_even = surface;
    colors.list_head = surface;
    colors.list_hover = raised;
    colors.list_active = raised;
    colors.list_active_border = link;
    colors.table = bg;
    colors.table_even = surface;
    colors.table_head = surface;
    colors.table_head_foreground = fg;
    colors.table_foot = surface;
    colors.table_foot_foreground = muted;
    colors.table_hover = raised;
    colors.table_active = raised;
    colors.table_active_border = link;
    colors.table_row_border = border.alpha(0.5);
    colors.tab = surface;
    colors.tab_foreground = muted;
    colors.tab_active = bg;
    colors.tab_active_foreground = fg;
    colors.tab_bar = surface;
    colors.tab_bar_segmented = raised;
    let alpha = if theme.is_dark() == Some(true) {
        CHROME_ALPHA_DARK
    } else {
        CHROME_ALPHA_LIGHT
    };
    colors.sidebar = surface.alpha(alpha);
    colors.sidebar_foreground = fg;
    colors.sidebar_border = border.alpha(0.5);
    colors.sidebar_accent = raised;
    colors.sidebar_accent_foreground = link;
    colors.title_bar = colors.sidebar;
    colors.title_bar_border = colors.sidebar_border;
    colors.status_bar = surface;
    colors.status_bar_border = border;
    colors.scrollbar = bg.alpha(0.0);
    colors.scrollbar_thumb = border;
    colors.scrollbar_thumb_hover = muted;
    colors.skeleton = raised;
    colors.switch = border;
    colors.switch_thumb = fg;
    colors.link = link;
    colors.link_hover = fg;
    colors.link_active = link;
    colors.ring = link;
    colors.selection = link.alpha(0.25);
    colors.drag_border = link;
    colors.drop_target = link.alpha(0.18);
    colors.progress_bar = colors.primary;
    colors.slider_bar = colors.primary;
    colors.slider_thumb = fg;
    colors.chart_grid = border;
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
    let selected = cx
        .try_global::<crate::preferences_view::GlobalPreferencesState>()
        .map(|state| state.preferences.app.theme)
        .unwrap_or(if mode.is_dark() {
            AppTheme::Dark
        } else {
            AppTheme::Light
        });
    // Reload even when the mode is unchanged: custom -> custom and custom ->
    // default must not inherit any of the previous palette's component roles.
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        brand_colors(&mut theme.colors, mode);
        custom_colors(&mut theme.colors, selected);
    });
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
            if mode.is_dark() {
                assert_eq!(colors.link, neutral(0x9bb4ff, 1.0));
                assert_ne!(colors.link, colors.primary);
            } else {
                assert_eq!(colors.link, colors.primary);
                assert_eq!(colors.link_hover, colors.primary_hover);
                assert_eq!(colors.link_active, colors.primary_active);
            }
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
    #[test]
    fn custom_palettes_cover_component_surfaces_and_are_distinct() {
        use iceon_core::preferences::AppTheme;
        let mut seen = Vec::new();
        for &selected in AppTheme::ALL {
            if super::palette(selected).is_none() {
                continue;
            }
            let mode = if selected.is_dark().unwrap() {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            };
            let mut colors = ThemeColor::default();
            brand_colors(&mut colors, mode);
            super::custom_colors(&mut colors, selected);
            assert_eq!(colors.table, colors.background);
            assert_eq!(colors.popover, colors.secondary);
            assert_eq!(colors.input, colors.border);
            assert_eq!(colors.table_head, colors.secondary);
            assert_eq!(colors.foreground, colors.popover_foreground);
            assert_eq!(colors.primary, neutral(PRIMARY_RGB, 1.0));
            assert_ne!(colors.link, colors.background);
            assert_ne!(colors.foreground, colors.background);
            let signature = (colors.background, colors.secondary, colors.foreground);
            assert!(
                !seen.contains(&signature),
                "duplicate palette: {selected:?}"
            );
            seen.push(signature);
        }
        assert_eq!(seen.len(), 16);
    }

    #[test]
    fn switching_same_mode_replaces_every_custom_surface() {
        use iceon_core::preferences::AppTheme;
        let mut changed = ThemeColor::default();
        super::custom_colors(&mut changed, AppTheme::CatppuccinMocha);
        super::custom_colors(&mut changed, AppTheme::SolarizedDark);
        let mut fresh = ThemeColor::default();
        super::custom_colors(&mut fresh, AppTheme::SolarizedDark);
        assert_eq!(changed, fresh);
    }
}

/// Preview roles use the same palette source as the live theme.
pub(crate) fn preview_colors(theme: AppTheme) -> [u32; 4] {
    if let Some(palette) = palette(theme) {
        [
            palette.background,
            palette.surface,
            palette.elevated,
            palette.link,
        ]
    } else if theme == AppTheme::Light {
        [0xffffff, 0xf6f6f7, 0xe4e4e7, PRIMARY_RGB]
    } else if theme == AppTheme::System {
        [0xfafafa, 0x18181b, 0xe4e4e7, PRIMARY_RGB]
    } else {
        [0x090909, 0x121214, 0x27272a, PRIMARY_RGB]
    }
}
