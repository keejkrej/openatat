//! Overlay **Edit** tile: spawn `openatat-ui --studio`. Never opens gpui here.

use std::path::PathBuf;

use crate::capture::Still;

#[derive(Debug, Default)]
pub struct StudioAttach {
    source: Option<PathBuf>,
    last_png: Option<Vec<u8>>,
}

impl StudioAttach {
    /// Write the still to the cache and spawn studio. Missing UI binary logs
    /// the same rebuild hint Settings already uses.
    pub fn edit(&mut self, png: &[u8]) {
        match crate::ui_spawn::write_still_and_spawn(png) {
            Ok(path) => {
                let dest = crate::ui_spawn::annotated_path(&path);
                eprintln!(
                    "openatatd: studio spawned for {} — export writes {} (local PNG, never HTTP). \
                     If this overlay is still up, the next click or keystroke reloads that file into the tile. \
                     Otherwise fire @@ again after export, or drop the annotated PNG on the Orb.",
                    path.display(),
                    dest.display()
                );
                self.source = Some(path);
                self.last_png = None;
            }
            Err(e) => eprintln!("openatatd: {e}"),
        }
    }

    pub fn take_export(&mut self) -> Option<Vec<u8>> {
        let src = self.source.as_ref()?;
        let png = crate::ui_spawn::load_annotated_png(src)?;
        if self.last_png.as_ref() == Some(&png) {
            return None;
        }
        self.last_png = Some(png.clone());
        Some(png)
    }
}

pub fn still_from_png(png: Vec<u8>) -> Option<Still> {
    crate::capture::Still::from_png(png).ok()
}

pub fn apply_still_bytes(
    has_tile: bool,
    png: Option<Vec<u8>>,
    session: &mut crate::session::Session,
) {
    if !has_tile {
        session.still = None;
        return;
    }
    if let Some(png) = png {
        if let Some(still) = still_from_png(png) {
            session.still = Some(still);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_without_ui_binary_keeps_no_source() {
        let _g = crate::paths::xdg_test_lock();
        let dir = std::env::temp_dir().join(format!("openatat-attach-{}", uuid::Uuid::new_v4()));
        let old = std::env::var_os("XDG_CACHE_HOME");
        std::env::set_var("XDG_CACHE_HOME", &dir);
        let mut a = StudioAttach::default();
        a.edit(&[0, 1, 2]);
        // find_ui_bin fails in this workspace test, or spawn fails — either way
        // we must not open a gpui window from the applet.
        assert!(a.take_export().is_none());
        match old {
            Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
            None => std::env::remove_var("XDG_CACHE_HOME"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
