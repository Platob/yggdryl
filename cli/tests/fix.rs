//! `cli/src/fix.rs`: categorized CRUD, reference integrity, help, and ingest -
//! with what `cli/src/warnings.rs` prints of what the reader could not keep.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::local::LocalFolder;
use yggdryl::{DataType, Field, FixCategory, FixDirection, FixId, FixRegistry, FixSource};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf, PathBuf);

impl Workspace {
    fn new() -> Self {
        let temporary = LocalFolder::temporary()
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
        self.run_on(None, args)
    }

    /// Runs the CLI where `GITHUB_ACTIONS` is `runner`, or is not set.
    fn run_on(&self, runner: Option<&str>, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yggdryl"));
        command
            .args(["fix", "--root"])
            .arg(self.root())
            .args(args)
            .env("NO_COLOR", "1");
        match runner {
            Some(value) => command.env("GITHUB_ACTIONS", value),
            None => command.env_remove("GITHUB_ACTIONS"),
        };
        command.output().expect("run CLI")
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
        let document =
            yggdryl::into_fix_document(field.clone()).expect("one FIX definition document");
        std::fs::write(
            &path,
            yggdryl::into_json_scalar(&document).expect("native JSON"),
        )
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
        let document = yggdryl::from_json_scalar(&output.stdout).expect("native JSON stdout");
        yggdryl::from_fix_document(document).expect("native Field stdout")
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
    // A message is a component carrying a message type: the
    // tree that named a fourth category is gone with it. `codesets` is a
    // store of its own rather than a category, and has its own verbs.
    for retired in ["list", "show", "set", "rm", "messages"] {
        workspace.failure(&[retired]);
    }
    workspace.failure(&["codesets"]);
    let codesets_help = output_text(&workspace.success(&["codesets", "write", "--help"]));
    assert!(codesets_help.contains("--merge"));
    assert!(codesets_help.contains("Examples:"));
}

#[test]
fn component_identifier_flags_reach_the_core_setter_and_replace_on_update() {
    let workspace = Workspace::new();
    workspace.success(&[
        "components",
        "create",
        "Order",
        "struct<ClOrdID: utf8, OrderID: utf8>",
        "--identifiers",
        "OrderID",
        "--identifiers",
        "ClOrdID",
    ]);
    assert_eq!(
        workspace
            .read("components", "Order")
            .as_fix()
            .identifiers()
            .collect::<Vec<_>>(),
        ["ClOrdID", "OrderID"],
    );
    workspace.success(&[
        "components",
        "update",
        "Order",
        "struct<ClOrdID: utf8, OrderID: utf8>",
        "--identifiers",
        "OrderID",
    ]);
    let before = workspace.read("components", "Order");
    assert_eq!(
        before.as_fix().identifiers().collect::<Vec<_>>(),
        ["OrderID"]
    );
    for bad in ["Missing", "ClOrdID,OrderID"] {
        let refusal = workspace.failure(&[
            "components",
            "update",
            "Order",
            "struct<ClOrdID: utf8, OrderID: utf8>",
            "--identifiers",
            bad,
        ]);
        assert!(output_text(&refusal).contains("FIX:identifiers"));
        assert_eq!(workspace.read("components", "Order"), before);
    }
    let document = workspace.document(&before);
    workspace.failure(&[
        "components",
        "update",
        "--input",
        document.to_str().unwrap(),
        "--identifiers",
        "OrderID",
    ]);
    workspace.success(&[
        "components",
        "update",
        "Order",
        "struct<ClOrdID: utf8, OrderID: utf8>",
    ]);
    assert_eq!(
        workspace
            .read("components", "Order")
            .as_fix()
            .identifiers()
            .count(),
        0
    );
}

#[test]
fn all_categories_roundtrip_update_and_delete_in_dependency_order() {
    let workspace = Workspace::new();
    workspace.success(&[
        "codesets",
        "write",
        "sidecodeset",
        "--codes",
        r#"[{"value":"1","name":"Buy"},{"value":"2","name":"Sell"}]"#,
    ]);
    let mut side = DataType::Int32.nullable_field("Side");
    side.as_fix_mut().set_tag(54).unwrap();
    side.as_fix_mut().set_codeset("sidecodeset").unwrap();
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
    let mut group = DataType::serie(component.clone()).nullable_field("Parties");
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
    // A field read names the vocabulary it reads by; the members are one
    // document, printed where they live.
    let named = output_text(&workspace.success(&["fields", "read", "54"]));
    assert!(named.contains("sidecodeset"), "{named}");
    assert!(!named.contains("Buy"), "{named}");
    let codes = output_text(&workspace.success(&["codesets", "read", "sidecodeset"]));
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
    assert_eq!(created.as_fix().sources().collect::<Vec<_>>(), ["alpha"]);
    assert!(created.as_fix().has_source("ALPHA"));
    assert!(!created.as_fix().has_source("beta"));

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
        updated.as_fix().sources().collect::<Vec<_>>(),
        ["alpha", "gamma"]
    );
    let shown = output_text(&workspace.success(&["fields", "read", "DeskValue"]));
    assert!(
        shown.contains("sources") && shown.contains("alpha, gamma"),
        "{shown}"
    );
    // A dialect name is held to the source id grammar; a refusal leaves the
    // field as it was.
    workspace.failure(&[
        "fields",
        "update",
        "DeskValue",
        "int32",
        "--tag",
        "5001",
        "--dialect",
        "a\"b",
    ]);
    assert_eq!(workspace.read("fields", "5001"), updated);
    workspace.success(&["fields", "update", "DeskValue", "int32", "--tag", "5001"]);
    assert_eq!(
        workspace.read("fields", "5001").as_fix().sources().count(),
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
    // each answers its own name, neither learns the other's as an alias, and
    // the bare tag keeps answering the holder - every command reloads the
    // store, which writes the holder first and reads it back so.
    create_member(&workspace, "OtherName", "int64", "5001", "beta");
    let other = workspace.read("fields", "OtherName");
    assert_eq!(other.dtype(), &DataType::Int64);
    assert_eq!(other.as_fix().sources().collect::<Vec<_>>(), ["beta"]);
    let holder = workspace.read("fields", "DeskValue");
    assert_eq!(holder.dtype(), &DataType::Int32);
    assert!(!holder.as_fix().names().any(|alias| alias == "OtherName"));
    assert!(!other.as_fix().names().any(|alias| alias == "DeskValue"));
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
    let stored = FixRegistry::from_handle(&LocalFolder::new(workspace.root()).expect("root"))
        .expect("stored dictionary");
    assert_eq!(stored.dialects(), ["alpha", "beta"]);
    assert!(
        stored
            .field_by_name("DeskValue")
            .unwrap()
            .as_fix()
            .has_source("alpha")
    );
    assert!(
        stored
            .field_by_name("OtherName")
            .unwrap()
            .as_fix()
            .has_source("beta")
    );

    // Deleting one of the two leaves the other alone on the tag, and the
    // deleted name answers nothing: no survivor carries it.
    workspace.success(&["fields", "delete", "DeskValue"]);
    let rows = output_text(&workspace.success(&["fields", "list", "5001"]));
    assert!(
        !rows.contains("DeskValue") && rows.contains("OtherName"),
        "{rows}"
    );
    workspace.failure(&["fields", "read", "DeskValue"]);
    assert_eq!(workspace.read("fields", "5001").name(), "OtherName");
    // A tag is its digits alone: a sign or a zero reaches no field, a delete
    // included, where the name rule finds none.
    for key in ["+5001", "0", "00"] {
        workspace.failure(&["fields", "read", key]);
        workspace.failure(&["fields", "delete", key]);
    }
    assert_eq!(workspace.read("fields", "05001").name(), "OtherName");
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
    let stored = FixRegistry::from_handle(&LocalFolder::new(workspace.root()).expect("root"))
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
fn a_code_set_is_named_once_and_every_field_reading_by_it_says_so() {
    let workspace = Workspace::new();
    // The set first: a field may not name a vocabulary the dictionary does
    // not hold, which is what `fields create --codes` is refused on below.
    workspace.failure(&[
        "fields",
        "create",
        "Side",
        "int32",
        "--tag",
        "54",
        "--codes",
        "sidecodeset",
    ]);
    workspace.success(&[
        "codesets",
        "write",
        "sidecodeset",
        "--codes",
        r#"[{"value":"2","name":"Sell"},{"value":"1","name":"Buy"}]"#,
    ]);
    workspace.success(&[
        "fields",
        "create",
        "Side",
        "int32",
        "--tag",
        "54",
        "--codes",
        "sidecodeset",
    ]);
    // One vocabulary, named by as many fields as read by it.
    workspace.success(&[
        "fields",
        "create",
        "LegSide",
        "int32",
        "--tag",
        "624",
        "--codes",
        "sidecodeset",
    ]);
    let field = workspace.read("fields", "54");
    assert_eq!(field.as_fix().codeset(), Some("sidecodeset"));
    assert_eq!(
        workspace.read("fields", "624").as_fix().codeset(),
        Some("sidecodeset")
    );

    let listed = output_text(&workspace.success(&["codesets", "list"]));
    assert!(listed.contains("sidecodeset"), "{listed}");
    let read = output_text(&workspace.success(&["codesets", "read", "sidecodeset"]));
    for spelling in ["Buy", "Sell"] {
        assert!(read.contains(spelling), "{read}");
    }

    // A document that is not one, and one naming two codes alike, leave the
    // set exactly as it was.
    let held = workspace.success(&["codesets", "read", "sidecodeset", "--json"]);
    for document in [
        "not json",
        r"[]junk",
        r#"[{"value":"1","name":"Buy"},{"value":"2","name":"Buy"}]"#,
    ] {
        workspace.failure(&["codesets", "write", "sidecodeset", "--codes", document]);
        assert_eq!(
            output_text(&workspace.success(&["codesets", "read", "sidecodeset", "--json"])),
            output_text(&held)
        );
    }

    // A merge adds what only the incoming side states and keeps the rest.
    workspace.success(&[
        "codesets",
        "write",
        "sidecodeset",
        "--merge",
        "--codes",
        r#"[{"value":"7","name":"Undisclosed"}]"#,
    ]);
    let merged = output_text(&workspace.success(&["codesets", "read", "sidecodeset"]));
    for spelling in ["Buy", "Sell", "Undisclosed"] {
        assert!(merged.contains(spelling), "{merged}");
    }

    // A set two fields still read by is not one a delete may take away.
    workspace.failure(&["codesets", "delete", "sidecodeset"]);
    workspace.success(&["fields", "delete", "624"]);
    workspace.success(&["fields", "update", "Side", "int32", "--tag", "54"]);
    assert_eq!(workspace.read("fields", "54").as_fix().codeset(), None);
    workspace.success(&["codesets", "delete", "sidecodeset"]);
    workspace.failure(&["codesets", "read", "sidecodeset"]);
}

#[test]
fn direction_rules_are_canonical_inline_metadata_and_invalid_updates_are_atomic() {
    let workspace = Workspace::new();
    workspace.success(&[
        "codesets",
        "write",
        "msgdirectioncodeset",
        "--codes",
        r#"[{"value":"R","name":"Receive"},{"value":"S","name":"Send"}]"#,
    ]);
    let codes = "msgdirectioncodeset";
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
        r#"[{"code":"S","patterns":["(?i)^TX\\b"]},{"code":"R","patterns":["(?i)^RX\\b"]}]"#,
    ]);
    let field = workspace.read("fields", "385");
    // The set it names and the rules it carries are one definition, and
    // neither write wiped the other.
    assert_eq!(field.as_fix().codeset(), Some("msgdirectioncodeset"));
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
        field.get_metadata("FIX:directions"),
        Some(concat!(
            r#"[{"code":"S","patterns":["(?i)^TX\\b"]},"#,
            r#"{"code":"R","patterns":["(?i)^RX\\b"]}]"#,
        ))
    );
    for document in ["not json", r"[]junk", r#"[{"code":"S","patterns":["("]}]"#] {
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
        r"[]",
    ]);
    let cleared = workspace.read("fields", "385");
    assert_eq!(cleared.as_fix().directions().count(), 0);
    assert_eq!(cleared.get_metadata("FIX:directions"), None);
    assert_eq!(cleared.as_fix().codeset(), Some("msgdirectioncodeset"));
}

