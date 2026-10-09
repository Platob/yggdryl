//! `rust/src/media/options.rs`: the row skip, the row and byte limits, and
//! the cadence.
//!
//! Everything a caller states and observes reaches the crate through
//! `yggdryl::`. The slicer a declared commit cadence pulls through is
//! crate-private, and that it cuts a stream without reading ahead is what
//! bounds a streamed write, so those three reach `yggdryl::internals`.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
use arrow_schema::{ArrowError, SchemaRef};

use yggdryl::IOMedia;
use yggdryl::arrow::BatchReader;
use yggdryl::avro::AvroOptions;
use yggdryl::excel::ExcelOptions;
use yggdryl::holder::Buffer;
use yggdryl::ipc::IpcOptions;
use yggdryl::media::{IORecordOptions, RecordOptions};
#[cfg(feature = "parquet")]
use yggdryl::parquet::ParquetOptions;
use yggdryl::{DataType, Field, StructType, Url};

/// A struct field is the schema of the batches it describes.
fn schema() -> Field {
    StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
}

/// One batch holding `ids` as its only column.
fn batch(ids: std::ops::Range<i64>) -> RecordBatch {
    RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![Arc::new(Int64Array::from_iter_values(ids))],
    )
    .unwrap()
}

/// A reader over `count` batches of `per_batch` rows each.
fn reader(count: i64, per_batch: i64) -> BatchReader {
    let batches: Vec<RecordBatch> = (0..count)
        .map(|index| batch(index * per_batch..(index + 1) * per_batch))
        .collect();
    yggdryl::arrow::batch_reader(schema().into_arrow_schema().unwrap(), batches)
}

/// The total rows a limited reader yields.
fn rows(reader: BatchReader) -> usize {
    reader.map(|batch| batch.unwrap().num_rows()).sum()
}

#[test]
fn the_declared_field_is_one_section_of_the_plan() {
    let declared = schema();
    let mut options = IpcOptions::new();

    // Nothing declared: no field, the default root name, an empty plan.
    assert!(options.field().is_none());
    assert_eq!(options.name(), yggdryl::media::DEFAULT_ROOT_NAME);
    assert!(options.plan().is_empty());
    let message = options.require_field().unwrap_err().to_string();
    assert!(message.contains("with_field"), "{message}");
    assert!(message.contains("with_dtype"), "{message}");

    // A field declares the `create` section and builds back the same.
    options.set_field(declared.clone());
    assert_eq!(options.name(), "row");
    assert_eq!(options.plan().to_string(), "create (id int64 not null)");
    assert_eq!(options.field(), Some(declared.clone()));
    assert_eq!(options.require_field().unwrap(), declared);

    // Declaring the datatype alone spells the same declaration: one stored
    // form, so the two compare and hash equal.
    let by_dtype = IpcOptions::new().with_dtype(declared.dtype().clone());
    assert_eq!(options, by_dtype);
    assert_eq!(
        RecordOptions::Ipc(options.clone()).stable_hash(),
        RecordOptions::Ipc(by_dtype).stable_hash()
    );

    // The root name is the `create` target, and the field takes it.
    options = options.with_name("trade");
    assert_eq!(options.name(), "trade");
    assert_eq!(
        options.plan().to_string(),
        "create trade (id int64 not null)"
    );
    assert_eq!(options.field().unwrap().name(), "trade");
    assert_eq!(options.field().unwrap().dtype(), declared.dtype());

    // Column metadata rides the declaration, spelled and read back.
    let mut child = declared.fields()[0].clone();
    child.insert_metadata("FIELD:comment", "the key").unwrap();
    let sourced = declared
        .clone()
        .try_with_dtype(DataType::from(StructType::from_fields([child]).unwrap()))
        .unwrap();
    let with_metadata = IpcOptions::new().with_field(sourced.clone());
    assert_eq!(
        with_metadata.plan().to_string(),
        "create (id int64 not null with (\"FIELD:comment\" = 'the key'))"
    );
    assert_eq!(with_metadata.field(), Some(sourced.clone()));
    let respelled = IpcOptions::new()
        .with_plan(with_metadata.plan().to_string())
        .unwrap();
    assert_eq!(respelled.field(), Some(sourced));

    // A declared field's nullability is not part of the declaration: the
    // build is the non-null row root.
    let nullable = IpcOptions::new().with_field(declared.clone().with_nullable(true));
    assert!(!nullable.field().unwrap().is_nullable());
    assert_eq!(nullable, IpcOptions::new().with_field(declared.clone()));

    // Taking the field clears the `create` columns; the name still names the
    // root a delegated write infers.
    let mut taken = options.clone();
    assert_eq!(taken.take_field(), Some(options.field().unwrap()));
    assert!(taken.field().is_none());
    assert_eq!(taken.name(), "trade");
    assert_eq!(taken.take_field(), None);

    // A plan lands in the properties section by section, and composes back.
    let planned = IpcOptions::new()
        .with_plan("create trade (id int64 not null) upsert by (id) select id where id > 1")
        .unwrap();
    assert_eq!(
        planned.plan().to_string(),
        "create trade (id int64 not null) upsert by (id) select id where id > 1"
    );
    assert_eq!(planned.name(), "trade");
    assert_eq!(planned.merge_by().to_string(), "id");
    assert_eq!(planned.select().to_string(), "id");
    assert_eq!(planned.filter().to_string(), "id > 1");
    assert_eq!(planned.apply_columns(), Some(vec!["id".to_owned()]));
    let narrowed = planned.with_filter("id = 7 and venue = 'XNAS'").unwrap();
    assert_eq!(
        narrowed.partition_pairs(),
        vec![
            ("id".to_owned(), "7".to_owned()),
            ("venue".to_owned(), "XNAS".to_owned())
        ]
    );

    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let options = RecordOptions::for_media_type(&media_type)
        .unwrap()
        .with_field(declared.clone());
    assert_eq!(options.field(), Some(declared.clone()));

    let RecordOptions::Ipc(inner) = options else {
        panic!("an arrows handle names the IPC encoding");
    };
    assert_eq!(inner.field.as_ref().map(Field::name), Some("row"));
    assert_eq!(inner.field, Some(declared));
}

