pub fn init() {
    use env_logger::Builder;

    let mut builder = Builder::new();
    builder.parse_env("RUST_LOG");

    builder.init()
}
