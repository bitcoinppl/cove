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

The supervisor is also the only writer of the durable lifecycle record: it holds
the one `LifecycleWriter`, which bootstrap moves into it. Work that runs off the
actor, such as the write lane, upload confirmation, wallet changes and the
integrity check, sends the supervisor a typed result message, and the supervisor
applies the write and reconciles. Verification evidence is tied to the wallet set
it read, so a wallet change that lands while a verification runs makes that
verification's result stale. A stale result neither overwrites the reset nor
shows on screen.

Waits go one way only. The write supervisor and other workers may `call!` the
supervisor, but the supervisor never waits on the write supervisor: it sends a
message and handles the reply as a later completion, so the two actors can't
deadlock waiting on each other.

Workers prepare keychain writes and the supervisor applies them. Each write
carries its producer's authority: the running operation's claim, or the
configuration generation that background work read. The supervisor applies it
only while that authority is current, so a write that arrives after a reset, a
Turn Off, a new configuration or a reinitialize is refused instead of restoring
wiped or older keys. An effect that can't be cancelled once it starts, such as Turn Off's local
cleanup or a passkey registration, puts its operation in an `Effecting` stage,
and the operation keeps the slot until the effect reports. A local reset, which
a full wipe uses, closes admission, cancels running operations, waits for any
in-flight effect to report, and then has the supervisor clear the Cloud Backup
keychain namespace. Catastrophic recovery clears the namespace only while no
Cloud Backup manager exists, so the namespace has one owner at a time.

A finished reinitialize, including one at the end of a Google Drive account
switch, hands its slot straight to a verification of the new backup under a new
claim. That verification reports the backup as enabled, so it reads the cloud,
while the reinitialize stays on screen until it ends.

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

### Who Writes What

Each fact has one writer. Other code sends that writer a typed result and never
writes the fact itself.

| Fact | Single writer | Typed inputs from producers |
|---|---|---|
| Durable lifecycle record (redb `PersistedCloudBackupState`): configuration, verification, sync fields, the Restore All marker, the pending upload confirmation, the account-switch phase and the committed disable | The supervisor's `OwnedRuntime`, which holds the only `LifecycleWriter` and reaches it only through `commit`, which publishes the write policy after every write | Verification results, upload confirmations (carrying the wallet-set generation they read), write-lane completions, wallet changes, the integrity check, restore activation, the account switch |
| Runtime operation (the body and the operation nested in it) | The supervisor's `OwnedRuntime` | Worker completions carrying the operation's claim |
| Write policy | Nobody: `Runtime::write_policy` derives it, and `OwnedRuntime` publishes it, versioned, to the write lane | — |
| Cloud Backup keychain namespace | The supervisor, which applies the keychain writes workers prepare | `OwnedKeychainWrite` results |
| Pending Enable journal (keychain) | `PendingEnableCoordinator` | The Enable and recovery operations |
| Detail inventory | The reducer | Detail refresh results |
| Passkey health, sync health | Their workers | Read-only: they never write the lifecycle record or the runtime |

Ownership is enforced in types, not by convention. `LifecycleWriter` needs a
`LifecycleCapability`, whose private field only `owner/lifecycle.rs` can
construct. Bootstrap creates it once, inside the exclusive bootstrap phase that
normalizes the stored record, and moves it into the supervisor. The database has
no other production route that writes the Cloud Backup tables.

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

A local wallet change doesn't wait for that refresh to correct the local rows.
The reducer drops rows for wallets that are no longer on this device at once,
and counts a deleted wallet's confirmed backup as cloud-only, so a deleted
wallet shows only under Not on This Device once the cloud-only rows are read
again, even when the refresh fails. Everything else waits for the refresh.
A detail result that read the local wallets before the deletion, from a refresh
or a verification, gets the same check against the current local wallets when
it lands, so it can't list the wallet as local again.

iOS can publish a fast, incomplete local wallet snapshot before completing its
mandatory FileManager-plus-`NSMetadataQuery` union. Android's first successful
Google Drive list is complete because Drive returns provider inventory directly.
The platform result feeds the same Rust reducer on both platforms.

Each native detail screen projects one progress presentation from verification
state, inventory state, retained detail, and visible wallet rows. Verification
progress takes priority over inventory progress. It uses a standalone card when
no wallet rows are visible and an inline row when local or loaded cloud-only
wallet rows stay on screen. `cloudOnlyCount` is not a visible row by itself.
The verification section does not render a second running indicator.

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

When an interrupted Enable leaves that evidence behind, the supervisor holds it
as its own runtime body, `PendingEnableRecovery`, which can run one recovery
operation at a time. The screen only shows this body. The body ends when the
supervisor finds the evidence gone. A status update from elsewhere can't end it.

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

The supervisor represents a live batch as one typed Restore All run containing
its exclusive claim and cancellation token. Claim-tagged reducer events own
progress, the current wallet name, cancellation, and terminal projection, and
reject stale or regressive updates. Each success is reconciled immediately so
the wallet moves to its network section. An ordinary wallet-local failure stays
on its row and the batch continues; retrying that row or Retry Remaining clears
its prior error. A
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

All cloud writes go through `CloudBackupWriteSupervisor`, remote deletes
included. The write supervisor runs one remote write at a time, in the order
they were queued.

The lane does not keep its own record of who may write. The supervisor derives a
`WritePolicy` from its runtime body (`Runtime::write_policy`) and sends a newer
versioned policy to the lane each time the body changes. The policy is never
stored. When the lane reaches a queued write, it decides that write against the
latest policy, at that moment. Each write carries a command class, and the
decision is one of three:

