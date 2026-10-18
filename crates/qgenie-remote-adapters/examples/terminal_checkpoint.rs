//! Export the canonical small terminal checkpoint used by conformance peers.
use std::io::Write;
fn main() -> anyhow::Result<()> {
    let mut screen = qgenie_remote_adapters::Screen::new(24, 80)?;
    screen.process(b"hello");
    std::io::stdout().write_all(&screen.snapshot()?.data)?;
    Ok(())
}
