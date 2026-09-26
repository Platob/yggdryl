//! `cli/src/quality.rs`: the findings `ygg fix check` reports.

use std::process::Command;

#[test]
fn a_catalog_of_the_crate_s_own_definitions_checks_clean() {
    // A folder holding no catalog opens with the crate's built-in
    // definitions, `srcuuids` among them: a field whose value is a serie of
    // UUIDs, not a group read item by item.
    let root = std::env::temp_dir().join(format!("ygg-cli-quality-{}", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_ygg"))
        .args(["fix", "--root"])
        .arg(&root)
        .arg("check")
        .env("NO_COLOR", "1")
        .env_remove("GITHUB_ACTIONS")
        .output()
        .expect("run CLI");
    let text = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "{text}");
    assert!(!text.contains("item is not a struct"), "{text}");
    assert!(text.contains("nothing to report"), "{text}");
}
