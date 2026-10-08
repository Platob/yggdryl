//! What the capture pipeline's Iceberg reads and writes cost in requests on
//! Amazon S3 Tables, per run: the store and control-plane counts
//! `docs/media/iceberg.md` states under "The capture pipeline on Amazon S3
//! Tables", pinned exactly.
//!
//! The shape is `python/tests/medallion.py`'s over the fake control plane and
//! the fake object store of `rust/tests/support/` ([`S3TablesFake`],
//! [`FakeS3`]): two table buckets, `bronze` and `silver`, one namespace each,
//! every table format version 3, partitioned by
//! `time_bucket('15 minutes', currunix) as partunix`, sorted by
//! `partunix, currunix, seqnum, currhashcode` and keyed by
//! `currunix, crosshashcode, seqnum, currhashcode`. The first stage is a keyed
//! append, every later one a `read_serie` of the window then an
//! `overwrite_serie`, and the books are read once into three event tables.
//! The FIX codec, the lifecycle and the book fold are left out - each stage
//! writes the rows it read - so what is counted is the Iceberg read and write
//! alone.
//!
//! A run is one keyed append, four reads and six overwrites of a
//! two-quarter window: run 1 creates every table over window A, run 2 is
//! window B through the handles run 1 kept, run 3 reruns window B through
//! them, and run 4 reruns it in a fresh lake - the next process. The small
//! scenario writes 200 rows of 64-byte bodies a quarter hour, the large one
//! 6,000 of 512 bytes. No run sends a store a `HEAD` or a listing.
//!
//! `--nocapture` prints the table the page states; `LEDGER_OUT` names a JSON
//! file the per-stage ledger - by method, by table and object kind, every
//! range asked for, every control-plane operation - is written to.

#![cfg(feature = "s3tables")]

#[path = "support/s3tables.rs"]
mod fake;
#[path = "support/server.rs"]
mod server;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray, TimestampNanosecondArray};
use fake::{ACCESS_KEY, REGION, S3TablesFake, SECRET_KEY};
use serde_json::{Value, json};
use server::FakeS3;
use yggdryl::aws::{Credentials, Session};
use yggdryl::iceberg::assign_field_ids;
use yggdryl::media::IORecordOptions;
use yggdryl::s3tables::{S3Tables, S3TablesCatalog};
use yggdryl::{
    Arn, ArrowCastOptions, Catalog, ChunkedSerie, DataType, Field, IOMedia, Namespace, Properties,
    Serie, StreamChunkedSerie, StructType, Table, TimeUnit, Timezone,
};

const QUARTER: i64 = 15 * 60 * 1_000_000_000;
/// 2026-10-07T00:00:00Z in nanoseconds.
const DAY: i64 = 1_791_331_200 * 1_000_000_000;
const NAMESPACE: &str = "record_keeping";
const BRONZE: [&str; 2] = ["log_messages", "fix_messages"];
const SILVER: [&str; 6] = [
    "fix_messages",
    "books",
    "orders",
    "quotes",
    "executions",
    "instruments",
];

/// What one run cost: every request the object store and the control plane
/// were sent, and of the store's the `HEAD`s and the listings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cost {
    store: usize,
    control: usize,
    head: usize,
    list: usize,
}

/// A run that sent `store` requests to the stores and `control` to the
/// control plane, none of them a `HEAD` or a listing.
const fn cost(store: usize, control: usize) -> Cost {
    Cost {
        store,
        control,
        head: 0,
        list: 0,
    }
}

/// The four runs, as the page's table names them.
const RUNS: [&str; 4] = [
    "run 1, window A, every table created",
    "run 2, window B, the handles held",
    "run 3, window B again, the handles held",
    "run 4, window B again, a fresh lake",
];
/// The small scenario's runs: 200 rows of 64-byte bodies a quarter hour,
/// every data file read whole by one suffix-ranged `GET` of its end.
const SMALL: [Cost; 4] = [cost(54, 39), cost(47, 7), cost(51, 6), cost(66, 15)];
/// The large scenario's runs: 6,000 rows of 512-byte bodies a quarter hour.
const LARGE: [Cost; 4] = [cost(62, 39), cost(55, 7), cost(61, 6), cost(76, 15)];

