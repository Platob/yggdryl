//! Process-level checks for categorized CRUD, reference integrity, and help.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::holder::local::Folder;
use yggdryl::{DataType, Field, FixCode};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf, PathBuf);

impl Workspace {
    fn new() -> Self {
        let temporary = Folder::temporary()
            .expect("native temporary folder")
            .path()
            .expect("native temporary path");
        let temporary = std::fs::canonicalize(temporary).expect("resolved temporary folder");
        let path = temporary.join(format!(
            "ygg-cli-fix-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("isolated test folder");
        Self(path, temporary)
    }

    fn root(&self) -> PathBuf {
        self.0.join("registry")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ygg"))
            .args(["fix", "--root"])
            .arg(self.root())
            .args(args)
            .env("NO_COLOR", "1")
            .env_remove("GITHUB_ACTIONS")
            .output()
            .expect("run CLI")
    }

    fn success(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            output_text(&output)
        );
        output
    }

    fn failure(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(!output.status.success(), "unexpected success: {args:?}");
        output
    }

    fn document(&self, field: &Field) -> PathBuf {
        let path = self.0.join(format!("{}.json", field.name()));
        std::fs::write(&path, field.clone().into_json_bytes().expect("native JSON"))
            .expect("write definition");
        path
    }

    fn input(&self, category: &str, operation: &str, path: &Path) -> Output {
        self.success(&[
            category,
            operation,
            "--input",
            path.to_str().expect("test path"),
        ])
    }

    fn read(&self, category: &str, name: &str) -> Field {
        let output = self.success(&[category, "read", name, "--json"]);
        Field::from_json_bytes(&output.stdout).expect("native Field stdout")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let target = std::fs::canonicalize(&self.0).expect("resolve owned test folder");
        assert_eq!(target, self.0, "test fixture target changed");
        assert_eq!(
            target.parent(),
            Some(self.1.as_path()),
            "test fixture escaped its parent"
        );
        std::fs::remove_dir_all(target).expect("remove owned test folder");
    }
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn categories_expose_all_crud_operations_and_examples() {
    let workspace = Workspace::new();
    let help = output_text(&workspace.success(&["--help"]));
    for category in ["fields", "messages", "components", "groups"] {
        assert!(help.contains(category), "{help}");
        let category_help = output_text(&workspace.success(&[category, "--help"]));
        for operation in ["list", "read", "create", "update", "delete"] {
            assert!(category_help.contains(operation), "{category_help}");
            workspace.success(&[category, operation, "--help"]);
        }
        let create_help = output_text(&workspace.success(&[category, "create", "--help"]));
        assert!(create_help.contains("--input"));
        assert!(create_help.contains("Examples:"));
    }
    for retired in ["list", "show", "set", "rm", "codesets"] {
        workspace.failure(&[retired]);
    }
}

#[test]
fn all_categories_roundtrip_update_and_delete_in_dependency_order() {
    let workspace = Workspace::new();
    let mut side = DataType::Int32.nullable_field("Side");
    side.as_fix_mut().set_tag(54).unwrap();
    side.as_fix_mut()
        .set_codes(&[FixCode::new("Buy", "1"), FixCode::new("Sell", "2")])
        .unwrap();
    workspace.input("fields", "create", &workspace.document(&side));
    workspace.success(&["fields", "create", "NoPartyIDs", "int32", "--tag", "453"]);
    workspace.success(&[
        "components",
        "create",
        "Party",
        "struct<PartyID: utf8>",
        "--required",
    ]);
    let component = workspace.read("components", "Party");
    let mut group = DataType::list(component.clone()).nullable_field("Parties");
    group.as_fix_mut().set_counter(453).expect("counter");
    group
        .as_fix_mut()
        .set_component("Party")
        .expect("component reference");
    workspace.input("groups", "create", &workspace.document(&group));
    workspace.success(&[
        "messages",
        "create",
        "Order",
        "struct<ClOrdID: utf8>",
        "--msgtype",
        "D",
    ]);

    for (category, name) in [
        ("fields", "Side"),
        ("components", "Party"),
        ("groups", "Parties"),
        ("messages", "Order"),
    ] {
        let original = workspace.read(category, name);
        let path = workspace.document(&original);
        workspace.failure(&[category, "create", "--input", path.to_str().expect("path")]);
        let mut missing = original.clone();
        missing.set_name(format!("Missing{name}"));
        let path = workspace.document(&missing);
        workspace.failure(&[category, "update", "--input", path.to_str().expect("path")]);
        let mut updated = original.clone();
        updated
            .as_fix_mut()
            .set_description("Reviewed definition")
            .expect("description");
        workspace.input(category, "update", &workspace.document(&updated));
        assert_eq!(workspace.read(category, name), updated);
        let list = output_text(&workspace.success(&[category, "list", name]));
        assert!(list.contains(name), "{list}");
    }
    let count = workspace.read("fields", "453");
    assert_eq!(count.name(), "NoPartyIDs");
    assert_eq!(count.dtype(), &DataType::Int32);
    let codes = output_text(&workspace.success(&["fields", "read", "54"]));
    assert!(codes.contains("Buy") && codes.contains("Sell"), "{codes}");
    workspace.failure(&["fields", "delete", "453"]);
    workspace.failure(&["components", "delete", "Party"]);
    workspace.success(&["check"]);
    for (category, name) in [
        ("messages", "Order"),
        ("groups", "Parties"),
        ("components", "Party"),
        ("fields", "Side"),
        ("fields", "NoPartyIDs"),
    ] {
        workspace.success(&[category, "delete", name]);
        workspace.failure(&[category, "read", name]);
        workspace.failure(&[category, "delete", name]);
    }
}

#[test]
fn field_crud_preserves_branch_identity_and_refuses_invalid_mutations() {
    let workspace = Workspace::new();
    workspace.success(&[
        "fields",
        "create",
        "DeskValue",
        "int32",
        "--tag",
        "5001",
        "--branch",
        "alpha",
    ]);
    workspace.success(&[
        "fields",
        "create",
        "DeskValue",
        "utf8",
        "--tag",
        "5001",
        "--branch",
        "beta",
    ]);
    workspace.failure(&[
        "fields",
        "create",
        "OtherName",
        "int64",
        "--tag",
        "5001",
        "--branch",
        "alpha",
    ]);
    workspace.failure(&[
        "fields", "update", "Missing", "int32", "--tag", "5002", "--branch", "alpha",
    ]);
    workspace.failure(&[
        "fields",
        "update",
        "DeskValue",
        "int32",
        "--tag",
        "5002",
        "--branch",
        "alpha",
    ]);
    workspace.failure(&[
        "fields",
        "create",
        "BadType",
        "struct<x: int32>",
        "--tag",
        "5003",
        "--branch",
        "alpha",
    ]);
    workspace.failure(&["fields", "read", "5001:alpha", "--branch", "beta"]);
    let output = workspace.success(&["fields", "read", "5001", "--branch", "alpha", "--json"]);
    let before = Field::from_json_bytes(&output.stdout).expect("field");
    assert_eq!(before.name(), "DeskValue");
    assert_eq!(before.dtype(), &DataType::Int32);
    workspace.success(&["fields", "delete", "5001", "--branch", "alpha"]);
    workspace.failure(&["fields", "read", "5001", "--branch", "alpha"]);
    let output = workspace.success(&["fields", "read", "5001", "--branch", "beta", "--json"]);
    let remaining = Field::from_json_bytes(&output.stdout).expect("remaining branch field");
    assert_eq!(remaining.dtype(), &DataType::Utf8);
}

#[test]
fn field_codes_are_canonical_inline_metadata_and_invalid_updates_are_atomic() {
    let workspace = Workspace::new();
    workspace.success(&[
        "fields",
        "create",
        "Side",
        "int32",
        "--tag",
        "54",
        "--codes",
        r#"{"codes":[{"value":"2","name":"Sell"},{"value":"1","name":"Buy"}]}"#,
    ]);
    let field = workspace.read("fields", "54");
    assert_eq!(field.as_fix().code_value("buy"), Some("1"));
    assert_eq!(field.as_fix().codes().next().unwrap().unwrap().value(), "1");
    for document in [
        "not json",
        r#"{"codes":[]}junk"#,
        r#"{"codes":[{"value":"1","name":"Buy"},{"value":"2","name":"Buy"}]}"#,
    ] {
        workspace.failure(&[
            "fields", "update", "Side", "int32", "--tag", "54", "--codes", document,
        ]);
        assert_eq!(workspace.read("fields", "54"), field);
    }
    workspace.failure(&[
        "fields",
        "create",
        "Other",
        "int32",
        "--tag",
        "55",
        "--codeset",
        "Side",
    ]);
    workspace.success(&[
        "fields",
        "update",
        "Side",
        "int32",
        "--tag",
        "54",
        "--codes",
        r#"{"codes":[]}"#,
    ]);
    assert_eq!(workspace.read("fields", "54").as_fix().codes().count(), 0);
}
