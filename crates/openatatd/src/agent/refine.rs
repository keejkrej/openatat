//! Preview refine (`R` + one more sentence). History never stores the agent reply.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefineSession {
    pub original_prompt: String,
    pub refine_sentence: Option<String>,
    pub preview: Option<String>,
}

impl RefineSession {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            original_prompt: prompt.into(),
            refine_sentence: None,
            preview: None,
        }
    }

    /// One more sentence. Replaces any previous refine. Attachments stay with the caller.
    pub fn apply_refine(&mut self, sentence: impl Into<String>) {
        self.refine_sentence = Some(sentence.into());
    }

    /// What we send to the CLI. Previous reply stays in memory only.
    pub fn launch_prompt(&self) -> String {
        match self.refine_sentence.as_deref() {
            Some(sentence) if !sentence.is_empty() => match self.preview.as_deref() {
                Some(prev) if !prev.is_empty() => format!(
                    "{}\n\nPrevious answer:\n{}\n\nRefine: {}",
                    self.original_prompt, prev, sentence
                ),
                _ => format!("{}\n\nRefine: {}", self.original_prompt, sentence),
            },
            _ => self.original_prompt.clone(),
        }
    }

    /// History row: the refine sentence (new row) or the original prompt.
    /// Never the agent response.
    pub fn history_prompt(&self) -> &str {
        self.refine_sentence
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.original_prompt)
    }

    pub fn replace_preview(&mut self, result: impl Into<String>) {
        self.preview = Some(result.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refine_replaces_preview_and_history_is_the_sentence() {
        let mut s = RefineSession::new("make this friendlier");
        assert_eq!(s.launch_prompt(), "make this friendlier");
        assert_eq!(s.history_prompt(), "make this friendlier");

        s.replace_preview("Hello there.");
        s.apply_refine("shorter, please");

        assert_eq!(s.history_prompt(), "shorter, please");
        let launch = s.launch_prompt();
        assert!(launch.contains("make this friendlier"), "{launch}");
        assert!(launch.contains("Hello there."), "{launch}");
        assert!(launch.contains("shorter, please"), "{launch}");
        assert!(
            !launch.contains("agent response stored"),
            "history contract is launch-only"
        );

        s.replace_preview("Hi.");
        assert_eq!(s.preview.as_deref(), Some("Hi."));
        assert_eq!(s.history_prompt(), "shorter, please");
    }
}
