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

/// Write the trades rows into `holder`, in the encoding its name declares.
fn write_rows_into(mut holder: Holder) {
    let options = RecordOptions::for_media_type(holder.media_type()).expect("a record encoding");
    let batch = trades_batch();
    holder
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .expect("the table is written");
}

/// Write the trades rows to `path` under `root`, in the encoding its name
/// declares.
fn write_rows(root: &Path, path: &str) {
    write_rows_into(leaf(root, path));
}

/// The trades rows encoded as an Arrow IPC stream, in memory.
fn arrow_stream() -> Buffer {
    let mut encoded = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let batch = trades_batch();
    encoded
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
        )
        .expect("the rows are encoded");
    encoded
}

/// Write `bytes` into `holder`.
fn write_bytes_into(mut holder: Holder, bytes: &[u8]) {
    holder
        .write_all_bytes(bytes)
        .expect("the bytes are written");
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

/// The catalog `name` over the folder `root`.
fn over(name: &str, root: &Path) -> Catalog {
    Catalog::new(name, Holder::folder(root).expect("the catalog holds"))
}

/// The seeded catalog `market`.
fn market(label: &str) -> Catalog {
    let root = catalog_root(label);
    seed(&root);
    over("market", &root)
}

fn paths(tables: &[Table]) -> Vec<String> {
    tables.iter().map(Table::path).collect()
}

/// Every table's name, sorted.
fn names(catalog: &Catalog) -> Vec<String> {
    let mut names: Vec<String> = catalog
        .tables()
        .expect("a listing")
        .iter()
        .map(|table| table.name().to_owned())
        .collect();
    names.sort();
    names
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
    assert!(!error.is_conflict(), "{error:?}");
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
fn an_absent_table_is_named_under_the_catalog_name_as_given() {
    let root = catalog_root("unicode-catalog");
    seed(&root);
    let catalog = over("marché été", &root);
    let error = catalog
        .table(Some("eu"), "ordres")
        .expect_err("no table is called ordres");
    assert_eq!(
        error.to_string(),
        "expected a table at \"marché été.eu.ordres\", got nothing"
    );
    assert_eq!(
        catalog
            .table(None, "trades")
            .expect("the root table")
            .path(),
        "marché été.trades",
        "the catalog's name leads the path, unescaped"
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
    assert_eq!(
        error.to_string(),
        "expected a table at \"market.trades.arrows\", got nothing"
    );
    let error = catalog
        .table(Some("eu"), "fills.arrows")
        .expect_err("the suffix is off under a schema too");
    assert!(error.is_absent(), "{error:?}");
    assert_eq!(
        error.to_string(),
        "expected a table at \"market.eu.fills.arrows\", got nothing"
    );
}

#[test]
fn a_name_and_a_schema_are_matched_exactly() {
    let catalog = market("exact");
    for (schema, name) in [
        (None, "TRADES"),
        (None, " trades"),
        (None, "trades "),
        (None, "\ttrades"),
        (None, "eu.fills"),
        (None, "market.trades"),
        (Some("EU"), "fills"),
        (Some(" eu"), "fills"),
        (Some("eu "), "fills"),
        (Some("eu"), "FILLS"),
        (Some("eu"), ""),
        (Some(""), "trades"),
        (Some("market"), "trades"),
    ] {
        let error = catalog
            .table(schema, name)
            .expect_err("only the exact spelling reaches a table");
        assert!(error.is_absent(), "{schema:?} {name:?}: {error:?}");
    }
    assert!(
        catalog.table(None, "eu").expect_err("a schema").is_absent(),
        "a schema is no table of the root"
    );
}

#[test]
fn schemas_differing_only_in_case_are_two_schemas() {
    let root = catalog_root("schema-case");
    write_rows(&root, "EU/fills.arrows");
    write_rows(&root, "eu/orders.arrows");
    let catalog = over("market", &root);
    assert_eq!(catalog.schemas().expect("a listing"), ["EU", "eu"]);
    assert_eq!(
        catalog
            .table(Some("EU"), "fills")
            .expect("the upper-case schema")
            .path(),
        "market.EU.fills"
    );
    assert!(
        catalog
            .table(Some("eu"), "fills")
            .expect_err("fills is under EU alone")
            .is_absent()
    );
    assert!(
        catalog
            .table(Some("EU"), "orders")
            .expect_err("orders is under eu alone")
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
    let empty_miss = catalog
        .table(Some("empty"), "fills")
        .expect_err("an empty schema holds no table");
    assert_eq!(
        empty_miss.to_string(),
        "expected a table at \"market.empty.fills\", got nothing"
    );
}

#[test]
fn two_leaves_differing_only_in_extension_conflict_naming_the_path() {
    let root = catalog_root("conflict-root");
    write_rows(&root, "trades.arrows");
    write_rows(&root, "trades.arrow");
    let catalog = over("market", &root);
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        ["market.trades", "market.trades"],
        "the listing states both leaves"
    );
    let error = catalog
        .table(None, "trades")
        .expect_err("two leaves are called trades");
    assert!(error.is_conflict(), "{error:?}");
    assert!(!error.is_absent(), "{error:?}");
    assert_eq!(
        error.to_string(),
        "expected to create a table at \"market.trades\", got an existing several leaves of that name"
    );
}

#[test]
fn two_leaves_under_a_schema_conflict_naming_the_three_part_path() {
    let root = catalog_root("conflict-schema-leaves");
    write_rows(&root, "eu/fills.arrows");
    write_rows(&root, "eu/fills.feather");
    write_rows(&root, "eu/fills.ipc");
    write_rows(&root, "eu/orders.arrows");
    let catalog = over("market", &root);
    let error = catalog
        .table(Some("eu"), "fills")
        .expect_err("three leaves are called fills");
    assert!(error.is_conflict(), "{error:?}");
    assert_eq!(
        error.to_string(),
        "expected to create a table at \"market.eu.fills\", got an existing several leaves of that name"
    );
    assert_eq!(
        catalog
            .table(Some("eu"), "orders")
            .expect("a name one leaf holds is still served")
            .path(),
        "market.eu.orders"
    );
}

#[test]
fn a_compressed_leaf_and_its_plain_twin_conflict() {
    let root = catalog_root("conflict-coded");
    write_rows(&root, "ticks.arrows");
    let mut compressed = leaf(&root, "ticks.arrows.gz");
    leaf(&root, "ticks.arrows")
        .compress_into(&mut compressed, Codec::Gzip)
        .expect("the rows are compressed");
    let catalog = over("market", &root);
    let error = catalog
        .table(None, "ticks")
        .expect_err("a content coding is an extension a media type claims");
    assert!(error.is_conflict(), "{error:?}");
    assert!(error.to_string().contains("\"market.ticks\""), "{error}");
}

#[test]
fn a_leaf_and_a_folder_of_one_name_under_a_schema_conflict() {
    let root = catalog_root("conflict-schema");
    write_rows(&root, "eu/fills.arrows");
    write_rows(&root, "eu/fills/part-0.arrows");
    let catalog = over("market", &root);
    let error = catalog
        .table(Some("eu"), "fills")
        .expect_err("a leaf and a folder are called fills");
    assert!(error.is_conflict(), "{error:?}");
    assert_eq!(
        error.to_string(),
        "expected to create a table at \"market.eu.fills\", got an existing several leaves of that name"
    );
}

#[test]
fn a_listing_failure_is_the_stores_own_refusal() {
    let root = catalog_root("listing-failure");
    write_rows(&root, "trades.arrows");
    // A folder handle over a file: the directory read itself fails.
    let catalog = over("market", &root.join("trades.arrows"));
    for error in [
        catalog
            .tables()
            .map(|_| ())
            .expect_err("a file lists nothing"),
        catalog
            .schemas()
            .map(|_| ())
            .expect_err("a file lists nothing"),
        catalog
            .table(None, "trades")
            .map(|_| ())
            .expect_err("a lookup lists first"),
    ] {
        assert!(
            !error.is_absent(),
            "a failed listing is not absence: {error:?}"
        );
        assert!(!error.is_conflict(), "{error:?}");
        match &error {
            yggdryl::Error::Io(io) => {
                assert_eq!(io.kind(), std::io::ErrorKind::NotADirectory, "{error:?}");
            }
            other => panic!("expected the store's I/O failure, got {other:?}"),
        }
    }
}

#[test]
fn an_unknown_catalog_is_a_client_fault_naming_it() {
    let root = catalog_root("no-catalog");
    seed(&root);
    let service = Service::new(ServiceOptions::new()).with_catalog(over("market", &root));

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
    let catalog = over("market", &root);
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
    assert!(
        format!("{catalog:?}").contains("market"),
        "the debug form names the catalog"
    );

    let described = catalog.with_description("the market catalog");
    assert_eq!(described.description(), Some("the market catalog"));
    assert_eq!(
        described.name(),
        "market",
        "a description changes nothing else"
    );
    assert_eq!(
        described.with_description("the second word").description(),
        Some("the second word"),
        "a later description replaces an earlier one"
    );
    let unicode = over("marché", &root).with_description("");
    assert_eq!(unicode.name(), "marché");
    assert_eq!(
        unicode.description(),
        Some(""),
        "an empty description is one"
    );
}

#[test]
fn a_holder_that_is_no_container_is_a_catalog_of_nothing() {
    let catalog = Catalog::new("memory", Holder::buffer(arrow_stream()));
    assert!(!catalog.holder().is_container());
    assert!(catalog.tables().expect("nothing to list").is_empty());
    assert!(catalog.schemas().expect("nothing to list").is_empty());
    assert_eq!(
        catalog
            .table(None, "trades")
            .expect_err("nothing is there")
            .to_string(),
        "expected a table at \"memory.trades\", got nothing"
    );
}

#[test]
fn a_catalog_over_a_located_path_reads_as_over_the_folder() {
    let root = catalog_root("located");
    seed(&root);
    let catalog = Catalog::new("market", Holder::local(&root).expect("the catalog holds"));
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        [
            "market.trades",
            "market.eu.fills",
            "market.eu.partitioned",
            "market.eu.staging",
        ]
    );
    assert_eq!(catalog.schemas().expect("a listing"), ["empty", "eu"]);
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
fn several_root_tables_and_schemas_list_in_the_stores_order() {
    let root = catalog_root("order");
    write_rows(&root, "b.arrows");
    write_rows(&root, "a.arrows");
    write_rows(&root, "z/2.arrows");
    write_rows(&root, "z/1.arrows");
    write_rows(&root, "m/k.arrows");
    create_folder(&root, "m/j");
    let catalog = over("market", &root);
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        [
            "market.a",
            "market.b",
            "market.m.j",
            "market.m.k",
            "market.z.1",
            "market.z.2",
        ],
        "the root's tables, then each schema's in turn, as a local store lists them"
    );
    assert_eq!(catalog.schemas().expect("a listing"), ["m", "z"]);
}

#[test]
fn schemas_are_the_root_folders_by_name_an_empty_one_included() {
    let catalog = market("schemas");
    assert_eq!(catalog.schemas().expect("a listing"), ["empty", "eu"]);
}

#[test]
fn a_schema_holding_only_leaves_no_medium_reads_is_a_schema_of_no_tables() {
    let root = catalog_root("schema-of-nothing");
    write_bytes(&root, "docs/README.md", b"# docs\n");
    write_bytes(&root, "docs/config.json", b"{}");
    let catalog = over("market", &root);
    assert_eq!(catalog.schemas().expect("a listing"), ["docs"]);
    assert!(catalog.tables().expect("a listing").is_empty());
}

// Tables.

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
    assert!(
        format!("{trades:?}").contains("trades"),
        "the debug form names the table"
    );
}

#[test]
fn a_table_of_no_rows_still_answers_its_field() {
    let root = catalog_root("no-rows");
    let schema = trades_field().into_arrow_schema().expect("an Arrow schema");
    let mut holder = leaf(&root, "quiet.arrows");
    holder
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(schema, Vec::<RecordBatch>::new()),
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
        )
        .expect("the empty table is written");
    let catalog = over("market", &root);
    let quiet = catalog.table(None, "quiet").expect("an empty table is one");
    assert_eq!(quiet.field().expect("the stored field"), trades_field());
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
        catalog
            .table(Some("eu"), "part-0")
            .expect_err("a leaf inside a table folder is no table of its own")
            .is_absent()
    );
    assert!(
        catalog
            .table(Some("partitioned"), "part-0")
            .expect_err("a table folder is no schema")
            .is_absent()
    );
}

