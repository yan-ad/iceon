//! Runtime projections of persisted preferences. This deliberately stays out of the settings UI.

use iceon_core::preferences::{Preferences, TerminalTheme};
use jiff::{Timestamp, tz::TimeZone};

/// Resolve the app's theme against the actual native window appearance.
pub(crate) fn theme_mode(prefs: &Preferences, dark: bool) -> gpui_kit::component::theme::ThemeMode {
    use gpui_kit::component::theme::ThemeMode;
    if prefs.app.theme.is_dark().unwrap_or(dark) {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    }
}

#[derive(Default)]
struct FontFamilyCache {
    installed: Option<Vec<String>>,
    resolved: std::collections::HashMap<String, gpui_kit::SharedString>,
}

impl gpui_kit::Global for FontFamilyCache {}

/// GPUI accepts one family, not a CSS comma-separated fallback list.
/// Resolve the first installed preference, retaining a native monospace fallback.
/// Lazily populated after startup font registration, and scoped to this App.
/// Repeated renders only clone a SharedString; native font discovery runs once.
pub(crate) fn font_family(value: &str, cx: &mut gpui_kit::App) -> gpui_kit::SharedString {
    if let Some(family) = cx
        .try_global::<FontFamilyCache>()
        .and_then(|cache| cache.resolved.get(value))
    {
        return family.clone();
    }
    if cx.default_global::<FontFamilyCache>().installed.is_none() {
        let installed = cx.text_system().all_font_names();
        cx.default_global::<FontFamilyCache>().installed = Some(installed);
    }
    let cache = cx.default_global::<FontFamilyCache>();
    let installed = cache.installed.as_ref().unwrap();
    let family: gpui_kit::SharedString = value
        .split(',')
        .map(|part| part.trim().trim_matches(['\'', '"']))
        .find(|part| installed.iter().any(|name| name.eq_ignore_ascii_case(part)))
        .unwrap_or("monospace")
        .to_owned()
        .into();
    cache.resolved.insert(value.to_owned(), family.clone());
    family
}

/// Format API timestamps in the saved IANA timezone. Invalid or unavailable
/// zones intentionally fall back to the source value rather than inventing a date.
pub(crate) fn timestamp(value: &str, timezone: &str) -> String {
    let Ok(parsed) = value.parse::<Timestamp>() else {
        return value.to_owned();
    };
    let zone = if timezone.eq_ignore_ascii_case("system") || timezone.is_empty() {
        TimeZone::system()
    } else {
        match TimeZone::get(timezone) {
            Ok(zone) => zone,
            Err(_) => return value.to_owned(),
        }
    };
    parsed
        .to_zoned(zone)
        .strftime("%Y-%m-%d %H:%M:%S %Z")
        .to_string()
}

/// Kubernetes log timestamps are a prefix; preserve payload bytes and stored/exported logs.
pub(crate) fn log_line(value: &str, timezone: &str) -> String {
    let Some((prefix, payload)) = value.split_once(' ') else {
        return value.to_owned();
    };
    if prefix.parse::<Timestamp>().is_err() {
        return value.to_owned();
    }
    format!("{} {payload}", timestamp(prefix, timezone))
}

/// Full presentation palette, shared by native exec/device terminals and logs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerminalPalette {
    pub background: gpui_kit::Hsla,
    pub foreground: gpui_kit::Hsla,
    pub ansi: [gpui_kit::Hsla; 16],
    pub cursor: gpui_kit::Hsla,
    pub cursor_text: gpui_kit::Hsla,
    pub selection: gpui_kit::Hsla,
}

impl TerminalPalette {
    pub(crate) fn default_for(dark: bool) -> Self {
        let color = |rgb| gpui_kit::Hsla::from(gpui_kit::rgb(rgb));
        let background = color(if dark { 0x000000 } else { 0xffffff });
        let foreground = color(if dark { 0xd4d4d4 } else { 0x1f1f1f });
        Self {
            background,
            foreground,
            ansi: [
                0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5,
                0x666666, 0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
            ]
            .map(color),
            cursor: foreground,
            cursor_text: background,
            selection: color(0x365f8c).alpha(0.65),
        }
    }
}

