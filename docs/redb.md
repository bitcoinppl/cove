# redb Compatibility

This project stores non-sensitive app state in redb typed tables. redb table metadata is part of the durable schema, not an implementation detail.

## Why This Matters

redb records the key and value `TypeName` for every table when the table is created. Later `open_table` calls compare the stored metadata with the current `TableDefinition` key and value types. If either name changes, `open_table` returns `TableTypeMismatch`; startup code may panic if table creation uses `expect`.

The common trap is `std::any::type_name::<T>()`: it returns the module path where `T` is defined. A public re-export does not hide that path. Moving a persisted type from `database::foo` to `database::foo::state` can change the redb type name even if the Rust API still imports it as `database::foo::Type`.

## Compatibility Checklist

Before changing any redb table or persisted database type:

- Inspect the previous committed `TableDefinition` and any wrapper `Value::type_name()` implementation
- Write down the old and new key/value type names, including module path changes
- Check whether any released, beta, TestFlight, internal, or PR build could have opened or created the table
- Preserve the exact historical type name when possible with a custom `Value` wrapper
- If multiple builds could have written different metadata, add a compatibility path for every known metadata shape
- Add a regression test that opens a database/table written with each historical metadata shape

Local app startup is not enough evidence unless the same app container went through the relevant historical build chain without wiping data.

## Safe Patterns

Stable wrapper types are preferable for durable redb tables. If a persisted value type might move modules, implement a table-specific `Value` wrapper with an explicit, pinned `TypeName` instead of relying on `std::any::type_name::<T>()`.

When preserving compatibility after a refactor, keep the serialized bytes unchanged and only pin or bridge the redb metadata. If the serialized data shape changes too, handle that separately with explicit versioned serde compatibility.

## Regression Tests

For a table metadata change, prefer tests that:

- create a temp redb database with the old table definition
- insert at least one representative row
- reopen it through the current production table API
- repeat for every known historical metadata shape
- cover a fresh database so new installs still create the intended metadata

Manual device tests should install builds over the same app data in the same order users could have experienced. Testing a fresh install of only the current build does not cover redb metadata compatibility.