#[test]
fn a_batch_is_shaped_as_a_reader_shapes_it() {
    use arrow_array::{Array, ArrayRef, Int32Array, StringArray};

    // A declared cast, a `where` over a column only the `select` builds, and
    // a stored field the shaped rows are completed onto.
    let declared = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let stored = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("ticker"),
        DataType::utf8().nullable_field("venue"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let options = RecordOptions::Ipc(IpcOptions::new())
        .with_field(declared)
        .with_select("id, trim(symbol) as ticker")
        .unwrap()
        .with_filter("ticker = 'MSFT'")
        .unwrap();
    let source = RecordBatch::try_from_iter([
        ("id", Arc::new(Int32Array::from(vec![1, 2, 3])) as ArrayRef),
        (
            "symbol",
            Arc::new(StringArray::from(vec![" AAPL", "MSFT ", " IBM"])),
        ),
    ])
    .unwrap();

    let shaped = options
        .apply_arrow_batch(source.clone(), Some(&stored))
        .unwrap();
    let streamed = options
        .apply_arrow_reader(
            yggdryl::arrow::batch_reader(source.schema(), [source]),
            Some(&stored),
        )
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();

    assert_eq!(streamed, std::slice::from_ref(&shaped));
    assert_eq!(shaped.schema(), stored.into_arrow_schema().unwrap());
    let ids = shaped
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(ids.values(), &[2]);
    assert!(shaped.column(2).is_null(0));
}

#[test]
fn record_options_have_complete_value_traits_and_stable_hashes() {
    fn assert_traits<T: Clone + Eq + Ord + std::hash::Hash>(_: &T) {}

    let text = yggdryl::text::TextOptions::new()
        .try_with_rowheader(r"^(?<id>\d+)")
        .unwrap();
    let options = RecordOptions::from(text.clone());
    let equal = options.clone();
    let changed = RecordOptions::from(text.clone().with_batch_row_size(3));

    assert_traits(&text);
    assert_traits(&options);
    assert_eq!(options, equal);
    assert_eq!(options.stable_hash(), equal.stable_hash());
    assert_ne!(options, changed);
    assert_ne!(options.stable_hash(), changed.stable_hash());

    // The workbook's own settings are part of its value and of its hash.
    let excel = ExcelOptions::new().with_sheet("Trades");
    let options = RecordOptions::from(excel.clone());
    let equal = RecordOptions::from(ExcelOptions::new().with_sheet("Trades"));
    assert_traits(&excel);
    assert_eq!(options, equal);
    assert_eq!(options.stable_hash(), equal.stable_hash());
    for changed in [
        excel.clone().with_sheet("Quotes"),
        excel.clone().with_header(false),
        excel.clone().with_range("A3:F".parse().unwrap()),
        excel.clone().with_batch_row_size(3),
    ] {
        let changed = RecordOptions::from(changed);
        assert_ne!(options, changed, "{changed:?}");
        assert_ne!(options.stable_hash(), changed.stable_hash(), "{changed:?}");
    }
}

#[test]
fn a_zero_row_limit_reads_the_schema_and_no_batches() {
    let options = IpcOptions::new().with_max_row_size(0);
    let mut limited = options.limit_arrow_reader(reader(3, 2)).unwrap();

    // The schema still answers - `Some(0)` is a valid ask, not an error.
    assert_eq!(limited.schema(), schema().into_arrow_schema().unwrap());
    assert!(limited.next().is_none());
}

#[test]
fn a_zero_byte_limit_reads_the_schema_and_no_batches() {
    let options = IpcOptions::new().with_max_byte_size(0);
    let mut limited = options.limit_arrow_reader(reader(3, 2)).unwrap();

    assert_eq!(limited.schema(), schema().into_arrow_schema().unwrap());
    assert!(limited.next().is_none());
}

#[test]
fn a_row_limit_at_exactly_the_stored_count_keeps_every_row() {
    let options = IpcOptions::new().with_max_row_size(6);
    assert_eq!(rows(options.limit_arrow_reader(reader(3, 2)).unwrap()), 6);
}

#[test]
fn a_row_limit_one_below_the_stored_count_slices_the_last_batch() {
    let options = IpcOptions::new().with_max_row_size(5);
    let batches: Vec<RecordBatch> = options
        .limit_arrow_reader(reader(3, 2))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    // The bound is exact: the last batch is cut mid-way, not dropped whole.
    assert_eq!(
        batches
            .iter()
            .map(RecordBatch::num_rows)
            .collect::<Vec<_>>(),
        [2, 2, 1]
    );
}

#[test]
fn a_row_limit_one_above_the_stored_count_changes_nothing() {
    let options = IpcOptions::new().with_max_row_size(7);
    assert_eq!(rows(options.limit_arrow_reader(reader(3, 2)).unwrap()), 6);
}

#[test]
fn a_byte_limit_landing_mid_batch_slices_a_view_of_the_last_batch() {
    let source = batch(0..8);
    // The limit counts bytes as `memory_size` does - the rows' own extent,
    // eight bytes a row here, not Arrow's whole-buffer accounting - so one
    // byte short of the batch is seven rows, handed out as a slice.
    let size = u64::try_from(yggdryl::arrow::memory_size(&source)).unwrap();
    assert_eq!(size, 8 * 8);
    let options = IpcOptions::new().with_max_byte_size(size - 1);
    let batches: Vec<RecordBatch> = options
        .limit_arrow_reader(yggdryl::arrow::batch_reader(
            source.schema(),
            [source.clone()],
        ))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 7);
    // A slice is a view: the cut batch's column still points at the source's
    // buffer rather than at a copy of it.
    assert_eq!(
        batches[0].column(0).to_data().buffers()[0].as_ptr(),
        source.column(0).to_data().buffers()[0].as_ptr()
    );
}

#[test]
fn a_nonzero_byte_limit_smaller_than_one_row_still_yields_one_row() {
    let options = IpcOptions::new().with_max_byte_size(1);
    let batches: Vec<RecordBatch> = options
        .limit_arrow_reader(reader(2, 4))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    // A bounded read must never be a silent total loss: only `Some(0)` yields
    // nothing.
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
}

#[test]
fn whichever_limit_binds_first_wins() {
    // The byte budget admits everything, so the row bound is what cuts.
    let generous = IpcOptions::new()
        .with_max_row_size(3)
        .with_max_byte_size(u64::MAX);
    assert_eq!(rows(generous.limit_arrow_reader(reader(3, 2)).unwrap()), 3);

    // One byte admits one row, so the byte bound cuts before the row bound.
    let tight = IpcOptions::new().with_max_row_size(3).with_max_byte_size(1);
    assert_eq!(rows(tight.limit_arrow_reader(reader(3, 2)).unwrap()), 1);
}

/// A reader counting how often its source is pulled.
struct Counting {
    inner: BatchReader,
    pulls: Arc<std::sync::atomic::AtomicUsize>,
}

impl Iterator for Counting {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.pulls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.next()
    }
}

impl RecordBatchReader for Counting {
    fn schema(&self) -> SchemaRef {
        self.inner.schema()
    }
}

#[test]
fn a_satisfied_limit_stops_pulling_the_inner_reader() {
    let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Box::new(Counting {
        inner: reader(20, 2),
        pulls: Arc::clone(&pulls),
    });
    let options = IpcOptions::new().with_max_row_size(10);
    let mut limited = options.limit_arrow_reader(counted).unwrap();

    // At most one batch is retained: every yield costs exactly one pull, so
    // nothing is prefetched or buffered behind the caller's back.
    for served in 1..=5 {
        assert_eq!(limited.next().unwrap().unwrap().num_rows(), 2);
        assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), served);
    }

    // The tenth row satisfied the limit, so the source is never touched
    // again - fifteen batches of the file go undecoded.
    assert!(limited.next().is_none());
    assert!(limited.next().is_none());
    assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 5);
}

/// What `docs/expression/grammar.md` states for an `unnest` standing as a
/// key: a key is one value per row, and an unnest is one row per element.
const UNNEST_IN_A_KEY: &str = "unnest is a select-list form: expected `unnest(xs)` as the whole term of a projection, got it in a key";

#[test]
fn an_unnest_is_refused_as_a_match_key_at_every_door_that_takes_one() {
    let refused = |error: yggdryl::Error| {
        let message = error.to_string();
        assert!(message.contains(UNNEST_IN_A_KEY), "{message}");
    };
    refused(IpcOptions::new().with_merge_by("unnest(xs)").unwrap_err());
    // `explode` is the same verb, and an alias does not make it one value.
    refused(
        IpcOptions::new()
            .with_merge_by("id, explode(xs) as x")
            .unwrap_err(),
    );
    refused(
        IpcOptions::new()
            .with_merge_by_scalar(&yggdryl::Scalar::from("unnest(xs)"))
            .unwrap_err(),
    );
    // An upsert's `by (...)` is the same key.
    refused(
        IpcOptions::new()
            .with_plan("upsert by (unnest(xs))")
            .unwrap_err(),
    );
    // A computed key that reads one value per row is still a key.
    let keyed = IpcOptions::new().with_merge_by("id, lower(tag)").unwrap();
    assert_eq!(keyed.merge_by().to_string(), "id, lower(tag)");
}

