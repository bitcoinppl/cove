# Android UI Parity Gaps

The following locations still carry over iOS-first styling instead of Material/Android idioms. Use them as a punch list when reworking the Android experience.

## Global theming

- `android/app/src/main/java/org/bitcoinppl/cove/ui/theme/Color.kt:5-74` – Palette is defined with iOS token names (`CoveColor.*`) and referenced directly from screens. This bypasses Material3 `colorScheme` roles and prevents dynamic color/theming.

## Shared components

- `android/app/src/main/java/org/bitcoinppl/cove/views/CustomBaseViews.kt:77-230` – Legacy `CustomSpacer`, `InfoRow`, `ClickableInfoRow`, and `CardItem` mimic SwiftUI grouped lists (hard-coded gray separators, inset cards, chevrons). Replace call sites with `MaterialSection`, `ListItem`, `Switch`, and `MaterialDivider`.
- `android/app/src/main/java/org/bitcoinppl/cove/views/SettingsItem.kt:136-187` – `SettingsItem` is deprecated but still consumed by detail screens. Migrate remaining usages to `MaterialSettingsItem` (Material3 `ListItem` implementation).

## Settings flow

- `android/app/src/main/java/org/bitcoinppl/cove/settings/WalletSettingsScreen.kt:120-235` plus `WalletSettingsScreen.kt:435-439` – Section layout relies on `CardItem`, `InfoRow`, and `ListSpacer` (which calls `CustomSpacer`). Needs `SectionHeader` + `MaterialSection` + `ListItem`/`Switch` rows with proper `MaterialDivider`.
- `android/app/src/main/java/org/bitcoinppl/cove/settings/FiatCurrencySettingsScreen.kt:59-111` – Still uses `CardItem`/`CustomSpacer` for the list of currencies; move to `SectionHeader` + `MaterialSection` with `ListItem` rows.
- `android/app/src/main/java/org/bitcoinppl/cove/settings/NetworkSettingsScreen.kt:58-137` – Similar inset-card layout and centered title. Adopt Material start-aligned `TopAppBar`, `MaterialSection`, and `ListItem` rows (the scaffolding already exists in `SettingsScreen.kt`).
- `android/app/src/main/java/org/bitcoinppl/cove/settings/NodeSettingsScreen.kt:217-355` – Both preset list and custom form use `CardItem` for grouping, `CustomSpacer` for dividers, and a centered `TopAppBar`. Rework with Material containers, `LazyColumn`, and standard input spacing.
- `android/app/src/main/java/org/bitcoinppl/cove/settings/AppearanceSettingsScreen.kt:58-135` – Uses improved `SectionHeader`/`MaterialSection`, but still centers the title and could leverage `ListItem` rows to get default typography/ripple.

## Wallet list, send, and new-wallet flows

- `android/app/src/main/java/org/bitcoinppl/cove/wallet_transactions/WalletTransactionsScreen.kt:139-240` – Top app bar is center-aligned, action icons mimic iOS. Content overlays `ListBackgroundLight/Dark` surfaces from `CoveColor`, blocking the decorative pattern. Adopt standard `TopAppBar`, Material surfaces with tonal elevation, and `ListItem` rows so typography and ripples match Android norms.
- `android/app/src/main/java/org/bitcoinppl/cove/send/SendScreen.kt:94-205` – Same centered bar and opaque, iOS-colored surfaces; text sizes are hard-coded instead of referencing `MaterialTheme.typography`.
- `android/app/src/main/java/org/bitcoinppl/cove/flow/new_wallet/NewWalletSelectScreen.kt:164-320` – Midnight-blue scaffold plus inset card layout hides the chain pattern and uses bespoke typography. Swap to Material background colors, let the pattern show through via translucent surfaces, and rely on `MaterialTheme.typography` for headings/buttons.

## Patterns that need Android treatment

- **Centered `TopAppBar` everywhere** (`wallet_transactions`, `send`, `settings`, `new wallet`, etc.). Replace with default small/medium/large `TopAppBar` variants that left-align titles unless a center-aligned bar is truly warranted.
- **Opaque overlays over the chain-code pattern** (`wallet_transactions`, `send`, `new wallet`). Use `Surface` and `colorScheme.surfaceContainer` with transparency/tonal elevation so the pattern is visible without harming contrast.
- **Manual typography and icon colors** – E.g. `wallet_transactions/WalletTransactionsScreen.kt:225-334` sets font sizes/weights and colors by hand. Switching to `MaterialTheme.typography` and `colorScheme` ensures platform-consistent type ramp and dynamic color support.

Addressing these files/components will get the Android UI off the iOS-inspired path and onto idiomatic Material 3 patterns while still sharing the Rust-backed manager architecture.
