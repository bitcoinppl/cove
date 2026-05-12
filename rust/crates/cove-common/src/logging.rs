use std::{
    collections::VecDeque,
    sync::{Mutex, Once},
};

use once_cell::sync::Lazy;
use tracing::{Event, Level, Subscriber, field::Visit};
use tracing_subscriber::{EnvFilter, Layer, layer::Context, registry::LookupSpan};

static INIT: Once = Once::new();
static TOR_LOG_BUFFER: Lazy<Mutex<VecDeque<String>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
const MAX_TOR_LOG_LINES: usize = 400;

#[derive(Default)]
struct TorEventVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl Visit for TorEventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let value = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(value.trim_matches('"').to_string());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }
}

struct TorLogLayer;

impl<S> Layer<S> for TorLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        if !should_capture_tor_event(metadata.level(), metadata.target()) {
            return;
        }

        let mut visitor = TorEventVisitor::default();
        event.record(&mut visitor);

        let message = visitor.message.unwrap_or_default();
        let fields = if visitor.fields.is_empty() {
            String::new()
        } else {
            format!(" {}", visitor.fields.join(" "))
        };
        let line = format!("[{} {}] {}{}", metadata.level(), metadata.target(), message, fields);

        if !is_tor_related(metadata.target(), &line) {
            return;
        }

        push_tor_log(line);
    }
}

fn should_capture_tor_event(level: &Level, target: &str) -> bool {
    match *level {
        Level::ERROR | Level::WARN | Level::INFO => true,
        Level::DEBUG => target == "arti_client::status" || target.contains("tor_runtime"),
        Level::TRACE => false,
    }
}

fn is_tor_related(target: &str, line: &str) -> bool {
    if target.starts_with("arti") || target.starts_with("tor_") || target.contains("tor_runtime") {
        return true;
    }

    let lowercase = line.to_ascii_lowercase();
    let has_tor_token = lowercase
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token == "tor" || token == "socks5" || token == "socks5h");

    has_tor_token
        || lowercase.contains("onion")
        || lowercase.contains("tor:")
        || lowercase.contains("tor=")
}

fn push_tor_log(line: String) {
    let mut buffer = TOR_LOG_BUFFER.lock().expect("tor log buffer poisoned");
    buffer.push_back(line);
    while buffer.len() > MAX_TOR_LOG_LINES {
        let _ = buffer.pop_front();
    }
}

pub fn tor_connection_logs() -> Vec<String> {
    let buffer = TOR_LOG_BUFFER.lock().expect("tor log buffer poisoned");
    buffer.iter().cloned().collect()
}

pub fn clear_tor_connection_logs() {
    let mut buffer = TOR_LOG_BUFFER.lock().expect("tor log buffer poisoned");
    buffer.clear();
}

pub fn init() {
    init_with_default_filter(default_log_filter());
}

pub fn init_with_default_filter(default_filter: &str) {
    INIT.call_once(|| {
        use tracing_subscriber::{fmt, prelude::*};

        #[cfg(target_os = "android")]
        let fmt_layer = fmt::layer().with_writer(std::io::stderr).with_ansi(false);

        #[cfg(not(target_os = "android"))]
        let fmt_layer = fmt::layer().with_writer(std::io::stdout).with_ansi(false);

        let fmt_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

        tracing_subscriber::registry()
            .with(fmt_layer.with_filter(fmt_filter))
            .with(TorLogLayer)
            .init();
    });
}

fn default_log_filter() -> &'static str {
    #[cfg(debug_assertions)]
    {
        "cove=debug,info"
    }

    #[cfg(not(debug_assertions))]
    {
        "cove=info,info"
    }
}
