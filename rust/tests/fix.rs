//! The FIX registry over storage: shard round trips through a temporary
//! folder, the tracked seed, the serialization it inherits, and the module's
//! isolation from the rest of the core.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use yggdryl::io::IOBase;
use yggdryl::local::Folder;
use yggdryl::{
    DataType, Field, FixMsg, FixRegistry, Scalar, from_json_scalar_with_field, into_json_scalar,
};

/// A fresh directory of this test's own under the platform temporary root.
fn scratch(label: &str) -> PathBuf {
    let path = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path")
        .join(format!("yggdryl-fix-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// A nullable text field carrying one canonical tag.
fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::Utf8.nullable_field(name);
    field.as_fix_mut().set_tag(tag).expect("a valid tag");
    field
}

/// The tracked seed dictionary, relative to the crate manifest.
fn seed_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix")
}

/// The sorted names under `root/records`.
fn shard_files(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("records"))
        .map(|entries| {
            entries
                .map(|entry| {
                    entry
                        .expect("a readable entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// A repeating group of parties and a component, every member tagged.
fn nested() -> (Field, Field) {
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let mut source = DataType::Utf8.nullable_field("PartyIDSource");
    source.as_fix_mut().set_tag(447).unwrap();
    let mut role = DataType::Int32.nullable_field("PartyRole");
    role.as_fix_mut().set_tag(452).unwrap();
    let item = DataType::from_fields([party_id, source, role])
        .unwrap()
        .required_field("item");
    let mut group = DataType::list(item).nullable_field("NoPartyIDs");
    group.as_fix_mut().set_tag(453).unwrap();
    group.as_fix_mut().set_aliases(["Parties"]).unwrap();
    group.as_fix_mut().set_description("The parties").unwrap();

    let mut instrument = DataType::from_fields([tagged("Symbol", 55), tagged("SecurityID", 48)])
        .unwrap()
        .nullable_field("Instrument");
    instrument.as_fix_mut().set_tag(1000).unwrap();
    instrument.as_fix_mut().set_tags(&[1001]).unwrap();
    (group, instrument)
}

#[test]
fn shards_round_trip_through_a_temporary_folder() {
    let root = scratch("shards");
    let mut folder = Folder::new(&root).unwrap();

    let registry = FixRegistry::from_fields([
        tagged("Zero", 0),
        tagged("NinetyNine", 99),
        tagged("Hundred", 100),
        tagged("HundredOne", 101),
        tagged("TenThousand", 10_000),
    ])
    .unwrap();
    registry.write_into(&mut folder).unwrap();
    assert_eq!(shard_files(&root), ["0.json", "1.json", "100.json"]);

    let reloaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(reloaded, registry);
    for (tag, name) in [
        (0, "Zero"),
        (99, "NinetyNine"),
        (100, "Hundred"),
        (101, "HundredOne"),
        (10_000, "TenThousand"),
    ] {
        assert_eq!(reloaded.field_by_tag(tag).unwrap().name(), name);
        assert_eq!(reloaded.field_by_name(name).unwrap().name(), name);
    }

    // An alternate tag in another hundred is an index entry, never a shard.
    let mut exec_type = tagged("ExecType", 150);
    exec_type.as_fix_mut().set_tags(&[20]).unwrap();
    let mut registry = registry;
    registry.insert(exec_type).unwrap();
    registry.write_into(&mut folder).unwrap();
    assert_eq!(shard_files(&root), ["0.json", "1.json", "100.json"]);
    let zero = std::fs::read_to_string(root.join("records").join("0.json")).unwrap();
    assert!(!zero.contains("ExecType"), "{zero}");
    let reloaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(reloaded.field_by_tag(20).unwrap().name(), "ExecType");
    assert_eq!(reloaded.field_by_tag(150).unwrap().name(), "ExecType");

    // A removed field's emptied shard disappears on the next write, so a
    // reload cannot resurrect it; a leaf that is not a shard is left alone.
    std::fs::write(root.join("records").join("README.md"), b"notes").unwrap();
    assert_eq!(registry.remove(10_000).unwrap().name(), "TenThousand");
    registry.write_into(&mut folder).unwrap();
    assert_eq!(shard_files(&root), ["0.json", "1.json", "README.md"]);
    let reloaded = FixRegistry::from_handle(&folder).unwrap();
    assert!(reloaded.get_field_by_tag(10_000).is_none());
    assert_eq!(reloaded, registry);

    // An absent folder loads as the empty registry.
    let absent = Folder::new(root.join("absent")).unwrap();
    assert!(FixRegistry::from_handle(&absent).unwrap().is_empty());
    assert!(!root.join("absent").exists(), "loading creates nothing");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_component_and_a_repeating_group_survive_a_store_round_trip() {
    let root = scratch("nested");
    let mut folder = Folder::new(&root).unwrap();
    let (group, instrument) = nested();
    let registry = FixRegistry::from_fields([group.clone(), instrument.clone()]).unwrap();
    registry.write_into(&mut folder).unwrap();
    assert_eq!(shard_files(&root), ["10.json", "4.json"]);

    let reloaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(reloaded, registry);
    assert_eq!(reloaded.field_by_tag(453).unwrap(), &group);
    assert_eq!(reloaded.field_by_name("parties").unwrap(), &group);
    assert_eq!(reloaded.field_by_tag(1001).unwrap(), &instrument);
    for (path, tag) in [
        ("NoPartyIDs.PartyID", 448),
        ("NoPartyIDs.PartyIDSource", 447),
        ("NoPartyIDs.item.PartyRole", 452),
        ("Instrument.Symbol", 55),
        ("Instrument.SecurityID", 48),
    ] {
        assert_eq!(
            reloaded
                .field_by_path(path)
                .unwrap()
                .as_fix()
                .tag()
                .unwrap(),
            Some(tag),
            "{path}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_malformed_shard_names_its_location() {
    let root = scratch("malformed");
    let records = root.join("records");
    std::fs::create_dir_all(&records).unwrap();
    let folder = Folder::new(&root).unwrap();

    // Not JSON at all.
    std::fs::write(records.join("0.json"), b"not json").unwrap();
    let message = FixRegistry::from_handle(&folder).unwrap_err().to_string();
    assert!(message.contains("0.json"), "{message}");

    // JSON, but not an array of fields.
    std::fs::write(records.join("0.json"), b"{}").unwrap();
    let message = FixRegistry::from_handle(&folder).unwrap_err().to_string();
    assert!(
        message.contains("a JSON array of field documents"),
        "{message}"
    );
    assert!(message.contains("0.json"), "{message}");

    // A field without a tag, then a tag another shard owns.
    let untagged = Scalar::from_sequence([DataType::Utf8.nullable_field("Symbol").into_value()]);
    std::fs::write(
        records.join("0.json"),
        yggdryl::json::into_bytes(&untagged).unwrap(),
    )
    .unwrap();
    let error = FixRegistry::from_handle(&folder).unwrap_err();
    assert!(error.is_absent(), "{error}");
    assert!(error.to_string().contains("0.json"), "{error}");
    let misplaced = Scalar::from_sequence([tagged("ExecType", 150).into_value()]);
    std::fs::write(
        records.join("0.json"),
        yggdryl::json::into_bytes(&misplaced).unwrap(),
    )
    .unwrap();
    let message = FixRegistry::from_handle(&folder).unwrap_err().to_string();
    assert!(
        message.contains("a tag of shard 0, from 0 to 99"),
        "{message}"
    );
    assert!(message.contains("tag 150"), "{message}");

    // Two fields claiming one tag: the conflict names both and the shard.
    let twice = Scalar::from_sequence([
        tagged("Symbol", 55).into_value(),
        tagged("Ticker", 55).into_value(),
    ]);
    std::fs::write(
        records.join("0.json"),
        yggdryl::json::into_bytes(&twice).unwrap(),
    )
    .unwrap();
    let error = FixRegistry::from_handle(&folder).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("Ticker") && message.contains("Symbol") && message.contains("0.json"),
        "{message}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_tracked_seed_loads_and_is_exactly_what_the_store_emits() {
    let root = seed_root();
    let folder = Folder::new(&root).unwrap();
    let registry = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(registry.len(), 34);

    // A tag, a name, an alias, an alternate tag, and a group member.
    assert_eq!(registry.field_by_tag(55).unwrap().name(), "Symbol");
    assert_eq!(registry.field_by_name("msgtype").unwrap().name(), "MsgType");
    assert_eq!(registry.field_by_name("TICKER").unwrap().name(), "Symbol");
    assert_eq!(
        registry.field_by_name("ClientOrderID").unwrap().name(),
        "ClOrdID"
    );
    assert_eq!(registry.field_by_name("qty").unwrap().name(), "OrderQty");
    assert_eq!(registry.field_by_name("px").unwrap().name(), "Price");
    assert_eq!(registry.field_by_tag(20).unwrap().name(), "ExecType");
    assert_eq!(
        registry
            .field_by_path("NoPartyIDs.PartyID")
            .unwrap()
            .as_fix()
            .tag()
            .unwrap(),
        Some(448)
    );
    assert_eq!(
        registry.field_by_tag(453).unwrap().display(),
        Some("Parties")
    );
    assert!(
        registry
            .field_by_tag(8)
            .unwrap()
            .as_fix()
            .description()
            .unwrap()
            .contains("first field")
    );
    assert_eq!(
        registry.field_by_tag(44).unwrap().dtype(),
        &DataType::decimal128(20, 8).unwrap()
    );

    // The layout is exactly `records/<shard>.json`, nothing else.
    let mut entries: Vec<String> = folder
        .ls(true, true)
        .map(|entry| {
            let entry = entry.unwrap();
            let url = entry.url().unwrap().clone();
            let path = url.into_path().unwrap();
            path.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        [
            "records",
            "records/0.json",
            "records/1.json",
            "records/4.json"
        ]
    );

    // Byte for byte, the tracked files are what `write_into` emits.
    let copy = scratch("seed-copy");
    let mut target = Folder::new(&copy).unwrap();
    registry.write_into(&mut target).unwrap();
    assert_eq!(shard_files(&copy), ["0.json", "1.json", "4.json"]);
    for name in ["0.json", "1.json", "4.json"] {
        let tracked = std::fs::read(root.join("records").join(name)).unwrap();
        let emitted = std::fs::read(copy.join("records").join(name)).unwrap();
        // The checkout may carry CRLF; the emitted document never does.
        let tracked: Vec<u8> = tracked.into_iter().filter(|byte| *byte != b'\r').collect();
        assert_eq!(tracked, emitted, "{name} differs from what the store emits");
    }
    let _ = std::fs::remove_dir_all(&copy);
}

#[test]
fn serialization_is_inherited_by_fields_and_messages() {
    let (group, instrument) = nested();
    let mut symbol = tagged("Symbol", 55);
    symbol.as_fix_mut().set_tags(&[65, 66]).unwrap();
    symbol.as_fix_mut().set_aliases(["Ticker", "Sym"]).unwrap();
    symbol
        .as_fix_mut()
        .set_description("Ticker symbol.")
        .unwrap();
    symbol.set_display("Symbol").unwrap();

    for field in [&symbol, &group, &instrument] {
        let bytes = field.clone().into_json_bytes().unwrap();
        assert_eq!(&Field::from_json_bytes(&bytes).unwrap(), field);
        let text = field.clone().into_json().unwrap();
        assert!(text.contains("fix:tag"), "{text}");
        assert_eq!(&Field::from_json(&text).unwrap(), field);
    }

    let mut qty = DataType::Int64.required_field("OrderQty");
    qty.as_fix_mut().set_tag(38).unwrap();
    let registry = Arc::new(
        FixRegistry::from_fields([
            qty.clone(),
            symbol.clone(),
            group.clone(),
            instrument.clone(),
        ])
        .unwrap(),
    );
    let root = DataType::from_fields([qty, symbol, group, instrument])
        .unwrap()
        .required_field("NewOrderSingle");
    let value = Scalar::from_record([
        ("Symbol", Scalar::from("AAPL")),
        ("OrderQty", Scalar::I64(100)),
        (
            "NoPartyIDs",
            Scalar::from_sequence([Scalar::from_record([
                ("PartyID", Scalar::from("BROKER")),
                ("PartyIDSource", Scalar::from("D")),
                ("PartyRole", Scalar::I64(1)),
            ])
            .unwrap()]),
        ),
        (
            "Instrument",
            Scalar::from_record([
                ("Symbol", Scalar::from("AAPL")),
                ("SecurityID", Scalar::Null),
            ])
            .unwrap(),
        ),
    ])
    .unwrap();
    let msg = FixMsg::with_registry(Arc::clone(&registry), root.clone(), value).unwrap();

    // The row follows the root's order, not the record's sorted one.
    let row = msg.as_value().as_sequence().unwrap();
    assert_eq!(row[0], Scalar::I64(100), "OrderQty comes first");
    assert_eq!(row[1], Scalar::from("AAPL"));
    assert_eq!(
        msg.by_path("NoPartyIDs.0.PartyRole").unwrap(),
        &Scalar::I32(1),
        "narrowed to Int32"
    );

    let text = into_json_scalar(msg.as_value()).unwrap();
    let read = from_json_scalar_with_field(&text, &root).unwrap();
    assert_eq!(&read, msg.as_value());
    let again = FixMsg::with_registry(registry, root, read).unwrap();
    assert_eq!(again, msg);
    assert_eq!(again.by_tag(38).unwrap(), &Scalar::I64(100));
    assert_eq!(
        again.by_path("Instrument.Symbol").unwrap(),
        &Scalar::from("AAPL")
    );
}

#[test]
fn nothing_outside_the_module_references_it() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut pending = vec![
        src.join("field"),
        src.join("metadata.rs"),
        src.join("iceberg"),
        src.join("io"),
        src.join("generic"),
    ];
    let mut scanned = 0;
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            pending.extend(
                std::fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        scanned += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("crate::fix"),
            "{} references crate::fix",
            path.display()
        );
        for (offset, _) in text.match_indices("fix::") {
            let before = text[..offset].chars().next_back();
            assert!(
                before.is_some_and(|character| character.is_alphanumeric() || character == '_'),
                "{} references fix:: at byte {offset}",
                path.display()
            );
        }
    }
    assert!(scanned > 20, "the scan covered {scanned} files");
}