fn client(fake: &S3TablesFake) -> S3Tables {
    S3Tables::new(
        Session::new()
            .with_environment(false)
            .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY))
            .with_region(REGION),
    )
    .try_with_endpoint_url(fake.endpoint())
    .expect("the fake's endpoint")
}

fn catalog(fake: &S3TablesFake, store: &FakeS3, name: &str, arn: &str) -> Catalog {
    Catalog::from(
        S3TablesCatalog::new(name, client(fake), Arn::from_str(arn).expect("an ARN"))
            .expect("a table bucket's ARN")
            .with_properties(
                Properties::new()
                    .with_property("s3.endpoint", store.endpoint())
                    .with_property("path_style", "true"),
            ),
    )
}

/// The row a stage writes: the event columns the pipeline keys on and a body.
fn row() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            }
            .required_field("currunix"),
            DataType::Int64.required_field("crosshashcode"),
            DataType::Int64.required_field("seqnum"),
            DataType::Int64.required_field("currhashcode"),
            DataType::utf8().required_field("crosscode"),
            DataType::utf8().nullable_field("body"),
        ])
        .expect("distinct columns"),
    )
    .required_field("row")
}

/// `row` laid out as `medallion.py` lays every table out.
fn declared() -> Field {
    let mut schema = row()
        .with_partition_by(["time_bucket('15 minutes', currunix) as partunix"
            .parse()
            .expect("a projection")])
        .expect("a partition");
    schema
        .as_sort_mut()
        .set_by_texts(["partunix", "currunix", "seqnum", "currhashcode"])
        .expect("an order");
    assign_field_ids(&mut schema, 1).expect("numbered");
    let mut key: Vec<i32> = ["currunix", "crosshashcode", "seqnum", "currhashcode"]
        .iter()
        .map(|name| {
            schema
                .get_field_by_path(name)
                .expect("a key column")
                .parquet_field_id()
                .expect("an id")
                .expect("numbered")
        })
        .collect();
    key.sort_unstable();
    schema
        .as_iceberg_mut()
        .set_identifier_field_ids(&key)
        .expect("the key");
    schema
}

fn properties() -> Properties {
    Properties::new().with_property("format-version", "3")
}

/// `per_quarter` rows in each quarter of `quarters`, each body `body_bytes`
/// of pseudo-random hex, so the encoding cannot shrink it away.
fn lines(quarters: std::ops::Range<i64>, per_quarter: usize, body_bytes: usize) -> Serie {
    let schema = row().into_arrow_schema().expect("an Arrow schema");
    let step = QUARTER / i64::try_from(per_quarter).expect("a count");
    let (mut ts, mut cross, mut seq, mut hash, mut code, mut body) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for quarter in quarters {
        for index in 0..i64::try_from(per_quarter).expect("a count") {
            let mut text = String::with_capacity(body_bytes + 16);
            while text.len() < body_bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                write!(text, "{state:016x}").expect("a write into a string");
            }
            text.truncate(body_bytes);
            ts.push(DAY + quarter * QUARTER + index * step);
            cross.push(7_i64);
            seq.push(index);
            hash.push(state.cast_signed());
            code.push("s3://capture/a.log");
            body.push(text);
        }
    }
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(
                TimestampNanosecondArray::from(ts)
                    .with_data_type(schema.field(0).data_type().clone()),
            ),
            Arc::new(Int64Array::from(cross)),
            Arc::new(Int64Array::from(seq)),
            Arc::new(Int64Array::from(hash)),
            Arc::new(StringArray::from(code)),
            Arc::new(StringArray::from(body)),
        ],
    )
    .expect("a batch");
    Serie::from_arrow_batch(Some(&row()), &batch, ArrowCastOptions::default()).expect("a record")
}

fn iso(quarter: i64) -> String {
    let minutes = quarter * 15;
    format!("2026-10-07T{:02}:{:02}:00Z", minutes / 60, minutes % 60)
}

