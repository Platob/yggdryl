//! What the XML for Analysis medium and provider cost in calls to the handle
//! underneath: the pins `rust/tests/iobase_calls.rs` keeps for every encoding
//! the core ships, kept here for the one it reaches by registration.
//!
//! `Counted` is the instrument, as there: it wraps the byte handle, forwards
//! every call unchanged and tallies it, so each count below is what the
//! medium asks of storage and never a bound.

#[path = "../../rust/tests/support/counting_filesystem.rs"]
mod counting_filesystem;

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::holder::Buffer;
use yggdryl::holder::counted::{Calls, Counted};
use yggdryl::{IOBase, IOMedia, Url};

/// A handle over `bytes`, named so its media type is what `url` says.
fn source(bytes: &[u8], url: &str) -> Counted<Buffer> {
    let url = Url::from_str(url).expect("a location");
    let mut buffer = Buffer::from_bytes(bytes.to_vec());
    buffer.set_media_type(url.media_type());
    Counted::new(buffer)
}

/// A payload of `size` bytes that does not compress to nothing.
fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}

/// Run `operation` and assert it cost exactly `expected` in calls.
fn costs(what: &str, calls: &Arc<Calls>, expected: &str, operation: impl FnOnce()) {
    calls.reset();
    operation();
    assert_eq!(calls.snapshot().to_string(), expected, "{what}");
}

fn batch(rows: usize) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from((0..rows as i64).collect::<Vec<_>>())),
            Arc::new(StringArray::from(
                (0..rows)
                    .map(|index| format!("SYM{index:04}"))
                    .collect::<Vec<_>>(),
            )),
        ],
    )
    .expect("a batch")
}

/// A handle holding `rows` rows written in the encoding `url` names, the
/// encoding registered so the name reaches it through the core's doors.
fn written(url: &str, rows: usize) -> Counted<Buffer> {
    yggdryl_xmla::register();
    let media_type = Url::from_str(url).expect("a location").media_type();
    let mut sink = Buffer::new();
    sink.set_media_type(media_type.clone());
    let options = sink.record_options().expect("record options");
    sink.overwrite_arrow_batch(batch(rows), &options)
        .expect("a write");
    let mut source = Buffer::from_bytes(sink.read_all_bytes().expect("the bytes"));
    source.set_media_type(media_type);
    Counted::new(source)
}

/// A record wrapper reads a tail as the handle it wraps does - one
/// `read_tail_bytes`, which a store answers with one suffix-ranged request -
/// and never as the default's `size` then `read_range_bytes`.
#[test]
fn the_wrapper_forwards_the_tail_read() {
    let bytes = payload(64);
    let tail = (bytes[60..].to_vec(), 64);
    let handle = source(&bytes, "file:///lake/part.bin");
    let calls = Arc::clone(handle.calls());
    let wrapper = yggdryl_xmla::Xmla::new(handle);
    costs("xmla", &calls, "read_tail_bytes=1", || {
        assert_eq!(wrapper.read_tail_bytes(4).expect("the tail"), tail);
    });
}

/// The counts every record encoding shares, measured as
/// `rust/tests/iobase_calls.rs` measures the core's: the encoding, the
/// schema, the column count, the row count and a full read.
#[test]
fn xmla_costs() {
    // A rowset document is held whole, as every structured text document
    // is: XML has no frame to read a prefix of. So the schema, the row count
    // and the rows are each one read of the whole handle, and the column
    // count is the schema's read - never a second one to break a tie, and
    // never a per-row call. Reaching the medium through the registry adds
    // no call: the handle is asked nothing to find the encoding.
    let handle = written("file:///lake/part.xmla", 64);
    let calls = Arc::clone(handle.calls());
    costs(
        "xmla: the encoding",
        &calls,
        "media_type=1 is_container=1",
        || {
            handle.record_options().expect("record options");
        },
    );
    let options = handle.record_options().expect("record options");
    costs(
        "xmla: the schema",
        &calls,
        "read_all_bytes=1 media_type=2 is_container=1",
        || {
            handle.read_arrow_field(&options).expect("a field");
        },
    );
    costs(
        "xmla: the column count",
        &calls,
        "read_all_bytes=1 size=1 media_type=3 is_container=2",
        || {
            assert_eq!(handle.column_size().expect("columns"), 2);
        },
    );
    costs(
        "xmla: the row count",
        &calls,
        "read_all_bytes=1 media_type=3 is_container=2",
        || {
            assert_eq!(handle.row_size().expect("rows"), 64);
        },
    );
    costs(
        "xmla: a full read",
        &calls,
        "read_all_bytes=1 media_type=3 is_container=1",
        || {
            let read: usize = handle
                .read_arrow_reader(&options)
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum();
            assert_eq!(read, 64);
        },
    );
}

