The role of this file is to describe common mistakes and confusion points that agents might encounter as they work in this project. If you ever encounter something in the project that surprises you, please alert the developer working with you and indicate that this is the case in the AGENTS.md file to help prevent future agents from having the same issue.

## Hard Rules

- Never add `#[cfg(test)]` to production items; test-only helpers must live inside `mod tests` or dedicated `test_support` modules

## Rust Rules

- Use `cove_util::ResultExt::map_err_str` instead of `.map_err(|e| Error::Variant(e.to_string()))` — it's cleaner and equivalent
- Use `cove_util::ResultExt::map_err_prefix` instead of `.map_err(|e| Error::Variant(format!("context: {e}")))` when the prefix is a static string — produces `"context: error_message"`
- Prefer `From` implementations for error conversions whenever possible, and avoid standalone conversion functions when `From` would do
- Read [ARCHITECTURE.md](ARCHITECTURE.md) before changing Rust actors, async manager methods, worker tasks, Rust closure-based orchestration, reconciliation, shared state, locks, dispatch, or UniFFI manager boundaries
- For topic-specific guidance (passkeys, iCloud Drive, iOS/Android parity), read the docs linked from [ARCHITECTURE.md](ARCHITECTURE.md)
- Before changing redb `TableDefinition`s, redb `Value::type_name()` implementations, persisted database structs/enums, or module paths containing persisted types, read [docs/redb.md](docs/redb.md) and verify old and new table metadata compatibility
- redb compatibility must account for every build that could have opened the table, including short-lived internal/TestFlight builds; local startup success is not enough unless the same app data went through the relevant historical build chain
- prefer structurally correct fixes over temporary workarounds, even when the diff is larger
- Make impossible states impossible; prefer typed domain models over caller-specific conditionals or UI-side compensation
- For long-lived UI-facing managers, prefer `dispatch(action:)` for user intents and keep named methods for reads, bootstrap/lifecycle hooks, and special service-style operations, use `state()` for the initial snapshot only, and send typed delta reconcile messages for ongoing UI updates instead of re-sending the whole state after every mutation
- Prefer scoped blocks to release locks or borrows instead of explicit `drop(...)` unless explicit `drop` is actually needed
- Add blank lines between logical steps in function bodies across Rust, Swift, and Kotlin: after setup/result bindings before new control flow, after multi-line `if`/`match`/`guard`/`do`/`Task` blocks before the next independent statement, between state mutations and final returns, and between multi-line `match`/`switch` arms. Keep tightly related consecutive assignments together
- Put explanatory comments immediately above the statement or arm they describe, separated from the previous step by a blank line
- never use `pub(in ...)` or `pub(super)`; if non-private visibility is needed, use `pub(crate)` or `pub`
- never manually edit generated files
- for local UniFFI updates use `just build-ios`/`just build-android`; `just rb` runs GitHub Actions for committed branch changes
- no mod.rs files use the other format module_name.rs module_name/new_module.rs
- generated UniFFI Kotlin enum readers have a tight ordinal contract with Rust enum variant order, for example `AppAction` ordinals in the generated `FfiConverterTypeAppAction` reader; never reorder, insert, or remove Rust variants without regenerating bindings and updating the generated checksum, because stale generated files can map ordinals like `SelectWallet`, `SelectLatestOrNewWallet`, and `ChangeNetwork` to the wrong action
- don't be afraid to change uniffi bindings and api shape
- data structures and UniFFI-derived Rust types may change when they directly serve the requested work; update generated bindings and affected Swift/Kotlin call sites when exported APIs change
- Don't mix test-only code into production code; `#[cfg(test)]` helpers should live in `mod tests` or dedicated `test_support` modules