/// One `CBlock` declaring tag 532 as `declared` and binding it in message `r`.
fn rejection(declared: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
	<vocabulary>
		<vocabulary-tag name="35" alt="MsgType" type="string" />
		<vocabulary-tag name="532" alt="MassCancelRejectReason" type="{declared}" />
		<vocabulary-tag name="58" alt="Text" type="string" />
	</vocabulary>
	<grammar-binding type="r"><grammar>
		<tag-constraint name="532" />
		<tag-constraint name="58" />
	</grammar></grammar-binding>
</cplugin-configuration>"#
    )
}

impl Workspace {
    /// Writes `CBlock`s under `cblocks/`, answering the folder.
    fn cblocks(&self, files: &[(&str, &str)]) -> PathBuf {
        let folder = self.0.join("cblocks");
        std::fs::create_dir_all(&folder).expect("a folder of CBlocks");
        for (name, body) in files {
            std::fs::write(folder.join(name), body).expect("one CBlock");
        }
        folder
    }

    fn loaded(&self) -> FixRegistry {
        FixRegistry::from_handle(&LocalFolder::new(self.root()).expect("store folder"))
            .expect("a stored dictionary")
    }
}

#[test]
fn ingest_folds_a_glob_of_cblocks_in_one_commit_and_names_what_it_passes_over() {
    let workspace = Workspace::new();
    // Three dialects over one wire tag: a flag, a number and text. The
    // first file in URL order is held, the text says less than it and folds
    // under it, and the number contradicts it and is named with its file.
    let folder = workspace.cblocks(&[
        ("bloomberg_fix44_dropcopy.cfb", &rejection("integer")),
        ("axessiq_fix44.cfb", &rejection("boolean")),
        ("tradeweb_fix44.cfb", &rejection("string")),
        ("notes.txt", "not a dictionary"),
    ]);
    let pattern = folder.join("*.cfb");
    let output = workspace.success(&["ingest", pattern.to_str().expect("test path")]);
    let text = output_text(&output);
    // One fold of every file: the contradicting declaration is named with its
    // file and its dialect, the restated one is counted, and the rest arrived
    // in one commit.
    assert!(text.contains("3 file(s)"), "{text}");
    assert!(text.contains("(1 restated)"), "{text}");
    assert!(text.contains("1 passed over"), "{text}");
    let named = text
        .lines()
        .find(|line| line.contains("bloomberg_fix44_dropcopy.cfb [bloomberg_fix44_dropcopy]"))
        .unwrap_or_else(|| panic!("the passed-over declaration named: {text}"));
    assert!(named.contains("masscancelrejectreason (532)"), "{named}");
    assert!(text.contains("committed: "), "{text}");
    let stored = workspace.loaded();
    let held = stored.field_by_tag(532).expect("tag 532 stored");
    assert_eq!(held.dtype(), &DataType::Boolean);
    assert_eq!(
        held.as_fix().sources().collect::<Vec<_>>(),
        ["axessiq_fix44", "tradeweb_fix44"]
    );
    assert!(stored.msgtype("r").is_ok(), "the message arrived");

    // The same files again change nothing, so nothing is committed.
    let again = output_text(&workspace.success(&["ingest", pattern.to_str().expect("test path")]));
    assert!(!again.contains("committed: "), "{again}");
    assert_eq!(workspace.loaded(), stored);

    // A shell that expanded the glob hands over the files; they fold as one,
    // and under --annotate each passed-over declaration is a workflow warning.
    let files: Vec<String> = [
        "tradeweb_fix44.cfb",
        "bloomberg_fix44_dropcopy.cfb",
        "axessiq_fix44.cfb",
    ]
    .iter()
    .map(|name| folder.join(name).to_str().expect("test path").to_owned())
    .collect();
    let mut arguments = vec!["ingest", "--annotate", "--dialect", "desk"];
    arguments.extend(files.iter().map(String::as_str));
    let annotated = output_text(&workspace.success(&arguments));
    assert!(annotated.contains("3 file(s)"), "{annotated}");
    assert!(
        annotated.lines().any(
            |line| line.starts_with("::warning title=fix passed over::file:")
                && line.contains("bloomberg_fix44_dropcopy.cfb")
        ),
        "{annotated}"
    );
}

