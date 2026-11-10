//! Runtime projections of persisted preferences. This deliberately stays out of the settings UI.

use iceon_core::preferences::{AppTheme, Preferences, TerminalTheme};
use jiff::{Timestamp, tz::TimeZone};

/// Resolve the app's theme against the actual native window appearance.
pub(crate) fn theme_mode(prefs: &Preferences, dark: bool) -> gpui_kit::component::theme::ThemeMode {
    use gpui_kit::component::theme::ThemeMode;
    match prefs.app.theme {
        AppTheme::Light => ThemeMode::Light,
        AppTheme::Dark => ThemeMode::Dark,
        AppTheme::System => {
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            }
        }
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

pub(crate) fn terminal_colors(theme: TerminalTheme, dark: bool) -> (u32, u32) {
    match theme {
        TerminalTheme::Dark => (0x000000, 0xd4d4d4),
        TerminalTheme::Light => (0xffffff, 0x1f1f1f),
        TerminalTheme::Match => {
            if dark {
                (0x000000, 0xd4d4d4)
            } else {
                (0xffffff, 0x1f1f1f)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iceon_core::preferences::Preferences;

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
    fn terminal_match_respects_app_but_explicit_theme_wins() {
        assert_ne!(
            terminal_colors(TerminalTheme::Match, true),
            terminal_colors(TerminalTheme::Match, false)
        );
        assert_eq!(
            terminal_colors(TerminalTheme::Light, true),
            terminal_colors(TerminalTheme::Light, false)
        );
        assert_eq!(terminal_colors(TerminalTheme::Dark, false), (0, 0xd4d4d4));
    }

    #[test]
    fn system_theme_tracks_appearance() {
        let mut p = Preferences::default();
        p.app.theme = AppTheme::System;
        assert!(theme_mode(&p, true).is_dark());
        assert!(!theme_mode(&p, false).is_dark());
    }
}
