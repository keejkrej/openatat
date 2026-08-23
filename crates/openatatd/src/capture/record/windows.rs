//! Windows C7: WGC `CreateForMonitor` + Media Foundation local mp4.
//!
//! cfg-gated. Linux CI does not link the Windows SDK. Overlay / Orb /
//! picker / stop-bar HWNDs are excluded (`WDA_EXCLUDEFROMCAPTURE`).
//! Never the system capture-picker UI. Never activates the host.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Media::MediaFoundation::{
    IMFSinkWriter, MFCreateSinkWriterFromURL, MFShutdown, MFStartup, MFSTARTUP_FULL,
};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use super::policy::useful_encoder_error;
use super::PickedRegion;
use crate::error::{Error, Result};

pub struct WinRecording {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    writer: IMFSinkWriter,
    path: PathBuf,
    stop_tx: mpsc::Sender<()>,
    join: Option<std::thread::JoinHandle<()>>,
}

pub fn start_recording(region: &PickedRegion, path: &Path) -> Result<WinRecording> {
    let _ = region;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::windows_runtime::ensure_com();
    unsafe {
        MFStartup(MF_VERSION_SAFE, MFSTARTUP_FULL).map_err(|e| {
            Error::msg(useful_encoder_error(&format!(
                "Media Foundation startup failed ({e})"
            )))
        })?;
    }

    let item = capture_item_for_fg_monitor()?;
    exclude_openatat_hwnds();

    let d3d = d3d_device()?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &d3d,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        item.Size().map_err(|e| Error::msg(useful_encoder_error(&e.to_string())))?,
    )
    .map_err(|e| Error::msg(useful_encoder_error(&e.to_string())))?;
    let session = pool
        .CreateCaptureSession(&item)
        .map_err(|e| Error::msg(useful_encoder_error(&e.to_string())))?;
    session
        .StartCapture()
        .map_err(|e| Error::msg(useful_encoder_error(&e.to_string())))?;

    let url = windows::core::HSTRING::from(path.to_string_lossy().as_ref());
    let writer = unsafe {
        MFCreateSinkWriterFromURL(&url, None, None).map_err(|e| {
            Error::msg(useful_encoder_error(&format!(
                "Media Foundation SinkWriter ({e})"
            )))
        })?
    };

    let (stop_tx, stop_rx) = mpsc::channel();
    let join = std::thread::spawn(move || {
        let _ = stop_rx.recv_timeout(Duration::from_secs(60 * 60));
    });

    Ok(WinRecording {
        session,
        pool,
        writer,
        path: path.to_path_buf(),
        stop_tx,
        join: Some(join),
    })
}

impl WinRecording {
    pub fn stop(mut self) -> Result<PathBuf> {
        let _ = self.stop_tx.send(());
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
        unsafe {
            let _ = self.writer.Finalize();
            let _ = MFShutdown();
        }
        if !self.path.is_file() || std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0) == 0 {
            return Err(Error::msg(useful_encoder_error(
                "WGC / Media Foundation wrote no local mp4",
            )));
        }
        Ok(self.path.clone())
    }

    pub fn cancel(self) -> Result<()> {
        let path = self.path.clone();
        let _ = self.stop();
        let _ = std::fs::remove_file(path);
        Ok(())
    }
}

const MF_VERSION_SAFE: u32 = (2u32 << 16) | 112;

fn capture_item_for_fg_monitor() -> Result<GraphicsCaptureItem> {
    let hwnd = unsafe { GetForegroundWindow() };
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return Err(Error::msg(useful_encoder_error(
            "WGC: no monitor for the foreground window",
        )));
    }
    let interop: IGraphicsCaptureItemInterop = windows::core::factory::<GraphicsCaptureItem, _>()
        .map_err(|e| Error::msg(useful_encoder_error(&e.to_string())))?;
    unsafe {
        interop
            .CreateForMonitor(monitor)
            .map_err(|e| Error::msg(useful_encoder_error(&e.to_string())))
    }
}

fn d3d_device() -> Result<IDirect3DDevice> {
    let mut device: Option<ID3D11Device> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
        .map_err(|e| Error::msg(useful_encoder_error(&e.to_string())))?;
    }
    let device = device.ok_or_else(|| Error::msg(useful_encoder_error("D3D11 device missing")))?;
    let dxgi: IDXGIDevice = device
        .cast()
        .map_err(|e| Error::msg(useful_encoder_error(&e.to_string())))?;
    unsafe {
        CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
            .map_err(|e| Error::msg(useful_encoder_error(&e.to_string())))
    }
}

fn exclude_openatat_hwnds() {
    for hwnd in [
        crate::overlay::windows_overlay_hwnd(),
        crate::orb::windows_orb_hwnd(),
        crate::capture::picker::windows_picker_hwnd(),
        super::windows_bar_hwnd(),
    ]
    .into_iter()
    .flatten()
    {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowDisplayAffinity(
                HWND(hwnd as *mut _),
                windows::Win32::UI::WindowsAndMessaging::WDA_EXCLUDEFROMCAPTURE,
            );
        }
    }
}

#[allow(dead_code)]
fn _monitor_rect(monitor: HMONITOR) -> RECT {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut info);
    }
    info.rcMonitor
}

use windows::Win32::Foundation::HMODULE;
