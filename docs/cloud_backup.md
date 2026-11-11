# Cloud Backup

Cloud Backup is split between a UI-facing manager, a reducer-backed state
projection, and a set of actors that own long-running work. The Rust core is
the source of truth for lifecycle state, inventory completeness and generations,
passkey policy, cloud writes, local database updates, and Restore All. Swift and
Kotlin consume `CloudBackupScreen` and send actions through the manager instead
of rebuilding cloud backup rules in UI code.

## Ownership Model

`RustCloudBackupManager` is the FFI facade. It owns the reducer, exposes
snapshots and reconcile messages, and forwards long-running work to
`CloudBackupSupervisor`.

`CloudBackupStateReducer` keeps the private lifecycle model. It also filters
stale exclusive-operation completions so an older async path cannot erase a
newer operation.

## Screen Contract

Swift and Kotlin draw only `CloudBackupScreen`. `state()` returns the one last
published as a `CloudBackupPublishedScreen`, with its revision. `project()` builds it from the supervisor's runtime, the same
`AdmissionSnapshot` dispatch decides with, the inventory facts, the notices and
the reducer, which supplies presentation only: the body comes from the runtime,
never from the reducer. After every runtime or reducer change the supervisor
projects the screen again, diffs it against the one it last sent, and sends what
changed as one `Screen` reconcile message holding a `CloudBackupScreenDelta`
at the next revision. Changes made while a publish is queued join it, and inputs
are read after the queued request is cleared, so a change that lands
mid-projection queues another publish instead of being lost. Each platform
applies the delta with `CloudBackupPublishedScreen.applying(delta)` in a single
state assignment, so no frame shows half an update.

The manager is a process singleton, so it can publish several screens before the
platform listens. The platform reads `state()` before it calls
`listen_for_updates`, and the deltas published before that read are still
queued for the listener. They are at or before the snapshot's revision, and
`applying` skips them instead of replaying them over it, so the screen never
goes back to an older or mixed one and no platform reaction, such as Android
clearing its Drive account binding on a Turn Off reaching Disabled, fires for a
transition the snapshot already includes. Messages that are events, such as
`EnableCompleted`, have no revision and arrive in the order they were sent.

The screen has a body (`Disabled`, `Enabling`, `Restoring`, `Configured`,
`FinishingDisable`, `DriveAccountSwitch`, `PendingEnableRecovery`, `LoadFailed`
or `Corrupted`), the Settings row, the persisted heal notice, the last outcome,
the root prompt and the global actions. A configured body lists its sections in
the order they are drawn, and Turn Off is in every one. Listed cloud rows carry
their own row actions. An extra copy offers Delete only while the wallet it
copies has its own current backup; until then it offers Back up first and Check
Again.

Every action is an entry with its availability, decided by the same admission
dispatch uses. A platform draws a control only for an action in the list and
enables it only when it is available; an unavailable action carries the reason
to show as its caption. Rust marks the Google Drive actions as Android only.
A recorded Google Drive account switch is its own admission body: it admits
only the switch's Retry and Cancel, and rejects everything else, Turn Off
included, as the switch running. A second switch call during one answers
`Busy`.

`just qa-check` enforces this in the Cloud Backup views: every Kotlin
`enabled =` (or `<name>Enabled =`) and every SwiftUI `.disabled(` must read an
action's or row action's `isAvailable`. The Cloud Backup views are the files in
Android's `cloudbackup` package and the iOS files named for Cloud Backup, plus
any other top-level view that reads the screen's actions or body, such as the
onboarding Cloud Backup step. The onboarding Continue next to that step follows
the onboarding flow's own state and isn't checked. The only other flags allowed are a few
per-view local facts, such as the setup checkboxes or a Cancel that waits on an
Enable with no cancel action, listed with their reasons in
`rust/xtask/src/qa/availability_gates.rs`.

