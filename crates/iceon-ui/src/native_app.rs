//! Small native application integration; never changes GPUI's delegate.
//!
//! The icon bytes are embedded from the same assets/icons directory used by the
//! packager. This works for `cargo run` from any cwd and for relocated binaries.

#[cfg(any(target_os = "macos", test))]
const APP_ICON: &[u8] = include_bytes!("../../../assets/icons/icon.icns");

/// Install the application/Dock icon for unbundled development runs as well as
/// packaged runs. Does not change activation policy, process identity, or menus.
pub(super) fn init() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::{AnyThread, MainThreadMarker};
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::NSData;
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            "Application icon updates must run on the macOS main thread".to_string()
        })?;
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
}
