//! Runs a fake Esplora and Electrum node with its JSON control API, for UI
//! tests that drive the app in a simulator or emulator
//!
//! Prints one JSON line with the three URLs once every port listens, then
//! serves until SIGINT or SIGTERM

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bitcoin::Network;
use clap::Parser;
use color_eyre::{Result, eyre::WrapErr as _};
use cove_fake_node::FakeNode;
use serde_json::json;
use tokio::signal::unix::{SignalKind, signal};

#[derive(Parser)]
#[command(name = "cove-fake-node", about = "Fake Esplora and Electrum node for Cove UI tests")]
struct Args {
    /// Network whose genesis block the fake chain starts from
    #[arg(long, default_value_t = Network::Signet)]
    network: Network,

    /// Address every server listens on
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    bind: IpAddr,

    /// Port of the JSON control API; 0 picks a free port
    #[arg(long, default_value_t = 39000)]
    control_port: u16,

    /// Port of the Esplora HTTP server; 0 picks a free port
    #[arg(long, default_value_t = 39001)]
    esplora_port: u16,

    /// Port of the Electrum TCP server; 0 picks a free port
    #[arg(long, default_value_t = 39002)]
    electrum_port: u16,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();
    let at = |port| SocketAddr::new(args.bind, port);

    let node = FakeNode::start_on(args.network, at(args.esplora_port), at(args.electrum_port))
        .wrap_err("failed to start the Esplora and Electrum servers")?;
    node.reset();

    let control =
        node.serve_control(at(args.control_port)).wrap_err("failed to start the control API")?;

    // the runner waits for this line, so it goes out only once every port listens
    let ready = json!({
        "control": control.url(),
        "esplora": node.esplora_url(),
        "electrum": node.electrum_url(),
        "network": args.network.to_string(),
    });
    println!("{ready}");

    // both servers stop and close their sockets when `control` and `node` drop
    wait_for_shutdown_signal()
}

fn wait_for_shutdown_signal() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .wrap_err("failed to build the signal runtime")?;

    runtime.block_on(async {
        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut terminate = signal(SignalKind::terminate())?;

        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }

        Ok::<_, std::io::Error>(())
    })?;

    Ok(())
}