Each section names the controls it draws: the verification, wallets,
other-backups and incomplete-inventory sections carry a typed `controls` list,
and the passkey card, Not On Device's Restore All state and the Turn Off section
draw theirs by kind. Android's top-bar `menu`, the Turn Off dialogs, the heal
notice and every other body's exits are named the same way.
`CloudBackupScreen.drawnControls()` lists each control with the surface that
draws it, and the action list holds exactly the actions it names, so Rust never
offers an action with no control. Both platforms draw a section's controls from
its list and decide nothing about which controls exist from what the section
shows. `screen/contract.rs` holds each control to its surface's content, written
from the platform views, and the fixture test and harness invariant 4a hold
every screen to it. In particular:

- `Verify` is offered with every verification state except a running one, one
  waiting for an upload to be confirmed, and a backup only a newer Cove can
  read, which nothing on this device can fix.
- `RecreateManifest` and `ReinitializeBackup` are offered only as the recovery
  a failed verification names, on its failure card on both platforms. A backup
  that verifies offers neither: Android's menu no longer lists them on a healthy
  backup and holds only Switch Google Account.
- `SyncUnsynced` is Sync Now under the wallets waiting to upload, or the Try
  Again of an upload confirmation that failed. With every wallet up to date it
  isn't offered.
- `DeleteUnreadableBackups` is offered with a verification report that counts
  backups this key can't decrypt.
- `RefreshDetail` is offered only with the Check Again of the incomplete
  inventory section or of an other-backups check that failed. Opening detail is
  `EnterDetail`, which loads inventory without a control, and a row's Check
  Again is a row action.
- `DiscardPendingEnable` is the Cancel of the force-new and passkey-choice
  prompts and of the saved-passkey wait. Pending-enable recovery offers only
  its cleanup, when cleanup is available.
- Restore All's start, retry and cancel follow the Not On Device section's
  Restore All state.

The supervisor owns the last outcome. It comes from dispatch rejections,
operation failures and a Turn Off that ended before its commit, which shows
next to Turn Off. A later accepted tap replaces it, and
`DismissNotice(LastOutcome)` clears it.

The Settings row shows `TurnOffIncomplete` while a committed Turn Off still has
to finish, `TurnedOnAgain` while the heal notice is on a configured backup, and
an error row while the record failed to load or is corrupted. Dismissing the
notice dispatches `DismissNotice(Heal)`, which clears it from the stored record.

A record that failed to load shows `LoadFailed` with `RetryLoad`, which loads
it again in place; a successful retry resumes as a launch would, including an
unfinished Turn Off. A corrupted record shows `Corrupted` with
`ResetOnThisDevice`, which runs the local reset and then stores Disabled, so
Cloud Backup can be turned on again without reinstalling.

Debug builds export `cloud_backup_screen_fixtures()`, a named screen for each
body and the main configured states, which the platform contract tests render.
Release builds return none. The iOS test requires an enabled control on the
detail for each available action unless every control Rust names for it is
Android's or iOS draws it off the detail form. The Android test reaches each
available control on its own surface, opening the overflow menu only for a
control Rust puts in the menu.

`PendingEnableCoordinator` owns the local keychain transaction for fresh and
recovered enables. It keeps CSPP staging and promotion ordered with the durable
pending-enable journal while the manager and supervisor retain cloud writes,
operation claims, and UI reconciliation.

`CloudBackupSupervisor` is the top-level operation owner. It starts enable,
restore, disable, repair, verification, sync, and cloud-only flows, then applies
typed completions back through the manager. Exclusive operations carry a
`CloudBackupExclusiveOperationClaim` so every async completion can prove it
still belongs to the active operation. Each run gets its own `OperationId`. A
Google Drive account switch's reinitialization also names the switch's persisted
transition id, which platform callbacks match across a relaunch, but resuming
the switch starts a run with a new `OperationId`. A completion from a run that a
reset revoked therefore never matches the run that resumed the switch.

Work that waits on the provider runs off the supervisor's mailbox, so the
supervisor keeps answering admission and resets meanwhile. A recovered Enable
with nothing left to upload settles its upload confirmation inside the
supervisor's handler, because that read stays local while the Enable runs: the
confirmation has no uploads to check, and its detail refresh skips the provider
until the lifecycle leaves Enabling.

