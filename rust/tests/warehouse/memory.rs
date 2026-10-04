//! `rust/src/warehouse/memory.rs`: catalogs and namespaces of registered
//! objects, kept in order with no storage.

use yggdryl::{
    Catalog, CatalogValue, IOKind, MediaTable, MemoryCatalog, MemoryNamespace, Namespace,
    NamespaceValue, Object, ObjectValue, Properties, Table, Url,
};

fn table(path: &str, file: &str) -> Table {
    Table::from(
        MediaTable::new(
            path,
            Url::from_str(&format!("file:///lake/{file}")).expect("a URL"),
        )
        .expect("a table"),
    )
}

#[test]
fn a_memory_catalog_registers_objects_one_level_below_it_in_order() {
    let catalog = MemoryCatalog::new("lake")
        .with_object(table("lake.trades", "trades.csv"))
        .expect("registered")
        .with_object(Namespace::Memory(
            MemoryNamespace::new("lake.eu").expect("a path"),
        ))
        .expect("registered");
    assert_eq!(catalog.kind(), IOKind::Catalog);
    assert_eq!(
        catalog.namespace_levels(),
        None,
        "namespaces nest to any depth"
    );
    assert_eq!(catalog.registered().len(), 2);
    let names: Vec<String> = catalog
        .children()
        .map(|child| child.map(|child| child.to_string()))
        .collect::<yggdryl::Result<_>>()
        .expect("a listing");
    assert_eq!(names, ["lake.trades", "lake.eu"], "registration order");
    assert_eq!(
        catalog.get("eu").expect("the namespace").kind(),
        IOKind::Namespace
    );
    assert_eq!(
        catalog
            .get("fills")
            .expect_err("nothing registered")
            .to_string(),
        "expected a child at \"lake.fills\", got nothing"
    );
    let catalog = Catalog::Memory(catalog);
    assert_eq!(
        catalog.table("trades").expect("the table").to_string(),
        "lake.trades"
    );
    assert_eq!(
        catalog
            .table("eu")
            .expect_err("a namespace is no table")
            .to_string(),
        "expected a table at \"lake.eu\", got nothing"
    );
}

#[test]
fn a_name_taken_at_a_level_is_a_conflict_and_a_misplaced_path_is_refused() {
    let catalog = MemoryCatalog::new("lake")
        .with_object(table("lake.trades", "trades.csv"))
        .expect("registered");
    let error = catalog
        .clone()
        .with_object(Namespace::Memory(
            MemoryNamespace::new("lake.trades").expect("a path"),
        ))
        .expect_err("the name is taken");
    assert_eq!(
        error.to_string(),
        "expected to create a namespace at \"lake.trades\", got an existing table"
    );
    let error = catalog
        .clone()
        .with_object(table("other.trades", "x.csv"))
        .expect_err("another catalog's object");
    assert_eq!(
        error.to_string(),
        "invalid record value at $.other.trades: expected an object one level below `lake`, \
         got `other.trades`"
    );
    let error = catalog
        .with_object(table("lake.eu.trades", "x.csv"))
        .expect_err("two levels down");
    assert!(
        error.to_string().contains("one level below `lake`"),
        "{error}"
    );
    let error = MemoryNamespace::new("alone").expect_err("a namespace sits under a catalog");
    assert!(error.to_string().contains("at least two parts"), "{error}");
}

#[test]
fn stated_properties_propagate_down_registered_levels() {
    let eu = MemoryNamespace::new("lake.eu")
        .expect("a path")
        .with_properties(Properties::new().with_property("region", "eu-west-1"))
        .with_object(
            table("lake.eu.trades", "eu/trades.csv")
                .into_media_properties(Properties::new().with_property("codec", "gzip")),
        )
        .expect("registered");
    let catalog = Catalog::Memory(
        MemoryCatalog::new("lake")
            .with_description("the lake")
            .with_properties(
                Properties::new()
                    .with_property("token", "t")
                    .with_property("region", "global"),
            )
            .with_object(Namespace::Memory(eu))
            .expect("registered"),
    );
    assert_eq!(
        catalog
            .properties()
            .expect("stated")
            .iter()
            .collect::<Vec<_>>(),
        [("token", "t"), ("region", "global")]
    );
    let eu = catalog.namespace("eu").expect("the namespace");
    assert_eq!(
        eu.properties()
            .expect("effective")
            .iter()
            .collect::<Vec<_>>(),
        [("token", "t"), ("region", "eu-west-1")],
        "the parent's order, the namespace's own value"
    );
    let trades = catalog.table("eu.trades").expect("the table");
    assert_eq!(
        trades
            .properties()
            .expect("effective")
            .iter()
            .collect::<Vec<_>>(),
        [("token", "t"), ("region", "eu-west-1"), ("codec", "gzip")]
    );
    assert_eq!(
        eu.resolve("trades")
            .expect("the same table")
            .properties()
            .expect("effective")
            .get("region"),
        Some("eu-west-1")
    );
    for child in eu.children() {
        assert_eq!(
            child
                .expect("a child")
                .properties()
                .expect("effective")
                .get("token"),
            Some("t")
        );
    }
}

#[test]
fn a_memory_catalog_creates_nothing() {
    let catalog = Catalog::Memory(MemoryCatalog::new("lake"));
    let error = catalog
        .create_namespace("eu", &Properties::new())
        .expect_err("no storage");
    assert_eq!(
        error.to_string(),
        "filesystem \"MemoryCatalog\" does not support creating a namespace"
    );
    let field = yggdryl::DataType::Int64.required_field("id");
    let error = catalog
        .create_table("t", &field, &Properties::new())
        .expect_err("no storage");
    assert_eq!(
        error.to_string(),
        "filesystem \"MemoryCatalog\" does not support creating a table"
    );
    let namespace = Namespace::Memory(MemoryNamespace::new("lake.eu").expect("a path"));
    let error = namespace
        .create_table("t", &field, &Properties::new())
        .expect_err("no storage");
    assert_eq!(
        error.to_string(),
        "filesystem \"MemoryNamespace\" does not support creating a table"
    );
    assert_eq!(
        catalog
            .tables()
            .create("t", &field, &Properties::new())
            .expect_err("no storage")
            .to_string(),
        "filesystem \"MemoryCatalog\" does not support creating a table"
    );
}

#[test]
fn equality_follows_the_registered_objects() {
    let one = MemoryCatalog::new("lake")
        .with_object(table("lake.trades", "trades.csv"))
        .expect("registered");
    let same = MemoryCatalog::new("lake")
        .with_object(table("lake.trades", "trades.csv"))
        .expect("registered");
    let other = MemoryCatalog::new("lake")
        .with_object(table("lake.trades", "other.csv"))
        .expect("registered");
    assert_eq!(one, same);
    assert_ne!(one, other);
    assert_eq!(
        Object::from(Catalog::Memory(one.clone())),
        Object::from(Catalog::Memory(same))
    );
    let _ = one;
}

/// A table's stated properties, through its own builder.
trait WithProperties {
    fn into_media_properties(self, properties: Properties) -> Self;
}

impl WithProperties for Table {
    fn into_media_properties(self, properties: Properties) -> Self {
        match self {
            Self::Media(table) => Self::Media(Box::new(table.with_properties(properties))),
            other => other,
        }
    }
}
