use std::{
    cell::Cell,
    collections::VecDeque,
    fmt,
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use parking_lot::Mutex;
use tracing::{Event, Subscriber, field::Visit};
use tracing_subscriber::{Layer, layer::Context};

use crate::consts::ROOT_DATA_DIR;

pub const RING_CAP_BYTES: usize = 256 * 1024;
pub const MAX_TOTAL_FILE_BYTES: usize = 2 * 1024 * 1024;

const LOG_FILE_BYTES: usize = RING_CAP_BYTES;
const ARCHIVE_FILE_COUNT: usize = (MAX_TOTAL_FILE_BYTES / LOG_FILE_BYTES) - 1;
const WRITER_QUEUE_CAPACITY: usize = 64;
const CURRENT_LOG_FILE: &str = "cove-rust.log";
const PANIC_MARKER_FILE: &str = "cove-rust-panics.log";

/// Restarts the panic marker file once a crash loop grows it past this size
const PANIC_MARKER_MAX_BYTES: u64 = 16 * 1024;

/// Bounds how long the panic hook waits for the capture lock before relying on the marker
const PANIC_CAPTURE_LOCK_TIMEOUT: Duration = Duration::from_millis(50);

/// Text that identifies a gap marker line in persisted history
const GAP_MARKER: &str = "diagnostics log gap:";

const CAPTURE_LOG_TARGET: &str = module_path!();

static CAPTURE: LazyLock<Capture> = LazyLock::new(Capture::default);

// kept outside the capture lock so the panic hook never waits on a lock the panicking
// thread may already hold
static PANIC_MARKER_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

thread_local! {
    static FORMATTING_EVENT: Cell<bool> = const { Cell::new(false) };
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("failed to create diagnostics log directory {path}: {source}")]
    CreateDir { path: String, source: std::io::Error },

    #[error("failed to open diagnostics log file {path}: {source}")]
    OpenFile { path: String, source: std::io::Error },

    #[error("failed to write diagnostics log file {path}: {source}")]
    WriteFile { path: String, source: std::io::Error },

    #[error("failed to rotate diagnostics log file {path}: {source}")]
    RotateFile { path: String, source: std::io::Error },

    #[error("failed to remove diagnostics log file {path}: {source}")]
    RemoveFile { path: String, source: std::io::Error },

    #[error("failed to remove diagnostics log directory {path}: {source}")]
    RemoveDir { path: String, source: std::io::Error },

    #[error("failed to start diagnostics log writer: {source}")]
    StartWriter { source: std::io::Error },

    #[error("diagnostics log writer is unavailable while trying to {action}")]
    WriterUnavailable { action: &'static str },
}

#[derive(Debug, Clone, Copy)]
pub struct CaptureLayer;

#[derive(Default)]
struct Capture {
    state: Mutex<CaptureState>,
}

struct CaptureState {
    ring: RingBuffer,
    writer: Option<LogWriter>,
    replayed_on_attach: bool,

    // sequence number of the last recorded line, so the writer can detect queue drops
    last_seq: u64,
}

#[derive(Debug)]
struct RingBuffer {
    lines: VecDeque<RingLine>,
    bytes: usize,
    cap_bytes: usize,
}

#[derive(Debug, Clone)]
struct RingLine {
    seq: u64,
    text: String,
}

#[derive(Debug)]
struct RollingLogFile {
    dir: PathBuf,
    file: File,
    current_size: usize,
}

struct LogWriter {
    handle: LogWriterHandle,
    join_handle: Option<JoinHandle<()>>,
}

#[derive(Clone)]
struct LogWriterHandle {
    dir: PathBuf,
    sender: mpsc::SyncSender<WriterCommand>,
    health: Arc<WriterHealth>,
}

/// Writer progress shared with snapshots, readable even when the writer thread is gone
#[derive(Default)]
struct WriterHealth {
    persisted_through: AtomicU64,
}

enum WriterCommand {
    Write {
        seq: u64,
        entry: String,
    },
    Flush {
        through_seq: u64,
        reply: mpsc::Sender<Result<Option<String>, CaptureError>>,
    },
    ClearAndWrite {
        marker: String,
        through_seq: u64,
        reply: mpsc::Sender<Result<(), CaptureError>>,
    },
    Shutdown,
}

/// Entries the writer could not persist since the last gap marker reached disk
#[derive(Debug, Default)]
struct PendingGap {
    queue_full: usize,
    write_failed: usize,
    last_error: Option<String>,
}

/// Writer-thread state that turns lost entries into gap markers in the persisted history
struct DiskWriter {
    file: RollingLogFile,
    health: Arc<WriterHealth>,
    last_seen_seq: u64,
    pending_gap: PendingGap,
}

pub fn layer() -> CaptureLayer {
    CaptureLayer
}

pub fn attach_to_default_logs_dir() -> Result<(), CaptureError> {
    attach(ROOT_DATA_DIR.join("logs"))
}

pub fn attach(logs_dir: PathBuf) -> Result<(), CaptureError> {
    CAPTURE.state.lock().attach(logs_dir)
}

pub fn snapshot_text() -> String {
    let snapshot = CAPTURE.state.lock().snapshot_start();
    snapshot.finish()
}

/// Record a panic line synchronously in the marker file, then in the capture if possible
///
/// Called from the panic hook, so it never blocks on the capture lock: the panicking thread
/// may already hold it, and release builds abort right after the hook returns
pub(crate) fn record_panic(line: &str) {
    let line = format!("{} ERROR {}: {line}", timestamp(), super::panic::PANIC_LOG_TARGET);

    let marker_dir = PANIC_MARKER_DIR.try_lock().and_then(|dir| dir.clone());
    if let Some(dir) = marker_dir {
        // nothing useful can be done with a failure inside the panic hook
        let _ = write_panic_marker(&dir, &line);
    }

    if let Some(mut state) = CAPTURE.state.try_lock_for(PANIC_CAPTURE_LOCK_TIMEOUT) {
        state.record_line(line);
    }
}

pub fn clear() -> Result<(), CaptureError> {
    let clear = CAPTURE.state.lock().start_clear()?;
    if let Some(clear) = clear {
        clear.wait()?;
    }

    Ok(())
}

pub fn clear_default_logs_dir() -> Result<(), CaptureError> {
    let logs_dir = ROOT_DATA_DIR.join("logs");
    let writer = CAPTURE.state.lock().detach_writer_for_dir(&logs_dir);
    let should_reattach = writer.is_some();
    drop(writer);

    let remove_result = remove_dir_all_if_exists(&logs_dir);
    let reattach_result = if should_reattach { attach(logs_dir) } else { Ok(()) };

    remove_result?;
    reattach_result?;

    Ok(())
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let Some(_guard) = ReentrancyGuard::enter() else {
            return;
        };

        let line = format_event(event);

        CAPTURE.state.lock().record_line(line);
    }
}

