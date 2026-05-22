# Cloud Backup

Cloud Backup is split between a UI-facing manager, a reducer-backed state
projection, and a set of actors that own long-running work. The Rust core is
the source of truth for lifecycle state, passkey checks, cloud writes, and local
database updates. Swift and Kotlin should consume `CloudBackupState` and send
actions through the manager instead of rebuilding cloud backup rules in UI code.

## Ownership Model

`RustCloudBackupManager` is the FFI facade. It owns the reducer, exposes
snapshots and reconcile messages, and forwards long-running work to
`CloudBackupSupervisor`.

`CloudBackupStateReducer` keeps the private lifecycle model and projects the
public UniFFI state. It also filters stale exclusive-operation completions so an
older async path cannot erase a newer operation.

`CloudBackupSupervisor` is the top-level operation owner. It starts enable,
restore, disable, repair, verification, sync, and cloud-only flows, then applies
typed completions back through the manager. Exclusive operations carry a
`CloudBackupExclusiveOperationClaim` so every async completion can prove it
still belongs to the active operation.

The child actors under `cloud_backup_manager::actors` own narrower work:

- `restore` owns restore progress and restore event delivery
- `uploads` owns dirty wallet upload scheduling and pending-upload verification
- `sync_health` owns background sync health refreshes
- `cleanup` owns namespace cleanup after merge or recovery flows
- `write` owns serialized cloud writes and post-write local completion

## Write Lane

All cloud writes go through `CloudBackupWriteSupervisor`. The write supervisor
queues one remote write at a time, rejects writes while disable is active, and
checks persisted disabling state so restart recovery keeps the write fence.

Operation-owned writes use `CloudBackupWriteClient::for_operation`. Their
command context includes the active operation claim, which is checked before the
write starts and again before local completion is applied. Background writes use
`CloudBackupWriteClient::new` and are not tied to an exclusive operation.

`CloudBackupWriteWorker` is intentionally small. It executes the remote storage
command and reports the result back to the write supervisor. The supervisor then
applies any local completion, such as marking uploaded blobs pending
confirmation or persisting the enabled state after final upload.

Disable uses `block_until_drained` before deleting the namespace. That prevents
an already-started upload from recreating remote data after disable removes the
backup namespace.

## Verification And Recovery

Pending upload verification confirms that blobs marked
`UploadedPendingConfirmation` are visible in cloud storage with the expected
revision. Authorization failures pause the verifier without rewriting the blob
state, so the app can resume after the platform cloud authorization is restored.

Deep verification checks the cloud master-key wrapper, local keychain state,
wallet blobs, and pending uploads. Some verification results prepare follow-up
work, such as repairing the passkey wrapper or uploading missing wallet blobs.
The supervisor owns those continuations so each follow-up remains tied to the
same exclusive-operation claim.

## Related Notes

- Read `docs/passkeys.md` before changing passkey registration, targeted auth,
  presence checks, or passkey repair
- Read `docs/icloud_drive.md` before changing iCloud discovery, metadata query,
  or coordinated file access
- Read `docs/redb.md` before changing persisted cloud backup state, table
  definitions, redb value type names, or persisted type module paths