#[test]
fn a_false_merge_key_is_refused_and_leaves_the_key_it_held() {
    let mut options = IpcOptions::new().with_merge_by("id").unwrap();
    let message = options
        .set_merge_by_scalar(&yggdryl::Scalar::from(false))
        .unwrap_err()
        .to_string();
    assert!(message.contains("$.merge_by"), "{message}");
    assert!(
        message.contains(
            "expected a column list, a selector text, null, or true (the destination's own key), got false"
        ),
        "{message}"
    );
    assert_eq!(options.merge_by().to_string(), "id");
    // An integer is no boolean: it stays the selector reader's refusal.
    let message = IpcOptions::new()
        .with_merge_by_scalar(&yggdryl::Scalar::from(1_i64))
        .unwrap_err()
        .to_string();
    assert!(!message.contains("got false"), "{message}");
}

#[test]
fn a_true_merge_key_is_the_destinations_own_and_clears_a_stated_one() {
    let mut options = IpcOptions::new().with_merge_by("id").unwrap();
    options
        .set_merge_by_scalar(&yggdryl::Scalar::from(true))
        .unwrap();
    assert!(options.merge_by().is_empty());
    // It is the state null spells, and the default: no flag is kept.
    assert_eq!(options, IpcOptions::new());
    assert_eq!(
        IpcOptions::new()
            .with_merge_by_scalar(&yggdryl::Scalar::Null)
            .unwrap(),
        options
    );
    let options = RecordOptions::Ipc(IpcOptions::new())
        .with_merge_by("id")
        .unwrap()
        .with_merge_by_scalar(&yggdryl::Scalar::from(true))
        .unwrap();
    assert_eq!(options, RecordOptions::Ipc(IpcOptions::new()));
    // A merge under it is keyed by the destination, and a bare options
    // value states none.
    let message = options
        .require_write_mode(yggdryl::IOMode::Merge)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("requires at least one merge_by column"),
        "{message}"
    );
    // Nothing is stated, so an append under it is not refused.
    options.require_write_mode(yggdryl::IOMode::Append).unwrap();
}

#[test]
fn a_plan_that_joins_rows_is_refused_and_leaves_the_options_unchanged() {
    let mut options = IpcOptions::new().with_filter("id > 1").unwrap();
    let before = options.clone();
    let plan = "select id from trades join venues using (venue) where id > 2"
        .parse()
        .unwrap();
    let message = options.set_plan(plan).unwrap_err().to_string();
    assert!(message.contains("$.join"), "{message}");
    assert!(
        message.contains("record options hold no join section"),
        "{message}"
    );
    assert_eq!(options, before);
    // The text door refuses it the same way.
    let message = IpcOptions::new()
        .with_plan("select * from t left join v on id = vid")
        .unwrap_err()
        .to_string();
    assert!(message.contains("$.join"), "{message}");
}

#[test]
fn a_limit_with_a_match_key_is_refused_naming_both_settings() {
    let options = IpcOptions::new()
        .with_max_row_size(10)
        .with_merge_by(["id"])
        .unwrap();
    let Err(error) = options.limit_arrow_reader(reader(1, 2)) else {
        panic!("a limited merge must be refused");
    };
    let message = error.to_string();

    assert!(message.contains("max_row_size = 10"), "{message}");
    assert!(message.contains("merge_by `id`"), "{message}");
}

#[test]
fn a_skip_with_a_match_key_is_refused_naming_both_settings() {
    let options = IpcOptions::new()
        .with_row_offset(4)
        .with_merge_by(["id"])
        .unwrap();
    let Err(error) = options.limit_arrow_reader(reader(1, 2)) else {
        panic!("a merge that skips rows must be refused");
    };
    let message = error.to_string();

    assert!(message.contains("row_offset = 4"), "{message}");
    assert!(message.contains("merge_by `id`"), "{message}");
}

/// The ids a reader yields, batch by batch.
fn id_batches(reader: BatchReader) -> Vec<Vec<i64>> {
    reader
        .map(|batch| {
            let batch = batch.unwrap();
            batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect()
}

#[test]
fn a_row_offset_drops_the_batches_it_covers_and_cuts_the_one_it_lands_in() {
    let options = IpcOptions::new().with_row_offset(3);
    // Rows 0..6 in batches of two: the first batch is dropped whole, the
    // second is cut after its first row, and nothing empty is handed on.
    assert_eq!(
        id_batches(options.limit_arrow_reader(reader(3, 2)).unwrap()),
        [vec![3], vec![4, 5]]
    );
}

#[test]
fn a_row_offset_is_taken_before_the_row_limit_counts() {
    let options = IpcOptions::new().with_row_offset(2).with_max_row_size(3);
    assert_eq!(
        id_batches(options.limit_arrow_reader(reader(3, 2)).unwrap()),
        [vec![2, 3], vec![4]]
    );
    // A skip past the stored rows reads the schema and no batches.
    let past = IpcOptions::new().with_row_offset(9);
    let mut limited = past.limit_arrow_reader(reader(3, 2)).unwrap();
    assert_eq!(limited.schema(), schema().into_arrow_schema().unwrap());
    assert!(limited.next().is_none());
}

#[test]
fn a_plan_offset_is_the_row_offset_of_the_options() {
    let options = IpcOptions::new()
        .with_plan("select id limit 3 offset 2")
        .unwrap();
    assert_eq!(options.max_row_size(), Some(3));
    assert_eq!(options.row_offset(), Some(2));
    assert_eq!(options.plan().row_offset(), Some(2));
    assert_eq!(options.plan().row_limit(), Some(3));

    // A read through a handle skips, then bounds - the plan's offset is never
    // dropped on the way into the media.
    let mut handle = Buffer::new().with_media_type(yggdryl::MimeType::ARROW_STREAM.into());
    let stored = handle.record_options().unwrap();
    handle
        .overwrite_arrow_reader(reader(3, 2), &stored)
        .unwrap();
    let read = handle
        .record_options()
        .unwrap()
        .with_plan("select id limit 3 offset 2")
        .unwrap();
    let ids: Vec<i64> = id_batches(handle.read_arrow_reader(&read).unwrap())
        .into_iter()
        .flatten()
        .collect();
    assert_eq!(ids, [2, 3, 4]);
}

#[test]
fn a_structured_document_is_refused_as_an_encoding_naming_its_own_doors() {
    let json = Url::from_str("file:///t.json").unwrap().media_type();
    let message = RecordOptions::for_media_type(&json)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("a record encoding this build implements"),
        "{message}"
    );
    assert!(
        message.contains("a json document is one value"),
        "{message}"
    );
    assert!(message.contains("read_serie"), "{message}");
    // Any other media type is refused with the encodings alone.
    let orc = yggdryl::MediaType::new(yggdryl::MimeType::ORC);
    let message = RecordOptions::for_media_type(&orc).unwrap_err().to_string();
    assert!(!message.contains("document is one value"), "{message}");
    assert!(
        message.contains("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "{message}"
    );

    // A workbook is a record encoding, never a refused document; the legacy
    // BIFF workbook is not one.
    assert!(matches!(
        RecordOptions::for_mime_type(&yggdryl::MimeType::XLSX),
        Ok(options) if options.settings::<ExcelOptions>().is_some()
    ));
    let message = RecordOptions::for_mime_type(&yggdryl::MimeType::XLS)
        .unwrap_err()
        .to_string();
    assert!(
        message.starts_with("invalid record value at $: "),
        "{message}"
    );
    assert!(
        message.contains("got application/vnd.ms-excel"),
        "{message}"
    );
}

#[test]
fn an_empty_batch_passes_through_without_ending_the_stream() {
    let empty = RecordBatch::new_empty(schema().into_arrow_schema().unwrap());
    let source = yggdryl::arrow::batch_reader(empty.schema(), [empty, batch(0..2)]);
    let options = IpcOptions::new().with_max_row_size(2);
    assert_eq!(rows(options.limit_arrow_reader(source).unwrap()), 2);
}

#[test]
fn the_enum_mirrors_the_limits_of_the_encoding_it_holds() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let options = RecordOptions::for_media_type(&media_type)
        .unwrap()
        .with_max_row_size(7)
        .with_max_byte_size(1024);

    assert_eq!(options.max_row_size(), Some(7));
    assert_eq!(options.max_byte_size(), Some(1024));
    let RecordOptions::Ipc(inner) = options else {
        panic!("an arrows handle names the IPC encoding");
    };
    assert_eq!(inner.max_row_size, Some(7));
    assert_eq!(inner.max_byte_size, Some(1024));

    let media_type = Url::from_str("file:///t.xlsx").unwrap().media_type();
    let options = RecordOptions::for_media_type(&media_type)
        .unwrap()
        .with_max_row_size(7)
        .with_max_byte_size(1024);
    assert_eq!(options.mime_type(), yggdryl::MimeType::XLSX);
    assert_eq!(options.max_row_size(), Some(7));
    assert_eq!(options.max_byte_size(), Some(1024));
    let Some(inner) = options.settings::<ExcelOptions>() else {
        panic!("an xlsx handle names the workbook encoding");
    };
    assert_eq!(inner.max_row_size, Some(7));
    assert_eq!(inner.max_byte_size, Some(1024));
    // Everything else is the workbook's default: the first sheet, a header
    // row, the whole grid.
    assert_eq!(inner.sheet, None);
    assert!(inner.header);
    assert_eq!(inner.range, None);
    assert_eq!(inner.cells(), yggdryl::excel::CellRange::all());
}

#[test]
fn record_options_preflight_each_write_intent_without_an_input_reader() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let plain = RecordOptions::for_media_type(&media_type).unwrap();

    plain
        .require_write_mode(yggdryl::IOMode::Overwrite)
        .unwrap();
    plain.require_write_mode(yggdryl::IOMode::Append).unwrap();
    let merge = plain
        .require_write_mode(yggdryl::IOMode::Merge)
        .unwrap_err()
        .to_string();
    assert!(merge.contains("write mode merge requires"), "{merge}");
    assert!(merge.contains("$.merge_by"), "{merge}");

    let keyed = plain.with_merge_by(["id"]).unwrap();
    keyed.require_write_mode(yggdryl::IOMode::Merge).unwrap();
    for refused in [
        keyed.require_write_mode(yggdryl::IOMode::Overwrite),
        keyed.require_write_mode(yggdryl::IOMode::Append),
    ] {
        let message = refused.unwrap_err().to_string();
        assert!(message.contains("does not accept merge_by"), "{message}");
        assert!(message.contains("use merge mode"), "{message}");
    }
}