struct ReentrancyGuard;

impl ReentrancyGuard {
    fn enter() -> Option<Self> {
        let already_formatting = FORMATTING_EVENT.with(|formatting| {
            let already_formatting = formatting.get();
            formatting.set(true);
            already_formatting
        });

        if already_formatting {
            return None;
        }

        Some(Self)
    }
}

impl Drop for ReentrancyGuard {
    fn drop(&mut self) {
        FORMATTING_EVENT.with(|formatting| formatting.set(false));
    }
}

impl Default for CaptureState {
    fn default() -> Self {
        Self {
            ring: RingBuffer::new(RING_CAP_BYTES),
            writer: None,
            replayed_on_attach: false,
            last_seq: 0,
        }
    }
}

impl CaptureState {
    fn attach(&mut self, logs_dir: PathBuf) -> Result<(), CaptureError> {
        let previous_writer = self.writer.take();
        drop(previous_writer);

        std::fs::create_dir_all(&logs_dir).map_err(|source| CaptureError::CreateDir {
            path: logs_dir.display().to_string(),
            source,
        })?;

        let mut file = RollingLogFile::open(logs_dir.clone())?;
        if !self.replayed_on_attach {
            for line in self.ring.iter() {
                file.write_entry(line)?;
            }

            self.replayed_on_attach = true;
        }

        self.writer = Some(LogWriter::spawn(file, self.last_seq)?);
        *PANIC_MARKER_DIR.lock() = Some(logs_dir);

        Ok(())
    }

    fn record_line(&mut self, line: impl Into<String>) {
        let mut line = line.into();
        line.push('\n');
        self.last_seq += 1;
        self.ring.push(self.last_seq, line.clone());

        if let Some(writer) = &self.writer {
            writer.write(self.last_seq, line);
        }
    }

