use tracing_subscriber::EnvFilter;

pub fn init() {
    use tracing_subscriber::{filter::filter_fn, fmt, prelude::*};

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "cove=debug,info");
    }

    let hide_log = filter_fn(|metadata| {
        !metadata.target().contains("ffi") && metadata.target().contains("cove")
    });

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stdout)
                .with_ansi(false)
                .with_filter(hide_log),
        )
        .with(EnvFilter::from_default_env())
        .init();
}
