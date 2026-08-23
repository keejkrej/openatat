//! Display-free C7 policy. Linux CI locks argv and cache paths without a GPU
//! and without wf-recorder actually capturing.
//!
//! Geometry is one argv element (`-g`, `x,y WxH`). Never a shell string.
//! Portal Screenshot is illegal on C1 and is not used here. PipeWire
//! ScreenCast is a last-resort fallback only when no region recorder is
//! installed — it does not replace grim.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Preferred Linux region recorder (Omarchy / Hyprland).
pub const WF_RECORDER: &str = "wf-recorder";
/// Fallback region recorder when wf-recorder is missing.
pub const GPU_SCREEN_RECORDER: &str = "gpu-screen-recorder";
/// Test / override binary. Same argv shape as wf-recorder (`-g` + `-f`).
pub const RECORD_BIN_ENV: &str = "OPENATAT_RECORD_BIN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderKind {
    WfRecorder,
    GpuScreenRecorder,
    /// `OPENATAT_RECORD_BIN` or a user-supplied argv[0]. Same `-g`/`-f` as wf-recorder.
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderPlan {
    pub kind: RecorderKind,
    pub bin: OsString,
}

/// Cache path: `~/.cache/openatat/record/<id>.mp4` (XDG). Desktop Save is later.
pub fn record_output_path(id: &str) -> PathBuf {
    crate::paths::record_dir().join(format!("{id}.mp4"))
}

pub fn wf_recorder_geometry(x: i32, y: i32, w: u32, h: u32) -> String {
    format!("{x},{y} {w}x{h}")
}

/// `wf-recorder -g <geom> -f <path>` as separate argv elements.
pub fn wf_recorder_argv(bin: impl AsRef<OsStr>, geom: &str, out: &Path) -> Vec<OsString> {
    vec![
        bin.as_ref().to_os_string(),
        OsString::from("-g"),
        OsString::from(geom),
        OsString::from("-f"),
        out.as_os_str().to_os_string(),
    ]
}

/// `gpu-screen-recorder -w region -region X Y W H -c mp4 -o <path>`.
/// Integers stay their own argv elements — never interpolated into `sh -c`.
pub fn gpu_screen_recorder_argv(
    bin: impl AsRef<OsStr>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    out: &Path,
) -> Vec<OsString> {
    vec![
        bin.as_ref().to_os_string(),
        OsString::from("-w"),
        OsString::from("region"),
        OsString::from("-region"),
        OsString::from(x.to_string()),
        OsString::from(y.to_string()),
        OsString::from(w.to_string()),
        OsString::from(h.to_string()),
        OsString::from("-c"),
        OsString::from("mp4"),
        OsString::from("-o"),
        out.as_os_str().to_os_string(),
    ]
}

pub fn argv_for_plan(plan: &RecorderPlan, x: i32, y: i32, w: u32, h: u32, out: &Path) -> Vec<OsString> {
    match plan.kind {
        RecorderKind::GpuScreenRecorder => {
            gpu_screen_recorder_argv(&plan.bin, x, y, w, h, out)
        }
        RecorderKind::WfRecorder | RecorderKind::Custom => {
            wf_recorder_argv(&plan.bin, &wf_recorder_geometry(x, y, w, h), out)
        }
    }
}

/// First executable on PATH. `override_bin` wins (tests / `OPENATAT_RECORD_BIN`).
pub fn resolve_linux_recorder(
    path_env: Option<&OsStr>,
    override_bin: Option<&OsStr>,
) -> Result<RecorderPlan, String> {
    if let Some(bin) = override_bin {
        if !bin.is_empty() {
            return Ok(RecorderPlan {
                kind: RecorderKind::Custom,
                bin: bin.to_os_string(),
            });
        }
    }
    if let Some(bin) = look_up(WF_RECORDER, path_env) {
        return Ok(RecorderPlan {
            kind: RecorderKind::WfRecorder,
            bin,
        });
    }
    if let Some(bin) = look_up(GPU_SCREEN_RECORDER, path_env) {
        return Ok(RecorderPlan {
            kind: RecorderKind::GpuScreenRecorder,
            bin,
        });
    }
    Err(no_region_recorder_message())
}

pub fn no_region_recorder_message() -> String {
    format!(
        "C7 recording needs `{WF_RECORDER}` (preferred: `wf-recorder -g <x,y WxH> -f <cache mp4>`) \
         or `{GPU_SCREEN_RECORDER}`. On Omarchy: `pacman -S wf-recorder`. \
         PipeWire ScreenCast portal is only a fallback when a region recorder is missing \
         and cannot encode a file by itself. xdg-desktop-portal Screenshot is never used \
         (that would replace C1)."
    )
}

/// ScreenCast (not Screenshot) is the only portal C7 may mention.
pub fn record_portal_interface() -> &'static str {
    "org.freedesktop.portal.ScreenCast"
}