    fn snapshot_start(&self) -> SnapshotStart {
        let Some(writer) = &self.writer else {
            return SnapshotStart::Ring(self.ring.text_after(0));
        };

        SnapshotStart::Writer {
            dir: writer.handle.dir.clone(),
            ring: self.ring.lines_after(0),
            health: writer.handle.health.clone(),
            flush: writer.handle.start_flush(self.last_seq),
        }
    }

    #[cfg(test)]
    fn snapshot_text(&self) -> String {
        self.snapshot_start().finish()
    }

    #[cfg(test)]
    fn clear(&mut self) -> Result<(), CaptureError> {
        if let Some(clear) = self.start_clear()? {
            clear.wait()?;
        }

        Ok(())
    }

    fn start_clear(&mut self) -> Result<Option<WriterReply>, CaptureError> {
        let marker = format!("diagnostics logs cleared at {}", timestamp());
        self.ring.clear();
        self.last_seq += 1;
        self.ring.push(self.last_seq, format!("{marker}\n"));

        self.writer
            .as_ref()
            .map(|writer| writer.handle.start_clear_and_write(format!("{marker}\n"), self.last_seq))
            .transpose()
    }

    fn detach_writer_for_dir(&mut self, logs_dir: &Path) -> Option<LogWriter> {
        self.ring.clear();
        self.replayed_on_attach = false;

        if self.writer.as_ref().is_none_or(|writer| writer.handle.dir != logs_dir) {
            return None;
        }

        self.writer.take()
    }
}

enum SnapshotStart {
    Ring(String),
    Writer {
        dir: PathBuf,
        ring: Vec<RingLine>,
        health: Arc<WriterHealth>,
        flush: Result<FlushReply, CaptureError>,
    },
}

impl SnapshotStart {
    fn finish(self) -> String {
        let text = match self {
            Self::Ring(text) => text,
            Self::Writer { dir, ring, health, flush } => {
                let pending_gap = match flush.and_then(FlushReply::wait) {
                    Ok(pending_gap) => pending_gap,
                    Err(error) => {
                        Some(format!("failed to flush Rust diagnostics log file: {error}"))
                    }
                };

                DiskSnapshot {
                    panics: read_panic_markers(&dir),
                    history: RollingLogFile::snapshot_text_in_dir(&dir),
                    pending_gap,
                    ring,
                    persisted_through: health.persisted_through.load(Ordering::Acquire),
                }
                .render()
            }
        };

        if text.is_empty() { "no Rust logs captured\n".to_string() } else { text }
    }
}

/// Persisted history plus whatever the writer has not managed to put on disk yet
struct DiskSnapshot {
    panics: String,
    history: String,
    pending_gap: Option<String>,
    ring: Vec<RingLine>,
    persisted_through: u64,
}

impl DiskSnapshot {
    fn render(self) -> String {
        let mut text = String::new();
        if !self.panics.is_empty() {
            text.push_str("Rust panics recorded since diagnostics logs were last cleared:\n");
            text.push_str(&self.panics);
            text.push_str("end of recorded Rust panics\n");
        }

        // one report-level summary, so a reader sees the gaps before reading the history
        let recorded_gaps = self.history.lines().filter(|line| line.contains(GAP_MARKER)).count();
        let summary = match (recorded_gaps, &self.pending_gap) {
            (0, None) => None,
            (gaps, None) => Some(format!("{gaps} gap(s) marked in saved history")),
            (0, Some(pending)) => Some(pending.clone()),
            (gaps, Some(pending)) => {
                Some(format!("{gaps} gap(s) marked in saved history; {pending}"))
            }
        };

        if let Some(summary) = summary {
            text.push_str(&format!(
                "{} WARN {CAPTURE_LOG_TARGET}: diagnostics log capture incomplete: {summary}\n",
                timestamp()
            ));
        }

        text.push_str(&self.history);

        // while the writer is failing, lines after the last persisted entry live only in memory
        if self.pending_gap.is_some() {
            let unpersisted = self.ring.iter().filter(|line| line.seq > self.persisted_through);
            text.push_str("Rust log lines not yet written to disk:\n");
            for line in unpersisted {
                text.push_str(&line.text);
            }
        }

        text
    }
}

struct FlushReply {
    receiver: mpsc::Receiver<Result<Option<String>, CaptureError>>,
}

impl FlushReply {
    fn wait(self) -> Result<Option<String>, CaptureError> {
        self.receiver
            .recv()
            .map_err(|_| CaptureError::WriterUnavailable { action: "flush diagnostics logs" })?
    }
}

struct WriterReply {
    receiver: mpsc::Receiver<Result<(), CaptureError>>,
    action: &'static str,
}

