use super::{Coverage, Layer, Manifest, Platform};

fn manifest(source: &str) -> Manifest {
    Manifest::parse(&[("qa/checklist/test.toml".to_string(), source.to_string())]).unwrap()
}

const ROWS: &str = r#"
[[row]]
id = "rust-only"
section = "10"
title = "Explorer template"
platforms = ["ios", "android"]
target = "rust"
paths = ["rust/src/explorer.rs"]
coverage = ["rust:rust/src/explorer.rs::template_test"]

[[row]]
id = "ios-unit-only"
section = "16"
title = "Send header at max font"
platforms = ["ios", "android"]
target = "ui"
paths = ["ios/Cove/Flows/SendFlow/**", "android/app/src/main/java/**/SendFlow/**"]
coverage = ["ios-unit:ios/CoveTests/SendTests.swift::testHeader", "agent:android"]

[[row]]
id = "nfc"
section = "9"
title = "NFC import"
platforms = ["ios", "android"]
target = "manual"
paths = ["rust/src/nfc/**"]
coverage = ["rust:rust/src/nfc/parse.rs::nfc_payload_parses", "human:ios", "agent:android"]
pending = ["rust:rust/src/nfc/tests.rs::nfc_test"]
pending_pr = 924
"#;

#[test]
fn plan_lists_only_touched_rows_and_uncovered_platforms() {
    let manifest = manifest(ROWS);
    let changed = vec![
        "ios/Cove/Flows/SendFlow/SendHeader.swift".to_string(),
        "rust/src/explorer.rs".to_string(),
    ];

    let plan = manifest.plan(Some(&changed), Some("master...HEAD")).unwrap();

    assert!(plan.contains("1 touched rows are decided by automated tests"), "{plan}");
    assert!(plan.contains("## Agent (1 rows)"), "{plan}");
    assert!(plan.contains("`ios-unit-only` Send header at max font (android)"), "{plan}");
    assert!(!plan.contains("`nfc`"), "{plan}");
    assert!(!plan.contains("`rust-only`"), "{plan}");
}

#[test]
fn full_plan_splits_platforms_between_agent_and_human() {
    let plan = manifest(ROWS).plan(None, None).unwrap();

    assert!(plan.contains("## Human (1 rows)\n\n- §9 `nfc` NFC import (ios)"), "{plan}");
    assert!(plan.contains("- §9 `nfc` NFC import (android)"), "{plan}");
    assert!(
        plan.contains("## Covered once open PRs land\n\n- `nfc` NFC import (ios, android) #924"),
        "{plan}"
    );
}

#[test]
fn pending_rows_list_only_the_manual_platforms_they_would_cover() {
    let plan = manifest(
        r#"
[[row]]
id = "offline"
section = "14"
title = "Offline scan failure"
platforms = ["ios", "android"]
target = "ui"
coverage = ["agent"]
pending = ["android-instrumented:android/OfflineTest.kt::offlineScanFails"]
pending_pr = 930
"#,
    )
    .plan(None, None)
    .unwrap();

    assert!(plan.contains("- `offline` Offline scan failure (android) #930"), "{plan}");
}

#[test]
fn coverage_counts_each_platform_by_its_best_layer() {
    let report = manifest(ROWS).coverage_report();

    assert!(report.contains("| rust | 1 | 1 |"), "{report}");
    assert!(report.contains("| ios-unit | 1 | 0 |"), "{report}");
    assert!(report.contains("| agent | 0 | 2 |"), "{report}");
    assert!(report.contains("| human | 1 | 0 |"), "{report}");
    assert!(report.contains("| **automated** | 2 | 1 |"), "{report}");
}

