use std::sync::Once;

use tracing::Level;
use tracing_subscriber::{
    EnvFilter,
    filter::{FilterExt as _, Targets, filter_fn},
};

pub mod capture;
mod panic;

static INIT: Once = Once::new();

/// Capture filter for diagnostics reports, independent of `RUST_LOG`
///
/// Cove crates log at INFO and dependencies at WARN so a developer's console filter
/// can neither flood the bounded capture with debug output nor silence it
const CAPTURE_FILTER: &str = "warn,cove=info";
const CAPTURE_DEFAULT_LEVEL: Level = Level::WARN;
const CAPTURE_COVE_LEVEL: Level = Level::INFO;

pub fn init() {
    INIT.call_once(|| {
        use tracing_subscriber::{fmt, prelude::*};

        if std::env::var("RUST_LOG").is_err() {
            #[cfg(debug_assertions)]
            unsafe {
                std::env::set_var("RUST_LOG", "cove=debug,info")
            };

            #[cfg(not(debug_assertions))]
            unsafe {
                std::env::set_var("RUST_LOG", "cove=info")
            };
        }

        let console_filter = std::env::var("RUST_LOG").unwrap_or_default();

        // the panic hook records its own capture line without blocking, so the capture
        // layer skips the console copy instead of taking the capture lock inside the hook
        let capture_filter = capture_targets().and(filter_fn(|metadata| {
            metadata.target() != panic::PANIC_LOG_TARGET
        }));

        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_writer(std::io::stdout)
                    .with_ansi(false)
                    .with_filter(EnvFilter::from_default_env()),
            )
            .with(capture::layer().with_filter(capture_filter))
            .init();

        panic::install_hook();

        tracing::info!(
            "Rust logging initialized: console_filter={console_filter} capture_filter={CAPTURE_FILTER}"
        );
    });
}

fn capture_targets() -> Targets {
    Targets::new().with_default(CAPTURE_DEFAULT_LEVEL).with_target("cove", CAPTURE_COVE_LEVEL)
}
