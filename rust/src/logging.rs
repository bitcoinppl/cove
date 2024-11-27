use tracing_subscriber::EnvFilter;

pub fn init() {
    use tracing_subscriber::{fmt, prelude::*};

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "cove=debug,info");
    }

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stdout).with_ansi(false))
        .with(EnvFilter::from_default_env())
        .init();
}
