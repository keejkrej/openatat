//! Explorer cwd + selected PIDLs via `IShellWindows` → `IFolderView`.
//! The title bar is never consulted. Context-menu DLL waits.

use std::ffi::c_void;
use std::path::PathBuf;

use windows::core::{IUnknown, Interface};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IServiceProvider, CLSCTX_ALL,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IFolderView, IFolderView2, IPersistFolder2, IShellBrowser, IShellItem, IShellWindows,
    IWebBrowserApp, ShellWindows, SHGetPathFromIDListW, SID_STopLevelBrowser, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use openatat_ipc::FocusSnapshot;

use super::policy::{
    decide_finder_tiles, frontmost_is_explorer, paths_from_title_bar, AutomationError, FinderPaths,
    FinderProbe,
};
use crate::a11y;

pub fn probe(focus: &FocusSnapshot) -> FinderProbe {
    let class = a11y::foreground_class();
    let exe = a11y::foreground_exe();
    let is_explorer = frontmost_is_explorer(
        Some(class.as_str()).filter(|s| !s.is_empty()),
        exe.as_deref(),
    ) || frontmost_is_explorer(focus.app_id.as_deref(), None);
    if !is_explorer {
        return FinderProbe::NotFinder;
    }
    // Title is passed only so the policy can prove it is unused.
    let owned_title = a11y::foreground_title();
    let title = focus.title.as_deref().or(owned_title.as_deref());
    let _ = paths_from_title_bar(title.unwrap_or(""));
    let automation = read_shell_windows();
    decide_finder_tiles(true, automation, title)
}

fn read_shell_windows() -> std::result::Result<FinderPaths, AutomationError> {
    crate::windows_runtime::ensure_com();
    let want = a11y::explorer_root_hwnd().unwrap_or(0);
    let fg = unsafe { GetForegroundWindow() };
    let want = if want == 0 { fg.0 as isize } else { want };

    let windows: IShellWindows = unsafe {
        CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).map_err(|_| AutomationError::Failed)?
    };
    let count = unsafe { windows.Count().map_err(|_| AutomationError::Failed)? };
    for i in 0..count {
        let item = unsafe {
            windows
                .Item(&VARIANT::from(i))
                .map_err(|_| AutomationError::Failed)?
        };
        let browser = match item.cast::<IWebBrowserApp>() {
            Ok(b) => b,
            Err(_) => continue,
        };
        let handle = unsafe { browser.HWND().map_err(|_| AutomationError::Failed)? };
        let hwnd = HWND(handle.0 as *mut c_void);
        if hwnd.0 as isize != want && !is_child_of(hwnd, HWND(want as *mut c_void)) {
            continue;
        }
        let unk: IUnknown = item.cast().map_err(|_| AutomationError::Failed)?;
        return folder_from_browser(&unk);
    }
    Err(AutomationError::Failed)
}

fn is_child_of(child: HWND, root: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetParent;
    let mut cur = child;
    unsafe {
        for _ in 0..8 {
            if cur.0 == root.0 {
                return true;
            }
            if cur.0.is_null() {
                return false;
            }
            cur = GetParent(cur).unwrap_or_default();
        }
    }
    false
}

fn folder_from_browser(item: &IUnknown) -> std::result::Result<FinderPaths, AutomationError> {
    let sp: IServiceProvider = item.cast().map_err(|_| AutomationError::Failed)?;
    let browser: IShellBrowser = unsafe {
        sp.QueryService(&SID_STopLevelBrowser)
            .map_err(|_| AutomationError::Failed)?
    };
    let view = unsafe { browser.QueryActiveShellView().map_err(|_| AutomationError::Failed)? };
    let folder: IFolderView = view.cast().map_err(|_| AutomationError::Failed)?;

    let cwd = current_folder_path(&folder);
    let files = selected_paths(&folder);
    Ok(FinderPaths { cwd, files })
}

fn current_folder_path(folder: &IFolderView) -> Option<PathBuf> {
    unsafe {
        let persist: IPersistFolder2 = folder.GetFolder().ok()?;
        let pidl = persist.GetCurFolder().ok()?;
        if pidl.is_null() {
            return None;
        }
        let path = pidl_to_path(pidl);
        CoTaskMemFree(Some(pidl as *const c_void));
        path
    }
}

fn selected_paths(folder: &IFolderView) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(fv2) = folder.cast::<IFolderView2>() else {
        return out;
    };
    let Ok(items) = (unsafe { fv2.GetSelection(false) }) else {
        return out;
    };
    let n = unsafe { items.GetCount().unwrap_or(0) };
    for i in 0..n {
        if let Ok(item) = unsafe { items.GetItemAt(i) } {
            if let Some(p) = shell_item_path(&item) {
                out.push(p);
            }
        }
    }
    out
}

fn shell_item_path(item: &IShellItem) -> Option<PathBuf> {
    let s = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH).ok()? };
    let t = unsafe { s.to_string().ok()? };
    if t.is_empty() {
        None
    } else {
        Some(PathBuf::from(t))
    }
}

fn pidl_to_path(pidl: *mut ITEMIDLIST) -> Option<PathBuf> {
    if pidl.is_null() {
        return None;
    }
    let mut buf = [0u16; 260];
    let ok = unsafe { SHGetPathFromIDListW(pidl, &mut buf) };
    if !ok.as_bool() {
        return None;
    }
    let nul = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let s = String::from_utf16_lossy(&buf[..nul]);
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}
