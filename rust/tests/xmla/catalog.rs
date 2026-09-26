//! `rust/src/xmla/catalog.rs`: a catalog read two levels deep - the root's
//! tabular leaves and table-format folders as tables, its other folders as
//! schemas, and every tabular leaf and folder under a schema as a table - and
//! each table's name, path, description, encoding and field.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::local::LocalFolder;
use yggdryl::media::RecordOptions;
use yggdryl::soap::{Envelope, FaultCode};
use yggdryl::xmla::{Catalog, Execute, PropertyList, Request, Service, ServiceOptions, Table};
use yggdryl::{Codec, DataType, Field, IOBase, IOMedia, MimeType, StructType, Url};

/// A fresh catalog folder under the temporary directory, named after `label`.
fn catalog_root(label: &str) -> PathBuf {
    let mut root = LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path");
    root.push(format!(
        "yggdryl-xmla-catalog-{label}-{}-{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("main")
            .replace("::", "-")
    ));
    let _ = std::fs::remove_dir_all(&root);
    LocalFolder::new(&root)
        .expect("a folder")
        .create()
        .expect("the folder is created");
    root
}

/// The trades table: three rows of `symbol`, `price`, `size`.
fn trades_field() -> Field {
    StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::Int64.nullable_field("size"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row")
}

fn trades_batch() -> RecordBatch {
    let schema = trades_field().into_arrow_schema().expect("an Arrow schema");
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["AAPL", "MSFT", "GOOG"])),
            Arc::new(Float64Array::from(vec![187.5, 410.25, 141.0])),
            Arc::new(Int64Array::from(vec![Some(100), None, Some(25)])),
        ],
    )
    .expect("a batch")
}

/// The leaf at the file-system path `path` under `root`.
fn leaf(root: &Path, path: &str) -> Holder {
    Holder::file(root.join(path)).expect("the leaf holds")
}

/// Write the trades rows to `path` under `root`, in the encoding its name
/// declares.
fn write_rows(root: &Path, path: &str) {
    let mut leaf = leaf(root, path);
    let options = RecordOptions::for_media_type(leaf.media_type()).expect("a record encoding");
    let batch = trades_batch();
    leaf.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        &options,
    )
    .expect("the table is written");
}

/// Write `bytes` to `path` under `root`.
fn write_bytes(root: &Path, path: &str, bytes: &[u8]) {
    leaf(root, path)
        .write_all_bytes(bytes)
        .expect("the bytes are written");
}

/// Create the empty folder `path` under `root`.
fn create_folder(root: &Path, path: &str) {
    LocalFolder::new(root.join(path))
        .expect("a folder")
        .create()
        .expect("the folder is created");
}

/// The catalog every test reads, laid out as:
///
/// ```text
/// .hidden.arrows           private: no table
/// LICENSE                  no suffix: no table
/// README.md                markdown: no table
/// empty/                   a schema holding nothing
/// eu/                      a schema
///   fills.arrows           a table
///   partitioned/           a table: a folder read as the rows beneath it
///     part-0.arrows
///   staging/               a table: every folder under a schema is one
/// notes.bin                octet-stream: no table
/// trades.arrows            a table
/// ```
fn seed(root: &Path) {
    write_rows(root, "trades.arrows");
    write_rows(root, "eu/fills.arrows");
    write_rows(root, "eu/partitioned/part-0.arrows");
    create_folder(root, "eu/staging");
    create_folder(root, "empty");
    write_rows(root, ".hidden.arrows");
    write_bytes(root, "notes.bin", &[0x00, 0x01, 0xFF]);
    write_bytes(root, "LICENSE", b"MIT\n");
    write_bytes(root, "README.md", b"# not a table\n");
}

/// The seeded catalog `market`.
fn market(label: &str) -> Catalog {
    let root = catalog_root(label);
    seed(&root);
    Catalog::new("market", Holder::folder(&root).expect("the catalog holds"))
}

fn paths(tables: &[Table]) -> Vec<String> {
    tables.iter().map(Table::path).collect()
}