The supervisor is also the only writer of the durable lifecycle record: it holds
the one `LifecycleWriter`, which bootstrap moves into it. Work that runs off the
actor, such as the write lane, upload confirmation, wallet changes and the
integrity check, sends the supervisor a typed result message, and the supervisor
applies the write and reconciles. Verification evidence is tied to the wallet set
it read, so a wallet change that lands while a verification runs makes that
verification's result stale. A stale result neither overwrites the reset nor
shows on screen.

Waits go one way only. The write supervisor and other workers may `call!` the
supervisor, but the supervisor never waits on them. It sends the write
supervisor a message and handles the reply as a later completion. It tells the
upload, sync-health and cleanup workers what to do with `send!` and never
awaits them. So no actor can deadlock waiting on another, and a busy worker
can't hold a reset or a lifecycle completion behind its mailbox.

Workers that prove a passkey or a master key off the actor don't write the
keychain themselves. They hand the writes to the write lane with the authority
they read before their work started: the running operation's claim, or the
background configuration generation, and the namespace the keys were proven
for. The lane decides the save at its queue head like any other command, so a
save from an operation that ended or a configuration that was replaced is
refused, and background saves wait while Turn Off, an account switch or a reset
runs. The save then runs on a blocking thread as the lane's in-flight command and
is refused if the keychain holds another namespace by then. Turn Off's drain and
a reset's wait for an idle lane both wait for a save that already started, so a
late proof can't restore wiped keys or replace the next backup's keys. An effect that can't be cancelled once it starts, such as Turn Off's local
cleanup or a passkey registration, puts its operation in an `Effecting` stage,
and the operation keeps the slot until the effect reports. A local reset, which
a full wipe uses, closes admission, cancels running operations, waits for any
in-flight effect to report, and then has the supervisor clear the Cloud Backup
keychain namespace. Before that clear starts, the supervisor commits Cloud Backup
off and records the clear, because even a partial clear can't be taken back. So
every later load of the record, whether after a failed wipe, a relaunch or a
crash mid-wipe, finds Cloud Backup off once its keys may be gone. If that commit
fails, the supervisor refuses the clear and the keys stay. A wipe that fails
before the clear leaves the backup as it was.
Catastrophic recovery clears the namespace only while no
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

### Accessibility Identifiers

Both apps give each control and surface below the same identifier, so a UI test
on either platform finds a control by the action it sends. Rust owns the
mapping: `CloudBackupUiElement.accessibilityIdentifier()` returns it, iOS calls
it, and Android keeps a copy in `CloudBackupTags` because its previews run
without the Rust library. `CloudBackupScreenContractTest` (Android) checks that
copy, and an iOS unit test checks the iOS UI test helper, against every element
`cloud_backup_ui_elements()` lists; a Rust test checks this table. `<action>` is
the `CloudBackupScreenAction` case name in lower camel case, such as `turnOff`
or `acceptEnablePromptCreateNew`, and `<rowControl>` is the
`CloudBackupRowControl` case name: `restore`, `delete`, `backUpFirst`,
`checkAgain` or `verify`. Provider names stay in visible text and never appear
in an identifier.

