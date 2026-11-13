# cove-fake-node

A fake Esplora and Electrum node backed by one deterministic in-memory chain. Both protocols answer from the same chain, so a change made once shows up on both.

It runs two ways:

- **In process**, for Rust tests: `FakeNode::start(network)` binds free loopback ports. Tests mutate the chain through `FakeNode::chain()`, read the request log, and script requests to fail, stall, run slowly or wait on a gate (`FakeNode::on`). The app crate takes this library as a dev-dependency only, and `rust/src/test_support/fake_node.rs` adds the helpers that point Cove's node settings at it
- **As the `cove-fake-node` binary**, for iOS and Android UI tests: it serves the node on the ports it is given with a JSON control API, so a test running in a simulator or emulator can fund a wallet, mine blocks and check what the app broadcast

## Broadcasts

Esplora `POST /tx` and Electrum `blockchain.transaction.broadcast` check a new transaction before accepting it, the way consensus would:

- Every input must spend an output the chain knows, whether confirmed or pending, that no confirmed transaction has spent. Otherwise the reply is `bad-txns-inputs-missingorspent`.
- A transaction whose outputs add up to more than its inputs is refused with `bad-txns-in-belowout`.
- A transaction that spends the same output as pending ones replaces them, along with their pending descendants, the way Bitcoin Core 28 and later do with full RBF: the replaced transactions need not signal BIP 125, but the replacement must pay for them. The checks run in Core's order. The replacement may not spend an output of a transaction it evicts. Its fee rate, in whole sat/kvB of Core's virtual size, the larger of its weight and 20 times its signature operation cost, divided by 4 and rounded up, must be above that of each transaction it double-spends. It may evict at most 100 transactions, counting each one it double-spends with its descendants. It may not spend a pending output unless an evicted transaction, descendants included, spent from the same parent. Its fee must be at least that of everything it evicts, and the difference must pay 1 sat/vB of that size, Core's incremental relay fee before 29.1. Otherwise the reply starts with Core's `bad-txns-spends-conflicting-tx`, `insufficient fee`, `too many potential replacements` or `replacement-adds-unconfirmed`, and the mempool is left as it was. A double spend of a transaction added with `add_to_mempool` whose inputs the chain doesn't know is refused, since its fee is unknown.
- Every input's scripts and signatures must satisfy the output it spends, checked by libbitcoinconsensus from Bitcoin Core 26 with taproot enabled. Otherwise the reply is `mandatory-script-verify-flag-failed (input N)`.
- A transaction that is already confirmed is refused with `Transaction already in block chain`. A rebroadcast of one in the mempool is accepted again.

Esplora replies with status 400 and the message, and Electrum replies with a JSON-RPC error. Other mempool policy is not checked: the fee of a transaction that replaces nothing, dust, standardness and coinbase maturity all pass. Funding transactions from `pay`, `/fund` and `/fund-wallet` spend synthetic `OP_1` outputs and skip the check, since they never go through a broadcast.

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

To run a command with the node up, use the xtask wrapper. It builds and starts the binary on the first free block of ports from 39100 in steps of 100, reserved with a lock until the node stops, checks `/health`, runs the command from the current directory with `COVE_UI_TEST_PORT_BASE` set to the block's control port, then stops the node:

```sh
just fake-node-run <command>                     # runs from rust/
cd rust && cargo xtask fake-node run -- <command>
cargo xtask fake-node run --android -- <command> # fail when no emulator is connected
```

Without `--android`, the wrapper forwards the three ports with `adb reverse` to the connected emulator, if any, and removes the forwards afterwards. It only uses emulators, the one `ANDROID_SERIAL` names or else the only one connected, and never a physical device. The device always sees 39000-39002, whichever block the run took, and the run locks the device so a second run can't replace its forwards. No run takes 39000-39002 on the host, since revisions before port blocks use it. Every test resets and funds the one node, so the wrapper refuses to run with more than one emulator connected unless `ANDROID_SERIAL` picks one. The command runs with `ANDROID_SERIAL` set to the emulator, which Gradle honors. Interrupting or terminating the run with SIGINT, SIGTERM or SIGHUP still stops the command and the node and removes the forwards. `xtask ios-ui` (behind `just ios-ui-background` and `just ios-ui-foreground`), `xtask android-ui-manual` and `just android-instrumented-ci` run their tests with the node up. On Android the app reaches the node at `http://127.0.0.1:39001`. The iOS simulator shares the host's loopback, so the iOS tests read the block from `COVE_UI_TEST_PORT_BASE` and reach Esplora at base + 1, such as 39101.

Point the app at the node with a custom Esplora node at its Esplora port: `http://127.0.0.1:39001` on Android, or base + 1 on the host, such as `http://127.0.0.1:39101` for a lone run. Started by hand with its defaults, the binary's Esplora port is 39001.

The Esplora port also answers mempool.space's `GET /api/v1/fees/recommended` with fixed fee rates. Debug builds of the app fetch fees from `COVE_FEE_URL` when it is set, so pointing it at `/api/v1/fees/recommended` on the Esplora port, such as `http://127.0.0.1:39001/api/v1/fees/recommended` on Android, keeps sends off the internet.

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