/// Answer `request` and read the fault it earns.
fn fault(service: &Service, request: impl Into<Request>) -> yggdryl::soap::Fault {
    let bytes = service
        .answer(&request.into(), Vec::new())
        .expect("the answer is written");
    Envelope::from_bytes(&bytes)
        .expect("an envelope")
        .fault()
        .cloned()
        .unwrap_or_else(|| panic!("expected a fault:\n{}", String::from_utf8_lossy(&bytes)))
}

// Refusals.

#[test]
fn an_unknown_table_is_absent_naming_the_catalog_and_the_table() {
    let catalog = market("unknown-table");
    let error = catalog
        .table(None, "nowhere")
        .expect_err("no table is called nowhere");
    assert!(error.is_absent(), "{error:?}");
    assert_eq!(
        error.to_string(),
        "expected a table at \"market.nowhere\", got nothing"
    );
}

#[test]
fn an_unknown_schema_is_absent_naming_the_whole_path() {
    let catalog = market("unknown-schema");
    let error = catalog
        .table(Some("asia"), "fills")
        .expect_err("no schema is called asia");
    assert!(error.is_absent(), "{error:?}");
    assert_eq!(
        error.to_string(),
        "expected a table at \"market.asia.fills\", got nothing"
    );
}

#[test]
fn a_table_is_named_without_its_suffix_and_the_suffixed_name_reaches_nothing() {
    let catalog = market("suffix");
    assert_eq!(
        catalog.table(None, "trades").expect("the table").name(),
        "trades"
    );
    let error = catalog
        .table(None, "trades.arrows")
        .expect_err("the file name is not the table name");
    assert!(error.is_absent(), "{error:?}");
    assert!(
        error.to_string().contains("market.trades.arrows"),
        "{error}"
    );
    assert!(
        catalog.table(Some("eu"), "fills.arrows").is_err(),
        "the suffix is off under a schema too"
    );
}

#[test]
fn a_name_and_a_schema_are_matched_exactly() {
    let catalog = market("exact");
    assert!(catalog.table(None, "TRADES").expect_err("case").is_absent());
    assert!(
        catalog
            .table(None, " trades")
            .expect_err("space")
            .is_absent()
    );
    assert!(
        catalog
            .table(Some("EU"), "fills")
            .expect_err("case")
            .is_absent()
    );
    assert!(
        catalog
            .table(Some(""), "trades")
            .expect_err("empty")
            .is_absent()
    );
}

#[test]
fn a_table_is_found_only_under_its_own_schema() {
    let catalog = market("own-schema");
    let root_miss = catalog.table(None, "fills").expect_err("fills is under eu");
    assert_eq!(
        root_miss.to_string(),
        "expected a table at \"market.fills\", got nothing"
    );
    let schema_miss = catalog
        .table(Some("eu"), "trades")
        .expect_err("trades is at the root");
    assert_eq!(
        schema_miss.to_string(),
        "expected a table at \"market.eu.trades\", got nothing"
    );
    assert!(
        catalog.table(Some("empty"), "fills").is_err(),
        "an empty schema holds no table"
    );
}

#[test]
fn two_leaves_differing_only_in_extension_conflict_naming_the_path() {
    let root = catalog_root("conflict-root");
    write_rows(&root, "trades.arrows");
    write_rows(&root, "trades.arrow");
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        ["market.trades", "market.trades"],
        "the listing states both leaves"
    );
    let error = catalog
        .table(None, "trades")
        .expect_err("two leaves are called trades");
    assert!(error.is_conflict(), "{error:?}");
    assert_eq!(
        error.to_string(),
        "expected to create a table at \"market.trades\", got an existing several leaves of that name"
    );
}

#[test]
fn a_leaf_and_a_folder_of_one_name_under_a_schema_conflict() {
    let root = catalog_root("conflict-schema");
    write_rows(&root, "eu/fills.arrows");
    write_rows(&root, "eu/fills/part-0.arrows");
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    let error = catalog
        .table(Some("eu"), "fills")
        .expect_err("a leaf and a folder are called fills");
    assert!(error.is_conflict(), "{error:?}");
    assert!(error.to_string().contains("\"market.eu.fills\""), "{error}");
}