| Element | Identifier |
|---|---|
| A global action's control, wherever it is drawn, including Turn Off's Confirm and Cancel and a prompt's choices | `cloudBackup.action.<action>` |
| The reason under a disabled action's control | `cloudBackup.action.<action>.unavailable` |
| The confirm button of the dialog an action opens | `cloudBackup.action.<action>.confirm` |
| A row control | `cloudBackup.row.<recordId>.<rowControl>` |
| The confirm button of the dialog a row control opens | `cloudBackup.row.<recordId>.<rowControl>.confirm` |
| A wallet's row in any list; on Android a listed row opens its action sheet | `cloudBackup.wallet.<recordId>` |
| The reason every row in a section shares | `cloudBackup.rows.unavailable` |
| The last outcome and its dismiss | `cloudBackup.lastOutcome`, `cloudBackup.lastOutcome.dismiss` |
| The heal notice and its Turn Off | `cloudBackup.healNotice`, `cloudBackup.healNotice.turnOff` |
| Turn Off progress | `cloudBackup.turnOff.preparing`, `cloudBackup.turnOff.draining` |
| Turn Off offline dialog and its Try Again | `cloudBackup.turnOff.offline`, `cloudBackup.turnOff.offline.tryAgain` |
| Turn Off review, its Continue and Check Again, and the final dialog | `cloudBackup.turnOff.review`, `cloudBackup.turnOff.continue`, `cloudBackup.turnOff.checkAgain`, `cloudBackup.turnOff.final` |
| Body panels | `cloudBackup.finishingDisable`, `cloudBackup.loadFailed`, `cloudBackup.corrupted`, `cloudBackup.driveAccountSwitch` (Android only) |
| Inventory progress | `cloudBackup.inventory.checking`, `cloudBackup.inventory.incomplete` |
| The support code an incomplete setup shows | `cloudBackup.recovery.supportCode` |
| The top-bar menu (Android only) | `cloudBackup.menu` |
| Root prompts | `cloudBackup.prompt.existingBackupFound`, `cloudBackup.prompt.passkeyChoice`, `cloudBackup.prompt.missingPasskeyReminder`, `cloudBackup.prompt.verification` |
| The repair prompt's Create New Passkey, which sends `repairPasskey` like its Use Existing Passkey | `cloudBackup.prompt.passkeyChoice.createNewPasskey` |

A system alert or confirmation dialog can't carry an identifier on its container
on iOS, and a dialog button may not carry its identifier on every OS version,
so UI tests match dialog buttons by label too.

### Dialogs and Prompts Follow the Screen

An open dialog, sheet or prompt keeps only which Rust target it is for: the
record and row control (`CloudBackupRowTarget`) for a row's sheet or delete
confirmation, and the root prompt for a prompt. Each frame reads that target's
current entry from the latest screen, through `CloudBackupScreen.listedRow` and
`rowAction`, so a dialog never shows an availability or a warning Rust
has replaced. A row Rust stops listing, or a control it stops offering, closes
the dialog bound to it, and a changed delete confirmation replaces the dialog
instead of changing the warning under a confirm button the user may already be
reaching for. Both apps present a row's dialogs over the whole detail screen,
so a row that moves to another section, such as an extra copy that becomes the
only copy, keeps its confirmation, which then warns with the new confirmation.

A prompt's choices are global actions like any other: the existing-backup and
passkey-choice prompts draw `acceptEnablePromptUseExisting` and
`acceptEnablePromptCreateNew` (or `repairPasskey` for both repair choices), and
the verification prompt draws `verify`. Each choice is drawn only when Rust
sends its entry, is disabled with Rust's reason when it can't run, and carries
its last outcome. A choice dispatches without closing the prompt: the prompt
closes when Rust stops projecting it, so a rejected choice leaves the prompt up
with the rejection in it. An iOS alert closes on any tap, so iOS presents the
prompt again while Rust still projects it, with the rejection in its message.
Cancel and Not Now are the prompts' escapes, which
Rust always admits, so they have no entry.

### Who Writes What

Each fact has one writer. Other code sends that writer a typed result and never
writes the fact itself.

