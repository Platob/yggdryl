//! `rust/src/warehouse/table.rs`: a table, and the enum over its
//! implementations, delegating every byte and record verb.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::media::IORecordOptions;
use yggdryl::{
    DataType, IOBase, IOKind, IOMedia, MediaTable, ObjectValue, Properties, StructType, Table,
    TableValue, Url,
};

fn root(label: &str) -> PathBuf {
    let root = LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path")
        .join(format!(
            "yggdryl-warehouse-table-{label}-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the folder is created");
    root
}

fn rows() -> RecordBatch {
    let field = DataType::from(
        StructType::from_fields([DataType::Int64.required_field("id")]).expect("a root"),
    )
    .required_field("row");
    RecordBatch::try_new(
        field.into_arrow_schema().expect("a schema"),
        vec![Arc::new(Int64Array::from(vec![1_i64, 2, 3]))],
    )
    .expect("a batch")
}

#[test]
fn a_table_delegates_every_verb_to_its_implementation() {
    let root = root("delegate");
    let url = Url::from_path(root.join("ticks.arrows")).expect("a URL");
    let mut table = Table::from(MediaTable::new("lake.ticks", url.clone()).expect("a table"));
    assert_eq!(ObjectValue::kind(&table), IOKind::Table);
    assert_eq!(table.name(), "ticks");
    assert_eq!(table.path(), ["lake", "ticks"]);
    assert_eq!(ObjectValue::url(&table), Some(&url));
    assert_eq!(IOBase::url(&table), Some(&url));
    assert_eq!(table.to_string(), "lake.ticks");
    assert_eq!(table.storage(), "application/vnd.apache.arrow.stream");
    assert!(table.properties().expect("stated").is_empty());
    let batch = rows();
    let options = table.record_options().expect("the handle's options");
    assert_eq!(
        options.name(),
        "ticks",
        "the record is named after the table"
    );
    table
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .expect("written");
    assert_eq!(table.field().expect("stored").field_len(), 1);
    assert_eq!(table.row_size().expect("rows"), 3);
    assert_eq!(table.column_size().expect("columns"), 1);
    assert!(IOBase::size(&table) > 0, "the bytes are the handle's");
    assert_eq!(
        IOBase::read_all_bytes(&table).expect("bytes").len() as u64,
        IOBase::size(&table)
    );
    assert!(IOBase::mtime(&table).is_some());
    assert_eq!(table.modified(), IOBase::mtime(&table));
    assert!(IOBase::is_tabular(&table));
    assert!(!IOBase::is_container(&table));
    assert_eq!(IOBase::ls(&table, false, false).count(), 0);
    assert!(IOBase::parent(&table).is_some());
    assert_eq!(table.read_arrow(None).expect("series").count(), 1);
    let held = Holder::from(table.clone());
    assert!(matches!(held, Holder::Table(_)));
    assert_eq!(held.row_size().expect("rows"), 3);
    assert!(held.exists());
    assert!(
        matches!(held.into_media(), Holder::Table(_)),
        "a table composes no second record wrapper"
    );
    IOBase::remove(&mut table, false).expect("removed");
    assert!(!Holder::from(table).exists());
}

#[test]
fn the_enum_narrows_and_compares_as_its_description() {
    let url = Url::from_str("file:///lake/ticks.arrows").expect("a URL");
    let table = Table::from(
        MediaTable::new("lake.ticks", url.clone())
            .expect("a table")
            .with_description("ticks"),
    );
    let Table::Media(media) = &table else {
        unreachable!()
    };
    assert_eq!(media.description(), Some("ticks"));
    assert_eq!(table.as_table().description(), Some("ticks"));
    assert_eq!(table.as_io().url(), Some(&url));
    let same = Table::from(
        MediaTable::new("lake.ticks", url.clone())
            .expect("a table")
            .with_description("ticks"),
    );
    assert_eq!(table, same);
    let stated = Table::from(
        MediaTable::new("lake.ticks", url)
            .expect("a table")
            .with_properties(Properties::new().with_property("codec", "zstd")),
    );
    assert_ne!(table, stated);
    assert_eq!(
        stated
            .update_properties(&Properties::new(), &[])
            .expect_err("a media table keeps nothing")
            .to_string(),
        "filesystem \"MediaTable\" does not support updating the properties it keeps"
    );
}
