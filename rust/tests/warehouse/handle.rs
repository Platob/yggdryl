//! `rust/src/warehouse/handle.rs`: the storage handle an object resolves once
//! and keeps, reached through the objects that hold one - a clone starts
//! unresolved, a bound handle rebuilds from its site, and an unlocated one
//! says so by name.

use std::path::PathBuf;

use yggdryl::holder::{Buffer, Holder};
use yggdryl::local::LocalFolder;
use yggdryl::{
    Catalog, FolderCatalog, FolderNamespace, IOBase, IOMedia, MediaTable, MimeType, Namespace,
    NamespaceValue, ObjectValue, Url,
};

fn root(label: &str) -> PathBuf {
    let root = LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path")
        .join(format!(
            "yggdryl-warehouse-handle-{label}-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("eu")).expect("folders");
    std::fs::write(root.join("eu/trades.csv"), "symbol,price\nAAPL,1\n").expect("a leaf");
    root
}

#[test]
fn a_clone_of_a_located_object_starts_unresolved_and_reads_the_same_store() {
    let root = root("located");
    let catalog = Catalog::Folder(Box::new(FolderCatalog::bound(
        "market",
        Holder::folder(&root).expect("holds"),
    )));
    let table = catalog.table("eu.trades").expect("the table");
    assert_eq!(table.row_size().expect("one row"), 1);
    let twin = table.clone();
    assert!(!IOBase::opened(&twin), "a clone starts unresolved");
    assert_eq!(twin, table);
    assert_eq!(twin.row_size().expect("the same store"), 1);
    assert_eq!(
        ObjectValue::url(&twin),
        ObjectValue::url(&table),
        "the site is the description's"
    );
    let namespace = catalog.namespace("eu").expect("the namespace").clone();
    assert_eq!(namespace.children().count(), 1);
    assert_eq!(
        catalog
            .clone()
            .table("eu.trades")
            .expect("the table")
            .name(),
        "trades"
    );
}

#[test]
fn an_object_bound_to_a_foreign_filesystem_rebuilds_on_the_same_binding() {
    let (_, folder) = crate::counting_filesystem::counted_folder("lake");
    let mut leaf = folder.child_by_path("eu/trades.csv").expect("a child");
    leaf.write_all_bytes(b"symbol,price\nAAPL,1\n")
        .expect("written");
    let catalog = Catalog::Folder(Box::new(FolderCatalog::bound(
        "market",
        Holder::FsFolder(folder),
    )));
    let table = catalog.table("eu.trades").expect("the table");
    assert!(IOBase::bound_location(&table).is_some());
    let twin = table.clone();
    assert_eq!(twin.row_size().expect("rebuilt on the bound filesystem"), 1);
    assert_eq!(twin, table);
}

#[test]
fn an_object_bound_to_an_unlocated_handle_refuses_by_name_after_a_clone() {
    let mut buffer = Buffer::new().with_media_type(MimeType::CSV.into());
    buffer
        .write_all_bytes(b"symbol,price\nAAPL,1\n")
        .expect("written");
    let table = MediaTable::bound("memory.trades", Holder::buffer(buffer)).expect("a table");
    assert_eq!(table.row_size().expect("the held bytes"), 1);
    let twin = table.clone();
    let error = twin.row_size().expect_err("nothing to rebuild from");
    assert!(
        error
            .to_string()
            .contains("expected a located handle to rebuild `memory.trades` from"),
        "{error}"
    );
    let namespace = FolderNamespace::bound(["local", "scratch"], Holder::buffer(Buffer::new()))
        .expect("a namespace");
    assert_eq!(namespace.children().count(), 0, "a buffer lists nothing");
    let twin = Namespace::Folder(Box::new(namespace.clone()));
    let error = twin
        .children()
        .next()
        .expect("one failing entry")
        .expect_err("nothing to rebuild from");
    assert!(
        error.to_string().contains("rebuild `local.scratch` from"),
        "{error}"
    );
    let error = twin.get("x").expect_err("nothing to rebuild from");
    assert!(
        !error.is_absent(),
        "a missing handle is not absence: {error}"
    );
}

#[test]
fn a_located_namespace_resolves_its_handle_under_its_effective_properties() {
    let root = root("properties");
    let url = Url::from_path(root.join("eu")).expect("a URL");
    let namespace = Namespace::Folder(Box::new(
        FolderNamespace::new("market.eu", url.clone()).expect("a namespace"),
    ));
    assert_eq!(ObjectValue::url(&namespace), Some(&url));
    assert_eq!(
        namespace
            .tables()
            .iter()
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("names"),
        ["trades"]
    );
    assert!(namespace.modified().is_some());
}