fn kind_of(key: &str) -> &'static str {
    if key.ends_with(".metadata.json") {
        "metadata"
    } else if key.contains("snap-") && key.ends_with(".avro") {
        "manifest_list"
    } else if key.ends_with(".avro") {
        "manifest"
    } else if key.ends_with(".parquet") {
        "data"
    } else {
        "other"
    }
}

/// One scenario's fakes, what each of its runs cost, and every stage's
/// detail.
struct Ledger {
    fake: S3TablesFake,
    store: FakeS3,
    bronze_arn: String,
    silver_arn: String,
    scenario: &'static str,
    per_quarter: usize,
    body_bytes: usize,
    run: usize,
    costs: [Cost; 4],
    stages: Vec<Value>,
}

impl Ledger {
    fn new(scenario: &'static str, per_quarter: usize, body_bytes: usize) -> Self {
        let fake = S3TablesFake::start();
        let store = FakeS3::start();
        store.create_buckets_on_write(true);
        let bronze_arn = fake.seed_bucket("bronze");
        let silver_arn = fake.seed_bucket("silver");
        Self {
            fake,
            store,
            bronze_arn,
            silver_arn,
            scenario,
            per_quarter,
            body_bytes,
            run: 0,
            costs: [Cost::default(); 4],
            stages: Vec::new(),
        }
    }

    /// Every table's warehouse bucket, named `catalog.table`.
    fn buckets(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for (catalog, names) in [("bronze", &BRONZE[..]), ("silver", &SILVER[..])] {
            for name in names {
                if let Some(state) = self.fake.table(catalog, NAMESPACE, name) {
                    let bucket = state
                        .warehouse_location
                        .trim_start_matches("s3://")
                        .trim_end_matches('/')
                        .to_owned();
                    map.insert(bucket, format!("{catalog}.{name}"));
                }
            }
        }
        map
    }

    /// `operation`, its requests added to the current run's cost and
    /// recorded as the stage `name`.
    fn stage<T>(&mut self, name: &str, operation: impl FnOnce() -> T) -> T {
        self.fake.clear_requests();
        self.store.clear_requests();
        let answer = operation();
        let buckets = self.buckets();
        let mut by_method: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_table: BTreeMap<String, usize> = BTreeMap::new();
        let mut ranges: Vec<String> = Vec::new();
        let store_requests = self.store.requests();
        let cost = &mut self.costs[self.run];
        for request in &store_requests {
            let listing = request
                .query
                .iter()
                .any(|(name, value)| name == "list-type" && value == "2");
            let range = request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("range"))
                .map(|(_, value)| value.clone());
            cost.head += usize::from(request.method == "HEAD");
            cost.list += usize::from(listing);
            let method = match (request.method.as_str(), listing, &range) {
                ("GET", true, _) => "LIST".to_owned(),
                ("GET", false, Some(_)) => "GET range".to_owned(),
                (method, _, _) => method.to_owned(),
            };
            let method = if request.status == 404 {
                format!("{method} 404")
            } else {
                method
            };
            *by_method.entry(method.clone()).or_default() += 1;
            let bucket = request
                .path
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or("");
            let table = buckets
                .get(bucket)
                .cloned()
                .unwrap_or_else(|| format!("?{bucket}"));
            let kind = kind_of(request.key.as_deref().unwrap_or_default());
            *by_table
                .entry(format!("{table} {kind} {method}"))
                .or_default() += 1;
            if let Some(range) = range {
                ranges.push(format!("{table} {kind} {range}"));
            }
        }
        let mut control: BTreeMap<String, usize> = BTreeMap::new();
        for request in self.fake.requests() {
            let segments: Vec<&str> = request.path.trim_start_matches('/').split('/').collect();
            let operation = match segments.as_slice() {
                [first] => format!("{} /{first}", request.method),
                [first, .., last] => format!("{} /{first}/../{last}", request.method),
                [] => format!("{} /", request.method),
            };
            let operation = if request.status >= 400 {
                format!("{operation} {}", request.status)
            } else {
                operation
            };
            *control.entry(operation).or_default() += 1;
        }
        let control_total = control.values().sum::<usize>();
        cost.store += store_requests.len();
        cost.control += control_total;
        self.stages.push(json!({
            "scenario": self.scenario,
            "run": RUNS[self.run],
            "stage": name,
            "store_total": store_requests.len(),
            "store_by_method": by_method,
            "store_by_table_kind_method": by_table,
            "store_ranges": ranges,
            "control_total": control_total,
            "control_by_op": control,
        }));
        answer
    }
}