#[test]
fn ingest_names_what_the_reader_could_not_keep_where_it_stands_and_what_it_kept() {
    let workspace = Workspace::new();
    let folder = workspace.cblocks(&[("a_venue.cfb", &rejection("widget"))]);
    let file = folder.join("a_venue.cfb");
    let file = file.to_str().expect("test path");

    // A type word nothing reads: the tag is kept as text, and the sentence
    // says what is wrong, where, and what the reader did instead, under the
    // dialect it was read for.
    let text = output_text(&workspace.success(&["ingest", file, "--dialect", "venue"]));
    assert!(text.contains("1 warning(s) while reading"), "{text}");
    let warned = text
        .lines()
        .find(|line| line.contains("\"widget\""))
        .unwrap_or_else(|| panic!("the unread type word named: {text}"));
    for said in ["[venue] ", "line 5, column ", "typed string"] {
        assert!(warned.contains(said), "{said}: {warned}");
    }
    let held = workspace.loaded();
    let held = held.field_by_tag(532).expect("tag 532 kept");
    assert_eq!(held.dtype(), &DataType::utf8());

    // Under --annotate the same sentence is one workflow warning.
    let annotated =
        output_text(&workspace.success(&["ingest", file, "--dialect", "venue", "--annotate"]));
    assert!(
        !annotated.contains("warning(s) while reading"),
        "{annotated}"
    );
    assert!(
        annotated.lines().any(|line| {
            line.starts_with("::warning title=fix reader::")
                && line.contains(" [venue] ")
                && line.contains("typed string")
        }),
        "{annotated}"
    );

    // A file the core cannot fold is left out and named after what the
    // files beside it were read as, and every other file still folds.
    let truncated = rejection("string").replace("</vocabulary>", "");
    workspace.cblocks(&[("b_venue.cfb", &truncated)]);
    let refused = output_text(&workspace.success(&[
        "ingest",
        folder.join("*.cfb").to_str().expect("test path"),
        "--dialect",
        "venue",
    ]));
    assert!(refused.contains("1 file(s) left out"), "{refused}");
    let warned = refused
        .find("typed string")
        .unwrap_or_else(|| panic!("the kept tag named: {refused}"));
    let refusal = refused
        .find("b_venue.cfb")
        .unwrap_or_else(|| panic!("the refused file named: {refused}"));
    assert!(warned < refusal, "{refused}");

    // A run whose every file is left out names each, then fails.
    let output = workspace.failure(&[
        "ingest",
        folder.join("b_venue.cfb").to_str().expect("test path"),
    ]);
    let text = output_text(&output);
    assert!(text.contains("b_venue.cfb"), "{text}");
    assert!(text.contains(".cfb file that folds"), "{text}");
}

