# Cloud Backup

Cloud Backup is split between a UI-facing manager, a screen projection built
from the supervisor's runtime and admission, and a set of actors that own
long-running work. The Rust core is the source of truth for lifecycle state,
inventory completeness and generations, passkey policy, cloud writes, local
database updates, and Restore All. Swift and Kotlin draw `CloudBackupScreen`
and send actions through the manager instead of rebuilding cloud backup rules
in UI code.

## Ownership Model

`RustCloudBackupManager` is the FFI facade. It owns the reducer, the
`InventoryOwner` and the last published screen, exposes snapshots and reconcile
messages, and forwards long-running work to `CloudBackupSupervisor`.

`CloudBackupStateReducer` holds presentation state: the Enable and restore
flows, passkey, verification and sync state, the root prompt and row-operation
progress. The supervisor's runtime holds the lifecycle body, and
`Runtime::accepts_completion` discards a completion whose claim no longer owns
the running operation, so an older async path can't finish or clear newer work.
The reducer's copy of the claim follows the same rule.

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
goes back to an older or mixed one. Messages that are events, such as
`EnableCompleted`, have no revision and arrive in the order they were sent.

`SettledOff` is the event for Cloud Backup coming to rest off on this device:
the owner's own body is `Disabled` and the screen shows Disabled, not an
Enable waiting on the user. It is sent once each time the owner settles there,
after the delta that shows it: when a committed Turn Off finishes, when a local
reset ends with Cloud Backup off, when an Enable or restore ends with nothing
kept, and on the first publish of a launch that finds Cloud Backup off, which
covers a process killed before the platform handled the earlier one. A running
local reset projects as Disabled too, but its body is `Resetting`, and a wipe
that fails before its key clear resumes the backup it found, so the reset never
sends it. Platform state that belongs to a backup, such as Android's Google
Drive account binding, is released on `SettledOff` and never on the screen
alone.

The screen has a body (`Disabled`, `Enabling`, `Restoring`, `Configured`,
`FinishingDisable`, `DriveAccountSwitch`, `PendingEnableRecovery`, `LoadFailed`
or `Corrupted`), the Settings row, the persisted heal notice, the last outcome,
the root prompt and the global actions. A configured body lists its sections in
the order they are drawn, and Turn Off is in every one. Listed cloud rows carry
their own row actions. An extra copy offers Delete only while the wallet it
copies has its own current backup; until then it offers Back up first and Check
Again.

Every delete Rust offers carries a consent: each row's Delete carries a
`CloudBackupRowDeleteConsent`, and the unreadable-backups control carries a
`CloudBackupBatchDeleteConsent`. A consent names the records the delete covers,
the category the inventory put them in (`ExtraCopy`, `NotOnDevice`,
`Unsupported` or `Unreadable`), the namespace the records were listed in, and
the configuration, cloud-only listing and local wallet changes it was read
under. The platforms treat it as opaque: a
confirmation keeps the consent of the delete it opened for and sends it back
unchanged, as `DeleteCloudWallet(consent)` or
`DeleteUndecryptableWalletBackups(consent)`. For a row, dispatch builds the
delete's authorization from the consent and checks it against the rows a delete
in the namespace the keychain holds would see at that moment. For the batch, it
mints the batch again from the inventory at that moment and starts it only when
the two are equal. A consent confirmed on evidence
that has since moved, such as an extra copy that is now the only copy, a new
listing, a wallet change on this device, a new configuration or another
namespace, is rejected as `ReviewStale` next to its control, and the
republished screen offers the delete with its current consent. The generations
are counters of the running process, so a consent only means something to the
process that minted it. The namespace is carried apart from the configuration,
because a reinitialize replaces the namespace inside one configuration
generation.

The batch is frozen when it is offered: it names the undecryptable rows the
inventory listed then, and its run reads only those records and deletes only
those the read still finds undecryptable, so a backup found after the
confirmation is never deleted under it. It needs the inventory loaded,
since the rows are what it names, and the control is drawn only once the
cloud-only records are listed.

Every action is an entry with its availability, decided by the same admission
dispatch uses. A platform draws a control only for an action in the list and
enables it only when it is available; an unavailable action carries the reason
to show as its caption. The one exception is the Turn Off dialogs' Cancel,
which always shows so the dialog can close; a rejected Cancel comes back as the
last outcome. Rust marks the Google Drive actions as Android only.
A recorded Google Drive account switch is its own admission body: it admits the
switch's own controls, navigation and dismissals, and rejects anything that
needs a configured backup, Turn Off included, as the switch running. A second
Switch during one answers `Busy`. Try Again is available once a step of the
switch failed, or once the switch stopped with nothing running. Cancel is
available as well until the switch's reinitialize starts. A switch paused for
its saved passkey offers only Confirm Saved Passkey.

`just qa-check` enforces this in the Cloud Backup views: every Kotlin `enabled
=` (or `<name>Enabled =`) and every SwiftUI `.disabled(` must read an action's
or row action's `isAvailable`. The Cloud Backup views are the files in Android's
`cloudbackup` package and the iOS files named for Cloud Backup, plus any other
top-level view that reads the screen's actions or body, such as the onboarding
Cloud Backup step. The onboarding Continue next to that step follows the
onboarding flow's own state and isn't checked. The only other flags allowed are
a few per-view local facts, such as the setup checkboxes or a Cancel that waits
on an Enable with no cancel action, listed with their reasons in
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
  backups this key can't decrypt and once the cloud-only records are listed, and
  its control carries the batch's consent.
  The dialog's count comes from the report; the records come from the consent.
- `RefreshDetail` is offered only with the Check Again of the incomplete
  inventory section or of an other-backups check that failed. Opening detail is
  `EnterDetail`, which loads inventory without a control, and a row's Check
  Again is a row action.
- `DiscardPendingEnable` is the Cancel of the force-new and passkey-choice
  prompts and of the saved-passkey confirmation, where admission accepts it.
  Pending-enable recovery offers only its cleanup, when cleanup is available.
- Restore All's start, retry and cancel follow the Not On Device section's
  Restore All state.
- `RepairPasskey` (Add New Passkey) is offered with the passkey card, and every
  passkey card offers it. The card shows when the passkey is missing, its
  provider is unsupported, or it needs repair.

The supervisor owns the last outcome. It comes from dispatch rejections,
operation failures, work a control did without an operation (such as Try Again
on a failed load), and a Turn Off that ended before its commit, which shows
next to Turn Off. A passkey prompt the user cancelled shows as not completed
rather than failed. A later accepted tap on any control clears it, and
`DismissNotice(LastOutcome)` clears it too.

The Enable step, in onboarding and in Settings, shows the outcome of any
control it can show: Enable, the existing-backup prompt's choices, Confirm
Passkey on its saved-passkey pause, and the pending-enable Discard and
cleanup. The step can move between those controls before an outcome comes
back, as when a failed setup upload returns Enable to its saved-passkey pause,
so its footer doesn't tie the outcome to the control on screen.

The Settings row shows `SettingUp` during an Enable, `Restoring` during a
restore, `Disabling` while Turn Off drains before its commit,
`TurnOffIncomplete` while a committed Turn Off still has to finish,
`TurnedOnAgain` while the heal notice is on a configured backup,
`RecoveryRequired` for pending-enable recovery and for an account switch that
needs Retry, and an error row while the record failed to load or is corrupted.
Otherwise a configured backup shows its passkey, verification and sync status.