/// Replaced atomically on every app-theme application, never read from disk in render.
pub(crate) struct ConsoleTheme(pub TerminalPalette);
impl gpui_kit::Global for ConsoleTheme {}

pub(crate) fn terminal_palette(theme: TerminalTheme, cx: &gpui_kit::App) -> TerminalPalette {
    use gpui_kit::component::theme::Theme;
    let matched = cx
        .try_global::<ConsoleTheme>()
        .map(|theme| theme.0.clone())
        .unwrap_or_else(|| TerminalPalette::default_for(Theme::global(cx).mode.is_dark()));
    resolve_terminal_palette(theme, matched)
}

fn resolve_terminal_palette(theme: TerminalTheme, matched: TerminalPalette) -> TerminalPalette {
    match theme {
        TerminalTheme::Dark => TerminalPalette::default_for(true),
        TerminalTheme::Light => TerminalPalette::default_for(false),
        TerminalTheme::Match => matched,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iceon_core::preferences::{AppTheme, Preferences};

    #[test]
    fn terminal_match_inherits_all_tokens_but_explicit_modes_ignore_import() {
        let mut imported = TerminalPalette::default_for(false);
        imported.background = gpui_kit::rgb(0x112233).into();
        imported.ansi[1] = gpui_kit::rgb(0x445566).into();
        imported.cursor = gpui_kit::rgb(0x778899).into();
        assert_eq!(
            resolve_terminal_palette(TerminalTheme::Match, imported.clone()),
            imported
        );
        assert_eq!(
            resolve_terminal_palette(TerminalTheme::Dark, imported.clone()),
            TerminalPalette::default_for(true)
        );
        assert_eq!(
            resolve_terminal_palette(TerminalTheme::Light, imported),
            TerminalPalette::default_for(false)
        );
    }

    #[test]
    fn preserves_invalid_timestamp_and_zone() {
        assert_eq!(timestamp("not-a-date", "UTC"), "not-a-date");
        assert_eq!(
            timestamp("2024-01-01T00:00:00Z", "No/Such_Zone"),
            "2024-01-01T00:00:00Z"
        );
    }

    #[test]
    fn log_timestamp_rendering_does_not_rewrite_payload() {
        assert_eq!(
            log_line("2024-01-01T00:00:00Z   token=abc", "UTC"),
            "2024-01-01 00:00:00 UTC   token=abc"
        );
        assert_eq!(
            log_line("plain application log", "UTC"),
            "plain application log"
        );
    }

    #[test]
    fn timestamps_observe_dst_and_offsets() {
        assert_eq!(
            timestamp("2024-01-01T00:00:00Z", "Asia/Jakarta"),
            "2024-01-01 07:00:00 WIB"
        );
        assert_eq!(
            timestamp("2024-01-01T00:00:00Z", "America/New_York"),
            "2023-12-31 19:00:00 EST"
        );
        assert_eq!(
            timestamp("2024-07-01T00:00:00Z", "America/New_York"),
            "2024-06-30 20:00:00 EDT"
        );
    }

    #[test]
    fn system_theme_tracks_appearance() {
        let mut p = Preferences::default();
        p.app.theme = AppTheme::System;
        assert!(theme_mode(&p, true).is_dark());
        assert!(!theme_mode(&p, false).is_dark());
    }
    #[test]
    fn explicit_palette_modes_ignore_system_appearance() {
        for &theme in AppTheme::ALL {
            let Some(dark) = theme.is_dark() else {
                continue;
            };
            let mut p = Preferences::default();
            p.app.theme = theme;
            assert_eq!(theme_mode(&p, true).is_dark(), dark);
            assert_eq!(theme_mode(&p, false).is_dark(), dark);
        }
    }
}
