use std::path::PathBuf;

/// `~/.local/share/openatat` (or `$XDG_DATA_HOME/openatat`).
pub fn data_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("openatat");
    }
    home_dir().join(".local/share/openatat")
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
}
