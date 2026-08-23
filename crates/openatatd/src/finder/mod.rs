//! Finder working-directory + selection tiles (C11/C12). Mac only.
//!
//! Linux Nautilus has no selection D-Bus API — this module never invents one.
//! Right-click Service is out of scope.

mod policy;

pub use policy::{
    automation_stderr_is_denied, decide_finder_tiles, frontmost_is_finder, looks_like_finder,
    parse_automation_payload, paths_from_title_bar, probe_is_usable, tile_label, AutomationError,
    FinderPaths, FinderProbe,
};

use openatat_ipc::FocusSnapshot;

/// Probe the frontmost file manager. Title is never used as a path.
pub fn probe(focus: &FocusSnapshot) -> FinderProbe {
    #[cfg(target_os = "macos")]
    {
        return macos::probe(focus);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = focus;
        FinderProbe::NotFinder
    }
}

#[cfg(target_os = "macos")]
mod macos;
