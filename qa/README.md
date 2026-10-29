# Release QA checklist manifest

`checklist/*.toml` holds one row per item of the manual release checklist, one file per checklist section. Each row records what currently decides it, so a release only sends an agent or a person to the rows nothing cheaper can cover.

## Commands

- `just qa-plan <git-range>` lists the rows whose `paths` the range touches and that no automated test decides, grouped into agent and human rows. The output is the whole brief for an agent pass on that change. The range goes straight to `git diff --name-only --no-renames`, so `master...HEAD` and `v1.4.0..HEAD` both work, and a moved file counts under both its old and new path. Rows with `pending` tests also list the still-manual platforms those tests would take over
- `just qa-plan-all` lists every row that still needs an agent or a human, for a full release pass
- `just qa-coverage` counts rows per platform by the cheapest layer that covers them, then by their target layer
- `just qa-check` fails when a row names a file that doesn't exist or a test that the file doesn't declare as a test outside comments, uses a path glob that matches no tracked file, lists a Rust module file such as `foo.rs` without any of the files under `foo/`, or leaves one of its platforms without coverage. CI runs it on every pull request

## Driving devices

An agent pass drives Android with [`tools/uia2`](../tools/uia2/README.md) and iOS with [`tools/wda`](../tools/wda/README.md): read the screen with `android-vis` or `ios-vis`, act with `android-tap-label`, `ios-tap-label` and the other commands, and take screenshots only for visual checks.

## Row format

```toml
[[row]]
id = "s10-empty-input-replaces-saved-custom-explorer"
section = "10"
title = "Empty input replaces saved custom explorer"
platforms = ["ios", "android"]
target = "rust"
paths = ["rust/src/database/global_config.rs", "ios/Cove/Flows/SettingsFlow/**"]
coverage = ["rust:rust/src/database/global_config.rs::custom_block_explorer_rejects_empty_input"]
pending = []
pending_pr = 924
```

- `platforms`: the platforms the item applies to
- `target`: the cheapest layer that could eventually decide the row on its own: `rust`, `ui` (simulator or emulator test), `device` (a plain script on a real device) or `manual`
- `paths`: repo-relative globs of the code the row depends on. `**` matches across directories and `*` stays within one. Rows with no paths only show up in `qa-plan-all`
- `coverage`: everything that checks the row today. An `agent` or `human` entry means the row still needs one on that platform, and any automated entries next to it cover only part of the row. Without one, the cheapest automated entry decides the platform
  - `rust:<file>::<fn>` applies to both platforms
  - `android-unit:<file>::<fun>` and `android-instrumented:<file>::<fun>` apply to Android
  - `ios-unit:<file>::<func>` and `ios-ui:<file>::<func>` apply to iOS
  - The named function must carry its runner's marker: a Rust attribute containing `test` such as `#[test]` or `#[tokio::test]`, Kotlin `@Test`, or a Swift `test` name prefix or `@Test`
  - `device-script:<script>::<row id>` applies to the platform directory the script lives in, `ios/` or `android/`; the script must contain the row id as a quoted string outside `#` comment lines. The scripts, how to run them and their result format are in [device/README.md](device/README.md)
  - `agent` and `human` apply to both platforms, or to one with `agent:ios` or `human:android`
- `pending` and `pending_pr`: tests on an open pull request that will decide the row once it merges. `qa-check` doesn't fail on them, and warns once one exists so it can move into `coverage`

Only list an automated test when it would fail if the row's behavior broke. A test that merely exercises the same screen belongs in a code review, not in `coverage`.
