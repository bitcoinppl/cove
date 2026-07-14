use std::sync::Once;

use tracing_subscriber::EnvFilter;

pub mod capture;

static INIT: Once = Once::new();

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

        tracing_subscriber::registry()
            .with(fmt::layer().with_writer(std::io::stdout).with_ansi(false))
            .with(capture::layer())
            .with(EnvFilter::from_default_env())
            .init();
    });
}