#[test]
fn a_nested_folder_under_a_schema_is_one_table_and_never_a_schema() {
    let root = catalog_root("nested-folder");
    write_rows(&root, "eu/lake/year=2024/part-0.arrows");
    write_rows(&root, "eu/lake/year=2025/part-0.arrows");
    let catalog = over("market", &root);
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        ["market.eu.lake"],
        "three levels down is one table, never a second schema level"
    );
    assert_eq!(catalog.schemas().expect("a listing"), ["eu"]);
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
fn a_leaf_no_record_medium_reads_under_a_schema_is_no_table() {
    let root = catalog_root("schema-other");
    write_rows(&root, "eu/fills.arrows");
    write_bytes(&root, "eu/README.md", b"# eu\n");
    write_bytes(&root, "eu/LICENSE", b"MIT\n");
    write_bytes(&root, "eu/quotes.csv", b"symbol,price\nAAPL,1\n");
    write_bytes(&root, "eu/config.json", b"{}");
    let catalog = over("market", &root);
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        ["market.eu.fills"],
        "a schema's leaves are tables only where a record medium reads them"
    );
    for name in ["README", "LICENSE", "quotes", "config"] {
        assert!(
            catalog.table(Some("eu"), name).expect_err(name).is_absent(),
            "{name} is no table"
        );
    }
}

