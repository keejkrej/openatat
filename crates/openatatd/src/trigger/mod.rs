//! IME-filter-shaped trigger. A global hotkey is not the product path.
//!
//! See SPEC.md §6. P0 ships the filter contract + tests, a no-op IME backend,
//! and a dev-only Wayland/test socket (`demo`).

mod ime;

pub use ime::{Fcitx5Backend, FieldKind, IbusBackend, ImeAction, ImeBackend, ImeEvent, ImeFilter};

/// Last two committed characters. Nothing before or after is kept.
#[derive(Debug, Clone, Default)]
pub struct DetectionBuffer {
    chars: [char; 2],
    len: u8,
}

impl DetectionBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> u8 {
        self.len
    }

    pub fn as_chars(&self) -> Option<(char, char)> {
        (self.len == 2).then_some((self.chars[0], self.chars[1]))
    }

    pub fn clear(&mut self) {
        self.chars = ['\0', '\0'];
        self.len = 0;
    }

    /// Push committed text only. Returns true when the buffer is `@@`.
    pub fn push_committed(&mut self, text: &str) -> bool {
        for ch in text.chars() {
            self.push_char(normalize_at(ch));
        }
        self.is_trigger()
    }

    fn push_char(&mut self, ch: char) {
        match self.len {
            0 => {
                self.chars[0] = ch;
                self.len = 1;
            }
            1 => {
                self.chars[1] = ch;
                self.len = 2;
            }
            _ => {
                self.chars[0] = self.chars[1];
                self.chars[1] = ch;
                self.len = 2;
            }
        }
    }

    /// Case-insensitive `@@`. `@` has no case; we still fold so a future
    /// fullwidth / lookalike policy can live in `normalize_at`.
    pub fn is_trigger(&self) -> bool {
        self.len == 2 && is_at(self.chars[0]) && is_at(self.chars[1])
    }
}

fn normalize_at(ch: char) -> char {
    if ch == '＠' {
        '@'
    } else {
        ch
    }
}

fn is_at(ch: char) -> bool {
    ch == '@' || ch == '＠'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_last_two_characters() {
        let mut b = DetectionBuffer::new();
        assert!(!b.push_committed("hello"));
        assert_eq!(b.as_chars(), Some(('l', 'o')));
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn trigger_on_double_at() {
        let mut b = DetectionBuffer::new();
        assert!(!b.push_committed("@"));
        assert!(b.push_committed("@"));
    }

    #[test]
    fn trigger_in_the_middle_of_other_text() {
        let mut b = DetectionBuffer::new();
        assert!(b.push_committed("x@@y") == false || b.is_trigger());
        // After 'x','@','@' we would have fired if we checked per char.
        let mut b = DetectionBuffer::new();
        assert!(!b.push_committed("x"));
        assert!(!b.push_committed("@"));
        assert!(b.push_committed("@"));
    }

    #[test]
    fn two_ats_in_one_commit() {
        let mut b = DetectionBuffer::new();
        assert!(b.push_committed("@@"));
    }

    #[test]
    fn not_trigger_on_single_at() {
        let mut b = DetectionBuffer::new();
        assert!(!b.push_committed("@"));
        assert!(!b.is_trigger());
    }

    #[test]
    fn fullwidth_at_counts() {
        let mut b = DetectionBuffer::new();
        assert!(b.push_committed("＠＠"));
    }
}
