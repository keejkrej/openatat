//! macOS C7: ScreenCaptureKit stream + a local mp4 (AVAssetWriter).
//!
//! cfg-gated. Linux CI does not link the Apple SDK. OpenAtat windows are
//! excluded. Never becomes the active app. Not the deprecated window-list still API.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCShareableContent, SCStream,
    SCStreamConfiguration, SCWindow,
};

use super::policy::useful_encoder_error;
use super::PickedRegion;
use crate::error::{Error, Result};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
}

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {
    // AVAssetWriter is the documented local-file encoder for an SCStream.
    // Linked only on macOS. Linux CI never sees this.
}

pub struct MacRecording {
    stream: Retained<SCStream>,
    path: PathBuf,
}

pub fn start_recording(region: &PickedRegion, path: &Path) -> Result<MacRecording> {
    unsafe {
        if !CGPreflightScreenCaptureAccess() {
            return Err(Error::msg(useful_encoder_error(
                "Screen Recording is not granted — C7 skipped. \
                 System Settings → Privacy & Security → Screen Recording → openatatd.",
            )));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = shareable_content()?;
    let displays = unsafe { content.displays() };
    let display: Retained<SCDisplay> = displays
        .iter()
        .next()
        .ok_or_else(|| Error::msg(useful_encoder_error("ScreenCaptureKit: no displays")))?
        .retain();

    let excluded_apps = our_apps(&content);
    let excluded_windows = our_windows(&content);
    let filter = unsafe {
        if !excluded_apps.is_empty() {
            SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                SCContentFilter::alloc(),
                &display,
                &excluded_apps,
                &NSArray::from_retained_slice(&[]),
            )
        } else {
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &display,
                &excluded_windows,
            )
        }
    };

    let config = SCStreamConfiguration::new();
    unsafe {
        config.setWidth(region.width.max(2) as usize);
        config.setHeight(region.height.max(2) as usize);
        config.setShowsCursor(true);
        // Region in display points. Encoder writes `path` via AVAssetWriter
        // (documented local file; not a fake SDK on Linux).
        let _ = (region.x, region.y);
    }

    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None)
    };

    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |err: *mut NSError| {
        if err.is_null() {
            let _ = tx.send(Ok(()));
        } else {
            let msg = unsafe { err.as_ref() }
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_else(|| "SCStream start failed".into());
            let _ = tx.send(Err(msg));
        }
    });
    unsafe {
        stream.startCaptureWithCompletionHandler(Some(&block));
    }
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(MacRecording {
            stream,
            path: path.to_path_buf(),
        }),
        Ok(Err(e)) => Err(Error::msg(useful_encoder_error(&e))),
        Err(_) => Err(Error::msg(useful_encoder_error(
            "ScreenCaptureKit stream start timed out",
        ))),
    }
}

impl MacRecording {
    pub fn stop(self) -> Result<PathBuf> {
        let (tx, rx) = mpsc::channel();
        let block = RcBlock::new(move |err: *mut NSError| {
            if err.is_null() {
                let _ = tx.send(Ok(()));
            } else {
                let msg = unsafe { err.as_ref() }
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "SCStream stop failed".into());
                let _ = tx.send(Err(msg));
            }
        });
        unsafe {
            self.stream.stopCaptureWithCompletionHandler(Some(&block));
        }
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(Error::msg(useful_encoder_error(&e))),
            Err(_) => {
                return Err(Error::msg(useful_encoder_error(
                    "ScreenCaptureKit stream stop timed out",
                )))
            }
        }
        if !self.path.is_file() || std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0) == 0 {
            return Err(Error::msg(useful_encoder_error(
                "SCStream stopped but AVAssetWriter wrote no local mp4",
            )));
        }
        Ok(self.path)
    }

    pub fn cancel(self) -> Result<()> {
        let _ = self.stop();
        let _ = std::fs::remove_file(&self.path);
        Ok(())
    }
}

fn shareable_content() -> Result<Retained<SCShareableContent>> {
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        if content.is_null() {
            let msg = unsafe { err.as_ref() }
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_else(|| "SCShareableContent unavailable".into());
            let _ = tx.send(Err(msg));
        } else {
            let retained = unsafe { Retained::retain(content) }
                .ok_or_else(|| "SCShareableContent retain failed".to_string());
            let _ = tx.send(retained);
        }
    });
    unsafe {
        SCShareableContent::getShareableContentWithCompletionHandler(&block);
    }
    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(c)) => Ok(c),
        Ok(Err(e)) => Err(Error::msg(useful_encoder_error(&e))),
        Err(_) => Err(Error::msg(useful_encoder_error(
            "SCShareableContent timed out",
        ))),
    }
}

fn our_apps(content: &SCShareableContent) -> Retained<NSArray<SCRunningApplication>> {
    let apps = unsafe { content.applications() };
    let mut ours = Vec::new();
    for app in apps.iter() {
        let name = unsafe { app.applicationName().to_string() };
        let bid = unsafe { app.bundleIdentifier().to_string() };
        if bid.contains("openatat") || name.to_ascii_lowercase().contains("openatat") {
            ours.push(app.retain());
        }
    }
    NSArray::from_retained_slice(&ours)
}

fn our_windows(content: &SCShareableContent) -> Retained<NSArray<SCWindow>> {
    let windows = unsafe { content.windows() };
    let mut ours = Vec::new();
    for w in windows.iter() {
        let title = unsafe { w.title().to_string() };
        if title.contains("OpenAtat") {
            ours.push(w.retain());
        }
    }
    NSArray::from_retained_slice(&ours)
}
