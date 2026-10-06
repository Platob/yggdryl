//! `rust/src/warehouse/namespace.rs`: a container of namespaces and tables,
//! the enum over its implementations, and the two collection views.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::{
    Catalog, DataType, FolderCatalog, IOBase, IOKind, IOMedia, MediaTable, MemoryCatalog,
    MemoryNamespace, Names, Namespace, NamespaceValue, ObjectValue, Properties, StructType, Table,
    TableValue, Url,
};

fn root(label: &str) -> PathBuf {
    let root = LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path")
        .join(format!(
            "yggdryl-warehouse-namespace-{label}-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("eu/fx")).expect("folders");
    std::fs::write(root.join("eu/trades.csv"), "symbol,price\nAAPL,1\n").expect("a leaf");
    std::fs::write(root.join("eu/fx/spot.csv"), "pair,rate\nEURUSD,1.1\n").expect("a leaf");
    root
}

fn folder(label: &str) -> Catalog {
    Catalog::Folder(Box::new(FolderCatalog::bound(
        "market",
        Holder::folder(root(label)).expect("holds"),
    )))
}

#[test]
fn a_namespace_resolves_paths_below_it_and_lists_its_children() {
    let catalog = folder("resolve");
    let eu = catalog.namespace("eu").expect("the namespace");
    assert_eq!(ObjectValue::kind(&eu), IOKind::Namespace);
    assert_eq!(eu.name(), "eu");
    assert_eq!(eu.path(), ["market", "eu"]);
    assert_eq!(eu.to_string(), "market.eu");
    assert!(ObjectValue::url(&eu).is_some());
    let children: Vec<String> = eu
        .children()
        .map(|child| child.map(|child| child.to_string()))
        .collect::<yggdryl::Result<_>>()
        .expect("a listing");
    assert_eq!(children, ["market.eu.fx", "market.eu.trades"]);
    assert_eq!(
        eu.resolve("trades").expect("the table").kind(),
        IOKind::Table
    );
    assert_eq!(
        eu.resolve(["fx"]).expect("a folder table").to_string(),
        "market.eu.fx"
    );
    assert_eq!(
        eu.resolve("")
            .expect_err("the empty path names nothing below")
            .to_string(),
        "invalid record value at $.market.eu: expected at least one part below the namespace, \
         got none"
    );
    assert_eq!(
        eu.resolve("trades.x")
            .expect_err("a path never descends into a table")
            .to_string(),
        "expected a namespace at \"market.eu.trades\", got nothing"
    );
    assert_eq!(
        eu.get("nowhere").expect_err("absent").to_string(),
        "expected a table at \"market.eu.nowhere\", got nothing"
    );
}

#[test]
fn a_namespace_is_a_container_handle() {
    let catalog = folder("handle");
    let mut eu = catalog.namespace("eu").expect("the namespace");
    assert_eq!(IOBase::kind(&eu), IOKind::Namespace);
    assert!(IOBase::is_container(&eu));
    assert!(!IOBase::is_atomic(&eu));
    assert!(!IOBase::is_tabular(&eu));
    assert_eq!(IOBase::ls(&eu, false, false).count(), 2);
    assert!(matches!(
        IOBase::child_by_path(&eu, "trades").expect("a handle"),
        Holder::Table(_)
    ));
    assert!(matches!(
        IOBase::read_all_bytes(&eu),
        Err(yggdryl::Error::NotAtomic { .. })
    ));
    assert_eq!(
        eu.read_arrow_field(
            &yggdryl::media::RecordOptions::for_mime_type(&yggdryl::MimeType::ARROW_STREAM)
                .expect("options")
        )
        .expect_err("no table")
        .to_string(),
        "invalid record value at $.market.eu: expected a table, got the namespace `market.eu`; \
         name a table under it"
    );
    assert!(IOBase::clear(&mut eu).is_err());
    let held = Holder::from(eu.clone());
    assert!(matches!(held, Holder::Namespace(_)));
    assert!(held.exists());
    assert_eq!(held.ls(false, false).count(), 2);
}

#[test]
fn the_views_descend_dotted_names_and_refuse_the_empty_one() {
    let catalog = folder("views");
    let namespaces = catalog.namespaces();
    assert_eq!(namespaces.get("eu").expect("one level").name(), "eu");
    assert!(
        namespaces
            .get("eu.fx")
            .expect_err("fx is a folder table under the default levels")
            .is_absent()
    );
    let tables = catalog.tables();
    assert_eq!(
        tables.get("eu.trades").expect("two levels").name(),
        "trades"
    );
    assert_eq!(
        tables.get("eu.fx").expect("a folder table").storage(),
        "directory"
    );
    for error in [
        tables.get("").expect_err("the empty path"),
        namespaces.get("  ").expect_err("the empty path"),
    ] {
        assert!(
            error
                .to_string()
                .contains("expected a name, got the empty path"),
            "{error}"
        );
    }
    assert!(
        !tables.contains("").is_ok(),
        "the empty name is a refusal, not an absence"
    );
    assert!(
        format!("{namespaces:?}").contains("Namespaces"),
        "the views are described"
    );
    assert!(format!("{tables:?}").contains("Tables"));
}

#[test]
fn names_is_a_fused_lazy_listing() {
    let mut names = Names::empty();
    assert!(names.next().is_none());
    let mut failing = Names::failing(yggdryl::Error::absent("table", "x"));
    assert!(failing.next().expect("the failure").is_err());
    assert!(failing.next().is_none(), "fused");
    let names = Names::new(
        vec![
            Ok(smol_str::SmolStr::new("a")),
            Err(yggdryl::Error::absent("table", "b")),
            Ok(smol_str::SmolStr::new("c")),
        ]
        .into_iter(),
    );
    assert_eq!(
        names.map(|name| name.is_ok()).collect::<Vec<_>>(),
        [true, false]
    );
    assert!(format!("{:?}", Names::empty()).contains("Names"));
}

#[test]
fn a_memory_namespace_through_the_enum_answers_its_registered_children() {
    let url = Url::from_str("file:///lake/eu/trades.csv").expect("a URL");
    let eu = Namespace::Memory(
        MemoryNamespace::new("lake.eu")
            .expect("a path")
            .with_description("europe")
            .with_object(Table::from(
                MediaTable::new("lake.eu.trades", url).expect("a table"),
            ))
            .expect("registered"),
    );
    assert_eq!(eu.description(), Some("europe"));
    assert_eq!(eu.as_namespace().name(), "eu");
    assert_eq!(
        eu.tables()
            .iter()
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("names"),
        ["trades"]
    );
    assert!(eu.namespaces().is_empty().expect("none"));
    assert!(eu.tables().contains("trades").expect("an answer"));
    assert!(!eu.tables().contains("fills").expect("an answer"));
    let nested = Namespace::Memory(
        MemoryNamespace::new("lake.eu")
            .expect("a path")
            .with_object(Namespace::Memory(
                MemoryNamespace::new("lake.eu.fx").expect("a path"),
            ))
            .expect("registered"),
    );
    assert_eq!(nested.namespaces().len().expect("a count"), 1);
    assert_eq!(
        nested.namespaces().get("fx").expect("nested").to_string(),
        "lake.eu.fx"
    );
    let held: Namespace = MemoryNamespace::new("lake.asia").expect("a path").into();
    assert_eq!(held.to_string(), "lake.asia");
}

#[test]
fn open_or_create_opens_what_is_there_and_creates_through_the_namespace_otherwise() {
    let catalog = Catalog::Memory(
        MemoryCatalog::new("lake")
            .with_object(Namespace::Memory(
                MemoryNamespace::new("lake.eu").expect("a path"),
            ))
            .expect("registered"),
    );
    let namespaces = catalog.namespaces();
    assert_eq!(
        namespaces
            .open_or_create("eu", &Properties::new())
            .expect("opened")
            .to_string(),
        "lake.eu"
    );
    let error = namespaces
        .open_or_create("asia", &Properties::new())
        .expect_err("a memory catalog creates nothing");
    assert_eq!(
        error.to_string(),
        "filesystem \"MemoryCatalog\" does not support creating a namespace"
    );
    let error = namespaces
        .create("eu.west", &Properties::new())
        .expect_err("a dotted name descends to the parent it is created under");
    assert_eq!(
        error.to_string(),
        "filesystem \"MemoryNamespace\" does not support creating a namespace"
    );
    let field = DataType::from(
        StructType::from_fields([DataType::Int64.required_field("id")]).expect("a root"),
    )
    .required_field("row");
    let error = catalog
        .tables()
        .open_or_create("eu.trades", &field, &Properties::new())
        .expect_err("nothing to open and nothing to create with");
    assert_eq!(
        error.to_string(),
        "filesystem \"MemoryNamespace\" does not support creating a table"
    );
    let batch = RecordBatch::try_new(
        field.into_arrow_schema().expect("a schema"),
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )
    .expect("a batch");
    let error = catalog
        .tables()
        .append_arrow_reader(
            "ticks",
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        )
        .expect_err("creating on first write needs a store");
    assert_eq!(
        error.to_string(),
        "filesystem \"MemoryCatalog\" does not support creating a table"
    );
}

#[test]
fn the_write_helpers_write_through_an_existing_table() {
    let root = root("writes");
    let url = Url::from_path(root.join("eu/ticks.arrows")).expect("a URL");
    let catalog = Catalog::Memory(
        MemoryCatalog::new("lake")
            .with_object(Table::from(
                MediaTable::new("lake.ticks", url).expect("a table"),
            ))
            .expect("registered"),
    );
    let field = DataType::from(
        StructType::from_fields([DataType::Int64.required_field("id")]).expect("a root"),
    )
    .required_field("row");
    let batch = || {
        RecordBatch::try_new(
            field.clone().into_arrow_schema().expect("a schema"),
            vec![Arc::new(Int64Array::from(vec![1_i64, 2]))],
        )
        .expect("a batch")
    };
    let tables = catalog.tables();
    let written = tables
        .overwrite_arrow_reader(
            "ticks",
            yggdryl::arrow::batch_reader(batch().schema(), [batch()]),
        )
        .expect("overwritten");
    assert_eq!(written.row_size().expect("rows"), 2);
    // An append resizes the file, so its previous reader must be finished.
    drop(written);
    let appended = tables
        .append_arrow_reader(
            "ticks",
            yggdryl::arrow::batch_reader(batch().schema(), [batch()]),
        )
        .expect("appended");
    assert_eq!(appended.row_size().expect("rows"), 4);
    let options = appended.record_options().expect("options");
    drop(appended);
    let again = tables
        .overwrite_arrow_reader_with_options(
            "ticks",
            yggdryl::arrow::batch_reader(batch().schema(), [batch()]),
            &options,
        )
        .expect("overwritten");
    assert_eq!(again.row_size().expect("rows"), 2);
    drop(again);
    let again = tables
        .append_arrow_reader_with_options(
            "ticks",
            yggdryl::arrow::batch_reader(batch().schema(), [batch()]),
            &options,
        )
        .expect("appended");
    assert_eq!(again.row_size().expect("rows"), 4);
    let opened = tables
        .open_or_create_from_arrow_reader(
            "ticks",
            &yggdryl::arrow::batch_reader(batch().schema(), [batch()]),
            &Properties::new(),
        )
        .expect("opened as it is");
    assert_eq!(opened.to_string(), "lake.ticks");
    assert!(tables.contains("ticks").expect("an answer"));
    assert_eq!(tables.len().expect("a count"), 1);
}
