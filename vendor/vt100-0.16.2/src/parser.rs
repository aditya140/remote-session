/// A parser for terminal output which produces an in-memory representation of
/// the terminal contents.
pub struct Parser<CB: crate::callbacks::Callbacks = ()> {
    parser: vte::Parser,
    screen: crate::perform::WrappedScreen<CB>,
}

impl Parser {
    /// Creates a new terminal parser of the given size and with the given
    /// amount of scrollback.
    #[must_use]
    pub fn new(rows: u16, cols: u16, scrollback_len: usize) -> Self {
        Self {
            parser: vte::Parser::new(),
            screen: crate::perform::WrappedScreen::new(
                rows,
                cols,
                scrollback_len,
            ),
        }
    }
}

impl<CB: crate::callbacks::Callbacks> Parser<CB> {
    /// Creates a new terminal parser of the given size and with the given
    /// amount of scrollback. Terminal events will be reported via method
    /// calls on the provided [`Callbacks`](crate::callbacks::Callbacks)
    /// implementation.
    pub fn new_with_callbacks(
        rows: u16,
        cols: u16,
        scrollback_len: usize,
        callbacks: CB,
    ) -> Self {
        Self {
            parser: vte::Parser::new(),
            screen: crate::perform::WrappedScreen::new_with_callbacks(
                rows,
                cols,
                scrollback_len,
                callbacks,
            ),
        }
    }

    /// Processes the contents of the given byte string, and updates the
    /// in-memory terminal state.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }

    /// Returns a reference to a [`Screen`](crate::Screen) object containing
    /// the terminal state.
    #[must_use]
    pub fn screen(&self) -> &crate::Screen {
        &self.screen.screen
    }

    /// Returns a mutable reference to a [`Screen`](crate::Screen) object
    /// containing the terminal state.
    #[must_use]
    pub fn screen_mut(&mut self) -> &mut crate::Screen {
        &mut self.screen.screen
    }

    /// Returns a reference to the [`Callbacks`](crate::callbacks::Callbacks)
    /// state object passed into the constructor.
    pub fn callbacks(&self) -> &CB {
        &self.screen.callbacks
    }

    /// Returns a mutable reference to the
    /// [`Callbacks`](crate::callbacks::Callbacks) state object passed into
    /// the constructor.
    pub fn callbacks_mut(&mut self) -> &mut CB {
        &mut self.screen.callbacks
    }
}

impl Default for Parser {
    /// Returns a parser with dimensions 80x24 and no scrollback.
    fn default() -> Self {
        Self::new(24, 80, 0)
    }
}

impl std::io::Write for Parser {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.process(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "checkpoint")]
impl Parser {
    /// Complete, versioned state, including both grids and incomplete parser input.
    /// The encoded checkpoint is limited to eight MiB.
    pub fn checkpoint(&self) -> std::io::Result<Vec<u8>> {
        if !self.screen.screen.checkpoint_valid() || !self.parser.checkpoint_valid() {
            return Err(std::io::Error::other("terminal state outside checkpoint bounds"));
        }
        let mut bytes = b"QGVT1".to_vec();
        bytes.extend(postcard::to_allocvec(&(&self.parser, &self.screen.screen))
            .map_err(|_| std::io::Error::other("terminal checkpoint encoding failed"))?);
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(std::io::Error::other("terminal checkpoint exceeds eight MiB"));
        }
        Ok(bytes)
    }
    /// Restore a bounded checkpoint. Malformed indexes, dimensions and cells are refused.
    pub fn from_checkpoint(bytes: &[u8]) -> std::io::Result<Self> {
        if bytes.len() > 8 * 1024 * 1024 || !bytes.starts_with(b"QGVT1") {
            return Err(std::io::Error::other("unsupported terminal checkpoint"));
        }
        let ((parser, screen), remaining): ((vte::Parser, crate::Screen), _) =
            postcard::take_from_bytes(&bytes[5..])
                .map_err(|_| std::io::Error::other("invalid terminal checkpoint"))?;
        if !remaining.is_empty() || !parser.checkpoint_valid() || !screen.checkpoint_valid() {
            return Err(std::io::Error::other("invalid terminal checkpoint state"));
        }
        Ok(Self { parser, screen: crate::perform::WrappedScreen { screen, callbacks: () } })
    }
}