impl WriterReply {
    fn wait(self) -> Result<(), CaptureError> {
        self.receiver.recv().map_err(|_| CaptureError::WriterUnavailable { action: self.action })?
    }
}

impl LogWriter {
    fn spawn(file: RollingLogFile, last_seq: u64) -> Result<Self, CaptureError> {
        let dir = file.dir.clone();
        let (sender, receiver) = mpsc::sync_channel(WRITER_QUEUE_CAPACITY);
        let health = Arc::new(WriterHealth { persisted_through: AtomicU64::new(last_seq) });
        let disk_writer = DiskWriter::new(file, health.clone(), last_seq);
        let join_handle = thread::Builder::new()
            .name("cove-diagnostics-log-writer".to_string())
            .spawn(move || disk_writer.run(receiver))
            .map_err(|source| CaptureError::StartWriter { source })?;

        let handle = LogWriterHandle { dir, sender, health };

        Ok(Self { handle, join_handle: Some(join_handle) })
    }

    fn write(&self, seq: u64, entry: String) {
        self.handle.write(seq, entry);
    }
}

impl LogWriterHandle {
    fn write(&self, seq: u64, entry: String) {
        let entry = if entry.len() > LOG_FILE_BYTES {
            last_bytes_at_token_boundary(&entry, LOG_FILE_BYTES)
        } else {
            entry
        };

        // a full queue drops the entry; the writer sees the sequence jump and marks the gap
        let _ = self.sender.try_send(WriterCommand::Write { seq, entry });
    }

    fn start_flush(&self, through_seq: u64) -> Result<FlushReply, CaptureError> {
        let (reply, receiver) = mpsc::channel();
        self.sender
            .send(WriterCommand::Flush { through_seq, reply })
            .map_err(|_| CaptureError::WriterUnavailable { action: "flush diagnostics logs" })?;

        Ok(FlushReply { receiver })
    }

