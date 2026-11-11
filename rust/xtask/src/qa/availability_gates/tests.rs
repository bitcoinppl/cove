use super::check_source;

const KOTLIN: &str = "android/app/src/main/java/org/bitcoinppl/cove/cloudbackup/Section.kt";
const SWIFT: &str = "ios/Cove/Flows/SettingsFlow/CloudBackupSection.swift";

fn kotlin(body: &str) -> Vec<String> {
    check_source(KOTLIN, &format!("@Composable\nfun Section() {{\n{body}\n}}\n"))
}

#[test]
fn kotlin_gates_that_read_an_availability_pass() {
    let errors = kotlin(
        r#"
        Button(onClick = {}, enabled = availability.isAvailable) {}
        TextButton(
            onClick = {},
            enabled = screen.actions.lookup(CloudBackupScreenAction.VERIFY)?.isAvailable == true,
        ) {}
        Row(modifier = Modifier.clickable(enabled = row.actions.any { it.availability.isAvailable }))
        val enabled = action.availability.isAvailable
        WalletItemRow(enabled = enabled)
        Card(retryEnabled = retryEnabled)
        "#,
    );

    assert_eq!(errors, Vec::<String>::new());
}

#[test]
fn kotlin_gates_on_derived_state_or_literals_fail_with_their_line() {
    let errors = kotlin(
        r#"
        Button(onClick = {}, enabled = manager.isDetailInventoryComplete) {}
        Button(
            onClick = {},
            enabled =
                !isBusy &&
                    availability.isAvailable,
        ) {}
        Dialog(verifyEnabled = false)
        val rowEnabled = row.wallet != null
        "#,
    );

    assert_eq!(errors.len(), 4, "{errors:#?}");
    assert!(errors[0]
        .starts_with(&format!("{KOTLIN}:4: `enabled = manager.isDetailInventoryComplete`")));
    assert!(errors[1].contains("`enabled = !isBusy && availability.isAvailable`"), "{}", errors[1]);
    assert!(errors[2].contains("`verifyEnabled = false`"), "{}", errors[2]);
    assert!(errors[3].contains("`rowEnabled = row.wallet != null`"), "{}", errors[3]);
}

#[test]
fn kotlin_ignores_parameters_comments_strings_and_files_without_composables() {
    let errors = kotlin(
        r#"
        fun Row(enabled: Boolean = true, retryEnabled: Boolean = false) {}
        // enabled = manager.isBusy
        Text("enabled = manager.isBusy")
        if (enabled == isReady) {}
        "#,
    );
    assert_eq!(errors, Vec::<String>::new());

    let manager = "class Manager {\n    fun refresh() {\n        isCloudBackupEnabled = rust.read()\n    }\n}\n";
    assert_eq!(check_source(KOTLIN, manager), Vec::<String>::new());
}

#[test]
fn swift_disabled_modifiers_must_read_an_availability() {
    let source = r#"
    Button("Verify") {}
        .disabled(!manager.screen.actions.isAvailable(.verify))
    Button("Delete") {}
        .disabled(isOperating || !manager.isDetailInventoryComplete)
    "#;

    let errors = check_source(SWIFT, source);

    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(errors[0].starts_with(&format!(
        "{SWIFT}:5: `.disabled(isOperating || !manager.isDetailInventoryComplete)`"
    )));
}

#[test]
fn swift_skips_enum_cases_named_disabled() {
    let source = r#"
    func action() -> Action {
        return .disabled(title: "Restore All")
    }
    switch action {
    case let .disabled(title): Text(title)
    }
    "#;

    assert_eq!(check_source(SWIFT, source), Vec::<String>::new());
}

#[test]
fn only_cloud_backup_views_are_checked() {
    let source = "@Composable\nfun Other() {\n    Button(enabled = isBusy) {}\n}\n";

    assert_eq!(
        check_source(
            "android/app/src/main/java/org/bitcoinppl/cove/flows/SendFlow/Send.kt",
            source
        ),
        Vec::<String>::new(),
    );
    assert_eq!(
        check_source("ios/Cove/Flows/SendFlow/SendView.swift", ".disabled(isBusy)"),
        Vec::<String>::new(),
    );
}

#[test]
fn allowed_flags_pass_only_in_the_view_that_justifies_them() {
    let enable = "@Composable\nfun Enable() {\n    Button(enabled = allChecked && primary?.isAvailable == true) {}\n}\n";
    let enable_view = "android/app/src/main/java/org/bitcoinppl/cove/cloudbackup/CloudBackupEnableOnboardingContent.kt";

    assert_eq!(check_source(enable_view, enable), Vec::<String>::new());
    assert_eq!(check_source(KOTLIN, enable).len(), 1);

    let cancel = "Button(\"Cancel\") {}\n    .disabled(isBusy)\n";
    let sections = "ios/Cove/Flows/SettingsFlow/CloudBackupEnableOnboardingSections.swift";

    assert_eq!(check_source(sections, cancel), Vec::<String>::new());
    assert_eq!(check_source(SWIFT, cancel).len(), 1);
}