pub fn c1_may_use_portal_screenshot() -> bool {
    false
}

pub fn look_up(name: &str, path_env: Option<&OsStr>) -> Option<OsString> {
    let path = path_env
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::var_os("PATH").map(|p| p.to_string_lossy().into_owned()))?;
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate.into_os_string());
        }
    }
    None
}

pub fn format_elapsed(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m:02}:{s:02}")
}

/// Encoder failures copy this text (not the prompt) to the clipboard.
pub fn useful_encoder_error(detail: &str) -> String {
    let detail = detail.trim();
    if detail.is_empty() {
        "C7 recorder failed to write a local mp4. Install wf-recorder, or check Screen Recording / graphics-capture permission.".into()
    } else {
        format!("C7 recorder failed: {detail}")
    }
}

/// argv must never be joined for `sh -c`.
pub fn argv_is_shell_string(args: &[OsString]) -> bool {
    args.iter().any(|a| {
        let s = a.to_string_lossy();
        s == "sh" || s.contains("sh -c") || s.contains("wf-recorder -g")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wf_recorder_argv_is_data_not_a_shell_string() {
        let out = Path::new("/tmp/openatat/record/abc.mp4");
        let args = wf_recorder_argv(WF_RECORDER, "10,20 30x40", out);
        assert_eq!(
            args,
            vec![
                OsString::from("wf-recorder"),
                OsString::from("-g"),
                OsString::from("10,20 30x40"),
                OsString::from("-f"),
                OsString::from("/tmp/openatat/record/abc.mp4"),
            ]
        );
        assert!(!argv_is_shell_string(&args));
        assert!(!args.iter().any(|a| a.to_string_lossy().contains(';')));
        let joined_forbidden = format!("wf-recorder -g {}", "10,20 30x40");
        assert!(!args.iter().any(|a| a.to_string_lossy() == joined_forbidden));
    }

    #[test]
    fn gpu_screen_recorder_region_is_separate_argv() {
        let out = Path::new("/tmp/out.mp4");
        let args = gpu_screen_recorder_argv(GPU_SCREEN_RECORDER, 8, 9, 16, 24, out);
        assert_eq!(args[0], GPU_SCREEN_RECORDER);
        assert_eq!(args[1], "-w");
        assert_eq!(args[2], "region");
        assert_eq!(args[4], "8");
        assert_eq!(args[5], "9");
        assert_eq!(args[6], "16");
        assert_eq!(args[7], "24");
        assert!(!argv_is_shell_string(&args));
    }

    #[test]
    fn resolve_prefers_wf_recorder_then_gpu_then_override() {
        let dir = std::env::temp_dir().join(format!(
            "openatat-rec-path-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let wf = dir.join(WF_RECORDER);
        std::fs::write(&wf, b"#!/bin/true\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&wf).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&wf, p).unwrap();
        }
        let path = dir.as_os_str();
        let plan = resolve_linux_recorder(Some(path), None).unwrap();
        assert_eq!(plan.kind, RecorderKind::WfRecorder);
        let custom = resolve_linux_recorder(Some(path), Some(OsStr::new("/opt/fake-rec"))).unwrap();
        assert_eq!(custom.kind, RecorderKind::Custom);
        assert_eq!(custom.bin, OsString::from("/opt/fake-rec"));
        std::fs::remove_file(&wf).unwrap();
        let gpu = dir.join(GPU_SCREEN_RECORDER);
        std::fs::write(&gpu, b"#!/bin/true\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&gpu).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&gpu, p).unwrap();
        }
        let plan = resolve_linux_recorder(Some(path), None).unwrap();
        assert_eq!(plan.kind, RecorderKind::GpuScreenRecorder);
        std::fs::remove_file(&gpu).unwrap();
        let err = resolve_linux_recorder(Some(path), None).unwrap_err();
        assert!(err.contains(WF_RECORDER));
        assert!(err.contains("ScreenCast"));
        assert!(err.contains("Screenshot"));
        assert!(err.contains("never used"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_path_is_record_id_mp4() {
        let p = record_output_path("deadbeef");
        assert!(p.ends_with("record/deadbeef.mp4"));
        assert_eq!(p.file_name().unwrap(), "deadbeef.mp4");
    }

    #[test]
    fn elapsed_is_mm_ss() {
        assert_eq!(format_elapsed(0), "00:00");
        assert_eq!(format_elapsed(65), "01:05");
    }

    #[test]
    fn encoder_error_is_not_a_prompt() {
        let msg = useful_encoder_error("wf-recorder: no such output");
        assert!(msg.contains("wf-recorder: no such output"));
        assert!(!msg.contains("@@"));
        assert!(!msg.to_ascii_lowercase().contains("prompt"));
        assert_eq!(record_portal_interface(), "org.freedesktop.portal.ScreenCast");
        assert!(!c1_may_use_portal_screenshot());
    }
}