    fn start_clear_and_write(
        &self,
        marker: String,
        through_seq: u64,
    ) -> Result<WriterReply, CaptureError> {
        let (reply, receiver) = mpsc::channel();
        self.sender
            .send(WriterCommand::ClearAndWrite { marker, through_seq, reply })
            .map_err(|_| CaptureError::WriterUnavailable { action: "clear diagnostics logs" })?;

        Ok(WriterReply { receiver, action: "clear diagnostics logs" })
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        let _ = self.handle.sender.send(WriterCommand::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl PendingGap {
    fn is_empty(&self) -> bool {
        self.queue_full == 0 && self.write_failed == 0
    }

    fn describe(&self) -> String {
        let missing = self.queue_full + self.write_failed;
        let mut text = format!(
            "{missing} entries not written to disk (writer queue full: {}, write failed: {})",
            self.queue_full, self.write_failed
        );
        if let Some(error) = &self.last_error {
            text.push_str(&format!(", last error: {error}"));
        }

        text
    }
}

impl DiskWriter {
    fn new(file: RollingLogFile, health: Arc<WriterHealth>, last_seq: u64) -> Self {
        Self { file, health, last_seen_seq: last_seq, pending_gap: PendingGap::default() }
    }

    fn run(mut self, receiver: mpsc::Receiver<WriterCommand>) {
        for command in receiver {
            match command {
                WriterCommand::Write { seq, entry } => {
                    self.note_sent_through(seq.saturating_sub(1));
                    self.last_seen_seq = seq;
                    self.write(seq, &entry);
                }
                WriterCommand::Flush { through_seq, reply } => {
                    self.note_sent_through(through_seq);
                    self.write_pending_gap_marker();

                    let pending_gap = (!self.pending_gap.is_empty()).then(|| {
                        format!("{} since the last saved entry", self.pending_gap.describe())
                    });
                    let _ = reply.send(self.file.flush().map(|()| pending_gap));
                }
                WriterCommand::ClearAndWrite { marker, through_seq, reply } => {
                    let result = self.file.clear_and_write(&marker);
                    if result.is_ok() {
                        self.pending_gap = PendingGap::default();
                        self.last_seen_seq = through_seq;
                        self.health.persisted_through.store(through_seq, Ordering::Release);
                    }

                    let _ = reply.send(result);
                }
                WriterCommand::Shutdown => {
                    self.write_pending_gap_marker();
                    break;
                }
            }
        }
    }

    /// Count entries recorded through `seq` that never reached the queue
    fn note_sent_through(&mut self, seq: u64) {
        if seq > self.last_seen_seq {
            self.pending_gap.queue_full += (seq - self.last_seen_seq) as usize;
            self.last_seen_seq = seq;
        }
    }

    fn write(&mut self, seq: u64, entry: &str) {
        // an entry written after an unmarked gap would hide the gap, so skip it too
        if !self.write_pending_gap_marker() {
            self.pending_gap.write_failed += 1;
            return;
        }

        match self.file.write_entry(entry) {
            Ok(()) => self.health.persisted_through.store(seq, Ordering::Release),
            Err(error) => {
                self.pending_gap.write_failed += 1;
                self.pending_gap.last_error = Some(error.to_string());
            }
        }
    }

    /// Persist one marker line for the pending gap, returning whether no gap remains unmarked
    fn write_pending_gap_marker(&mut self) -> bool {
        if self.pending_gap.is_empty() {
            return true;
        }

        let marker = format!(
            "{} WARN {CAPTURE_LOG_TARGET}: {GAP_MARKER} {}\n",
            timestamp(),
            self.pending_gap.describe()
        );

        match self.file.write_entry(&marker) {
            Ok(()) => {
                self.pending_gap = PendingGap::default();
                true
            }
            Err(error) => {
                self.pending_gap.last_error = Some(error.to_string());
                false
            }
        }
    }
}

impl RingBuffer {
    fn new(cap_bytes: usize) -> Self {
        Self { lines: VecDeque::new(), bytes: 0, cap_bytes }
    }

    fn push(&mut self, seq: u64, mut line: String) {
        if line.len() > self.cap_bytes {
            line = last_bytes_at_token_boundary(&line, self.cap_bytes);
        }

        self.bytes += line.len();
        self.lines.push_back(RingLine { seq, text: line });

        while self.bytes > self.cap_bytes {
            let Some(front) = self.lines.pop_front() else { break };
            self.bytes = self.bytes.saturating_sub(front.text.len());
        }
    }

    fn iter(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(|line| line.text.as_str())
    }

    fn lines_after(&self, seq: u64) -> Vec<RingLine> {
        self.lines.iter().filter(|line| line.seq > seq).cloned().collect()
    }

    fn text_after(&self, seq: u64) -> String {
        self.lines.iter().filter(|line| line.seq > seq).fold(String::new(), |mut text, line| {
            text.push_str(&line.text);
            text
        })
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.bytes = 0;
    }
}

impl RollingLogFile {
    fn open(dir: PathBuf) -> Result<Self, CaptureError> {
        let path = current_log_path(&dir);
        let file = OpenOptions::new().create(true).append(true).open(&path).map_err(|source| {
            CaptureError::OpenFile { path: path.display().to_string(), source }
        })?;
        let current_size =
            file.metadata().map(|metadata| metadata.len() as usize).unwrap_or_default();

        Ok(Self { dir, file, current_size })
    }

    fn write_entry(&mut self, entry: &str) -> Result<(), CaptureError> {
        let entry = if entry.len() > LOG_FILE_BYTES {
            last_bytes_at_token_boundary(entry, LOG_FILE_BYTES)
        } else {
            entry.to_string()
        };

        if self.current_size > 0 && self.current_size + entry.len() > LOG_FILE_BYTES {
            self.rotate()?;
        }

        self.file.write_all(entry.as_bytes()).map_err(|source| CaptureError::WriteFile {
            path: current_log_path(&self.dir).display().to_string(),
            source,
        })?;
        self.current_size += entry.len();

        Ok(())
    }

    fn flush(&mut self) -> Result<(), CaptureError> {
        self.file.flush().map_err(|source| CaptureError::WriteFile {
            path: current_log_path(&self.dir).display().to_string(),
            source,
        })
    }

    fn rotate(&mut self) -> Result<(), CaptureError> {
        self.file.flush().map_err(|source| CaptureError::WriteFile {
            path: current_log_path(&self.dir).display().to_string(),
            source,
        })?;

        let oldest = archived_log_path(&self.dir, ARCHIVE_FILE_COUNT);
        remove_file_if_exists(&oldest)?;

        for index in (1..ARCHIVE_FILE_COUNT).rev() {
            let source = archived_log_path(&self.dir, index);
            let destination = archived_log_path(&self.dir, index + 1);
            rename_if_exists(&source, &destination)?;
        }

        let current = current_log_path(&self.dir);
        let first_archive = archived_log_path(&self.dir, 1);
        rename_if_exists(&current, &first_archive)?;

        *self = Self::open(self.dir.clone())?;
        Ok(())
    }

    fn remove_all_logs(&self) -> Result<(), CaptureError> {
        remove_file_if_exists(&self.dir.join(PANIC_MARKER_FILE))?;
        remove_file_if_exists(&current_log_path(&self.dir))?;
        for index in 1..=ARCHIVE_FILE_COUNT {
            remove_file_if_exists(&archived_log_path(&self.dir, index))?;
        }

        Ok(())
    }

    fn clear_and_write(&mut self, marker: &str) -> Result<(), CaptureError> {
        self.remove_all_logs()?;
        *self = Self::open(self.dir.clone())?;
        self.write_entry(marker)
    }

    fn snapshot_text_in_dir(dir: &Path) -> String {
        Self::log_paths(dir).fold(String::new(), |mut text, path| {
            match std::fs::read_to_string(path) {
                Ok(file_text) => text.push_str(&file_text),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    text.push_str(&format!("failed to read Rust diagnostics log file: {error}\n"));
                }
            }

            text
        })
    }

    fn log_paths(dir: &Path) -> impl Iterator<Item = PathBuf> + '_ {
        (1..=ARCHIVE_FILE_COUNT)
            .rev()
            .map(|index| archived_log_path(dir, index))
            .chain(std::iter::once(current_log_path(dir)))
    }
}

fn format_event(event: &Event<'_>) -> String {
    let metadata = event.metadata();
    let mut visitor = EventVisitor::default();
    event.record(&mut visitor);

    let fields = visitor.fields.join(" ");
    match (visitor.message, fields.is_empty()) {
        (Some(message), true) => {
            format!("{} {} {}: {message}", timestamp(), metadata.level(), metadata.target())
        }
        (Some(message), false) => {
            format!(
                "{} {} {}: {message} {fields}",
                timestamp(),
                metadata.level(),
                metadata.target()
            )
        }
        (None, true) => format!("{} {} {}", timestamp(), metadata.level(), metadata.target()),
        (None, false) => {
            format!("{} {} {}: {fields}", timestamp(), metadata.level(), metadata.target())
        }
    }
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl EventVisitor {
    fn record_value(&mut self, field: &tracing::field::Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
            return;
        }

        self.fields.push(format!("{}={value}", field.name()));
    }
}

impl Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.record_value(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_value(field, value.to_string());
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_value(field, value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_value(field, value.to_string());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_value(field, value.to_string());
    }
}

fn write_panic_marker(dir: &Path, line: &str) -> std::io::Result<()> {
    let path = dir.join(PANIC_MARKER_FILE);

    // a crash loop restarts the file instead of growing it, keeping the newest marker
    let restart = path.metadata().is_ok_and(|metadata| metadata.len() > PANIC_MARKER_MAX_BYTES);
    let mut file = OpenOptions::new()
        .create(true)
        .append(!restart)
        .write(true)
        .truncate(restart)
        .open(&path)?;

    file.write_all(format!("{line}\n").as_bytes())?;
    file.sync_data()
}

fn read_panic_markers(dir: &Path) -> String {
    match std::fs::read_to_string(dir.join(PANIC_MARKER_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => format!("failed to read Rust panic marker file: {error}\n"),
    }
}

fn current_log_path(dir: &Path) -> PathBuf {
    dir.join(CURRENT_LOG_FILE)
}

fn archived_log_path(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("cove-rust.{index}.log"))
}

fn rename_if_exists(source: &Path, destination: &Path) -> Result<(), CaptureError> {
    if !source.exists() {
        return Ok(());
    }

    std::fs::rename(source, destination).map_err(|source_error| CaptureError::RotateFile {
        path: format!("{} -> {}", source.display(), destination.display()),
        source: source_error,
    })
}

fn remove_file_if_exists(path: &Path) -> Result<(), CaptureError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(CaptureError::RemoveFile { path: path.display().to_string(), source }),
    }
}

