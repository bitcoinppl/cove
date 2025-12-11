# Contributing to Cove

## Prerequisites

- [Rust](https://rustup.rs)
- [Just](https://github.com/casey/just) (`cargo install just`)
- **iOS**: Xcode 16.0+, swiftformat
- **Android**: Android Studio + NDK, ktlint

## Quick Start

1. Clone the repository
2. Build the Rust library and bindings:
   - iOS: `just build-ios` (simulator) or `just bidd` (device)
   - Android: `just build-android`
3. Open in Xcode (`ios/Cove.xcodeproj`) or Android Studio (`android/`)
4. Build and run

## Release Builds

### iOS

```bash
just build-ios-release    # or just bir
```

Then archive and distribute via Xcode (Product → Archive).

### Android

```bash
just build-android-release    # or just bar
```

Then build a signed APK/AAB via Android Studio (Build → Generate Signed Bundle/APK).

## Development Workflow

### Iterative Development

- **Rust-only changes**: Use `just bacon` or `just bcheck` for continuous feedback
- **UI changes (no Rust API changes)**: Use `just compile-ios` or `just compile-android` for faster iteration
- **Rust API changes**: Run `just build-ios` or `just build-android` to regenerate bindings
- **Tests**: Run `just wtest` in a separate terminal for continuous test feedback

### Common Commands

| Command | Description |
|---------|-------------|
| `just ba` | Build Android debug |
| `just bi` | Build iOS debug simulator |
| `just bidd` | Build iOS debug device |
| `just test` | Run test suite |
| `just wtest` | Watch mode tests |
| `just fmt` | Format all code (Rust, Swift, Kotlin) |
| `just ci` | Run all CI checks |
| `just clean` | Full cleanup of build artifacts |

Run `just` to see all available commands.

### Debugging Tips

- **iOS builds stuck?** Try `just xcode-reset` to clear Xcode caches
- **Clean slate needed?** Run `just clean` to remove all build artifacts
- **UniFFI binding issues?** Regenerate bindings after changing Rust exports
- **Actor not receiving messages?** Verify `FfiApp::init_on_start` is called during app startup

## Before Committing

1. Run `just fmt` to format all code
2. Run `just ci` to execute all checks (format, lint, clippy, tests, compilation)
3. Fix any issues reported by CI checks
4. If clippy reports warnings, run `just fix` first to auto-fix what's possible

## Commit Messages

Write clear, concise commit messages following these guidelines:

- **Use imperative mood**: "Add feature" not "Added feature"
- **Limit subject to 50 chars**: Be concise, this is the title
- **Capitalize the subject line**
- **No period at the end of the subject**
- **Separate subject from body with a blank line**
- **Wrap body at 72 chars**
- **Explain what and why, not how**: The code shows how

Example:
```
Add UTXO locking for coin control

Prevent selected UTXOs from being spent by other transactions
while a send flow is in progress. This avoids conflicts when
the user is manually selecting coins.
```

A good subject line completes: "If applied, this commit will ___"

See [How to Write a Git Commit Message](https://cbea.ms/git-commit/) for the full guide.

## Further Reading

- [ARCHITECTURE.md](ARCHITECTURE.md) - System design, Rust core, UniFFI, mobile patterns
- [docs/IOS_ANDROID_PARITY.md](docs/IOS_ANDROID_PARITY.md) - iOS/Android UI parity patterns