| Fact | Single writer | Typed inputs from producers |
|---|---|---|
| Durable lifecycle record (redb `PersistedCloudBackupState`): configuration, verification, sync fields, the Restore All marker, the pending upload confirmation, the account-switch phase and the committed disable | The supervisor's `OwnedRuntime`, which holds the only `LifecycleWriter` and reaches it through `commit`, which publishes the write policy after every write. One write skips that publish: a ready reset's `clear_keys_in_reset` commits Cloud Backup off before the key clear while the policy is already `Closed(LocalReset)`, which that write can't change, and `finish_reset` publishes the policy the reset ends with | Verification results, upload confirmations (carrying the configuration and wallet-set generation they read), write-lane completions, wallet changes, the integrity check, restore activation, the account switch |
| Runtime operation (the body and the operation nested in it) | The supervisor's `OwnedRuntime` | Worker completions carrying the operation's claim |
| Write policy | Nobody: `Runtime::write_policy` derives it, and `OwnedRuntime` publishes it, versioned, to the write lane | — |
| Cloud Backup keychain namespace | The supervisor, in a handler or an `Effecting` stage | — |
| Keys a worker proved (passkey credential, master key) | The write lane, which decides each save under the producer's `WriteOwner` and runs it off the actor | `OwnedKeychainWrite` saves |
| Pending Enable journal (keychain) | `PendingEnableCoordinator` | The Enable and recovery operations |
| Paused Enable or reinitialize (`EnableContinuation`): its staged master key, passkey material and owning work | The supervisor, which files it under the work whose claim paused it | Enable and reinitialize completions carrying their claim, and the restart, which reads the journal |
| Inventory: the active namespace's detail, its cloud-only rows and listing, and the other-backups scan | `InventoryOwner` (`owner/inventory.rs`), through `RustCloudBackupManager::apply_inventory` | `InventoryInput`s: detail refresh results, cloud-only fetches, restore and delete results, local wallet changes, the other-backups scan |
| Passkey health, sync health | Their workers | Read-only: they never write the lifecycle record or the runtime |

Ownership is enforced in types, not by convention. `LifecycleWriter` needs a
`LifecycleCapability`, whose private field only `owner/lifecycle.rs` can
construct. Bootstrap creates it once, inside the exclusive bootstrap phase that
normalizes the stored record, and moves it into the supervisor. The database has
no other production route that writes the Cloud Backup tables.

The writer also keeps the record it last read or committed, and the supervisor
derives its body, write policy and status from that record. A failed read gives
the writer's copy instead of a default, so a transient database error can't turn
a configured backup into an off one. A full wipe tells the writer the record is
the default one. If no read has succeeded since the load, the body is
`LoadFailed`, which admits nothing.

## Inventory

The detail inventory is explicitly progressive:

- `NotLoaded` has no inventory attempt yet
- `Checking` may retain the last known detail and wallet rows while a newer
  generation is in flight
- `Complete` is the only state that proves the current provider inventory is
  complete
- `Failed` carries a typed incomplete reason and may retain the last known rows

A timeout, authorization failure, offline result, or provider failure is
incomplete, not a confirmed empty inventory. Restore All and the actions that
depend on a record's absence stay disabled until the inventory is complete.
Turn Off doesn't wait for it: an incomplete inventory makes its review warn and
ask for a second confirmation, as described under Turn Off.

The inventory loads silently. Entering the detail screen only requests a
refresh; it never asks for the passkey. The cloud-only fetch reads records
with the master key already on this device, and with no local key for the
active namespace it lists every record as Unchecked, which the screen answers
with a Verify card. Only a user's Verify asks for the passkey.

The supervisor embeds a `DetailWorkflow` that owns refresh scheduling. It
permits one screen refresh in flight and coalesces further requests into one
trailing refresh. A Check Again tapped while the inventory shows Checking joins
that load (`CoalescibleIntent::RefreshDetail`); one tapped under a loaded
inventory, whose refresh runs silently, is accepted and queues the next load. Rust starts refresh generations no more than
once every five seconds and admits results by start order, so an older screen or
operation refresh cannot overwrite a newer one. Closing the detail screen
invalidates its refresh owner without cancelling manager-owned operations.

A local wallet change doesn't wait for that refresh to correct the local rows.
The inventory owner drops rows for wallets that are no longer on this device at once,
and counts a deleted wallet's confirmed backup as cloud-only, so a deleted
wallet shows only under Not on This Device once the cloud-only rows are read
again, even when the refresh fails. Everything else waits for the refresh.
A detail result that read the local wallets before the deletion, from a refresh
or a verification, gets the same check against the current local wallets when
it lands, so it can't list the wallet as local again. The owner also counts the
local wallet changes it applies. A backup integrity check's detail, and a deep
verification's or an upload confirmation's, carries the count from before it
read the local wallets: one read before a wallet was added, restored or deleted
is dropped, since the change already asked for the refresh that replaces it.

