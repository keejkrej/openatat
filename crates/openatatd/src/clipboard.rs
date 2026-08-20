//! Clipboard-first insert. A failed AT-SPI write is still a copy.

use crate::error::{Error, Result};

pub fn copy_text(text: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::copy_text(text);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = text;
        Err(Error::msg(
            "clipboard copy is Linux-only in P0 (NSPasteboard / Win32 later)",
        ))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    pub fn copy_text(text: &str) -> Result<()> {
        match copy_data_control(text) {
            Ok(()) => return Ok(()),
            Err(e) => tracing_fallback(&e),
        }
        copy_wl_copy_bin(text)
    }

    fn tracing_fallback(err: &Error) {
        eprintln!("openatatd: wlr-data-control copy failed ({err}); trying wl-copy");
    }

    fn copy_data_control(text: &str) -> Result<()> {
        use wl_clipboard_rs::copy::{MimeType, Options, Source};
        Options::new()
            .copy(
                Source::Bytes(text.as_bytes().to_vec().into_boxed_slice()),
                MimeType::Text,
            )
            .map_err(|e| Error::msg(format!("wlr-data-control: {e}")))
    }

    fn copy_wl_copy_bin(text: &str) -> Result<()> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::msg(format!("wl-copy: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        let status = child.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::msg("wl-copy exited unsuccessfully"))
        }
    }
}
