//! Process-level checks for categorized CRUD, reference integrity, and help.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::holder::local::Folder;
use yggdryl::{DataType, Field, FixCode, FixDirection, FixId, FixRegistry};

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
    for category in ["fields", "components", "groups"] {
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
    // A message is a component carrying a message type (decision 13): the
    // tree that named a fourth category is gone with it.
    for retired in ["list", "show", "set", "rm", "codesets", "messages"] {
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
        "components",
        "create",
        "Order",
        "struct<ClOrdID: utf8>",
        "--msgtype",
        "D",
    ]);
    // A message type makes the component a message, which is required.
    let order = workspace.read("components", "Order");
    assert!(!order.is_nullable());
    assert_eq!(order.as_fix().msgtype(), Some("D"));

    for (category, name) in [
        ("fields", "Side"),
        ("components", "Party"),
        ("groups", "Parties"),
        ("components", "Order"),
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
        ("components", "Order"),
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

/// Creates one scalar field under `dialect` through the tool.
fn create_member(workspace: &Workspace, name: &str, dtype: &str, tag: &str, dialect: &str) {
    workspace.success(&[
        "fields",
        "create",
        name,
        dtype,
        "--tag",
        tag,
        "--dialect",
        dialect,
    ]);
}

#[test]
fn field_membership_is_stamped_folded_and_replaced_by_update() {
    let workspace = Workspace::new();
    create_member(&workspace, "DeskValue", "int32", "5001", "Alpha");
    let created = workspace.read("fields", "5001");
    assert_eq!(created.name(), "DeskValue");
    assert_eq!(created.as_fix().branches().collect::<Vec<_>>(), ["alpha"]);
    assert!(created.as_fix().has_branch("ALPHA"));
    assert!(!created.as_fix().has_branch("beta"));

    // One namespace: the same tag under the same folded name is one identity,
    // whatever dictionary claims it.
    workspace.failure(&[
        "fields",
        "create",
        "desk_value",
        "utf8",
        "--tag",
        "5001",
        "--dialect",
        "beta",
    ]);
    // Update replaces membership with what it states, in sorted folded form.
    workspace.success(&[
        "fields",
        "update",
        "DeskValue",
        "int32",
        "--tag",
        "5001",
        "--dialect",
        "gamma",
        "--dialect",
        "alpha",
    ]);
    let updated = workspace.read("fields", "5001");
    assert_eq!(
        updated.as_fix().branches().collect::<Vec<_>>(),
        ["alpha", "gamma"]
    );
    let shown = output_text(&workspace.success(&["fields", "read", "DeskValue"]));
    assert!(
        shown.contains("dialects") && shown.contains("alpha, gamma"),
        "{shown}"
    );
    // A dialect name is held to the alias grammar; a refusal leaves the
    // field as it was.
    workspace.failure(&[
        "fields",
        "update",
        "DeskValue",
        "int32",
        "--tag",
        "5001",
        "--dialect",
        "a,b",
    ]);
    assert_eq!(workspace.read("fields", "5001"), updated);
    workspace.success(&["fields", "update", "DeskValue", "int32", "--tag", "5001"]);
    assert_eq!(
        workspace.read("fields", "5001").as_fix().branches().count(),
        0
    );

    workspace.failure(&[
        "fields",
        "update",
        "Missing",
        "int32",
        "--tag",
        "5002",
        "--dialect",
        "alpha",
    ]);
    workspace.failure(&["fields", "update", "DeskValue", "int32", "--tag", "5002"]);
    workspace.failure(&[
        "fields",
        "create",
        "BadType",
        "struct<x: int32>",
        "--tag",
        "5003",
    ]);
    // A colon-bearing key is a name, never an identity.
    workspace.failure(&["fields", "read", "5001:alpha"]);
}

#[test]
fn one_namespace_holds_two_fields_on_one_tag_and_lists_by_membership() {
    let workspace = Workspace::new();
    create_member(&workspace, "DeskValue", "int32", "5001", "alpha");
    // The same tag under another name is a second field beside the holder:
    // each answers its own name, the holder carries the newcomer's name as an
    // alias, and the bare tag keeps answering the holder - every command
    // reloads the store, which writes the holder first and reads it back so.
    create_member(&workspace, "OtherName", "int64", "5001", "beta");
    let other = workspace.read("fields", "OtherName");
    assert_eq!(other.dtype(), &DataType::Int64);
    assert_eq!(other.as_fix().branches().collect::<Vec<_>>(), ["beta"]);
    let holder = workspace.read("fields", "DeskValue");
    assert_eq!(holder.dtype(), &DataType::Int32);
    assert!(holder.as_fix().aliases().any(|alias| alias == "OtherName"));
    let by_tag = workspace.read("fields", "5001");
    assert_eq!(by_tag.as_fix().tag().unwrap(), Some(5001));
    assert_eq!(by_tag, holder);

    // A listing filters on membership and never resolves by it.
    let alpha = output_text(&workspace.success(&["fields", "list", "--dialect", "alpha"]));
    assert!(
        alpha.contains("DeskValue") && !alpha.contains("OtherName"),
        "{alpha}"
    );
    let beta = output_text(&workspace.success(&["fields", "list", "--dialect", "BETA"]));
    assert!(
        beta.contains("OtherName") && !beta.contains("DeskValue"),
        "{beta}"
    );
    let none = output_text(&workspace.success(&["fields", "list", "--dialect", "delta"]));
    assert!(
        !none.contains("DeskValue") && !none.contains("OtherName"),
        "{none}"
    );
    let all = output_text(&workspace.success(&["fields", "list", "5001"]));
    assert!(
        all.contains("DeskValue") && all.contains("OtherName"),
        "{all}"
    );

    // What the tool wrote is what the registry reads back.
    let stored = FixRegistry::from_handle(&Folder::new(workspace.root()).expect("root"))
        .expect("stored dictionary");
    assert_eq!(stored.dialects(), ["alpha", "beta"]);
    assert!(
        stored
            .field_by_name("DeskValue")
            .unwrap()
            .as_fix()
            .has_branch("alpha")
    );
    assert!(
        stored
            .field_by_name("OtherName")
            .unwrap()
            .as_fix()
            .has_branch("beta")
    );

    // Deleting one of the two leaves the other alone on the tag. Its name may
    // still answer as the survivor's alias - the fold gave the holder that
    // name - so absence is asserted on the listing, not on a name lookup.
    workspace.success(&["fields", "delete", "DeskValue"]);
    let rows = output_text(&workspace.success(&["fields", "list", "5001"]));
    assert!(
        !rows.contains("DeskValue") && rows.contains("OtherName"),
        "{rows}"
    );
    assert_eq!(workspace.read("fields", "5001").name(), "OtherName");
    workspace.success(&["fields", "delete", "5001"]);
    workspace.failure(&["fields", "read", "5001"]);
    workspace.failure(&["fields", "read", "OtherName"]);
    workspace.failure(&["fields", "read", "DeskValue"]);
}

#[test]
fn a_field_identity_is_its_tag_and_name_as_one_int() {
    let workspace = Workspace::new();
    workspace.success(&["fields", "create", "Desk_Value", "int32", "--tag", "5001"]);
    let expected = FixId::of(5001, "deskvalue").expect("identity");
    let field = workspace.read("fields", "5001");
    assert_eq!(field.as_fix().id().unwrap(), Some(expected));
    let shown = output_text(&workspace.success(&["fields", "read", "Desk_Value"]));
    let identity = shown
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("identity"))
        .expect("identity entry")
        .trim();
    assert_eq!(identity, expected.digest().to_string());
    let stored = FixRegistry::from_handle(&Folder::new(workspace.root()).expect("root"))
        .expect("stored dictionary");
    assert_eq!(
        stored.field_by_id(expected).expect("by id").name(),
        "Desk_Value"
    );
    assert_eq!(
        stored
            .field_by_id(FixId::from_digest(expected.digest()))
            .expect("by digest"),
        stored.field_by_tag(5001).expect("by tag")
    );
    assert!(
        stored
            .get_field_by_id(FixId::of(5001, "Other").unwrap())
            .is_none()
    );
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

#[test]
fn direction_rules_are_canonical_inline_metadata_and_invalid_updates_are_atomic() {
    let workspace = Workspace::new();
    let codes = r#"{"codes":[{"value":"R","name":"Receive"},{"value":"S","name":"Send"}]}"#;
    workspace.success(&[
        "fields",
        "create",
        "MsgDirection",
        "utf8",
        "--tag",
        "385",
        "--codes",
        codes,
        "--directions",
        r#"{"directions":[{"code":"S","patterns":["(?i)^TX\\b"]},{"code":"R","patterns":["(?i)^RX\\b"]}]}"#,
    ]);
    let field = workspace.read("fields", "385");
    // Each raw document was re-rendered through its typed setter, and neither
    // wiped the other: the set and its rules are one definition.
    assert_eq!(field.as_fix().code_value("send"), Some("S"));
    let rules = field
        .as_fix()
        .directions()
        .map(|rule| rule.map(FixDirection::from))
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        rules,
        [
            FixDirection::new("S", [r"(?i)^TX\b"]),
            FixDirection::new("R", [r"(?i)^RX\b"]),
        ]
    );
    assert_eq!(
        field.get_metadata("fix:directions"),
        Some(concat!(
            r#"{"directions":[{"code":"S","patterns":["(?i)^TX\\b"]},"#,
            r#"{"code":"R","patterns":["(?i)^RX\\b"]}]}"#,
        ))
    );
    for document in [
        "not json",
        r#"{"directions":[]}junk"#,
        r#"{"directions":[{"code":"S","patterns":["("]}]}"#,
    ] {
        workspace.failure(&[
            "fields",
            "update",
            "MsgDirection",
            "utf8",
            "--tag",
            "385",
            "--codes",
            codes,
            "--directions",
            document,
        ]);
        assert_eq!(workspace.read("fields", "385"), field);
    }
    workspace.success(&[
        "fields",
        "update",
        "MsgDirection",
        "utf8",
        "--tag",
        "385",
        "--codes",
        codes,
        "--directions",
        r#"{"directions":[]}"#,
    ]);
    let cleared = workspace.read("fields", "385");
    assert_eq!(cleared.as_fix().directions().count(), 0);
    assert_eq!(cleared.get_metadata("fix:directions"), None);
    assert_eq!(cleared.as_fix().code_value("send"), Some("S"));
}
