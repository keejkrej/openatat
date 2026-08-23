//! Linux C7: `wf-recorder` (preferred) or `gpu-screen-recorder`.
//!
//! Geometry is argv data. Never `sh -c`. Stop sends SIGINT, then waits for
//! the cache mp4. xdg-desktop-portal Screenshot is not used. PipeWire
//! ScreenCast is only mentioned when no region recorder is installed — it
//! does not replace grim / C1.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::policy::{
    argv_for_plan, no_region_recorder_message, resolve_linux_recorder, useful_encoder_error,
    RecorderPlan, RECORD_BIN_ENV,
};
use super::PickedRegion;
use crate::error::{Error, Result};

pub struct ChildRecording {
    child: Child,
    path: PathBuf,
    #[allow(dead_code)]
    plan: RecorderPlan,
}

pub fn start_recording(region: &PickedRegion, path: &Path) -> Result<ChildRecording> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let override_bin = std::env::var_os(RECORD_BIN_ENV);
    let plan = resolve_linux_recorder(std::env::var_os("PATH").as_deref(), override_bin.as_deref())
        .map_err(Error::msg)?;
    let argv = argv_for_plan(&plan, region.x, region.y, region.width, region.height, path);
    if argv.is_empty() {
        return Err(Error::msg(no_region_recorder_message()));
    }
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| {
        Error::msg(useful_encoder_error(&format!(
            "could not start `{}` ({e}). {}",
            argv[0].to_string_lossy(),
            no_region_recorder_message()
        )))
    })?;
    Ok(ChildRecording {
        child,
        path: path.to_path_buf(),
        plan,
    })
}

impl ChildRecording {
    pub fn stop(mut self) -> Result<PathBuf> {
        interrupt(&self.child)?;
        let status = wait_with_timeout(&mut self.child, Duration::from_secs(8))?;
        if !status.success() && file_ready(&self.path).is_err() {
            let stderr = self
                .child
                .stderr
                .take()
                .and_then(|mut s| {
                    let mut buf = String::new();
                    let _ = std::io::Read::read_to_string(&mut s, &mut buf);
                    Some(buf)
                })
                .unwrap_or_default();
            return Err(Error::msg(useful_encoder_error(&first_line(&stderr))));
        }
        wait_for_file(&self.path)?;
        Ok(self.path)
    }

    pub fn cancel(mut self) -> Result<()> {
        let _ = interrupt(&self.child);
        let _ = wait_with_timeout(&mut self.child, Duration::from_secs(2));
        let _ = std::fs::remove_file(&self.path);
        Ok(())
    }
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn interrupt(child: &Child) -> Result<()> {
    let pid = child.id() as i32;
    let r = unsafe { libc::kill(pid, libc::SIGINT) };
    if r != 0 {
        let r2 = unsafe { libc::kill(pid, libc::SIGTERM) };
        if r2 != 0 {
            return Err(Error::msg(useful_encoder_error(
                "could not signal the recorder (SIGINT/SIGTERM)",
            )));
        }
    }
    Ok(())
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => return Ok(status),
            None if start.elapsed() >= timeout => {
                let pid = child.id() as i32;
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                return child.wait().map_err(Error::from);
            }
            None => std::thread::sleep(Duration::from_millis(40)),
        }
    }
}

fn file_ready(path: &Path) -> Result<()> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(Error::msg(useful_encoder_error(
            "recorder exited but the mp4 is missing or empty",
        )));
    }
    Ok(())
}

fn wait_for_file(path: &Path) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(4) {
        if file_ready(path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Err(Error::msg(useful_encoder_error(
        "recorder stopped but no mp4 appeared under ~/.cache/openatat/record",
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::picker::PickedRegion;

    fn stub_script(dir: &Path) -> PathBuf {
        let stub = dir.join("openatat-record-stub");
        std::fs::write(
            &stub,
            r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-f" ] || [ "$1" = "-o" ]; then
    shift
    out="$1"
  fi
  shift
done
if [ -z "$out" ]; then
  echo "stub: missing -f/-o" >&2
  exit 2
fi
trap 'printf stub-mp4 > "$out"; exit 0' INT TERM
printf 'stub-mp4' > "$out"
while true; do sleep 0.05; done
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&stub).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&stub, p).unwrap();
        }
        stub
    }

    #[test]
    fn stub_recorder_stop_writes_cache_mp4_without_capturing() {
        let _g = crate::paths::xdg_test_lock();
        let dir = std::env::temp_dir().join(format!(
            "openatat-rec-stub-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let stub = stub_script(&dir);
        let cache = dir.join("cache");
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var(RECORD_BIN_ENV, &stub);
        let out = cache.join("openatat/record/test.mp4");
        let region = PickedRegion {
            x: 1,
            y: 2,
            width: 16,
            height: 16,
            output: None,
            surface_w: 16,
            surface_h: 16,
        };
        let rec = start_recording(&region, &out).expect("start stub recorder");
        // Give the stub a tick to create the file before SIGINT.
        std::thread::sleep(std::time::Duration::from_millis(80));
        let path = rec.stop().expect("stop stub recorder");
        assert_eq!(path, out);
        assert!(std::fs::read(&path).unwrap().starts_with(b"stub-mp4"));
        std::env::remove_var(RECORD_BIN_ENV);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn linux_source_never_shells_wf_recorder_or_uses_screenshot_portal() {
        let src = include_str!("linux.rs");
        assert!(src.contains("Command::new"));
        assert!(!src.contains("Command::new(\"sh\")"));
        assert!(!src.contains("std::process::Command::new(\"bash\")"));
        let ashpd = ["ash", "pd::"].concat();
        assert!(!src.contains(&ashpd));
        let shot = ["org.freedesktop.portal.", "Screenshot"].concat();
        assert!(!src.contains(&shot), "C7 must not call the Screenshot portal");
        let c1 = ["capture_active", "_output"].concat();
        assert!(!src.contains(&c1));
    }
}
