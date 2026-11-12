//! Default component icons plus the extra Lucide assets used by Iceon.
use gpui_kit::{AssetSource, SharedString};
use std::borrow::Cow;

gpui_kit_assets::icon_assets!(ExtraIcons, [Pin, PinOff, Pencil, Trash]);

pub(crate) struct AppAssets;
impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::AppAssets;
    use gpui_kit::AssetSource;
    use gpui_kit_assets::IconName;

    #[test]
    fn pin_unpin_and_default_component_icons_are_embedded() {
        for icon in [
            IconName::Pin,
            IconName::PinOff,
            IconName::Pencil,
            IconName::Trash,
            IconName::Check,
        ] {
            let path = icon.path();
            let bytes = AppAssets.load(&path).unwrap().expect("embedded SVG");
            assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"));
            assert!(AppAssets.list("icons/").unwrap().contains(&path));
        }
    }
}
