# Passkeys

Notes on passkey behavior that matters for Cloud Backup flows.

## Who may present a request

A passkey request puts a system sheet on screen, so only a user's tap may start
one, and only inside the operation that tap started.

- Admission mints a permit ticket only for these taps: Enable, Confirm Saved
  Passkey, Restore from Cloud Backup, Verify, Repair Passkey, Recover Other
  Backups, Recreate Manifest, Restore Cloud Wallet, Restore All and its Retry
  Remaining, Reinitialize, Delete Undecryptable Wallet Backups, and Switch Drive
  Account and its Try Again. Restore Cloud Wallet and Restore All present a
  request only when the local master key is missing.
- Reconnect Google Drive gets no ticket. On a configured backup it is the
  tapped inventory refresh, whose listing may show Google's consent sheet, and
  on a committed Turn Off it resumes that Turn Off; neither asks for the
  passkey, so checking the backup stays the separate Verify tap.
- A Turn Off review button carries the permit of the work it names: Verify
  First gets Verify's, and Restore Them First gets Restore Cloud Wallet's or
  Restore All's.
- The runtime keeps the permit's grant on the record of the operation the tap
  started. The permit is live while that record holds the grant. A finish, a
  cancel, a reset or a hand-off that drops the record revokes every copy at
  once. A hand-off moves the grant to the operation that takes the slot.
- Try Again on a Drive switch whose reinitialize stopped with no run, as after
  a reset that failed before its wipe, can't start the run itself: only the
  platform's state report says whether the staged account is still there. The
  switch holds the tap's ticket for that one report. A restart of the same
  switch runs under it, and every other outcome drops it, as do a reset, even
  one that fails, and the end of the switch. A report with no Try Again before
  it, as at launch, restarts the run with no permit.
- `PlatformAuthorizationRetrier` is the only caller of the provider's
  interactive methods. A clippy `disallowed-methods` rule in `rust/clippy.toml`
  keeps it that way. Before each attempt, including each automatic retry, it
  checks the permit is still live.
- The platform checks again in the step that starts the sheet, after its
  presenter is ready, by calling `passkey_request_may_present`. The operation
  can end while the platform waits for a presenter; its request then reports
  `NotCompleted` and shows nothing. A presence check never asks, because it
  never holds the request gate.
- Cancelling a restore or Restore All revokes its permit at once. The run keeps
  its claim until its worker drains, but nothing it starts after the cancel can
  present.
- Background and follow-up work holds `Silent` authority: uploads, sync, the
  background cloud fetch, detail entry and refresh, reconnects and relaunch
  reconciliation. Any request it makes fails with `NeedsPasskey` before it
  reaches the platform.
- One request is on screen at a time. A request that starts while another is
  still open fails with `PasskeyBusy` instead of queueing behind a sheet the
  user can't see, even when the open request belongs to an operation that has
  already ended.
- A request the user leaves without completing ends the operation it belongs
  to. It is not retried, it does not fall back to discovery or registration,
  and it does not mark the stored credential missing. The root verification
  prompt is dismissed by the tap, and it does not return after a relaunch.
- A passkey failure is logged only in its redacted form, and Rust owns the one
  allowlist of platform error domains that decides it. `PasskeyError`'s
  `Display` and `Debug` print only its `PasskeyErrorDiagnostic`: the outcome,
  the operation and, for a listed domain, that domain and its code. Any other
  domain shows as `error_domain=unlisted`, and the platform's own text never
  shows. The raw diagnostic still reaches Rust inside
  `PasskeyFailureReason::Unknown`, but nothing formats it. So every Rust log of
  a passkey error is redacted, and so is the error text Swift and Kotlin print.
  A finished request logs the same outcome and underlying error with the
  elapsed time.
- Native logs never format a platform error themselves. iOS passes an
  `NSError`'s domain and code, and Android a credential exception's type, to
  `passkeyRequestLogLine`, which applies the same allowlist.

## Interactive request timeout

iOS registration and assertion requests wait at most five minutes for
Authentication Services to complete. When that bound is reached, Cove cancels
the authorization controller on the main queue and reports a presented platform
authorization failure. Delegate completion is terminal and idempotent, so a
late success, cancellation, or error callback cannot replace the timeout result.

The one-second presence probe is separate and remains intentionally short. A
presence timeout is indeterminate rather than an interactive request failure.

## Presence checks

Do not use passkey presence checks for background polling.

Platform presence checks can show passkey UI:

- iOS uses authorization requests that may present system passkey UI
- Android uses `CredentialManager.getCredential`, which may present UI

This matters after registering a new Cloud Backup passkey. The Rust enable flow
must stage the saved-passkey confirmation after the short save-settle delay
before authenticating again.

For manual confirmation, the explicit confirmation action is
`ConfirmSavedPasskey`. It reuses the staged credential ID and performs targeted
PRF authentication only after the user taps `Continue`.

For automatic confirmation, Rust owns the continuation after the same settle
delay. UI layers should render the loading state and should not dispatch
`ConfirmSavedPasskey` on behalf of the user.

Do not reintroduce post-registration presence polling to decide whether the new
passkey has become available. That polling can cause the system passkey sheet to
appear and dismiss immediately after creation.
