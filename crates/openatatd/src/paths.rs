use std::path::PathBuf;

/// `~/.local/share/openatat` (or `$XDG_DATA_HOME/openatat`).
pub fn data_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("openatat");
    }
    home_dir().join(".local/share/openatat")
}

/// `~/.config/openatat` (or `$XDG_CONFIG_HOME/openatat`).
pub fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("openatat");
    }
    home_dir().join(".config/openatat")
}

/// BYO CLI template / provider pick. Edit on disk or via `openatat-ui`.
pub fn agent_config_path() -> PathBuf {
    config_dir().join("agent.toml")
}

/// `~/.cache/openatat` (or `$XDG_CACHE_HOME/openatat`). Scratch workspaces live here.
pub fn cache_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("openatat");
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

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_respects_xdg() {
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
}
