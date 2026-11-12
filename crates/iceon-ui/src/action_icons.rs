//! Shared Lucide action glyphs across inspector, lists and settings.
use gpui_kit::component::Icon;
pub(crate) const ATTACH_PATH: &str = "icons/square-arrow-up-left-exit.svg";
pub(crate) fn logs() -> Icon {
    Icon::new(gpui_kit_assets::IconName::Logs)
}
pub(crate) fn force_delete() -> Icon {
    Icon::new(gpui_kit_assets::IconName::Shredder)
}
pub(crate) fn delete() -> Icon {
    Icon::new(gpui_kit_assets::IconName::Trash)
}
pub(crate) fn refresh() -> Icon {
    Icon::new(gpui_kit_assets::IconName::RefreshCw)
}
pub(crate) fn attach() -> Icon {
    Icon::default().path(ATTACH_PATH)
}
