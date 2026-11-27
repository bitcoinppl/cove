//! Debug-only scripted searches for the node settings UI tests
//!
//! A UI test can't put an Electrum server on the network the simulator or emulator is on, and
//! every simulator on a machine shares that network, so the sweep itself is left to the Rust
//! tests. While `COVE_UI_TEST_LAN_DISCOVERY` holds `{"searches": [...]}`, each search takes the
//! next entry instead of sweeping, and the last entry answers every search after it. An entry
//! fails with its `error`, `noLocalNetwork` or `localNetworkBlocked`, or finds its `servers`,
//! each a `host` and an Electrum `port` on loopback, which the simulator shares with the machine
//! and the emulator reaches through its port forwards. Those servers go through the same check
//! as swept ones, so one on another network isn't listed. With `keepSearching`, the search
//! stays open after checking them until it is stopped or reaches its time limit
//!
//! It replaces the search with whatever the variable says, so it is compiled only with debug
//! assertions, which no release profile enables, and applies only while the variable is set

use std::net::{Ipv4Addr, SocketAddr};

use parking_lot::Mutex;
use serde::Deserialize;

use super::{Candidate, DiscoveredNode, NodeDiscoveryError, Route, Search};
use crate::{network::Network, node::Node};

/// Set by the UI test to script the searches
const SCRIPT_ENV: &str = "COVE_UI_TEST_LAN_DISCOVERY";

#[derive(Debug, Deserialize)]
struct Script {
    searches: Vec<ScriptedEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptedEntry {
    error: Option<ScriptedError>,
    #[serde(default)]
    servers: Vec<ScriptedServer>,
    #[serde(default)]
    keep_searching: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ScriptedError {
    NoLocalNetwork,
    LocalNetworkBlocked,
}

#[derive(Debug, Clone, Deserialize)]
struct ScriptedServer {
    host: String,
    port: u16,
}

/// A scripted search in place of a sweep
#[derive(Debug)]
pub struct ScriptedSearch {
    pub search: Search,
    /// Whether the search stays open once its servers are checked
    pub keeps_searching: bool,
}

/// The script last read and how many of its searches ran, so an Android test run, which keeps
/// one process across tests, starts each new script from its first search
static SEARCHES_MADE: Mutex<Option<(String, usize)>> = Mutex::new(None);

/// The next scripted search, or `None` when no UI test set the variable
pub fn next_search(network: Network) -> Option<Result<ScriptedSearch, NodeDiscoveryError>> {
    let json = std::env::var(SCRIPT_ENV).ok()?;

    let script: Script = serde_json::from_str(&json)
        .unwrap_or_else(|error| panic!("{SCRIPT_ENV} is not a valid search script: {error}"));

    let made = {
        let mut searches_made = SEARCHES_MADE.lock();
        let made = match searches_made.as_ref() {
            Some((last_json, made)) if *last_json == json => *made,
            _ => 0,
        };

        *searches_made = Some((json, made + 1));
        made
    };

    let last = script.searches.last().unwrap_or_else(|| panic!("{SCRIPT_ENV} has no searches"));
    let entry = script.searches.get(made).unwrap_or(last).clone();

    match entry.error {
        Some(ScriptedError::NoLocalNetwork) => {
            return Some(Err(NodeDiscoveryError::NoLocalNetwork));
        }
        Some(ScriptedError::LocalNetworkBlocked) => {
            return Some(Err(NodeDiscoveryError::LocalNetworkBlocked));
        }
        None => {}
    }

    let servers = entry.servers.into_iter().map(|server| (server.host, server.port));
    let search = loopback_search(network, servers);

    Some(Ok(ScriptedSearch { search, keeps_searching: entry.keep_searching }))
}

/// A search that checks servers on loopback, each a hostname and port, instead of sweeping
pub fn loopback_search(
    network: Network,
    servers: impl IntoIterator<Item = (String, u16)>,
) -> Search {
    let candidates = servers
        .into_iter()
        .map(|(host, port)| {
            let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            let node = Node::new_electrum(host.clone(), format!("tcp://{address}"), network);
            let found = DiscoveredNode { node, host, address: address.to_string(), software: None };

            Candidate { found, address, route: Route::UiTestLoopback }
        })
        .collect();

    Search { network, candidates }
}
