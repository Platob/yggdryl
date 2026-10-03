//! `rust/src/warehouse/system.rs`: the process's one warehouse.

use yggdryl::{
    CatalogValue, IOKind, MediaTable, Namespace, ObjectValue, SystemWarehouse, Table, TableValue,
    Url,
};

#[test]
fn the_system_warehouse_starts_with_the_local_catalog_of_folder_namespaces() {
    let local = SystemWarehouse::catalog("local").expect("the local catalog");
    assert_eq!(local.kind(), IOKind::Catalog);
    assert_eq!(local.namespace_levels(), None);
    let names: Vec<String> = local
        .namespaces()
        .iter()
        .map(|name| name.map(|name| name.to_string()))
        .collect::<yggdryl::Result<_>>()
        .expect("names");
    assert!(names.contains(&"temporary".to_owned()), "{names:?}");
    let temporary = SystemWarehouse::namespace("local.temporary").expect("the namespace");
    assert!(matches!(temporary, Namespace::Folder(_)));
    assert_eq!(
        ObjectValue::url(&temporary).map(|url| url.scheme().as_str()),
        Some("file")
    );
    assert!(
        SystemWarehouse::catalogs()
            .iter()
            .any(|catalog| catalog.name() == "local")
    );
    assert_eq!(
        SystemWarehouse::get("local").expect("the catalog").kind(),
        IOKind::Catalog
    );
    assert!(SystemWarehouse::with(|warehouse| warehouse
        .catalog("local")
        .is_ok()));
}

#[test]
fn registration_and_resolution_take_the_lock_per_call() {
    let name = format!("system_{}", std::process::id());
    let path = format!("{name}.eu.trades");
    let url = Url::from_str("file:///lake/eu/trades.csv").expect("a URL");
    SystemWarehouse::register(Table::from(
        MediaTable::new(path.as_str(), url.clone()).expect("a table"),
    ))
    .expect("registered");
    let table = SystemWarehouse::table(path.as_str()).expect("resolved");
    assert_eq!(table.to_string(), path);
    assert_eq!(table.storage(), "text/csv");
    assert_eq!(
        SystemWarehouse::properties_for(&url).len(),
        0,
        "nothing stated along the path"
    );
    assert!(
        SystemWarehouse::register(Table::from(
            MediaTable::new(path.as_str(), url.clone()).expect("a table")
        ))
        .expect_err("the name is taken")
        .is_conflict()
    );
    assert!(
        SystemWarehouse::replace(Table::from(
            MediaTable::new(path.as_str(), url).expect("a table")
        ))
        .expect("replaced")
        .is_some()
    );
    assert!(SystemWarehouse::with_mut(|warehouse| warehouse.catalog(&name).map(|_| ())).is_ok());
    let removed = SystemWarehouse::unregister(name.as_str()).expect("the catalog goes");
    assert_eq!(removed.kind(), IOKind::Catalog);
    assert!(SystemWarehouse::catalog(&name).is_err());
}
