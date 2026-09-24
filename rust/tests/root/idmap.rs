//! `rust/src/idmap.rs`: the sorted map of identifiers under upper-cased keys.

use std::collections::BTreeMap;

use yggdryl::{DataType, IdMap, Scalar};

fn pairs(ids: &IdMap) -> Vec<(String, String)> {
    ids.iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn sorted_and_unique(ids: &IdMap) -> bool {
    ids.iter()
        .map(|(key, _)| key)
        .collect::<Vec<_>>()
        .windows(2)
        .all(|pair| pair[0] < pair[1])
}

#[test]
fn every_verb_folds_its_key_and_keeps_the_map_sorted() {
    let mut ids = IdMap::new();
    assert!(ids.is_empty());
    assert_eq!(ids.len(), 0);
    assert_eq!(ids.get("account"), None);
    assert_eq!(ids.to_string(), "{}");

    assert!(ids.insert("user", "U-9").unwrap());
    assert!(
        ids.insert("account", " ACC-1 ").unwrap(),
        "the value is trimmed"
    );
    assert!(!ids.insert("ACCOUNT", "ACC-2").unwrap(), "fill only");
    assert_eq!(ids.get("Account"), Some("ACC-1"));
    assert_eq!(ids.get(" account "), Some("ACC-1"), "the key is trimmed");
    assert!(ids.contains_key("USER"));
    assert!(!ids.contains_key("missing"));
    assert_eq!(ids.len(), 2);
    assert_eq!(
        pairs(&ids),
        [("ACCOUNT", "ACC-1"), ("USER", "U-9")]
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
    assert_eq!(ids.to_string(), "{ACCOUNT=ACC-1, USER=U-9}");

    assert!(
        ids.set("account", "ACC-2").unwrap(),
        "a replacement changes the map"
    );
    assert!(
        !ids.set("ACCOUNT", "ACC-2").unwrap(),
        "the same value does not"
    );
    assert!(ids.set("desk", "D1").unwrap(), "set fills an absent key");
    assert_eq!(ids.get("account"), Some("ACC-2"));
    assert_eq!(
        ids.iter().map(|(key, _)| key).collect::<Vec<_>>(),
        ["ACCOUNT", "DESK", "USER"]
    );
    assert!(sorted_and_unique(&ids));

    assert_eq!(ids.remove("Desk").as_deref(), Some("D1"));
    assert_eq!(ids.remove("desk"), None);
    assert_eq!(ids.len(), 2);

    let other = ids.clone();
    assert_eq!(other, ids);
    ids.clear();
    assert!(ids.is_empty());
    assert_ne!(other, ids);
    assert_eq!(IdMap::default(), ids);
}

#[test]
fn a_null_like_value_adds_nothing_and_replaces_nothing() {
    let mut ids = IdMap::new();
    for null in [
        "null", "NULL", "None", "n/a", "N/A", "[n/a]", "[N/A]", "", "   ",
    ] {
        assert!(
            !ids.insert("account", null).unwrap(),
            "{null:?} adds nothing"
        );
        assert!(!ids.set("account", null).unwrap(), "{null:?} sets nothing");
    }
    assert!(ids.is_empty());
    ids.insert("account", "ACC-1").unwrap();
    assert!(!ids.set("account", "null").unwrap());
    assert_eq!(
        ids.get("account"),
        Some("ACC-1"),
        "a null-like never clears"
    );
}

#[test]
fn width_and_repertoire_refusals_name_the_key_and_the_rule() {
    let mut ids = IdMap::new();
    let refused = |result: yggdryl::Result<bool>| match result.unwrap_err() {
        yggdryl::Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected a located refusal, got {other}"),
    };

    let (path, reason) = refused(ids.insert("", "value"));
    assert_eq!(path, "");
    assert_eq!(reason, "expected a key of 1 to 32 ASCII bytes, got \"\"");

    let (path, reason) = refused(ids.insert("   ", "value"));
    assert_eq!(path, "   ");
    assert!(reason.ends_with("got \"\""), "{reason}");

    let (path, reason) = refused(ids.insert("acc\u{e9}s", "value"));
    assert_eq!(path, "acc\u{e9}s");
    assert_eq!(
        reason,
        "expected a key of 1 to 32 ASCII bytes, got a non-ASCII byte 0xC3 at 3"
    );

    let (_, reason) = refused(ids.insert("acc\tount", "value"));
    assert!(reason.contains("a control byte 0x09 at 3"), "{reason}");

    let wide = "k".repeat(33);
    let (path, reason) = refused(ids.insert(&wide, "value"));
    assert_eq!(path, wide);
    assert_eq!(
        reason,
        "expected a key of 1 to 32 ASCII bytes, got 33 bytes"
    );
    assert!(
        ids.insert(&"k".repeat(32), "value").unwrap(),
        "32 bytes fit"
    );

    let (path, reason) = refused(ids.insert("account", &"v".repeat(65)));
    assert_eq!(path, "account");
    assert_eq!(
        reason,
        "expected a value of 1 to 64 ASCII bytes, got 65 bytes"
    );
    assert!(
        ids.insert("account", &"v".repeat(64)).unwrap(),
        "64 bytes fit"
    );

    let (_, reason) = refused(ids.set("user", "caf\u{e9}"));
    assert_eq!(
        reason,
        "expected a value of 1 to 64 ASCII bytes, got a non-ASCII byte 0xC3 at 3"
    );
    assert_eq!(
        ids.get(&wide),
        None,
        "a key no door accepts is held by none"
    );
    assert_eq!(ids.remove("acc\u{e9}s"), None);
    assert_eq!(ids.len(), 2);
}

#[test]
fn merge_keeps_this_side_and_first_of_reads_in_the_order_asked() {
    let mut mine = IdMap::new();
    mine.insert("account", "MINE").unwrap();
    mine.insert("user", "U-1").unwrap();
    let mut theirs = IdMap::new();
    theirs.insert("account", "THEIRS").unwrap();
    theirs.insert("desk", "D-1").unwrap();
    theirs.insert("zone", "Z-1").unwrap();

    assert!(mine.merge(&theirs));
    assert_eq!(
        pairs(&mine),
        [
            ("ACCOUNT", "MINE"),
            ("DESK", "D-1"),
            ("USER", "U-1"),
            ("ZONE", "Z-1")
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
    assert!(!mine.merge(&theirs), "a second merge adds nothing");
    assert!(!mine.merge(&IdMap::new()));
    let mut empty = IdMap::new();
    assert!(empty.merge(&theirs));
    assert_eq!(empty, theirs);

    assert_eq!(
        mine.first_of(&["trader", "desk", "account"]),
        Some(("DESK", "D-1"))
    );
    assert_eq!(
        mine.first_of(&["Account", "desk"]),
        Some(("ACCOUNT", "MINE"))
    );
    assert_eq!(mine.first_of(&["trader", "\u{e9}"]), None);
    assert_eq!(mine.first_of(&[]), None);
}

#[test]
fn the_arrow_shape_round_trips_and_refuses_what_it_cannot_hold() {
    let dtype = IdMap::dtype();
    assert_eq!(dtype, SecurityShape::expected());
    let entries = dtype.map_entries().unwrap();
    assert!(!entries.is_nullable());
    let DataType::Struct(pair) = entries.dtype() else {
        panic!("a map's entries are a struct, got {}", entries.dtype());
    };
    assert_eq!(
        pair.iter()
            .map(|field| (field.name(), field.is_nullable()))
            .collect::<Vec<_>>(),
        [("key", false), ("value", false)]
    );

    let mut ids = IdMap::new();
    ids.insert("user", "U-1").unwrap();
    ids.insert("account", "ACC-1").unwrap();
    let scalar = ids.to_scalar();
    assert_eq!(scalar.kind(), "sorted_map");
    assert_eq!(
        scalar
            .as_mapping()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.as_str().unwrap(), value.as_str().unwrap()))
            .collect::<Vec<_>>(),
        [("ACCOUNT", "ACC-1"), ("USER", "U-1")]
    );
    assert_eq!(IdMap::from_scalar(&scalar).unwrap(), ids);
    assert_eq!(
        IdMap::from_scalar(&IdMap::new().to_scalar()).unwrap(),
        IdMap::new()
    );

    let located = |scalar: Scalar| match IdMap::from_scalar(&scalar).unwrap_err() {
        yggdryl::Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected a located refusal, got {other}"),
    };
    let mapping =
        |entries: &[(Scalar, Scalar)]| match Scalar::from_mapping(entries.to_vec()).unwrap() {
            Scalar::Map(entries) => Scalar::SortedMap(entries),
            other => other,
        };
    let text = |text: &str| Scalar::from(text);

    let (path, reason) = located(Scalar::from("ACCOUNT=ACC-1"));
    assert_eq!(path, "$");
    assert_eq!(
        reason,
        "expected a sorted map of text keys and values, got string"
    );
    let (path, _) = located(mapping(&[
        (text("ACCOUNT"), text("ACC-1")),
        (text("account"), text("ACC-2")),
    ]));
    assert_eq!(path, "$[1].key", "a key that repeats once folded");
    let (path, reason) = located(mapping(&[
        (text("USER"), text("U-1")),
        (text("ACCOUNT"), text("ACC-1")),
    ]));
    assert_eq!(
        (path.as_str(), reason.contains("out of order")),
        ("$[1].key", true)
    );
    let (path, _) = located(mapping(&[(Scalar::Null, text("ACC-1"))]));
    assert_eq!(path, "$[0].key");
    let (path, _) = located(mapping(&[(text("ACCOUNT"), Scalar::Null)]));
    assert_eq!(path, "$[0].value");
    let (path, _) = located(mapping(&[(Scalar::from(1_i64), text("ACC-1"))]));
    assert_eq!(path, "$[0].key");
    let (path, reason) = located(mapping(&[(text("ACCOUNT"), text("null"))]));
    assert_eq!(
        (path.as_str(), reason.contains("states nothing")),
        ("$[0].value", true)
    );
    let (path, _) = located(mapping(&[(text("acc\u{e9}s"), text("ACC-1"))]));
    assert_eq!(
        path, "$[0].key",
        "the key's own rule is located on the entry"
    );
}

/// The map's Arrow shape, built without the type under test.
struct SecurityShape;

impl SecurityShape {
    fn expected() -> DataType {
        let entries = yggdryl::StructType::from_fields([
            DataType::utf8().required_field("key"),
            DataType::utf8().required_field("value"),
        ])
        .map(DataType::from)
        .unwrap();
        DataType::map(entries.required_field("entries"), true).unwrap()
    }
}

/// A seeded linear congruential generator, so the walk is reproducible and
/// the suite takes no new dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    fn below(&mut self, bound: u32) -> u32 {
        self.next() % bound
    }
}

