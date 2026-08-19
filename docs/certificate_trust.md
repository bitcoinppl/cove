# Certificate Trust

Cove can remember a pinned fingerprint or a custom CA for a custom SSL Electrum endpoint. This data controls a security decision. Rust owns the decision and its durable state.

## Invariants

- The certificate identity is the canonical `ssl://host:port` endpoint. Host case, credentials, paths, queries, fragments, and a trailing slash do not create a new identity. The default port is `50002`.
- Each endpoint has zero or one valid trust value. A value must produce a usable TLS client configuration before Cove stores it.
- Trust for one endpoint must not apply to another endpoint.
- An endpoint with two different valid trust claims is conflicted. Cove must fail closed for that endpoint.
- Invalid durable storage is not a missing trust value. Cove must report the storage error for a custom SSL node that has no valid embedded pin.
- A trusted certificate must not change without a new explicit user decision. The normal certificate prompt does not replace existing trust.
- Trust state, selected-node state, and the shared read snapshot must describe the same committed transaction.
- An asynchronous platform result is valid only for the exact node request that started it.

## State Model

`GlobalConfigTable` is the owner of certificate trust. It owns these parts:

- `CertificateTrustStore`: the durable map from a canonical endpoint to `TlsTrust`
- selected-node records: old records can contain an embedded `Node.tls` trust value
- `CertificateTrustSnapshot`: the effective read state from the durable map and all embedded legacy pins
- `certificate_trust_writer`: the shared lock for all trust-affecting writes
- `CertificateTrustCache`: the shared snapshot that existing `NodeSelector` values read

The effective state has three forms:

| State | Meaning | Required behavior |
| --- | --- | --- |
| Valid trust | All claims for an endpoint have the same usable value | Hydrate an unpinned SSL Electrum node with that value |
| Endpoint-conflicted trust | Two or more valid claims for one endpoint differ | Fail closed only for that endpoint. Keep unrelated endpoint reads available |
| Invalid storage | The durable map cannot be read or validated | Keep the error in the shared snapshot and report it when the flow needs the store |

Fail closed means that Cove does not select one conflicting value and does not hydrate an unpinned node. A valid pin that is already embedded in a selected node stays attached to that node and can enforce that exact value. If a stored selected SSL node has no embedded pin and hydration fails, Cove uses the safe default node. Non-TLS nodes do not need certificate trust state.

`NodeSelector` reads the shared snapshot. It does not own migration or recovery. Android and iOS present decisions from Rust and do not change durable trust directly.

## Persistence and Migration

The durable map is JSON in `GlobalConfigKey::CertificateTrustStore`. Selected-node records can also contain old embedded pins. On start and after each trust-affecting write, Cove builds the effective snapshot from both sources for all networks.

When Cove writes a selected node, it performs these steps in one redb transaction:

1. Read the current durable map and the previous selected node.
2. Add the previous embedded pin to the candidate map.
3. Add the incoming embedded pin to the candidate map.
4. Hydrate an unpinned incoming SSL Electrum node from the candidate map when trust exists.
5. Write the map and the selected node.
6. Build and validate the effective snapshot again.
7. Commit, then publish the snapshot.

This sequence moves a legacy pin to the shared map before Cove replaces the old selected-node record. A conflict or validation error rolls back the node and trust writes.

Do not change the redb table definition, stored key, JSON shape, or type metadata without a separate compatibility plan. Also follow [redb.md](redb.md).

## Backup and Restore

Backups contain the durable certificate trust map and the selected-node records. Old backups without the trust-map field use an empty map.

Restore merges the incoming map before it restores selected nodes:

- A new endpoint is added.
- A matching value is a no-op.
- A different value for an existing endpoint does not replace the existing value. Restore reports the endpoint as a conflict and continues with other valid entries.
- An invalid incoming map is reported as a settings restore error and does not prevent wallet restoration.
- A conflict that is already in the effective local state causes the full trust-map write to roll back.
- A restored selected-node record goes through the normal selected-node write. This migrates any embedded legacy pin and applies the same conflict rules.

Export reads the raw durable value. Missing data becomes an empty map. Valid data keeps the existing map shape. Invalid local trust is preserved in the backup with a warning, so it does not block wallet export or silently disappear. A database read failure remains an export error.

## Recovery

