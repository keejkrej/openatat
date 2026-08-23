//! User override: `~/.config/openatat/agent.toml`.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use super::providers::ProviderKind;
use super::template::CommandTemplate;
use crate::error::Result;
use crate::paths::agent_config_path;

/// On-disk settings. Edit this file or use `openatat-ui` Settings.
///
/// ```toml
/// # auto = first provider binary found on PATH
/// provider = "auto"
///
/// # Optional full argv override (not a shell line).
/// # {prompt}      → prompt as one argv element
/// # {prompt_file} → path of a temp file that already holds the prompt
/// # neither       → prompt on stdin
/// # argv = ["claude", "--print", "--permission-mode", "plan", "{prompt}"]
///
/// # [custom]
/// # bin = "my-agent"
/// # argv = ["my-agent", "--ask", "{prompt}"]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AgentConfig {
    pub provider: Option<String>,
    pub argv: Option<Vec<String>>,
    pub custom: Option<CustomCli>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CustomCli {
    pub bin: Option<String>,
    pub argv: Option<Vec<String>>,
}

impl AgentConfig {
    pub fn load() -> Self {
        load_from(&agent_config_path()).unwrap_or_default()
    }

    pub fn requested_kind(&self) -> Option<ProviderKind> {
        self.provider
            .as_deref()
            .and_then(|p| match p.trim() {
                "" | "auto" => None,
                other => ProviderKind::parse(other),
            })
    }

    pub fn argv_override(&self) -> Result<Option<CommandTemplate>> {
        if let Some(argv) = &self.argv {
            return Ok(Some(CommandTemplate::new(argv.clone())?));
        }
        if self.requested_kind() == Some(ProviderKind::Custom) {
            if let Some(custom) = &self.custom {
                if let Some(argv) = &custom.argv {
                    return Ok(Some(CommandTemplate::new(argv.clone())?));
                }
                if let Some(bin) = &custom.bin {
                    return Ok(Some(CommandTemplate::from_bin(bin)));
                }
            }
        }
        Ok(None)
    }
}

pub fn load_from(path: &Path) -> Option<AgentConfig> {
    let raw = fs::read_to_string(path).ok()?;
    match toml::from_str::<AgentConfig>(&raw) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            eprintln!(
                "openatatd: ignoring invalid {} ({e})",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn parses_provider_and_argv() {
        let dir = std::env::temp_dir().join(format!("openatat-cfg-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent.toml");
        std::fs::write(
            &path,
            r#"
provider = "claude"
argv = ["claude", "--print", "{prompt}"]
"#,
        )
        .unwrap();
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.requested_kind(), Some(ProviderKind::Claude));
        let t = cfg.argv_override().unwrap().unwrap();
        assert_eq!(t.argv, ["claude", "--print", "{prompt}"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn auto_means_no_forced_kind() {
        let cfg = AgentConfig {
            provider: Some("auto".into()),
            ..Default::default()
        };
        assert_eq!(cfg.requested_kind(), None);
    }
}
