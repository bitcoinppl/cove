//! Times a built-in Tor start and one request through it, for the startup numbers in the docs
//!
//! `cargo run -p cove-tor --release --example bootstrap_timing -- <dir>` starts cold when `<dir>`
//! is empty and warm when an earlier run left its cache there

use std::{path::PathBuf, time::Instant};

use cove_tor::{BootstrapState, BuiltInTor, BuiltInTorConfig};

#[tokio::main]
async fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: bootstrap_timing <dir>"));
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = BuiltInTorConfig { state_dir: dir.join("state"), cache_dir: dir.join("cache") };
    let started = Instant::now();
    let tor = BuiltInTor::start(config, &tokio::runtime::Handle::current()).unwrap();

    let mut state = tor.state();
    while *state.borrow_and_update() != BootstrapState::Ready {
        state.changed().await.unwrap();
    }

    let ready = started.elapsed();
    let client = cove_http::new_client(&cove_http::NetworkRoute::Tor(tor.proxy().clone())).unwrap();
    let body = client.get("https://check.torproject.org/api/ip").send().await.unwrap();
    let body = body.text().await.unwrap();

    println!("ready after {ready:?}, first request done after {:?}: {body}", started.elapsed());
}
