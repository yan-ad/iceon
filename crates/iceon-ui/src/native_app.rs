//! Small native application integration; never changes GPUI's delegate.
//!
//! The icon bytes are embedded from the same assets/icons directory used by the
//! packager. This works for `cargo run` from any cwd and for relocated binaries.

#[cfg(any(target_os = "macos", test))]
const APP_ICON: &[u8] = include_bytes!("../../../assets/icons/icon.icns");

/// Bundled apps must use their compiled Icon Composer asset. A custom NSImage
/// replaces the Dock tile and prevents the system selecting dark/mono variants.
#[cfg(any(target_os = "macos", test))]
fn is_app_bundle_executable(executable: &std::path::Path) -> bool {
    let Some(macos) = executable.parent() else {
        return false;
    };
    let Some(contents) = macos.parent() else {
        return false;
    };
    macos.file_name().is_some_and(|name| name == "MacOS")
        && contents.file_name().is_some_and(|name| name == "Contents")
        && contents
            .parent()
            .is_some_and(|bundle| bundle.extension().is_some_and(|ext| ext == "app"))
}

/// Supply only an unbundled development fallback. Packaged icons are system-owned.
pub(super) fn init() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::{AnyThread, MainThreadMarker};
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::NSData;
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            "Application icon updates must run on the macOS main thread".to_string()
        })?;

        // Never install a flattened image over a bundle's appearance-aware icon.
        // Fail closed if the executable location cannot be determined.
        let Ok(executable) = std::env::current_exe() else {
            return Ok(());
        };
        if is_app_bundle_executable(&executable) {
            return Ok(());
        }
        let data = NSData::with_bytes(APP_ICON);
        let image = NSImage::initWithData(NSImage::alloc(), &data)
            .ok_or_else(|| "Could not decode assets/icons/icon.icns".to_string())?;

        // SAFETY: A valid decoded, non-nil image is supplied on the main thread.
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::APP_ICON;

    #[test]
    fn development_app_icon_is_the_packaged_icns_asset() {
        assert_eq!(&APP_ICON[..4], b"icns");
        let length = u32::from_be_bytes(APP_ICON[4..8].try_into().unwrap());
        assert_eq!(length as usize, APP_ICON.len());
    }

    #[test]
    fn bundle_icons_are_system_owned_but_development_uses_fallback() {
        use std::path::Path;
        assert!(super::is_app_bundle_executable(Path::new(
            "/Applications/Iceon.app/Contents/MacOS/iceon"
        )));
        assert!(super::is_app_bundle_executable(Path::new(
            "/tmp/Relocated.app/Contents/MacOS/iceon"
        )));
        for path in [
            "/tmp/iceon",
            "target/debug/iceon",
            "/tmp/Iceon.app/iceon",
            "/tmp/Contents/MacOS/iceon",
        ] {
            assert!(!super::is_app_bundle_executable(Path::new(path)), "{path}");
        }
    }
}