/// The lake a run writes through: two catalogs, their namespaces, and the
/// tables a stage opened or created, each one object every stage reads and
/// writes through by reference - as `medallion.py`'s lake keeps one Python
/// object per table, where a Rust `Table` clone opens afresh.
struct Lake {
    bronze: Catalog,
    silver: Catalog,
    namespaces: BTreeMap<&'static str, Namespace>,
    tables: BTreeMap<(&'static str, &'static str), Table>,
}

impl Lake {
    fn new(ledger: &Ledger) -> Self {
        Self {
            bronze: catalog(&ledger.fake, &ledger.store, "bronze", &ledger.bronze_arn),
            silver: catalog(&ledger.fake, &ledger.store, "silver", &ledger.silver_arn),
            namespaces: BTreeMap::new(),
            tables: BTreeMap::new(),
        }
    }

    fn catalog(&self, name: &str) -> &Catalog {
        if name == "bronze" {
            &self.bronze
        } else {
            &self.silver
        }
    }

    fn held(&mut self, key: (&'static str, &'static str)) -> &mut Table {
        self.tables.get_mut(&key).expect("held")
    }

    /// The table a stage writes, opened or created under its namespace.
    fn target(&mut self, catalog: &'static str, name: &'static str) {
        if self.tables.contains_key(&(catalog, name)) {
            return;
        }
        if !self.namespaces.contains_key(catalog) {
            let namespace = self
                .catalog(catalog)
                .namespaces()
                .open_or_create(NAMESPACE, &Properties::new())
                .expect("the namespace");
            self.namespaces.insert(catalog, namespace);
        }
        let table = self.namespaces[catalog]
            .tables()
            .open_or_create(name, &declared(), &properties())
            .expect("the table");
        self.tables.insert((catalog, name), table);
    }

    /// The table a stage reads, resolved through its catalog.
    fn source(&mut self, catalog: &'static str, name: &'static str) {
        if self.tables.contains_key(&(catalog, name)) {
            return;
        }
        let table = self
            .catalog(catalog)
            .table(format!("{NAMESPACE}.{name}").as_str())
            .expect("the source");
        self.tables.insert((catalog, name), table);
    }
}

/// The window's rows of `table`, drained: `read_serie` with the window and
/// the `select` pushed in.
fn stored_rows(table: &Table, start: i64, end: i64) -> Serie {
    let options = table
        .record_options()
        .expect("the table's options")
        .with_select("* exclude (partunix)")
        .expect("a select")
        .with_filter(format!(
            "currunix >= '{}' and currunix < '{}'",
            iso(start),
            iso(end)
        ))
        .expect("a filter");
    let stream = StreamChunkedSerie::from_serie(table.read_serie(Some(&options)).expect("a read"))
        .expect("a record stream");
    let field = stream.field().clone();
    let chunks: Vec<Serie> = stream
        .into_chunks()
        .map(|chunk| chunk.expect("a chunk"))
        .collect();
    Serie::from(
        ChunkedSerie::from_series(Some(&field), chunks, ArrowCastOptions::default())
            .expect("the chunks"),
    )
}

