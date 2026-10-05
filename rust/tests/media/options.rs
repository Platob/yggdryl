//! `rust/src/media/options.rs`: the row skip, the row and byte limits, and
//! the cadence.
//!
//! Everything a caller states and observes reaches the crate through
//! `yggdryl::`. The slicer a declared commit cadence pulls through is
//! crate-private, and that it cuts a stream without reading ahead is what
//! bounds a streamed write, so those three reach `yggdryl::internals`.

use std::sync::Arc;
use yggdryl::RecordHeader;

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
use arrow_schema::{ArrowError, SchemaRef};

use yggdryl::IOMedia;
use yggdryl::arrow::BatchReader;
use yggdryl::excel::ExcelOptions;
use yggdryl::holder::Buffer;
use yggdryl::ipc::IpcOptions;
use yggdryl::media::{IORecordOptions, RecordOptions};
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
    let equal = RecordOptions::Excel(ExcelOptions::new().with_sheet("Trades"));
    assert_traits(&excel);
    assert_eq!(options, equal);
    assert_eq!(options.stable_hash(), equal.stable_hash());
    for changed in [
        excel.clone().with_sheet("Quotes"),
        excel.clone().with_header(RecordHeader::None),
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
        Ok(RecordOptions::Excel(_))
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
    let RecordOptions::Excel(inner) = options else {
        panic!("an xlsx handle names the workbook encoding");
    };
    assert_eq!(inner.max_row_size, Some(7));
    assert_eq!(inner.max_byte_size, Some(1024));
    // Everything else is the workbook's default: the first sheet, a header
    // row, the whole grid.
    assert_eq!(inner.sheet(), None);
    assert_eq!(inner.header, RecordHeader::Source);
    assert_eq!(inner.range(), None);
    assert_eq!(inner.cells().unwrap(), yggdryl::excel::CellRange::all());
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

    assert_eq!(options.avro_block_codec(), Some("deflate"));
    assert_eq!(options.avro_sync_marker(), None);

    options.set_avro_block_codec("null").unwrap();
    assert_eq!(options.avro_block_codec(), Some("null"));
    let marker = *b"0123456789abcdef";
    options.set_avro_sync_marker(Some(&marker)).unwrap();
    assert_eq!(options.avro_sync_marker(), Some(&marker));

    let mut handle = Buffer::new().with_media_type(media_type);
    handle.overwrite_arrow_batch(batch(0..2), &options).unwrap();
    assert!(handle.as_slice().ends_with(&marker));
    assert_eq!(rows(handle.read_arrow_reader(&options).unwrap()), 2);

    options.set_avro_sync_marker(None).unwrap();
    assert_eq!(options.avro_sync_marker(), None);

    let codec = options.set_avro_block_codec("brotli").unwrap_err();
    assert!(matches!(
        codec,
        yggdryl::Error::Codec { format: "avro", .. }
    ));
    assert!(codec.to_string().contains("brotli"));

    let length = options.set_avro_sync_marker(Some(b"short")).unwrap_err();
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

    assert_eq!(options.avro_block_codec(), None);
    assert_eq!(options.avro_sync_marker(), None);
    for error in [
        options.set_avro_block_codec("null").unwrap_err(),
        options.set_avro_sync_marker(None).unwrap_err(),
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

    assert_eq!(options.excel_sheet(), None);
    assert_eq!(options.header(), Some(RecordHeader::Source));
    assert_eq!(options.excel_range(), None);

    options.set_excel_sheet(Some("Trades")).unwrap();
    options.set_header(RecordHeader::None).unwrap();
    let range: yggdryl::excel::CellRange = "B2:D9".parse().unwrap();
    options.set_excel_range(Some(range)).unwrap();
    assert_eq!(options.excel_sheet(), Some("Trades"));
    assert_eq!(options.header(), Some(RecordHeader::None));
    assert_eq!(options.excel_range(), Some(range));
    let RecordOptions::Excel(inner) = &options else {
        panic!("an xlsx handle names the workbook encoding");
    };
    assert_eq!(inner.sheet(), Some("Trades"));
    assert_eq!(inner.header, RecordHeader::None);
    assert_eq!(inner.cells().unwrap(), range);

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
        let error = options.set_excel_sheet(Some(name)).unwrap_err();
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
        assert_eq!(options.excel_sheet(), Some("Trades"), "{name:?}");
    }

    // `None` clears back to the defaults: the first sheet, the whole grid.
    options.set_excel_sheet(None).unwrap();
    options.set_excel_range(None).unwrap();
    assert_eq!(options.excel_sheet(), None);
    assert_eq!(options.excel_range(), None);
}