- `Run`: the write starts
- `Paused(PauseReason)`: the write answers `CloudBackupError::WritesPaused`. A
  paused upload stays `Dirty` and sets no retry timer. Only a lost connection
  defers an upload with a retry timer. A wallet batch paused part-way answers
  the pause too, even when recording the uploads it finished is paused as well.
- `Stale`: the write belongs to an operation that ended, or to an older
  configuration. It is dropped with a debug log.

| Class | Sent by | Runs when | Paused when |
|---|---|---|---|
| `Ordinary { generation }` | background uploads, batch backups, sync, integrity check | backup on with no Turn Off running | Turn Off, backup off, account switch, reset |
| `SourceCleanup { generation }` | merged-namespace cleanup | as `Ordinary` | as `Ordinary` |
| `Operation(claim)` | operation bodies, including Turn Off's drain | the claim is the running operation | never |
| `TurnOffDelete` | Turn Off's committed namespace delete | Turn Off committed that namespace and still runs | never |
| `RowDelete(claim)` | Delete Cloud Wallet, Delete Undecryptable | that row delete is running | never |
| `OtherBackupDelete(claim)` | Delete and Recover Other Backups | that operation is running | never |
| `EnableDiscard(claim)` | discarding an interrupted Enable's namespace | that discard is running | never |

For every class without a pause, a write that does not meet the "Runs when"
condition is stale.

The same rules by runtime body:

| Runtime body | Policy body | Runs | Everything else |
|---|---|---|---|
| `Configured`, no operation | `Configured` | `Ordinary` and `SourceCleanup` of the current generation | stale |
| `Configured`, Turn Off before its commit | `Configured` (Turn Off writer) | Turn Off's own `Operation` writes, such as its drain barrier | `Ordinary` and `SourceCleanup` paused (`TurnOff`), the rest stale |
| `Configured`, a row delete | `Configured` (row writer) | `RowDelete` and `Operation` of that claim, `Ordinary` | stale |
| `Configured`, Recover or Delete Other Backups | `Configured` (other-backups writer) | `OtherBackupDelete` and `Operation` of that claim, `Ordinary` | stale |
| `Configured`, any other operation | `Configured` | `Operation` of that claim, `Ordinary` | stale |
| `Disabled`, `Corrupted`, `PendingEnableRecovery` | `Off` | Enable's `Operation`, or the discard's `EnableDiscard`, of the running claim | `Ordinary` paused (`BackupOff`), the rest stale |
| `FinishingDisable` | `FinishingDisable` | `TurnOffDelete` of the committed namespace and `Operation`, both of the running claim | `Ordinary` paused (`TurnOff`), the rest stale |
| `DriveAccountSwitch` | `AccountSwitch` | `Operation` of the running claim | `Ordinary` paused (`AccountSwitch`), the rest stale |
| `LoadFailed` | `Closed(LoadFailed)` | nothing | `Ordinary` paused, the rest stale |
| `Resetting` | `Closed(LocalReset)` | nothing | `Ordinary` paused, the rest stale |

A configuration generation is a process-local number. The supervisor starts a
new one when the body changes kind, for example when backup turns on or off, or
when a reinitialize ends. `Ordinary` and `SourceCleanup` writes from an older
generation are stale. That covers cleanup jobs already queued or being prepared
for a configuration that was replaced.

When a new policy lets ordinary writes run again, the lane tells the supervisor
(`ordinary_writes_resumed`). The supervisor then resumes uploads from the
persisted dirty state and restarts the pending-upload verifier, so a paused
upload runs exactly once. Upload confirmations carry the wallet-set generation
they read, and the supervisor drops one that a wallet change outdated.

Operation-owned writes use `CloudBackupWriteClient::for_operation`, so their
class carries the operation claim. Background writes take a ticket for the
current generation.

`CloudBackupWriteWorker` is intentionally small. It executes the remote storage
command and reports the result back to the write supervisor. The supervisor then
applies any local completion, such as marking uploaded blobs pending
confirmation. It asks the supervisor to persist the enabled state after a final
upload and waits for that answer before it marks the wallets uploaded. It
re-decides the class before it applies the completion, so a write whose
operation ended meanwhile is reported cancelled.

Turn Off pauses ordinary writes through the policy, then queues a drain barrier
of its own. The barrier comes back only after the write in flight and those
queued ahead of it have finished. Only then does Turn Off commit and delete the
namespace, so an upload that already started can't recreate remote data after
the delete. A Google Drive account switch drains the same way. A local reset
publishes a closed policy first, then waits for the lane to go idle.

## Verification And Recovery

Pending upload verification confirms that blobs marked
`UploadedPendingConfirmation` are visible in cloud storage with the expected
revision. Authorization failures pause the verifier without rewriting the blob
state, so the app can resume after the platform cloud authorization is restored.

Deep verification checks the cloud master-key wrapper, local keychain state,
wallet blobs, and pending uploads. Some verification results prepare follow-up
work, such as repairing the passkey wrapper or uploading missing wallet blobs.
The supervisor owns those continuations so each follow-up remains tied to the
same exclusive-operation claim. A verification the user or the detail screen
starts claims a `Verify` operation in the turn that admits it, before it shows,
and holds it through repair, sync and its connectivity retry. Each attempt
carries the wallet set it read, and its evidence is checked against that set.

## Related Notes

- Read `docs/passkeys.md` before changing passkey registration, targeted auth,
  presence checks, or passkey repair
- Read `docs/icloud_drive.md` before changing iCloud discovery, metadata query,
  or coordinated file access
- Read `docs/redb.md` before changing persisted cloud backup state, table
  definitions, redb value type names, or persisted type module paths
