//! Finder (C11/C12) policy. Display-free: never guess paths from a title bar.

use std::path::{Path, PathBuf};

/// Result of a Finder probe. Automation denied must not invent paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinderProbe {
    /// Frontmost app is not Finder.
    NotFinder,
    /// Finder is frontmost but Automation was denied / failed. Do not guess.
    Denied,
    /// Real POSIX paths from Finder Automation.
    Ready {
        cwd: Option<PathBuf>,
        files: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinderPaths {
    pub cwd: Option<PathBuf>,
    pub files: Vec<PathBuf>,
}

/// How Automation failed. Both variants stay [`FinderProbe::Denied`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationError {
    Denied,
    Failed,
}

/// Bundle / app-id check. Title is intentionally unused.
pub fn frontmost_is_finder(app_id: Option<&str>, bundle_or_name: Option<&str>) -> bool {
    [app_id, bundle_or_name].into_iter().flatten().any(looks_like_finder)
}

pub fn looks_like_finder(id: &str) -> bool {
    let s = id.trim();
    s.eq_ignore_ascii_case("com.apple.finder")
        || s.eq_ignore_ascii_case("finder")
        || s.eq_ignore_ascii_case("Finder")
}

/// Combine frontmost + Automation. `title` is accepted only so callers can
/// pass it — it is never parsed into a path.
pub fn decide_finder_tiles(
    frontmost_is_finder: bool,
    automation: Result<FinderPaths, AutomationError>,
    title: Option<&str>,
) -> FinderProbe {
    let _ = title;
    if !frontmost_is_finder {
        return FinderProbe::NotFinder;
    }
    match automation {
        Ok(paths) => FinderProbe::Ready {
            cwd: paths.cwd.filter(|p| p.is_absolute()),
            files: paths
                .files
                .into_iter()
                .filter(|p| p.is_absolute())
                .collect(),
        },
        Err(AutomationError::Denied) | Err(AutomationError::Failed) => FinderProbe::Denied,
    }
}

/// Landmine: window titles like `~/Projects` or `/Users/me` are not paths.
/// This always returns `None` so a future caller cannot "just parse the title."
pub fn paths_from_title_bar(_title: &str) -> Option<PathBuf> {
    None
}

pub fn probe_is_usable(probe: &FinderProbe) -> bool {
    matches!(probe, FinderProbe::Ready { .. })
}

/// Parse the static AppleScript / osascript payload: `cwd<GS>file1\nfile2`.
/// Empty cwd is `None`. Relative fragments are dropped (Automation must be POSIX).
pub fn parse_automation_payload(raw: &str) -> FinderPaths {
    let (cwd_raw, rest) = match raw.split_once('\u{001d}') {
        Some(pair) => pair,
        None => match raw.split_once('\n') {
            Some(pair) => pair,
            None => (raw, ""),
        },
    };
    let cwd = abs_path(cwd_raw.trim());
    let files = rest
        .lines()
        .filter_map(|l| abs_path(l.trim()))
        .collect();
    FinderPaths { cwd, files }
}

fn abs_path(s: &str) -> Option<PathBuf> {
    if s.is_empty() {
        return None;
    }
    let p = PathBuf::from(s);
    p.is_absolute().then_some(p)
}

/// stderr / Apple Event codes that mean TCC Automation was refused.
pub fn automation_stderr_is_denied(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("not authorized")
        || s.contains("not allowed to send apple events")
        || s.contains("-1743")
        || s.contains("erræeventnotpermitted")
        || s.contains("erraeeventnotpermitted")
        || s.contains("osstatus -1743")
        || s.contains("error 1002")
        || s.contains("(-1743)")
}

pub fn tile_label(path: &Path, is_cwd: bool) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("file"));
    if is_cwd {
        format!("cwd {name}")
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_automation_does_not_use_title_bar() {
        let title = "/Users/me/Projects/openatat";
        let probe = decide_finder_tiles(
            true,
            Err(AutomationError::Denied),
            Some(title),
        );
        assert_eq!(probe, FinderProbe::Denied);
        assert!(paths_from_title_bar(title).is_none());
        assert!(!probe_is_usable(&probe));
    }

    #[test]
    fn failed_automation_is_also_denied_not_a_guess() {
        let probe = decide_finder_tiles(
            true,
            Err(AutomationError::Failed),
            Some("~/Documents"),
        );
        assert_eq!(probe, FinderProbe::Denied);
        assert!(paths_from_title_bar("~/Documents").is_none());
    }

    #[test]
    fn not_finder_ignores_even_successful_automation() {
        let probe = decide_finder_tiles(
            false,
            Ok(FinderPaths {
                cwd: Some(PathBuf::from("/Users/me")),
                files: vec![PathBuf::from("/Users/me/a.txt")],
            }),
            Some("/Users/me"),
        );
        assert_eq!(probe, FinderProbe::NotFinder);
    }

    #[test]
    fn ready_keeps_absolute_paths_only() {
        let probe = decide_finder_tiles(
            true,
            Ok(FinderPaths {
                cwd: Some(PathBuf::from("/Users/me/work")),
                files: vec![
                    PathBuf::from("/Users/me/work/a.rs"),
                    PathBuf::from("relative.txt"),
                ],
            }),
            Some("this title is a trap"),
        );
        match probe {
            FinderProbe::Ready { cwd, files } => {
                assert_eq!(cwd.as_deref(), Some(Path::new("/Users/me/work")));
                assert_eq!(files, vec![PathBuf::from("/Users/me/work/a.rs")]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn finder_bundle_ids() {
        assert!(frontmost_is_finder(Some("com.apple.finder"), None));
        assert!(frontmost_is_finder(None, Some("Finder")));
        assert!(!frontmost_is_finder(Some("com.apple.Safari"), Some("Safari")));
        assert!(!frontmost_is_finder(None, None));
    }

    #[test]
    fn parse_payload_splits_cwd_and_files() {
        let p = parse_automation_payload("/Users/me/work\u{001d}/Users/me/work/a.rs\n/Users/me/work/b.rs");
        assert_eq!(p.cwd.as_deref(), Some(Path::new("/Users/me/work")));
        assert_eq!(p.files.len(), 2);
        let empty = parse_automation_payload("");
        assert!(empty.cwd.is_none() && empty.files.is_empty());
    }

    #[test]
    fn denied_stderr_codes() {
        assert!(automation_stderr_is_denied("not authorized to send Apple events to Finder"));
        assert!(automation_stderr_is_denied("osascript: error -1743"));
        assert!(!automation_stderr_is_denied("syntax error"));
    }
}