#[test]
fn every_concrete_options_type_carries_the_same_commit_cadence() {
    fn assert_cadence(mut options: impl IORecordOptions) {
        assert_eq!(options.commit_batch_num(), None);
        options.set_commit_batch_num(Some(17));
        assert_eq!(options.commit_batch_num(), Some(17));
    }

    assert_cadence(IpcOptions::new());
    assert_cadence(yggdryl::avro::AvroOptions::new());
    assert_cadence(yggdryl::text::TextOptions::new());
    assert_cadence(ExcelOptions::new());
    #[cfg(feature = "parquet")]
    assert_cadence(yggdryl::parquet::ParquetOptions::new());

    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_batch_num(5);
    assert_eq!(options.commit_batch_num(), Some(5));
    let RecordOptions::Ipc(inner) = options else {
        unreachable!()
    };
    assert_eq!(inner.commit_batch_num, Some(5));
}

#[test]
fn avro_only_options_validate_codec_and_sync_marker_in_the_core() {
    let media_type = Url::from_str("file:///t.avro").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert_eq!(
        options
            .settings::<AvroOptions>()
            .map(AvroOptions::block_codec),
        Some("deflate")
    );
    assert_eq!(
        options
            .settings::<AvroOptions>()
            .and_then(AvroOptions::sync_marker),
        None
    );

    options
        .require_settings_mut::<AvroOptions>("$.block_codec", "a block codec")
        .unwrap()
        .set_block_codec("null")
        .unwrap();
    assert_eq!(
        options
            .settings::<AvroOptions>()
            .map(AvroOptions::block_codec),
        Some("null")
    );
    let marker = *b"0123456789abcdef";
    options
        .require_settings_mut::<AvroOptions>("$.sync_marker", "a synchronization marker")
        .unwrap()
        .set_sync_marker(Some(&marker))
        .unwrap();
    assert_eq!(
        options
            .settings::<AvroOptions>()
            .and_then(AvroOptions::sync_marker),
        Some(&marker)
    );

    let mut handle = Buffer::new().with_media_type(media_type);
    handle.overwrite_arrow_batch(batch(0..2), &options).unwrap();
    assert!(handle.as_slice().ends_with(&marker));
    assert_eq!(rows(handle.read_arrow_reader(&options).unwrap()), 2);

    options
        .require_settings_mut::<AvroOptions>("$.sync_marker", "a synchronization marker")
        .unwrap()
        .set_sync_marker(None)
        .unwrap();
    assert_eq!(
        options
            .settings::<AvroOptions>()
            .and_then(AvroOptions::sync_marker),
        None
    );

    let codec = options
        .require_settings_mut::<AvroOptions>("$.block_codec", "a block codec")
        .unwrap()
        .set_block_codec("brotli")
        .unwrap_err();
    assert!(matches!(
        codec,
        yggdryl::Error::Codec { format: "avro", .. }
    ));
    assert!(codec.to_string().contains("brotli"));

    let length = options
        .require_settings_mut::<AvroOptions>("$.sync_marker", "a synchronization marker")
        .unwrap()
        .set_sync_marker(Some(b"short"))
        .unwrap_err();
    assert!(matches!(length, yggdryl::Error::InvalidRecord { .. }));
    let message = length.to_string();
    assert!(message.contains("$.sync_marker"), "{message}");
    assert!(message.contains("16 bytes"), "{message}");
    assert!(message.contains("got 5"), "{message}");
}

#[test]
fn avro_only_setters_reject_another_inferred_encoding() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert_eq!(
        options
            .settings::<AvroOptions>()
            .map(AvroOptions::block_codec),
        None
    );
    assert_eq!(
        options
            .settings::<AvroOptions>()
            .and_then(AvroOptions::sync_marker),
        None
    );
    for error in [
        options
            .require_settings_mut::<AvroOptions>("$.block_codec", "a block codec")
            .unwrap_err(),
        options
            .require_settings_mut::<AvroOptions>("$.sync_marker", "a synchronization marker")
            .unwrap_err(),
    ] {
        assert!(matches!(error, yggdryl::Error::InvalidRecord { .. }));
        let message = error.to_string();
        assert!(message.contains("Avro"), "{message}");
        assert!(message.contains("arrow.stream"), "{message}");
    }
}

