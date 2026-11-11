//! Panic hook that leaves a safe record of a Rust panic for the next diagnostics report
//!
//! Release builds abort on panic, so the async log writer never gets a chance to flush the
//! cause. The hook writes a small marker file synchronously before the abort and the next
//! report shows it

use std::panic::PanicHookInfo;

use super::capture;

/// Target shown on the panic line in the marker and the capture
pub(crate) const PANIC_LOG_TARGET: &str = "cove_common::logging::panic";

pub(crate) fn install_hook() {
    let previous_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        let line = panic_line(info);
        // no tracing call here: a panic raised inside a subscriber that holds its own lock
        // would hang re-entering it instead of aborting
        capture::record_panic(&line);

        // the default hook prints the payload, which can hold secrets from a formatted
        // message, so only debug builds keep it for local backtraces
        if cfg!(debug_assertions) {
            previous_hook(info);
        }
    }));
}

/// Describe a panic by location, thread and current span, never by its payload
fn panic_line(info: &PanicHookInfo<'_>) -> String {
    let location = info
        .location()
        .map(|location| format!("{}:{}:{}", location.file(), location.line(), location.column()))
        .unwrap_or_else(|| "unknown".to_string());

    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    let span = tracing::Span::current();
    let operation = span.metadata().map_or("none", |metadata| metadata.name());

    format!("Rust panic at {location} thread={thread_name} span={operation}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_line_omits_payload() {
        let line = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
        let hook_line = line.clone();
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            *hook_line.lock() = panic_line(info);
        }));

        let result = std::thread::Builder::new()
            .name("panic-line-test".to_string())
            .spawn(|| panic!("secret payload"))
            .expect("thread spawns")
            .join();
        std::panic::set_hook(previous_hook);

        let line = line.lock().clone();
        assert!(result.is_err());
        assert!(line.contains("logging/panic.rs"));
        assert!(line.contains("thread=panic-line-test"));
        assert!(!line.contains("secret payload"));
    }
}
