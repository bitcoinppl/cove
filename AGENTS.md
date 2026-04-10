The role of this file is to describe common mistakes and confusion points that agents might encounter as they work in this project. If you ever encounter something in the project that surprises you, please alert the developer working with you and indicate that this is the case in the AGENTS.md file to help prevent future agents from having the same issue.

- Use `cove_util::ResultExt::map_err_str` instead of `.map_err(|e| Error::Variant(e.to_string()))` — it's cleaner and equivalent
- Use `cove_util::ResultExt::map_err_prefix` instead of `.map_err(|e| Error::Variant(format!("context: {e}")))` when the prefix is a static string — produces `"context: error_message"`
- For long-lived UI-facing managers, prefer `dispatch(action:)` for user intents and keep named methods for reads, bootstrap/lifecycle hooks, and special service-style operations, use `state()` for the initial snapshot only, and send typed delta reconcile messages for ongoing UI updates instead of re-sending the whole state after every mutation
- Prefer scoped blocks to release locks or borrows instead of explicit `drop(...)` unless explicit `drop` is actually needed
- never use `pub(in ...)` or `pub(super)`; if non-private visibility is needed, use `pub(crate)` or `pub`
- never manually edit generated files
- no mod.rs files use the other format module_name.rs module_name/new_module.rs
