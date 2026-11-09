# Cloud Backup written by v1.3.0

What the v1.3.0 code uploaded during a fresh enable, for `ops/tests/golden_v1_3_0_restore.rs` to restore on the current code.

- `cloud/<namespace>/` holds every file the cloud received, at the relative path v1.3.0 asked the provider to write it (`master-key/…` and `wallets/…`), byte for byte
- `manifest.json` names the namespace and each file's record id. It also has the passkey side of the enable: the credential id and PRF output the scripted passkey returned, the PRF salt in the uploaded master key wrapper, and every passkey request v1.3.0 made with its relying party id and salt. And it lists the wallets put on the device before enabling, with what a restore must bring back
- `generator.patch` is the change to the `v1.3.0` tree that produced it: an ignored test, `ops/tests/golden_v1_3_0.rs`, plus test-only recording in v1.3.0's `ops/test_support.rs` mock cloud and mock passkey. It changes no production code

The wallets are a hot wallet with BIP329 transaction, address and output labels, an xpub watch-only wallet with a label, a hardware wallet kept as public descriptors, a TAPSIGNER wallet with a fake card backup, and a testnet hot wallet in decoy mode. All keys come from public BIP39 test mnemonics, and the TAPSIGNER backup, credential id and PRF output are fixed fake bytes. The master key, salts and nonces are random, so each run gives different bytes.

## Regenerating

From the repository root:

```sh
git worktree add ../cove-v1.3.0 v1.3.0
cd ../cove-v1.3.0
git apply <this checkout>/rust/src/manager/cloud_backup_manager/ops/tests/fixtures/v1_3_0_cloud_backup/generator.patch
cd rust
mkdir -p ../golden-out
COVE_GOLDEN_OUT="$PWD/../golden-out" cargo test --locked -p cove --lib golden_v1_3_0 -- --ignored
```

Then replace `cloud/` and `manifest.json` here with the ones in `golden-out`, keep `generator.patch` in step with the generator, and remove the worktree. The generator points the data directory at a temporary one before anything opens it, so the wallet and label databases it writes for its fixed wallet ids go away with it and never reach the real data directory.
