//! Collects formatted tracing output for log privacy assertions

use std::io;
use std::sync::Arc;

use parking_lot::Mutex;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedBuffer {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Formats every tracing event on the current thread, at every level, while it is alive
///
/// It is a thread-local default subscriber, so use it from sync tests or a current-thread
/// runtime where the code under test runs on the test thread
pub(crate) struct LogCapture {
    buffer: SharedBuffer,
    _guard: DefaultGuard,
}

impl LogCapture {
    pub(crate) fn start() -> Self {
        let buffer = SharedBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish();

        let guard = tracing::subscriber::set_default(subscriber);
        Self { buffer, _guard: guard }
    }

    /// Everything logged so far
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.buffer.0.lock()).into_owned()
    }
}
