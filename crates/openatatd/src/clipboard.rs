//! Clipboard-first insert and C14 shelf read/write.
//!
//! A failed AT-SPI write is still a copy. Shelf reads never log payloads.

use crate::error::{Error, Result};
use crate::shelf::Incoming;

pub fn copy_text(text: &str) -> Result<()> {
    copy_plain_or_html(text, None)
}

/// Write a shelf item back (plain + html/rtf/image when present).
pub fn write_offer(
    plain: &str,
    html: Option<&str>,
    rtf: Option<&str>,
    image: Option<&[u8]>,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::write_offer(plain, html, rtf, image);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::write_offer(plain, html, rtf, image);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::write_offer(plain, html, rtf, image);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (plain, html, rtf, image);
        copy_plain_or_html(plain, html)
    }
}

/// Current clipboard offer. `None` if empty or the protocol is unavailable.
/// Never logs the payload.
pub fn read_offer() -> Option<Incoming> {
    #[cfg(target_os = "linux")]
    {
        return linux::read_offer();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::read_offer();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::read_offer();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
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

    pub fn write_offer(
        plain: &str,
        html: Option<&str>,
        _rtf: Option<&str>,
        image: Option<&[u8]>,
    ) -> Result<()> {
        use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};
        let mut parts = Vec::new();
        if let Some(html) = html {
            parts.push(MimeSource {
                source: Source::Bytes(html.as_bytes().to_vec().into_boxed_slice()),
                mime_type: MimeType::Specific("text/html".into()),
            });
        }
        if let Some(img) = image {
            if !img.is_empty() && img.len() <= crate::shelf::policy::MAX_IMAGE_BYTES {
                parts.push(MimeSource {
                    source: Source::Bytes(img.to_vec().into_boxed_slice()),
                    mime_type: MimeType::Specific("image/png".into()),
                });
            }
        }
        parts.push(MimeSource {
            source: Source::Bytes(plain.as_bytes().to_vec().into_boxed_slice()),
            mime_type: MimeType::Text,
        });
        if parts.len() > 1 {
            if Options::new().copy_multi(parts).is_ok() {
                return Ok(());
            }
        }
        copy_plain_or_html(plain, html)
    }

    pub fn read_offer() -> Option<Incoming> {
        let plain = read_mime(wl_clipboard_rs::paste::MimeType::Text)?;
        let html = read_mime(wl_clipboard_rs::paste::MimeType::Specific(
            "text/html".into(),
        ));
        let rtf = read_mime(wl_clipboard_rs::paste::MimeType::Specific(
            "text/rtf".into(),
        ))
        .or_else(|| {
            read_mime(wl_clipboard_rs::paste::MimeType::Specific(
                "application/rtf".into(),
            ))
        });
        let image = read_mime_bytes(wl_clipboard_rs::paste::MimeType::Specific(
            "image/png".into(),
        ))
        .or_else(|| {
            read_mime_bytes(wl_clipboard_rs::paste::MimeType::Specific(
                "image/jpeg".into(),
            ))
        })
        .filter(|b| !b.is_empty() && b.len() <= crate::shelf::policy::MAX_IMAGE_BYTES);
        let incoming = Incoming {
            plain,
            html,
            rtf,
            image,
        };
        if incoming.is_empty() {
            None
        } else {
            Some(incoming)
        }
    }

    fn read_mime(mime: wl_clipboard_rs::paste::MimeType) -> Option<String> {
        use std::io::Read;
        use wl_clipboard_rs::paste::{get_contents, ClipboardType, Seat};
        let (mut pipe, _) = get_contents(ClipboardType::Regular, Seat::Unspecified, mime).ok()?;
        let mut buf = String::new();
        pipe.read_to_string(&mut buf).ok()?;
        if buf.is_empty() {
            None
        } else {
            Some(buf)
        }
    }

    fn read_mime_bytes(mime: wl_clipboard_rs::paste::MimeType) -> Option<Vec<u8>> {
        use std::io::Read;
        use wl_clipboard_rs::paste::{get_contents, ClipboardType, Seat};
        let (mut pipe, _) = get_contents(ClipboardType::Regular, Seat::Unspecified, mime).ok()?;
        let mut buf = Vec::new();
        pipe.read_to_end(&mut buf).ok()?;
        if buf.is_empty() {
            None
        } else {
            Some(buf)
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

    pub fn write_offer(
        text: &str,
        html: Option<&str>,
        _rtf: Option<&str>,
        _image: Option<&[u8]>,
    ) -> Result<()> {
        copy_plain_or_html(text, html)
    }

    pub fn read_offer() -> Option<crate::shelf::Incoming> {
        use objc2_app_kit::NSPasteboardTypeRTF;
        let pb = unsafe { NSPasteboard::generalPasteboard() };
        let plain = unsafe { pb.stringForType(NSPasteboardTypeString) }
            .map(|s| s.to_string())
            .unwrap_or_default();
        let html = unsafe { pb.stringForType(NSPasteboardTypeHTML) }.map(|s| s.to_string());
        let rtf = unsafe { pb.stringForType(NSPasteboardTypeRTF) }.map(|s| s.to_string());
        let incoming = crate::shelf::Incoming {
            plain,
            html,
            rtf,
            image: None,
        };
        if incoming.is_empty() {
            None
        } else {
            Some(incoming)
        }
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

    pub fn write_offer(
        text: &str,
        html: Option<&str>,
        _rtf: Option<&str>,
        _image: Option<&[u8]>,
    ) -> Result<()> {
        copy_plain_or_html(text, html)
    }

    pub fn read_offer() -> Option<crate::shelf::Incoming> {
        let text = read_unicode().unwrap_or_default();
        let incoming = crate::shelf::Incoming {
            plain: text,
            html: None,
            rtf: None,
            image: None,
        };
        if incoming.is_empty() {
            None
        } else {
            Some(incoming)
        }
    }

    fn read_unicode() -> Option<String> {
        use ::windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
        use ::windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
        unsafe {
            OpenClipboard(Some(HWND::default())).ok()?;
            let handle = GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok();
            let Some(handle) = handle else {
                let _ = CloseClipboard();
                return None;
            };
            let ptr = GlobalLock(::windows::Win32::Foundation::HGLOBAL(handle.0));
            if ptr.is_null() {
                let _ = CloseClipboard();
                return None;
            }
            let mut len = 0usize;
            let w = ptr as *const u16;
            while !w.add(len).is_null() && *w.add(len) != 0 {
                len += 1;
                if len > 4 * 1024 * 1024 {
                    break;
                }
            }
            let slice = std::slice::from_raw_parts(w, len);
            let text = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(::windows::Win32::Foundation::HGLOBAL(handle.0));
            let _ = CloseClipboard();
            Some(text)
        }
    }
}