#[test]
fn an_unknown_catalog_is_a_client_fault_naming_it() {
    let root = catalog_root("no-catalog");
    seed(&root);
    let service = Service::new(ServiceOptions::new()).with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ));

    let dotted = fault(&service, Execute::statement("select * from nope.eu.fills"));
    assert_eq!(dotted.code(), &FaultCode::Client);
    assert!(
        dotted
            .string()
            .contains("expected a catalog at \"nope\", got nothing"),
        "{}",
        dotted.string()
    );

    let property = fault(
        &service,
        Execute::statement("select * from trades")
            .with_properties(PropertyList::new().with("Catalog", "nope")),
    );
    assert_eq!(property.code(), &FaultCode::Client);
    assert!(
        property
            .string()
            .contains("expected a catalog at \"nope\", got nothing"),
        "{}",
        property.string()
    );
}

// The catalog itself.

#[test]
fn a_catalog_states_its_name_container_and_description() {
    let root = catalog_root("accessors");
    seed(&root);
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    assert_eq!(catalog.name(), "market");
    assert_eq!(catalog.description(), None, "no description until given");
    assert!(catalog.holder().is_container());
    let url = catalog
        .url()
        .expect("a local folder has a location")
        .clone();
    assert_eq!(url.into_path().expect("a local path"), root);
    let modified = catalog.modified().expect("a local folder keeps its mtime");
    assert!(modified > 0, "nanoseconds since the epoch: {modified}");

    let described = catalog.with_description("the market catalog");
    assert_eq!(described.description(), Some("the market catalog"));
    assert_eq!(
        described.name(),
        "market",
        "a description changes nothing else"
    );
    let unicode = Catalog::new("marché", Holder::folder(&root).expect("the catalog holds"))
        .with_description("");
    assert_eq!(unicode.name(), "marché");
    assert_eq!(
        unicode.description(),
        Some(""),
        "an empty description is one"
    );
}

#[test]
fn tables_lists_the_root_leaves_then_each_schema_in_listing_order() {
    let catalog = market("tables");
    let tables = catalog.tables().expect("a listing");
    assert_eq!(
        paths(&tables),
        [
            "market.trades",
            "market.eu.fills",
            "market.eu.partitioned",
            "market.eu.staging",
        ],
        "the root's tables come first although `eu` sorts before `trades`"
    );
    for table in &tables {
        assert_eq!(table.catalog(), "market");
        assert_eq!(table.table_type(), "TABLE");
    }
}

#[test]
fn schemas_are_the_root_folders_by_name_an_empty_one_included() {
    let catalog = market("schemas");
    assert_eq!(catalog.schemas().expect("a listing"), ["empty", "eu"]);
}

#[test]
fn a_root_leaf_table_answers_its_path_media_type_encoding_and_field() {
    let catalog = market("root-leaf");
    let trades = catalog.table(None, "trades").expect("the table");
    assert_eq!(trades.catalog(), "market");
    assert_eq!(trades.schema(), None);
    assert_eq!(trades.name(), "trades");
    assert_eq!(trades.table_type(), "TABLE");
    assert_eq!(trades.description(), "application/vnd.apache.arrow.stream");
    assert!(!trades.holder().is_container());
    assert_eq!(
        trades.url().and_then(Url::file_name),
        Some("trades.arrows"),
        "the location is the leaf's"
    );
    assert!(trades.modified().expect("a local file keeps its mtime") > 0);
    assert_eq!(
        trades.record_options().expect("an encoding").mime_type(),
        MimeType::ARROW_STREAM
    );
    assert_eq!(trades.field().expect("the stored field"), trades_field());
    assert_eq!(trades.path(), "market.trades");
    assert_eq!(trades.to_string(), "market.trades");
}

#[test]
fn a_leaf_under_a_schema_answers_the_three_part_path() {
    let catalog = market("schema-leaf");
    let fills = catalog.table(Some("eu"), "fills").expect("the table");
    assert_eq!(fills.catalog(), "market");
    assert_eq!(fills.schema(), Some("eu"));
    assert_eq!(fills.name(), "fills");
    assert_eq!(fills.description(), "application/vnd.apache.arrow.stream");
    assert_eq!(fills.url().and_then(Url::file_name), Some("fills.arrows"));
    assert_eq!(fills.field().expect("the stored field"), trades_field());
    assert_eq!(fills.path(), "market.eu.fills");
    assert_eq!(format!("{fills}"), "market.eu.fills");
}