#[test]
fn check_reports_missing_tests_paths_and_platforms() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("rust/src")).unwrap();
    std::fs::write(
        root.path().join("rust/src/explorer.rs"),
        r##"
#[tokio::test(flavor = "multi_thread")]
async fn template_test() {
    let url = "https://example.com/*";
}

// #[test]
// fn commented_test() {}

/* #[test]
   fn block_test() {} */

fn helper_test() {}

#[test]
fn fixture_test() {
    let _ = r#"#[test] fn quoted_test() {}"#;
}
"##,
    )
    .unwrap();

    let manifest = manifest(
        r#"
[[row]]
id = "ok"
section = "10"
title = "ok"
platforms = ["ios", "android"]
target = "rust"
paths = ["rust/src/explorer.rs"]
coverage = ["rust:rust/src/explorer.rs::template_test"]

[[row]]
id = "broken"
section = "10"
title = "broken"
platforms = ["ios", "android"]
target = "rust"
paths = ["rust/src/gone/**"]
coverage = ["ios-unit:ios/CoveTests/Gone.swift::testGone"]

[[row]]
id = "renamed"
section = "10"
title = "renamed"
platforms = ["android"]
target = "rust"
coverage = ["rust:rust/src/explorer.rs::renamed_test"]

[[row]]
id = "not-running"
section = "10"
title = "not running"
platforms = ["android"]
target = "rust"
coverage = [
    "rust:rust/src/explorer.rs::commented_test",
    "rust:rust/src/explorer.rs::block_test",
    "rust:rust/src/explorer.rs::helper_test",
    "rust:rust/src/explorer.rs::quoted_test",
    "rust:rust/src/explorer.rs::fixture_test",
]
"#,
    );

    let tracked = vec!["rust/src/explorer.rs".to_string()];
    let report = manifest.check(root.path(), &tracked);
    let errors = report.errors.join("\n");

    assert_eq!(report.errors.len(), 8, "{errors}");
    for name in ["commented_test", "block_test", "helper_test", "quoted_test"] {
        assert!(errors.contains(&format!("`not-running` names `{name}`")), "{errors}");
    }

    assert!(errors.contains("`broken` has no coverage entry for android"), "{errors}");
    assert!(errors.contains("path `rust/src/gone/**` matches no tracked file"), "{errors}");
    assert!(errors.contains("names `renamed_test`"), "{errors}");
    assert!(errors.contains("names missing file `ios/CoveTests/Gone.swift`"), "{errors}");
}

#[test]
fn device_scripts_apply_to_the_platform_in_their_path() {
    let coverage: Coverage =
        "device-script:qa/device/android/background.sh::s14-offline".parse().unwrap();

    assert_eq!(coverage.layer, Layer::DeviceScript);
    assert_eq!(coverage.platforms, vec![Platform::Android]);
    assert!("device-script:qa/device/background.sh::s14-offline".parse::<Coverage>().is_err());
    assert!("rust:rust/src/explorer.rs".parse::<Coverage>().is_err());
}

#[test]
fn platform_tests_need_their_test_annotation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("SendTest.kt"),
        "class SendTest {\n    @Test\n    fun sendsDraft() {}\n\n    fun helper() {}\n}\n",
    )
    .unwrap();

    std::fs::write(
        root.path().join("SendTests.swift"),
        "final class SendTests: XCTestCase {\n    func testHeader() {}\n    // func testOld() {}\n    func helper() {}\n}\n@Test func draftParses() {}\n",
    )
    .unwrap();

    let manifest = manifest(
        r#"
[[row]]
id = "send"
section = "16"
title = "send"
platforms = ["ios", "android"]
target = "ui"
coverage = [
    "android-unit:SendTest.kt::sendsDraft",
    "android-unit:SendTest.kt::helper",
    "ios-unit:SendTests.swift::testHeader",
    "ios-unit:SendTests.swift::draftParses",
    "ios-unit:SendTests.swift::testOld",
    "ios-unit:SendTests.swift::helper",
]
"#,
    );

    let report = manifest.check(root.path(), &[]);
    let errors = report.errors.join("\n");

    assert_eq!(report.errors.len(), 3, "{errors}");
    for name in ["helper", "testOld"] {
        assert!(errors.contains(&format!("names `{name}`")), "{errors}");
    }
}

#[test]
fn check_requires_rust_submodules_next_to_their_module_file() {
    let root = tempfile::tempdir().unwrap();
    let manifest = manifest(
        r#"
[[row]]
id = "root-only"
section = "18"
title = "root only"
platforms = ["ios"]
target = "manual"
paths = ["rust/src/send.rs"]
coverage = ["human"]

[[row]]
id = "with-dir"
section = "18"
title = "with dir"
platforms = ["ios"]
target = "manual"
paths = ["rust/src/send.rs", "rust/src/send/**"]
coverage = ["human"]

[[row]]
id = "one-submodule"
section = "18"
title = "one submodule"
platforms = ["ios"]
target = "manual"
paths = ["rust/src/send.rs", "rust/src/send/finalize.rs"]
coverage = ["human"]
"#,
    );

    let tracked = ["rust/src/send.rs", "rust/src/send/finalize.rs"].map(String::from);
    let report = manifest.check(root.path(), &tracked);
    let errors = report.errors.join("\n");

    assert_eq!(report.errors.len(), 1, "{errors}");
    assert!(errors.contains("`root-only` path `rust/src/send.rs` leaves out"), "{errors}");
}