#[test]
fn record_setters_reject_another_inferred_encoding() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert_eq!(options.excel_sheet(), None);
    assert_eq!(options.header(), None);
    assert_eq!(options.excel_range(), None);
    for (error, path, encoding, setting) in [
        (
            options.set_excel_sheet(Some("Trades")).unwrap_err(),
            "$.sheet",
            "Excel",
            "a worksheet",
        ),
        (
            options.set_header(RecordHeader::None).unwrap_err(),
            "$.header",
            "CSV or Excel",
            "a header",
        ),
        (
            options
                .set_excel_range(Some("A1:B2".parse().unwrap()))
                .unwrap_err(),
            "$.range",
            "Excel",
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
                "invalid record value at {path}: expected {encoding} options to set {setting}, \
                 got application/vnd.apache.arrow.stream options"
            )
        );
    }
    assert_eq!(options, RecordOptions::for_media_type(&media_type).unwrap());

    // The other encodings' setters refuse workbook options the same way.
    let workbook = Url::from_str("file:///t.xlsx").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&workbook).unwrap();
    assert_eq!(options.avro_block_codec(), None);
    assert_eq!(options.timezone(), None);
    let message = options
        .set_avro_block_codec("null")
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
        options.parquet_compression_name().as_deref(),
        Some("zstd(1)")
    );
    options.set_parquet_compression_name("gzip(4)").unwrap();
    assert_eq!(
        options.parquet_compression_name().as_deref(),
        Some("gzip(4)")
    );

    options.set_parquet_max_row_group_size(17).unwrap();
    assert_eq!(options.parquet_max_row_group_size(), Some(17));
    options
        .set_parquet_key_value_metadata(vec![("source".into(), "test".into())])
        .unwrap();
    options.push_parquet_key_value("version", "1").unwrap();
    assert_eq!(
        options.parquet_key_value_metadata().unwrap(),
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

    assert!(options.parquet_compression_name().is_none());
    assert!(options.parquet_max_row_group_size().is_none());
    assert!(options.parquet_key_value_metadata().is_none());
    for error in [
        options.set_parquet_compression_name("snappy").unwrap_err(),
        options.set_parquet_max_row_group_size(17).unwrap_err(),
        options
            .set_parquet_key_value_metadata(Vec::new())
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
    use yggdryl::avro::AvroOptions;
    use yggdryl::internals::media_options::{file_threads, set_file_threads};
    use yggdryl::parquet::ParquetOptions;

    for mut options in [
        RecordOptions::Avro(AvroOptions::new()),
        RecordOptions::Parquet(ParquetOptions::new()),
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
    let mut excel = RecordOptions::Excel(ExcelOptions::new());
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
fn result_field_preserves_identity_metadata_and_binds_source_or_result_clauses() {
    let mut child = DataType::Int64.required_field("id");
    child.set_comment("source child metadata").unwrap();
    let mut source = StructType::from_fields([child])
        .map(DataType::from)
        .unwrap()
        .required_field("records");
    source.set_comment("source root metadata").unwrap();
    let options = IpcOptions::new();
    assert_eq!(options.result_field(source.clone()).unwrap(), source);

    let selected = options.with_select("id as key").unwrap();
    for filter in ["id > 0", "key > 0"] {
        let options = selected.clone().with_filter(filter).unwrap();
        let output = options.result_field(source.clone()).unwrap();
        assert_eq!(output.fields().len(), 1);
        assert_eq!(output.fields()[0].name(), "key");
        assert_eq!(output.fields()[0].dtype(), &DataType::Int64);
        // The field resolver and the real expression reader use the same
        // binding order without needing any source rows.
        let reader = yggdryl::arrow::batch_reader(source.clone().into_arrow_schema().unwrap(), []);
        let reader = options.apply_arrow_expressions(reader).unwrap();
        assert_eq!(output.into_arrow_schema().unwrap(), reader.schema());
    }
    for options in [
        IpcOptions::new().with_select("missing").unwrap(),
        IpcOptions::new().with_filter("missing > 0").unwrap(),
        selected.with_filter("missing > 0").unwrap(),
    ] {
        let error = options.result_field(source.clone()).unwrap_err();
        match error {
            yggdryl::Error::InvalidRecord { path, reason } => {
                assert!(!path.is_empty());
                assert!(reason.contains("missing"), "{reason}");
            }
            error => panic!("expected a located schema binding error, got {error}"),
        }
    }
}

#[test]
fn selection_scalar_record_options_exposes_typed_selection_only_for_excel() {
    use yggdryl::excel::ExcelSelection;
    let mut options = RecordOptions::from(ExcelOptions::new());
    let selected = ExcelSelection::from_scalar(
        &yggdryl::from_json_scalar(r#"{"sheet":"Data","range":"A1:B2"}"#).unwrap(),
    )
    .unwrap();
    options.set_excel_selection(selected.clone()).unwrap();
    assert_eq!(options.excel_selection(), Some(&selected));
    assert_eq!(options.excel_sheet(), Some("Data"));
    assert_eq!(options.excel_range(), Some("A1:B2".parse().unwrap()));

    let mut other = RecordOptions::Ipc(IpcOptions::new());
    let before = other.clone();
    assert_eq!(other.excel_selection(), None);
    let message = other.set_excel_selection(selected).unwrap_err().to_string();
    assert!(message.contains("$.selection"), "{message}");
    assert_eq!(other, before);
}

#[test]
fn selection_scalar_typed_null_setters_preserve_inactive_arm() {
    use yggdryl::excel::ExcelSelection;
    let mut table = RecordOptions::from(ExcelOptions::new().with_table("Orders"));
    let original = table.clone();
    table.set_excel_sheet(None).unwrap();
    table.set_excel_range(None).unwrap();
    assert_eq!(table, original);
    let mut worksheet = RecordOptions::from(
        ExcelOptions::new()
            .with_sheet("Data")
            .with_range("B2:C4".parse().unwrap()),
    );
    let original = worksheet.clone();
    worksheet.set_excel_table(None).unwrap();
    assert_eq!(worksheet, original);
    assert_eq!(
        worksheet.excel_selection(),
        Some(&ExcelSelection::Worksheet {
            sheet: Some("Data".into()),
            range: Some("B2:C4".parse().unwrap()),
        })
    );
}

#[test]
fn csv_rows_one_is_source_for_quoted_multiline_write_and_read() {
    use arrow_array::StringArray;
    use yggdryl::csv::CsvOptions;

    let field = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("note"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let mut source_bytes = None;
    for header in [RecordHeader::Source, RecordHeader::Rows(1)] {
        let mut options = RecordOptions::Csv(CsvOptions::new());
        options.set_header(header).unwrap();
        assert_eq!(options.header(), Some(RecordHeader::Source));

        let batch = RecordBatch::try_new(
            field.clone().into_arrow_schema().unwrap(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec!["alpha\n\"quoted\", beta", "plain"])),
            ],
        )
        .unwrap();
        let mut held = Buffer::new()
            .with_media_type(Url::from_str("file:///quoted.csv").unwrap().media_type());
        held.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .unwrap();
        let bytes = held.as_slice().to_vec();
        assert!(
            std::str::from_utf8(&bytes)
                .unwrap()
                .contains("\"alpha\n\"\"quoted\"\", beta\""),
            "{bytes:?}"
        );
        if let Some(source) = &source_bytes {
            assert_eq!(&bytes, source);
        } else {
            source_bytes = Some(bytes);
        }

        let rows = held
            .read_arrow_reader(&options)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(rows.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
        let notes = rows
            .iter()
            .flat_map(|batch| {
                let column = batch
                    .column_by_name("note")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| column.value(row).to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(notes, ["alpha\n\"quoted\", beta", "plain"]);
    }
}

#[test]
fn generic_header_policy_refusals_are_located_and_atomic() {
    use yggdryl::csv::CsvOptions;

    let mut csv = RecordOptions::Csv(CsvOptions::new());
    let original = csv.clone();
    for header in [
        RecordHeader::Rows(0),
        RecordHeader::Rows(2),
        RecordHeader::Infer,
    ] {
        let error = csv.set_header(header).unwrap_err();
        let yggdryl::Error::InvalidRecord { path, reason } = error else {
            panic!("expected located CSV refusal for {header:?}, got {error}");
        };
        assert_eq!(path.as_str(), "$.header");
        assert!(
            reason.contains("CSV") && reason.contains("header"),
            "{reason}"
        );
        assert_eq!(csv, original);
    }

    let mut excel = RecordOptions::Excel(ExcelOptions::new());
    excel.set_header(RecordHeader::Rows(2)).unwrap();
    assert_eq!(excel.header(), Some(RecordHeader::Rows(2)));
    excel.set_header(RecordHeader::Infer).unwrap();
    assert_eq!(excel.header(), Some(RecordHeader::Infer));
    let before_grid = excel.clone();
    let error = excel.set_header(RecordHeader::Rows(1_048_577)).unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected a located grid refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.header");
    assert!(reason.contains("1048576"), "{reason}");
    assert_eq!(excel, before_grid);

    let mut named = RecordOptions::Excel(ExcelOptions::new().with_table("Quantities"));
    let before_table = named.clone();
    let error = named.set_header(RecordHeader::Rows(1)).unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected a located named-table refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.header");
    assert!(reason.contains("tableColumn"), "{reason}");
    assert_eq!(named, before_table);
}

#[test]
fn excel_selection_setter_table_refuses_rows_header_atomically() {
    let mut by_table = RecordOptions::Excel(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let before = by_table.clone();
    let error = by_table.set_excel_table(Some("Quantities")).unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected located header refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.header");
    assert!(reason.contains("tableColumn"), "{reason}");
    assert_eq!(by_table, before);
}

#[test]
fn excel_selection_setter_typed_table_refuses_rows_header_atomically() {
    use yggdryl::excel::ExcelSelection;
    let mut by_selection =
        RecordOptions::Excel(ExcelOptions::new().with_header(RecordHeader::Rows(2)));
    let before = by_selection.clone();
    let error = by_selection
        .set_excel_selection(ExcelSelection::Table {
            name: "Quantities".into(),
        })
        .unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected located header refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.header");
    assert!(reason.contains("tableColumn"), "{reason}");
    assert_eq!(by_selection, before);
}

#[test]
fn excel_selection_setter_typed_empty_table_refuses_atomically() {
    use yggdryl::excel::ExcelSelection;
    let mut empty = RecordOptions::Excel(ExcelOptions::new());
    let before_empty = empty.clone();
    let error = empty
        .set_excel_selection(ExcelSelection::Table { name: "".into() })
        .unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected located empty-table refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.selection.table");
    assert!(reason.contains("nonempty"), "{reason}");
    assert_eq!(empty, before_empty);
}

#[test]
fn excel_selection_setter_typed_invalid_sheet_refuses_atomically() {
    use yggdryl::excel::ExcelSelection;
    let mut options = RecordOptions::Excel(ExcelOptions::new());
    let before = options.clone();
    let error = options
        .set_excel_selection(ExcelSelection::Worksheet {
            sheet: Some("Q1/Q2".into()),
            range: None,
        })
        .unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected located sheet-name refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.selection.sheet");
    assert!(reason.contains("Q1/Q2"), "{reason}");
    assert_eq!(options, before);
    options
        .set_excel_selection(ExcelSelection::Worksheet {
            sheet: Some("Data".into()),
            range: None,
        })
        .unwrap();
    assert_eq!(options.excel_sheet(), Some("Data"));
}

#[test]
fn excel_selection_setter_typed_invalid_range_refuses_atomically() {
    use yggdryl::excel::{CellRange, CellRef, ExcelSelection};
    let outside = CellRange::new(CellRef::new(1_048_576, 0), CellRef::new(1_048_576, 0));
    let mut options = RecordOptions::Excel(ExcelOptions::new());
    let before = options.clone();
    let error = options
        .set_excel_selection(ExcelSelection::Worksheet {
            sheet: None,
            range: Some(outside),
        })
        .unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected located range refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.selection.range");
    assert!(reason.contains("1048576"), "{reason}");
    assert_eq!(options, before);
}

#[test]
fn excel_selection_setter_range_refuses_out_of_grid_atomically() {
    use yggdryl::excel::{CellRange, CellRef};
    let outside = CellRange::new(CellRef::new(0, 16_384), CellRef::new(0, 16_384));
    let mut options = RecordOptions::Excel(ExcelOptions::new());
    let before = options.clone();
    let error = options.set_excel_range(Some(outside)).unwrap_err();
    let yggdryl::Error::InvalidRecord { path, reason } = error else {
        panic!("expected located range refusal, got {error}");
    };
    assert_eq!(path.as_str(), "$.range");
    assert!(reason.contains("16384"), "{reason}");
    assert_eq!(options, before);
    let edge = CellRange::new(
        CellRef::new(1_048_575, 16_383),
        CellRef::new(1_048_575, 16_383),
    );
    options.set_excel_range(Some(edge)).unwrap();
    assert_eq!(options.excel_range(), Some(edge));
}