#[test]
fn excel_only_options_are_owned_by_the_generic_core_variant() {
    let media_type = Url::from_str("file:///t.xlsx").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::sheet),
        None
    );
    assert_eq!(options.header(), Some(true));
    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::range),
        None
    );

    options
        .require_settings_mut::<ExcelOptions>("$.sheet", "a worksheet")
        .unwrap()
        .set_sheet(Some("Trades"))
        .unwrap();
    options.set_header(false).unwrap();
    let range: yggdryl::excel::CellRange = "B2:D9".parse().unwrap();
    options
        .require_settings_mut::<ExcelOptions>("$.range", "a cell range")
        .unwrap()
        .set_range(Some(range));
    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::sheet),
        Some("Trades")
    );
    assert_eq!(options.header(), Some(false));
    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::range),
        Some(range)
    );
    let Some(inner) = options.settings::<ExcelOptions>() else {
        panic!("an xlsx handle names the workbook encoding");
    };
    assert_eq!(inner.sheet.as_deref(), Some("Trades"));
    assert!(!inner.header);
    assert_eq!(inner.cells(), range);

    // A name Excel refuses is refused by the setter, which leaves the sheet
    // it had.
    for (name, reason) in [
        ("", "expected a sheet name, got the empty text"),
        (
            "a sheet name longer than thirty-one",
            "expected a sheet name of at most 31 characters, got 35 in",
        ),
        (
            "Q1:Q2",
            "expected a sheet name without any of \\ / ? * [ ] :, got ':' in \"Q1:Q2\"",
        ),
        (
            "'quoted'",
            "expected a sheet name that neither opens nor closes with an apostrophe",
        ),
        (
            "history",
            "expected a sheet name other than the reserved `History`",
        ),
    ] {
        let error = options
            .require_settings_mut::<ExcelOptions>("$.sheet", "a worksheet")
            .unwrap()
            .set_sheet(Some(name))
            .unwrap_err();
        assert!(
            matches!(error, yggdryl::Error::InvalidRecord { .. }),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(
            message.starts_with("invalid record value at $.sheet: "),
            "{message}"
        );
        assert!(message.contains(reason), "{name:?}: {message}");
        assert_eq!(
            options
                .settings::<ExcelOptions>()
                .and_then(ExcelOptions::sheet),
            Some("Trades"),
            "{name:?}"
        );
    }

    // `None` clears back to the defaults: the first sheet, the whole grid.
    options
        .require_settings_mut::<ExcelOptions>("$.sheet", "a worksheet")
        .unwrap()
        .set_sheet(None)
        .unwrap();
    options
        .require_settings_mut::<ExcelOptions>("$.range", "a cell range")
        .unwrap()
        .set_range(None);
    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::sheet),
        None
    );
    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::range),
        None
    );
}

#[test]
fn excel_only_setters_reject_another_inferred_encoding() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::sheet),
        None
    );
    assert_eq!(options.header(), None);
    assert_eq!(
        options
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::range),
        None
    );
    for (error, path, setting) in [
        (
            options
                .require_settings_mut::<ExcelOptions>("$.sheet", "a worksheet")
                .unwrap_err(),
            "$.sheet",
            "a worksheet",
        ),
        (
            options
                .require_settings_mut::<ExcelOptions>("$.range", "a cell range")
                .unwrap_err(),
            "$.range",
            "a cell range",
        ),
    ] {
        assert!(
            matches!(error, yggdryl::Error::InvalidRecord { .. }),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!(
                "invalid record value at {path}: expected Excel options to set {setting}, \
                 got application/vnd.apache.arrow.stream options"
            )
        );
    }
    assert_eq!(options, RecordOptions::for_media_type(&media_type).unwrap());

    // The other encodings' setters refuse workbook options the same way.
    let workbook = Url::from_str("file:///t.xlsx").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&workbook).unwrap();
    assert_eq!(
        options
            .settings::<AvroOptions>()
            .map(AvroOptions::block_codec),
        None
    );
    assert_eq!(options.timezone(), None);
    let message = options
        .require_settings_mut::<AvroOptions>("$.block_codec", "a block codec")
        .unwrap_err()
        .to_string();
    assert!(message.contains("expected Avro options"), "{message}");
    assert!(
        message.contains(
            "got application/vnd.openxmlformats-officedocument.spreadsheetml.sheet options"
        ),
        "{message}"
    );
}

#[cfg(feature = "parquet")]
#[test]
fn parquet_only_options_are_owned_by_the_generic_core_variant() {
    let media_type = Url::from_str("file:///t.parquet").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert_eq!(
        options
            .settings::<ParquetOptions>()
            .map(ParquetOptions::compression_name)
            .as_deref(),
        Some("zstd(1)")
    );
    options
        .require_settings_mut::<ParquetOptions>("$.compression", "a page compression")
        .unwrap()
        .set_compression_name("gzip(4)")
        .unwrap();
    assert_eq!(
        options
            .settings::<ParquetOptions>()
            .map(ParquetOptions::compression_name)
            .as_deref(),
        Some("gzip(4)")
    );

    options
        .require_settings_mut::<ParquetOptions>("$.max_row_group_size", "a row-group size")
        .unwrap()
        .set_max_row_group_size(17);
    assert_eq!(
        options
            .settings::<ParquetOptions>()
            .map(|parquet| parquet.max_row_group_size),
        Some(17)
    );
    options
        .require_settings_mut::<ParquetOptions>("$.key_value_metadata", "footer metadata")
        .unwrap()
        .set_key_value_metadata(vec![("source".into(), "test".into())]);
    options
        .require_settings_mut::<ParquetOptions>("$.key_value_metadata", "footer metadata")
        .unwrap()
        .push_key_value("version", "1");
    assert_eq!(
        options
            .settings::<ParquetOptions>()
            .map(|parquet| parquet.key_value_metadata.as_slice())
            .unwrap(),
        [
            ("source".into(), "test".into()),
            ("version".into(), "1".into())
        ]
    );
}

#[cfg(feature = "parquet")]
#[test]
fn parquet_only_setters_reject_another_inferred_encoding() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert!(
        options
            .settings::<ParquetOptions>()
            .map(ParquetOptions::compression_name)
            .is_none()
    );
    assert!(
        options
            .settings::<ParquetOptions>()
            .map(|parquet| parquet.max_row_group_size)
            .is_none()
    );
    assert!(
        options
            .settings::<ParquetOptions>()
            .map(|parquet| parquet.key_value_metadata.as_slice())
            .is_none()
    );
    for error in [
        options
            .require_settings_mut::<ParquetOptions>("$.compression", "a page compression")
            .unwrap_err(),
        options
            .require_settings_mut::<ParquetOptions>("$.max_row_group_size", "a row-group size")
            .unwrap_err(),
        options
            .require_settings_mut::<ParquetOptions>("$.key_value_metadata", "footer metadata")
            .unwrap_err(),
    ] {
        let message = error.to_string();
        assert!(message.contains("Parquet"), "{message}");
        assert!(message.contains("arrow.stream"), "{message}");
    }
}

#[test]
fn the_default_cadences_are_the_batch_rows_and_the_session_bytes() {
    assert_eq!(yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE, 65_536);
    // A write session with no stated cadence publishes by these bytes,
    // because it exists to publish between awaits.
    assert_eq!(yggdryl::media::DEFAULT_COMMIT_BYTE_SIZE, 64 * 1024 * 1024);
    // A one-shot write with no stated cadence publishes once at the end.
    assert_eq!(IpcOptions::new().commit_batch_num(), None);
}

