use std::{
    cell::Cell,
    collections::VecDeque,
    fmt,
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{LazyLock, mpsc},
    thread::{self, JoinHandle},
};

use parking_lot::Mutex;
use tracing::{Event, Subscriber, field::Visit};
use tracing_subscriber::{Layer, layer::Context};

use crate::consts::ROOT_DATA_DIR;

pub const RING_CAP_BYTES: usize = 256 * 1024;
pub const MAX_TOTAL_FILE_BYTES: usize = 2 * 1024 * 1024;

const LOG_FILE_BYTES: usize = RING_CAP_BYTES;
const ARCHIVE_FILE_COUNT: usize = (MAX_TOTAL_FILE_BYTES / LOG_FILE_BYTES) - 1;
const CURRENT_LOG_FILE: &str = "cove-rust.log";

static CAPTURE: LazyLock<Capture> = LazyLock::new(Capture::default);

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
}

#[derive(Debug)]
struct RingBuffer {
    lines: VecDeque<String>,
    bytes: usize,
    cap_bytes: usize,
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
    sender: mpsc::Sender<WriterCommand>,
}

enum WriterCommand {
    Write(String),
    Flush(mpsc::Sender<Result<Option<String>, CaptureError>>),
    ClearAndWrite { marker: String, reply: mpsc::Sender<Result<(), CaptureError>> },
    Shutdown,
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
        Self { ring: RingBuffer::new(RING_CAP_BYTES), writer: None, replayed_on_attach: false }
    }
}

impl CaptureState {
    fn attach(&mut self, logs_dir: PathBuf) -> Result<(), CaptureError> {
        std::fs::create_dir_all(&logs_dir).map_err(|source| CaptureError::CreateDir {
            path: logs_dir.display().to_string(),
            source,
        })?;

        let mut file = RollingLogFile::open(logs_dir)?;
        if !self.replayed_on_attach {
            for line in self.ring.iter() {
                file.write_entry(line)?;
            }

            self.replayed_on_attach = true;
        }

        self.writer = Some(LogWriter::spawn(file)?);
        Ok(())
    }

    fn record_line(&mut self, line: impl Into<String>) {
        let mut line = line.into();
        line.push('\n');
        self.ring.push(line.clone());

        if let Some(writer) = &self.writer {
            writer.write(line);
        }
    }

