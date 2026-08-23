//! File-manager working-directory + selection tiles (C11/C12).
//!
//! macOS: Finder Automation. Windows: Explorer `IShellWindows` → `IFolderView`.
//! Linux Nautilus has no selection D-Bus API — this module never invents one.
//! Title bars are never parsed. Context-menu DLLs / right-click Service wait.

mod policy;

pub use policy::{
    automation_stderr_is_denied, decide_finder_tiles, frontmost_is_explorer, frontmost_is_finder,
    looks_like_explorer, looks_like_finder, parse_automation_payload, paths_from_title_bar,
    probe_is_usable, tile_label, AutomationError, FinderPaths, FinderProbe,
};

use openatat_ipc::FocusSnapshot;

/// Probe the frontmost file manager. Title is never used as a path.
pub fn probe(focus: &FocusSnapshot) -> FinderProbe {
    #[cfg(target_os = "macos")]
    {
        return macos::probe(focus);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::probe(focus);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = focus;
        FinderProbe::NotFinder
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
