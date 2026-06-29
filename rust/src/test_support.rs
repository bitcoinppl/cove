use std::sync::OnceLock;

/// Serializes tests that mutate process-wide database or global config state
pub(crate) fn global_state_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

    LOCK.get_or_init(tokio::sync::Mutex::default)
}

/// Starts a process-long Tokio runtime for tests that need the shared cove_tokio handle
pub(crate) fn ensure_tokio_runtime() {
    static INIT: OnceLock<()> = OnceLock::new();

    INIT.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);

        std::thread::Builder::new()
            .name("cove-test-tokio".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("create cove test tokio runtime");

                runtime.block_on(async move {
                    cove_tokio::init();
                    sender.send(()).expect("signal cove test tokio runtime");
                    std::future::pending::<()>().await;
                });
            })
            .expect("spawn cove test tokio runtime thread");

        receiver.recv().expect("wait for cove test tokio runtime");
    });
}
