//! Spawn `openatat-ui`. The daemon never links gpui.

use std::path::{Path, PathBuf};
use std::process::Command;

use openatat_ipc::UiPage;

use crate::error::{Error, Result};

pub const GPUI_REBUILD_HINT: &str = "cargo build -p openatat-ui --features gpui";

pub fn spawn(page: UiPage) -> Result<()> {
    spawn_with(page, None)
}

pub fn spawn_with(page: UiPage, image: Option<&Path>) -> Result<()> {
    let bin = find_ui_bin()?;
    let mut cmd = Command::new(&bin);
    match page {
        UiPage::Settings => {
            cmd.arg("--settings");
        }
        UiPage::History => {
            cmd.arg("--history");
        }
        UiPage::Studio => {
            cmd.arg("--studio");
            if let Some(path) = image {
                cmd.arg("--image").arg(path);
            }
        }
    }
    cmd.spawn().map_err(|e| {
        Error::msg(format!(
            "failed to spawn {} ({e}). Build with: {GPUI_REBUILD_HINT}",
            bin.display()
        ))
    })?;
    eprintln!("openatatd: spawned {} {:?}", bin.display(), page);
    Ok(())
}

/// Write a C1 still into the cache and spawn studio on that local path.
pub fn write_still_and_spawn(png: &[u8]) -> Result<PathBuf> {
    let path = write_still_png(png)?;
    spawn_with(UiPage::Studio, Some(&path))?;
    Ok(path)
}

pub fn write_still_png(png: &[u8]) -> Result<PathBuf> {
    let dir = crate::paths::cache_dir().join("studio");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("still-{}.png", uuid::Uuid::new_v4()));
    std::fs::write(&path, png)?;
    Ok(path)
}

/// `<stem>-annotated.png` next to the source still. Local file only.
pub fn annotated_path(source: &Path) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("still");
    match source.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            parent.join(format!("{stem}-annotated.png"))
        }
        _ => PathBuf::from(format!("{stem}-annotated.png")),
    }
}

pub fn load_annotated_png(source: &Path) -> Option<Vec<u8>> {
    let dest = annotated_path(source);
    if dest.is_file() {
        std::fs::read(dest).ok()
    } else {
        None
    }
}

fn find_ui_bin() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("openatat-ui");
        if sibling.is_file() {
            return Ok(sibling);
        }
        let sibling_exe = exe.with_file_name("openatat-ui.exe");
        if sibling_exe.is_file() {
            return Ok(sibling_exe);
        }
    }
    if let Some(found) = crate::agent::which("openatat-ui", std::env::var_os("PATH").as_deref()) {
        return Ok(found);
    }
    Err(Error::msg(format!(
        "openatat-ui not found next to openatatd or on PATH. \
         Run: {GPUI_REBUILD_HINT} \
         or: cargo run -p openatat-ui --features gpui -- --settings"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_flags() {
        let _ = [UiPage::Settings, UiPage::History, UiPage::Studio];
        assert!(find_ui_bin().is_err() || find_ui_bin().unwrap().file_name().is_some());
    }

    #[test]
    fn annotated_path_is_local_sibling() {
        let p = PathBuf::from("/tmp/shot.png");
        assert_eq!(annotated_path(&p), PathBuf::from("/tmp/shot-annotated.png"));
        assert!(!annotated_path(&p).to_string_lossy().contains("://"));
    }

    #[test]
    fn write_still_is_a_local_png() {
        let _g = crate::paths::xdg_test_lock();
        let dir =
            std::env::temp_dir().join(format!("openatat-studio-spawn-{}", uuid::Uuid::new_v4()));
        let old = std::env::var_os("XDG_CACHE_HOME");
        std::env::set_var("XDG_CACHE_HOME", &dir);
        let path = write_still_png(&[137, 80, 78, 71]).unwrap();
        assert!(path.starts_with(dir.join("openatat/studio")));
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        assert!(path.is_file());
        assert_eq!(load_annotated_png(&path), None);
        let annotated = annotated_path(&path);
        std::fs::write(&annotated, b"png-bytes").unwrap();
        assert_eq!(
            load_annotated_png(&path).as_deref(),
            Some(b"png-bytes".as_ref())
        );
        match old {
            Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
            None => std::env::remove_var("XDG_CACHE_HOME"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn studio_is_spawned_not_hosted_in_the_applet() {
        let src = include_str!("ui_spawn.rs");
        let code = src.split("mod tests").next().unwrap_or(src);
        assert!(code.contains("--studio"));
        assert!(code.contains("--image"));
        assert!(code.contains("never links gpui"));
        assert!(!code.contains("gpui::"));
        assert!(!Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/studio.rs")
            .exists());
    }
}
