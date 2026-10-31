//! Literal matching on newly emitted printable PTY text, not terminal screen state.
use crate::OutputItem;
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{wire::AgentEvent, *};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMatchOutcome {
    Matched,
    EndedBeforeMatch,
    HistoryUnavailable,
    ResourceLimit,
    Timeout,
    Cancelled,
}

/// Replays from sequence zero to establish escape-parser state. Text at or before
/// the explicit boundary cannot contribute to a match. Memory is independent of output size.
pub struct OutputLiteral {
    after: Counter,
    cursor: Counter,
    parser: vte::Parser,
    text: MatchText,
    bytes: usize,
}
struct MatchText {
    needle: Vec<char>,
    prefix: Vec<usize>,
    position: usize,
    enabled: bool,
    matched: bool,
}
impl MatchText {
    fn accept(&mut self, character: char) {
        if !self.enabled || self.matched {
            return;
        }
        while self.position > 0 && self.needle[self.position] != character {
            self.position = self.prefix[self.position - 1];
        }
        if self.needle[self.position] == character {
            self.position += 1;
        }
        if self.position == self.needle.len() {
            self.matched = true;
        }
    }
}
impl vte::Perform for MatchText {
    fn print(&mut self, character: char) {
        self.accept(character);
    }
    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | b'\r' | b'\t' => self.accept(char::from(byte)),
            8 => self.position = 0,
            _ => {}
        }
    }
}
impl OutputLiteral {
    pub fn new(after: Counter, literal: &str) -> Result<Self, RemoteError> {
        if literal.is_empty()
            || literal.len() > 512
            || literal
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "literal must contain 1..=512 bytes of printable text or whitespace",
            ));
        }
        let needle: Vec<_> = literal.chars().collect();
        let mut prefix = vec![0; needle.len()];
        for index in 1..needle.len() {
            let mut previous = prefix[index - 1];
            while previous > 0 && needle[index] != needle[previous] {
                previous = prefix[previous - 1];
            }
            if needle[index] == needle[previous] {
                previous += 1;
            }
            prefix[index] = previous;
        }
        Ok(Self {
            after,
            cursor: Counter(0),
            parser: vte::Parser::new(),
            text: MatchText {
                needle,
                prefix,
                position: 0,
                enabled: false,
                matched: false,
            },
            bytes: 0,
        })
    }
    pub fn cursor(&self) -> Counter {
        self.cursor
    }
    /// Caller must use OutputCursor to verify ordering, duplicates, and replay continuity.
    pub fn event(&mut self, item: &OutputItem) -> Result<Option<OutputMatchOutcome>, RemoteError> {
        if item.seq <= self.cursor {
            return Ok(None);
        }
        if item.seq.0 != self.cursor.0.saturating_add(1) {
            return Ok(Some(OutputMatchOutcome::HistoryUnavailable));
        }
        self.cursor = item.seq;
        match &item.event {
            AgentEvent::PtyOutput { data } => {
                let bytes = STANDARD.decode(data).map_err(|_| {
                    RemoteError::rejected(ErrorCode::InvalidFrame, "invalid PTY output encoding")
                })?;
                self.bytes = self.bytes.saturating_add(bytes.len());
                if self.bytes > 8 * 1024 * 1024 {
                    return Ok(Some(OutputMatchOutcome::ResourceLimit));
                }
                self.text.enabled = item.seq > self.after;
                if !self.text.enabled {
                    self.text.position = 0;
                }
                self.parser.advance(&mut self.text, &bytes);
                if self.text.matched {
                    return Ok(Some(OutputMatchOutcome::Matched));
                }
            }
            AgentEvent::Gap { .. } | AgentEvent::Opaque { .. } => {
                return Ok(Some(OutputMatchOutcome::HistoryUnavailable));
            }
            AgentEvent::Exited { .. } => return Ok(Some(OutputMatchOutcome::EndedBeforeMatch)),
            _ => {}
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(seq: u64, bytes: &[u8]) -> OutputItem {
        OutputItem {
            seq: Counter(seq),
            at: None,
            event: AgentEvent::PtyOutput {
                data: STANDARD.encode(bytes),
            },
        }
    }
    #[test]
    fn every_utf8_and_escape_chunk_boundary_matches_only_new_printable_text() {
        let bytes = "\x1b]52;clipboard;ready🦀\x07re\x1b[31mady🦀\x1b[0m".as_bytes();
        for split in 0..=bytes.len() {
            let mut matcher = OutputLiteral::new(Counter(1), "ready🦀").unwrap();
            assert_eq!(
                matcher.event(&item(1, b"ready\xf0\x9f\xa6\x80")).unwrap(),
                None
            );
            let a = matcher.event(&item(2, &bytes[..split])).unwrap();
            let b = matcher.event(&item(3, &bytes[split..])).unwrap();
            assert!(
                a == Some(OutputMatchOutcome::Matched) || b == Some(OutputMatchOutcome::Matched),
                "split {split}"
            );
        }
    }
    #[test]
    fn old_prefix_and_control_payloads_never_satisfy_a_fresh_wait() {
        let mut matcher = OutputLiteral::new(Counter(1), "ready").unwrap();
        assert_eq!(matcher.event(&item(1, b"rea")).unwrap(), None);
        assert_eq!(
            matcher
                .event(&item(2, b"dy\x1b]2;ready\x07\x1bPready\x1b\\"))
                .unwrap(),
            None
        );
        assert_eq!(matcher.event(&item(3, b"re")).unwrap(), None);
        assert_eq!(matcher.event(&item(3, b"re")).unwrap(), None);
        assert_eq!(
            matcher.event(&item(4, b"ady")).unwrap(),
            Some(OutputMatchOutcome::Matched)
        );
        let mut inside = OutputLiteral::new(Counter(1), "ready").unwrap();
        assert_eq!(inside.event(&item(1, b"\x1b]2;")).unwrap(), None);
        assert_eq!(inside.event(&item(2, b"ready\x07")).unwrap(), None);
        assert_eq!(
            inside.event(&item(4, b"ready")).unwrap(),
            Some(OutputMatchOutcome::HistoryUnavailable)
        );
    }
    #[test]
    fn output_and_literal_limits_are_explicit() {
        assert!(OutputLiteral::new(Counter(0), "").is_err());
        assert!(OutputLiteral::new(Counter(0), &"a".repeat(513)).is_err());
        let mut matcher = OutputLiteral::new(Counter(0), "ready").unwrap();
        for seq in 1..=8 {
            assert_eq!(
                matcher.event(&item(seq, &vec![b'x'; 1024 * 1024])).unwrap(),
                None
            );
        }
        assert_eq!(
            matcher.event(&item(9, b"ready")).unwrap(),
            Some(OutputMatchOutcome::ResourceLimit)
        );
    }
}
