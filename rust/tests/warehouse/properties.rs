//! `rust/src/warehouse/properties.rs`: the ordered name/value bag every
//! object, target and handle door reads.

use yggdryl::Properties;

#[test]
fn a_bag_keeps_the_order_written_and_one_value_per_name() {
    let bag = Properties::new()
        .with_property("region", "eu-west-1")
        .with_property("token", "a")
        .with_property("region", "us-east-1");
    assert_eq!(
        bag.iter().collect::<Vec<_>>(),
        [("region", "us-east-1"), ("token", "a")],
        "a later set replaces in place, keeping the place of the first writing"
    );
    assert_eq!(bag.len(), 2);
    assert!(!bag.is_empty());
    assert_eq!(bag.get("region"), Some("us-east-1"));
    assert_eq!(bag.get("REGION"), None, "names are kept exactly as written");
    assert!(bag.contains("token"));
    assert!(Properties::new().is_empty());
}

#[test]
fn a_bag_is_built_from_any_pairs_and_extended() {
    let mut bag: Properties = [("a", "1"), ("b", "2")].into_iter().collect();
    bag.extend([
        ("b".to_owned(), "3".to_owned()),
        ("c".to_owned(), "4".to_owned()),
    ]);
    assert_eq!(
        bag.iter().collect::<Vec<_>>(),
        [("a", "1"), ("b", "3"), ("c", "4")]
    );
    assert_eq!(bag.remove("b"), Some("3".into()));
    assert_eq!(bag.remove("b"), None);
    assert_eq!(bag.iter().collect::<Vec<_>>(), [("a", "1"), ("c", "4")]);
    let with = Properties::new().with_properties(bag.iter());
    assert_eq!(with, bag);
    let pairs: Vec<(&str, &str)> = (&bag).into_iter().collect();
    assert_eq!(pairs, [("a", "1"), ("c", "4")]);
}

#[test]
fn a_knob_reads_a_typed_value_and_refuses_one_that_does_not_parse() {
    let bag = Properties::new()
        .with_property("batch_row_size", " 1024 ")
        .with_property("safe", "maybe");
    assert_eq!(
        bag.knob::<u64>("batch_row_size", "a row count")
            .expect("a count"),
        Some(1024)
    );
    assert_eq!(
        bag.knob::<u64>("max_row_size", "a row count")
            .expect("unset is none"),
        None
    );
    let error = bag
        .knob::<bool>("safe", "`true` or `false`")
        .expect_err("not a boolean");
    assert_eq!(
        error.to_string(),
        "invalid record value at $.with.safe: expected `true` or `false`, got \"maybe\""
    );
}

#[test]
fn inherit_takes_the_parents_entries_and_replaces_them_by_name() {
    let parent = Properties::new()
        .with_property("region", "eu-west-1")
        .with_property("token", "parent");
    let child = Properties::new()
        .with_property("token", "child")
        .with_property("endpoint", "http://localhost");
    assert_eq!(
        child.inherit(&parent).iter().collect::<Vec<_>>(),
        [
            ("region", "eu-west-1"),
            ("token", "child"),
            ("endpoint", "http://localhost")
        ],
        "the parent's order, the child's values, the child's own appended"
    );
    assert_eq!(Properties::new().inherit(&parent), parent);
    assert_eq!(child.inherit(&Properties::new()), child);
}

#[test]
fn a_bag_serializes_as_an_object_in_order_and_displays_as_a_with_clause() {
    let bag = Properties::new()
        .with_property("media type", "text/csv")
        .with_property("it's", "quoted");
    assert_eq!(
        serde_json::to_string(&bag).expect("JSON"),
        r#"{"media type":"text/csv","it's":"quoted"}"#
    );
    let read: Properties = serde_json::from_str(r#"{"b":"2","a":"1","b":"3"}"#).expect("an object");
    assert_eq!(read.iter().collect::<Vec<_>>(), [("b", "3"), ("a", "1")]);
    assert!(
        serde_json::from_str::<Properties>(r#"[["a","1"]]"#).is_err(),
        "a bag is an object, never a list of pairs"
    );
    assert_eq!(
        bag.to_string(),
        r#""media type" = 'text/csv', "it's" = 'quoted'"#,
        "names quoted only where the grammar needs it, values as text literals"
    );
    assert_eq!(
        Properties::new().with_property("codec", "gzip").to_string(),
        "codec = 'gzip'"
    );
    assert_eq!(Properties::new().to_string(), "");
}

#[test]
fn equality_order_and_hash_follow_the_entries() {
    use std::collections::HashSet;

    let one = Properties::new().with_property("a", "1");
    let same = Properties::new().with_property("a", "1");
    let other = Properties::new().with_property("a", "2");
    assert_eq!(one, same);
    assert_ne!(one, other);
    assert!(one < other);
    let set: HashSet<Properties> = [one.clone(), same, other.clone()].into_iter().collect();
    assert_eq!(set.len(), 2);
    assert_eq!(
        Properties::default(),
        Properties::new(),
        "the default is the empty bag"
    );
    let _ = one;
    let _ = other;
}