An own row whose wallet holds no secret on this device is marked Secret Missing
Locally only when its cloud backup holds one, such as a seed for a wallet now
watch-only here. The record header doesn't say which kind of secret a backup
holds, so the detail load reads it from the records it already decrypts with
the master key on this device; an imported xpub backed up as watch-only is never
marked. When a watch-only wallet's backup couldn't be read, its secret is
unchecked, and the inventory's assessment for a Turn Off stays partial.

iOS can publish a fast, incomplete local wallet snapshot before completing its
mandatory FileManager-plus-`NSMetadataQuery` union. Android's first successful
Google Drive list is complete because Drive returns provider inventory directly.
The platform result feeds the same Rust inventory owner on both platforms.

Rust projects the progress section from verification and inventory state.
Verification progress takes priority over inventory progress: a standalone card
when no wallet rows are listed, an inline row when they are. The verification
section draws no second running indicator.

Each listed record is one typed row: Not on This Device, Extra Copy,
Unsupported, Unreadable (with why: the download, the provider, parsing or
decryption failed) or Unchecked (no local master key). Only a listing whose
every record was read completes, so Unreadable and Unchecked rows keep Restore
All off. Turn Off doesn't block on them: since either may be the only copy, its
review warns about them and asks for a second confirmation.

An Extra Copy is another backup record of a wallet this device already holds.
Coverage names every local wallet that holds the record's identity by wallet
ID, never by name, so a renamed wallet still covers its old copy and a
same-name wallet never covers another's. The copy can be deleted once one of
those wallets has its own current backup (matched by its own record ID);
until then it offers Back up first.

Rust also splits the cloud rows into sections: Extra Copies under Extra Copies,
and every other cloud row under Not on This Device, with Restore All and the
incomplete list's Check Again. The platforms draw the sections in the order
Rust lists them and derive none of them.

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

Every way into Enable, onboarding's included, is an intent that admission
decides against the runtime body, so an Enable can't start beside a running
Restore. An answer to an Enable prompt is decided as the Enable it starts, and
the supervisor clears the prompt and starts that Enable in the same turn. When
the Enable can't start, the prompt stays.

Fresh enable material is tracked by a durable pending-enable journal through
passkey registration, remote writes, and local promotion. A newly saved passkey
is targeted and confirmed before upload continues. Accepted provider writes are
recorded for visibility confirmation so a restart or reconnect can resume the
confirmation work without leaving the lifecycle stranded in `Enabling`.
Promotion happens only for journal-owned staged material. Failure or
cancellation restores the prior local metadata and only cleans remote material
that is proven to be incomplete and owned by that fresh attempt.

A newly saved passkey, or an upload that failed, pauses the work that ran it.
The supervisor keeps what that work needs to go on as one `EnableContinuation`,
typed with its owner: a fresh Enable while backup is off, or a reinitialize of
the configured backup. Confirm Saved Passkey restarts the owner's operation, so
a paused reinitialize goes on as a reinitialize: it uploads the new backup,
switches to it and verifies it. Admission accepts Confirm Saved Passkey and
Discard only for a continuation that belongs to the current lifecycle, and
Discard also for an Enable waiting on one of its prompts, the existing-backup
prompt or the passkey choice. A Discard that arrives after Enable finished is
rejected and changes nothing. Discarding a stopped
reinitialize deletes only the namespace it wrote, and the configured backup
stays on its own.

