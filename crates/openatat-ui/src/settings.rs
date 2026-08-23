//! Load / save `~/.config/openatat/agent.toml` without destroying unknown keys.

use std::fs;
use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Value};

use crate::providers::{default_argv, normalize_provider};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEdit {
    /// `auto` / `claude` / … / `custom` (and any unknown string we keep).
    pub provider: String,
    /// Empty means “no argv override” — daemon uses the provider default.
    pub argv: Vec<String>,
}

impl Default for AgentEdit {
    fn default() -> Self {
        Self {
            provider: "auto".into(),
            argv: Vec::new(),
        }
    }
}

impl AgentEdit {
    pub fn load_default_path() -> Self {
        load_from(&crate::paths::agent_config_path()).unwrap_or_default()
    }

    pub fn argv_as_lines(&self) -> String {
        self.argv.join("\n")
    }

    pub fn set_argv_from_lines(&mut self, text: &str) {
        self.argv = parse_argv_lines(text);
    }
}

/// One argv element per line. Blank lines are dropped. Never a shell string.
pub fn parse_argv_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

pub fn load_from(path: &Path) -> Option<AgentEdit> {
    let raw = fs::read_to_string(path).ok()?;
    parse_agent_toml(&raw)
}

pub fn parse_agent_toml(raw: &str) -> Option<AgentEdit> {
    let doc: DocumentMut = raw.parse().ok()?;
    let provider = doc
        .get("provider")
        .and_then(|i| i.as_str())
        .map(normalize_provider)
        .unwrap_or_else(|| "auto".into());

    let mut argv = array_of_strings(doc.get("argv"));
    if argv.is_empty() {
        if let Some(custom) = doc.get("custom").and_then(|i| i.as_table()) {
            argv = array_of_strings(custom.get("argv"));
            if argv.is_empty() {
                if let Some(bin) = custom.get("bin").and_then(|i| i.as_str()) {
                    if !bin.is_empty() {
                        argv = vec![bin.to_string()];
                    }
                }
            }
        }
    }
    Some(AgentEdit { provider, argv })
}

fn array_of_strings(item: Option<&Item>) -> Vec<String> {
    let Some(item) = item else {
        return Vec::new();
    };
    match item.as_array() {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        None => item
            .as_str()
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
    }
}

pub fn save_to(path: &Path, edit: &AgentEdit) -> Result<(), String> {
    let mut doc = if path.exists() {
        let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
        if raw.trim().is_empty() {
            DocumentMut::new()
        } else {
            raw.parse::<DocumentMut>()
                .map_err(|e| format!("invalid agent.toml: {e}"))?
        }
    } else {
        DocumentMut::new()
    };

    let provider = if edit.provider.trim().is_empty() {
        "auto"
    } else {
        edit.provider.trim()
    };
    doc["provider"] = toml_edit::value(provider);

    if edit.argv.is_empty() {
        if let Some(table) = doc.as_table_mut().get_mut("custom") {
            if let Some(t) = table.as_table_mut() {
                t.remove("argv");
            }
        }
        doc.as_table_mut().remove("argv");
    } else {
        let mut arr = Array::new();
        for a in &edit.argv {
            arr.push(a.as_str());
        }
        doc["argv"] = Item::Value(Value::Array(arr));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, doc.to_string()).map_err(|e| e.to_string())?;
    Ok(())
}

fn is_stock_argv(current: &[String]) -> bool {
    if current.is_empty() {
        return true;
    }
    crate::providers::REGISTRY.iter().any(|s| {
        s.argv
            .iter()
            .map(|a| *a)
            .eq(current.iter().map(|a| a.as_str()))
    })
}

pub fn apply_provider_pick(edit: &mut AgentEdit, provider: &str) {
    let provider = normalize_provider(provider);
    let replace = is_stock_argv(&edit.argv);
    edit.provider = provider.clone();
    if replace {
        edit.argv = default_argv(&provider);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "openatat-ui-cfg-{}-{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("agent.toml")
    }

    #[test]
    fn parse_argv_is_list_not_shell() {
        let v = parse_argv_lines("claude\n--print\n{prompt}\n");
        assert_eq!(v, ["claude", "--print", "{prompt}"]);
        // A single line is one argv element — we do not parse shell quoting.
        assert_eq!(
            parse_argv_lines("claude --print '{prompt}'"),
            ["claude --print '{prompt}'"]
        );
    }

    #[test]
    fn save_preserves_unknown_keys_and_comments() {
        let path = tmp("preserve");
        std::fs::write(
            &path,
            r#"# keep me
provider = "auto"
extra_flag = true

[custom]
bin = "old"
note = "leave this"
"#,
        )
        .unwrap();
        let edit = AgentEdit {
            provider: "claude".into(),
            argv: vec!["claude".into(), "--print".into(), "{prompt}".into()],
        };
        save_to(&path, &edit).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# keep me"), "{raw}");
        assert!(raw.contains("extra_flag = true"), "{raw}");
        assert!(raw.contains("note = \"leave this\""), "{raw}");
        assert!(raw.contains("provider = \"claude\""), "{raw}");
        assert!(raw.contains("{prompt}"), "{raw}");
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.provider, "claude");
        assert_eq!(loaded.argv, ["claude", "--print", "{prompt}"]);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn empty_argv_removes_override() {
        let path = tmp("empty");
        std::fs::write(&path, "provider = \"claude\"\nargv = [\"x\"]\n").unwrap();
        save_to(
            &path,
            &AgentEdit {
                provider: "auto".into(),
                argv: vec![],
            },
        )
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("argv"), "{raw}");
        assert!(raw.contains("provider = \"auto\""));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn custom_bin_becomes_argv() {
        let edit = parse_agent_toml(
            r#"
provider = "custom"
[custom]
bin = "my-agent"
"#,
        )
        .unwrap();
        assert_eq!(edit.provider, "custom");
        assert_eq!(edit.argv, ["my-agent"]);
    }

    #[test]
    fn pick_replaces_stock_argv() {
        let mut edit = AgentEdit {
            provider: "auto".into(),
            argv: vec![],
        };
        apply_provider_pick(&mut edit, "grok");
        assert_eq!(edit.provider, "grok");
        assert!(edit.argv.iter().any(|a| a == "{prompt_file}"));
        apply_provider_pick(&mut edit, "pi");
        assert_eq!(edit.argv, ["pi", "--print", "{prompt}"]);
        edit.argv = vec!["my".into(), "custom".into(), "{prompt}".into()];
        apply_provider_pick(&mut edit, "claude");
        assert_eq!(edit.argv, ["my", "custom", "{prompt}"]);
    }
}