    fn snapshot_start(&self) -> SnapshotStart {
        let Some(writer) = &self.writer else {
            return SnapshotStart::Ring(self.ring.text());
        };

        SnapshotStart::Writer {
            dir: writer.handle.dir.clone(),
            ring_text: self.ring.text(),
            flush: writer.handle.start_flush(),
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
        self.ring.push(format!("{marker}\n"));

        self.writer
            .as_ref()
            .map(|writer| writer.handle.start_clear_and_write(format!("{marker}\n")))
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
    Writer { dir: PathBuf, ring_text: String, flush: Result<FlushReply, CaptureError> },
}

impl SnapshotStart {
    fn finish(self) -> String {
        let text = match self {
            Self::Ring(text) => text,
            Self::Writer { dir, ring_text, flush } => {
                match flush {
                    Ok(flush) => match flush.wait() {
                        Ok(Some(error)) => {
                            return disk_incomplete_snapshot(
                                format!("failed to write Rust diagnostics log file: {error}"),
                                &ring_text,
                            );
                        }
                        Ok(None) => {}
                        Err(error) => {
                            return disk_incomplete_snapshot(
                                format!("failed to flush Rust diagnostics log file: {error}"),
                                &ring_text,
                            );
                        }
                    },
                    Err(error) => {
                        return disk_incomplete_snapshot(
                            format!("failed to flush Rust diagnostics log file: {error}"),
                            &ring_text,
                        );
                    }
                }

                let mut text = String::new();
                text.push_str(&RollingLogFile::snapshot_text_in_dir(&dir));
                text
            }
        };

        if text.is_empty() { "no Rust logs captured\n".to_string() } else { text }
    }
}

fn disk_incomplete_snapshot(reason: String, ring_text: &str) -> String {
    let mut text = String::new();
    text.push_str(&reason);
    text.push('\n');
    text.push_str(
        "using in-memory Rust diagnostics log fallback because disk capture may be incomplete\n",
    );
    text.push_str(ring_text);

    text
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
    fn spawn(file: RollingLogFile) -> Result<Self, CaptureError> {
        let dir = file.dir.clone();
        let (sender, receiver) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("cove-diagnostics-log-writer".to_string())
            .spawn(move || run_writer(file, receiver))
            .map_err(|source| CaptureError::StartWriter { source })?;

        Ok(Self { handle: LogWriterHandle { dir, sender }, join_handle: Some(join_handle) })
    }

    fn write(&self, entry: String) {
        self.handle.write(entry);
    }
}

impl LogWriterHandle {
    fn write(&self, entry: String) {
        let _ = self.sender.send(WriterCommand::Write(entry));
    }

    fn start_flush(&self) -> Result<FlushReply, CaptureError> {
        let (reply, receiver) = mpsc::channel();
        self.sender
            .send(WriterCommand::Flush(reply))
            .map_err(|_| CaptureError::WriterUnavailable { action: "flush diagnostics logs" })?;

        Ok(FlushReply { receiver })
    }

    fn start_clear_and_write(&self, marker: String) -> Result<WriterReply, CaptureError> {
        let (reply, receiver) = mpsc::channel();
        self.sender
            .send(WriterCommand::ClearAndWrite { marker, reply })
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

fn run_writer(mut file: RollingLogFile, receiver: mpsc::Receiver<WriterCommand>) {
    let mut last_write_error = None;

    for command in receiver {
        match command {
            WriterCommand::Write(entry) => {
                if let Err(error) = file.write_entry(&entry) {
                    last_write_error = Some(error.to_string());
                }
            }
            WriterCommand::Flush(reply) => {
                let result = file.flush().map(|()| last_write_error.clone());
                let _ = reply.send(result);
            }
            WriterCommand::ClearAndWrite { marker, reply } => {
                let result = file.clear_and_write(&marker);
                if result.is_ok() {
                    last_write_error = None;
                }
                let _ = reply.send(result);
            }
            WriterCommand::Shutdown => break,
        }
    }
}

impl RingBuffer {
    fn new(cap_bytes: usize) -> Self {
        Self { lines: VecDeque::new(), bytes: 0, cap_bytes }
    }

    fn push(&mut self, mut line: String) {
        if line.len() > self.cap_bytes {
            line = last_bytes_at_token_boundary(&line, self.cap_bytes);
        }

        self.bytes += line.len();
        self.lines.push_back(line);

        while self.bytes > self.cap_bytes {
            let Some(front) = self.lines.pop_front() else { break };
            self.bytes = self.bytes.saturating_sub(front.len());
        }
    }

    fn iter(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    fn text(&self) -> String {
        self.lines.iter().fold(String::new(), |mut text, line| {
            text.push_str(line);
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

    #[test]
    fn ring_cap_keeps_latest_lines_in_order() {
        let mut ring = RingBuffer::new(18);

        ring.push("first\n".to_string());
        ring.push("second\n".to_string());
        ring.push("third\n".to_string());

        assert_eq!(ring.text(), "second\nthird\n");
    }

    #[test]
    fn oversized_line_drops_partial_leading_token() {
        let mut ring = RingBuffer::new(10);

        ring.push("prefix xprvSECRET\n".to_string());

        assert_eq!(ring.text(), "\n");
    }

    #[test]
    fn oversized_line_keeps_tail_from_token_boundary() {
        let mut ring = RingBuffer::new(17);

        ring.push("prefix xprvSECRET suffix\n".to_string());

        assert_eq!(ring.text(), " suffix\n");
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
    fn snapshot_uses_ring_fallback_after_writer_failure() -> eyre::Result<()> {
        let dir = TempDir::new()?;
        let mut state = CaptureState::default();
        state.attach(dir.path().to_path_buf())?;
        std::fs::remove_dir_all(dir.path())?;

        state.record_line(" ".repeat(LOG_FILE_BYTES));
        state.record_line("after disk failure");

        let text = state.snapshot_text();

        assert!(text.contains("failed to write Rust diagnostics log file"));
        assert!(text.contains("disk capture may be incomplete"));
        assert!(text.contains("after disk failure"));

        let text = state.snapshot_text();
        assert!(text.contains("failed to write Rust diagnostics log file"));
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
