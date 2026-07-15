## Rust Rules

### Required Context

- Read [ARCHITECTURE.md](ARCHITECTURE.md) before changing Rust actors, async manager methods, worker tasks, Rust closure-based orchestration, reconciliation, shared state, locks, dispatch, or UniFFI manager boundaries
- For topic-specific guidance such as passkeys, iCloud Drive, or iOS/Android parity, read the docs linked from [ARCHITECTURE.md](ARCHITECTURE.md)
- Before changing redb `TableDefinition`s, redb `Value::type_name()` implementations, persisted database structs or enums, or module paths containing persisted types, read [docs/redb.md](docs/redb.md) and verify old and new table metadata compatibility
- Before changing Android manager ownership, generated UniFFI `.rust` access, `close()`, or route-level `DisposableEffect` cleanup, read the Mobile Frontends manager ownership guidance in [ARCHITECTURE.md](ARCHITECTURE.md) and the lifecycle notes in [docs/ios_android_parity.md](docs/ios_android_parity.md)

### Architecture and APIs

- Model invariants in typed domain types and proper owners rather than caller-specific conditionals, UI-side compensation, or temporary workarounds
- Make required UniFFI and data-structure changes instead of narrowing scope to avoid them; update generated bindings and affected Swift/Kotlin call sites when exported APIs change
- Prefer `From` implementations for error conversions whenever possible, and avoid standalone conversion functions when `From` would do
- Android generated UniFFI manager handles stay private to platform managers; constructors that receive them should be internal, and Rust access should go through wrapper methods backed by the shared `RustHandleGuard`
- For long-lived UI-facing managers:
  - Prefer `dispatch(action:)` for user intents
  - Keep named methods for reads, bootstrap and lifecycle hooks, and special service-style operations
  - Use `state()` only for the initial snapshot
  - Send typed delta reconcile messages for ongoing UI updates instead of re-sending the whole state after every mutation
- Prefer scoped blocks to release locks or borrows instead of explicit `drop(...)` unless explicit `drop` is actually needed
- Never use `pub(in ...)` or `pub(super)`; if non-private visibility is needed, use `pub(crate)` or `pub`
- Never manually edit generated files
- Use `cove_util::ResultExt::map_err_str` and `.map_err_prefix` instead of `.map_err(|e| Error::Variant(e.to_string()))` and `.map_err(|e| Error::Variant(format!("context: {e}")))`
- Do not use `mod.rs`; use `module_name.rs` and `module_name/new_module.rs`

### Formatting

- Put explanatory comments immediately above the statement or arm they describe, separated from the previous step by a blank line
- Add blank lines between logical steps in Rust, Swift, and Kotlin function bodies:
  - After setup or result bindings before new control flow
  - After multi-line `if`, `match`, `guard`, `do`, or `Task` blocks before the next independent statement
  - Between state mutations and final returns
  - Between multi-line `match` or `switch` arms
- Keep tightly related consecutive assignments together

### Verification and Side Effects

- After local UniFFI changes, run `just build-ios` for affected iOS bindings and call sites and `just build-android` for affected Android bindings and call sites; run both when the exported API affects both platforms
- To install and launch iOS from the CLI without Xcode, use `just build-run-ios --udid <device-udid>` and keep the phone unlocked
- Create commits, push branches, or install and launch applications on devices only when the user request authorizes the action
