# Contributing to Cove

## Philosophy

Cove is a simple, intuitive, but powerful Bitcoin wallet. The goal is to help new users get up and running quickly while still supporting the features power users need, such as hardware wallet support.

That balance matters whenever adding new functionality. Every feature must earn its place by remaining simple and intuitive. If we cannot make a feature feel simple and intuitive, we probably should not add it.

## Prerequisites

- [Rust](https://rustup.rs)
- [Just](https://github.com/casey/just) (`cargo install just`)
- `cargo-nextest` (`cargo install cargo-nextest`)
- **iOS**: Xcode 16.0+, swiftformat
- **Android**: Android Studio + NDK, Java 17/JDK
- **Optional**: bacon, watchexec

## Environment Setup

Copy `.envrc.example` to your preferred local environment setup and load the Android variables before running Android builds. At minimum, make sure `ANDROID_HOME`, `ANDROID_SDK_ROOT`, `ANDROID_NDK_HOME`, and `JAVA_HOME` are set correctly for your machine.

The `COVE_KEYSTORE_*` variables in `.envrc.example` are only needed for signed Android release builds and bundles. Regular development work does not require them.

## Quick Start

1. Clone the repository
2. Build the Rust library and bindings:
   - iOS: `just build-ios` (`just bi`) for simulator or `just build-ios-debug-device` (`just bidd`) for device
   - Android: `just build-android` (`just ba`)
3. Open in Xcode (`ios/Cove.xcodeproj`) or Android Studio (`android/`)
4. Build and run

## Release Builds

### iOS

```bash
just build-ios-release    # alias: just bir
```

Then archive and distribute via Xcode (Product → Archive).

### Android

```bash
just build-android-release    # alias: just bar
```

Then build a signed APK/AAB via Android Studio (Build → Generate Signed Bundle/APK).

## Development Workflow

### Iterative Development

- **Rust-only changes**: Use `just bacon` or `just bcheck` for continuous feedback
- **UI changes (no Rust API changes)**: Use `just compile-ios` or `just compile-android` for faster iteration
- **Rust API or UniFFI changes**: Run `just build-ios` or `just build-android` to rebuild Rust and regenerate bindings
- **Tests**: Run `just watch-test` (`just wtest`) in a separate terminal for continuous test feedback

`just build-ios` and `just build-android` rebuild the Rust core, regenerate UniFFI bindings, and update the mobile projects. `just compile-ios` and `just compile-android` only rebuild the native apps, so use them when Rust exports have not changed.

### Common Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `just build-android` | `just ba` | Build Android debug |
| `just build-android-release` | `just bar` | Build Android release |
| `just build-ios` | `just bi` | Build iOS debug simulator |
| `just build-ios-debug-device` | `just bidd` | Build iOS debug device |
| `just build-ios-release` | `just bir` | Build iOS release |
| `just compile-ios` | - | Compile iOS without regenerating bindings |
| `just compile-android` | - | Compile Android without regenerating bindings |
| `just test` | - | Run the Rust test suite with nextest |
| `just watch-test` | `just wtest` | Watch and re-run tests on Rust file changes |
| `just fmt` | - | Format Rust, Swift, and Android code |
| `just ci` | - | Run format, lint, compile, and test checks |
| `just clean` | - | Remove build artifacts |

Run `just` to see the public recipes. Aliases are shortcuts for commands you use often.

### Generated Code

- Do not manually edit generated UniFFI bindings
- Regenerate bindings with `just build-ios` or `just build-android` after changing exported Rust APIs
- Use `just compile-ios` and `just compile-android` only when Rust exports have not changed

### Debugging Tips

- **iOS builds stuck?** Try `just xcode-reset` to clear Xcode caches
- **Clean slate needed?** Run `just clean` to remove all build artifacts
- **UniFFI binding issues?** Regenerate bindings after changing Rust exports

## Before Committing

1. Run `just fmt` to format all code
2. Run `just ci` to execute all checks (format, lint, clippy, tests, compilation)
3. Fix any issues reported by CI checks
4. If clippy reports warnings, run `just fix` first to auto-fix what's possible
5. If you changed Rust exports that generate bindings, run `just build-ios` and `just build-android` before committing
6. Merge the latest `master` into your branch if `master` has changed since you started your work

## Commit Messages

Write clear, concise commit messages that explain what changed and why. Let the code describe how.

Helpful defaults:

- **Use imperative mood**: "Add feature" not "Added feature"
- **Capitalize the subject line**
- **No period at the end of the subject**
- **Add a body when it helps explain context or motivation**

Example:
```
Add UTXO locking for coin control

Prevent selected UTXOs from being spent by other transactions
while a send flow is in progress. This avoids conflicts when
the user is manually selecting coins.
```

A good subject line completes: "If applied, this commit will ___"

See [How to Write a Git Commit Message](https://cbea.ms/git-commit/) for the full guide.

## Pull Requests

- If you are addressing review feedback, add follow-up commits instead of squashing so reviewers can easily see what changed since the last review
- Merge the latest `master` into your branch when needed instead of rebasing. We squash commits when the pull request is merged
- If changes were requested on your pull request and you addressed them, request review again
- If you do not get a review within two days, ping Praveen on Discord or tag him in the GitHub pull request

## Further Reading

- [ARCHITECTURE.md](ARCHITECTURE.md) - System design, Rust core, UniFFI, mobile patterns
- [docs/IOS_ANDROID_PARITY.md](docs/IOS_ANDROID_PARITY.md) - iOS/Android UI parity patterns