Automatic recovery is allowed only for a typed endpoint conflict and only when the user selects an unpinned replacement node. Recovery must use current durable state, not only the snapshot that detected the conflict.

In one transaction, recovery must:

1. Confirm that the endpoint is still conflicted.
2. Confirm that the old selected node has an embedded trust value for that endpoint.
3. Remove a durable entry only when it is equal to the old embedded claim.
4. Write the unpinned replacement node.
5. Rebuild the effective snapshot and confirm that no conflict remains.
6. Commit, then publish the snapshot.

Recovery rolls back when the conflict is stale, storage is invalid, the old claim does not match, or any effective conflict remains. It must not remove a newer or different durable trust value.

## Concurrency

All `GlobalConfigTable` clones share one certificate-trust writer lock. The lock serializes selected-node writes, trust-map restore, recovery, and generic writes or deletes that affect selected nodes or the trust map.

The write transaction commits before Cove replaces the shared snapshot. The writer lock stays held through snapshot publication. This order prevents an older writer from publishing its snapshot after a newer commit. A failed transaction does not change the shared snapshot.

Readers use the shared `RwLock` snapshot. Existing `NodeSelector` values must see trust that a later successful writer commits. Send update messages only after the writer lock is released.

## Certificate Request Identity

Rust normalizes the endpoint and checks existing trust before it reads a server certificate. It checks trust again after the network read. If trust appears during the read, Rust returns `Changed` and does not offer the new certificate.

Both apps bind an asynchronous result to a request that contains the selected option and the parsed `Node`. They discard connection errors, certificate decisions, and certificate alerts when the current form no longer describes that request. They check the request again when the user accepts the certificate.

iOS can use value equality for `Node` and `TlsTrust`. Android must compare certificate byte arrays by content. Its request identity therefore compares the node name, network, API type, URL, and TLS value explicitly.

A certificate accepted in the current screen session becomes one typed `EndpointCertificateTrust` value with the normalized endpoint. Both apps pass that value back to `parseCustomNode`. Rust normalizes the current raw URL and applies the session trust only when both canonical endpoints match. A different endpoint, TCP node, or Esplora node does not receive the trust. Acceptance retries the same request with the accepted pin. Durable trust is written only after the connection check succeeds.

## Migration and Test Matrix

| Starting data or event | Expected result | Required test evidence |
| --- | --- | --- |
| Fresh install with no trust data | Empty valid snapshot | Unpinned nodes stay unpinned. Non-TLS nodes work |
| One legacy selected-node pin | Pin is usable at start and moves to the durable map before node replacement | Reopen or rewrite through the production table API and hydrate the old endpoint |
| Same endpoint and same trust in more than one source | One valid effective value | Equivalent claims do not conflict |
| Same endpoint and different trust in two sources | Endpoint-conflicted state | The endpoint fails closed and unrelated trust remains usable |
| Malformed map, invalid endpoint, invalid fingerprint, or unusable CA | Invalid-storage state | An unpinned custom SSL node reports the error. Export preserves the raw value with a warning. An embedded valid pin and a non-TLS node keep their defined behavior |
| Selected-node write fails | Durable data and shared snapshot stay unchanged | Verify transaction rollback and an existing selector read |
| Recovery removes one old claim | Replacement commits only if the remaining effective state is valid | Cover success, stale state, mismatched durable value, remaining conflict, and invalid storage |
| Backup has no trust-map field | Empty incoming map | Old backup deserializes and restores |
| Backup has new, matching, and conflicting entries | Add new, ignore matching, keep existing on conflict, and report conflict | Verify the committed map and restore report |
| Local trust is invalid during export | Wallet data exports and raw trust is retained as invalid | Verify the warning, raw round trip, and later settings-only import error |
| Two trust-affecting writers overlap | Commit order and snapshot publication stay consistent | Use a controlled interleaving or lock-ownership test. Verify that a failed writer does not publish |
| User edits a node while an async check or prompt is active | Stale result is discarded | Cover URL, node type, selected option, name, and TLS identity changes on Android and iOS |
| Raw and normalized URLs name the accepted endpoint | Rust applies the same session trust | Cover scheme inference, case, path, query, default port, another endpoint, TCP, and Esplora |

When a new state or migration path is added, update this matrix before you change callers. Put the invariant in Rust unless it is only about platform request identity or presentation.
