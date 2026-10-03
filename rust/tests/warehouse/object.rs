//! `rust/src/warehouse/object.rs`: what every object answers, the one enum
//! over the three kinds, the path intake and the lazy children.

use smol_str::SmolStr;
use yggdryl::expression::Location;
use yggdryl::holder::Holder;
use yggdryl::{
    IOBase, IOKind, IntoObjectPath, MediaTable, MemoryCatalog, MemoryNamespace, Object,
    ObjectValue, Objects, Properties, Url,
};

fn parts(path: impl IntoObjectPath) -> Vec<String> {
    path.into_object_path()
        .expect("a path of parts")
        .into_iter()
        .map(|part| part.to_string())
        .collect()
}

#[test]
fn a_path_is_read_through_the_location_grammar() {
    assert_eq!(parts("lake.eu.trades"), ["lake", "eu", "trades"]);
    assert_eq!(
        parts("lake.\"eu west\".fills"),
        ["lake", "eu west", "fills"]
    );
    assert_eq!(parts("lake.`eu west`.fills"), ["lake", "eu west", "fills"]);
    assert_eq!(parts("lake.[eu west].fills"), ["lake", "eu west", "fills"]);
    assert_eq!(
        parts("\"a.b\".c"),
        ["a.b", "c"],
        "a quoted dot is no separator"
    );
    assert_eq!(parts("  lake.eu  "), ["lake", "eu"]);
    assert_eq!(
        parts(""),
        Vec::<String>::new(),
        "the empty text is the root"
    );
    assert_eq!(parts("   "), Vec::<String>::new());
    assert_eq!(parts(String::from("a.b")), ["a", "b"]);
    let owned = String::from("a.b");
    assert_eq!(parts(&owned), ["a", "b"]);
}

#[test]
fn parts_arrive_as_they_are() {
    assert_eq!(parts(["lake", "eu west"]), ["lake", "eu west"]);
    let array: &[&str; 2] = &["a.b", "c"];
    assert_eq!(
        parts(array),
        ["a.b", "c"],
        "an array part is one part, dot and all"
    );
    assert_eq!(parts(vec!["x"]), ["x"]);
    assert_eq!(parts(&["x", "y"][..]), ["x", "y"]);
    let smol = vec![SmolStr::new("a"), SmolStr::new("b")];
    assert_eq!(parts(&smol), ["a", "b"]);
    assert_eq!(parts(smol.as_slice()), ["a", "b"]);
    assert_eq!(parts(smol.clone()), ["a", "b"]);
    let location = Location::parts(["m", "n"]);
    assert_eq!(parts(&location), ["m", "n"]);
    assert_eq!(parts(Location::parts(["m", "n"])), ["m", "n"]);
    assert_eq!(parts(Vec::<&str>::new()), Vec::<String>::new());
}

#[test]
fn a_url_or_a_with_clause_where_a_path_is_expected_is_refused_at_the_path() {
    let error = "'file:///lake/trades.parquet'"
        .into_object_path()
        .expect_err("a URL is no path of parts");
    assert_eq!(
        error.to_string(),
        "invalid record value at $.path: expected a path of parts, got the URL \
         file:///lake/trades.parquet"
    );
    let error = Location::Url(Url::from_str("s3://bucket/key").expect("a URL"))
        .into_object_path()
        .expect_err("a URL location is no path");
    assert!(error.to_string().contains("$.path"), "{error}");
    let error = "lake.eu with (media_type = 'text/csv')"
        .into_object_path()
        .expect_err("properties are no part of a path");
    assert_eq!(
        error.to_string(),
        "invalid record value at $.path: expected a path of parts, got a `with (...)` clause \
         on `lake.eu`"
    );
    let error = "select"
        .into_object_path()
        .expect_err("a section word is no part");
    assert!(
        error.to_string().starts_with(
            "invalid record value at $.path: expected a path of parts, got \"select\""
        ),
        "{error}"
    );
    let error = "a..b".into_object_path().expect_err("an empty part");
    assert!(error.to_string().contains("$.path"), "{error}");
}

#[test]
fn an_object_answers_its_kind_name_path_and_display_through_the_enum() {
    let catalog = Object::from(yggdryl::Catalog::Memory(
        MemoryCatalog::new("lake").with_description("the lake"),
    ));
    assert_eq!(catalog.kind(), IOKind::Catalog);
    assert_eq!(catalog.name(), "lake");
    assert_eq!(catalog.path(), ["lake"]);
    assert_eq!(catalog.description(), Some("the lake"));
    assert_eq!(catalog.url(), None);
    assert_eq!(catalog.modified(), None);
    assert_eq!(catalog.to_string(), "lake");
    assert!(catalog.as_catalog().is_some());
    assert!(catalog.as_namespace().is_none());
    assert!(catalog.as_table().is_none());

    let namespace = Object::from(yggdryl::Namespace::Memory(
        MemoryNamespace::new("lake.\"eu west\"").expect("a path"),
    ));
    assert_eq!(namespace.kind(), IOKind::Namespace);
    assert_eq!(namespace.name(), "eu west");
    assert_eq!(namespace.to_string(), "lake.\"eu west\"");
    assert!(namespace.as_namespace().is_some());

    let url = Url::from_str("file:///lake/eu/trades.csv").expect("a URL");
    let table = Object::from(yggdryl::Table::from(
        MediaTable::new("lake.eu.trades", url.clone()).expect("a table"),
    ));
    assert_eq!(table.kind(), IOKind::Table);
    assert_eq!(table.name(), "trades");
    assert_eq!(table.url(), Some(&url));
    assert_eq!(table.to_string(), "lake.eu.trades");
    assert!(table.as_table().is_some());
    assert_eq!(table.as_object().kind(), IOKind::Table);
}

