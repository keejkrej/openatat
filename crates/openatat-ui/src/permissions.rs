//! First-run honesty: every Linux gate is optional. This page only documents
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
        unlocks: "C1 auto-still of the focused Hyprland output (silent).",
        how: "grim must be allowed to capture outputs. Do not route auto-attach through the xdg-desktop-portal screenshot picker.",
    },
    PermissionGrant {
        name: "hyprctl",
        unlocks: "Active output name + `activewindow` address (insert abort).",
        how: "Hyprland instance signature socket. Compared again at insert time.",
    },
    PermissionGrant {
        name: "clipboard",
        unlocks: "Clipboard-first insert (`Tab` copies before AT-SPI).",
        how: "`wlr-data-control` (via `wl-clipboard-rs`) or the `wl-copy` binary.",
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

pub fn grants_copy() -> String {
    let mut s = String::from(
        "Each Linux grant is optional. Deny one and the rest of OpenAtat still works.\n\
         This window does not request OS permissions; it only names what each unlocks.\n",
    );
    for g in LINUX_GRANTS {
        s.push_str(&format!("\n{} — {}\n  {}\n", g.name, g.unlocks, g.how));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_five_linux_grants() {
        let names: Vec<_> = LINUX_GRANTS.iter().map(|g| g.name).collect();
        assert_eq!(names, ["grim", "hyprctl", "clipboard", "AT-SPI", "Fcitx5"]);
        let copy = grants_copy();
        assert!(copy.contains("optional"));
        assert!(copy.contains("does not request OS permissions"));
    }
}