#[test]
fn ingest_refuses_a_location_holding_nothing_and_sync_names_the_verb_for_a_cblock() {
    let workspace = Workspace::new();
    let folder = workspace.cblocks(&[("venue.cfb", &rejection("string"))]);
    let output = workspace.failure(&["ingest", folder.join("*.xml").to_str().expect("test path")]);
    assert!(
        output_text(&output).contains("*.xml"),
        "{}",
        output_text(&output)
    );
    let empty = folder.join("empty");
    std::fs::create_dir_all(&empty).expect("an empty folder");
    let output = workspace.failure(&["ingest", empty.to_str().expect("test path")]);
    assert!(
        output_text(&output).contains(".cfb file the locations hold"),
        "{}",
        output_text(&output)
    );
    let output = workspace.failure(&[
        "ingest",
        folder.join("absent.cfb").to_str().expect("test path"),
    ]);
    assert!(
        output_text(&output).contains("absent.cfb"),
        "{}",
        output_text(&output)
    );
    let output = workspace.failure(&[
        "sync",
        folder.join("venue.cfb").to_str().expect("test path"),
    ]);
    assert!(
        output_text(&output).contains("ingest"),
        "{}",
        output_text(&output)
    );
    assert!(
        !workspace.root().join("fields").exists(),
        "nothing was written"
    );

    // A single file under a named dialect.
    let text = output_text(&workspace.success(&[
        "ingest",
        folder.join("venue.cfb").to_str().expect("test path"),
        "--dialect",
        "venue",
    ]));
    assert!(text.contains("1 file(s)"), "{text}");
    assert_eq!(
        workspace
            .loaded()
            .field_by_tag(532)
            .expect("stored")
            .as_fix()
            .sources()
            .collect::<Vec<_>>(),
        ["venue"]
    );
}

