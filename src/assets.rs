//! The desktop's assets: the kit's own icons, and the Lucide icons this app
//! uses beyond them, embedded. The browser fetches any icon from the page's
//! `assets/icons/`, so it needs no list.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(
    AppIcons,
    [Clock, Infinity, Paperclip, Send, Terminal, Trash, WandSparkles]
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match AppIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(AppIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::assets::IconName;

    #[test]
    fn every_icon_the_app_uses_is_embedded() {
        for icon in [
            IconName::Clock,
            IconName::Infinity,
            IconName::Paperclip,
            IconName::Send,
            IconName::Terminal,
            IconName::Trash,
            IconName::WandSparkles,
            IconName::FolderOpen,
        ] {
            let path = icon.path();
            assert!(Assets.load(&path).ok().flatten().is_some(), "{path} is missing");
        }
    }
}