When an interrupted Enable leaves that evidence behind, the supervisor holds it
as its own runtime body, `PendingEnableRecovery`, which can run one recovery
operation at a time. The screen only shows this body. The body ends when the
supervisor finds the evidence gone. A status update from elsewhere can't end it.
A local reset sets the recovery aside and keeps it on screen. A reset that is
abandoned before its wipe returns to the recovery while its evidence remains.

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
`DeletedFromDevice`, and only from a classified inventory: the inventory owner
builds a `ClassifiedInventory` proof only when the provider confirmed the
listing and every listed record was read. Being confirmed by the provider and
having every record read are separate facts, and the proof needs both. Restore
All's start, its Retry Remaining, and the marker clear all take the run's
wallets from the proof's `RestoreAllBatch`, so none of them can act on a
partial inventory. Initial Restore All is shown for at least two eligible
wallets, and is disabled until the inventory is classified. While the marker
stays, Retry Remaining shows once the cloud-only rows have loaded, and stays
disabled until the inventory is classified, even when no row is eligible yet.
Once a classified inventory has no eligible wallet left, the marker clears and
the retry goes away. Availability also requires no conflicting exclusive
operation. At start, Rust freezes the visible order, refetches authoritative
active-namespace inventory, and intersects the two sets. Newly appeared, stale,
unsupported, or already-local rows are not added to the run.

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
| `Ordinary { generation }` | background uploads, batch backups, sync, integrity check | backup on with no Turn Off or reinitialize running | Turn Off, reinitialize, backup off, account switch, reset |
| `SourceCleanup { generation }` | merged-namespace cleanup | as `Ordinary` | as `Ordinary` |
| `Operation(claim)` | operation bodies, including Turn Off's drain | the claim is the running operation | never |
| `TurnOffDelete` | Turn Off's committed namespace delete | Turn Off committed that namespace and still runs | never |
| `RowDelete(claim)` | Delete Cloud Wallet, Delete Undecryptable | that row delete is running | never |
| `OtherBackupDelete(claim)` | Delete and Recover Other Backups | that operation is running | never |
| `EnableDiscard(claim)` | discarding the namespace of an interrupted Enable or a stopped reinitialize | that discard is running | never |

For every class without a pause, a write that does not meet the "Runs when"
condition is stale.

The same rules by runtime body:

| Runtime body | Policy body | Runs | Everything else |
|---|---|---|---|
| `Configured`, no operation | `Configured` | `Ordinary` and `SourceCleanup` of the current generation | stale |
| `Configured`, Turn Off before its commit | `Configured` (Turn Off writer) | Turn Off's own `Operation` writes, such as its drain barrier | `Ordinary` and `SourceCleanup` paused (`TurnOff`), the rest stale |
| `Configured`, a reinitialize | `Configured` (reinitialize writer) | the reinitialize's own `Operation` writes, such as its drain barrier | `Ordinary` and `SourceCleanup` paused (`Reinitialize`), the rest stale |
| `Configured`, a row delete | `Configured` (row writer) | `RowDelete` and `Operation` of that claim, `Ordinary` | stale |
| `Configured`, Recover or Delete Other Backups | `Configured` (other-backups writer) | `OtherBackupDelete` and `Operation` of that claim, `Ordinary` | stale |
| `Configured`, discarding a stopped reinitialize | `Configured` (discard writer) | `EnableDiscard` and `Operation` of that claim, `Ordinary` | stale |
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
upload runs exactly once. Upload confirmations carry the configuration and
wallet-set generation they read (`EvidenceBasis`), and the supervisor drops one
that a wallet change or a new configuration outdated, such as a Drive account
switch that keeps the namespace.

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

A confirmed Turn Off pauses ordinary writes through the policy, then queues a
drain barrier of its own. The barrier comes back only after the write in flight
and those queued ahead of it have finished. Only then does Turn Off commit and
delete the namespace, so an upload that already started can't recreate remote
data after the delete. A Google Drive account switch drains the same way. A
local reset publishes a closed policy first, then waits for the lane to go idle.

## Turn Off

Turn Off never blocks on what the backup holds. It shows what would be lost and
asks for consent instead.

