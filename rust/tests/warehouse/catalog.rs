//! `rust/src/warehouse/catalog.rs`: the first namespace layer, the enum over
//! its implementations, and `Catalog::from_url`.

use std::path::PathBuf;

use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::{
    Catalog, CatalogValue, FolderCatalog, IOBase, IOKind, IOMedia, MemoryCatalog, NamespaceValue,
    ObjectValue, Properties, Url,
};

fn root(label: &str) -> PathBuf {
    let root = LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path")
        .join(format!(
            "yggdryl-warehouse-catalog-{label}-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("eu")).expect("folders");
    std::fs::write(root.join("eu/trades.csv"), "symbol,price\nAAPL,1\n").expect("a leaf");
    std::fs::write(root.join("notes.csv"), "a\n1\n").expect("a leaf");
    root
}

#[test]
fn from_url_answers_a_folder_catalog_named_after_the_last_segment() {
    let root = root("from-url");
    let url = Url::from_path(&root).expect("a URL");
    let catalog = Catalog::from_url(&url, &Properties::new()).expect("a catalog");
    assert!(matches!(catalog, Catalog::Folder(_)));
    assert_eq!(catalog.name(), url.file_name().expect("a segment"));
    assert_eq!(catalog.namespace_levels(), Some(1));
    assert_eq!(ObjectValue::url(&catalog), Some(&url));
    assert_eq!(
        catalog.table("eu.trades").expect("the table").to_string(),
        format!("\"{}\".eu.trades", catalog.name())
    );
    let trailing = Url::from_str(&format!("{url}/")).expect("a URL");
    assert_eq!(
        Catalog::from_url(&trailing, &Properties::new())
            .expect("a catalog")
            .name(),
        catalog.name(),
        "a trailing slash names the same folder"
    );
}

#[test]
fn from_url_reads_the_name_and_the_type_properties_and_passes_the_rest_on() {
    let root = root("typed");
    let url = Url::from_path(&root).expect("a URL");
    let properties = Properties::new()
        .with_property("name", "market")
        .with_property("type", "folder")
        .with_property("region", "eu-west-1");
    let catalog = Catalog::from_url(&url, &properties).expect("a catalog");
    assert_eq!(catalog.name(), "market");
    assert_eq!(
        catalog.properties().expect("stated"),
        properties,
        "every property travels on, the read ones included"
    );
    assert_eq!(
        catalog
            .table("eu.trades")
            .expect("the table")
            .properties()
            .expect("inherited")
            .get("region"),
        Some("eu-west-1")
    );

    let memory = Catalog::from_url(
        &url,
        &Properties::new()
            .with_property("type", "memory")
            .with_property("name", "scratch"),
    )
    .expect("a memory catalog");
    assert!(matches!(memory, Catalog::Memory(_)));
    assert_eq!(memory.name(), "scratch");
    assert_eq!(memory.namespace_levels(), None);
    assert!(
        memory.children().next().is_none(),
        "a memory catalog lists what is registered"
    );
}

#[test]
fn from_url_refuses_a_type_this_build_has_no_catalog_for_and_a_nameless_url() {
    let root = root("refused");
    let url = Url::from_path(&root).expect("a URL");
    // A build that reads Iceberg answers `hadoop`; every other build refuses
    // it with the rest, and the refusal lists what the build answers.
    let (refused, expected): (&[&str], &str) = if cfg!(feature = "iceberg") {
        (&["rest", "xmla"], "`memory`, `folder` or `hadoop`")
    } else {
        (&["hadoop", "rest", "xmla"], "`memory` or `folder`")
    };
    for kind in refused {
        let error = Catalog::from_url(&url, &Properties::new().with_property("type", *kind))
            .expect_err(kind);
        assert_eq!(
            error.to_string(),
            format!(
                "invalid record value at $.with.type: expected {expected}, got `{kind}`; this \
                 build has no catalog of that type"
            )
        );
    }
    let error = Catalog::from_url(&url, &Properties::new().with_property("type", "glue"))
        .expect_err("unknown");
    assert_eq!(
        error.to_string(),
        format!("invalid record value at $.with.type: expected {expected}, got \"glue\"")
    );
    // A table bucket's location is its S3 Tables catalog where the build
    // has one, which `rust/tests/s3tables/catalog.rs` pins, and refused by
    // name where it does not.
    #[cfg(not(feature = "s3tables"))]
    {
        let error = Catalog::from_url(
            Url::from_str("s3tables://lake").expect("a URL"),
            &Properties::new(),
        )
        .expect_err("no S3 Tables catalog in this build");
        assert_eq!(
            error.to_string(),
            "filesystem \"s3tables\" does not support holding an S3 Tables catalog in this build"
        );
    }
    let error = Catalog::from_url(
        Url::from_str("file:///").expect("a URL"),
        &Properties::new(),
    )
    .expect_err("no segment to name it by");
    assert!(error.to_string().contains("$.with.name"), "{error}");
    assert_eq!(
        Catalog::from_url(
            Url::from_str("s3://bucket").expect("a URL"),
            &Properties::new().with_property("type", "memory")
        )
        .expect("the host names it")
        .name(),
        "bucket"
    );
}

#[test]
fn a_catalog_is_a_container_handle_whose_children_are_handles() {
    let root = root("handle");
    let catalog = Catalog::Folder(Box::new(FolderCatalog::bound(
        "market",
        Holder::folder(&root).expect("holds"),
    )));
    assert_eq!(IOBase::kind(&catalog), IOKind::Catalog);
    assert!(IOBase::is_container(&catalog));
    assert!(!IOBase::is_atomic(&catalog));
    assert!(!IOBase::is_tabular(&catalog));
    assert_eq!(IOBase::size(&catalog), 0);
    assert_eq!(
        IOBase::media_type(&catalog).base(),
        &yggdryl::MimeType::DIRECTORY
    );
    assert_eq!(IOBase::url(&catalog), ObjectValue::url(&catalog));
    assert!(IOBase::uri(&catalog).is_some());
    let children: Vec<(IOKind, String)> = IOBase::ls(&catalog, false, false)
        .map(|child| {
            child.map(|child| {
                (
                    child.kind(),
                    child
                        .url()
                        .and_then(Url::file_name)
                        .unwrap_or("")
                        .to_owned(),
                )
            })
        })
        .collect::<yggdryl::Result<_>>()
        .expect("handles");
    assert_eq!(
        children,
        [
            (IOKind::Namespace, "eu".to_owned()),
            (IOKind::File, "notes.csv".to_owned())
        ],
        "a namespace is its own handle, a leaf table its handle's kind"
    );
    let recursive = IOBase::ls(&catalog, true, false).count();
    assert_eq!(recursive, 3, "a recursive listing descends the namespace");
    let trades = IOBase::child_by_path(&catalog, "eu/trades").expect("resolved by parts");
    assert!(matches!(trades, Holder::Table(_)));
    assert_eq!(trades.row_size().expect("one row"), 1);
    let eu = IOBase::child_by_path(&catalog, "eu").expect("the namespace");
    assert!(matches!(eu, Holder::Namespace(_)));
    let itself = IOBase::child_by_path(&catalog, "").expect("the root");
    assert!(matches!(itself, Holder::Catalog(_)));
    assert!(
        IOBase::child_by_path(&catalog, "asia/x")
            .expect_err("nothing")
            .is_absent()
    );
}

#[test]
fn a_catalogs_byte_verbs_are_not_atomic_and_its_record_verbs_name_a_table() {
    let root = root("verbs");
    let mut catalog = Catalog::Folder(Box::new(FolderCatalog::bound(
        "market",
        Holder::folder(&root).expect("holds"),
    )));
    let error = IOBase::read_all_bytes(&catalog).expect_err("no bytes");
    assert!(
        matches!(error, yggdryl::Error::NotAtomic { .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("got a catalog"), "{error}");
    assert!(matches!(
        IOBase::write_all_bytes(&mut catalog, b"x"),
        Err(yggdryl::Error::NotAtomic { .. })
    ));
    assert!(matches!(
        IOBase::pread(&catalog, 0, &mut [0; 4]),
        Err(yggdryl::Error::NotAtomic { .. })
    ));
    let error = catalog.record_options().expect_err("no table");
    assert_eq!(
        error.to_string(),
        "invalid record value at $.market: expected a table, got the catalog `market`; name a \
         table under it"
    );
    assert!(catalog.read_serie(None).is_err());
    assert!(
        IOBase::remove(&mut catalog, true).is_err(),
        "a catalog is not removed through its handle"
    );
    let mut memory = Catalog::Memory(MemoryCatalog::new("lake"));
    assert!(IOBase::flush(&mut memory).is_ok());
    assert!(IOBase::open(&mut memory).is_ok());
    assert!(IOBase::close(&mut memory).is_ok());
    assert!(!IOBase::opened(&memory));
}

#[test]
fn display_from_and_the_views_read_through_the_enum() {
    let root = root("enum");
    let folder: Catalog =
        FolderCatalog::bound("market", Holder::folder(&root).expect("holds")).into();
    assert_eq!(folder.to_string(), "market");
    assert_eq!(
        folder
            .namespaces()
            .iter()
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("names"),
        ["eu"]
    );
    assert_eq!(
        folder
            .tables()
            .iter()
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("names"),
        ["notes"]
    );
    assert_eq!(
        folder.resolve("eu").expect("the namespace").kind(),
        IOKind::Namespace
    );
    assert_eq!(
        folder
            .namespace("eu.trades")
            .expect_err("a table is no namespace")
            .to_string(),
        "expected a namespace at \"market.eu.trades\", got nothing"
    );
    let memory: Catalog = MemoryCatalog::new("lake").into();
    assert_eq!(memory.to_string(), "lake");
    assert_eq!(memory.as_catalog().name(), "lake");
}
