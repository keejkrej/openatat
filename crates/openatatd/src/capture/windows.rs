//! C1 via Windows.Graphics.Capture `CreateForMonitor` (monitor of the
//! foreground window). One frame, then close. `GraphicsCapturePicker` is
//! never used — it shows system UI and breaks the Atat moment.
//!
//! DXGI Desktop Duplication is fallback only. Overlay HWND is excluded
//! (`WDA_EXCLUDEFROMCAPTURE` on the popover).

use std::mem::size_of;
use std::sync::mpsc;
use std::time::Duration;

use windows::core::{Interface, Result as WinResult};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{RECT, HMODULE};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1,
    IDXGIOutput, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use super::{downscale_long_edge, Still, LONG_EDGE};
use crate::error::Result;

pub fn capture_active_output() -> Result<Still> {
    crate::windows_runtime::ensure_com();
    downscale_long_edge(&capture_monitor_png()?, LONG_EDGE)
}

pub fn capture_region(region: &super::picker::PickedRegion) -> Result<Still> {
    crate::windows_runtime::ensure_com();
    let png = capture_monitor_png()?;
    let img = image::load_from_memory(&png)?;
    let mapped = super::policy::map_surface_rect_to_image(
        region.x,
        region.y,
        region.width,
        region.height,
        region.surface_w.max(1),
        region.surface_h.max(1),
        img.width(),
        img.height(),
    )
    .ok_or_else(|| crate::error::Error::msg("C2 crop is empty"))?;
    super::crop_png(&png, mapped.0, mapped.1, mapped.2, mapped.3)
}

fn capture_monitor_png() -> Result<Vec<u8>> {
    match capture_wgc() {
        Ok(png) => Ok(png),
        Err(wgc) => {
            eprintln!("openatatd: WGC C1 failed ({wgc}); trying DXGI Desktop Duplication");
            capture_dxgi().map_err(|dxgi| {
                crate::error::Error::msg(format!(
                    "Windows capture failed. WGC needs privacy consent \
                     (Settings → Privacy & security → Screenshots and apps / graphics capture). \
                     WGC: {wgc}; DXGI: {dxgi}"
                ))
            })
        }
    }
}

fn capture_wgc() -> std::result::Result<Vec<u8>, String> {
    // CreateForMonitor of the monitor that owns the foreground window.
    // Do not use GraphicsCapturePicker — that is interactive system UI.
    let hwnd = unsafe { GetForegroundWindow() };
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return Err("MonitorFromWindow failed".into());
    }

    let d3d = create_d3d_device().map_err(|e| format!("D3D11: {e}"))?;
    let winrt = winrt_device(&d3d).map_err(|e| format!("IDirect3DDevice: {e}"))?;

    let item = capture_item_for_monitor(monitor).map_err(|e| format!("CreateForMonitor: {e}"))?;
    let size = item.Size().map_err(|e| format!("capture item size: {e}"))?;
    if size.Width <= 0 || size.Height <= 0 {
        return Err("GraphicsCaptureItem has empty size".into());
    }

    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &winrt,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        1,
        size,
    )
    .map_err(|e| format!("CreateFreeThreaded: {e}"))?;

    let session = pool
        .CreateCaptureSession(&item)
        .map_err(|e| format!("CreateCaptureSession: {e}"))?;
    let _ = session.SetIsCursorCaptureEnabled(false);

    let (tx, rx) = mpsc::channel();
    let handler = TypedEventHandler::<Direct3D11CaptureFramePool, windows::core::IInspectable>::new(
        move |sender, _| {
            if let Some(pool) = sender.as_ref() {
                if let Ok(frame) = pool.TryGetNextFrame() {
                    let _ = tx.send(frame);
                }
            }
            Ok(())
        },
    );
    let token = pool
        .FrameArrived(&handler)
        .map_err(|e| format!("FrameArrived: {e}"))?;

    session
        .StartCapture()
        .map_err(|e| format!("StartCapture: {e}"))?;

    let frame = rx
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| {
            "WGC frame timed out (privacy consent denied? Settings → Privacy & security)".to_string()
        })?;

    let png = frame_to_png(&d3d, &frame)?;
    let _ = pool.RemoveFrameArrived(token);
    let _ = session.Close();
    let _ = pool.Close();
    let _ = hwnd;
    Ok(png)
}

fn capture_item_for_monitor(monitor: HMONITOR) -> WinResult<GraphicsCaptureItem> {
    let interop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    unsafe { interop.CreateForMonitor(monitor) }
}

fn create_d3d_device() -> WinResult<ID3D11Device> {
    let mut device = None;
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
        )?;
    }
    device.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
}