#[test]
fn every_concrete_options_type_carries_the_same_thread_count() {
    fn assert_threads(mut options: impl IORecordOptions) {
        assert_eq!(options.num_threads(), None, "the destination's own answer");
        options.set_num_threads(Some(3));
        assert_eq!(options.num_threads(), Some(3));
        options.set_num_threads(None);
        assert_eq!(options.num_threads(), None);
    }

    assert_threads(IpcOptions::new());
    assert_threads(yggdryl::avro::AvroOptions::new());
    assert_threads(yggdryl::text::TextOptions::new());
    assert_threads(ExcelOptions::new());
    assert_threads(yggdryl::csv::CsvOptions::new());
    assert_threads(yggdryl::xmla::XmlaOptions::new());
    #[cfg(feature = "parquet")]
    assert_threads(yggdryl::parquet::ParquetOptions::new());

    let options = RecordOptions::Ipc(IpcOptions::new()).with_num_threads(2);
    assert_eq!(options.num_threads(), Some(2));
    // The count is a fact of the options: two options differing in it differ.
    assert_ne!(options, RecordOptions::Ipc(IpcOptions::new()));
    let RecordOptions::Ipc(inner) = options else {
        unreachable!()
    };
    assert_eq!(inner.num_threads, Some(2));
}

#[test]
fn zero_num_threads_is_a_typed_preflight_error() {
    let options = RecordOptions::Ipc(IpcOptions::new()).with_num_threads(0);
    let message = options.require_num_threads().unwrap_err().to_string();
    assert!(message.contains("$.num_threads"), "{message}");
    assert!(message.contains("non-zero thread count"), "{message}");
    assert!(message.contains("got 0"), "{message}");
    assert_eq!(
        RecordOptions::Ipc(IpcOptions::new())
            .with_num_threads(4)
            .require_num_threads()
            .unwrap(),
        Some(4)
    );
}

#[test]
fn zero_commit_batch_num_is_a_typed_preflight_error() {
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_batch_num(0);
    let message = options.require_commit_batch_num().unwrap_err().to_string();
    assert!(message.contains("$.commit_batch_num"), "{message}");
    assert!(message.contains("non-zero batch count"), "{message}");
    assert!(message.contains("got 0"), "{message}");
}

#[test]
fn a_reader_without_limits_is_returned_as_it_stands() {
    // The default options carry no bounds, so the seam costs nothing when
    // unused: six rows flow untouched, exactly as the source yields them.
    let options = IpcOptions::new();
    assert_eq!(options.max_row_size(), None);
    assert_eq!(options.max_byte_size(), None);
    assert_eq!(rows(options.limit_arrow_reader(reader(3, 2)).unwrap()), 6);
}

/// The rows and the batches one publication reader carries.
#[cfg(feature = "internals")]
fn rows_and_batches(reader: BatchReader) -> (usize, usize) {
    reader.fold((0, 0), |(rows, batches), batch| {
        (rows + batch.unwrap().num_rows(), batches + 1)
    })
}

/// `reader(count, per_batch)` with a zero-row batch before, between and
/// after its batches.
#[cfg(feature = "internals")]
fn reader_with_empty_batches(count: i64, per_batch: i64) -> BatchReader {
    let schema = schema().into_arrow_schema().unwrap();
    let empty = RecordBatch::new_empty(Arc::clone(&schema));
    let mut batches = vec![empty.clone()];
    for index in 0..count {
        batches.push(batch(index * per_batch..(index + 1) * per_batch));
        batches.push(empty.clone());
    }
    yggdryl::arrow::batch_reader(schema, batches)
}

#[cfg(feature = "internals")]
#[test]
fn commit_readers_cut_whole_batches_at_the_cadence() {
    // A cadence counts batches and never cuts one: two batches a commit
    // over batches of two, four and one rows is six rows, then one.
    let schema = schema().into_arrow_schema().unwrap();
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_batch_num(2);
    let commits = |source: BatchReader| {
        yggdryl::internals::media_options::commit_arrow_readers(&options, source)
            .unwrap()
            .map(|commit| rows_and_batches(commit.unwrap()))
            .collect::<Vec<_>>()
    };
    let source =
        yggdryl::arrow::batch_reader(Arc::clone(&schema), [batch(0..2), batch(2..6), batch(6..7)]);
    assert_eq!(commits(source), [(6, 2), (1, 1)]);

    // An empty batch counts for nothing and is never published: the same
    // rows with zero-row batches between them commit exactly the same.
    let empty = RecordBatch::new_empty(Arc::clone(&schema));
    let source = yggdryl::arrow::batch_reader(
        Arc::clone(&schema),
        [
            batch(0..2),
            empty.clone(),
            batch(2..6),
            empty.clone(),
            batch(6..7),
            empty,
        ],
    );
    assert_eq!(commits(source), [(6, 2), (1, 1)]);
}

#[cfg(feature = "internals")]
#[test]
fn a_byte_cadence_closes_when_the_held_batches_reach_the_target() {
    // Two int64 rows a batch: sixteen bytes each, as `memory_size` counts.
    let commits = |options: &RecordOptions, source: BatchReader, target: u64| {
        yggdryl::internals::media_options::commit_arrow_readers_by_bytes(options, source, target)
            .unwrap()
            .map(|commit| rows_and_batches(commit.unwrap()))
            .collect::<Vec<_>>()
    };
    let unstated = RecordOptions::Ipc(IpcOptions::new());
    // An empty batch counts for nothing, its zero bytes included, and is
    // never published, so a stream with zero-row batches between its
    // batches commits exactly as the same stream without them.
    for source in [reader, reader_with_empty_batches] {
        // A target under one batch: every batch is a cadence, none is cut.
        assert_eq!(
            commits(&unstated, source(4, 2), 1),
            [(2, 1), (2, 1), (2, 1), (2, 1)]
        );
        // Reached inside the second batch: two batches a cadence.
        assert_eq!(commits(&unstated, source(4, 2), 24), [(4, 2), (4, 2)]);
        assert_eq!(commits(&unstated, source(4, 2), 32), [(4, 2), (4, 2)]);
        // Never reached: one remainder.
        assert_eq!(commits(&unstated, source(4, 2), 1_000), [(8, 4)]);
        // A stated batch count wins over the destination's byte default.
        let stated = RecordOptions::Ipc(IpcOptions::new()).with_commit_batch_num(3);
        assert_eq!(commits(&stated, source(4, 2), 1), [(6, 3), (2, 1)]);
    }
}

#[cfg(feature = "internals")]
#[test]
fn a_commit_larger_than_the_stream_yields_one_final_remainder() {
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_batch_num(20);
    let commits = yggdryl::internals::media_options::commit_arrow_readers(&options, reader(3, 2))
        .unwrap()
        .map(|commit| rows(commit.unwrap()))
        .collect::<Vec<_>>();

    assert_eq!(commits, [6]);
}

#[cfg(feature = "internals")]
#[test]
fn a_full_commit_does_not_read_ahead() {
    let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Box::new(Counting {
        inner: reader(2, 4),
        pulls: Arc::clone(&pulls),
    });
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_batch_num(1);
    let mut commits =
        yggdryl::internals::media_options::commit_arrow_readers(&options, counted).unwrap();

    // One pull fills a cadence of one batch; the next batch is not pulled
    // until the caller asks for the next cadence.
    assert_eq!(rows(commits.next().unwrap().unwrap()), 4);
    assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(rows(commits.next().unwrap().unwrap()), 4);
    assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 2);
}

