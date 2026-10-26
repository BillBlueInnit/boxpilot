use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        Rss,
        Zap,
        Power,
        Link,
        Pencil,
        Download,
        Shield,
        ShieldCheck,
        Activity,
        ScrollText,
        Gauge,
        Unplug,
        ArrowDownUp,
        Cable,
        Earth,
        Route,
        Waypoints,
        ListFilter,
        CirclePlay,
        CircleStop,
        ClipboardPaste,
        Trash,
        Timer,
        FolderOpen,
        ListChecks,
    ]
);

/// Default component icons plus the extra Lucide icons this app uses.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
