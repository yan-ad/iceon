//! Embedded console typefaces, independent of installed fonts and UI preferences.
//!
//! Register once during application initialization, before constructing console views.
//! GPUI caches native face resolution; we also retain the regular font descriptor so
//! rendering never needs to enumerate system fonts or rebuild a fallback selection.

use std::borrow::Cow;

use gpui_kit::{App, Font, Global, SharedString};

pub(crate) const FAMILY: &str = "JetBrains Mono";

const REGULAR: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf");
const ITALIC: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf");
const BOLD_ITALIC: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf");

struct ConsoleFont(Font);
impl Global for ConsoleFont {}

/// Install the bundled regular, bold, italic, and bold-italic console faces.
///
/// Failing to load our embedded assets is an application initialization error,
/// not a reason to silently depend on a machine's installed monospace fonts.
pub(crate) fn init(cx: &mut App) {
    if cx.try_global::<ConsoleFont>().is_some() {
        return;
    }
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(REGULAR),
            Cow::Borrowed(BOLD),
            Cow::Borrowed(ITALIC),
            Cow::Borrowed(BOLD_ITALIC),
        ])
        .expect("failed to register bundled JetBrains Mono console fonts");
    let face = gpui_kit::font(family());
    // Warm GPUI's native font-resolution cache once, after all faces are installed.
    cx.text_system().resolve_font(&face);
    cx.set_global(ConsoleFont(face));
}

/// The unconditional console family; unrelated to the application's UI font.
pub(crate) fn family() -> SharedString {
    FAMILY.into()
}

/// Reuse the registered regular face. Call `init` before constructing consoles.
pub(crate) fn font(cx: &App) -> Font {
    cx.global::<ConsoleFont>().0.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_console_styles_are_bundled_truetype_fonts() {
        for bytes in [REGULAR, BOLD, ITALIC, BOLD_ITALIC] {
            assert!(bytes.len() > 100_000, "font asset is unexpectedly small");
            assert_eq!(&bytes[..4], b"\x00\x01\x00\x00", "invalid TrueType header");
        }
    }

    #[test]
    fn bundled_license_preserves_upstream_copyright_and_ofl() {
        let license = include_str!("../assets/fonts/OFL.txt");
        assert!(license.contains("The JetBrains Mono Project Authors"));
        assert!(license.contains("SIL OPEN FONT LICENSE Version 1.1"));
        assert!(license.contains("PERMISSION & CONDITIONS"));
    }
}
