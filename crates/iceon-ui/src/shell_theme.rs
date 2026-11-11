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
    let selected = if mode.is_dark() {
        AppTheme::Dark
    } else {
        AppTheme::Light
    };
    let [
        background,
        surface,
        _elevated,
        foreground,
        muted,
        border,
        link,
        primary,
    ] = selected.palette_colors().expect("built-in default palette");
    if mode.is_dark() {
        colors.sidebar = neutral(surface, CHROME_ALPHA_DARK);
        colors.title_bar = colors.sidebar;
        colors.sidebar_foreground = neutral(foreground, 1.0);
        colors.sidebar_border = neutral(0xffffff, 0.08);
        colors.title_bar_border = colors.sidebar_border;
        colors.secondary_active = neutral(0xffffff, SELECTED_ALPHA);

        colors.background = neutral(background, 1.0);
        colors.foreground = colors.sidebar_foreground;
        colors.muted = neutral(0x151515, 1.0);
        colors.muted_foreground = neutral(muted, 1.0);
        colors.popover = neutral(background, 1.0);
        colors.popover_foreground = colors.foreground;
        colors.accent = colors.secondary_active;
        colors.accent_foreground = colors.foreground;
        colors.secondary = neutral(0x151515, 1.0);
        colors.secondary_foreground = colors.foreground;
        colors.secondary_hover = neutral(0xffffff, 0.08);
        colors.border = neutral(0xffffff, 0.10);
    } else {
        colors.sidebar = neutral(surface, CHROME_ALPHA_LIGHT);
        colors.title_bar = colors.sidebar;
        colors.sidebar_foreground = neutral(foreground, 1.0);
        colors.sidebar_border = neutral(0x000000, 0.08);
        colors.title_bar_border = colors.sidebar_border;
        colors.background = neutral(background, 1.0);
        colors.foreground = neutral(foreground, 1.0);
        colors.muted_foreground = neutral(muted, 1.0);
        colors.border = neutral(border, 1.0);
        colors.secondary_active = neutral(0x000000, 0.06);
        colors.secondary_hover = neutral(0x000000, 0.05);
    }

    // Brand interactive tokens in both modes without tinting the glass surfaces
    // or changing semantic success, warning, info, and danger colors.
    colors.primary = neutral(primary, 1.0);
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
        neutral(link, 1.0)
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
        neutral(link, 1.0)
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
    if matches!(theme, AppTheme::System | AppTheme::Light | AppTheme::Dark) {
        return None;
    }
    let [
        background,
        surface,
        elevated,
        foreground,
        muted,
        border,
        link,
        _,
    ] = theme.palette_colors()?;
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

fn primary_rgb(theme: AppTheme) -> u32 {
    theme
        .palette_colors()
        .map_or(PRIMARY_RGB, |colors| colors[7])
}