/// Every stage over the window `[start, end)`, in quarters of the day, as
/// run `run`: the keyed append, the three stages each reading the one
/// before, and the books read once into the three event tables.
fn run(ledger: &mut Ledger, lake: &mut Lake, run: usize, start: i64, end: i64) {
    ledger.run = run;
    let rows = lines(start..end, ledger.per_quarter, ledger.body_bytes);
    let window = rows.len();
    // A window's rows are new to the table the first time and declined by
    // its key every time after.
    let appended = if run < 2 { window } else { 0 };
    let key = ("bronze", "log_messages");
    ledger.stage("bronze.log_messages target", || lake.target(key.0, key.1));
    let result = ledger.stage("bronze.log_messages append", || {
        lake.held(key).append_serie(rows, None).expect("an append")
    });
    assert_eq!(
        usize::try_from(result.written_rows).expect("a count"),
        appended,
        "{} {}: the keyed append",
        ledger.scenario,
        RUNS[run]
    );

    for (source, target, stage) in [
        (
            ("bronze", "log_messages"),
            ("bronze", "fix_messages"),
            "bronze.fix_messages",
        ),
        (
            ("bronze", "fix_messages"),
            ("silver", "fix_messages"),
            "silver.fix_messages",
        ),
        (
            ("silver", "fix_messages"),
            ("silver", "books"),
            "silver.books",
        ),
    ] {
        ledger.stage(&format!("{stage} source"), || {
            lake.source(source.0, source.1)
        });
        let read = ledger.stage(&format!("{stage} read"), || {
            stored_rows(lake.held(source), start, end)
        });
        assert_eq!(
            read.len(),
            window,
            "{} {}: {stage} read",
            ledger.scenario,
            RUNS[run]
        );
        ledger.stage(&format!("{stage} target"), || {
            lake.target(target.0, target.1)
        });
        ledger.stage(&format!("{stage} overwrite"), || {
            lake.held(target)
                .overwrite_serie(read, None)
                .expect("an overwrite")
        });
    }

    ledger.stage("silver.events source", || lake.source("silver", "books"));
    let books = ledger.stage("silver.events read", || {
        stored_rows(lake.held(("silver", "books")), start, end)
    });
    assert_eq!(
        books.len(),
        window,
        "{} {}: the books read",
        ledger.scenario,
        RUNS[run]
    );
    for name in ["orders", "quotes", "executions"] {
        ledger.stage(&format!("silver.{name} target"), || {
            lake.target("silver", name)
        });
        let rows = books.clone();
        ledger.stage(&format!("silver.{name} overwrite"), || {
            lake.held(("silver", name))
                .overwrite_serie(rows, None)
                .expect("an overwrite")
        });
    }
}

/// The four runs of one scenario: run 1 creates every table over window A;
/// run 2 is window B through the handles run 1 kept; run 3 reruns window B
/// through the same handles; run 4 is window B again in a fresh lake - the
/// next process - every source resolved through its catalog and every target
/// opened.
fn scenario(name: &'static str, per_quarter: usize, body_bytes: usize) -> Ledger {
    let mut ledger = Ledger::new(name, per_quarter, body_bytes);
    let mut lake = Lake::new(&ledger);
    run(&mut ledger, &mut lake, 0, 0, 2);
    run(&mut ledger, &mut lake, 1, 2, 4);
    run(&mut ledger, &mut lake, 2, 2, 4);
    let mut fresh = Lake::new(&ledger);
    run(&mut ledger, &mut fresh, 3, 2, 4);
    ledger
}

#[test]
fn every_run_of_the_pipeline_costs_the_requests_the_page_states() {
    let small = scenario("small", 200, 64);
    let large = scenario("large", 6_000, 512);

    println!(
        "| per run | small, store | large, store | control plane, small and large | HEAD | listings |"
    );
    println!("| --- | ---: | ---: | ---: | ---: | ---: |");
    for (index, label) in RUNS.iter().enumerate() {
        let (small, large) = (small.costs[index], large.costs[index]);
        let control = if small.control == large.control {
            small.control.to_string()
        } else {
            format!("{} and {}", small.control, large.control)
        };
        println!(
            "| {label} | {} | {} | {control} | {} | {} |",
            small.store,
            large.store,
            small.head + large.head,
            small.list + large.list
        );
    }
    if let Some(path) = std::env::var_os("LEDGER_OUT") {
        let ledger = json!({
            "small": small.stages,
            "large": large.stages,
        });
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&ledger).expect("a JSON document"),
        )
        .expect("the ledger written");
        println!("ledger written to {}", path.to_string_lossy());
    }

    assert_eq!(small.costs, SMALL, "the small scenario's runs");
    assert_eq!(large.costs, LARGE, "the large scenario's runs");
}
