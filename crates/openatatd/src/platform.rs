//! Tiny OS helpers used by the daemon entry path.

/// Whether the native overlay can map a surface this process.
pub fn has_overlay_display() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("WAYLAND_DISPLAY").is_some()
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_follows_wayland_display() {
        #[cfg(target_os = "linux")]
        {
            let old = std::env::var_os("WAYLAND_DISPLAY");
            std::env::remove_var("WAYLAND_DISPLAY");
            assert!(!has_overlay_display());
            match old {
                Some(v) => std::env::set_var("WAYLAND_DISPLAY", v),
                None => {}
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = has_overlay_display();
        }
    }
}
