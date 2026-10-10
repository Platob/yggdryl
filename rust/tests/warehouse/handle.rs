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

/// An object a caller's object-store folder roots keeps that folder's own
/// client: a listed table and its clone resolve against the endpoint the
/// folder was built with, signed with its key pair, and building either -
/// listing aside - sends nothing. Before, a clone was rebuilt from the
/// location under default options, reaching another store.
#[cfg(feature = "s3")]
#[test]
fn a_clone_of_an_object_on_a_callers_store_folder_keeps_its_client() {
    use yggdryl::s3::{self, Credentials, S3Options};

    let store = crate::server::FakeS3::start();
    store.create_bucket("market");
    store.put(
        "market",
        "lake/eu/trades.csv",
        b"symbol,price
AAPL,1
",
    );
    let options = S3Options::default()
        .with_environment(false)
        .with_endpoint(store.endpoint())
        .with_region("us-east-1")
        .with_path_style(true)
        .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"));
    let catalog = Catalog::Folder(Box::new(FolderCatalog::bound(
        "market",
        Holder::from(s3::folder_with("s3://market/lake", options).expect("a folder")),
    )));
    let table = catalog.table("eu.trades").expect("the table");
    store.clear_requests();
    let twin = table.clone();
    assert!(!IOBase::opened(&twin), "a clone starts unresolved");
    assert_eq!(store.request_count(), 0, "a clone sends nothing");

    assert_eq!(twin.row_size().expect("one row"), 1);
    let requests = store.requests();
    assert!(!requests.is_empty(), "the clone read the fake store");
    for request in &requests {
        assert!(
            request.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("authorization")
                    && value.contains("Credential=AKIAIOSFODNN7EXAMPLE/")
            }),
            "signed with the folder's key pair: {request:?}"
        );
    }
}
