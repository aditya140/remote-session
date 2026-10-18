use anyhow::{Result, ensure};
use std::collections::VecDeque;
use zeroize::{Zeroize, Zeroizing};

/// Explicit sample integration point for verified host rules. Debug/serialization omit secrets.
/// This literal matcher does not claim compatibility with unavailable corporate redaction.
#[derive(Clone, Default)]
pub struct RedactionRules {
    patterns: Vec<Zeroizing<Vec<u8>>>,
}

impl RedactionRules {
    pub fn literals(patterns: Vec<Vec<u8>>) -> Result<Self> {
        let patterns: Vec<_> = patterns.into_iter().map(Zeroizing::new).collect();
        ensure!(patterns.len() <= 128, "too many redaction rules");
        ensure!(
            patterns.iter().all(|p| !p.is_empty() && p.len() <= 256),
            "invalid redaction rule size"
        );
        ensure!(
            patterns.iter().map(|p| p.len()).sum::<usize>() <= 8192,
            "redaction rules exceed budget"
        );
        Ok(Self { patterns })
    }
}

struct Byte {
    source: u64,
    value: u8,
}
impl Drop for Byte {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

pub(super) struct Redacted {
    pub source: u64,
    pub consumed: usize,
    pub output: Vec<u8>,
}

pub(super) struct Redactor {
    rules: RedactionRules,
    pending: VecDeque<Byte>,
}
impl Redactor {
    pub fn new(rules: RedactionRules) -> Self {
        Self {
            rules,
            pending: VecDeque::new(),
        }
    }

    pub fn push(&mut self, source: u64, bytes: &[u8]) -> Vec<Redacted> {
        let mut output = Vec::new();
        for &value in bytes {
            self.pending.push_back(Byte { source, value });
            self.drain(false, &mut output);
        }
        output
    }

    // EOF masks incomplete possible secrets instead of revealing a partial credential.
    pub fn finish(&mut self) -> Vec<Redacted> {
        let mut output = Vec::new();
        self.drain(true, &mut output);
        output
    }

    fn drain(&mut self, finish: bool, output: &mut Vec<Redacted>) {
        while !self.pending.is_empty() {
            let mut longest = 0;
            let mut incomplete = false;
            for pattern in &self.rules.patterns {
                if pattern
                    .iter()
                    .zip(&self.pending)
                    .all(|(a, b)| *a == b.value)
                {
                    if pattern.len() > self.pending.len() {
                        incomplete = true;
                    } else {
                        longest = longest.max(pattern.len());
                    }
                }
            }
            if incomplete && !finish {
                return;
            }
            let mask = longest > 0 || incomplete;
            let count = if incomplete && finish {
                self.pending.len()
            } else {
                longest.max(1)
            };
            for index in 0..count {
                let byte = self.pending.pop_front().expect("bounded prefix");
                let data: &[u8] = if mask {
                    if index == 0 { b"[REDACTED]" } else { b"" }
                } else {
                    std::slice::from_ref(&byte.value)
                };
                if let Some(last) = output.last_mut().filter(|last| last.source == byte.source) {
                    last.consumed += 1;
                    last.output.extend_from_slice(data);
                } else {
                    output.push(Redacted {
                        source: byte.source,
                        consumed: 1,
                        output: data.to_vec(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_boundary_overlap_binary_and_incomplete_secret_remain_redacted() {
        let input = b"\xff start abcdef ghi abc end ab";
        for cut in 0..=input.len() {
            let mut redactor = Redactor::new(
                RedactionRules::literals(vec![
                    b"abc".to_vec(),
                    b"abcdef".to_vec(),
                    b"ghi".to_vec(),
                ])
                .unwrap(),
            );
            let mut pieces = redactor.push(1, &input[..cut]);
            pieces.extend(redactor.push(2, &input[cut..]));
            pieces.extend(redactor.finish());
            assert_eq!(
                pieces.iter().map(|p| p.consumed).sum::<usize>(),
                input.len()
            );
            assert_eq!(
                pieces
                    .into_iter()
                    .flat_map(|p| p.output)
                    .collect::<Vec<_>>(),
                b"\xff start [REDACTED] [REDACTED] [REDACTED] end [REDACTED]",
                "cut {cut}"
            );
        }
    }
}