#[test]
fn a_private_leaf_is_no_table() {
    let catalog = market("private");
    assert!(
        catalog
            .table(None, ".hidden")
            .expect_err("private")
            .is_absent()
    );
    assert!(catalog.table(None, "").expect_err("empty").is_absent());
    assert!(
        catalog
            .table(None, "hidden")
            .expect_err("private")
            .is_absent()
    );
    assert!(
        !paths(&catalog.tables().expect("a listing"))
            .iter()
            .any(|path| path.contains("hidden")),
        "a dot-prefixed name is private and is not listed"
    );
}

#[test]
fn a_private_folder_is_neither_a_schema_nor_a_table() {
    let root = catalog_root("private-folders");
    write_rows(&root, ".git/objects.arrows");
    write_rows(&root, "eu/fills.arrows");
    write_rows(&root, "eu/.staging/part-0.arrows");
    write_rows(&root, "eu/.draft.arrows");
    let catalog = over("market", &root);
    assert_eq!(catalog.schemas().expect("a listing"), ["eu"]);
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        ["market.eu.fills"],
        "a dot-prefixed folder or leaf is private at every level"
    );
}

#[test]
fn every_record_encoding_this_build_implements_makes_a_leaf_a_table() {
    let root = catalog_root("encodings");
    write_rows(&root, "stream.arrows");
    write_rows(&root, "file.arrow");
    write_rows(&root, "feather.feather");
    write_rows(&root, "ipc.ipc");
    write_rows(&root, "avro.avro");
    write_bytes(&root, "journal.txt", b"one\n");
    write_bytes(&root, "memo.text", b"one\n");
    write_bytes(&root, "audit.log", b"one\n");
    for other in [
        "doc.json",
        "rows.jsonl",
        "quotes.csv",
        "quotes.tsv",
        "conf.yaml",
        "conf.toml",
        "page.xml",
        "cols.orc",
        "page.html",
        "notes.md",
    ] {
        write_bytes(&root, other, b"x");
    }
    let catalog = over("market", &root);
    assert_eq!(
        names(&catalog),
        [
            "audit", "avro", "feather", "file", "ipc", "journal", "memo", "stream"
        ],
        "IPC in both framings, Avro and plain text are record encodings; JSON, CSV, YAML, TOML, XML, ORC, HTML and Markdown are not"
    );
    for (name, description, encoding) in [
        (
            "stream",
            "application/vnd.apache.arrow.stream",
            MimeType::ARROW_STREAM,
        ),
        (
            "file",
            "application/vnd.apache.arrow.file",
            MimeType::ARROW_STREAM,
        ),
        ("avro", "application/avro", MimeType::AVRO),
        ("audit", "text/plain", MimeType::PLAIN_TEXT),
    ] {
        let table = catalog.table(None, name).expect(name);
        assert_eq!(table.description(), description, "{name}");
        assert_eq!(
            table.record_options().expect(name).mime_type(),
            encoding,
            "{name}"
        );
    }
    assert_eq!(
        catalog
            .table(None, "avro")
            .expect("the Avro table")
            .field()
            .expect("the Avro container's field")
            .fields()
            .len(),
        3
    );
}

