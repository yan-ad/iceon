//! Small native application integration; never changes GPUI's delegate.
//!
//! The icon bytes are embedded from the same assets/icons directory used by the
//! packager. This works for `cargo run` from any cwd and for relocated binaries.

#[cfg(any(target_os = "macos", test))]
const APP_ICON: &[u8] = include_bytes!("../../../assets/icons/icon.icns");

#[cfg(any(target_os = "macos", test))]
const APP_ICON_FOREGROUND: &[u8] =
    include_bytes!("../../../assets/icons/iceon.icon/Assets/transparent.png");

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

        let accent = crate::system_accent::current_accent_color();
        let rendered = accent.and_then(render_dock_icon);

        let image = if let Some(rendered) = rendered {
            rendered
        } else {
            let data = NSData::with_bytes(APP_ICON);
            NSImage::initWithData(NSImage::alloc(), &data)
                .ok_or_else(|| "Could not decode assets/icons/icon.icns".to_string())?
        };

        // SAFETY: A valid decoded, non-nil image is supplied on the main thread.
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn update_dock_icon(accent_rgb: u32) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if let Some(image) = render_dock_icon(accent_rgb) {
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn render_dock_icon(
    accent_rgb: u32,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSImage>> {
    use block2::RcBlock;
    use objc2::AnyThread;
    use objc2_app_kit::{NSBezierPath, NSColor, NSCompositingOperation, NSGradient, NSImage};
    use objc2_foundation::{NSData, NSPoint, NSRect, NSSize};

    let size = NSSize::new(1024.0, 1024.0);

    let fg_data = NSData::with_bytes(APP_ICON_FOREGROUND);
    let fg_image = NSImage::initWithData(NSImage::alloc(), &fg_data)?;

    let r = ((accent_rgb >> 16) & 0xff) as f64 / 255.0;
    let g = ((accent_rgb >> 8) & 0xff) as f64 / 255.0;
    let b = (accent_rgb & 0xff) as f64 / 255.0;

    let starting_color = NSColor::colorWithSRGBRed_green_blue_alpha(
        (r * 0.88).max(0.0),
        (g * 0.88).max(0.0),
        (b * 0.88).max(0.0),
        1.0,
    );
    let ending_color = NSColor::colorWithSRGBRed_green_blue_alpha(
        (r * 1.08).min(1.0),
        (g * 1.08).min(1.0),
        (b * 1.08).min(1.0),
        1.0,
    );

    let gradient = NSGradient::initWithStartingColor_endingColor(
        NSGradient::alloc(),
        &starting_color,
        &ending_color,
    );
    let solid_color = NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);

    let block = RcBlock::new(move |_rect: NSRect| -> objc2::runtime::Bool {
        let icon_rect = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(824.0, 824.0));
        let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(icon_rect, 185.0, 185.0);
        if let Some(ref grad) = gradient {
            grad.drawInBezierPath_angle(&path, 90.0);
        } else {
            solid_color.setFill();
            path.fill();
        }

        fg_image.drawInRect_fromRect_operation_fraction(
            NSRect::new(NSPoint::new(0.0, 0.0), size),
            NSRect::new(NSPoint::new(0.0, 0.0), size),
            NSCompositingOperation::SourceOver,
            1.0,
        );

        objc2::runtime::Bool::YES
    });

    Some(NSImage::imageWithSize_flipped_drawingHandler(
        size, false, &block,
    ))
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

    #[cfg(target_os = "macos")]
    #[test]
    fn foreground_logo_is_valid_png() {
        assert_eq!(&super::APP_ICON_FOREGROUND[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn render_dock_icon_creates_valid_image() {
        let image = super::render_dock_icon(0x007AFF);
        assert!(image.is_some());
    }
}
