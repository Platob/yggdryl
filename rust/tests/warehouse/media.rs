//! `rust/src/warehouse/media.rs`: a table over any location a record medium
//! reads.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::local::LocalFolder;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{
    DataType, Field, FolderLayout, IOBase, IOKind, IOMedia, MediaTable, MimeType, ObjectValue,
    Properties, StructType, TableValue, Url,
};

fn root(label: &str) -> PathBuf {
    let root = LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path")
        .join(format!(
            "yggdryl-warehouse-media-{label}-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the folder is created");
    root
}

fn trades_field() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row")
}

fn batch() -> RecordBatch {
    RecordBatch::try_new(
        trades_field().into_arrow_schema().expect("a schema"),
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2, 3])),
            Arc::new(StringArray::from(vec![Some("XNAS"), None, Some("XNYS")])),
        ],
    )
    .expect("a batch")
}

fn write(url: &Url) {
    let mut holder = Holder::from_url(url, Properties::new().iter()).expect("holds");
    let options = RecordOptions::for_media_type(&url.media_type()).expect("an encoding");
    let rows = batch();
    holder
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(rows.schema(), [rows]),
            &options,
        )
        .expect("written");
}

#[test]
fn a_table_over_a_url_is_a_description_until_a_verb_needs_its_handle() {
    let root = root("by-url");
    let url = Url::from_path(root.join("trades.arrows")).expect("a URL");
    let table = MediaTable::new("lake.eu.trades", url.clone()).expect("a table");
    assert_eq!(table.name(), "trades");
    assert_eq!(table.path(), ["lake", "eu", "trades"]);
    assert_eq!(ObjectValue::kind(&table), IOKind::Table);
    assert_eq!(ObjectValue::url(&table), Some(&url));
    assert_eq!(
        table.uri().map(ToString::to_string).as_deref(),
        Some(url.to_string().as_str())
    );
    assert_eq!(table.layout(), FolderLayout::Leaf);
    assert_eq!(table.storage(), "application/vnd.apache.arrow.stream");
    assert_eq!(table.description(), None);
    assert_eq!(table.declared_field(), None);
    assert!(table.properties().expect("stated").is_empty());
    assert_eq!(table.to_string(), "lake.eu.trades");
    assert!(!IOBase::opened(&table));

    // Nothing is there yet: the table reads as the empty stream of no schema.
    assert!(!IOBase::is_container(&table));
    assert!(IOBase::is_tabular(&table));
    assert!(!IOBase::is_atomic(&table));
    write(&url);
    assert_eq!(
        table.field().expect("the stored field"),
        trades_field().with_name("trades"),
        "the row is named after the table"
    );
    assert_eq!(table.row_size().expect("three rows"), 3);
    assert_eq!(IOBase::kind(&table), IOKind::File, "the handle's own kind");
    assert_eq!(IOBase::media_type(&table).base(), &MimeType::ARROW_STREAM);
    assert!(table.modified().is_some());
    let described = table
        .clone()
        .with_description("three trades")
        .with_properties(Properties::new().with_property("batch_row_size", "2"));
    assert_eq!(described.description(), Some("three trades"));
    assert_eq!(
        described
            .properties()
            .expect("stated")
            .get("batch_row_size"),
        Some("2")
    );
    assert_ne!(described, table, "a description is part of equality");
    assert_eq!(table.clone(), table, "a clone is the same description");
}

#[test]
fn a_declared_field_is_named_after_the_table_and_answered_before_any_read() {
    let root = root("declared");
    let url = Url::from_path(root.join("trades.arrows")).expect("a URL");
    write(&url);
    let narrow = DataType::from(
        StructType::from_fields([DataType::Int32.required_field("id")]).expect("a root"),
    )
    .required_field("anything");
    let table = MediaTable::new("lake.trades", url.clone())
        .expect("a table")
        .with_field(narrow.clone());
    assert_eq!(
        table.declared_field().map(Field::name),
        Some("trades"),
        "a declared field's name is the table's"
    );
    let field = table.field().expect("the declared field");
    assert_eq!(field.name(), "trades");
    assert_eq!(field.field_len(), 1);
    let options = table.record_options().expect("options");
    assert_eq!(options.name(), "trades");
    assert_eq!(options.field().map(|field| field.field_len()), Some(1));
    assert_eq!(
        table.column_size().expect("the declared columns"),
        1,
        "the declared field answers the width with no read"
    );
    let rows: Vec<RecordBatch> = table
        .read_arrow_reader(&options)
        .expect("a read under the declared field")
        .collect::<Result<_, _>>()
        .expect("batches");
    assert_eq!(rows.iter().map(RecordBatch::num_rows).sum::<usize>(), 3);
    assert_eq!(rows[0].schema().fields().len(), 1, "the read projects");
    let column = rows[0].column(0);
    assert_eq!(
        column.data_type(),
        &arrow_schema::DataType::Int32,
        "and casts"
    );

    // With a dtype alone, the nullability and metadata of a declared field
    // are kept; without one, the table declares a required field.
    let typed = table
        .with_dtype(DataType::from(
            StructType::from_fields([DataType::Int64.required_field("id")]).expect("a root"),
        ))
        .expect("a datatype");
    let declared = typed.declared_field().expect("declared");
    assert_eq!(declared.name(), "trades");
    assert!(!declared.is_nullable());
    let fresh = MediaTable::new("lake.trades", url)
        .expect("a table")
        .with_dtype(DataType::Int64)
        .expect("a datatype");
    let declared = fresh.declared_field().expect("declared");
    assert_eq!((declared.name(), declared.is_nullable()), ("trades", false));
}