#[test]
fn a_leaf_is_a_table_by_its_name_with_no_read() {
    let root = catalog_root("no-read");
    // A four-byte length prefix and four bytes that are no IPC message.
    write_bytes(&root, "broken.arrows", b"\x04\x00\x00\x00junk");
    write_bytes(&root, "empty.arrows", b"");
    let stream = arrow_stream().read_all_bytes().expect("the IPC bytes");
    write_bytes(&root, "disguised.bin", &stream);
    write_bytes(&root, "disguised", &stream);
    let catalog = over("market", &root);
    assert_eq!(
        names(&catalog),
        ["broken", "empty"],
        "the name declares the encoding: garbage under `.arrows` is listed, Arrow under `.bin` or no suffix is not"
    );
    let broken = catalog.table(None, "broken").expect("listed by its name");
    assert_eq!(broken.description(), "application/vnd.apache.arrow.stream");
    let error = broken
        .field()
        .expect_err("a table whose bytes do not decode fails where it is read");
    assert!(
        error.to_string().starts_with("Arrow schema error: "),
        "the decoding failure is the encoding's own: {error}"
    );
}

#[test]
fn a_plain_text_leaf_is_a_table_because_text_is_a_record_encoding() {
    let root = catalog_root("text");
    write_bytes(&root, "journal.log", b"one line\nanother line\n");
    let catalog = over("market", &root);
    let journal = catalog.table(None, "journal").expect("text reads as lines");
    assert_eq!(journal.description(), "text/plain");
    assert!(matches!(
        journal.record_options().expect("the line encoding"),
        RecordOptions::Text(_)
    ));
    let field = journal.field().expect("the line projection");
    assert!(
        field.fields().iter().any(|column| column.name() == "body"),
        "a line's payload is its `body` column: {field}"
    );
}