// A table hands each file its share of the threads, whatever encoding the
// file is: Parquet splits its row groups and columns over it and Avro its
// blocks, while Arrow IPC and text decode on one thread already.
#[cfg(all(feature = "internals", feature = "parquet"))]
#[test]
fn a_file_takes_its_thread_share_in_every_encoding_that_splits() {
    use yggdryl::internals::media_options::{file_threads, set_file_threads};

    for mut options in [
        RecordOptions::from(AvroOptions::new()),
        RecordOptions::from(ParquetOptions::new()),
    ] {
        assert_eq!(file_threads(&options), None);
        set_file_threads(&mut options, 3);
        assert_eq!(file_threads(&options), Some(3));
        // No share is zero threads.
        set_file_threads(&mut options, 0);
        assert_eq!(file_threads(&options), Some(1));
    }
    let mut ipc = RecordOptions::Ipc(IpcOptions::new());
    set_file_threads(&mut ipc, 3);
    assert_eq!(file_threads(&ipc), None);
    // A workbook reads its sheet on one thread.
    let mut excel = RecordOptions::from(ExcelOptions::new());
    set_file_threads(&mut excel, 3);
    assert_eq!(file_threads(&excel), None);
}

/// The stored `value` column of a write, and a batch of text to complete onto
/// it.
fn stored_value(nullable: bool) -> Field {
    DataType::from(
        StructType::from_fields([Field::new("value", DataType::Int64, nullable)]).unwrap(),
    )
    .required_field("row")
}

fn text_values(values: &[Option<&str>]) -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "value",
            arrow_schema::DataType::Utf8,
            true,
        )])),
        vec![Arc::new(arrow_array::StringArray::from(values.to_vec()))],
    )
    .unwrap()
}

#[test]
fn the_declared_and_the_stored_layer_cast_by_one_declared_column_rule() {
    let options = RecordOptions::Ipc(IpcOptions::default());
    // A caller who says nothing lets a nullable column take a value it cannot
    // convert as null; a not-null column refuses it by that value, and a
    // null by the column's path, rather than writing its canonical default.
    assert!(options.safe());
    let nulled = options
        .apply_arrow_batch(
            text_values(&[Some("1"), Some("x")]),
            Some(&stored_value(true)),
        )
        .unwrap();
    assert!(nulled.column(0).is_null(1));
    let refused = options
        .apply_arrow_batch(
            text_values(&[Some("1"), Some("x")]),
            Some(&stored_value(false)),
        )
        .unwrap_err()
        .to_string();
    assert!(refused.contains("'x'"), "{refused}");
    for values in [[Some("1"), None], [Some("1"), Some("")]] {
        let refused = options
            .apply_arrow_batch(text_values(&values), Some(&stored_value(false)))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("$.value"), "{refused}");
    }

    // `safe = false` refuses the nullable column's value too, and a declared
    // field answers exactly as the stored one it completes onto.
    let strict = options.clone().with_safe(false);
    let refused = strict
        .apply_arrow_batch(text_values(&[Some("x")]), Some(&stored_value(true)))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("'x'"), "{refused}");
    let declared = options.clone().with_field(stored_value(false));
    let refused = declared
        .apply_arrow_batch(text_values(&[Some("1"), None]), None)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("$.value"), "{refused}");
}

#[test]
fn a_write_onto_a_stored_not_null_column_refuses_and_leaves_the_resource() {
    let mut handle = Buffer::new().with_media_type(yggdryl::MimeType::ARROW_STREAM.into());
    let options = handle.record_options().unwrap();
    let stored = RecordBatch::try_new(
        stored_value(false).into_arrow_schema().unwrap(),
        vec![Arc::new(Int64Array::from(vec![7]))],
    )
    .unwrap();
    handle
        .overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(stored.schema(), [stored]),
            &options,
        )
        .unwrap();

    for values in [[Some("x")], [None]] {
        let incoming = text_values(&values);
        let refused = handle
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming.clone()]),
                &options,
            )
            .unwrap_err()
            .to_string();
        assert!(refused.contains("value"), "{refused}");
        let refused = handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                &options,
            )
            .unwrap_err()
            .to_string();
        assert!(refused.contains("value"), "{refused}");
    }
    let kept: Vec<i64> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .flat_map(|batch| {
            let batch = batch.unwrap();
            let values = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            values.values().to_vec()
        })
        .collect();
    assert_eq!(kept, [7]);
}

#[test]
fn a_batch_and_a_stream_split_source_predicates_from_selected_aliases() {
    let source = RecordBatch::try_from_iter([
        (
            "id",
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![1, 2, 3]))
                as arrow_array::ArrayRef,
        ),
        (
            "value",
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![2, 4, 6]))
                as arrow_array::ArrayRef,
        ),
    ])
    .unwrap();
    let options = RecordOptions::Ipc(IpcOptions::new())
        .with_select("id as key, value")
        .unwrap()
        .with_filter("key > 1 and id = 2")
        .unwrap();
    let shaped = options.apply_arrow_batch(source.clone(), None).unwrap();
    assert_eq!(shaped.num_rows(), 1);
    let stream = options
        .apply_arrow_reader(
            yggdryl::arrow::batch_reader(source.schema(), [source]),
            None,
        )
        .unwrap();
    assert_eq!(stream.map(Result::unwrap).collect::<Vec<_>>(), [shaped]);
}

/// The values `RecordOptions` hashes and orders by, pinned before the media
/// extension point rebuilds the enum.
///
/// A rebuilt `RecordOptions` is byte-identical when every default medium, every
/// shared section and every medium-own setting still hashes to the number
/// below, and the variants still sort in the order they were declared - so a
/// moved number here is a changed identity, never a number to re-pin.
mod s2_pins {
    use yggdryl::avro::AvroOptions;
    use yggdryl::excel::ExcelOptions;
    use yggdryl::ipc::IpcOptions;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    #[cfg(feature = "parquet")]
    use yggdryl::parquet::ParquetOptions;
    use yggdryl::xmla::XmlaOptions;
    use yggdryl::{MimeType, Timezone};

    use super::schema;

    /// Every medium this build reads, with the MIME type
    /// `RecordOptions::for_mime_type` builds its default options from: `ipc`
    /// `ARROW_STREAM`, `parquet` `PARQUET` (feature `parquet`), `avro` `AVRO`,
    /// `text` `PLAIN_TEXT`, `xmla` `XMLA`, `csv` `CSV`, `tsv` `TSV` (the CSV
    /// variant with a tab) and `excel` `XLSX`.
    fn media() -> Vec<(&'static str, MimeType)> {
        let mut media = vec![("ipc", MimeType::ARROW_STREAM)];
        #[cfg(feature = "parquet")]
        media.push(("parquet", MimeType::PARQUET));
        media.extend([
            ("avro", MimeType::AVRO),
            ("text", MimeType::PLAIN_TEXT),
            ("xmla", MimeType::XMLA),
            ("csv", MimeType::CSV),
            ("tsv", MimeType::TSV),
            ("excel", MimeType::XLSX),
        ]);
        media
    }

    /// The default options of the medium a MIME type names.
    fn default_of(mime_type: &MimeType) -> RecordOptions {
        RecordOptions::for_mime_type(mime_type).unwrap()
    }