#[test]
fn into_table_and_into_namespace_narrow_or_answer_absence_at_the_path() {
    let namespace = Object::from(yggdryl::Namespace::Memory(
        MemoryNamespace::new("lake.eu").expect("a path"),
    ));
    assert_eq!(
        namespace
            .clone()
            .into_table()
            .expect_err("a namespace is no table")
            .to_string(),
        "expected a table at \"lake.eu\", got nothing"
    );
    assert_eq!(
        namespace.into_namespace().expect("a namespace").to_string(),
        "lake.eu"
    );
    let url = Url::from_str("file:///lake/eu/trades.csv").expect("a URL");
    let table = Object::from(yggdryl::Table::from(
        MediaTable::new("lake.eu.trades", url).expect("a table"),
    ));
    assert_eq!(
        table
            .clone()
            .into_namespace()
            .expect_err("a table is no namespace")
            .to_string(),
        "expected a namespace at \"lake.eu.trades\", got nothing"
    );
    assert_eq!(table.into_table().expect("a table").name(), "trades");
}

#[test]
fn an_object_is_a_handle_of_its_kind() {
    let catalog = Object::from(yggdryl::Catalog::Memory(MemoryCatalog::new("lake")));
    assert!(matches!(catalog.clone().into_holder(), Holder::Catalog(_)));
    assert!(matches!(Holder::from(catalog), Holder::Catalog(_)));
    let namespace = Object::from(yggdryl::Namespace::Memory(
        MemoryNamespace::new("lake.eu").expect("a path"),
    ));
    assert!(matches!(namespace.into_holder(), Holder::Namespace(_)));
    let url = Url::from_str("file:///lake/eu/trades.csv").expect("a URL");
    let table = Object::from(yggdryl::Table::from(
        MediaTable::new("lake.eu.trades", url).expect("a table"),
    ));
    let held = table.into_holder();
    assert!(matches!(held, Holder::Table(_)));
    assert_eq!(
        held.url().map(ToString::to_string).as_deref(),
        Some("file:///lake/eu/trades.csv")
    );
}

#[test]
fn equality_and_hash_are_the_descriptions() {
    use std::hash::{DefaultHasher, Hash, Hasher};

    fn hashed(object: &Object) -> u64 {
        let mut hasher = DefaultHasher::new();
        object.hash(&mut hasher);
        hasher.finish()
    }

    let url = Url::from_str("file:///lake/eu/trades.csv").expect("a URL");
    let one = Object::from(yggdryl::Table::from(
        MediaTable::new("lake.eu.trades", url.clone()).expect("a table"),
    ));
    let same = Object::from(yggdryl::Table::from(
        MediaTable::new("lake.eu.trades", url.clone()).expect("a table"),
    ));
    let described = Object::from(yggdryl::Table::from(
        MediaTable::new("lake.eu.trades", url)
            .expect("a table")
            .with_properties(Properties::new().with_property("codec", "gzip")),
    ));
    assert_eq!(one, same);
    assert_ne!(
        one, described,
        "stated properties are part of the description"
    );
    assert_eq!(hashed(&one), hashed(&same));
    assert_ne!(hashed(&one), hashed(&described));
}

#[test]
fn update_properties_is_refused_by_implementation_where_nothing_is_kept() {
    let catalog = Object::from(yggdryl::Catalog::Memory(MemoryCatalog::new("lake")));
    let error = catalog
        .update_properties(&Properties::new().with_property("a", "1"), &[])
        .expect_err("a memory catalog keeps nothing");
    assert_eq!(
        error.to_string(),
        "filesystem \"MemoryCatalog\" does not support updating the properties it keeps"
    );
}

#[test]
fn objects_is_a_fused_lazy_listing() {
    let listing = Objects::empty();
    assert_eq!(listing.count(), 0);
    assert!(format!("{:?}", Objects::default()).contains("Objects"));
    let mut failing = Objects::failing(yggdryl::Error::absent("table", "x"));
    assert!(failing.next().expect("one entry").is_err());
    assert!(failing.next().is_none(), "fused after the failure");
    let collected: Objects = vec![
        Ok(Object::from(yggdryl::Catalog::Memory(MemoryCatalog::new(
            "a",
        )))),
        Err(yggdryl::Error::absent("table", "b")),
        Ok(Object::from(yggdryl::Catalog::Memory(MemoryCatalog::new(
            "c",
        )))),
    ]
    .into_iter()
    .collect();
    let seen: Vec<bool> = collected.map(|entry| entry.is_ok()).collect();
    assert_eq!(seen, [true, false], "a listing ends at its first failure");
    let handles = vec![Ok(Object::from(yggdryl::Catalog::Memory(
        MemoryCatalog::new("a"),
    )))]
    .into_iter()
    .collect::<Objects>()
    .into_listing(false)
    .count();
    assert_eq!(handles, 1);
}