#[test]
fn a_runner_variable_turns_annotations_on_by_what_it_says_rather_than_by_being_set() {
    let workspace = Workspace::new();
    let folder = workspace.cblocks(&[("a_venue.cfb", &rejection("widget"))]);
    let file = folder.join("a_venue.cfb");
    let file = file.to_str().expect("test path");
    let ingest = ["ingest", file, "--dialect", "venue"];
    let annotated = |text: &str| {
        text.lines()
            .any(|line| line.starts_with("::warning title=fix reader::"))
    };

    // What a runner sets, and every spelling of true the boolean door reads.
    for value in ["true", "TRUE", " yes ", "on", "1"] {
        let output = workspace.run_on(Some(value), &ingest);
        let text = output_text(&output);
        assert!(output.status.success(), "{value:?}: {text}");
        assert!(annotated(&text), "{value:?}: {text}");
        assert!(
            !text.contains("warning(s) while reading"),
            "{value:?}: {text}"
        );
    }

    // Set to false, or to nothing, it says no runner, as unset does: the
    // same findings come as the table.
    for value in ["false", "False", "no", "off", "0", "", "  "] {
        let output = workspace.run_on(Some(value), &ingest);
        let text = output_text(&output);
        assert!(output.status.success(), "{value:?}: {text}");
        assert!(!annotated(&text), "{value:?}: {text}");
        assert!(
            text.contains("warning(s) while reading"),
            "{value:?}: {text}"
        );
    }

    // The switch is still its own: it turns annotations on whatever the
    // variable says.
    let text = output_text(&workspace.run_on(
        Some("false"),
        &["ingest", file, "--dialect", "venue", "--annotate"],
    ));
    assert!(annotated(&text), "{text}");

    // Text no boolean spells is refused by the variable's name, before any
    // verb runs and whichever namespace was asked for.
    let refused = workspace.run_on(Some("maybe"), &ingest);
    let text = output_text(&refused);
    assert!(!refused.status.success(), "{text}");
    assert!(text.contains("GITHUB_ACTIONS"), "{text}");
    assert!(!text.contains("warning(s) while reading"), "{text}");
}

