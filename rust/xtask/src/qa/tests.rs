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
    assert!(plan.contains("## Covered once open PRs land\n\n- `nfc` NFC import #924"), "{plan}");
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
    std::fs::write(root.path().join("rust/src/explorer.rs"), "#[test]\nfn template_test() {}\n")
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
"#,
    );

    let tracked = vec!["rust/src/explorer.rs".to_string()];
    let report = manifest.check(root.path(), &tracked);
    let errors = report.errors.join("\n");

    assert_eq!(report.errors.len(), 4, "{errors}");
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
