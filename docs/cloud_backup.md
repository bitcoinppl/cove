# Cloud Backup

Cloud Backup is split between a UI-facing manager, a reducer-backed state
projection, and a set of actors that own long-running work. The Rust core is
the source of truth for lifecycle state, inventory completeness and generations,
passkey policy, cloud writes, local database updates, and Restore All. Swift and
Kotlin consume `CloudBackupState` and send actions through the manager instead
of rebuilding cloud backup rules in UI code.

## Ownership Model

`RustCloudBackupManager` is the FFI facade. It owns the reducer, exposes
snapshots and reconcile messages, and forwards long-running work to
`CloudBackupSupervisor`.

`CloudBackupStateReducer` keeps the private lifecycle model and projects the
public UniFFI state. It also filters stale exclusive-operation completions so an
older async path cannot erase a newer operation.

`PendingEnableCoordinator` owns the local keychain transaction for fresh and
recovered enables. It keeps CSPP staging and promotion ordered with the durable
pending-enable journal while the manager and supervisor retain cloud writes,
operation claims, and UI reconciliation.

`CloudBackupSupervisor` is the top-level operation owner. It starts enable,
restore, disable, repair, verification, sync, and cloud-only flows, then applies
typed completions back through the manager. Exclusive operations carry a
`CloudBackupExclusiveOperationClaim` so every async completion can prove it
still belongs to the active operation.

Swift and Kotlin own provider mechanics and native presentation classification.
They return typed cloud-storage and passkey outcomes to Rust; they do not decide
whether inventory is complete, whether an enable path is safe, or which wallets
belong in a batch. Android's generated UniFFI manager handle stays private to
`CloudBackupManager` and is accessed through its guarded wrapper methods.

The child actors under `cloud_backup_manager::actors` own narrower work:

- `restore` owns restore progress and restore event delivery
- `uploads` owns dirty wallet upload scheduling and pending-upload verification
- `CloudBackupSyncHealthWorker` owns background sync health refreshes
- `cleanup` owns namespace cleanup after merge or recovery flows
- `write` owns serialized cloud writes and post-write local completion

## Inventory

The detail inventory is explicitly progressive:

- `NotLoaded` has no inventory attempt yet
- `Checking` may retain the last known detail and wallet rows while a newer
  generation is in flight
- `Complete` is the only state that proves the current provider inventory is
  complete
- `Failed` carries a typed incomplete reason and may retain the last known rows

A timeout, authorization failure, offline result, or provider failure is
incomplete, not a confirmed empty inventory. Restore and destructive actions
that depend on absence or a complete set stay disabled until the state is
`Complete`.

The supervisor embeds a `DetailWorkflow` that owns detail entry planning,
runtime passkey authorization, pending verification completion, and refresh
scheduling. It permits one screen refresh in flight and coalesces further
requests into one trailing refresh. Rust starts refresh generations no more than
once every five seconds and admits results by start order, so an older screen or
operation refresh cannot overwrite a newer one. Closing the detail screen
invalidates its refresh owner without cancelling manager-owned operations.

iOS can publish a fast, incomplete local wallet snapshot before completing its
mandatory FileManager-plus-`NSMetadataQuery` union. Android's first successful
Google Drive list is complete because Drive returns provider inventory directly.
The platform result feeds the same Rust reducer on both platforms.

## Passkeys And Enable

Cloud access has an explicit presentation boundary. Silent startup discovery
must not present provider consent. User-initiated restore or enable work may use
consent-allowed access; on Android, this is the boundary at which Google Drive
authorization may appear.

Enable behavior depends on what discovery proved:

- a visible existing backup uses its existing passkey and namespace
- visible namespace metadata whose recovery files are still loading is treated
  as pending, and only the existing-passkey path is offered
- an explicit Create New Backup path stages a fresh master key, new namespace,
  and passkey material without replacing the current active material

Fresh enable material is tracked by a durable pending-enable journal through
passkey registration, remote writes, and local promotion. A newly saved passkey
is targeted and confirmed before upload continues. Accepted provider writes are
recorded for visibility confirmation so a restart or reconnect can resume the
confirmation work without leaving the lifecycle stranded in `Enabling`.
Promotion happens only for journal-owned staged material. Failure or
cancellation restores the prior local metadata and only cleans remote material
that is proven to be incomplete and owned by that fresh attempt.

Automatic passkey retry is shared and bounded. It applies only to a typed
platform-authorization failure that occurred before native presentation.
Presented failures, user cancellation, credential mismatch, unsupported
providers, and invalid results are returned to the owning flow without
automatic retry or registration fallback. Only a typed no-credential result
may continue into the owning enable or repair flow's explicit registration
step. Interactive passkey and Drive sheets are not wrapped in a global
watchdog.

## Restore All

Restore All is not a second restore implementation. Individual restore and
Restore All both use `CloudBackupPreparedWalletRestore`, so wallet persistence,
duplicate handling, labels warnings, and the final outcome match Restore to This
Device. A batch prepares one identity-aware reader/session and restores records
sequentially.

Rust derives eligibility from active-namespace rows whose status is
`DeletedFromDevice`. Initial Restore All is shown for at least two eligible
wallets; Retry Remaining can be shown for one or more. Availability additionally
requires complete inventory and no conflicting exclusive operation. At start,
Rust freezes the visible order, refetches authoritative active-namespace
inventory, and intersects the two sets. Newly appeared, stale, unsupported, or
already-local rows are not added to the run.

The manager owns inline progress, the current wallet name, cancellation, and row
outcomes. Each success is reconciled immediately so the wallet moves to its
network section. An ordinary wallet-local failure stays on its row and the batch
continues; retrying that row or Retry Remaining clears its prior error. A
provider, authorization, or offline failure stops before scheduling the next
wallet and leaves Retry Remaining available after inventory is complete.

Cancellation is cooperative between atomic wallet restores. Leaving the detail
screen does not cancel the batch. A namespace-only marker records that a batch
may have been interrupted; after process death, startup performs a refresh and
offers Retry Remaining for the authoritative eligible set without automatically
restoring wallets or presenting passkey UI. Completion and clean cancellation
clear the marker. There is no Restore All confirmation, terminal result screen,
or persistent per-wallet batch history.

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
