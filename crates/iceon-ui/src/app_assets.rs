//! Default component icons plus the extra Lucide assets used by Iceon.
use gpui_kit::{AssetSource, SharedString};
use iceon_core::zed_icon_themes::ZedIconTheme;
use std::borrow::Cow;
use std::sync::{Arc, RwLock};

static ACTIVE_ICON_THEME: RwLock<Option<Arc<ZedIconTheme>>> = RwLock::new(None);

pub(crate) fn set_active_icon_theme(theme: Option<ZedIconTheme>) {
    if let Ok(mut lock) = ACTIVE_ICON_THEME.write() {
        *lock = theme.map(Arc::new);
    }
}

pub(crate) fn active_icon_theme() -> Option<Arc<ZedIconTheme>> {
    ACTIVE_ICON_THEME
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

pub(crate) fn init_icon_resolver(cx: &mut gpui_kit::App) {
    gpui_kit::component::set_icon_resolver(cx, |path, _cx| {
        let active = active_icon_theme()?;
        let rel_path = active.resolve_icon(path)?;
        Some(SharedString::from(format!(
            "zed-icons/{}/{}",
            active.extension, rel_path
        )))
    });
}

gpui_kit_assets::icon_assets!(
    ExtraIcons,
    [
        Archive,
        ArrowLeftRight,
        ArrowUpDown,
        BadgeCheck,
        Bell,
        Blocks,
        Box,
        Cable,
        CalendarClock,
        ChartPie,
        ClipboardCheck,
        Contact,
        Container,
        Copy,
        CopyCheck,
        Crown,
        Database,
        FileCheck,
        FileText,
        Folder,
        Gauge,
        HardDrive,
        HardDriveDownload,
        KeyRound,
        Layers,
        Layers2,
        LayoutDashboard,
        Link,
        Link2,
        ListChecks,
        ListOrdered,
        MapPin,
        Network,
        Package,
        PackageCheck,
        Pencil,
        Plug,
        Puzzle,
        Route,
        Server,
        Shield,
        ShieldAlert,
        ShieldCheck,
        Signpost,
        SlidersHorizontal,
        Square,
        Timer,
        Download,
        Trash,
        Logs,
        Shredder,
        RefreshCw,
        UserCog,
        Users,
        Webhook
    ]
);

pub(crate) struct AppAssets;
impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(rest) = path.strip_prefix("zed-icons/")
            && let Some((ext, rel)) = rest.split_once('/')
            && let Ok(bytes) = iceon_core::zed_icon_themes::read_svg(ext, rel)
        {
            return Ok(Some(Cow::Owned(bytes)));
        }
        if let Some((_, bytes)) = crate::brand_icons::ASSETS
            .iter()
            .find(|(name, _)| *name == path)
        {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if path == crate::action_icons::ATTACH_PATH {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/square-arrow-up-left-exit.svg"
            ))));
        }
        if path.starts_with("icons/")
            && !path.starts_with("icons/custom-brand/")
            && let Some(active) = active_icon_theme()
            && let Some(rel) = active.resolve_icon(path)
            && let Ok(bytes) = iceon_core::zed_icon_themes::read_svg(&active.extension, &rel)
        {
            return Ok(Some(Cow::Owned(bytes)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.extend(
            crate::brand_icons::ASSETS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        if crate::action_icons::ATTACH_PATH.starts_with(path) {
            paths.push(crate::action_icons::ATTACH_PATH.into());
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::AppAssets;
    use gpui_kit::AssetSource;
    #[test]
    fn all_custom_brand_svgs_are_embedded_and_listed() {
        let assets = crate::brand_icons::ASSETS;
        assert_eq!(assets.len(), 15);
        let listed = AppAssets.list("icons/custom-brand/").unwrap();
        assert_eq!(listed.len(), assets.len());
        for &(path, source) in assets {
            assert_eq!(AppAssets.load(path).unwrap().unwrap().as_ref(), source);
            assert!(std::str::from_utf8(source).unwrap().contains("<svg"));
            for prefix in ["", "icons/", "icons/custom-brand/", path] {
                assert!(
                    AppAssets
                        .list(prefix)
                        .unwrap()
                        .iter()
                        .any(|p| p.as_ref() == path)
                );
            }
        }
        assert!(AppAssets.load("icons/custom-brand/unknown.svg").is_err());
        assert!(AppAssets.list("not-icons/").unwrap().is_empty());
    }
    #[test]
    fn every_sidebar_destination_icon_has_an_embedded_svg() {
        for group in crate::navigation::CLUSTER_SECTIONS {
            for destination in std::iter::once(group.default_child)
                .chain(group.children.iter().map(|child| child.id))
            {
                let icon = crate::sidebar_view::section_asset_icon(destination);
                let path = icon.path();
                let bytes = AppAssets.load(&path).unwrap().expect("sidebar icon SVG");
                assert!(
                    std::str::from_utf8(&bytes).unwrap().contains("<svg"),
                    "{destination}: {path}"
                );
            }
        }
    }

    use gpui_kit_assets::IconName;

    #[test]
    fn extra_and_default_component_icons_are_embedded() {
        for icon in [
            IconName::Pencil,
            IconName::Trash,
            IconName::Logs,
            IconName::Shredder,
            IconName::RefreshCw,
            IconName::Check,
            IconName::Download,
            IconName::Layers2,
        ] {
            let path = icon.path();
            let bytes = AppAssets.load(&path).unwrap().expect("embedded SVG");
            assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"));
            assert!(AppAssets.list("icons/").unwrap().contains(&path));
        }
    }
    #[test]
    fn attach_action_svg_is_embedded() {
        let path = crate::action_icons::ATTACH_PATH;
        let bytes = AppAssets.load(path).unwrap().expect("attach SVG");
        assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"));
        assert!(
            AppAssets
                .list("icons/")
                .unwrap()
                .iter()
                .any(|p| p.as_ref() == path)
        );
    }
    #[test]
    fn zed_icon_theme_switching_and_fallback_to_default_pack() {
        use iceon_core::zed_icon_themes::{FileIconDef, ZedIconTheme};
        use std::collections::BTreeMap;

        super::set_active_icon_theme(None);
        let default_folder = AppAssets.load("icons/folder.svg").unwrap().unwrap();
        assert!(
            std::str::from_utf8(&default_folder)
                .unwrap()
                .contains("<svg")
        );

        let mut file_icons = BTreeMap::new();
        file_icons.insert(
            "custom".into(),
            FileIconDef {
                path: "./icons/custom.svg".into(),
            },
        );
        let theme = ZedIconTheme {
            id: "fixture-icons/Dark".into(),
            extension: "fixture-icons".into(),
            name: "Dark".into(),
            appearance: "dark".into(),
            directory_icons: None,
            chevron_icons: None,
            named_directory_icons: BTreeMap::new(),
            file_stems: BTreeMap::new(),
            file_suffixes: BTreeMap::new(),
            file_icons,
        };
        super::set_active_icon_theme(Some(theme));

        let fallback = AppAssets.load("icons/folder.svg").unwrap().unwrap();
        assert_eq!(fallback.as_ref(), default_folder.as_ref());

        super::set_active_icon_theme(None);
        assert!(super::active_icon_theme().is_none());
    }
}
