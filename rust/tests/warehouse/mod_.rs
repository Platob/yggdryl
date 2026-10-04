//! `rust/src/warehouse/mod.rs`: the registry of catalogs a path resolves
//! against.

use std::path::PathBuf;

use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::{
    Catalog, FolderCatalog, IOKind, IOMedia, MediaTable, MemoryCatalog, MemoryNamespace, Namespace,
    Object, ObjectValue, Properties, Table, TableValue, Url, Warehouse,
};

fn root(label: &str) -> PathBuf {
    let root = LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path")
        .join(format!(
            "yggdryl-warehouse-mod-{label}-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("eu")).expect("folders");
    std::fs::write(root.join("eu/trades.csv"), "symbol,price\nAAPL,1\n").expect("a leaf");
    root
}

fn table(path: &str, url: &str) -> Table {
    Table::from(MediaTable::new(path, Url::from_str(url).expect("a URL")).expect("a table"))
}

#[test]
fn registering_a_table_at_a_path_builds_the_memory_levels_along_it() {
    let mut warehouse = Warehouse::new();
    assert!(warehouse.catalogs().is_empty());
    warehouse
        .register(table("lake.eu.trades", "file:///lake/eu/trades.csv"))
        .expect("registered");
    assert_eq!(warehouse.catalogs().len(), 1);
    let lake = warehouse.catalog("lake").expect("built along the path");
    assert!(matches!(lake, Catalog::Memory(_)));
    assert_eq!(
        warehouse.get("lake").expect("the catalog").kind(),
        IOKind::Catalog
    );
    let eu = warehouse
        .namespace("lake.eu")
        .expect("built along the path");
    assert!(matches!(eu, Namespace::Memory(_)));
    assert_eq!(
        warehouse
            .table("lake.eu.trades")
            .expect("the table")
            .to_string(),
        "lake.eu.trades"
    );
    warehouse
        .register(table("lake.eu.fills", "file:///lake/eu/fills.csv"))
        .expect("a sibling joins the level");
    assert_eq!(
        warehouse
            .namespace("lake.eu")
            .expect("the namespace")
            .tables()
            .iter()
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("names"),
        ["trades", "fills"],
        "registration order"
    );
    warehouse
        .register(Namespace::Memory(
            MemoryNamespace::new("lake.asia.jp").expect("a path"),
        ))
        .expect("a namespace two levels down");
    assert_eq!(
        warehouse.get("lake.asia.jp").expect("nested").kind(),
        IOKind::Namespace
    );
    assert_eq!(warehouse, warehouse.clone());
    assert_eq!(Warehouse::default(), Warehouse::new());
}

#[test]
fn a_taken_name_conflicts_and_a_store_backed_catalog_is_not_registered_under() {
    let root = root("conflicts");
    let mut warehouse = Warehouse::new();
    warehouse
        .register(FolderCatalog::bound(
            "market",
            Holder::folder(&root).expect("holds"),
        ))
        .expect("registered");
    let error = warehouse
        .register(MemoryCatalog::new("market"))
        .expect_err("the name is taken");
    assert_eq!(
        error.to_string(),
        "expected to create a catalog at \"market\", got an existing catalog"
    );
    let error = warehouse
        .register(table("market.eu.fills", "file:///x/fills.csv"))
        .expect_err("a folder catalog lists its own store");
    assert_eq!(
        error.to_string(),
        "filesystem \"FolderCatalog `market`\" does not support registering under an object that \
         lists its own store"
    );
    warehouse
        .register(table("lake.trades", "file:///lake/trades.csv"))
        .expect("registered");
    let error = warehouse
        .register(table("lake.trades", "file:///lake/other.csv"))
        .expect_err("the name is taken");
    assert_eq!(
        error.to_string(),
        "expected to create a table at \"lake.trades\", got an existing table"
    );
    let error = warehouse
        .register(table("lake.trades.x", "file:///lake/x.csv"))
        .expect_err("a table is no level to register under");
    assert_eq!(
        error.to_string(),
        "filesystem \"MediaTable `lake.trades`\" does not support registering under an object \
         that lists its own store"
    );
    let error = warehouse
        .register(Object::from(Namespace::Memory(
            MemoryNamespace::new("lake.eu").expect("a path"),
        )))
        .map(|()| warehouse.register(table("lake.eu", "file:///lake/eu.csv")))
        .expect("the namespace registers")
        .expect_err("a table of the namespace's name");
    assert_eq!(
        error.to_string(),
        "expected to create a table at \"lake.eu\", got an existing namespace"
    );
}

#[test]
fn replace_swaps_the_object_of_a_name_and_unregister_removes_it() {
    let mut warehouse = Warehouse::new();
    warehouse
        .register(table("lake.trades", "file:///lake/trades.csv"))
        .expect("registered");
    let replaced = warehouse
        .replace(table("lake.trades", "file:///lake/trades.parquet"))
        .expect("replaced")
        .expect("the earlier table");
    assert_eq!(
        replaced.url().map(ToString::to_string).as_deref(),
        Some("file:///lake/trades.csv")
    );
    assert_eq!(
        warehouse
            .table("lake.trades")
            .expect("the later table")
            .url()
            .map(ToString::to_string)
            .as_deref(),
        Some("file:///lake/trades.parquet")
    );
    assert!(
        warehouse
            .replace(table("lake.fills", "file:///lake/fills.csv"))
            .expect("registered")
            .is_none(),
        "nothing to replace"
    );
    assert!(
        warehouse
            .replace(MemoryCatalog::new("other"))
            .expect("registered")
            .is_none()
    );
    assert_eq!(
        warehouse
            .replace(MemoryCatalog::new("other").with_description("two"))
            .expect("replaced")
            .map(|old| old.description().map(str::to_owned)),
        Some(None)
    );
    let removed = warehouse.unregister("lake.trades").expect("removed");
    assert_eq!(removed.kind(), IOKind::Table);
    assert!(
        warehouse
            .table("lake.trades")
            .expect_err("gone")
            .is_absent()
    );
    assert_eq!(
        warehouse
            .unregister("lake.trades")
            .expect_err("already gone")
            .to_string(),
        "expected a child at \"lake.trades\", got nothing"
    );
    assert_eq!(
        warehouse
            .unregister("nowhere.x")
            .expect_err("no catalog")
            .to_string(),
        "expected a catalog at \"nowhere\", got nothing"
    );
    let catalog = warehouse
        .unregister("other")
        .expect("a catalog by its name");
    assert_eq!(catalog.kind(), IOKind::Catalog);
    assert!(warehouse.catalog("other").is_err());
    assert!(
        warehouse.unregister("").is_err(),
        "the empty path names nothing"
    );
}

#[test]
fn resolution_names_the_catalog_or_the_path_that_is_missing() {
    let root = root("resolve");
    let mut warehouse = Warehouse::new();
    warehouse
        .register(FolderCatalog::bound(
            "market",
            Holder::folder(&root).expect("holds"),
        ))
        .expect("registered");
    assert_eq!(
        warehouse.get("nope").expect_err("no catalog").to_string(),
        "expected a catalog at \"nope\", got nothing"
    );
    assert_eq!(
        warehouse
            .table("nope.eu.trades")
            .expect_err("no catalog")
            .to_string(),
        "expected a catalog at \"nope\", got nothing"
    );
    assert_eq!(
        warehouse
            .table("market.eu.nowhere")
            .expect_err("no table")
            .to_string(),
        "expected a table at \"market.eu.nowhere\", got nothing"
    );
    assert_eq!(
        warehouse
            .table("market.eu")
            .expect_err("a namespace")
            .to_string(),
        "expected a table at \"market.eu\", got nothing"
    );
    assert_eq!(
        warehouse
            .table("market")
            .expect_err("a catalog")
            .to_string(),
        "expected a table at \"market\", got nothing"
    );
    assert_eq!(
        warehouse
            .namespace("market")
            .expect_err("a catalog is no namespace")
            .to_string(),
        "expected a namespace at \"market\", got nothing"
    );
    assert!(warehouse.get("").is_err(), "the empty path names nothing");
    let trades = warehouse.table("market.eu.trades").expect("the table");
    assert_eq!(trades.storage(), "text/csv");
    assert_eq!(trades.row_size().expect("one row"), 1);
    assert_eq!(
        warehouse.get(["market", "eu"]).expect("parts").kind(),
        IOKind::Namespace
    );
}

#[test]
fn properties_for_answers_the_deepest_registered_object_holding_a_url() {
    let mut warehouse = Warehouse::new();
    let lake = Url::from_str("file:///a/lake").expect("a URL");
    warehouse
        .register(
            FolderCatalog::new("lake", lake)
                .expect("a catalog")
                .with_properties(Properties::new().with_property("region", "lake")),
        )
        .expect("registered");
    warehouse
        .register(Table::from(
            MediaTable::new(
                "mem.eu.trades",
                Url::from_str("file:///a/lake/eu/trades.csv").expect("a URL"),
            )
            .expect("a table")
            .with_properties(
                Properties::new()
                    .with_property("region", "table")
                    .with_property("codec", "gzip"),
            ),
        ))
        .expect("registered");
    let at = |text: &str| warehouse.properties_for(&Url::from_str(text).expect("a URL"));
    assert_eq!(
        at("file:///a/lake/eu/fills.csv").get("region"),
        Some("lake")
    );
    assert_eq!(
        at("file:///a/lake").get("region"),
        Some("lake"),
        "the boundary itself"
    );
    assert_eq!(at("file:///a/lake?x=1").get("region"), Some("lake"));
    assert_eq!(at("file:///a/lake#part").get("region"), Some("lake"));
    assert!(
        at("file:///a/lakehouse/x.csv").is_empty(),
        "a prefix matches on a path boundary only"
    );
    assert!(at("file:///elsewhere").is_empty());
    assert_eq!(
        at("file:///a/lake/eu/trades.csv")
            .iter()
            .collect::<Vec<_>>(),
        [("region", "table"), ("codec", "gzip")],
        "the deepest object wins"
    );
    assert_eq!(
        at("file:///a/lake/eu/trades.csv/part").get("codec"),
        Some("gzip"),
        "below the table's URL"
    );
}
