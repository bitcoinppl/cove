## Rust Rules

- Read [ARCHITECTURE.md](ARCHITECTURE.md) before changing Rust actors, async manager methods, worker tasks, Rust closure-based orchestration, reconciliation, shared state, locks, dispatch, or UniFFI manager boundaries
- prefer `From` implementations for error conversions whenever possible, and avoid standalone conversion functions when `From` would do
- for topic-specific guidance (passkeys, iCloud Drive, iOS/Android parity), read the docs linked from [ARCHITECTURE.md](ARCHITECTURE.md)
- Before changing redb `TableDefinition`s, redb `Value::type_name()` implementations, persisted database structs/enums, or module paths containing persisted types, read [docs/redb.md](docs/redb.md) and verify old and new table metadata compatibility
- prefer structurally correct fixes over temporary workarounds, even when the diff is larger
- before changing Android manager ownership, generated UniFFI `.rust` access, `close()`, or route-level `DisposableEffect` cleanup, read the Mobile Frontends manager ownership guidance in [ARCHITECTURE.md](ARCHITECTURE.md) and the lifecycle notes in [docs/ios_android_parity.md](docs/ios_android_parity.md)
- make impossible states impossible; prefer typed domain models over caller-specific conditionals or UI-side compensation
- for long-lived UI-facing managers, prefer `dispatch(action:)` for user intents and keep named methods for reads, bootstrap/lifecycle hooks, and special service-style operations, use `state()` for the initial snapshot only, and send typed delta reconcile messages for ongoing UI updates instead of re-sending the whole state after every mutation
- prefer scoped blocks to release locks or borrows instead of explicit `drop(...)` unless explicit `drop` is actually needed
- put explanatory comments immediately above the statement or arm they describe, separated from the previous step by a blank line
- never use `pub(in ...)` or `pub(super)`; if non-private visibility is needed, use `pub(crate)` or `pub`
- never manually edit generated files
- use `cove_util::ResultExt::map_err_str` and `..map_err_prefix` instead of `.map_err(|e| Error::Variant(e.to_string()))`, and `.map_err(|e| Error::Variant(format!("context: {e}")))`
- for local UniFFI updates use `just build-ios`/`just build-android`; `just rb` runs GitHub Actions for committed branch changes
- no mod.rs files use the other format module_name.rs module_name/new_module.rs
- data structures and UniFFI-derived Rust types may change when they directly serve the requested work; update generated bindings and affected Swift/Kotlin call sites when exported APIs change
- don't mix test-only code into production code; `#[cfg(test)]` helpers should live in `mod tests` or dedicated `test_support` modules
- add blank lines between logical steps in function bodies across Rust, Swift, and Kotlin: after setup/result bindings before new control flow, after multi-line `if`/`match`/`guard`/`do`/`Task` blocks before the next independent statement, between state mutations and final returns, and between multi-line `match`/`switch` arms. Keep tightly related consecutive assignments together
