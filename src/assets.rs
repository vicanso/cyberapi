//! The desktop's assets: the kit's own icons, and the Lucide icons this app
//! uses beyond them, embedded. The browser fetches any icon from the page's
//! `assets/icons/`, so it needs no list.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(
    AppIcons,
    [
        ArchiveRestore,
        Braces,
        ChevronsDownUp,
        CircleUser,
        ClipboardPaste,
        Clock,
        Cloud,
        CloudOff,
        CloudUpload,
        Cookie,
        CopyPlus,
        DatabaseBackup,
        Download,
        FileCode,
        FilePlus,
        FolderInput,
        FolderPlus,
        Funnel,
        Heading,
        Import,
        Infinity,
        KeyRound,
        Keyboard,
        ListPlus,
        ListTree,
        Lock,
        LockOpen,
        LogOut,
        Monitor,
        Paperclip,
        Pencil,
        Pin,
        PinOff,
        Play,
        Plug,
        Send,
        Table,
        Terminal,
        Trash,
        Undo2,
        Ungroup,
        UserCog,
        Users,
        WandSparkles
    ]
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
            IconName::ArchiveRestore,
            IconName::Braces,
            IconName::ChevronsDownUp,
            IconName::CircleUser,
            IconName::ClipboardPaste,
            IconName::Clock,
            IconName::Cloud,
            IconName::CloudOff,
            IconName::CloudUpload,
            IconName::Cookie,
            IconName::CopyPlus,
            IconName::DatabaseBackup,
            IconName::Download,
            IconName::FileCode,
            IconName::FilePlus,
            IconName::FolderInput,
            IconName::FolderPlus,
            IconName::Heading,
            IconName::Import,
            IconName::Infinity,
            IconName::KeyRound,
            IconName::Keyboard,
            IconName::ListPlus,
            IconName::ListTree,
            IconName::LogOut,
            IconName::Monitor,
            IconName::Paperclip,
            IconName::Pencil,
            IconName::Pin,
            IconName::PinOff,
            IconName::Play,
            IconName::Send,
            IconName::Table,
            IconName::Terminal,
            IconName::Trash,
            IconName::Undo2,
            IconName::Ungroup,
            IconName::UserCog,
            IconName::Users,
            IconName::WandSparkles,
            IconName::FolderOpen,
            IconName::Folder,
            IconName::Globe,
            IconName::Search,
            IconName::Settings,
        ] {
            let path = icon.path();
            assert!(Assets.load(&path).ok().flatten().is_some(), "{path} is missing");
        }
    }
}