#[test]
fn a_dialect_creates_its_sources_entry_and_check_reports_what_dangles_or_is_unreferenced() {
    let workspace = Workspace::new();
    create_member(&workspace, "DeskValue", "int32", "5001", "Alpha");
    // The entry the id names is written beside the categories, folded, and
    // read back as the catalog it is.
    let stored = std::fs::read_to_string(workspace.root().join("sources.json")).unwrap();
    let catalog = yggdryl::from_json_scalar(&stored).unwrap();
    assert_eq!(
        yggdryl::into_json_scalar(&catalog).unwrap(),
        r#"[{"id":"alpha","pluginside":"UKNW"}]"#
    );
    let registry = FixRegistry::from_handle(&LocalFolder::new(workspace.root()).expect("root"))
        .expect("stored dictionary");
    assert_eq!(registry.get_source("alpha").unwrap().file(), None);
    let shown = output_text(&workspace.success(&["fields", "read", "DeskValue"]));
    assert!(
        shown.contains("sources") && shown.contains("alpha"),
        "{shown}"
    );
    let checked = output_text(&workspace.success(&["check"]));
    assert!(checked.contains("sources"), "{checked}");
    // Update under a second dialect adds its entry and keeps the first.
    workspace.success(&[
        "fields",
        "update",
        "DeskValue",
        "int32",
        "--tag",
        "5001",
        "--dialect",
        "gamma",
    ]);
    let registry = FixRegistry::from_handle(&LocalFolder::new(workspace.root()).expect("root"))
        .expect("stored dictionary");
    assert_eq!(
        registry.sources().map(FixSource::id).collect::<Vec<_>>(),
        ["alpha", "gamma"]
    );

    // An id no entry holds fails the check, naming the definition and the
    // id; an entry nothing names is a note, in the shape the code-set note
    // has. Both are staged outside the tool, which never writes either: the
    // definition is replaced rather than merged, since a merge unions the
    // ids and would keep `gamma` named.
    let mut folder = LocalFolder::new(workspace.root()).expect("root");
    let mut registry = FixRegistry::from_handle(&folder).expect("stored dictionary");
    let mut field = registry.field_by_name("DeskValue").unwrap().clone();
    field.as_fix_mut().set_sources(["ghost"]).unwrap();
    registry
        .update_definition(FixCategory::Fields, field)
        .unwrap();
    assert!(registry.add_source(FixSource::new("orphan").unwrap()));
    registry.commit(&mut folder).unwrap();
    let text = output_text(&workspace.failure(&["check", "--annotate"]));
    assert!(
        text.contains(
            "::error title=fix sources::fields/DeskValue: names source \"ghost\", which sources.json does not hold"
        ),
        "{text}"
    );
    for unreferenced in ["alpha", "gamma", "orphan"] {
        assert!(
            text.contains(&format!(
                "::note title=fix sources::sources/{unreferenced}: no field or definition names it"
            )),
            "{text}"
        );
    }
}

