//! Native operating system accent color provider and observer.
//!
//! Provides system accent colors on macOS (via NSColor controlAccentColor) and
//! Windows (via UISettings.GetColorValue(UIColorType.Accent)), falling back to
//! Iceon's default brand blue on Linux and unsupported environments.
//! Also synchronizes macOS `.icon` package gradient and dynamic Dock icon.

use std::path::Path;

#[cfg(target_os = "macos")]
pub(crate) struct SystemAccentObserver {
    center: objc2::rc::Retained<objc2_foundation::NSNotificationCenter>,
    token:
        objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>,
}

#[cfg(target_os = "macos")]
impl Drop for SystemAccentObserver {
    fn drop(&mut self) {
        unsafe {
            self.center.removeObserver(self.token.as_ref());
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) struct SystemAccentObserver {
    settings: windows::UI::ViewManagement::UISettings,
    token: windows::core::EventRegistrationToken,
}

#[cfg(target_os = "windows")]
impl Drop for SystemAccentObserver {
    fn drop(&mut self) {
        let _ = self.settings.RemoveColorValuesChanged(self.token);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) struct SystemAccentObserver;

/// Retrieve the active operating system accent color as `0xRRGGBB`.
pub(crate) fn current_accent_color() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSColor, NSColorSpace};
        let color = NSColor::controlAccentColor();
        let srgb = NSColorSpace::sRGBColorSpace();
        let srgb_color = color.colorUsingColorSpace(&srgb)?;
        let r = (srgb_color.redComponent() * 255.0)
            .round()
            .clamp(0.0, 255.0) as u32;
        let g = (srgb_color.greenComponent() * 255.0)
            .round()
            .clamp(0.0, 255.0) as u32;
        let b = (srgb_color.blueComponent() * 255.0)
            .round()
            .clamp(0.0, 255.0) as u32;
        Some((r << 16) | (g << 8) | b)
    }

    #[cfg(target_os = "windows")]
    {
        use windows::UI::ViewManagement::{UIColorType, UISettings};
        let settings = UISettings::new().ok()?;
        let color = settings.GetColorValue(UIColorType::Accent).ok()?;
        Some(((color.R as u32) << 16) | ((color.G as u32) << 8) | (color.B as u32))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Retrieve the active operating system accent color in Display P3 color space as `(r, g, b)`.
pub(crate) fn current_accent_display_p3() -> Option<(f64, f64, f64)> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSColor, NSColorSpace};
        let color = NSColor::controlAccentColor();
        let p3 = NSColorSpace::displayP3ColorSpace();
        let p3_color = color.colorUsingColorSpace(&p3)?;
        Some((
            p3_color.redComponent(),
            p3_color.greenComponent(),
            p3_color.blueComponent(),
        ))
    }

    #[cfg(not(target_os = "macos"))]
    {
        current_accent_color().map(|rgb| {
            let r = ((rgb >> 16) & 0xff) as f64 / 255.0;
            let g = ((rgb >> 8) & 0xff) as f64 / 255.0;
            let b = (rgb & 0xff) as f64 / 255.0;
            (r, g, b)
        })
    }
}

/// Update an `.icon` package's `icon.json` fill automatic-gradient with Display P3 values.
pub(crate) fn update_icon_json(path: &Path, r: f64, g: f64, b: f64) -> Result<(), std::io::Error> {
    if !path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(path)?;
    let gradient_line =
        format!("\"automatic-gradient\" : \"display-p3:{r:.5},{g:.5},{b:.5},1.00000\"");
    let mut updated_lines = Vec::new();
    for line in content.lines() {
        if line.contains("\"automatic-gradient\"") {
            let indent = line.find('"').unwrap_or(0);
            let spaces = " ".repeat(indent);
            updated_lines.push(format!("{spaces}{gradient_line}"));
        } else {
            updated_lines.push(line.to_string());
        }
    }
    let updated = updated_lines.join("\n") + "\n";
    std::fs::write(path, updated)
}

/// Synchronize known macOS `.icon` directories (`assets/icons/iceon.icon` and user download path).
pub(crate) fn sync_icon_packages(r: f64, g: f64, b: f64) {
    let repo_icon =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/iceon.icon/icon.json");
    let _ = update_icon_json(&repo_icon, r, g, b);

    if let Some(home) = iceon_core::runtime::home_dir() {
        let downloads_icon = home.join("Downloads/iceon.icon/icon.json");
        let _ = update_icon_json(&downloads_icon, r, g, b);
    }
}

/// Register a native listener for OS accent color changes.
pub(crate) fn watch_system_accent<F: Fn() + Send + Sync + 'static>(
    callback: F,
) -> Option<SystemAccentObserver> {
    #[cfg(target_os = "macos")]
    {
        use block2::RcBlock;
        use objc2_app_kit::NSSystemColorsDidChangeNotification;
        use objc2_foundation::NSNotificationCenter;

        let center = NSNotificationCenter::defaultCenter();
        let block = RcBlock::new(
            move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                callback();
            },
        );
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSSystemColorsDidChangeNotification),
                None,
                None,
                &block,
            )
        };
        Some(SystemAccentObserver { center, token })
    }

    #[cfg(target_os = "windows")]
    {
        use windows::Foundation::TypedEventHandler;
        use windows::UI::ViewManagement::UISettings;

        let settings = UISettings::new().ok()?;
        let cb = std::sync::Arc::new(callback);
        let handler = TypedEventHandler::new(move |_, _| {
            cb();
            Ok(())
        });
        let token = settings.ColorValuesChanged(&handler).ok()?;
        Some(SystemAccentObserver { settings, token })
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = callback;
        None
    }
}

struct GlobalSystemAccent {
    _observer: Option<SystemAccentObserver>,
}
impl gpui_kit::Global for GlobalSystemAccent {}

