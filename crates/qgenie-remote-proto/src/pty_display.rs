//! A bounded decoder guard shared by consumers of public PTY byte streams.
//! Apply only to presentation input; preserve original bytes for transport/recording.
const MAX_OSC_BYTES: usize = 4096;
#[derive(Default)]
pub struct PtyDisplayGuard {
    escaped: bool,
    osc: Option<usize>,
    discarding: bool,
}
impl PtyDisplayGuard {
    /// Bound OSC strings across arbitrary PTY frame boundaries. A truncated OSC
    /// receives CAN, then its remaining content is discarded until a terminator.
    /// Ordinary terminal bytes, including split UTF-8 and CSI sequences, pass unchanged.
    pub fn decode(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if self.discarding {
                match byte {
                    0x07 | 0x18 | 0x1a => {
                        self.discarding = false;
                        output.push(byte);
                    }
                    0x1b => {
                        self.discarding = false;
                        self.escaped = true;
                        output.push(byte);
                    }
                    _ => {}
                }
                continue;
            }
            if let Some(length) = &mut self.osc {
                match byte {
                    0x07 | 0x18 | 0x1a => self.osc = None,
                    0x1b => {
                        self.osc = None;
                        self.escaped = true;
                    }
                    _ if *length >= MAX_OSC_BYTES => {
                        output.push(0x18);
                        self.osc = None;
                        self.discarding = true;
                        continue;
                    }
                    _ => *length += 1,
                }
            } else if self.escaped {
                match byte {
                    b']' => {
                        self.escaped = false;
                        self.osc = Some(0);
                    }
                    0x18 | 0x1a | 0x20..=0x7e => self.escaped = false,
                    _ => {}
                }
            } else if byte == 0x1b {
                self.escaped = true;
            }
            output.push(byte);
        }
        output
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_bytes_and_bounds_unterminated_osc_across_frames() {
        let mut guard = PtyDisplayGuard::default();
        let plain = "✓漢\x1b[31mred\x1b[0m\x1b]0;short title\x07".as_bytes();
        let actual: Vec<_> = plain
            .iter()
            .flat_map(|byte| guard.decode(&[*byte]))
            .collect();
        assert_eq!(actual, plain);
        // C0 controls inside ESC preserve the parser's escape state.
        assert_eq!(guard.decode(b"\x1b\0]0;"), b"\x1b\0]0;");
        let chunk = vec![b'x'; 32768];
        let mut retained = 0;
        for _ in 0..256 {
            retained += guard.decode(&chunk).len();
        }
        assert!(retained <= MAX_OSC_BYTES + 1);
        assert_eq!(guard.decode(b"\x1b\\after"), b"\x1b\\after");
        assert_eq!(
            guard.decode(b"\x1b]0;normal\x07end"),
            b"\x1b]0;normal\x07end"
        );
    }
}
