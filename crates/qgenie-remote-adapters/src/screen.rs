use qgenie_remote_proto::{RemoteError, validate_size};
/// Terminal parsing is native adapter state; the durable cursor belongs to sessiond.
/// At most 262144 visible cells and 64 scrollback lines (786432 cells including the alternate screen).
pub struct Screen {
    parser: vt100::Parser,
}
#[derive(Debug)]
pub struct ScreenSnapshot {
    pub format: qgenie_remote_proto::SnapshotFormat,
    pub rows: u16,
    pub cols: u16,
    pub data: Vec<u8>,
}
impl Screen {
    pub fn new(rows: u16, cols: u16) -> Result<Self, RemoteError> {
        validate_size(rows, cols)?;
        Ok(Self {
            parser: vt100::Parser::new(rows, cols, 64),
        })
    }
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), RemoteError> {
        validate_size(rows, cols)?;
        self.parser.screen_mut().set_size(rows, cols);
        Ok(())
    }
    pub fn snapshot(&self) -> Result<ScreenSnapshot, RemoteError> {
        let (rows, cols) = self.parser.screen().size();
        Ok(ScreenSnapshot {
            format: qgenie_remote_proto::SnapshotFormat::Vt100StateV1,
            rows,
            cols,
            data: self.parser.checkpoint().map_err(|_| {
                RemoteError::rejected(
                    qgenie_remote_proto::ErrorCode::FrameTooLarge,
                    "terminal checkpoint is unavailable or exceeds eight MiB",
                )
            })?,
        })
    }
    pub fn from_checkpoint(data: &[u8]) -> Result<Self, RemoteError> {
        Ok(Self {
            parser: vt100::Parser::from_checkpoint(data).map_err(|_| {
                RemoteError::rejected(
                    qgenie_remote_proto::ErrorCode::HistoryTruncated,
                    "terminal checkpoint is invalid",
                )
            })?,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_utf8_ansi_alternate_screen_and_resize_snapshot() {
        let mut screen = Screen::new(24, 80).unwrap();
        screen.process(b"main\x1b[?1049h\x1b[2J\x1b[H\x1b[31m");
        for byte in "✓漢".as_bytes() {
            screen.process(&[*byte]);
        }
        let snapshot = screen.snapshot().unwrap();
        let restored = vt100::Parser::from_checkpoint(&snapshot.data).unwrap();
        assert!(restored.screen().contents().contains("✓漢"));
        assert_eq!(
            restored.screen().cell(0, 0).unwrap().fgcolor(),
            vt100::Color::Idx(1)
        );
        screen.process(b"\x1b[?1049l");
        screen.resize(30, 100).unwrap();
        let snapshot = screen.snapshot().unwrap();
        let restored = vt100::Parser::from_checkpoint(&snapshot.data).unwrap();
        assert!(restored.screen().contents().contains("main"));
        assert_eq!(restored.screen().size(), (30, 100));
        assert!(Screen::new(4096, 4096).is_err());
    }

    #[test]
    fn every_cutover_preserves_hidden_buffers_saved_cursor_modes_and_partial_sequences() {
        let script = concat!(
            "main✓漢\r\nsecond\x1b[3;8r\x1b[?6h\x1b[2;7H\x1b[31m\x1b7",
            "\x1b[?1049h\x1b[2J\x1b[Halt😀\x1b[38;2;12;34;56mcolour",
            "\x1b]0;ignored title\x07\x1bP1;2qignored\x1b\\",
            "\x1b[?1049l\x1b8restored\n\n\n\n\n\n\n\n\nscroll",
            "\x1b[?6l\x1b[r\x1b[?2004h\x1b[?1006h\x1b[?1002h"
        )
        .as_bytes();
        let mut expected = Screen::new(12, 40).unwrap();
        expected.process(script);
        let expected = expected.parser.screen().clone();
        for cut in 0..=script.len() {
            let mut original = Screen::new(12, 40).unwrap();
            original.process(&script[..cut]);
            let checkpoint = original
                .snapshot()
                .unwrap_or_else(|e| panic!("cut {cut}: {e}"));
            let mut restored = Screen::from_checkpoint(&checkpoint.data).unwrap();
            for byte in &script[cut..] {
                restored.process(&[*byte]);
            }
            assert!(
                restored.parser.screen() == &expected,
                "different terminal state at cut {cut}"
            );
        }
    }

    #[test]
    fn checkpoints_bound_osc_and_reject_corruption_without_panics() {
        let mut screen = Screen::new(4, 8).unwrap();
        screen.process(b"\x1b]0;");
        for _ in 0..256 {
            screen.process(&[b'x'; 32768]);
        }
        let checkpoint = screen.snapshot().unwrap();
        assert!(checkpoint.data.len() < 8192);
        let mut restored = Screen::from_checkpoint(&checkpoint.data).unwrap();
        restored.process(b"\x1b\\safe");
        assert!(restored.parser.screen().contents().contains("safe"));
        for index in 0..checkpoint.data.len() {
            let mut mutated = checkpoint.data.clone();
            mutated[index] ^= 0xff;
            if let Ok(mut parser) = Screen::from_checkpoint(&mutated) {
                parser.process(b"tail\x1b[1;2H\x1b7\x1b8\x1b[2J");
                let _ = parser.snapshot();
            }
        }
        assert!(Screen::from_checkpoint(&checkpoint.data[..checkpoint.data.len() - 1]).is_err());
        let mut trailing = checkpoint.data;
        trailing.push(0);
        assert!(Screen::from_checkpoint(&trailing).is_err());
    }

    #[test]
    fn golden_checkpoint_and_narrow_unicode_resizes_remain_safe() {
        let fixture = include_bytes!("../../../conformance/terminal/vt100-state-v1.bin");
        let mut restored = Screen::from_checkpoint(fixture).unwrap();
        assert_eq!(restored.parser.screen().contents(), "hello");
        assert!(restored.snapshot().unwrap().data == fixture);
        restored.process("漢".as_bytes());
        restored.resize(24, 6).unwrap(); // cut a wide glyph in half
        restored.process(b"\x1b[1;6H\x1b[Kx");
        let bytes = restored.snapshot().unwrap().data;
        Screen::from_checkpoint(&bytes).unwrap();
        let mut narrow = Screen::new(2, 1).unwrap();
        narrow.process("漢😀\x1b[2J\u{301}".as_bytes());
        let snapshot = narrow.snapshot().unwrap();
        Screen::from_checkpoint(&snapshot.data).unwrap();
    }
}
