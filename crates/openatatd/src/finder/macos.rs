//! Finder Automation. Real POSIX paths only. Title bar is never consulted.

use std::process::Command;

use openatat_ipc::FocusSnapshot;

use super::policy::{
    automation_stderr_is_denied, decide_finder_tiles, frontmost_is_finder, parse_automation_payload,
    AutomationError, FinderProbe,
};

/// Static script. No interpolation of window titles or user strings.
const FINDER_SCRIPT: &str = r#"
tell application "Finder"
    set insertPath to ""
    try
        set insertPath to POSIX path of (insertion location as alias)
    end try
    set pathList to {}
    repeat with s in (get selection)
        try
            set end of pathList to POSIX path of (s as alias)
        end try
    end repeat
    set astid to AppleScript's text item delimiters
    set AppleScript's text item delimiters to linefeed
    set selText to pathList as text
    set AppleScript's text item delimiters to astid
    return insertPath & (ASCII character 29) & selText
end tell
"#;

pub fn probe(focus: &FocusSnapshot) -> FinderProbe {
    let is_finder = frontmost_is_finder(focus.app_id.as_deref(), None);
    if !is_finder {
        return FinderProbe::NotFinder;
    }
    let automation = run_automation();
    // Title is passed only so the policy can prove it is unused.
    decide_finder_tiles(true, automation, focus.title.as_deref())
}

fn run_automation() -> Result<super::FinderPaths, AutomationError> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(FINDER_SCRIPT)
        .output()
        .map_err(|_| AutomationError::Failed)?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        if automation_stderr_is_denied(&stderr) {
            return Err(AutomationError::Denied);
        }
        eprintln!("openatatd: Finder Automation failed ({stderr})");
        return Err(AutomationError::Failed);
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(parse_automation_payload(&stdout))
}
