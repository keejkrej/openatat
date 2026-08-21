//! Argv command templates. The prompt is data, never a shell string.

use std::path::Path;

use crate::error::{Error, Result};

/// Placeholder replaced with the prompt as a single argv element.
pub const PROMPT: &str = "{prompt}";
/// Placeholder replaced with a temp file path that already contains the prompt.
pub const PROMPT_FILE: &str = "{prompt_file}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptPass {
    /// Write the prompt bytes to the child's stdin.
    Stdin,
    /// `{prompt}` appears in argv (and is substituted as one element).
    Argv,
    /// `{prompt_file}` appears in argv; we write the prompt to that file.
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTemplate {
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub pass: PromptPass,
    pub prompt_file: Option<std::path::PathBuf>,
}

impl CommandTemplate {
    pub fn new(argv: impl IntoIterator<Item = impl Into<String>>) -> Result<Self> {
        let argv: Vec<String> = argv.into_iter().map(Into::into).collect();
        if argv.is_empty() {
            return Err(Error::msg("command template argv must not be empty"));
        }
        Ok(Self { argv })
    }

    pub fn from_bin(bin: &str) -> Self {
        Self {
            argv: vec![bin.to_string()],
        }
    }

    pub fn display(&self) -> String {
        self.argv.join(" ")
    }

    pub fn pass(&self) -> PromptPass {
        let has_file = self.argv.iter().any(|a| a.contains(PROMPT_FILE));
        let has_prompt = self.argv.iter().any(|a| a.contains(PROMPT));
        if has_file {
            PromptPass::File
        } else if has_prompt {
            PromptPass::Argv
        } else {
            PromptPass::Stdin
        }
    }

    /// Substitute placeholders. `{prompt}` becomes `prompt` as data inside argv.
    /// `{prompt_file}` becomes `prompt_file`'s path. No shell is involved.
    pub fn render(&self, prompt: &str, prompt_file: Option<&Path>) -> Result<RenderedCommand> {
        let pass = self.pass();
        if pass == PromptPass::File && prompt_file.is_none() {
            return Err(Error::msg(
                "template uses {prompt_file} but no prompt file path was provided",
            ));
        }
        let file_s = prompt_file.map(|p| p.to_string_lossy().into_owned());
        let mut out: Vec<String> = Vec::with_capacity(self.argv.len());
        for part in &self.argv {
            let mut s = part.clone();
            if let Some(path) = &file_s {
                s = s.replace(PROMPT_FILE, path);
            }
            s = s.replace(PROMPT, prompt);
            out.push(s);
        }
        let program = out.remove(0);
        Ok(RenderedCommand {
            program,
            args: out,
            pass,
            prompt_file: prompt_file.map(|p| p.to_path_buf()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn renders_prompt_as_single_argv_element() {
        let t = CommandTemplate::new(["tool", "--ask", "{prompt}"]).unwrap();
        let nasty = "hello \"quoted\"\nand a newline; $(whoami)";
        let r = t.render(nasty, None).unwrap();
        assert_eq!(r.program, "tool");
        assert_eq!(r.args, vec!["--ask".to_string(), nasty.to_string()]);
        assert_eq!(r.pass, PromptPass::Argv);
        // Would be multiple words / a shell injection if we had joined a command line.
        assert!(r.args[1].contains('\n'));
        assert!(r.args[1].contains('"'));
        assert!(r.args[1].contains("$(whoami)"));
    }

    #[test]
    fn prompt_file_placeholder_becomes_path() {
        let t = CommandTemplate::new(["grok", "--prompt-file", "{prompt_file}"]).unwrap();
        let path = PathBuf::from("/tmp/openatat-prompt.txt");
        let r = t.render("ignored-for-argv", Some(&path)).unwrap();
        assert_eq!(r.pass, PromptPass::File);
        assert_eq!(r.args, vec!["--prompt-file", "/tmp/openatat-prompt.txt"]);
        assert!(!r.args.iter().any(|a| a.contains("ignored-for-argv")));
    }

    #[test]
    fn no_placeholder_means_stdin() {
        let t = CommandTemplate::new(["codex", "exec", "-"]).unwrap();
        let r = t.render("the prompt", None).unwrap();
        assert_eq!(r.pass, PromptPass::Stdin);
        assert_eq!(r.args, vec!["exec", "-"]);
    }

    #[test]
    fn display_keeps_placeholders() {
        let t = CommandTemplate::new(["claude", "--print", "{prompt}"]).unwrap();
        assert_eq!(t.display(), "claude --print {prompt}");
    }
}
