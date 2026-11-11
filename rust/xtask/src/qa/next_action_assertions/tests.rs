use super::{check_source, is_cloud_backup_ui_test};

const SWIFT: &str = "ios/CoveUITests/CloudBackupExampleFullAppUITests.swift";

fn swift(body: &str) -> Vec<String> {
    check_source(
        SWIFT,
        &format!("import XCTest\n\nfinal class Example: XCTestCase {{\n{body}\n}}\n"),
    )
}

#[test]
fn test_methods_that_call_the_helper_pass() {
    let errors = swift(
        r#"
    func testTurnOff() throws {
        let control = try open()
        if control.isReady {
            control.assertNextActions("Turn Off is offered", available: ["turnOff"])
        }
    }

    func testScenario() {
        scenarioState.assertNextActions (
            "a scenario test reads its state file"
        )
    }

    private func open() throws -> CloudBackupHostControl {
        try CloudBackupHostControl(script: .init())
    }
    "#,
    );

    assert_eq!(errors, Vec::<String>::new());
}

#[test]
fn a_test_without_the_call_fails_with_its_line() {
    let errors = swift(
        r#"
    func testCalls() {
        control.assertNextActions("offered")
    }

    func testLooksOnlyAtTheScreen() {
        XCTAssertTrue(app.buttons["cloudBackup.action.turnOff"].exists)
    }
    "#,
    );

    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(
        errors[0].starts_with(&format!("{SWIFT}:9: `testLooksOnlyAtTheScreen` never calls")),
        "{}",
        errors[0]
    );
}

#[test]
fn helpers_comments_strings_and_other_names_dont_count() {
    let errors = swift(
        r#"
    func testCallsOnlyAHelper() {
        waitForTurnedOff(control)
    }

    func testMentionsItInAComment() {
        // control.assertNextActions("offered")
        let message = "assertNextActions(is not called)"
        control.assertNextActionsLater("offered")
    }

    private func waitForTurnedOff(_ control: CloudBackupHostControl) {
        control.assertNextActions("off", body: "disabled")
    }
    "#,
    );

    assert_eq!(errors.len(), 2, "{errors:#?}");
    assert!(errors[0].contains("`testCallsOnlyAHelper`"), "{}", errors[0]);
    assert!(errors[1].contains("`testMentionsItInAComment`"), "{}", errors[1]);
}

#[test]
fn only_ios_cloud_backup_ui_test_classes_are_checked() {
    assert!(is_cloud_backup_ui_test("ios/CoveUITests/CloudBackupTurnOffFullAppUITests.swift"));
    assert!(!is_cloud_backup_ui_test("ios/CoveUITests/CloudBackupHostControl.swift"));
    assert!(!is_cloud_backup_ui_test("ios/CoveUITests/OnboardingFullLaunchUITests.swift"));
    assert!(!is_cloud_backup_ui_test("ios/CoveTests/CloudBackupScreenUITests.swift"));
    assert!(!is_cloud_backup_ui_test("ios/CoveUITests/nested/CloudBackupFullAppUITests.swift"));
}