#[test]
fn a_parquet_leaf_is_a_table_only_where_the_build_reads_parquet() {
    let root = catalog_root("parquet");
    write_bytes(&root, "quotes.parquet", b"PAR1PAR1");
    write_bytes(&root, "bids.pq", b"PAR1PAR1");
    let catalog = over("market", &root);
    for name in ["quotes", "bids"] {
        let found = catalog.table(None, name);
        assert_eq!(
            found.is_ok(),
            cfg!(feature = "parquet"),
            "the name declares the encoding, with no read: {found:?}"
        );
        if let Err(error) = found {
            assert!(error.is_absent(), "{error:?}");
        }
    }
}

#[test]
fn every_extension_a_media_type_claims_comes_off_the_name() {
    let root = catalog_root("names");
    write_rows(&root, "2024.report.arrow");
    write_rows(&root, "notes.json.arrows");
    write_rows(&root, "v1.2.arrows");
    write_rows(&root, "arrows.arrows");
    write_rows(&root, "a.b.c.arrows");
    let mut compressed = leaf(&root, "ticks.arrows.gz");
    arrow_stream()
        .compress_into(&mut compressed, Codec::Gzip)
        .expect("the rows are compressed");
    let catalog = over("market", &root);
    assert_eq!(
        names(&catalog),
        ["2024.report", "a.b.c", "arrows", "notes", "ticks", "v1.2"],
        "a word no media type claims stays, and a name is never emptied"
    );
    let ticks = catalog.table(None, "ticks").expect("the compressed table");
    assert_eq!(
        ticks.url().and_then(Url::file_name),
        Some("ticks.arrows.gz")
    );
    assert_eq!(
        ticks.description(),
        "application/vnd.apache.arrow.stream;encodings=application/gzip",
        "a leaf is described by its whole media type, its coding included"
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
fn an_extension_comes_off_the_name_whatever_its_case() {
    let root = catalog_root("extension-case");
    write_rows(&root, "QUOTES.ARROWS");
    write_rows(&root, "Fills.Arrows");
    let catalog = over("market", &root);
    assert_eq!(
        names(&catalog),
        ["Fills", "QUOTES"],
        "an extension is matched without case, and the stem keeps its own"
    );
    assert!(
        catalog
            .table(None, "quotes")
            .expect_err("the stem is matched exactly")
            .is_absent()
    );
}

#[test]
fn a_folder_keeps_a_name_a_media_type_would_claim() {
    let root = catalog_root("folder-names");
    write_rows(&root, "archive.arrows/fills.arrows");
    write_rows(&root, "archive.arrows/book.arrows/part-0.arrows");
    let catalog = over("market", &root);
    assert_eq!(
        catalog.schemas().expect("a listing"),
        ["archive.arrows"],
        "a folder at the root is a schema whatever its name"
    );
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        [
            "market.archive.arrows.book.arrows",
            "market.archive.arrows.fills",
        ],
        "a folder's name stands and a leaf's suffix comes off"
    );
    let book = catalog
        .table(Some("archive.arrows"), "book.arrows")
        .expect("the folder keeps its name");
    assert!(book.holder().is_container());
    assert_eq!(book.field().expect("the rows beneath it"), trades_field());
}

#[test]
fn a_schema_is_named_by_its_folder_name_as_written() {
    let root = catalog_root("unicode-schema");
    write_rows(&root, "société/fills.arrows");
    let catalog = over("market", &root);
    assert_eq!(
        catalog.schemas().expect("a listing"),
        ["société"],
        "the folder's name, never its URI escape"
    );
    let fills = catalog
        .table(Some("société"), "fills")
        .expect("the schema is its folder name");
    assert_eq!(fills.schema(), Some("société"));
    assert_eq!(fills.path(), "market.société.fills");
}

#[test]
fn a_table_is_named_by_its_file_name_as_written() {
    let root = catalog_root("unicode-table");
    write_rows(&root, "order book.arrows");
    write_rows(&root, "ålesund.arrows");
    write_rows(&root, "東京.arrows");
    let catalog = over("market", &root);
    assert_eq!(
        names(&catalog),
        ["order book", "ålesund", "東京"],
        "the file's name less its suffix, never its URI escape"
    );
    let book = catalog
        .table(None, "order book")
        .expect("the name is the file name");
    assert_eq!(book.path(), "market.order book");
}

#[test]
fn a_name_is_decoded_from_its_escape_exactly_once() {
    let root = catalog_root("percent");
    write_rows(&root, "100%.arrows");
    write_rows(&root, "a%20b.arrows");
    write_rows(&root, "q%41/fills.arrows");
    write_rows(&root, "eu/x%2Fy/part-0.arrows");
    let catalog = over("market", &root);
    assert_eq!(
        catalog.schemas().expect("a listing"),
        ["eu", "q%41"],
        "a `%` a folder's name holds is a character, not an escape"
    );
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        [
            "market.100%",
            "market.a%20b",
            "market.eu.x%2Fy",
            "market.q%41.fills",
        ],
        "a `%` a file's name holds is a character, not an escape"
    );
    assert_eq!(
        catalog
            .table(None, "a%20b")
            .expect("the name as written")
            .url()
            .and_then(Url::file_name),
        Some("a%2520b.arrows"),
        "the location keeps the escape the name needs"
    );
    assert!(
        catalog
            .table(None, "a b")
            .expect_err("the name is not decoded twice")
            .is_absent()
    );
}