mod provider {
    //! What the XML for Analysis provider asks of the store behind a catalog,
    //! per request. A catalog takes a `Holder`, so the count is taken on a
    //! filesystem behind an `FsFolder` rather than on `Counted`; what a
    //! Discover costs is the catalog's listing and, for the columns, each
    //! table's schema. An Execute reopens its table by URL, which an
    //! `FsFolder` has none of that `Holder::from_url` holds, so its cost is
    //! measured by the `media/xmla/service/execute` benchmark and not pinned
    //! here.

    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::holder::Holder;
    use yggdryl::media::RecordOptions;
    use yggdryl::{DataType, Field, FolderCatalog, IOBase, IOMedia, MimeType, StructType};
    use yggdryl_xmla::{
        Discover, PropertyList, Request, RequestType, Response, Service, ServiceOptions,
    };

    use crate::counting_filesystem::{CountingFileSystem, counted_folder};

    fn trades_field() -> Field {
        StructType::from_fields([
            DataType::utf8().required_field("symbol"),
            DataType::Int64.required_field("size"),
        ])
        .map(DataType::from)
        .expect("a valid root")
        .required_field("row")
    }

    fn trades_batch() -> RecordBatch {
        RecordBatch::try_new(
            trades_field().into_arrow_schema().expect("an Arrow schema"),
            vec![
                Arc::new(StringArray::from(vec!["AAPL", "MSFT", "GOOG"])),
                Arc::new(Int64Array::from(vec![100, 250, 75])),
            ],
        )
        .expect("a batch")
    }

    /// A service over a `market` catalog holding one IPC table, `trades`.
    fn ipc_catalog() -> (Arc<CountingFileSystem>, Service) {
        let (filesystem, folder) = counted_folder("market");
        let root = Holder::from(folder);
        let mut leaf = root
            .child_by_path("trades.arrows")
            .expect("the table resolves");
        let batch = trades_batch();
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
        )
        .expect("the table is written");
        let service =
            Service::new(ServiceOptions::new()).with_catalog(FolderCatalog::bound("market", root));
        (filesystem, service)
    }

    /// The calls one Discover of `request_type`, under the `market` catalog,
    /// makes, by name; the answer is checked to be a rowset of `rows` rows.
    fn discover(
        filesystem: &CountingFileSystem,
        service: &Service,
        request_type: RequestType,
        rows: usize,
    ) -> String {
        let message = Request::from(
            Discover::new(request_type.clone())
                .with_properties(PropertyList::new().with("Catalog", "market")),
        )
        .into_bytes()
        .expect("the request encodes");
        let mut answer = Vec::new();
        let costs = filesystem.costs(|| {
            answer = service.handle(&message, Vec::new()).expect("answered");
        });
        let response = Response::from_bytes(&answer, None)
            .unwrap_or_else(|error| panic!("{request_type}: {error}"));
        assert_eq!(
            response.rows().map(yggdryl::Serie::len),
            Some(rows),
            "{request_type}"
        );
        costs
    }

    #[test]
    fn a_discover_over_an_ipc_catalog_costs_its_listing_and_the_columns_a_schema_read() {
        let (filesystem, service) = ipc_catalog();
        let properties = discover(&filesystem, &service, RequestType::DiscoverProperties, 53);
        let catalogs = discover(&filesystem, &service, RequestType::DbschemaCatalogs, 1);
        let cubes = discover(&filesystem, &service, RequestType::MdschemaCubes, 1);
        let tables = discover(&filesystem, &service, RequestType::DbschemaTables, 1);
        let columns = discover(&filesystem, &service, RequestType::DbschemaColumns, 2);
        // Nothing is read for what the provider states about itself, and a
        // catalog row - or the cube row that restates it - costs nothing over
        // this store. The tables are the one listing of the root plus two
        // `file_info` per table - the `fs` backend answers a listed child's
        // kind and modification time by asking the store again, each once,
        // since the folder catalog hands the leaf on as the listing gave it -
        // and the columns add one open of the leaf's stream and the schema
        // read; never a read of a row.
        assert_eq!(
            [properties, catalogs, cubes, tables, columns],
            [
                "none",
                "none",
                "none",
                "file_info=2 list=1",
                "file_info=2 list=1 open_input_stream=1",
            ],
            "properties, catalogs, cubes, tables, columns"
        );
    }
}
