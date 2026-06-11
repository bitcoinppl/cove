# Passkeys

Notes on passkey behavior that matters for Cloud Backup flows.

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