fn remove_dir_all_if_exists(path: &Path) -> Result<(), CaptureError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(CaptureError::RemoveDir { path: path.display().to_string(), source }),
    }
}

fn last_bytes_at_token_boundary(value: &str, max_bytes: usize) -> String {
    let mut start = value.len().saturating_sub(max_bytes);
    while !value.is_char_boundary(start) {
        start += 1;
    }

    if start == 0 {
        return value.to_string();
    }

    while start < value.len() {
        let Some(character) = value[start..].chars().next() else {
            break;
        };
        if !is_redaction_token_character(character) {
            break;
        }

        start += character.len_utf8();
    }

    value[start..].to_string()
}

fn is_redaction_token_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
}

fn timestamp() -> String {
    jiff::Timestamp::now().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn writer_that_replaces_current_file_on_shutdown(dir: &Path) -> LogWriter {
        let writer_dir = dir.to_path_buf();
        let (sender, receiver) = mpsc::sync_channel(WRITER_QUEUE_CAPACITY);
        let join_handle = thread::spawn(move || {
            while let Ok(command) = receiver.recv() {
                if matches!(command, WriterCommand::Shutdown) {
                    std::fs::remove_file(current_log_path(&writer_dir)).unwrap();
                    std::fs::write(current_log_path(&writer_dir), "replacement\n").unwrap();
                    break;
                }
            }
        });

        LogWriter {
            handle: LogWriterHandle {
                dir: dir.to_path_buf(),
                sender,
                health: Arc::new(WriterHealth::default()),
            },
            join_handle: Some(join_handle),
        }
    }

    #[test]
    fn full_writer_queue_keeps_disk_history_and_marks_gap() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        let file = RollingLogFile::open(dir.path().to_path_buf())?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let health = Arc::new(WriterHealth::default());
        let handle =
            LogWriterHandle { dir: dir.path().to_path_buf(), sender, health: health.clone() };

        handle.write(1, "first\n".to_string());
        handle.write(2, "dropped\n".to_string());
        handle.write(3, "also dropped\n".to_string());

        let writer = DiskWriter::new(file, health, 0);
        let join_handle = thread::spawn(move || writer.run(receiver));
        assert_eq!(handle.start_flush(3)?.wait()?, None);

        let history = std::fs::read_to_string(current_log_path(dir.path()))?;
        assert!(history.starts_with("first\n"));
        assert!(history.contains("diagnostics log gap: 2 entries not written to disk"));
        assert!(!history.contains("dropped\n"));

        handle.write(4, "after gap\n".to_string());
        handle.start_clear_and_write("cleared\n".to_string(), 5)?.wait()?;
        assert_eq!(handle.start_flush(5)?.wait()?, None);
        assert_eq!(std::fs::read_to_string(current_log_path(dir.path()))?, "cleared\n");

        handle.sender.send(WriterCommand::Shutdown)?;
        join_handle.join().expect("writer exits cleanly");

        Ok(())
    }

    #[test]
    fn write_error_keeps_disk_history_and_gap_survives_restart() -> eyre::Result<()> {
        let dir = TempDir::new()?;

        // a directory where rotation expects the oldest archive makes the next rotation fail
        std::fs::create_dir_all(archived_log_path(dir.path(), ARCHIVE_FILE_COUNT).join("blocker"))?;

        {
            let mut state = CaptureState::default();
            state.attach(dir.path().to_path_buf())?;
            state.record_line("before failure");
            state.record_line(" ".repeat(LOG_FILE_BYTES));
            state.record_line("after failure");

            let text = state.snapshot_text();
            assert!(text.contains("before failure"));
            assert!(text.contains("after failure"));
            assert!(text.contains("diagnostics log gap: 1 entries not written to disk"));
            assert!(text.contains("diagnostics log capture incomplete: 1 gap(s)"));
            assert!(!text.contains("not yet written to disk:"));
        }

        let mut state = CaptureState::default();
        state.attach(dir.path().to_path_buf())?;
        let text = state.snapshot_text();

        assert!(text.contains("before failure"));
        assert!(text.contains("diagnostics log capture incomplete: 1 gap(s)"));

        Ok(())
    }

    #[test]
    fn panic_marker_survives_into_next_snapshot_until_cleared() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        {
            let mut state = CaptureState::default();
            state.attach(dir.path().to_path_buf())?;
            record_panic("Rust panic at src/example.rs:1:1 thread=main span=none");
        }

        let mut state = CaptureState::default();
        state.attach(dir.path().to_path_buf())?;
        let text = state.snapshot_text();

        assert!(text.starts_with("Rust panics recorded since diagnostics logs were last cleared:"));
        assert!(
            text.contains("ERROR cove_common::logging::panic: Rust panic at src/example.rs:1:1")
        );

        state.clear()?;
        assert!(!state.snapshot_text().contains("Rust panic at"));

        Ok(())
    }

    #[test]
    fn ring_cap_keeps_latest_lines_in_order() {
        let mut ring = RingBuffer::new(18);

        ring.push(1, "first\n".to_string());
        ring.push(2, "second\n".to_string());
        ring.push(3, "third\n".to_string());

        assert_eq!(ring.text_after(0), "second\nthird\n");
    }

    #[test]
    fn oversized_line_drops_partial_leading_token() {
        let mut ring = RingBuffer::new(10);

        ring.push(1, "prefix xprvSECRET\n".to_string());

        assert_eq!(ring.text_after(0), "\n");
    }

    #[test]
    fn oversized_line_keeps_tail_from_token_boundary() {
        let mut ring = RingBuffer::new(17);

        ring.push(1, "prefix xprvSECRET suffix\n".to_string());

        assert_eq!(ring.text_after(0), " suffix\n");
    }

    #[test]
    fn rolling_file_caps_total_archives() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        let mut file = RollingLogFile::open(dir.path().to_path_buf())?;
        let entry = format!("{}\n", "x".repeat(LOG_FILE_BYTES / 2));

        for _ in 0..20 {
            file.write_entry(&entry)?;
        }

        let total_bytes = std::fs::read_dir(dir.path())?
            .map(|entry| entry.map(|entry| entry.metadata().map(|metadata| metadata.len())))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum::<u64>();

        assert!(total_bytes <= MAX_TOTAL_FILE_BYTES as u64);

        Ok(())
    }

    #[test]
    fn attach_replays_ring_once() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        let mut state = CaptureState::default();
        state.record_line("before attach");

        state.attach(dir.path().to_path_buf())?;
        state.attach(dir.path().to_path_buf())?;

        let text = std::fs::read_to_string(current_log_path(dir.path()))?;
        assert_eq!(text.matches("before attach").count(), 1);

        Ok(())
    }

    #[test]
    fn reattach_stops_previous_writer_before_opening_replacement() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        std::fs::write(current_log_path(dir.path()), "original\n")?;

        let mut state = CaptureState {
            writer: Some(writer_that_replaces_current_file_on_shutdown(dir.path())),
            ..CaptureState::default()
        };

        state.attach(dir.path().to_path_buf())?;
        state.record_line("after reattach");

        assert_eq!(state.snapshot_text(), "replacement\nafter reattach\n");

        Ok(())
    }

    #[test]
    fn snapshot_reads_persisted_current_file_after_restart() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        {
            let mut state = CaptureState::default();
            state.attach(dir.path().to_path_buf())?;
            state.record_line("before restart");
        }

        let mut state = CaptureState::default();
        state.attach(dir.path().to_path_buf())?;

        assert!(state.snapshot_text().contains("before restart"));

        Ok(())
    }

    #[test]
    fn snapshot_reads_archives_before_current_file() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        std::fs::write(archived_log_path(dir.path(), 2), "oldest\n")?;
        std::fs::write(archived_log_path(dir.path(), 1), "older\n")?;
        std::fs::write(current_log_path(dir.path()), "current\n")?;

        let mut state = CaptureState::default();
        state.attach(dir.path().to_path_buf())?;

        assert_eq!(state.snapshot_text(), "oldest\nolder\ncurrent\n");

        Ok(())
    }

    #[test]
    fn snapshot_appends_unpersisted_lines_while_writer_fails() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        let mut state = CaptureState::default();
        state.attach(dir.path().to_path_buf())?;
        std::fs::remove_dir_all(dir.path())?;

        state.record_line(" ".repeat(LOG_FILE_BYTES));
        state.record_line("after disk failure");

        let text = state.snapshot_text();

        assert!(text.contains("diagnostics log capture incomplete: 1 entries not written"));
        assert!(text.contains("Rust log lines not yet written to disk:"));
        assert!(text.contains("after disk failure"));

        let text = state.snapshot_text();
        assert!(text.contains("diagnostics log capture incomplete"));
        assert!(text.contains("after disk failure"));

        Ok(())
    }

    #[test]
    fn clear_reopens_file_with_marker() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        let mut state = CaptureState::default();
        state.attach(dir.path().to_path_buf())?;
        state.record_line("before clear");

        state.clear()?;
        state.record_line("after clear");

        let text = state.snapshot_text();
        assert!(!text.contains("before clear"));
        assert!(text.contains("diagnostics logs cleared at"));
        assert!(text.contains("after clear"));

        Ok(())
    }
}