fn winrt_device(d3d: &ID3D11Device) -> WinResult<IDirect3DDevice> {
    let dxgi: IDXGIDevice = d3d.cast()?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? };
    inspectable.cast()
}

fn frame_to_png(d3d: &ID3D11Device, frame: &Direct3D11CaptureFrame) -> std::result::Result<Vec<u8>, String> {
    let surface = frame
        .Surface()
        .map_err(|e| format!("frame surface: {e}"))?;
    let access: IDirect3DDxgiInterfaceAccess = surface
        .cast()
        .map_err(|e| format!("IDirect3DDxgiInterfaceAccess: {e}"))?;
    let texture: ID3D11Texture2D = unsafe {
        access
            .GetInterface::<ID3D11Texture2D>()
            .map_err(|e| format!("GetInterface texture: {e}"))?
    };
    texture_to_png(d3d, &texture)
}

fn texture_to_png(d3d: &ID3D11Device, texture: &ID3D11Texture2D) -> std::result::Result<Vec<u8>, String> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };
    let w = desc.Width;
    let h = desc.Height;
    if w == 0 || h == 0 {
        return Err("empty capture texture".into());
    }
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    desc.SampleDesc = DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
    };
    desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;

    let mut staging = None;
    unsafe {
        d3d.CreateTexture2D(&desc, None, Some(&mut staging))
            .map_err(|e| format!("CreateTexture2D staging: {e}"))?;
    }
    let staging = staging.ok_or_else(|| "staging texture missing".to_string())?;
    let ctx: ID3D11DeviceContext = unsafe {
        d3d.GetImmediateContext()
            .map_err(|e| format!("GetImmediateContext: {e}"))?
    };
    unsafe { ctx.CopyResource(&staging, texture) };

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| format!("Map staging: {e}"))?;
    }
    let pitch = mapped.RowPitch as usize;
    let src = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, pitch * h as usize) };
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as usize {
        let row = &src[y * pitch..y * pitch + (w as usize * 4)];
        bgra[y * w as usize * 4..(y + 1) * w as usize * 4].copy_from_slice(row);
    }
    unsafe { ctx.Unmap(&staging, 0) };
    bgra_to_png(w, h, &bgra)
}

fn bgra_to_png(w: u32, h: u32, bgra: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let mut rgba = Vec::with_capacity(bgra.len());
    for px in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    let img = image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| "BGRA size mismatch".to_string())?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png)
}

fn capture_dxgi() -> std::result::Result<Vec<u8>, String> {
    let hwnd = unsafe { GetForegroundWindow() };
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let target = monitor_rect(monitor);

    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1: {e}"))? };
    let d3d = create_d3d_device().map_err(|e| format!("D3D11: {e}"))?;

    for i in 0..8u32 {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(i) } {
            Ok(a) => a,
            Err(_) => break,
        };
        for j in 0..8u32 {
            let output: IDXGIOutput = match unsafe { adapter.EnumOutputs(j) } {
                Ok(o) => o,
                Err(_) => break,
            };
            let out1: IDXGIOutput1 = output.cast().map_err(|e| format!("IDXGIOutput1: {e}"))?;
            let desc = unsafe { output.GetDesc().map_err(|e| format!("GetDesc: {e}"))? };
            if !rects_overlap(desc.DesktopCoordinates, target) && i + j > 0 {
                continue;
            }
            let dup: IDXGIOutputDuplication = unsafe {
                out1.DuplicateOutput(&d3d)
                    .map_err(|e| format!("DuplicateOutput: {e}"))?
            };
            // First frame can be timeout; try a few times.
            for _ in 0..8 {
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut resource = None;
                match unsafe { dup.AcquireNextFrame(400, &mut info, &mut resource) } {
                    Ok(()) => {}
                    Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
                    Err(e) => return Err(format!("AcquireNextFrame: {e}")),
                }
                let resource = resource.ok_or_else(|| "DXGI frame resource missing".to_string())?;
                let texture: ID3D11Texture2D = resource
                    .cast()
                    .map_err(|e| format!("DXGI texture: {e}"))?;
                let png = texture_to_png(&d3d, &texture)?;
                let _ = unsafe { dup.ReleaseFrame() };
                return Ok(png);
            }
        }
    }
    Err("DXGI Desktop Duplication found no frame".into())
}

fn monitor_rect(monitor: HMONITOR) -> RECT {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut info);
    }
    info.rcMonitor
}

fn rects_overlap(a: RECT, b: RECT) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_never_uses_picker_or_activates() {
        let src = include_str!("windows.rs");
        assert!(
            !src.contains("GraphicsCapturePicker"),
            "auto-attach must not show GraphicsCapturePicker"
        );
        assert!(!src.contains("SetForegroundWindow"));
    }
}
