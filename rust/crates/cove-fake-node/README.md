# cove-fake-node

A fake Esplora and Electrum node backed by one deterministic in-memory chain. Both protocols answer from the same chain, so a change made once shows up on both.

It runs two ways:

- **In process**, for Rust tests: `FakeNode::start(network)` binds free loopback ports. Tests mutate the chain through `FakeNode::chain()`, read the request log, and script requests to fail, stall, run slowly or wait on a gate (`FakeNode::on`). The app crate takes this library as a dev-dependency only, and `rust/src/test_support/fake_node.rs` adds the helpers that point Cove's node settings at it
- **As the `cove-fake-node` binary**, for iOS and Android UI tests: it serves the node on fixed ports with a JSON control API, so a test running in a simulator or emulator can fund a wallet, mine blocks and check what the app broadcast

## Running the binary

```sh
cd rust && cargo run -p cove-fake-node
```

| Flag | Default | |
| --- | --- | --- |
| `--network` | `signet` | `bitcoin`, `testnet`, `testnet4`, `signet` or `regtest`; the chain starts from that network's genesis block |
| `--bind` | `127.0.0.1` | Address every server listens on |
| `--control-port` | `39000` | JSON control API |
| `--esplora-port` | `39001` | Esplora HTTP API |
| `--electrum-port` | `39002` | Electrum TCP (no TLS) |

Port `0` picks a free port. Once every port listens, the binary prints one JSON line with the URLs and serves until SIGINT or SIGTERM:

```json
{"control":"http://127.0.0.1:39000","electrum":"tcp://127.0.0.1:39002","esplora":"http://127.0.0.1:39001","network":"signet"}
```

It exits with an error without printing that line when a port is taken. The chain starts as it is after `POST /reset`.

To run a command with the node up, use the xtask wrapper. It builds and starts the binary on the fixed ports, checks `/health`, runs the command from the current directory, then stops the node:

```sh
just fake-node-run <command>                     # runs from rust/
cd rust && cargo xtask fake-node run -- <command>
cargo xtask fake-node run --android -- <command> # fail when no Android device is connected
```

Without `--android`, the wrapper forwards the three ports with `adb reverse` to the connected Android device, if any, and removes the forwards afterwards. Every test resets and funds the one node, so the wrapper refuses to run with more than one device connected unless `ANDROID_SERIAL` picks one; Gradle honors `ANDROID_SERIAL` too. Interrupting the run with Ctrl-C skips that cleanup, so remove leftover forwards with `adb reverse --remove-all`. `xtask ios-ui` (behind `just ios-ui-background` and `just ios-ui-foreground`), `xtask android-ui-manual` and `just android-instrumented-ci` run their tests with the node up. The iOS simulator shares the host's loopback, so the app reaches the node at `http://127.0.0.1:39001` on both platforms.

Point the app at the node with a custom Esplora node at `http://127.0.0.1:39001`.

The Esplora port also answers mempool.space's `GET /api/v1/fees/recommended` with fixed fee rates. Debug builds of the app fetch fees from `COVE_FEE_URL` when it is set, so pointing it at `http://127.0.0.1:39001/api/v1/fees/recommended` keeps sends off the internet.

## Control API

Every response is JSON. Errors are `{"error":"…"}` with status 400 for bad input and 404 for an unknown route or output. Unknown request fields are rejected, and an empty body is treated as `{}`.

### `GET /health`

```json
{"ok":true,"esplora":"http://127.0.0.1:39001","electrum":"tcp://127.0.0.1:39002","network":"signet","tip":10}
```

### `POST /reset`

Replaces the chain with genesis plus 10 empty blocks (`RESET_BLOCKS`) and clears the broadcast log, the request log and any scripted rules. Returns `{"ok":true,"tip":10}`.

### `POST /fund`

```json
{"address":"tb1q…","sats":[100000,60000],"confirm":true}
```

Pays each amount to the address from its own funding transaction, so each amount is its own UTXO at output 0. `confirm` defaults to `true` and mines one block that confirms these transactions; with `false` they stay in the mempool. The address must belong to the node's network.

```json
{"outputs":[{"txid":"…","vout":0,"sats":100000,"address":"tb1q…"},{"txid":"…","vout":0,"sats":60000,"address":"tb1q…"}]}
```

### `POST /fund-wallet`

```json
{"mnemonic":"abandon … about","account_path":"m/84'/1'/0'","outputs":[{"index":0,"sats":100000}],"confirm":true}
```

Derives the BIP84 native segwit receive address `account_path/0/index` of the mnemonic (no passphrase) on the node's network and funds it as `/fund` does. `account_path` defaults to `m/84'/0'/0'` on mainnet and `m/84'/1'/0'` elsewhere. Returns the same shape as `/fund`. For the abandon ×11 about mnemonic on Signet, index 0 is `tb1q6rz28mcfaxtmd6v789l9rrlrusdprr9pqcpvkl`.

### `POST /mine`

`{"blocks":1}` (the default) mines that many blocks. The first one confirms every transaction in the mempool. Returns `{"tip":<height>}`.

### `GET /broadcasts`

Transactions the app sent through Esplora `POST /tx` or Electrum `blockchain.transaction.broadcast`, oldest first. A rebroadcast of a transaction already in the mempool is not logged again.

```json
[{"txid":"…","inputs":["<txid>:<vout>"],"outputs":[{"address":"tb1q…","sats":99000}]}]
```

`address` is `null` for an output script with no address form.

### `GET /outspend/<txid>/<vout>`

```json
{"spent":true,"spending_txid":"…"}
```

`spending_txid` is `null` when the output is unspent. The spend counts whether it is pending or confirmed. An output the chain does not know is a 404.