/// Initialize system accent integration and real-time change listener.
pub(crate) fn init(cx: &mut gpui_kit::App) {
    // Initial sync of the .icon package to match system accent
    if let Some((r, g, b)) = current_accent_display_p3() {
        sync_icon_packages(r, g, b);
    }

    use futures::StreamExt as _;
    let (sender, mut receiver) = futures::channel::mpsc::unbounded::<()>();

    let observer = watch_system_accent(move || {
        let _ = sender.unbounded_send(());
    });

    cx.spawn(async move |cx| {
        while receiver.next().await.is_some() {
            cx.update(|cx| {
                if let Some((r, g, b)) = current_accent_display_p3() {
                    sync_icon_packages(r, g, b);
                }
                #[cfg(target_os = "macos")]
                if let Some(accent) = current_accent_color() {
                    crate::native_menu::native_app::update_dock_icon(accent);
                }
                crate::shell_theme::apply_current(cx);
                cx.refresh_windows();
            });
        }
    })
    .detach();

    cx.set_global(GlobalSystemAccent {
        _observer: observer,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_accent_color_is_bounded_or_none() {
        if let Some(accent) = current_accent_color() {
            assert!(
                accent <= 0xFFFFFF,
                "accent color should be standard 24-bit RGB"
            );
        }
    }

    #[test]
    fn current_accent_display_p3_is_normalized() {
        if let Some((r, g, b)) = current_accent_display_p3() {
            assert!((0.0..=1.0).contains(&r), "r component in bounds: {r}");
            assert!((0.0..=1.0).contains(&g), "g component in bounds: {g}");
            assert!((0.0..=1.0).contains(&b), "b component in bounds: {b}");
        }
    }

    #[test]
    fn update_icon_json_preserves_structure() {
        let dir = std::env::temp_dir().join(format!("iceon-icon-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let icon_file = dir.join("icon.json");
        let initial_json = r#"{
  "fill" : {
    "automatic-gradient" : "display-p3:0.05490,0.28627,0.72157,1.00000"
  },
  "groups" : [
    {
      "layers" : [
        {
          "image-name" : "iceon-transparent 2.png",
          "name" : "iceon-transparent 2"
        }
      ]
    }
  ]
}"#;
        std::fs::write(&icon_file, initial_json).unwrap();

        update_icon_json(&icon_file, 0.96863, 0.30980, 0.61961).unwrap();
        let updated = std::fs::read_to_string(&icon_file).unwrap();

        assert!(
            updated.contains(
                "\"automatic-gradient\" : \"display-p3:0.96863,0.30980,0.61961,1.00000\""
            ),
            "updated json contains new gradient: {updated}"
        );
        assert!(
            updated.contains("\"image-name\" : \"iceon-transparent 2.png\""),
            "structure preserved"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_standard_accent_colors_compute_valid_contrast_and_shades() {
        // Known standard macOS & Windows accent colors
        let accents = [
            ("Blue", 0x007AFF),
            ("Purple", 0xAF52DE),
            ("Pink", 0xFF2D55),
            ("Red", 0xFF3B30),
            ("Orange", 0xFF9500),
            ("Yellow", 0xFFCC00),
            ("Green", 0x34C759),
            ("Graphite", 0x8E8E93),
        ];

        for (name, rgb) in accents {
            let mut colors = gpui_kit::component::theme::ThemeColor::default();
            crate::shell_theme::brand_colors_with_accent(
                &mut colors,
                gpui_kit::component::theme::ThemeMode::Dark,
                Some(rgb),
            );

            assert_eq!(
                colors.primary,
                gpui_kit::Hsla::from(gpui_kit::rgb(rgb)).alpha(1.0),
                "accent {name} primary color applied"
            );
            assert_ne!(
                colors.primary_hover, colors.primary,
                "{name} hover is distinct"
            );
            assert_ne!(
                colors.primary_active, colors.primary,
                "{name} active is distinct"
            );
            assert!(
                colors.primary_foreground.a >= 0.99,
                "{name} foreground has full opacity"
            );
        }
    }

    #[test]
    fn custom_zed_theme_is_never_overridden_by_system_accent() {
        use iceon_core::zed_themes::ZedTheme;
        let mut colors = gpui_kit::component::theme::ThemeColor::default();
        let custom_color = 0x336699;
        let zed = ZedTheme {
            id: "fixture/Custom".into(),
            extension: "fixture".into(),
            name: "Custom Theme".into(),
            appearance: "dark".into(),
            style: [("element.active".into(), "#336699".into())]
                .into_iter()
                .collect(),
        };

        crate::shell_theme::brand_colors_with_accent(
            &mut colors,
            gpui_kit::component::theme::ThemeMode::Dark,
            Some(0xFF2D55),
        );
        assert_eq!(
            colors.primary,
            gpui_kit::Hsla::from(gpui_kit::rgb(0xFF2D55)).alpha(1.0)
        );

        crate::shell_theme::apply_zed_theme(&mut colors, &zed);
        assert_eq!(
            colors.secondary_active,
            gpui_kit::Hsla::from(gpui_kit::rgb(custom_color)).alpha(1.0)
        );
    }

    #[test]
    fn sync_icon_packages_updates_actual_files() {
        if let Some((r, g, b)) = current_accent_display_p3() {
            sync_icon_packages(r, g, b);
            let repo_icon = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/icons/iceon.icon/icon.json");
            let content = std::fs::read_to_string(repo_icon).unwrap();
            let expected_gradient = format!("display-p3:{r:.5},{g:.5},{b:.5},1.00000");
            assert!(
                content.contains(&expected_gradient),
                "expected gradient {expected_gradient} in {content}"
            );
        }
    }
}