#[test]
fn a_folder_under_a_schema_is_one_table_read_as_the_rows_beneath_it() {
    let catalog = market("schema-folder");
    let partitioned = catalog
        .table(Some("eu"), "partitioned")
        .expect("the folder is a table");
    assert!(partitioned.holder().is_container());
    assert_eq!(partitioned.name(), "partitioned", "a folder keeps its name");
    assert_eq!(partitioned.description(), "directory", "a folder's kind");
    assert!(partitioned.modified().is_some());
    assert_eq!(
        partitioned
            .record_options()
            .expect("the leaves decide")
            .mime_type(),
        MimeType::ARROW_STREAM
    );
    assert_eq!(
        partitioned.field().expect("the rows beneath it"),
        trades_field()
    );
    assert_eq!(partitioned.path(), "market.eu.partitioned");
    assert!(
        catalog.table(Some("eu"), "part-0").is_err(),
        "a leaf inside a table folder is no table of its own"
    );
}

#[test]
fn an_empty_folder_under_a_schema_is_a_table_with_no_encoding() {
    let catalog = market("schema-empty-folder");
    let staging = catalog
        .table(Some("eu"), "staging")
        .expect("every folder under a schema is a table");
    assert_eq!(staging.description(), "directory");
    assert!(
        staging.record_options().is_err(),
        "no encoding covers an empty folder"
    );
    assert!(staging.field().is_err(), "a field needs an encoding");
}

#[test]
fn a_leaf_no_record_medium_reads_is_neither_a_table_nor_a_schema() {
    let catalog = market("other");
    let schemas = catalog.schemas().expect("a listing");
    let tables = paths(&catalog.tables().expect("a listing"));
    for name in ["notes", "notes.bin", "LICENSE", "README", "README.md"] {
        assert!(
            catalog.table(None, name).expect_err(name).is_absent(),
            "{name} is no table"
        );
        assert!(!schemas.iter().any(|schema| schema == name), "{name}");
        assert!(!tables.iter().any(|path| path.ends_with(name)), "{name}");
    }
}

#[test]
fn a_private_leaf_is_no_table() {
    let catalog = market("private");
    assert!(catalog.table(None, ".hidden").is_err());
    assert!(catalog.table(None, "").is_err());
    assert!(
        !paths(&catalog.tables().expect("a listing"))
            .iter()
            .any(|path| path.contains("hidden")),
        "a dot-prefixed name is private and is not listed"
    );
}

#[test]
fn a_plain_text_leaf_is_a_table_because_text_is_a_record_encoding() {
    let root = catalog_root("text");
    write_bytes(&root, "journal.log", b"one line\nanother line\n");
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    let journal = catalog.table(None, "journal").expect("text reads as lines");
    assert_eq!(journal.description(), "text/plain");
    assert!(matches!(
        journal.record_options().expect("the line encoding"),
        RecordOptions::Text(_)
    ));
}

#[test]
fn a_parquet_leaf_is_a_table_only_where_the_build_reads_parquet() {
    let root = catalog_root("parquet");
    write_bytes(&root, "quotes.parquet", b"PAR1PAR1");
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    let found = catalog.table(None, "quotes");
    assert_eq!(
        found.is_ok(),
        cfg!(feature = "parquet"),
        "the name declares the encoding, with no read: {found:?}"
    );
}