#[test]
fn random_edits_agree_with_a_btree_model() {
    let keys: Vec<String> = (0..40).map(|index| format!("K{index:02}")).collect();
    let spellings = |key: &str, random: &mut Lcg| match random.below(3) {
        0 => key.to_owned(),
        1 => key.to_ascii_lowercase(),
        _ => format!(" {} ", key.to_ascii_lowercase()),
    };
    let values = |random: &mut Lcg| match random.below(10) {
        0 => "null".to_owned(),
        1 => " N/A ".to_owned(),
        2 => String::new(),
        n => format!("V{}-{n}", random.below(100)),
    };
    let mut random = Lcg(0x5eed_1d4a_9f33_0001);
    let mut ids = IdMap::new();
    let mut model: BTreeMap<String, String> = BTreeMap::new();
    for step in 0..2_000 {
        let key = &keys[random.below(40) as usize];
        match random.below(4) {
            0 => {
                let value = values(&mut random);
                let added = ids.insert(&spellings(key, &mut random), &value).unwrap();
                let trimmed = value.trim();
                let null_like = trimmed.is_empty()
                    || ["null", "n/a"].contains(&trimmed.to_ascii_lowercase().as_str());
                let expected = !null_like && !model.contains_key(key);
                if expected {
                    model.insert(key.clone(), trimmed.to_owned());
                }
                assert_eq!(added, expected, "step {step}: insert {key}={value:?}");
            }
            1 => {
                let value = values(&mut random);
                let changed = ids.set(&spellings(key, &mut random), &value).unwrap();
                let trimmed = value.trim();
                let null_like = trimmed.is_empty()
                    || ["null", "n/a"].contains(&trimmed.to_ascii_lowercase().as_str());
                let expected = !null_like && model.get(key).map(String::as_str) != Some(trimmed);
                if expected {
                    model.insert(key.clone(), trimmed.to_owned());
                }
                assert_eq!(changed, expected, "step {step}: set {key}={value:?}");
            }
            2 => {
                let removed = ids.remove(&spellings(key, &mut random));
                assert_eq!(
                    removed.as_deref(),
                    model.remove(key).as_deref(),
                    "step {step}: remove {key}"
                );
            }
            _ => {
                let mut other = IdMap::new();
                let mut other_model = BTreeMap::new();
                for _ in 0..random.below(5) {
                    let key = &keys[random.below(40) as usize];
                    let value = format!("M{}", random.below(100));
                    if other.insert(key, &value).unwrap() {
                        other_model.insert(key.clone(), value);
                    }
                }
                let added = ids.merge(&other);
                let mut expected = false;
                for (key, value) in other_model {
                    if let std::collections::btree_map::Entry::Vacant(absent) = model.entry(key) {
                        absent.insert(value);
                        expected = true;
                    }
                }
                assert_eq!(added, expected, "step {step}: merge");
            }
        }
        assert!(sorted_and_unique(&ids), "step {step}: sorted and unique");
        assert_eq!(
            pairs(&ids),
            model
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Vec<_>>(),
            "step {step}: the map and the model agree"
        );
        assert_eq!(ids.len(), model.len());
        assert_eq!(ids.get(key).map(str::to_owned), model.get(key).cloned());
        assert_eq!(IdMap::from_scalar(&ids.to_scalar()).unwrap(), ids);
    }
}

#[test]
fn an_idmap_is_fifty_six_bytes() {
    assert_eq!(std::mem::size_of::<IdMap>(), 56);
}