fn luminance(rgb: u32) -> f64 {
    let linear = |shift| {
        let channel = f64::from((rgb >> shift) & 255u32) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
}

fn primary_ink(rgb: u32) -> u32 {
    let l = luminance(rgb);
    let darkest = luminance(shade(rgb, 0.88));
    if (darkest + 0.05) / 0.05 >= 1.05 / (l + 0.05) {
        0x000000
    } else {
        0xffffff
    }
}

fn shade(rgb: u32, factor: f32) -> u32 {
    let channel = |shift| (((rgb >> shift) & 255u32) as f32 * factor).round() as u32;
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
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
    let primary = primary_rgb(theme);
    // Darken both interaction states; choose one foreground with contrast
    // against all three filled states, rather than inheriting default white.
    let hover = shade(primary, 0.94);
    let active = shade(primary, 0.88);
    let ink = primary_ink(primary);
    colors.primary = neutral(primary, 1.0);
    colors.primary_hover = neutral(hover, 1.0);
    colors.primary_active = neutral(active, 1.0);
    colors.primary_foreground = neutral(ink, 1.0);
    colors.button_primary = colors.primary;
    colors.button_primary_hover = colors.primary_hover;
    colors.button_primary_active = colors.primary_active;
    colors.button_primary_foreground = colors.primary_foreground;
    colors.sidebar_primary = colors.primary;
    colors.sidebar_primary_foreground = colors.primary_foreground;

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
    colors.sidebar_accent = colors.primary.alpha(0.16);
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
    colors.ring = colors.primary;
    colors.selection = colors.primary.alpha(0.25);
    colors.drag_border = link;
    colors.drop_target = link.alpha(0.18);
    colors.progress_bar = colors.primary;
    colors.slider_bar = colors.primary;
    colors.slider_thumb = fg;
    colors.chart_grid = border;
}

/// Shared semantic tag roles, deliberately applied after every custom palette.
fn semantic_tag_colors(colors: &mut ThemeColor, mode: ThemeMode) {
    let [danger, success, warning, info] = if mode.is_dark() {
        iceon_core::preferences::DARK_TAG_COLORS
    } else {
        iceon_core::preferences::LIGHT_TAG_COLORS
    };
    colors.danger = neutral(danger[0], 1.0);
    colors.danger_foreground = neutral(danger[1], 1.0);
    colors.success = neutral(success[0], 1.0);
    colors.success_foreground = neutral(success[1], 1.0);
    colors.warning = neutral(warning[0], 1.0);
    colors.warning_foreground = neutral(warning[1], 1.0);
    colors.info = neutral(info[0], 1.0);
    colors.info_foreground = neutral(info[1], 1.0);
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
        semantic_tag_colors(&mut theme.colors, mode);
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
    fn semantic_tags_match_requested_colors_for_every_palette() {
        use iceon_core::preferences::AppTheme;
        for &selected in AppTheme::ALL {
            for dark in [false, true] {
                let mode = if selected.is_dark().unwrap_or(dark) {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                };
                let mut colors = ThemeColor::default();
                brand_colors(&mut colors, mode);
                super::custom_colors(&mut colors, selected);
                super::semantic_tag_colors(&mut colors, mode);
                let expected = if mode.is_dark() {
                    [
                        [0xffb4ab, 0x690005],
                        [0xd0ffad, 0x153800],
                        [0xffb873, 0x4a2800],
                        [0xa3c9ff, 0x00315c],
                    ]
                } else {
                    [
                        [0xba1a1a, 0xffffff],
                        [0x2f6c00, 0xffffff],
                        [0x884e00, 0xffffff],
                        [0x005faa, 0xffffff],
                    ]
                };
                let actual = [
                    [colors.danger, colors.danger_foreground],
                    [colors.success, colors.success_foreground],
                    [colors.warning, colors.warning_foreground],
                    [colors.info, colors.info_foreground],
                ];
                for (pair, expected) in actual.into_iter().zip(expected) {
                    assert_eq!(pair, expected.map(|rgb| neutral(rgb, 1.0)), "{selected:?}");
                }
            }
        }
    }

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
    fn light_content_palette_uses_json_roles() {
        let mut colors = ThemeColor::default();
        brand_colors(&mut colors, ThemeMode::Light);
        let palette = iceon_core::preferences::AppTheme::Light
            .palette_colors()
            .unwrap();
        assert_eq!(colors.background, neutral(palette[0], 1.0));
        assert_eq!(colors.foreground, neutral(palette[3], 1.0));
        assert_eq!(colors.muted_foreground, neutral(palette[4], 1.0));
    }

    #[test]
    fn dark_content_is_opaque_and_neutral() {
        let mut colors = ThemeColor::default();
        brand_colors(&mut colors, ThemeMode::Dark);
        assert_eq!(colors.background, neutral(0x090909, 1.0));
        assert_eq!(colors.muted_foreground, neutral(0x969696, 1.0));
    }
    #[test]
    fn github_highlights_follow_green_accent() {
        use iceon_core::preferences::AppTheme;
        let mut colors = ThemeColor::default();
        super::custom_colors(&mut colors, AppTheme::GitHubDark);
        assert_eq!(colors.link, neutral(0x56d364, 1.0));
        assert_eq!(colors.sidebar_accent_foreground, colors.link);
        assert_eq!(colors.sidebar_accent, colors.primary.alpha(0.16));
        let palette = AppTheme::GitHubDark.palette_colors().unwrap();
        let contrast =
            (super::luminance(palette[6]) + 0.05) / (super::luminance(palette[0]) + 0.05);
        assert!(contrast >= 4.5);
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
            assert_eq!(colors.primary, neutral(super::primary_rgb(selected), 1.0));
            assert_ne!(colors.primary, neutral(PRIMARY_RGB, 1.0));
            assert_eq!(colors.button_primary, colors.primary);
            assert_eq!(colors.sidebar_primary, colors.primary);
            assert_eq!(colors.ring, colors.primary);
            let primary = super::primary_rgb(selected);
            let ink = super::luminance(super::primary_ink(primary));
            for factor in [1.0, 0.94, 0.88] {
                let fill = super::luminance(super::shade(primary, factor));
                let contrast = (fill.max(ink) + 0.05) / (fill.min(ink) + 0.05);
                assert!(contrast >= 4.5, "{selected:?}: primary contrast {contrast}");
            }
            assert_ne!(colors.link, colors.background);
            assert_ne!(colors.foreground, colors.background);
            let signature = (colors.background, colors.secondary, colors.foreground);
            assert!(
                !seen.contains(&signature),
                "duplicate palette: {selected:?}"
            );
            seen.push(signature);
        }
        assert_eq!(seen.len(), AppTheme::ALL.len() - 3);
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
    let [background, surface, elevated, _, _, _, _, primary] =
        theme.palette_colors().unwrap_or_else(|| {
            let mut colors = AppTheme::Light.palette_colors().expect("Light palette");
            colors[1] = AppTheme::Dark.palette_colors().expect("Dark palette")[1];
            colors
        });
    [background, surface, elevated, primary]
}