#[test]
fn every_extension_a_media_type_claims_comes_off_the_name() {
    let root = catalog_root("names");
    write_rows(&root, "2024.report.arrow");
    write_rows(&root, "notes.json.arrows");
    write_rows(&root, "v1.2.arrows");
    write_rows(&root, "arrows.arrows");
    let mut encoded = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let batch = trades_batch();
    encoded
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
        )
        .expect("the rows are encoded");
    let mut compressed = leaf(&root, "ticks.arrows.gz");
    encoded
        .compress_into(&mut compressed, Codec::Gzip)
        .expect("the rows are compressed");
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    let mut names: Vec<String> = catalog
        .tables()
        .expect("a listing")
        .iter()
        .map(|table| table.name().to_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["2024.report", "arrows", "notes", "ticks", "v1.2"],
        "a word no media type claims stays, and a name is never emptied"
    );
    let ticks = catalog.table(None, "ticks").expect("the compressed table");
    assert_eq!(
        ticks.url().and_then(Url::file_name),
        Some("ticks.arrows.gz")
    );
    assert_eq!(
        ticks
            .record_options()
            .expect("the base encoding")
            .mime_type(),
        MimeType::ARROW_STREAM
    );
    assert_eq!(
        ticks.field().expect("the decoded rows' field"),
        trades_field()
    );
}

#[test]
fn a_schema_is_named_by_its_folder_name_as_written() {
    let root = catalog_root("unicode-schema");
    write_rows(&root, "société/fills.arrows");
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    assert_eq!(
        catalog.schemas().expect("a listing"),
        ["société"],
        "the folder's name, never its URI escape"
    );
    let fills = catalog
        .table(Some("société"), "fills")
        .expect("the schema is its folder name");
    assert_eq!(fills.path(), "market.société.fills");
}

#[test]
fn a_table_is_named_by_its_file_name_as_written() {
    let root = catalog_root("unicode-table");
    write_rows(&root, "order book.arrows");
    write_rows(&root, "ålesund.arrows");
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    let mut names: Vec<String> = catalog
        .tables()
        .expect("a listing")
        .iter()
        .map(|table| table.name().to_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["order book", "ålesund"],
        "the file's name less its suffix, never its URI escape"
    );
    let book = catalog
        .table(None, "order book")
        .expect("the name is the file name");
    assert_eq!(book.path(), "market.order book");
}

#[test]
fn a_table_written_after_the_catalog_was_built_is_served_on_the_next_request() {
    let root = catalog_root("uncached");
    seed(&root);
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    assert!(catalog.table(Some("eu"), "orders").is_err());
    write_rows(&root, "eu/orders.arrows");
    create_folder(&root, "asia");
    assert_eq!(
        catalog
            .table(Some("eu"), "orders")
            .expect("listed on ask")
            .path(),
        "market.eu.orders"
    );
    assert_eq!(
        catalog.schemas().expect("a listing"),
        ["asia", "empty", "eu"]
    );
}

#[test]
fn a_catalog_over_a_missing_folder_lists_nothing() {
    let root = catalog_root("missing").join("absent");
    let catalog = Catalog::new("ghost", Holder::folder(&root).expect("the catalog holds"));
    assert!(
        catalog
            .tables()
            .expect("an absent store lists nothing")
            .is_empty()
    );
    assert!(
        catalog
            .schemas()
            .expect("an absent store lists nothing")
            .is_empty()
    );
    assert_eq!(catalog.modified(), None, "no folder, no mtime");
    assert!(catalog.url().is_some(), "the location is still named");
    let error = catalog.table(None, "trades").expect_err("nothing is there");
    assert_eq!(
        error.to_string(),
        "expected a table at \"ghost.trades\", got nothing"
    );
}

#[cfg(feature = "iceberg")]
#[test]
fn an_iceberg_table_at_the_root_is_a_table_rather_than_a_schema() {
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, assign_field_ids};

    let root = catalog_root("iceberg");
    seed(&root);
    let mut schema = trades_field();
    assign_field_ids(&mut schema, 1).expect("field ids");
    yggdryl::iceberg::Table::create(
        LocalFolder::new(root.join("ledger")).expect("a folder"),
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )
    .expect("the table is created");
    let catalog = Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    assert!(
        !catalog
            .schemas()
            .expect("a listing")
            .iter()
            .any(|schema| schema == "ledger"),
        "a table format's folder at the root is a table, not a schema"
    );
    let ledger = catalog
        .table(None, "ledger")
        .expect("an Iceberg table at the root is a table");
    assert_eq!(ledger.description(), "table");
    assert_eq!(ledger.path(), "market.ledger");
}
