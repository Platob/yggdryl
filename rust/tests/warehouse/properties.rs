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
fn a_knob_reads_through_the_one_reader_of_its_type_and_refuses_what_it_cannot_read() {
    let bag = Properties::new()
        .with_property("batch_row_size", " 1024 ")
        .with_property("num_threads", "+4")
        .with_property("safe", "maybe")
        .with_property("spill", "Yes");
    assert_eq!(
        bag.knob_count::<u64>("batch_row_size").expect("a count"),
        Some(1024)
    );
    // A count reads every spelling a whole number has, a stated sign
    // included, and a flag every spelling a boolean has, in any case.
    assert_eq!(
        bag.knob_count::<usize>("num_threads").expect("a count"),
        Some(4)
    );
    assert_eq!(bag.knob_bool("spill").expect("a flag"), Some(true));
    assert_eq!(
        bag.knob_count::<u64>("max_row_size")
            .expect("unset is none"),
        None
    );
    let error = bag.knob_bool("safe").expect_err("not a boolean");
    assert_eq!(
        error.to_string(),
        "invalid record value at $.with.safe: expected true/false, yes/no, y/n, on/off or 1/0, got \"maybe\""
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

#[test]
fn debug_hides_every_value_a_credential_name_holds() {
    let bag = Properties::new()
        .with_property("region", "eu-west-3")
        .with_property("access_key_id", "AKIAIOSFODNN7EXAMPLE")
        .with_property("secret_access_key", "wJalrXUtnFEMI-secret")
        .with_property("s3tables.session-token", "IQoJ-session")
        .with_property("client.secret-access-key", "client-secret-value")
        .with_property("SAS_TOKEN", "sv=2024-sig")
        .with_property("account_key", "azure-account-key")
        .with_property("sse_key", "customer-key")
        .with_property("basic_auth", "user:pass-word")
        .with_property("header.Authorization", "Bearer bearer-value")
        .with_property("connection_string", "AccountKey=conn-key")
        .with_property("service_account_json", "{google-json}")
        .with_property("password", "hunter2");
    let shown = format!("{bag:?}");
    assert!(shown.contains(r#""region": "eu-west-3""#), "{shown}");
    for (name, secret) in bag.iter().skip(1) {
        assert!(!shown.contains(secret), "{name} shown in {shown}");
        assert!(
            shown.contains(&format!("{name:?}: <redacted>")),
            "{name} in {shown}"
        );
    }
    // The data forms carry every value: the clause a plan writes back, and
    // the document serde writes.
    assert!(bag.to_string().contains("'hunter2'"));
    assert!(
        serde_json::to_string(&bag)
            .expect("a document")
            .contains("wJalrXUtnFEMI-secret")
    );
}
