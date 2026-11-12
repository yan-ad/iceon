//! Default component icons plus the extra Lucide assets used by Iceon.
use gpui_kit::{AssetSource, SharedString};
use std::borrow::Cow;

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
        if path == crate::action_icons::ATTACH_PATH {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/square-arrow-up-left-exit.svg"
            ))));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
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
}
