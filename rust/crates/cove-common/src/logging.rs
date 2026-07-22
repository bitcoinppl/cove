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

        let fmt_layer = fmt::layer().with_ansi(false);

        #[cfg(target_os = "android")]
        let fmt_layer = fmt_layer.with_writer(std::io::stderr);

        #[cfg(not(target_os = "android"))]
        let fmt_layer = fmt_layer.with_writer(std::io::stdout);

        tracing_subscriber::registry()
            .with(fmt_layer)
            .with(capture::layer())
            .with(EnvFilter::from_default_env())
            .init();
    });
}
