## Summary

<!-- describe what changed and why -->

## Testing

<!-- list the commands you ran or explain why testing was not needed -->

### Platform Coverage

- [ ] Tested on iOS device
- [ ] Tested on Android device
- [ ] Tested on iOS simulator
- [ ] Tested on Android simulator
- [ ] Not tested

## Cloud Backup

<!-- only for changes under rust/src/manager/cloud_backup_manager or the Cloud Backup screens; delete otherwise. See docs/cloud_backup.md for the ownership and write-policy tables -->

- **No band-aids:** does any change compensate for a wrong state in a caller, the UI or a retry instead of fixing the owner? If so, why is the owner right as it is?
- **New writers:** does anything new write the lifecycle record, the runtime, the keychain namespace or a cloud namespace? Name its owner in the ownership table.
- **New early returns, flags, timers, gates or bypasses:** list each one, with the state it leaves and how that state ends.
- **New platform gates:** does Swift or Kotlin now decide something Rust owns?
- **States without a way out:** can any new state be reached with no next action that leaves it?
- **Next actions:** do the sequence tests assert `then_can` and `then_cannot` at every new step?
- **Invariants:** which harness invariants did this turn on or change, and how was each shown to fail when its guard is removed?
- **Deletion ledger:** for every test deleted or renamed, what protected that behavior before and what protects it now, and which `qa/checklist` rows reference it.
- **iOS UI test output:** paste the output of `just ios-ui-background "iPhone 17" CoveCloudBackupUITests/<Class>` for each class touched.

## Checklist

- [ ] I have read [CONTRIBUTING.md](https://github.com/bitcoinppl/cove/blob/master/CONTRIBUTING.md)
- [ ] I have read [ARCHITECTURE.md](https://github.com/bitcoinppl/cove/blob/master/ARCHITECTURE.md)
