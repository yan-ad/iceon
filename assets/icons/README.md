# Iceon icons

These are native branding assets, not a generated frontend asset bundle.

| File | Current use |
| --- | --- |
| `icon.png` | Embedded in the GPUI app by `crates/iceon-ui/src/app_assets.rs` |
| `128x128.png` | Repository README logo |
| `32x32.png` | Embedded macOS status/tray icon in `crates/iceon-ui/src/tray.rs` |
| `icon.icns` | macOS `Iceon.app` bundle icon, copied by the native packager |
| `icon.ico` | Windows branding source; not currently embedded by portable ZIP packaging |

GPUI component/Lucide icons are embedded separately through Rust asset sources.
Changing a source icon requires rebuilding the application or package; no external
icon directory needs to be installed beside the executable.
