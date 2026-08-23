//! Clipboard-first insert. A failed AT-SPI write is still a copy.

use crate::error::{Error, Result};

pub fn copy_text(text: &str) -> Result<()> {
    copy_plain_or_html(text, None)
}

/// Keep formatting when AT-SPI gave us HTML attributes; otherwise plain text.
pub fn copy_plain_or_html(text: &str, html: Option<&str>) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::copy_plain_or_html(text, html);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (text, html);
        Err(Error::msg(
            "clipboard copy is Linux-only in P0 (NSPasteboard / Win32 later)",
        ))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    pub fn copy_plain_or_html(text: &str, html: Option<&str>) -> Result<()> {
        if let Some(html) = html {
            if copy_data_control_both(text, html).is_ok() {
                return Ok(());
            }
        }
        match copy_data_control(text) {
            Ok(()) => return Ok(()),
            Err(e) => tracing_fallback(&e),
        }
        copy_wl_copy_bin(text)
    }

    fn copy_data_control_both(plain: &str, html: &str) -> Result<()> {
        use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};
        Options::new()
            .copy_multi(vec![
                MimeSource {
                    source: Source::Bytes(html.as_bytes().to_vec().into_boxed_slice()),
                    mime_type: MimeType::Specific("text/html".into()),
                },
                MimeSource {
                    source: Source::Bytes(plain.as_bytes().to_vec().into_boxed_slice()),
                    mime_type: MimeType::Text,
                },
            ])
            .map_err(|e| Error::msg(format!("wlr-data-control: {e}")))
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
