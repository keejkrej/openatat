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
    #[cfg(target_os = "macos")]
    {
        return macos::copy_plain_or_html(text, html);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::copy_plain_or_html(text, html);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (text, html);
        Err(Error::msg("clipboard copy is unsupported on this OS"))
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

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    pub fn copy_plain_or_html(text: &str, html: Option<&str>) -> Result<()> {
        let pb = unsafe { NSPasteboard::generalPasteboard() };
        pb.clearContents();
        let plain = NSString::from_str(text);
        if !pb.setString_forType(&plain, unsafe { NSPasteboardTypeString }) {
            return Err(Error::msg("NSPasteboard setString failed"));
        }
        if let Some(html) = html {
            let hs = NSString::from_str(html);
            let _ = pb.setString_forType(&hs, unsafe { NSPasteboardTypeHTML });
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use ::windows::core::HSTRING;
    use ::windows::Win32::Foundation::HWND;
    use ::windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use ::windows::Win32::System::Ole::CF_UNICODETEXT;
    use ::windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };

    pub fn copy_plain_or_html(text: &str, _html: Option<&str>) -> Result<()> {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);
        unsafe {
            OpenClipboard(Some(HWND::default()))
                .map_err(|e| Error::msg(format!("OpenClipboard: {e}")))?;
            let _ = EmptyClipboard();
            let bytes = wide.len() * 2;
            let hg = GlobalAlloc(GMEM_MOVEABLE, bytes)
                .map_err(|e| {
                    let _ = CloseClipboard();
                    Error::msg(format!("GlobalAlloc: {e}"))
                })?;
            let ptr = GlobalLock(hg);
            if ptr.is_null() {
                let _ = CloseClipboard();
                return Err(Error::msg("GlobalLock failed"));
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr as *mut u16, wide.len());
            let _ = GlobalUnlock(hg);
            if SetClipboardData(
                u32::from(CF_UNICODETEXT.0),
                Some(::windows::Win32::Foundation::HANDLE(hg.0)),
            )
            .is_err()
            {
                let _ = CloseClipboard();
                return Err(Error::msg("SetClipboardData failed"));
            }
            let _ = CloseClipboard();
        }
        let _ = HSTRING::new();
        Ok(())
    }
}