    /// Each labelled options' hash is the pin of its label.
    ///
    /// A pin the build has no medium for (`parquet` without its feature) is
    /// left unread, and a label without a pin fails, so one table serves both
    /// feature lanes and a non-parquet medium cannot hash differently in one.
    fn assert_pinned(actual: &[(&'static str, RecordOptions)], pinned: &[(&str, u64)]) {
        let actual: Vec<(&str, u64)> = actual
            .iter()
            .map(|(label, options)| (*label, options.stable_hash()))
            .collect();
        let expected: Vec<(&str, u64)> = actual
            .iter()
            .map(|(label, _)| {
                let pin = pinned
                    .iter()
                    .find(|(pinned, _)| pinned == label)
                    .unwrap_or_else(|| panic!("no pin for {label}"));
                (*label, pin.1)
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn every_medium_default_options_hash_to_their_pinned_value() {
        let options: Vec<_> = media()
            .into_iter()
            .map(|(label, mime_type)| (label, default_of(&mime_type)))
            .collect();
        assert_pinned(
            &options,
            &[
                ("ipc", 12_422_255_642_484_437_054),
                ("parquet", 8_840_416_273_347_448_133),
                ("avro", 6_316_100_862_033_290_799),
                ("text", 9_086_959_791_203_163_073),
                ("xmla", 8_486_799_845_904_195_949),
                ("csv", 12_641_179_747_585_823_142),
                ("tsv", 13_049_347_064_351_962_713),
                ("excel", 9_017_146_332_497_625_255),
            ],
        );
    }

    #[test]
    fn shared_sections_feed_the_hash_the_same_on_every_medium() {
        let options: Vec<_> = media()
            .into_iter()
            .map(|(label, mime_type)| {
                let default = default_of(&mime_type);
                let mut options = default.clone();
                options.set_field(schema());
                options.set_name("pinned".into());
                options.set_max_row_size(Some(5));
                let options = options.with_filter("id > 1").unwrap();
                assert_eq!(options.name(), "pinned", "{label}");
                assert_ne!(options.stable_hash(), default.stable_hash(), "{label}");
                (label, options)
            })
            .collect();
        assert_pinned(
            &options,
            &[
                ("ipc", 3_699_406_654_356_496_144),
                ("parquet", 9_870_246_672_429_389_253),
                ("avro", 16_103_427_577_062_353_326),
                ("text", 7_498_870_975_766_000_812),
                ("xmla", 11_124_752_632_585_582_100),
                ("csv", 637_864_656_599_364_788),
                ("tsv", 2_756_740_722_792_913_074),
                ("excel", 15_607_496_437_425_493_778),
            ],
        );
    }

    #[test]
    fn the_media_order_by_their_rank() {
        // Fed in reverse, so a sort that moved nothing would show.
        let mut options: Vec<RecordOptions> = media()
            .iter()
            .rev()
            .map(|(_, mime_type)| default_of(mime_type))
            .collect();
        options.sort();
        let order: Vec<String> = options
            .iter()
            .map(|options| options.mime_type().to_string())
            .collect();
        // `tsv` before `csv`: both are the CSV variant, ordered by their options.
        let mut pinned = vec!["application/vnd.apache.arrow.stream"];
        if cfg!(feature = "parquet") {
            pinned.push("application/vnd.apache.parquet");
        }
        pinned.extend([
            "application/avro",
            "text/plain",
            "application/xmla+xml",
            "text/tab-separated-values",
            "text/csv",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ]);
        assert_eq!(order, pinned);

        // The medium decides before any setting does: the most-set options of
        // an earlier medium still sort before the defaults of a later one.
        let mut ipc = default_of(&MimeType::ARROW_STREAM);
        ipc.set_max_row_size(Some(u64::MAX));
        assert!(ipc < default_of(&MimeType::AVRO));
        let mut csv = default_of(&MimeType::CSV);
        csv.set_max_row_size(Some(u64::MAX));
        assert!(csv < default_of(&MimeType::XLSX));
        assert!(default_of(&MimeType::XLSX) > default_of(&MimeType::TSV));
    }

    #[test]
    fn the_medium_settings_feed_the_hash() {
        // Each entry is the medium's defaults with exactly one medium-own
        // setting stated through the typed settings door, or through the
        // medium's own options value where the setting is the options'
        // (`ipc` has none; `xmla`'s envelope is its options').
        let mut stated: Vec<(&'static str, RecordOptions, RecordOptions)> = Vec::new();
        let mut state = |label, mime_type: &MimeType, set: &dyn Fn(&mut RecordOptions)| {
            let default = default_of(mime_type);
            let mut options = default.clone();
            set(&mut options);
            assert_ne!(options, default, "{label}");
            stated.push((label, default, options));
        };

        state("avro/codec=null", &MimeType::AVRO, &|options| {
            options
                .require_settings_mut::<AvroOptions>("$.block_codec", "a block codec")
                .unwrap()
                .set_block_codec("null")
                .unwrap();
        });
        state("avro/sync_marker", &MimeType::AVRO, &|options| {
            options
                .require_settings_mut::<AvroOptions>("$.sync_marker", "a synchronization marker")
                .unwrap()
                .set_sync_marker(Some(b"0123456789abcdef"))
                .unwrap();
        });
        #[cfg(feature = "parquet")]
        {
            state(
                "parquet/compression=gzip(4)",
                &MimeType::PARQUET,
                &|options| {
                    options
                        .require_settings_mut::<ParquetOptions>(
                            "$.compression",
                            "a page compression",
                        )
                        .unwrap()
                        .set_compression_name("gzip(4)")
                        .unwrap();
                },
            );
            state(
                "parquet/max_row_group_size=17",
                &MimeType::PARQUET,
                &|options| {
                    options
                        .require_settings_mut::<ParquetOptions>(
                            "$.max_row_group_size",
                            "a row-group size",
                        )
                        .unwrap()
                        .set_max_row_group_size(17);
                },
            );
            state("parquet/key_value", &MimeType::PARQUET, &|options| {
                options
                    .require_settings_mut::<ParquetOptions>(
                        "$.key_value_metadata",
                        "footer metadata",
                    )
                    .unwrap()
                    .push_key_value("source", "test");
            });
        }
        state("text/timezone=UTC", &MimeType::PLAIN_TEXT, &|options| {
            options.set_timezone(Some(Timezone::UTC)).unwrap();
        });
        state("xmla/without_envelope", &MimeType::XMLA, &|options| {
            *options = RecordOptions::from(XmlaOptions::new().without_envelope());
        });
        state("csv/separator=;", &MimeType::CSV, &|options| {
            options.set_csv_separator(b';').unwrap();
        });
        state("csv/header=false", &MimeType::CSV, &|options| {
            options.set_header(false).unwrap();
        });
        state("excel/sheet=Trades", &MimeType::XLSX, &|options| {
            options
                .require_settings_mut::<ExcelOptions>("$.sheet", "a worksheet")
                .unwrap()
                .set_sheet(Some("Trades"))
                .unwrap();
        });
        state("excel/header=false", &MimeType::XLSX, &|options| {
            options.set_header(false).unwrap();
        });
        // The enum's own constructors reach the same value as the typed door.
        assert_eq!(
            RecordOptions::from(ExcelOptions::new().with_sheet("Trades")),
            stated
                .iter()
                .find(|(label, ..)| *label == "excel/sheet=Trades")
                .unwrap()
                .2
        );
        assert_eq!(
            RecordOptions::Ipc(IpcOptions::new()),
            default_of(&MimeType::ARROW_STREAM)
        );

        for (label, default, options) in &stated {
            assert_ne!(options.stable_hash(), default.stable_hash(), "{label}");
        }
        let options: Vec<_> = stated
            .into_iter()
            .map(|(label, _, options)| (label, options))
            .collect();
        assert_pinned(
            &options,
            &[
                ("avro/codec=null", 2_341_579_485_644_533_238),
                ("avro/sync_marker", 10_665_391_877_395_609_766),
                ("parquet/compression=gzip(4)", 7_904_555_134_641_360_236),
                ("parquet/max_row_group_size=17", 18_196_550_021_450_903_816),
                ("parquet/key_value", 6_469_640_053_124_428_298),
                ("text/timezone=UTC", 16_253_849_957_344_799_104),
                ("xmla/without_envelope", 2_087_720_147_871_917_823),
                ("csv/separator=;", 3_263_054_625_663_635_824),
                ("csv/header=false", 5_228_038_379_540_985_085),
                ("excel/sheet=Trades", 15_880_239_124_503_455_895),
                ("excel/header=false", 4_068_267_976_377_555_643),
            ],
        );
    }
}