#[test]
fn a_table_written_after_the_catalog_was_built_is_served_on_the_next_request() {
    let root = catalog_root("uncached");
    seed(&root);
    let catalog = over("market", &root);
    assert!(
        catalog
            .table(Some("eu"), "orders")
            .expect_err("not yet written")
            .is_absent()
    );
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
fn a_table_removed_after_the_catalog_was_built_is_absent_on_the_next_request() {
    let root = catalog_root("uncached-removal");
    seed(&root);
    let catalog = over("market", &root);
    assert_eq!(
        catalog.table(None, "trades").expect("served").path(),
        "market.trades"
    );
    std::fs::remove_file(root.join("trades.arrows")).expect("the leaf is removed");
    std::fs::remove_dir_all(root.join("empty")).expect("the schema is removed");
    let error = catalog
        .table(None, "trades")
        .expect_err("nothing remembers a removed table");
    assert!(error.is_absent(), "{error:?}");
    assert_eq!(catalog.schemas().expect("a listing"), ["eu"]);
}

#[test]
fn a_catalog_over_a_missing_folder_lists_nothing() {
    let root = catalog_root("missing").join("absent");
    let catalog = over("ghost", &root);
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

// Table formats.

/// An Iceberg table's layout, written by hand: `metadata/` holding the hint
/// a catalog-less table keeps, or a metadata document, beside `data/`.
fn iceberg_layout(root: &Path, folder: &str, marker: &str) {
    write_bytes(root, &format!("{folder}/metadata/{marker}"), b"1");
    write_rows(root, &format!("{folder}/data/part-0.arrows"));
}

#[test]
fn a_root_folder_laid_out_as_an_iceberg_table_is_a_table_in_every_build() {
    let root = catalog_root("iceberg-layout");
    seed(&root);
    iceberg_layout(&root, "ledger", "version-hint.text");
    iceberg_layout(&root, "book", "00000-4b1f.metadata.json");
    let catalog = over("market", &root);
    assert_eq!(
        catalog.schemas().expect("a listing"),
        ["empty", "eu"],
        "a table format's folder at the root is a table, not a schema"
    );
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        [
            "market.book",
            "market.ledger",
            "market.trades",
            "market.eu.fills",
            "market.eu.partitioned",
            "market.eu.staging",
        ],
        "its `data/` and `metadata/` are never tables of their own"
    );
    for name in ["ledger", "book"] {
        let table = catalog.table(None, name).expect(name);
        assert_eq!(table.schema(), None);
        assert_eq!(table.name(), name, "a folder keeps its name");
        assert_eq!(table.description(), "table", "the layout says table format");
        assert!(table.holder().is_container());
        assert_eq!(table.path(), format!("market.{name}"));
    }
    assert!(
        catalog
            .table(Some("ledger"), "data")
            .expect_err("a table is no schema")
            .is_absent()
    );
}

#[test]
fn a_table_format_folder_under_a_schema_is_described_as_a_table() {
    let root = catalog_root("iceberg-under-schema");
    iceberg_layout(&root, "eu/ledger", "version-hint.text");
    write_rows(&root, "eu/lake/part-0.arrows");
    let catalog = over("market", &root);
    assert_eq!(
        paths(&catalog.tables().expect("a listing")),
        ["market.eu.lake", "market.eu.ledger"]
    );
    assert_eq!(
        catalog
            .table(Some("eu"), "ledger")
            .expect("the table")
            .description(),
        "table"
    );
    assert_eq!(
        catalog
            .table(Some("eu"), "lake")
            .expect("the table")
            .description(),
        "directory",
        "a folder with no table layout is described by its kind"
    );
}

#[test]
fn a_metadata_folder_with_no_hint_and_no_metadata_document_leaves_a_schema() {
    let root = catalog_root("iceberg-lookalikes");
    // Neither marker: a document of another name, a hint of another suffix,
    // the right name one level too deep, a private one, another case, and a
    // `metadata` that is a leaf rather than a folder.
    write_bytes(&root, "notes/metadata/v1.json", b"{}");
    write_bytes(&root, "hint/metadata/version-hint.txt", b"1");
    write_bytes(&root, "deep/metadata/old/v1.metadata.json", b"{}");
    write_bytes(&root, "private/metadata/.metadata.json", b"{}");
    write_bytes(&root, "upper/Metadata/version-hint.text", b"1");
    write_bytes(&root, "flat/metadata", b"version-hint.text");
    write_bytes(&root, "sibling/version-hint.text", b"1");
    let catalog = over("market", &root);
    assert_eq!(
        catalog.schemas().expect("a listing"),
        [
            "deep", "flat", "hint", "notes", "private", "sibling", "upper"
        ],
        "only a hint or a metadata document directly in `metadata/` makes a table format"
    );
    let tables = paths(&catalog.tables().expect("a listing"));
    assert!(
        tables.contains(&"market.sibling.version-hint".to_owned()),
        "a hint outside `metadata/` is a plain-text leaf of a schema: {tables:?}"
    );
    assert!(
        tables.contains(&"market.notes.metadata".to_owned()),
        "a `metadata` folder under a schema is a table like any other: {tables:?}"
    );
}

#[test]
fn a_table_format_folder_is_never_read_as_the_leaves_it_holds() {
    // An Iceberg table's shape: a hint and a metadata document, a manifest
    // list in Avro beside them, and Parquet data files - none of it a table
    // this hand-written layout can be read as.
    let root = catalog_root("iceberg-no-read");
    write_bytes(&root, "ledger/metadata/version-hint.text", b"1");
    write_bytes(&root, "ledger/metadata/v1.metadata.json", b"{}");
    write_rows(&root, "ledger/metadata/snap-1-manifest-list.avro");
    write_bytes(&root, "ledger/data/00000-0.parquet", b"PAR1PAR1");
    let catalog = over("market", &root);
    let ledger = catalog.table(None, "ledger").expect("the table");
    assert_eq!(ledger.description(), "table");
    let field = ledger.field();
    assert!(
        field.is_err(),
        "a table format is read through its metadata or refused - never as the \
         leaves it holds, a manifest list among them: {field:?}"
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
    let catalog = over("market", &root);
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
    assert!(
        !paths(&catalog.tables().expect("a listing"))
            .iter()
            .any(|path| path.starts_with("market.ledger.")),
        "an Iceberg table's folders are never tables of their own"
    );
}

// Other stores.

#[test]
fn a_catalog_over_a_zip_archive_names_each_table_by_its_member() {
    let root = catalog_root("zip");
    let mut archive = yggdryl::zip::mount(Holder::local(root.join("market.zip")).expect("a path"));
    for member in ["trades.arrows", "eu/fills.arrows", "eu/lake/part-0.arrows"] {
        write_rows_into(archive.child_by_path(member).expect("a member"));
    }
    write_bytes_into(
        archive.child_by_path("README.md").expect("a member"),
        b"# not a table\n",
    );
    archive.flush().expect("the archive is published");
    let catalog = Catalog::new("market", archive);
    let listed = (
        catalog.schemas().expect("a listing"),
        paths(&catalog.tables().expect("a listing")),
    );
    assert_eq!(
        listed,
        (
            vec!["eu".into()],
            vec![
                "market.trades".to_owned(),
                "market.eu.fills".to_owned(),
                "market.eu.lake".to_owned(),
            ]
        ),
        "a prefix of the archive's members is a schema named by its last segment, \
         and a member is named by its own name, never by the archive's"
    );
    assert_eq!(
        catalog
            .table(Some("eu"), "fills")
            .expect("a member table")
            .field()
            .expect("the member's field"),
        trades_field()
    );
}
