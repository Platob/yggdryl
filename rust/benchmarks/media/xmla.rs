//! XML for Analysis rowsets: the document a `.xmla` handle holds, written
//! from and read back into record columns, and the Discover envelope a
//! client sends.
//!
//! The fixture is a small market row - an identifier, a nullable symbol, a
//! price and a flag - so the numbers describe a table a provider serves, not
//! a synthetic best case.

use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::xmla::{
    Content, Discover, Method, PropertyList, Request, RequestType, Response, Restrictions, Rowset,
    write_rowset,
};
use yggdryl::{DataType, Field, Scalar, Serie, StructType};

use crate::bench_profile::corpus;

/// The record field every benchmark row is laid out under.
fn field() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::Boolean.required_field("live"),
        ])
        .expect("a valid root"),
    )
    .required_field("row")
}

/// `rows` representative rows as one record column.
fn batch(field: &Field, rows: usize) -> Serie {
    Serie::from_scalars(
        field.clone(),
        (0..rows).map(|index| {
            Scalar::from_sequence([
                Scalar::from(index as i64),
                if index % 5 == 0 {
                    Scalar::Null
                } else {
                    Scalar::from(format!("SYM{index:04}"))
                },
                Scalar::from(index as f64 * 0.25),
                Scalar::from(index % 2 == 0),
            ])
        }),
    )
    .expect("rows under the field")
}

pub(crate) fn xmla_benchmarks(criterion: &mut Criterion) {
    let rows = corpus(10_000, 64);
    let field = field();
    let rowset = Rowset::new(field.clone()).expect("a rowset over a record field");
    let batch = batch(&field, rows);
    let encoded = write_rowset(
        Vec::new(),
        &[],
        Method::Discover,
        &rowset,
        [Ok(batch.clone())],
        Content::SchemaData,
    )
    .expect("the representative rowset encodes");
    // Proven once outside the timers: the document round-trips.
    let response = Response::from_bytes(&encoded, None).expect("the document decodes");
    assert_eq!(response.rows().map(Serie::len), Some(rows));

    let mut group = criterion.benchmark_group("media/xmla");
    group.throughput(Throughput::Elements(rows as u64));
    group.bench_function("write_rowset", |bencher| {
        bencher.iter(|| {
            write_rowset(
                Vec::with_capacity(encoded.len()),
                &[],
                Method::Discover,
                black_box(&rowset),
                [Ok(batch.clone())],
                Content::SchemaData,
            )
            .expect("encodes")
        });
    });
    group.bench_function("read_rowset", |bencher| {
        bencher.iter(|| Response::from_bytes(black_box(&encoded), None).expect("decodes"));
    });
    group.finish();

    let discover = Request::from(
        Discover::new(RequestType::DbschemaTables)
            .with_restrictions(
                Restrictions::new()
                    .with("TABLE_CATALOG", "market")
                    .with("TABLE_TYPE", "TABLE"),
            )
            .with_properties(
                PropertyList::new()
                    .with("Content", "SchemaData")
                    .with("Format", "Tabular"),
            ),
    );
    let envelope = discover.into_bytes().expect("the request encodes");
    assert_eq!(
        Request::from_bytes(&envelope)
            .expect("the request decodes")
            .kind(),
        Method::Discover
    );

    let mut group = criterion.benchmark_group("media/xmla/request");
    group.throughput(Throughput::Bytes(envelope.len() as u64));
    group.bench_function("write_discover", |bencher| {
        bencher.iter(|| black_box(&discover).into_bytes().expect("encodes"));
    });
    group.bench_function("read_discover", |bencher| {
        bencher.iter(|| Request::from_bytes(black_box(&envelope)).expect("decodes"));
    });
    group.finish();

    service_benchmarks(criterion, &field);
}

/// The provider end to end: `Service::handle` over a folder catalog, the
/// request's bytes in and the response's bytes out, which is what the server
/// puts on the socket minus the socket. The metadata burst a client opens
/// with - the properties, the catalogs, the tables, the columns - and an
/// Execute over the whole table at the three sizes an Excel import is timed
/// at, so the cost is measured where it is paid: the catalog's listing, the
/// table's read, the rowset's write.
fn service_benchmarks(criterion: &mut Criterion, field: &Field) {
    use yggdryl::holder::Holder;
    use yggdryl::media::RecordOptions;
    use yggdryl::xmla::{Catalog, Execute, Service, ServiceOptions};
    use yggdryl::{IOBase, IOMedia, MimeType};

    let root = std::env::temp_dir().join(format!("yggdryl-bench-xmla-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the scratch catalog");
    let sizes = [
        ("trades_10k", corpus(10_000, 64)),
        ("trades_100k", corpus(100_000, 128)),
        ("trades_1m", corpus(1_000_000, 256)),
    ];
    for (name, rows) in sizes {
        let mut leaf = Holder::folder(&root)
            .expect("the root holds")
            .child_by_path(&format!("{name}.arrows"))
            .expect("the table resolves");
        let batch = batch(field, rows).into_arrow_batch().expect("a batch");
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
        )
        .expect("the table is written");
    }
    let service = Service::new(ServiceOptions::new()).with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("holds"),
    ));
    let request = |request: Request| request.into_bytes().expect("the request encodes");
    let catalog = PropertyList::new().with("Catalog", "market");
    let burst = [
        (
            "discover_properties",
            request(Request::from(Discover::new(
                RequestType::DiscoverProperties,
            ))),
        ),
        (
            "dbschema_catalogs",
            request(Request::from(Discover::new(RequestType::DbschemaCatalogs))),
        ),
        (
            "dbschema_tables",
            request(Request::from(
                Discover::new(RequestType::DbschemaTables).with_properties(catalog.clone()),
            )),
        ),
        (
            "dbschema_columns",
            request(Request::from(
                Discover::new(RequestType::DbschemaColumns).with_properties(catalog.clone()),
            )),
        ),
    ];
    // Proven once outside the timers: the burst answers rowsets, not faults.
    for (name, message) in &burst {
        let answer = service.handle(message, Vec::new()).expect("answered");
        let response =
            Response::from_bytes(&answer, None).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(
            response.rows().is_some_and(|rows| !rows.is_empty()),
            "{name}"
        );
    }

    let mut group = criterion.benchmark_group("media/xmla/service");
    for (name, message) in &burst {
        group.bench_function(*name, |bencher| {
            bencher.iter(|| {
                service
                    .handle(black_box(message), Vec::with_capacity(1 << 16))
                    .expect("answered")
            });
        });
    }
    group.finish();

    let mut group = criterion.benchmark_group("media/xmla/service/execute");
    group.sample_size(10);
    for (name, rows) in sizes {
        let message = request(Request::from(
            Execute::statement(format!("select * from market.{name}"))
                .with_properties(catalog.clone()),
        ));
        let answer = service.handle(&message, Vec::new()).expect("answered");
        // Proven once outside the timer by the row elements the answer
        // carries: a million rows is a document past the XML codec's input
        // bound, and reading one back is `media/xmla/read_rowset`'s measure.
        let carried = answer.windows(5).filter(|bytes| *bytes == b"<row>").count();
        assert_eq!(carried, rows, "{name}");
        group.throughput(Throughput::Elements(rows as u64));
        group.bench_function(name, |bencher| {
            bencher.iter(|| {
                service
                    .handle(black_box(&message), Vec::with_capacity(answer.len()))
                    .expect("answered")
            });
        });
    }
    group.finish();
    let _ = std::fs::remove_dir_all(&root);
}