A record that failed to load shows `LoadFailed` with `RetryLoad`, which loads
it again in place; a successful retry resumes as a launch would, including an
unfinished Turn Off and the pending-enable journal's recovery. A corrupted
record shows `Corrupted` with `ResetOnThisDevice`, which runs the local reset
and then stores Disabled, so Cloud Backup can be turned on again without
reinstalling.

Every build exports `cloud_backup_screen_fixtures()`. Debug builds return a
named screen for each body and the main configured states, which the platform
contract tests render; release builds return an empty list. The iOS test
(`CloudBackupIOSSafetyHelpersTests.testEveryScreenFixtureDrawsItsAvailableActionsOnTheDetail`)
requires an enabled control on the detail for each available action unless
every control Rust names for it is Android's or iOS draws it off the detail
form. The Android test reaches each available control on its own surface,
opening the overflow menu only for a control Rust puts in the menu.

The heal notice offers `DismissHealNotice` as a global action, which dispatches
`DismissNotice(Heal)` and clears the notice from the stored record.

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

The account switch has one owner, the supervisor, and Android only carries out
its account-storage steps. Switch, Try Again and Cancel are dispatch actions.
The supervisor asks for each platform step on the reconcile channel (choose an
account, report the platform's state, commit, finalize, roll back), and Android
answers each with a typed `DriveAccountSwitchPlatformResult` through
`report_drive_account_switch_platform_result`. The supervisor keeps the one
step it waits on and acts only on the answer to that step, matched by step and
transition id; any other step result, such as a late answer for a switch a
reset ended or a second answer to a step already answered, is logged and
dropped. A state report is always taken, since the reconcile reads it against
the switch's record, and asking for one, as Try Again does, keeps the step the
switch waits on, whose answer may still come. The supervisor records the switch's phase, decides the next step, and projects the switch's stage:
switching, or needing attention with a message for the reader. A switch
paused on its saved passkey shows the saved-passkey confirmation instead. Android reports its state once at launch, so the supervisor
picks up a switch a relaunch interrupted. A failed reinitialize is rerun only
on Try Again, never at launch, because it may ask for a passkey; Try Again is
its way out, and Cancel stays unavailable, since the reinitialize may already
have begun moving this device to the new backup. A relaunch restores the
failed attempt's registered passkey for confirmation, and the switch records
that pause too, so the screen shows Confirm Saved Passkey at once, before
Android reports anything. A switch with nothing running always shows a way
out: Confirm Saved Passkey while paused, otherwise Try Again. A switch that rolls back also
discards the key material its stopped reinitialize kept on this device, so it
can't come back at the next launch as a reinitialize of the previous backup.
A local reset that leaves Cloud Backup off ends the switch with its record, a
step that failed included, and with the step it waited on, so the screen shows
Cloud Backup off rather than the switch's Try Again, and Android's late answer
to that step moves nothing. Android's next state report then finalizes or rolls back
whatever it still holds of a switch Rust no longer records. A reset that
resumes the backup keeps the switch as it was, and runs the pending-enable
journal's recovery, which hands the switch a reinitialize whose promotion it
finished.

A run of the switch's reinitialize ends one of four ways: succeeded, failed,
paused for its saved passkey, or stopped after its local promotion started.
The first three move the switch to their next phase, and only a run that
succeeded keeps its claim, through the platform's commit. A run that stopped
after its promotion started, say because the keychain refused the journal's
final delete after the durable completion was saved, or because the promotion's
active-key write and its rollback both failed, has not failed: the switch
stays reinitializing and the promotion is the journal recovery's. A promotion
that fails and rolls back fully is an ordinary failure, with the pause its
journal records again, or, for a recovered backup, nothing left of the run. The
screen shows the load failure's Try Again, Android's state report waits
meanwhile instead of starting another reinitialize, and that Try Again, or the
next launch, runs the recovery, which hands the switch its finished
reinitialize. A plain reinitialize stopped the same way isn't offered again
either. When
saving that phase fails, the run's claim ends anyway, the record still says
reinitializing, and the supervisor keeps how the run ended in memory. The
switch then shows the save failure with Try Again. Try Again, Android's state
report and Confirm Saved Passkey each save that phase before they read the
switch, and go on from it: to the commit, a rerun of the failed reinitialize,
or the pause. A held claim therefore always means a run is in progress. A
relaunch loses the unsaved result, and Android's launch report runs the
reinitialize again, as after a crash during one.

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
verification's result stale. A stale result neither overwrites the
verification requirement the wallet change recorded nor shows on screen.

Waits go one way only. The write supervisor and other workers may `call!` the
supervisor, but the supervisor never waits on them. It sends the write
supervisor a message and handles the reply as a later completion. It tells the
upload, sync-health and cleanup workers what to do with `send!` and never
awaits them. So no actor can deadlock waiting on another, and a busy worker
can't hold a reset or a lifecycle completion behind its mailbox.

Workers that prove a passkey or a master key off the actor don't write the
keychain themselves. They hand the writes to the write lane with the authority
they read before their work started: the running operation's claim, or the
background configuration generation, and the namespace the keys were proven for.
The lane decides the save at its queue head like any other command, so a save
from an operation that ended or a configuration that was replaced is refused,
and background saves are refused as paused while Turn Off, an account switch, a
reset or backup-off holds the lane, and run again when ordinary writes resume.
The save then runs on a blocking thread as the lane's in-flight command and is
refused if the keychain holds another namespace by then. Turn Off's drain and a
reset's wait for an idle lane both wait for a save that already started, so a
late proof can't restore wiped keys or replace the next backup's keys.

An effect that can't be cancelled once it starts, such as Turn Off's local
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
while the reinitialize stays on screen until that verification ends.

Swift and Kotlin own provider mechanics and native presentation classification.
They return typed cloud-storage and passkey outcomes to Rust; they do not decide
whether inventory is complete, whether an enable path is safe, or which wallets
belong in a batch. Android's generated UniFFI manager handle stays private to
`CloudBackupManager` and is accessed through its guarded wrapper methods.

The child modules under `cloud_backup_manager::actors` own narrower work:

- `restore` holds restore work, which the supervisor runs off its mailbox, and
  restore event delivery
- `uploads` owns dirty wallet upload scheduling and pending-upload verification
- `CloudBackupSyncHealthWorker` owns background sync health refreshes
- `cleanup` owns namespace cleanup after merge or recovery flows
- `write` owns serialized cloud writes, proven-key keychain saves and
  post-write local completion

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
on iOS, so UI tests find a dialog by its title. On iOS 27, XCUITest lists an
alert button that carries an identifier twice, as the action and as UIKit's
view nested in it, while VoiceOver reads it once, so iOS UI tests find an
identified alert button by its identifier and take the first match, and match
a button without one, such as a plain Cancel, by label.

The last outcome shows next to the control its target names. When no drawn
control answers it, because Rust names no control for the action, the screen
no longer lists the row, or the control is Android's Switch Google Account menu
item, both platforms show it at the top of the screen instead.

### Dialogs and Prompts Follow the Screen

An open dialog, sheet or prompt keeps only which Rust target it is for: the
record and row control (`CloudBackupRowTarget`) for a row's sheet or delete
confirmation, and the root prompt for a prompt. A delete confirmation also keeps
the consent it opened with, which its confirm button sends. Each frame reads that target's
current entry from the latest screen, through `CloudBackupScreen.listedRow` and
`rowAction`, so a dialog never shows an availability or a warning Rust
has replaced. A row Rust stops listing, or a control it stops offering, closes
the dialog bound to it, and a delete Rust offers with another consent, whether
for a new confirmation or on new evidence, replaces the dialog with one bound
to that consent instead of changing the warning under a confirm button the user
may already be reaching for. Both apps present a row's dialogs over the whole detail screen,
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
| Pending Enable journal (keychain), and the in-memory view of it admission and the screen read (`PendingWork`) | `PendingEnableCoordinator`; the journal's keychain accessors update the view on every write, read and delete | The Enable and recovery operations; the supervisor names the work that owns the journal when that work starts or the recovery takes it up |
| Paused Enable or reinitialize (`EnableContinuation`): its staged master key, passkey material and owning work | The supervisor, which files it under the work whose claim paused it | Enable and reinitialize completions carrying their claim; the restart, a reset abandoned before its wipe, and any Confirm, Discard or run that finds none held, which take it up from the journal |
| Inventory: the active namespace's detail, its cloud-only rows and listing, and the other-backups scan | `InventoryOwner` (`owner/inventory.rs`), through `RustCloudBackupManager::apply_inventory` | `InventoryInput`s: detail refresh results, cloud-only fetches, restore and delete results, local wallet changes, the other-backups scan |
| Upload confirmations proved by a remote read (a pending wallet upload marked confirmed) | The supervisor (`confirm_remote_revisions`), only for rows in the namespace the read came from and only while the producer's configuration is current | `RemoteRevisionEvidence` from detail refreshes and verification passes, tagged with that namespace and configuration generation |
| Passkey health, sync health | Their workers | Read-only: they never write the lifecycle record or the runtime |

Ownership is enforced in types, not by convention. `LifecycleWriter` needs a
`LifecycleCapability`, whose private field only `owner/lifecycle.rs` can
construct. Bootstrap creates it once, inside the exclusive bootstrap phase that
normalizes the stored record, and moves it into the supervisor. The database has
no other production route that writes the lifecycle record; restore activation,
which writes the record with its dirty rows, needs the capability too.

The writer also keeps the record it last read or committed, and the supervisor
derives its body, write policy and status from that record. A failed read gives
the writer's copy instead of a default, so a transient database error can't turn
a configured backup into an off one. A full wipe tells the writer the record is
the default one. If no read has succeeded since the load, the body is
`LoadFailed`, which admits only Retry (`RetryLoad`).

## Inventory

The detail inventory is explicitly progressive:

- `NotLoaded` has no inventory attempt yet
- `Checking` appears only while no complete inventory is loaded; it keeps the
  rows of a failed load or a provisional snapshot. A refresh over a `Complete`
  inventory leaves it `Complete` and runs silently
- `Complete` the provider confirmed is the only state that proves the
  inventory is complete; iOS's trusted local snapshot is `Complete` but enables
  only row actions
- `Failed` carries a typed incomplete reason and may retain the last known rows

A timeout, authorization failure, offline result, or provider failure is
incomplete, not a confirmed empty inventory. Restore All, Recreate Manifest and
Reinitialize stay unavailable until the inventory is `Complete` and confirmed
by the provider. A row's own Restore or Delete needs a trusted inventory:
`Complete`, or `Failed` with its rows kept for a reason other than being
offline. Turn Off doesn't wait for the inventory: an incomplete one makes its
review warn and ask for a second confirmation (see [Turn Off](#turn-off)).

The inventory loads silently. Entering the detail screen requests a refresh and
starts the silent cloud-only and other-backups reads; it never asks for the
passkey. The cloud-only fetch reads records with the master key already on this
device, and with no local key for the active namespace it lists every record as
Unchecked, and each Unchecked row offers Verify. Loading never asks for the
passkey; Verify does, and so does a restore when the local key is gone.

The supervisor embeds a `DetailWorkflow` that owns refresh scheduling. It
permits one screen refresh in flight and coalesces further requests into one
trailing refresh. A Check Again tapped while the inventory shows Checking joins
that load (`CoalescibleIntent::RefreshDetail`) and starts no second one, and
the load after it carries the tap's authority. One tapped under a loaded
inventory, whose refresh runs silently, is accepted and queues the next load.
Rust starts refresh generations no more than once every five seconds and admits
results by start order, so an older screen or operation refresh cannot overwrite
a newer one. Leaving the detail screen ends a Turn Off review on screen and
cancels nothing; refreshes keep running and their results still apply. A local
reset suspends the cycle and drops its queued work, and a completed reset
leaves it closed. A failed reset that resumes the backup reopens a cycle that
was open before or opened during the reset, with a new owner and nobody
tapping, so the inventory the reset cleared loads again silently.

A local wallet change doesn't wait for that refresh to correct the local rows.
The inventory owner drops rows for wallets that are no longer on this device at
once, and counts a deleted wallet's confirmed backup as cloud-only, so a deleted
wallet shows only under Not on This Device once the cloud-only rows are read
again, even when the refresh fails. Everything else waits for the refresh. A
detail result that read the local wallets before the deletion, from a refresh or
a verification, gets the same check against the current local wallets when it
lands, so it can't list the wallet as local again. The owner also counts the
local wallet changes it applies. A backup integrity check's detail, and a deep
verification's or an upload confirmation's, carries the count from before it
read the local wallets: one read before a wallet was added, restored or deleted
is dropped, since the change already asked for the refresh that replaces it.

The inventory also belongs to one configuration. The owner's `InventoryScope`
names the configuration generation the write policy publishes, and it advances
whenever that generation does: when the body changes kind, as Turn Off and a
reset do, and when a reinitialize ends, as an account switch's does. Advancing
it clears the inventory and asks for a fresh load. Every inventory result
carries the scope its request started in: a detail refresh, a provisional local
snapshot, an operation's refresh, a verification's detail, the cloud-only fetch
and the other-backups scan. The owner drops a result from any other scope, so a
read of the old account can't land in the new one. `InventoryReadAt` pairs the
scope with the local wallet count for the results that check both.

The launch integrity check binds one read context before any remote work: its
write client, then its `InventoryReadAt`, then the namespace from the keychain,
in that order, so neither the uploads' generation nor the read point is newer
than the namespace. The first listing, the uploads, the follow-up listing after
them and that listing's fallback all run under this one context, so a listing
of the old namespace is never handed to the owner under the scope of the
configuration that replaced it. A new configuration's read is its own request,
started by its new scope.

An own row whose wallet holds no secret on this device is marked Secret Missing
Locally only when its cloud backup holds one, such as a seed for a wallet now
watch-only here. The record header doesn't say which kind of secret a backup
holds, so the detail load reads it from the records it already decrypts with
the master key on this device; an imported xpub backed up as watch-only is never
marked. The inventory keeps what each own backup holds, a secret, none, or
unread, and Turn Off pairs it with the secrets this device holds when it
reviews.

iOS can publish a fast, incomplete local wallet snapshot before completing its
mandatory FileManager-plus-`NSMetadataQuery` union. Android's first successful
Google Drive list is complete because Drive returns provider inventory directly.
The platform result feeds the same Rust inventory owner on both platforms.

Rust projects the progress section from verification and inventory state. A
running verification's progress comes first: a card when no wallet rows are
listed, an inline row when they are. Inventory progress still shows below it.
The verification section draws no second running indicator.

Each listed record is one typed row: Not on This Device, Extra Copy,
Unsupported, Unreadable (with why: the download, the provider, parsing or
decryption failed) or Unchecked (no local master key). Only a listing whose
every record was read completes, so Unreadable and Unchecked rows keep Restore
All off. Turn Off doesn't block on them: since either may be the only copy, its
review counts them as unreadable or unchecked and asks for a second
confirmation.

An Extra Copy is another backup record of a wallet this device already holds.
Coverage names every local wallet that holds the record's identity by wallet
ID, never by name, so a renamed wallet still covers its old copy and a
same-name wallet never covers another's. The copy can be deleted once one of
those wallets has its own current backup (matched by its own record ID);
until then it offers Back up first and Check Again.

Rust also splits the cloud rows into sections: Extra Copies under Extra Copies,
and every other cloud row under Not on This Device, with Restore All. The
incomplete inventory's Check Again is in its own progress section above the
rows. The platforms draw the sections in the order Rust lists them and derive
none of them.

## Passkeys And Enable

Cloud access has an explicit presentation boundary. Silent startup discovery
must not present provider consent. User-initiated restore or enable work may use
consent-allowed access; on Android, this is the boundary at which Google Drive
authorization may appear.

A detail refresh carries who started it. Opening Cloud Backup, Check Again and
Reconnect Google Drive on a configured backup are taps, so their listing may ask
for consent. Reconnect is that tapped refresh and nothing more: it authorizes
Google Drive and reloads the inventory, never asks for the passkey, and leaves
checking the backup to Verify. Launch discovery, a provider or wallet change, a
new configuration and the refresh after a verification start with nobody
tapping, so they list silently; one that needs consent ends as an incomplete
inventory that offers Reconnect. A tap that joins a waiting or
running refresh gives the refresh after it the tap's authority. A local reset
closes the detail refresh, and nothing reopens it while the reset holds the
runtime. The listing that finishes a pending upload verification also lists
silently, since the app resumes that verification at startup and on returning
to the foreground. The refresh after a sync, restore, delete or passkey repair
may ask, since each of those operations starts from a tap.

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
typed with its owner: a fresh Enable while backup is off, a reinitialize of
the configured backup, or the reinitialize of a Google Drive account switch.
Confirm Saved Passkey restarts the owner's operation, so a paused reinitialize
goes on as a reinitialize: it uploads the new backup, switches to it and
verifies it. A paused switch holds its phase until the user confirms; it asks
the platform for no commit, and its confirmed reinitialize goes on as that
switch's run, which then asks the platform to commit the new account. A
switch's paused reinitialize has no Discard, since the account it wrote to is
the one the switch is moving to. Admission accepts Confirm Saved Passkey and
Discard only for paused work that belongs to the current lifecycle, and
Discard also for an Enable waiting on one of its prompts, the existing-backup
prompt or the passkey choice. A Discard that arrives after Enable finished is
rejected and changes nothing. Discarding a stopped
reinitialize deletes only the namespace it wrote, and the configured backup
stays on its own.

The pending-enable journal is the durable record of that paused work, and it
lasts until the work finishes or is discarded. `PendingEnableCoordinator` keeps
one in-memory view of it, `PendingWork`: nothing paused, work paused with its
owner, or unavailable. The launch's recovery fills the view in, and the
journal's keychain accessors keep it current for every writer, restart cleanup
and reset included, so admission and the screen read the view and never the
keychain. The recovery waits while the lifecycle record can't be read: the
record says whose work a paused journal is, a reinitialize's on a configured
backup or a fresh Enable's on an off one, and whether a started promotion
commits. The Try Again that loads the record runs it then, so a paused
reinitialize comes back as the reinitialize's, with its Confirm and Discard.
The continuation only keeps the material in memory, so it can end first: a
failed confirmation or upload drops it. A confirmed upload that fails
goes back to the saved-passkey pause its journal records: the screen shows its
Confirm and Discard, and Restore, a new Enable, Turn Off, a Google Drive account
switch and every other intent that would replace the configuration are refused
until one of them ends it. Confirm is the retry, with one more tap of the same
passkey. Confirm and Discard take the pause up from the journal when no
continuation holds its material, as do a relaunch and any Enable, reinitialize
or switch reinitialize that starts with none held, instead of staging a second
backup the journal would refuse. A take-up that can't read the journal shows
why next to the control that sent it and keeps the pause, so the same tap works
once the keychain reads again. A switch's rollback discards the switch's
journal too. An Enable's prompts hold no material, so their Discard
closes them without reading the journal.

A journal the recovery can't read is unavailable, never a saved passkey. So is
a started promotion no running finalization owns, and so is the journal of a
recovered backup nothing runs for: recovered work has no saved-passkey pause,
so its journal outside a run is cleanup a failed run couldn't finish, which no
new recovery may stage beside. While nothing runs, the screen shows the load
failure with Try Again, which runs the recovery again and, once the keychain
reads, gives the pause back, finishes the promotion, or runs the cleanup
again, without a relaunch. Android's switch state reports wait while it is shown.

A recovered backup's run that fails rolls the device all the way back to
before it began, through the one abort the restart's recovery also runs. An
existing backup is only recovered while no local key is active, and the
coordinator refuses to stage one otherwise, so before it began there was no
key: the abort removes the recovered key and its stage, restores the passkey
metadata and deletes the journal, and the run offered again recovers the
backup from the start. When a step of the abort fails, the run stops for the
recovery instead of failing, and a switch stays reinitializing rather than
offering its own retry, which would skip the recovery. The journal keeps the
work the recovery's, and Try Again or a relaunch runs the abort again. Each
step can run again, so the abort picks up after any step that finished, as long
as the keychain holds only this recovery's material: the journal's staged key,
or, once the key's rollback finished, no active key, so it refuses a key active
then. A journal that can't be read keeps the load failure until it reads. Any
other error in the recovery, a refused abort included, goes to the recovery
assessment: for an idle Enable or reinitialize, that offers local cleanup when
the device still matches its prior configuration, and support otherwise. A
switch can't enter that recovery and keeps the load failure.

A local promotion runs to its commit or its rollback under the restart's
recovery, which a relaunch and a local reset abandoned before its wipe both
run. The reset cancels the finalization between the promotion and the durable
completion, and the write lane drops that finalization's late writes, so the
recovery decides from durable state alone: a matching durable completion
finishes the promotion, and without one the promotion rolls back to the stage
it promoted, the previous backup is active again, and the pause waits on its
saved passkey as before. A recovered backup's promotion has no pause to wait
in, so without a completion it rolls all the way back instead. A Google Drive
account switch's reinitialize records its durable completion in the backup the
switch carries, and the recovery
reads it there. When the switch still records that reinitialize, the recovery
hands the switch its finished reinitialize: the switch saves that it waits for
the platform's commit and asks for it, so the platform's next report doesn't
run a second reinitialize beside the backup that exists. A matching completion
beside a switch in any other phase is a conflict, which the interrupted-Enable
recovery shows. The same recovery returns any other pause the reset dropped,
with the controls it had.

Work paused on its saved passkey owns the screen while nothing runs, whichever
work it is: the body is `SavedPasskeyConfirmation`, with Rust's title and
message for that work (an Enable, a reinitialize, or a Google Drive account
switch) and the controls admission accepts for it, computed when the screen
is projected. A paused Enable and a paused reinitialize offer Confirm and
Discard; a switch's paused reinitialize offers only Confirm. Until one of them
ends the pause, admission refuses every other intent that starts work, Turn
Off and Verify included, as the operation the paused work continues; prompts,
notices, the detail screen's lifecycle and a local reset still answer. The settings row
reads as setting up for a paused Enable, and as the configured backup for the
other two, since nothing is replaced until the new passkey is confirmed.

A passkey's provider hint, which says where the passkey was created, is
optional metadata stored with the credential it describes. Every save of a
credential writes its hint in the same rollback-protected write: Enable
promotion saves the hint the pending-enable journal staged, a passkey repair
saves the hint of the passkey it created or reused, and a passkey that unlocks
the cloud wrapper saves the hint that wrapper carries. A restore, and an Enable
that recovers a namespace its passkey opens, save the hint the matched wrapper
carries, read only when the wrapper's metadata names that namespace. The
snapshot a failed restore activation or enable rollback restores includes the
stored hint as opaque text. Reusing a credential keeps its hint. A credential saved without one
never reads an earlier credential's hint, and clearing local Cloud Backup state
removes it. An unreadable hint reads as no hint, so it never fails the passkey
request that reuses the credential, and it never blocks recovery of an
interrupted Enable.

The missing passkey's card says where that passkey was created. While the card
shows a missing passkey, each projection reads the hint stored for the
credential the keychain holds now and sends it as the configured screen's
`passkey_hint`, so a hint stored for an earlier credential is never shown. The
card for an unsupported provider shows none, since that card is about the
provider that refused. Both platforms draw one line on the card: "Created in
{provider} as Cove Cloud Backup ({suffix})", or "Created as Cove Cloud Backup
({suffix})" when the provider isn't one Cove recognizes. No provider gets an
unlock step. With no stored hint, the card has no line.

When an interrupted Enable leaves that evidence behind, the supervisor holds it
as its own runtime body, `PendingEnableRecovery`, which can run one recovery
operation at a time. The screen only shows this body. The body ends when the
supervisor finds the evidence gone. A status update from elsewhere can't end it.
A local reset sets the recovery aside and keeps it on screen. A reset that is
abandoned before its wipe returns to the recovery while its evidence remains.

Automatic passkey retry is shared and bounded. It applies only to a typed
platform-authorization failure that occurred before native presentation.
Presented failures, requests that didn't complete (`NotCompleted`), requests
with no ready presenter (`NotReady`), credential mismatch, unsupported
providers, invalid results, and Rust's own refusals (`NeedsPasskey`,
`PasskeyBusy`) go back to the owning flow without automatic retry or
registration fallback. Only a typed no-credential result may continue into the
owning enable or repair flow's explicit registration step. Rust puts no timeout
around interactive passkey and Drive sheets; each platform bounds a passkey
request at five minutes. Every request, and every automatic retry, needs the
live permit of the tap that started the operation; see `docs/passkeys.md`.

## Restore All

Restore All is not a second restore implementation. Individual restore and
Restore All both use `CloudBackupPreparedWalletRestore`, so wallet persistence,
duplicate handling, labels warnings, and the final outcome match Restore to This
Device. A batch prepares one identity-aware reader/session and restores records
sequentially.

Rust derives eligibility from active-namespace rows whose status is
`DeletedFromDevice`, and only from a classified inventory: the inventory owner
builds a `ClassifiedInventory` proof only when the provider confirmed the
listing, every listed record was read, and both happened in the current
configuration. Being confirmed by the provider and having every record read are
separate facts, and the proof needs both. Restore All's start, its Retry
Remaining, and the marker clear all take the run's wallets from the proof's
`RestoreAllBatch`, so none of them can act on a partial inventory. Initial
Restore All is shown for at least two eligible wallets, and is disabled until
the inventory is classified. While the marker stays, Retry Remaining shows once
the cloud-only rows have loaded, and stays disabled until the inventory is
classified, even when no row is eligible yet. Once a classified inventory has
no eligible wallet left, the marker clears and the retry goes away.
Availability also requires no conflicting exclusive operation. At start, Rust
freezes the visible order, refetches authoritative active-namespace inventory,
and intersects the two sets. Newly appeared, stale, unsupported, or
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
persisted, and nothing sets or clears it: the lane keeps only the latest copy.
When the lane reaches a queued write, it decides that write against the latest
policy, at that moment. Each write carries a command class, and the decision is
one of four:

- `Run`: the write starts
- `Paused(PauseReason)`: the write answers `CloudBackupError::WritesPaused`. A
  paused upload stays `Dirty` and sets no retry timer. A lost connection
  defers the upload with a retry timer, and a retryable failure retries on a
  backoff; a pause sets neither. A wallet batch paused part-way answers
  the pause too, even when recording the uploads it finished is paused as well.
- `Stale`: the write belongs to an operation that ended, or to an older
  configuration. It is dropped with a debug log. A stale operation write
  answers `Cancelled`; a stale background write answers
  `WritesPaused(ConfigurationChanged)`, so its row stays `Dirty` like a pause.
- `StaleTarget(StaleTarget)`: a row delete the policy would run, but whose
  authorization no longer matches the inventory. Nothing is deleted. See
  [Row Deletes](#row-deletes).

| Class | Sent by | Runs when | Paused when |
|---|---|---|---|
| `Ordinary { generation }` | background uploads, batch backups, sync, integrity check | backup on with no Turn Off or reinitialize running | Turn Off, reinitialize, backup off, account switch, reset, a failed load |
| `SourceCleanup { generation }` | merged-namespace cleanup | as `Ordinary` | as `Ordinary` |
| `Operation(claim)` | operation bodies, including Turn Off's drain | the claim is the running operation | never |
| `TurnOffDelete` | Turn Off's committed namespace delete | Turn Off committed that namespace and still runs | never |
| `RowDelete { claim, .. }` | Delete Cloud Wallet, Delete Undecryptable | that row delete is running and its authorization still matches the inventory | never |
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
| `Configured`, a row delete | `Configured` (row writer) | `RowDelete` and `Operation` of that claim, `Ordinary` and `SourceCleanup` | stale |
| `Configured`, Recover or Delete Other Backups | `Configured` (other-backups writer) | `OtherBackupDelete` and `Operation` of that claim, `Ordinary` and `SourceCleanup` | stale |
| `Configured`, discarding a stopped reinitialize | `Configured` (discard writer) | `EnableDiscard` and `Operation` of that claim, `Ordinary` and `SourceCleanup` | stale |
| `Configured`, any other operation | `Configured` | `Operation` of that claim, `Ordinary` and `SourceCleanup` | stale |
| `Disabled`, `PendingEnableRecovery` | `Off` | Enable's `Operation`, or the discard's `EnableDiscard`, of the running claim | `Ordinary` paused (`BackupOff`), the rest stale |
| `Corrupted` | `Off` | nothing | `Ordinary` paused (`BackupOff`), the rest stale |
| `FinishingDisable` | `FinishingDisable` | `TurnOffDelete` of the committed namespace and `Operation`, both of the running claim | `Ordinary` paused (`TurnOff`), the rest stale |
| `DriveAccountSwitch` | `AccountSwitch` | `Operation` of the running claim | `Ordinary` paused (`AccountSwitch`), the rest stale |
| `LoadFailed` | `Closed(LoadFailed)` | nothing | `Ordinary` paused, the rest stale |
| `Resetting` | `Closed(LocalReset)` | nothing | `Ordinary` paused, the rest stale |

A configuration generation is a process-local number. The supervisor starts a
new one when the policy body changes kind, for example when backup turns on or
off, or when a reinitialize ends. `Ordinary` and `SourceCleanup` writes from an
older generation are stale. That covers cleanup jobs already queued or being
prepared for a configuration that was replaced.

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
command and reports the result back to the write supervisor. The write
supervisor then applies any local completion, such as marking uploaded blobs
pending confirmation. It asks the lifecycle supervisor to persist the enabled
state after a final upload and waits for that answer before it marks the wallets
uploaded. It re-decides the class before it applies the completion, so a write
whose operation ended meanwhile is reported cancelled.

A reinitialize owns the namespaces from its start, the same way. Its policy
pauses ordinary writes and source cleanup, so a cleanup whose proof was read
before the reinitialize began never deletes a source it may recover. It then
drains the lane before it lists the namespaces, so a cleanup delete that was
already running finishes before the reinitialize reads what exists. When it
ends, the new generation makes anything left from before stale.

### Uploads never drop key material

A wallet upload replaces the wallet's cloud record. The record may hold the last
copy of a seed, a master private key or a TAPSIGNER backup. For example, a hot
wallet whose key is missing from the keychain, such as one left by a delete that
stopped after its keychain stage, opens as watch-only. A TAPSIGNER wallet whose
backup is missing also uploads as watch-only. So the lane never lets a record
without key material replace one that has it. The one gap is a record this
device can't see yet, described below.

- Every wallet upload is a `WalletUpload`, built only by encrypting a prepared
  entry. It records whether that entry carries key material
  (`BackupSecret::Held`) or not (`WatchOnly`), and an upload without key
  material keeps the key that reads the record it would replace.
- The worker checks just before the remote write, so a record another device
  wrote while the upload waited is the one it checks, once this device's
  provider can see it. An upload without key material reads the current record
  first.
  - It first asks the provider whether it holds the record
    (`WalletBackupReader::summary_if_present`, through `is_backup_uploaded`),
    and reads the record only if so. A record the provider doesn't hold has
    nothing to lose, so the upload goes ahead. Asking first matters on iOS,
    where a consent-allowed read of a record iCloud doesn't hold waits for it
    to sync and ends in `SyncPending`, never `NotFound`; without it, the first
    upload of every watch-only wallet failed there.
  - On iOS, "holds" means the record is in this device's iCloud metadata index
    or is already a local file. A record this device can't see yet, whether
    another device just wrote it or the index hasn't listed it yet, counts as
    absent, so a watch-only upload can replace it unchecked. This is tracked as
    a cross-device follow-up.
  - If the record holds key material (`BackupSecret::may_replace` is false), or
    its version is newer than this build reads, nothing is written. The lane
    answers `WalletUploadOutcome::KeptKeyMaterial` and applies no local
    completion, so the wallet is never marked pending or confirmed.
  - If the record doesn't decrypt or parse, the upload fails instead of
    replacing it unchecked.
- A batch skips the kept wallet and uploads the rest. The dirty-upload queue
  marks the row `Failed` with no retry. Restoring the key marks the wallet dirty
  again.
- The inventory applies the same `may_replace` rule. A local wallet whose cloud
  copy holds key material this device lacks shows `SecretMissingLocally`, not
  any upload state, and its own row is Secret Missing Locally.
- This check is the only one: no caller decides on its own whether an upload
  is safe. A watch-only wallet whose cloud copy is watch-only too still uploads
  and confirms.

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
   lists the namespace, reads this device's wallets and the secret each holds
   from the keychain, and counts other backup namespaces, all silently, and
   never asks for a passkey or lets the provider ask for consent; the recheck
   after the drain gathers the same way. The listing proves which records are
   this device's own backups. An own backup is at risk when this device doesn't
   hold the wallet's secret now and the detail inventory says the backup holds
   one, and unchecked when the inventory couldn't read what the backup holds or
   doesn't support it, even while this device holds the seed. An own backup the
   inventory hasn't read yet, such as the one Turn Off's own drain uploaded, is
   safe while the wallet identity coverage check a restore uses finds a local
   hot wallet holding the wallet's seed, since its record is that wallet's, and
   unchecked otherwise: a public-only wallet never holds a seed, and a keychain
   read error leaves it unknown. A wallet whose
   secret was deleted while its record stayed is named this way, though the
   inventory read it while it still held its secret. An extra copy is judged by
   the same read: the inventory keeps which local wallets covered it and
   whether it holds a secret or public keys only, and it stays an extra copy
   only while one of those wallets is still here and, for a secret, still holds
   it. When every covering wallet that is left lacks the secret, those wallets
   are named as missing their secret; when none is left, or one couldn't be
   read, the copy is unchecked. The local read
   is bound to the configuration and the count of local wallet changes the
   inventory reflects, so a wallet change or a new configuration the inventory
   applies while it reads makes the impact unknown. The review's assessment is
   always the detail inventory's: it is complete only when the inventory owner
   proves every listed record classified, so an inventory that isn't asks
   twice even when every listed record is this device's own. Other records are
   classified by the detail inventory as not on this device, extra copies,
   unreadable, unsupported or unchecked. A record the inventory hasn't
   classified counts as unchecked and makes the assessment partial. The screen
   shows `Preparing`, then `Reviewing` with a `CloudBackupDisableImpact`, or
   `Offline` when the connectivity check or the listing fails as offline. Any
   other listing failure, authorization included, gives a review whose
   assessment is unknown. A failed read of this device's own namespace shows no
   review; it records a failure next to Turn Off. A Turn Off tap while the
   review is `Preparing`, or while a confirmed Turn Off drains (`Draining`),
   joins it.

   Gathering this evidence is bounded. A listing that hasn't answered within 20
   seconds is dropped and makes the impact unknown, which asks twice. The count
   of other backups never holds it: the count is left out when it isn't in by
   the time the listing is. This device's wallets are read last, after the
   listing, so their secrets are as close to a commit as the evidence gets.

   Tapping Turn Off again is the dialogs' Check Again. Rust offers it by naming
   `TurnOff` on the `TurnOffDialogs` surface while the review's assessment
   isn't complete, and both apps draw it only there. It asks the inventory
   owner for a fresh classification and joins a refresh that is already
   running. The review is built once that refresh lands or fails, or at the
   same 20-second deadline, with the assessment the inventory reached. A
   result dropped as from an earlier configuration doesn't count as landing.

   The `Offline` dialog says Cove can't reach iCloud or Google Drive right now
   and that turning off Cloud Backup needs a connection. Its Try Again sends
   `DisableCloudBackup` again and follows Turn Off's availability; its Cancel
   sends `CancelDisable`.
2. **Confirmation.** The impact asks once only with a known safe set: the
   assessment is complete, the namespace lists at least one record, and every
   listed record is this device's own backup or an extra copy, known safe by
   the rules above. Anything else asks twice: an empty backup, anything
   not on this device, a local wallet's missing secret, anything unchecked,
   unreadable or unsupported, or an assessment that isn't complete; a failed
   listing makes it unknown. Names are capped at three with a count of the
   rest. Backups made with another passkey are a line in the dialog and are
   left in place.
3. **Consent.** `ConfirmDisable { review_id }` is admitted only for the review
   on screen. It starts the Turn Off operation, which drains as above, then
   gathers the evidence again, keychain read included. If the fresh impact puts
   any new record in a risk category, such as a wallet whose secret left this
   device since the review, or a wallet covering an extra copy lost its secret,
   or a wallet whose own backup is listed left this device while that backup is
   at risk or unchecked, or its assessment is worse, the operation ends and a
   new review replaces the old one with `changed_since_confirmation`. If the
   recheck finds the provider offline or needing authorization, Turn Off ends
   with `DisableNotStarted`; if the configuration changed, it is rejected as
   `ReviewStale`. Otherwise Turn Off commits: the runtime writes the commit and
   moves the body to `FinishingDisable` in one step, so a commit always comes
   with the run that finishes it, and a Turn Off that has already ended writes
   nothing.
4. **Leaving for the risk.** A review that names wallets not on this device
   offers `RestoreFirst`, and one with backups Cove couldn't check offers
   `VerifyFirst`. Each sends one intent bound to the review, like Confirm:
   `RestoreBeforeTurnOff { review_id }` or `VerifyBeforeTurnOff { review_id }`.
   Rust picks the work: Restore Them First restores the one wallet when the
   review names one, and runs Restore All when it names several; Verify First
   runs the detail verification. A stale id is rejected as `ReviewStale`;
   otherwise the button is admitted as the work it starts, so it shows that
   work's reason when it can't start, such as Restore All over an incomplete
   inventory. An admitted tap ends the review and starts the work in the same
   turn, and a failure is drawn next to that work's own control. A rejected one
   starts nothing and the review stays open. Nothing continues into Turn Off
   afterwards; the user taps Turn Off again.
5. **Before the commit** nothing is persisted. `CancelDisable` ends the review,
   the drain or the recheck. A failure ends it with a `DisableNotStarted` last
   outcome (or `Rejected(ReviewStale)` when the configuration changed) and
   leaves backup on.
6. **After the commit** the body is `FinishingDisable` and only goes forward: it
   deletes the namespace (already gone counts as deleted), clears this device's
   Cloud Backup keychain items and sync rows, and writes `Disabled`. A failed
   run shows `Failed { reason }` and persists `retry_after`. `ResumeDisable` is
   Try Again. A launch resumes it at once, and a reconnect or cloud change
   resumes it after `retry_after`; a reconnect that comes earlier arms one
   retry for that deadline, which a run that starts or a new attempt drops.
   - Admission allocates each finishing run with how it started. A run the
     user tapped (the commit, Try Again, Reconnect) deletes with a client that
     may show the provider's consent sheet; an automatic resume deletes
     silently, so a provider that wants consent fails it with
     `AuthorizationRequired`.
   - That failure offers `ReconnectDrive` on Android, which resumes the same
     committed Turn Off with consent. Switch Google Account stays unavailable.
   - `StopUsingOnThisDevice` is always drawn on this body, but it is enabled
     only once a Try Again (or an earlier Stop Using) has failed; until then it
     shows "Try again first". It finishes on this device alone and leaves the
     remote backup. Android asks for confirmation first; iOS shows the same
     warning as the section's footer. The record keeps both facts: a failed
     Try Again keeps Stop Using offered after a relaunch, and Stop Using saves
     a local-only disposition before its cleanup starts, so no later resume,
     retry or relaunch deletes the remote backup.

### Heal notice

A Turn Off from an older build could leave a `Disabling` record that never
deleted anything (it has no `delete_started_at`). Loading the record, at
bootstrap or on `RetryLoad`, turns it back into the configured backup it was,
with a heal notice, in one write. If that write fails, the body is
`LoadFailed`. The screen exports the notice as
`CloudBackupHealNotice::StuckTurnOffHealed`, the Settings row shows
`TurnedOnAgain`, and the detail screen shows a notice that Cloud Backup is on
again because an earlier Turn Off didn't finish. The notice offers Turn Off,
which sends `DisableCloudBackup` and opens the normal review, and Dismiss,
which sends `DismissNotice(Heal)` and takes the notice off the stored record.

## Row Deletes

A row delete removes one listed backup record: Delete Cloud Wallet removes one
row, and Delete Undecryptable removes the records the configured key couldn't
decrypt. Each delete runs under a `DeleteAuthorization` built from the consent
the user confirmed. Only `RowDeleteAuthority` can build one, the supervisor
holds the only authority, created at bootstrap, and the lane checks the
authorization again right before the delete runs.

### What each row may delete

A row offers Delete only when `CloudBackupDeleteCategory::of` gives its record a
category, and the consent its Delete carries names that category. The screen
draws each row's controls from that rule, and the authorization checks against
the same rule, so a row the screen offers is a row the lane deletes:

| Row | Rule |
|---|---|
| Extra copy, a local wallet holds everything it can recover, and the latest read found that wallet's current revision in the cloud | delete, standard confirmation |
| Extra copy, but the latest read didn't find the covering wallet's current backup | no Delete: back up first (the row offers Back Up First and Check Again) |
| Not on this device, including a seed backup that only a watch-only or cold wallet matches | delete, may-be-the-only-copy confirmation |
| Unsupported version | delete, may-be-the-only-copy confirmation |
| Unreadable (decrypt or parse), with the key this configuration verified | delete, may-be-the-only-copy confirmation |
| Unreadable (decrypt or parse), any other key trust | no Delete: verify first |
| Unreadable (not downloadable or provider failure) | no Delete: check again |
| Not checked yet | no Delete: verify first |

A decrypt failure counts as one of the verified key when the row itself says
so. The inventory owner keeps one fact per record: what its latest read found,
readable, gone, an unsupported version, or failed and at which stage, and for a
decrypt failure whether the key it failed under is the one this configuration
verified. Delete Undecryptable's reads arrive as `RecordReads`, and each one
replaces the row of a record still shown as unreadable, so the row's category,
its key trust and the consent its Delete carries all change together. A read
that finds the record readable makes the row what it now is, such as Not on
This Device, and the Delete offered on the unreadable row no longer holds. A
row a later fetch already read as something else is newer than the read, and
stays.

A public-only wallet never covers a seed backup. Until a read finds the covering
wallet's own backup current, deleting the copy could delete the only backup of
the seed. For example, after Restore adds a seed to a watch-only wallet, its
earlier backup holds only public keys. The covering wallets are matched by their
own record IDs, so a renamed wallet or one with the same name never stands in,
and an own row counts as current only when the read found the wallet's current
revision in the cloud. A sync row that says Confirmed is not enough, because
another device may have removed or replaced that backup since.

### Authorization

The authorization is the consent, field by field: the records it names, the
namespace they were listed in, the listing generation, where the inventory was
read, the configuration generation, and the category. It is checked against a
`DeleteView` that the inventory owner gives under one lock. The view holds:

- the active namespace
- the listing generation, which a listing with a different set of records, an
  invalidation, or a local wallet change bumps
- where the inventory was read (`InventoryReadAt`): its configuration scope,
  and the local wallets epoch, which every local wallet change bumps
- the configuration generation
- the category each row listed in the active namespace would be deleted as now,
  read through the same rule the screen draws with, but only while the rows were
  read under the latest listing

Delete Cloud Wallet's dispatch builds the authorization from the row's consent
and checks it against the view. If the namespace, the listing, where the
inventory was read, the configuration, or the record's category moved, the
delete is rejected as `ReviewStale` next to its control and nothing starts. The
turn that starts the delete operation builds the authorization again from the
same consent. Rows an invalidation left on screen offer no Delete, and a
consent from before it authorizes nothing, until a fetch reads them again. The
invalidation has the inventory owner start that fetch itself (see below).

### Execution check

When a row delete reaches the head of the lane and the policy runs it, the lane
reads the view again and checks the authorization against it. The namespace,
the listing generation, where the inventory was read, and the configuration
must be unchanged, and the record must still be deletable in the same category.
Otherwise the command answers `StaleTarget` and deletes nothing; a view the
lane can't read counts as stale too. The row's Delete says the backup changed
and asks to check again, and the inventory is invalidated, so it's read again.
A delete the provider fails says why at the row's Delete too. The screen
outcome holds both messages, as it holds a batch's, so reading the rows again
doesn't clear them; the next accepted tap replaces them.

An unreadable row's Delete follows the batch's rule (see
[Frozen batches](#frozen-batches)): the lane reads the record again with this
device's key for the namespace, just before deleting it, and deletes it only if
it still fails to decrypt. A record another device rewrote while the delete
waited is kept, the inventory owner gets the read, and the row says the backup
changed. A row delete never asks for the passkey, so without that key nothing
is read or deleted.

A record that's already gone counts as deleted. A row delete ends when the
provider answers it: the row is removed, the operation's claim is released, and
the inventory is invalidated. The lane never lists the backups after a delete.
Reading the rows again is the inventory owner's effect: any invalidation, from
a row delete, a batch that removed a record, or a stale target, starts the
silent cloud-only fetch. That fetch asks for no passkey, holds no lane slot and
no operation, and a fetch still running under an older listing is replaced by
one under the newer listing rather than waited for. So a listing that never
answers leaves the rows without Delete and the count as it was, and never
holds the lane, another operation or Turn Off. A delete doesn't record a new
backup count; the count stays the last one recorded until a finished upload
lists the backups again.

### Frozen batches

Delete Undecryptable is offered with a verification report that found records
the key couldn't decrypt, and its consent names the undecryptable rows listed
then. Dispatch starts it only when that consent equals the batch the inventory
offers at that moment. Before it reads anything, it takes the listing
generation and the configuration. Then it reads the local wallets and only the
records the consent names. The inventory owner keeps what it read only if the
listing hasn't changed since, so a wallet imported during the read can't leave
its own backup named as unreadable. The owner also drops reads made under
another configuration. Every confirmed record not on this device is read, and
each read, readable or not, goes to the owner as that record's fact.

The authority then builds the authorization from the consent, keeping only the
confirmed records whose rows the view still deletes as undecryptable under the
verified key. A confirmed record that reads again, that a later fetch
classified as anything but a decrypt failure, or whose read the owner dropped,
is skipped, and the log counts the skipped records without naming them.
Deleting part of what the user confirmed never deletes anything they didn't. When
nothing is left, nothing is deleted and the batch ends as a finished one does.

The batch deletes the authorized records in record order. Each one is a lane
command that carries the reader with the verified key, and the lane reads the
record again when it runs it, just before the delete, so a record another
device rewrote while the delete waited behind an upload is the one checked.
Only a record that still fails to decrypt is deleted. Any other read keeps the
record and goes to the inventory owner, which replaces the row, so a record
that now decrypts shows as what it is. A read that finds the provider
unreachable or needing authorization stops the batch instead. Each delete is
checked at the lane like any row delete. The batch stops at the first stale target or failure:
the records it already removed stay removed, and the records it hadn't reached
stay. A stale target shows that the backup changed and asks to check again. A
finished batch, or one that found nothing to delete, verifies again. A batch
never grows: a record found after the confirmation needs a new consent.

## Verification And Recovery

Pending upload verification confirms that blobs marked
`UploadedPendingConfirmation` are visible in cloud storage with the expected
revision. Authorization failures pause the verifier without rewriting the blob
state, so the app can resume after the platform cloud authorization is restored.

The verifier reads each blob in the namespace its sync row names, and the upload
owns that namespace: an upload, or the finalization of an Enable or
reinitialize, writes the namespace it uploaded to into the row it starts or
marks pending. A reinitialize or a Google Drive account switch moves the backup
to a new namespace under a new master key. If a wallet's row still named the
replaced namespace, the verifier would read the replaced backup with the new
key and record a permanent decryption failure for an upload that landed.

A detail refresh or a verification pass that reads a wallet's revision can also
confirm an upload waiting for it. The read is tagged with the namespace it read
and the configuration generation its producer started under
(`RemoteRevisionEvidence`), and the supervisor confirms only waiting rows in that
namespace, only while that configuration is current. A read of one backup that
outlives a reset and an Enable of another, or a Drive account switch, therefore
confirms nothing in the backup that replaced it, even when both hold the same
wallet revision.

Deep verification checks the cloud master-key wrapper, local keychain state,
wallet blobs, and pending uploads. A verification the user starts (Verify,
Verify First in the Turn Off review, or Reconnect Drive) is an exclusive
operation like any other: it holds the slot from its admission until its result
lands, so Turn Off and other operations are refused until it finishes, and a
second Verify joins the one running. Entering the detail screen and a reconnect
never start one. A non-discoverable Verify only joins the pending upload
confirmation when uploads already wait for it. Some verification results prepare
follow-up work, such as repairing the passkey wrapper or uploading missing
wallet blobs. The supervisor owns those continuations so each follow-up remains
tied to the same exclusive-operation claim. Each run carries the configuration
and wallet-set generation it read, so its evidence is judged against its own
read, never another run's or another configuration's.

Verification rebuilds the cloud wrapper from the local master key in three
cases: the wrapper is missing, neither the stored credential nor discovery
finds a passkey, or the passkey answers but no longer opens the wrapper. It
asks for a new passkey in the first two and reuses the one that answered in
the third. How that request ends decides what the screen offers next:

- If the request doesn't complete (the sheet is dismissed or the platform
  cancels it), or no window can present it yet, the verification ends
  `NotCompleted`. Neither says anything about the passkey, so it is never
  marked missing, no repair card or missing-passkey reminder shows, and the
  outcome shows beside Verify.
- If the provider can't create or use a passkey and no passkey was found, the
  verification ends `PasskeyMissing`. Only that failure of the stored passkey
  proves it missing. The passkey is marked missing, and the passkey card shows
  in its needs-repair state with Add New Passkey and Verify.
- Any other failure, including a failed rebuild when the wrapper was missing
  or the passkey had just answered, ends as a failed verification with Verify
  available and no passkey card.

`CloudBackupError::passkey_request_failure` sorts a passkey error into not
completed or failed, and the verification's typed repair cause says whether
the stored passkey was found, so the decision never rests on an error message.

A deep verification runs once per tap. A connectivity failure leaves the
verification failed with its retry action, and nothing verifies again by
itself on reconnect, on foreground or on opening the detail screen, since
verifying may need a passkey request that only the user's tap may start.

## Related Notes

- Read `docs/passkeys.md` before changing passkey registration, targeted auth,
  presence checks, or passkey repair
- Read `docs/icloud_drive.md` before changing iCloud discovery, metadata query,
  or coordinated file access
- Read `docs/redb.md` before changing persisted cloud backup state, table
  definitions, redb value type names, or persisted type module paths
