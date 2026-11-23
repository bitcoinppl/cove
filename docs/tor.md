# Tor

Cove can send its network traffic through Tor so the servers it talks to never see the user's IP address, and so it can reach `.onion` nodes. This note covers the modes, the routing invariant that keeps traffic from leaving Tor, what traffic is covered, and how the pieces fit together.

## Modes

The user picks one mode in Settings → Tor. `TorSettings` (`rust/src/tor/settings.rs`) is the saved value and `TorMode` is its UniFFI shape.

- **Off**: connections go directly to their servers.
- **Built-in**: Cove runs [Arti](https://gitlab.torproject.org/tpo/core/arti), the Rust Tor client, in process. Arti only starts in this mode, so Off and Proxy pay no startup or memory cost for it.
- **Proxy**: Cove sends traffic to a Tor SOCKS5 proxy the user runs, such as Orbot. The address defaults to `127.0.0.1:9050`. Cove doesn't detect Orbot or talk to it beyond SOCKS.

## The routing invariant

`TorService` (`rust/src/tor.rs`) is the process-wide owner. It loads the saved mode at bootstrap, runs the built-in client when that mode is on, and publishes the `NetworkRoute` (`rust/crates/cove-http/src/route.rs`) that every connection uses:

- `NetworkRoute::Direct`, only while the saved mode is Off
- `NetworkRoute::Tor(SocksProxy)`, the SOCKS5 proxy to use

Every client builder asks for the route when it builds a client, through `tor::current_route()` for fixed hosts or `Node::route()` / `tor::route_for_host()` for node hosts. While Tor is saved as on, route resolution never returns `Direct`. When Tor can't carry traffic, resolution fails with `RouteError::TorUnavailable` and the connection never starts:

- the built-in client failed to start
- the saved Tor setting can't be read, which blocks every connection instead of falling back to Off
- a full wipe is in progress (`TorService::stop_for_wipe` blocks connections until `restart` reads the wiped database)

A `.onion` host on a direct route fails with `RouteError::OnionNeedsTor`, so the system resolver never sees an onion name. Turning Tor off is refused while any network's selected node is a `.onion` node.

Host names are resolved by the proxy, never on the device: reqwest and Esplora use `socks5h://` URLs, and electrum-client sends the host name in the SOCKS request. TLS runs inside the SOCKS stream, so certificate checks still use the real host name.

### Enforcement

- `rust/clippy.toml` bans the raw constructors (`reqwest::Client::new`/`builder`, `electrum_client::Client::new`/`from_config`, `esplora_client::Builder::build_async`, and `TcpStream::connect` in std and tokio). The one builder per client type that applies the route carries an `#[allow]`, so a new client that skips the route fails `just clippy`.
- `rust/src/tor/tests.rs` drives the app's real node clients against the fake node, with a recording SOCKS5 proxy (`rust/src/test_support/socks_proxy.rs`) standing in for Tor. The tests check that node traffic reaches the proxy with names unresolved, that a down proxy fails connections instead of going direct, that unreadable settings block connections, and that onion nodes need Tor. `rust/src/fee_client.rs` has the same checks for fee requests.

### Switching modes

Changing the mode stops the old built-in client before publishing the new route, so no stream outlives its mode. Cached clients notice the change because the route is part of `NodeConnectionIdentity` (`rust/src/node.rs`): the wallet actor and the transaction watcher drop a client whose identity no longer matches and build one on the new route. The fee and price clients cache one reqwest client per route through `cove_http::RoutedClient`.

## What goes through Tor

While Tor is on, these go through it:

- Electrum and Esplora node connections, including node checks in Settings
- fee estimates
- current and historical fiat prices
- Payjoin (OHTTP relay requests and the fallback broadcast)
- diagnostics report uploads
- the Tor check in Settings → Tor, which asks `check.torproject.org` whether traffic leaves through a Tor exit

These stay on the regular connection, and the Tor screen says so:

- **iCloud backups** (iOS) and **Google Drive backups** (Android). The OS runs iCloud, and Drive's sign-in runs in Google Play services, so Cove can't route either through Tor.
- **Passkeys**, which the OS handles.
- **Links opened in the browser**, such as block explorer links.

Nodes on the local network don't work while Tor is on: a Tor exit can't reach a private address, and Arti refuses to connect to one. Use the node's `.onion` address instead, or turn Tor off.

## Built-in Tor

The `cove-tor` crate (`rust/crates/cove-tor`) wraps Arti. `BuiltInTor::start` creates an unbootstrapped `TorClient` with on-demand bootstrap, then starts a SOCKS5 server on `127.0.0.1` at a random port that requires a random username and password, so other apps on the device can't use it. Cove's clients reach the built-in client through that server, so built-in and proxy modes share one client code path.

- Each target host gets its own `IsolationToken`, so streams to different servers never share a circuit.
- Bootstrap runs in the background and retries after 5, 15, 30, then 60 seconds. `TorStatus` reports `Starting { percent }`, `Ready`, or `Failed { message }`; connections made before bootstrap finishes wait for it.
- Arti keeps its state and directory cache in `ROOT_DATA_DIR/tor/{state,cache}` (about 10 MB of cache), and a full wipe removes that directory.

## Settings screen

`RustTorManager` (`rust/src/manager/tor_manager.rs`) is a process singleton. `state()` is the initial snapshot. User intents go through `dispatch(.selectMode(mode))` and `dispatch(.restart)`, and saving runs off the caller's thread. Reconcile messages carry full values (`Mode`, `Status`, and `ModeRejected` for a refused change), so a reconciler that applies them in order converges on the current state without a revision counter.

The saved mode is the `tor_settings` key in `GlobalConfigTable`, stored as JSON with explicit names, such as `{"mode":"proxy","address":"127.0.0.1:9050"}`. The table is a string table, so its redb metadata doesn't change.
