//! First-run honesty: every grant is optional. This page only documents
//! them. The Settings window must not request OS permissions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionGrant {
    pub name: &'static str,
    pub unlocks: &'static str,
    pub how: &'static str,
}

/// Matches SPEC.md §5 Linux / README permissions. Deny one; the rest still works.
pub const LINUX_GRANTS: &[PermissionGrant] = &[
    PermissionGrant {
        name: "grim",
        unlocks: "C1 auto-still of the focused Hyprland output (silent), plus C2/C4 stills after the native picker.",
        how: "grim must be allowed to capture outputs. Do not route auto-attach through the xdg-desktop-portal screenshot picker. C2 uses grim geometry after the native rubber-band; slurp is only a fallback if that picker cannot map.",
    },
    PermissionGrant {
        name: "wf-recorder",
        unlocks: "C7 region recording to ~/.cache/openatat/record/<id>.mp4.",
        how: "Preferred binary: wf-recorder -g <x,y WxH> -f <path> (argv data, never a shell string). gpu-screen-recorder is the fallback. PipeWire ScreenCast is only when neither is installed and must not replace C1. Portal Screenshot is never used.",
    },
    PermissionGrant {
        name: "hyprctl",
        unlocks: "Active output name + `activewindow` address (insert abort).",
        how: "Hyprland instance signature socket. Compared again at insert time.",
    },
    PermissionGrant {
        name: "clipboard",
        unlocks: "Clipboard-first insert (`Tab` copies before AT-SPI) and C14 shelf watch.",
        how: "`wlr-data-control` (via `wl-clipboard-rs`) or the `wl-copy` binary. Shelf history stays on this machine.",
    },
    PermissionGrant {
        name: "AT-SPI",
        unlocks: "Insert into a focused text field; password-role probe every key.",
        how: "`org.a11y.Bus`. Enable accessibility. Some apps need toolkit a11y.",
    },
    PermissionGrant {
        name: "Fcitx5",
        unlocks: "Product `@@` trigger (committed text only; compose never fires).",
        how: "Install `fcitx5-openatat`. Missing `trigger.sock` fails quietly.",
    },
];

/// macOS TCC gates. All optional. Deny one; the rest of OpenAtat still works.
pub const MAC_GRANTS: &[PermissionGrant] = &[
    PermissionGrant {
        name: "Input Monitoring",
        unlocks: "Product `@@` trigger (listen-only event tap → ImeFilter).",
        how: "System Settings → Privacy & Security → Input Monitoring → openatatd. Missing tap: --demo and the unix socket still work.",
    },
    PermissionGrant {
        name: "Accessibility",
        unlocks: "AX insert, @@ swallow, C10 AXSelectedText, secure-field probe.",
        how: "System Settings → Privacy & Security → Accessibility → openatatd. Re-probed every key; never cached.",
    },
    PermissionGrant {
        name: "Screen Recording",
        unlocks: "C1 / C2 / C4 stills and C7 SCStream recording via ScreenCaptureKit (OpenAtat windows excluded).",
        how: "System Settings → Privacy & Security → Screen Recording → openatatd. Denied: skip the tile. Not CGWindowListCreateImage.",
    },
    PermissionGrant {
        name: "Finder Automation",
        unlocks: "C11/C12 cwd tile + selected file tiles (real POSIX paths).",
        how: "System Settings → Privacy & Security → Automation → openatatd → Finder. Denied: do not guess from the title bar. Right-click Service waits.",
    },
];

/// Windows grants. All optional. Deny one; the rest of OpenAtat still works.
pub const WIN_GRANTS: &[PermissionGrant] = &[
    PermissionGrant {
        name: "UI Automation",
        unlocks: "UIA insert, @@ swallow, C10 TextPattern, IsPassword / ES_PASSWORD probe every key.",
        how: "No extra consent dialog on most builds. Re-probed every key; never cached. Password fields are never read.",
    },
    PermissionGrant {
        name: "Graphics Capture",
        unlocks: "C1 / C2 / C4 stills and C7 WGC + Media Foundation recording via CreateForMonitor (overlay / picker / stop-bar HWND excluded).",
        how: "Settings → Privacy & security → Screenshots and apps (graphics capture). Denied: skip the tile. Not GraphicsCapturePicker.",
    },
    PermissionGrant {
        name: "Explorer shell",
        unlocks: "C11/C12 cwd + selected files via IShellWindows → IFolderView.",
        how: "Only when Explorer is frontmost. Title bar is never parsed. Context-menu DLL waits.",
    },
];

pub fn grants_copy() -> String {
    let mut s = String::from(
        "Each grant is optional. Deny one and the rest of OpenAtat still works.\n\
         This window does not request OS permissions; it only names what each unlocks.\n",
    );
    s.push_str("\nLinux\n");
    for g in LINUX_GRANTS {
        s.push_str(&format!("\n{} — {}\n  {}\n", g.name, g.unlocks, g.how));
    }
    s.push_str("\nmacOS\n");
    for g in MAC_GRANTS {
        s.push_str(&format!("\n{} — {}\n  {}\n", g.name, g.unlocks, g.how));
    }
    s.push_str("\nWindows\n");
    for g in WIN_GRANTS {
        s.push_str(&format!("\n{} — {}\n  {}\n", g.name, g.unlocks, g.how));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_linux_grants() {
        let names: Vec<_> = LINUX_GRANTS.iter().map(|g| g.name).collect();
        assert_eq!(
            names,
            ["grim", "wf-recorder", "hyprctl", "clipboard", "AT-SPI", "Fcitx5"]
        );
        let copy = grants_copy();
        assert!(copy.contains("optional"));
        assert!(copy.contains("does not request OS permissions"));
    }

    #[test]
    fn names_the_four_mac_tcc_grants() {
        let names: Vec<_> = MAC_GRANTS.iter().map(|g| g.name).collect();
        assert_eq!(
            names,
            [
                "Input Monitoring",
                "Accessibility",
                "Screen Recording",
                "Finder Automation"
            ]
        );
        let copy = grants_copy();
        assert!(copy.contains("Input Monitoring"));
        assert!(copy.contains("title bar"));
    }

    #[test]
    fn names_the_three_windows_grants() {
        let names: Vec<_> = WIN_GRANTS.iter().map(|g| g.name).collect();
        assert_eq!(
            names,
            ["UI Automation", "Graphics Capture", "Explorer shell"]
        );
        let copy = grants_copy();
        assert!(copy.contains("CreateForMonitor"));
        assert!(copy.contains("GraphicsCapturePicker"));
        assert!(copy.contains("title bar"));
    }
}