/// The fixture `rejection` carries, under a root naming the plugin's class
/// the way a vendor's file does.
fn sided(role: &str) -> String {
    rejection("string").replace(
        "<cplugin-configuration fix-version=\"4.4\">",
        &format!(
            "<cplugin-configuration type=\"com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.{role}FIXCPluginCBlock\" version=\"1.2\" fix-version=\"4.4\">"
        ),
    )
}

/// The plugin's role is a column of the schema right after the plugin id,
/// read by an intrinsic set the tool lists and reads like any other and
/// refuses to write, and a fresh store holding one ingested Sell-Side file
/// - whose entry states `SELL` - is clean under `check`.
#[test]
fn the_plugin_side_is_a_schema_column_an_intrinsic_set_and_clean_under_check() {
    use yggdryl::PluginSide;
    let workspace = Workspace::new();
    let folder = workspace.cblocks(&[("ms_fix44.cfb", &sided("SellSide"))]);
    let file = folder.join("ms_fix44.cfb");
    let text = output_text(&workspace.success(&["ingest", file.to_str().expect("test path")]));
    assert!(text.contains("1 file(s)"), "{text}");
    let registry = workspace.loaded();
    assert_eq!(
        registry.get_source("ms_fix44").unwrap().pluginside(),
        PluginSide::SellSide
    );
    let stored = std::fs::read_to_string(workspace.root().join("sources.json")).unwrap();
    assert_eq!(
        yggdryl::into_json_scalar(&yggdryl::from_json_scalar(&stored).unwrap()).unwrap(),
        r#"[{"file":"ms_fix44.cfb","id":"ms_fix44","pluginside":"SELL"}]"#
    );

    // The schema lists the column after the plugin id; the strike moves its
    // tag to 65043, at the terminal and as JSON.
    let schema = output_text(&workspace.success(&["schema"]));
    let plugin = schema
        .lines()
        .position(|line| line.contains("msgpluginid"))
        .unwrap_or_else(|| panic!("msgpluginid listed: {schema}"));
    let side = schema
        .lines()
        .position(|line| line.contains("msgpluginside"))
        .unwrap_or_else(|| panic!("msgpluginside listed: {schema}"));
    assert!(side > plugin, "{schema}");
    let line = schema.lines().nth(side).unwrap();
    assert!(
        line.contains("pluginside")
            && line.contains("65043")
            && line.contains("Message Plugin Side"),
        "{line}"
    );
    let out = workspace.0.join("schema.json");
    workspace.success(&["schema", "--out", out.to_str().expect("test path")]);
    let written = Field::from_json(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let columns: Vec<&str> = written.fields().iter().map(Field::name).collect();
    let at = columns
        .iter()
        .position(|name| *name == "msgpluginside")
        .unwrap();
    assert_eq!(columns[at - 1], "msgpluginid");
    assert_eq!(columns[at + 1], "msgoriginator");
    assert_eq!(written.fields()[at].dtype(), &DataType::PluginSide);
    assert!(!written.fields()[at].is_nullable());

    // The intrinsic set is listed and read like any other, and refuses a
    // write; the store carries it as a document.
    let listed = output_text(&workspace.success(&["codesets", "list"]));
    assert!(listed.contains("msgpluginsidecodeset"), "{listed}");
    let read = output_text(&workspace.success(&["codesets", "read", "msgpluginsidecodeset"]));
    for spelling in ["UKNW", "BUYS", "SELL"] {
        assert!(read.contains(spelling), "{read}");
    }
    workspace.failure(&[
        "codesets",
        "write",
        "msgpluginsidecodeset",
        "--codes",
        r#"[{"value":"3","name":"MIDL"}]"#,
    ]);
    assert!(
        workspace
            .root()
            .join("codesets/msgpluginsidecodeset.json")
            .is_file()
    );

    // Clean: the entry is named by the ingested fields, and the set is read
    // by the crate's own column, so neither is a finding.
    let checked = output_text(&workspace.success(&["check"]));
    assert!(!checked.contains("msgpluginside"), "{checked}");
    assert!(!checked.contains("ms_fix44"), "{checked}");
}