1. **Review.** `DisableCloudBackup` builds a review without running an
   operation or pausing writes. Only an idle configured body holds it, and a
   running operation replaces that body, so the two never exist together: any
   operation start, body change, Cancel or `LeaveDetail` ends it. The review
   lists the namespace and counts other backup namespaces silently, and never
   asks for a passkey. The listing proves which records are this device's own
   backups, and the detail inventory says whether an own backup holds a secret
   this device lacks. An own backup whose secret the inventory couldn't check
   counts as unchecked. The review's assessment is always the detail
   inventory's, so an inventory that isn't complete asks twice even when every
   listed record is this device's own. Other records are classified by the
   detail inventory as not on this device, extra copies, unreadable,
   unsupported or unchecked. A record the inventory hasn't classified counts as
   unchecked. The screen shows `Preparing`, then `Reviewing` with a
   `CloudBackupDisableImpact`, or `Offline` when the provider can't be reached.
   Tapping Turn Off again is the dialogs' Check Again.
2. **Confirmation.** The impact asks once when only this device's backups and
   extra copies would be deleted. It asks twice when anything is not on this
   device, a local wallet's secret is missing, anything is unchecked, unreadable
   or unsupported, or the assessment isn't complete; a failed listing makes it
   unknown. Names are capped at three with a count of the rest. Backups made
   with another passkey are a line in the dialog and are left in place.
3. **Consent.** `ConfirmDisable { review_id }` is admitted only for the review
   on screen. It starts the Turn Off operation, which drains as above, then
   lists again. If the fresh impact puts any new record in a risk category, or
   its assessment is worse, the operation ends and a new review replaces the
   old one with `changed_since_confirmation`. Otherwise Turn Off commits.
4. **Before the commit** nothing is persisted. `CancelDisable` ends the review
   or the drain. A failure ends it with a `DisableNotStarted` last outcome and
   leaves backup on.
5. **After the commit** the body is `FinishingDisable` and only goes forward: it
   deletes the namespace (already gone counts as deleted), clears this device's
   Cloud Backup keychain items and sync rows, and writes `Disabled`. A failed
   run shows `Failed { reason }` and persists `retry_after`. `ResumeDisable` is
   Try Again. A launch resumes it at once, and a reconnect or cloud change
   resumes it after `retry_after`. Once a Try Again has failed,
   `StopUsingOnThisDevice` finishes on this device alone and leaves the remote
   backup.

## Verification And Recovery

Pending upload verification confirms that blobs marked
`UploadedPendingConfirmation` are visible in cloud storage with the expected
revision. Authorization failures pause the verifier without rewriting the blob
state, so the app can resume after the platform cloud authorization is restored.

A detail refresh or a verification pass that reads a wallet's revision can also
confirm an upload waiting for it. The read is tagged with the namespace it read
and the configuration generation its producer started under
(`RemoteRevisionEvidence`), and the supervisor confirms only waiting rows in that
namespace, only while that configuration is current. A read of one backup that
outlives a reset and an Enable of another, or a Drive account switch, therefore
confirms nothing in the backup that replaced it, even when both hold the same
wallet revision.

Deep verification checks the cloud master-key wrapper, local keychain state,
wallet blobs, and pending uploads. A verification the user starts, from Verify,
detail entry or a retry after the connection returns, is an exclusive operation
like any other: it holds the slot from its admission until its result lands, so
Turn Off and other operations wait for it, and a second Verify joins the one
running. Verify only joins the pending upload confirmation when uploads already
wait for it. Some verification results prepare follow-up work, such as
repairing the passkey wrapper or uploading missing wallet blobs. The supervisor
owns those continuations so each follow-up remains tied to the same
exclusive-operation claim. Each run carries the configuration and wallet-set
generation it read, including across a retry after lost connectivity, so its
evidence is judged against its own read, never another run's or another
configuration's.

## Related Notes

- Read `docs/passkeys.md` before changing passkey registration, targeted auth,
  presence checks, or passkey repair
- Read `docs/icloud_drive.md` before changing iCloud discovery, metadata query,
  or coordinated file access
- Read `docs/redb.md` before changing persisted cloud backup state, table
  definitions, redb value type names, or persisted type module paths
