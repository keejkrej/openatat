//! XDG paths. Same contract as `openatatd::paths` — do not depend on the daemon
//! crate (that would pull Wayland / layer-shell into this process).

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("openatat");
    }
    home_dir().join(".local/share/openatat")
}

pub fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("openatat");
    }
    home_dir().join(".config/openatat")
}

pub fn agent_config_path() -> PathBuf {
    config_dir().join("agent.toml")
}

pub fn history_path() -> PathBuf {
    data_dir().join("history.jsonl")
}

/// `~/.cache/openatat` (or `$XDG_CACHE_HOME/openatat`). Studio working copies live here.
pub fn cache_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("openatat");
    }
    home_dir().join(".cache/openatat")
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
    fn respects_xdg() {
        let old_data = std::env::var_os("XDG_DATA_HOME");
        let old_cfg = std::env::var_os("XDG_CONFIG_HOME");
        let old_cache = std::env::var_os("XDG_CACHE_HOME");
        std::env::set_var("XDG_DATA_HOME", "/tmp/openatat-ui-data");
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/openatat-ui-cfg");
        std::env::set_var("XDG_CACHE_HOME", "/tmp/openatat-ui-cache");
        assert_eq!(
            history_path(),
            PathBuf::from("/tmp/openatat-ui-data/openatat/history.jsonl")
        );
        assert_eq!(
            agent_config_path(),
            PathBuf::from("/tmp/openatat-ui-cfg/openatat/agent.toml")
        );
        assert_eq!(
            cache_dir(),
            PathBuf::from("/tmp/openatat-ui-cache/openatat")
        );
        match old_data {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
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
