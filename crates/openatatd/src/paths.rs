use std::path::PathBuf;

/// `~/.local/share/openatat` (or `$XDG_DATA_HOME/openatat`).
/// Windows: `%LOCALAPPDATA%\openatat` unless XDG is set.
pub fn data_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("openatat");
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("openatat");
        }
    }
    home_dir().join(".local/share/openatat")
}

/// `~/.config/openatat` (or `$XDG_CONFIG_HOME/openatat`).
/// Windows: `%APPDATA%\openatat` unless XDG is set.
pub fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("openatat");
    }
    #[cfg(windows)]
    {
        if let Some(roam) = std::env::var_os("APPDATA") {
            return PathBuf::from(roam).join("openatat");
        }
    }
    home_dir().join(".config/openatat")
}

/// BYO CLI template / provider pick. Edit on disk or via `openatat-ui`.
pub fn agent_config_path() -> PathBuf {
    config_dir().join("agent.toml")
}

/// `~/.cache/openatat` (or `$XDG_CACHE_HOME/openatat`). Scratch workspaces live here.
/// Windows: `%LOCALAPPDATA%\openatat\cache` unless XDG is set.
pub fn cache_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("openatat");
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("openatat").join("cache");
        }
    }
    home_dir().join(".cache/openatat")
}

/// `$XDG_RUNTIME_DIR/openatat` (socket lives here).
pub fn runtime_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("openatat")
}

pub fn history_path() -> PathBuf {
    data_dir().join("history.jsonl")
}

pub fn trigger_socket_path() -> PathBuf {
    runtime_dir().join("trigger.sock")
}

/// Windows TCP port file for `openatatd trigger` (127.0.0.1).
pub fn trigger_port_path() -> PathBuf {
    runtime_dir().join("trigger.port")
}

/// `$XDG_RUNTIME_DIR/openatat/status.json` — bar chip reads this, no GPU surface.
pub fn status_file_path() -> PathBuf {
    runtime_dir().join("status.json")
}

/// Per-output Orb rest position. Hide is not stored here (this launch only).
pub fn orb_position_path() -> PathBuf {
    data_dir().join("orb-position.json")
}

/// Serialize tests that mutate process-wide XDG_* vars.
#[cfg(test)]
pub(crate) fn xdg_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn home_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile);
    }
    PathBuf::from(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_respects_xdg() {
        let _g = xdg_test_lock();
        let old = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", "/tmp/openatat-test-xdg");
        assert_eq!(data_dir(), PathBuf::from("/tmp/openatat-test-xdg/openatat"));
        match old {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn config_and_cache_respect_xdg() {
        let _g = xdg_test_lock();
        let old_cfg = std::env::var_os("XDG_CONFIG_HOME");
        let old_cache = std::env::var_os("XDG_CACHE_HOME");
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/openatat-test-cfg");
        std::env::set_var("XDG_CACHE_HOME", "/tmp/openatat-test-cache");
        assert_eq!(
            agent_config_path(),
            PathBuf::from("/tmp/openatat-test-cfg/openatat/agent.toml")
        );
        assert_eq!(
            cache_dir(),
            PathBuf::from("/tmp/openatat-test-cache/openatat")
        );
        match old_cfg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match old_cache {
            Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
            None => std::env::remove_var("XDG_CACHE_HOME"),
        }
    }

    #[test]
    fn status_file_lives_next_to_the_trigger_socket() {
        let _g = xdg_test_lock();
        let old = std::env::var_os("XDG_RUNTIME_DIR");
        std::env::set_var("XDG_RUNTIME_DIR", "/tmp/openatat-test-run");
        assert_eq!(
            status_file_path(),
            PathBuf::from("/tmp/openatat-test-run/openatat/status.json")
        );
        assert_eq!(
            trigger_socket_path(),
            PathBuf::from("/tmp/openatat-test-run/openatat/trigger.sock")
        );
        match old {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
    }
}