#[test]
fn a_table_bound_to_a_handle_composes_what_its_name_declares() {
    let root = root("bound");
    let url = Url::from_path(root.join("trades.arrows")).expect("a URL");
    write(&url);
    let table = MediaTable::bound(
        "lake.trades",
        Holder::local(root.join("trades.arrows")).expect("holds"),
    )
    .expect("a table");
    assert_eq!(table.layout(), FolderLayout::Leaf);
    assert_eq!(ObjectValue::url(&table), Some(&url));
    assert_eq!(
        table.field().expect("the stored field"),
        trades_field().with_name("trades")
    );
    let folder =
        MediaTable::bound("lake.part", Holder::folder(&root).expect("holds")).expect("a table");
    assert_eq!(
        folder.layout(),
        FolderLayout::Folder,
        "a container binds as a folder table"
    );
    assert_eq!(folder.storage(), "directory");
    assert!(IOBase::is_container(&folder));
    assert_eq!(folder.row_size().expect("the rows beneath it"), 3);
    let format = folder.clone().with_layout(FolderLayout::Format);
    assert_eq!(format.storage(), "table");
    assert_eq!(IOBase::kind(&format), IOKind::Table);
    if !cfg!(feature = "iceberg") {
        assert!(
            format.record_options().is_err(),
            "refused by name without the feature"
        );
    }
}

#[test]
fn a_table_over_a_buffer_cannot_be_rebuilt_after_a_clone_and_says_so() {
    let mut buffer = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let rows = batch();
    buffer
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(rows.schema(), [rows]),
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
        )
        .expect("encoded");
    let table = MediaTable::bound("memory.trades", Holder::buffer(buffer)).expect("a table");
    assert_eq!(ObjectValue::url(&table), None);
    assert_eq!(
        table.field().expect("the held bytes decode"),
        trades_field().with_name("trades")
    );
    assert_eq!(
        table.storage(),
        "application/vnd.apache.arrow.stream",
        "the handle's media type"
    );
    let twin = table.clone();
    assert_eq!(twin, table, "the description is the same");
    let error = twin.field().expect_err("nothing to rebuild from");
    assert_eq!(
        error.to_string(),
        "invalid record value at $.memory.trades: expected a located handle to rebuild \
         `memory.trades` from, got one with no URL; a clone of an object bound to an unlocated \
         handle has nothing to open"
    );
    assert_eq!(
        IOBase::size(&twin),
        0,
        "an unresolvable handle answers the empty value"
    );
    assert!(
        IOBase::ls(&twin, false, false)
            .next()
            .is_some_and(|entry| entry.is_err())
    );
}

#[test]
fn stated_properties_open_the_handle() {
    let root = root("properties");
    let url = Url::from_path(root.join("trades.bin")).expect("a URL");
    let mut holder = Holder::from_url(
        &url,
        [("media_type", "application/vnd.apache.arrow.stream")],
    )
    .expect("holds");
    let rows = batch();
    holder
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(rows.schema(), [rows]),
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
        )
        .expect("written");
    let bare = MediaTable::new("lake.trades", url.clone()).expect("a table");
    assert!(
        bare.record_options().is_err(),
        "an extensionless name declares no encoding"
    );
    let typed = MediaTable::new("lake.trades", url)
        .expect("a table")
        .with_properties(
            Properties::new().with_property("media_type", "application/vnd.apache.arrow.stream"),
        );
    assert_eq!(
        typed.field().expect("read under the stated media type"),
        trades_field().with_name("trades")
    );
    assert_eq!(typed.row_size().expect("three rows"), 3);
}

#[test]
fn every_write_verb_reaches_the_handle() {
    let root = root("writes");
    let url = Url::from_path(root.join("trades.arrows")).expect("a URL");
    let mut table = MediaTable::new("lake.trades", url).expect("a table");
    let rows = batch();
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options");
    table
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(rows.schema(), [rows.clone()]),
            &options,
        )
        .expect("overwritten");
    assert_eq!(table.row_size().expect("rows"), 3);
    table
        .append_arrow_reader(
            yggdryl::arrow::batch_reader(rows.schema(), [rows]),
            &options,
        )
        .expect("appended");
    assert_eq!(table.row_size().expect("rows"), 6);
    let read = table
        .read_serie(None)
        .expect("a serie reader under the table's own options")
        .map(|serie| serie.map(|serie| serie.len()))
        .collect::<Result<Vec<_>, _>>()
        .expect("series");
    assert_eq!(read.iter().sum::<usize>(), 6);
    assert!(Holder::from(yggdryl::Table::from(table.clone())).exists());
    table.remove(false).expect("removed through the handle");
    assert_eq!(table.row_size().expect("absent reads empty"), 0);
}

#[test]
fn an_empty_path_or_a_url_as_a_path_is_refused() {
    let url = Url::from_str("file:///lake/trades.csv").expect("a URL");
    let error = MediaTable::new("", url.clone()).expect_err("no name");
    assert_eq!(
        error.to_string(),
        "invalid record value at $.path: expected a path naming the table, got the empty path"
    );
    let error = MediaTable::new("'file:///x'", url).expect_err("a URL is no path");
    assert!(error.to_string().contains("got the URL"), "{error}");
    let error =
        MediaTable::bound(Vec::<&str>::new(), Holder::buffer(Buffer::new())).expect_err("no name");
    assert!(error.to_string().contains("empty path"), "{error}");
}
