//! The auto-spill doors of `rust/src/serie/spill.rs`'s `settled`: every door
//! where the crate lays a column out itself settles it under the process
//! default, and every door that only shares a caller's buffers, or writes
//! where a column stands, does not.
//!
//! The process default is one `OnceLock` a process settles once, so this
//! target owns its own process: the first thing every test does is
//! [`installed`], which installs a bound of [`BOUND`] bytes before anything
//! resolves the default from the environment. No other harness installs one,
//! so no other target's columns spill unasked.

use std::sync::{Arc, OnceLock};

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, RecordBatch, RecordBatchIterator};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::arrow::BatchReader;
use yggdryl::{
    ArrowCastOptions, ArrowCastPlan, ChunkedSerie, DataType, Field, JoinKind, JoinOptions, Scalar,
    Serie, SortOptions, SpillOptions, StreamChunkedSerie, StructType,
};

/// The process default every test here settles under: a few hundred bytes,
/// which a thousand int64 rows pass eight thousand bytes over and three rows
/// stay under.
const BOUND: u64 = 256;

/// The rows a column past the bound holds.
const ROWS: usize = 1_000;

/// The rows a column under the bound holds.
const FEW: usize = 3;

/// Install the process default once, before any door resolves it.
fn installed() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        SpillOptions::install_env(SpillOptions::new().with_byte_size(BOUND))
            .expect("nothing in this process resolved the default before the first test");
    });
    let resolved = SpillOptions::from_env().expect("the installed default");
    assert_eq!(resolved.byte_size(), BOUND);
}

/// The required int64 field every flat column here is laid out under.
fn price() -> Field {
    DataType::Int64.required_field("x")
}

/// `rows` int64 values counting up from zero.
fn values(rows: usize) -> Vec<i64> {
    (0..rows as i64).collect()
}

/// The rows `values` reads as.
fn expected(rows: usize) -> Vec<Scalar> {
    values(rows).into_iter().map(Scalar::from).collect()
}

/// An Arrow int64 array of `rows` values counting up.
fn int64_array(rows: usize) -> ArrayRef {
    Arc::new(Int64Array::from(values(rows)))
}

/// A column of `rows` int64 values landed as it stands: the caller's
/// buffers shared, nothing settled.
fn heap(rows: usize) -> Serie {
    let column =
        Serie::from_arrow_array(Some(&price()), int64_array(rows), ArrowCastOptions::new())
            .expect("an exact column");
    assert!(
        !column.is_spilled(),
        "an exact landing shares the caller's buffers"
    );
    column
}

/// The record `quote{x: int64}` a batch of the one int64 column lands as.
fn quote_root() -> Field {
    DataType::from(StructType::from_fields([price()]).expect("one child")).required_field("quote")
}

/// A batch of one `x` column of `rows` values, as `int64` or `int32`.
fn batch(rows: usize, narrow: bool) -> RecordBatch {
    let (dtype, column): (ArrowDataType, ArrayRef) = if narrow {
        (
            ArrowDataType::Int32,
            Arc::new(Int32Array::from((0..rows as i32).collect::<Vec<_>>())),
        )
    } else {
        (ArrowDataType::Int64, int64_array(rows))
    };
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![ArrowField::new("x", dtype, false)])),
        vec![column],
    )
    .expect("a batch")
}

/// A stream of `batches` int64 batches of `rows` rows each.
fn stream(batches: usize, rows: usize) -> BatchReader {
    let schema = batch(rows, false).schema();
    Box::new(RecordBatchIterator::new(
        (0..batches).map(move |_| Ok(batch(rows, false))),
        schema,
    ))
}

/// `column`'s `x` rows: the column itself, or a record's one child.
fn xs(column: &Serie) -> Vec<Scalar> {
    match column.child("x") {
        Some(child) => child.rows().into_owned(),
        None => column.rows().into_owned(),
    }
}

/// `column` is spilled and reads `rows` as it did before the door.
fn assert_settled(door: &str, column: &Serie, rows: &[Scalar]) {
    assert!(column.is_spilled(), "{door}: settled past the bound");
    assert_eq!(column.resident_size(), 0, "{door}");
    assert_eq!(xs(column), rows, "{door}: the rows read back");
}

/// `column` holds its rows on the heap.
fn assert_resident(door: &str, column: &Serie) {
    assert!(!column.is_spilled(), "{door}: never settled");
    assert_eq!(column.resident_size(), column.memory_size(), "{door}");
}

#[test]
fn from_scalars_settles_what_it_lays_out() {
    installed();
    let column = Serie::from_scalars(price(), expected(ROWS)).expect("rows");
    assert_settled("from_scalars", &column, &expected(ROWS));
}

#[test]
fn from_default_is_a_constant_column_the_bound_never_reaches() {
    installed();
    let column = Serie::from_default(price(), ROWS).expect("defaults");
    assert!(
        column.as_lit().is_some(),
        "a default is one value, held once"
    );
    assert!(!column.is_spilled(), "nothing of it lies in a file");
    assert!(
        column.resident_size() < BOUND as usize,
        "{}",
        column.resident_size()
    );
    assert_eq!(xs(&column), vec![Scalar::from(0_i64); ROWS]);
    // Laid out for a typed reader, it still spills nothing: the layout is
    // forgotten under the bound rather than written.
    assert_eq!(
        column
            .as_int64()
            .expect("laid out on demand")
            .values()
            .len(),
        ROWS
    );
    assert!(!column.is_spilled());
}

#[test]
fn from_arrow_array_settles_a_layout_it_casts() {
    installed();
    let narrow: ArrayRef = Arc::new(Int32Array::from((0..ROWS as i32).collect::<Vec<_>>()));
    let column = Serie::from_arrow_array(Some(&price()), narrow, ArrowCastOptions::new())
        .expect("an int32 array cast to int64");
    assert_settled("from_arrow_array, cast", &column, &expected(ROWS));
}

#[test]
fn from_arrow_batch_settles_a_layout_it_casts() {
    installed();
    let column = Serie::from_arrow_batch(
        Some(&quote_root()),
        &batch(ROWS, true),
        ArrowCastOptions::new(),
    )
    .expect("an int32 batch cast to int64");
    assert_settled("from_arrow_batch, cast", &column, &expected(ROWS));
}

#[test]
fn a_cast_settles_what_its_plan_lays_out() {
    installed();
    let target = Field::new("x", DataType::Float64, false);
    let floats: Vec<Scalar> = values(ROWS)
        .into_iter()
        .map(|value| Scalar::from(value as f64))
        .collect();

    let cast = heap(ROWS)
        .cast(&target, ArrowCastOptions::new())
        .expect("a cast");
    assert_settled("Serie::cast", &cast, &floats);

    let plan = ArrowCastPlan::compile(&price(), &target, ArrowCastOptions::new()).expect("a plan");
    let applied = plan.apply(&heap(ROWS)).expect("applied");
    assert_settled("ArrowCastPlan::apply", &applied, &floats);
}

#[test]
fn the_ordering_reads_settle_the_column_they_take() {
    installed();
    let column = heap(ROWS);
    let reversed: Vec<Scalar> = expected(ROWS).into_iter().rev().collect();

    assert_settled(
        "into_sorted",
        &column
            .into_sorted(SortOptions::descending())
            .expect("sorted"),
        &reversed,
    );
    assert_settled("into_reversed", &column.into_reversed(), &reversed);
    let picks = Serie::new((0..ROWS as u32).rev().map(Scalar::from).collect::<Vec<_>>());
    assert_settled(
        "into_taken",
        &column.into_taken(&picks).expect("taken"),
        &reversed,
    );
    let keep_all = Serie::new(vec![Scalar::from(true); ROWS]);
    assert_settled(
        "into_filtered",
        &column.into_filtered(&keep_all).expect("filtered"),
        &expected(ROWS),
    );
    assert_settled(
        "into_unique",
        &column.into_unique().expect("unique"),
        &expected(ROWS),
    );
}

#[test]
fn the_ordering_writes_settle_the_column_in_place() {
    installed();
    let reversed: Vec<Scalar> = expected(ROWS).into_iter().rev().collect();

    let mut sorted = heap(ROWS);
    sorted
        .as_sorted(SortOptions::descending())
        .expect("sorted in place");
    assert_settled("as_sorted", &sorted, &reversed);

    let mut flipped = heap(ROWS);
    flipped.as_reversed().expect("reversed in place");
    assert_settled("as_reversed", &flipped, &reversed);

    // A column held alone sorts its buffers where they stand, and settles
    // them all the same.
    let mut alone = Serie::from_arrow_array(
        Some(&price()),
        Arc::new(Int64Array::from(
            values(ROWS).into_iter().rev().collect::<Vec<_>>(),
        )),
        ArrowCastOptions::new(),
    )
    .expect("an exact column");
    assert!(!alone.is_spilled());
    alone
        .as_sorted(SortOptions::default())
        .expect("sorted in place");
    assert_settled("as_sorted held alone", &alone, &expected(ROWS));
}

#[test]
fn extend_from_serie_settles_the_column_it_grew() {
    installed();
    let doubled: Vec<Scalar> = expected(ROWS).into_iter().chain(expected(ROWS)).collect();

    // One datatype: buffer to buffer.
    let mut grown = heap(ROWS);
    grown.extend_from_serie(&heap(ROWS)).expect("appended");
    assert_settled("extend_from_serie, buffers", &grown, &doubled);

    // A run's rows read through the field.
    let mut grown = heap(ROWS);
    grown
        .extend_from_serie(&Serie::new(expected(ROWS)))
        .expect("appended");
    assert_settled("extend_from_serie, rows", &grown, &doubled);
}

#[test]
fn a_chunked_join_settles_the_one_column_it_concatenates() {
    installed();
    let chunked = ChunkedSerie::from_series(
        Some(&price()),
        [heap(ROWS), heap(ROWS)],
        ArrowCastOptions::new(),
    )
    .expect("two chunks");
    assert!(
        chunked.chunks().iter().all(|chunk| !chunk.is_spilled()),
        "from_series settles nothing"
    );
    let joined = chunked.into_serie().expect("one column");
    let doubled: Vec<Scalar> = expected(ROWS).into_iter().chain(expected(ROWS)).collect();
    assert_settled("ChunkedSerie::into_serie", &joined, &doubled);
}

#[test]
fn push_chunk_settles_the_chunks_heaviest_first() {
    installed();
    let mut chunked =
        ChunkedSerie::from_series(Some(&price()), [heap(ROWS)], ArrowCastOptions::new())
            .expect("one chunk");
    assert!(!chunked.is_spilled());
    chunked
        .push_chunk(heap(FEW), ArrowCastOptions::new())
        .expect("a chunk of the field");
    // The heavy chunk spills; the light one stays under the bound.
    assert!(chunked.chunk(0).expect("the heavy chunk").is_spilled());
    assert!(!chunked.chunk(1).expect("the light chunk").is_spilled());
    assert!(u64::try_from(chunked.resident_size()).expect("a byte count") <= BOUND);
    assert_eq!(
        chunked.rows(),
        expected(ROWS)
            .into_iter()
            .chain(expected(FEW))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_drained_reader_settles_every_chunk_it_collects() {
    installed();
    let chunked = ChunkedSerie::from_chunked_stream(
        StreamChunkedSerie::from_arrow_reader(None, stream(2, ROWS), ArrowCastOptions::new())
            .expect("a stream"),
    )
    .expect("two chunks");
    assert_eq!(chunked.num_chunks(), 2);
    for chunk in chunked.chunks() {
        assert_settled("ChunkedSerie::from_chunked_stream", chunk, &expected(ROWS));
    }
    assert!(chunked.is_spilled());

    let chunked = ChunkedSerie::from_arrow_reader(None, stream(2, ROWS), ArrowCastOptions::new())
        .expect("two chunks");
    assert!(
        chunked.is_spilled(),
        "ChunkedSerie::from_arrow_reader collects through the same door"
    );

    let column = Serie::from_arrow_reader(None, stream(2, ROWS), ArrowCastOptions::new())
        .expect("one column");
    let doubled: Vec<Scalar> = expected(ROWS).into_iter().chain(expected(ROWS)).collect();
    assert_settled("Serie::from_arrow_reader", &column, &doubled);
}

/// `(id, value)` records: `id` cycles through `0..keys`, `value` counts up,
/// laid out as they stand.
fn pairs(side: &str, rows: usize, keys: i64) -> Serie {
    let value = if side == "l" {
        "left_value"
    } else {
        "right_value"
    };
    let root = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Int64.required_field(value),
        ])
        .expect("two children"),
    )
    .required_field(side);
    let batch = RecordBatch::try_new(
        root.clone().into_arrow_schema().expect("a schema"),
        vec![
            Arc::new(Int64Array::from(
                (0..rows as i64)
                    .map(|index| index % keys)
                    .collect::<Vec<_>>(),
            )),
            int64_array(rows),
        ],
    )
    .expect("a batch");
    Serie::from_arrow_batch(Some(&root), &batch, ArrowCastOptions::new()).expect("exact records")
}

#[test]
fn a_join_settles_its_output_under_the_process_default() {
    installed();
    let left = pairs("l", ROWS, ROWS as i64);
    let right = pairs("r", ROWS, ROWS as i64);
    assert!(!left.is_spilled() && !right.is_spilled());
    let joined = left
        .join_with(&right, "id", JoinKind::Inner, &JoinOptions::new())
        .expect("a join");
    assert_eq!(joined.len(), ROWS);
    assert!(joined.is_spilled(), "the joined column is settled");
    assert_eq!(joined.resident_size(), 0);
    let mut ids = joined.child("id").expect("the key").rows().into_owned();
    ids.sort();
    assert_eq!(ids, expected(ROWS));
}

#[test]
fn a_join_settles_its_output_under_the_bound_its_options_state_instead() {
    installed();
    let never =
        JoinOptions::new().with_spill(SpillOptions::new().with_byte_size(SpillOptions::NEVER));
    let left = pairs("l", ROWS, ROWS as i64);
    let right = pairs("r", ROWS, ROWS as i64);
    let joined = left
        .join_with(&right, "id", JoinKind::Inner, &never)
        .expect("a join");
    assert_eq!(joined.len(), ROWS);
    assert_resident("join_with under never", &joined);

    let halves = ChunkedSerie::from_series(
        None,
        [
            left.slice(0, ROWS / 2).expect("a slice"),
            left.slice(ROWS / 2, ROWS / 2).expect("a slice"),
        ],
        ArrowCastOptions::new(),
    )
    .expect("two chunks");
    let other = ChunkedSerie::from_serie(right).expect("one chunk");
    // A chunked join keeps its output chunks apart, each settled as it is
    // produced: under never every chunk stays resident whole, under the
    // default none stays resident past the bound.
    let joined = halves
        .join_with(&other, "id", JoinKind::Inner, &never)
        .expect("a join");
    assert_eq!(joined.len(), ROWS);
    assert!(u64::try_from(joined.memory_size()).expect("a byte count") > BOUND);
    for chunk in joined.chunks() {
        assert_resident("a chunked join under never", chunk);
    }
    let joined = halves
        .join_with(&other, "id", JoinKind::Inner, &JoinOptions::new())
        .expect("a join");
    assert_eq!(joined.len(), ROWS);
    for chunk in joined.chunks() {
        assert!(
            u64::try_from(chunk.resident_size()).expect("a byte count") <= BOUND,
            "a chunked join's chunk of {} bytes resident",
            chunk.resident_size()
        );
    }
}

#[test]
fn an_exact_landing_shares_the_callers_buffers_and_never_settles() {
    installed();
    let array = int64_array(ROWS);
    let column =
        Serie::from_arrow_array(Some(&price()), Arc::clone(&array), ArrowCastOptions::new())
            .expect("an exact column");
    assert_resident("from_arrow_array, exact", &column);
    let landed = column.into_arrow_array().expect("a column").to_data();
    assert_eq!(
        landed.buffers()[0].as_ptr(),
        array.to_data().buffers()[0].as_ptr(),
        "the caller's very buffer"
    );
    assert_eq!(column.rows(), expected(ROWS));

    let own = Serie::from_arrow_array(None, Arc::clone(&array), ArrowCastOptions::new())
        .expect("a column of its own field");
    assert_resident("from_arrow_array, no field", &own);

    let records = Serie::from_arrow_batch(
        Some(&quote_root()),
        &batch(ROWS, false),
        ArrowCastOptions::new(),
    )
    .expect("an exact batch");
    assert_resident("from_arrow_batch, exact", &records);
    let records = Serie::from_arrow_batch(None, &batch(ROWS, false), ArrowCastOptions::new())
        .expect("a batch of its own schema");
    assert_resident("from_arrow_batch, no root", &records);
}

#[test]
fn a_slice_and_a_window_never_settle() {
    installed();
    let column = heap(ROWS);
    let cut = column.slice(10, 900).expect("a slice in range");
    assert_resident("slice", &cut);
    assert_eq!(cut.rows(), &expected(ROWS)[10..910]);
    let window = column.window(10, 900).expect("a window in range");
    assert!(!window.is_spilled(), "window");
    assert_eq!(window.resident_size(), window.memory_size());
    assert_eq!(window.rows(), &expected(ROWS)[10..910]);
}

#[test]
fn a_reader_landing_a_batch_under_its_identity_plan_never_settles() {
    installed();
    let mut reader =
        StreamChunkedSerie::from_arrow_reader(None, stream(2, ROWS), ArrowCastOptions::new())
            .expect("a stream");
    for _ in 0..2 {
        let record = reader
            .next_chunk()
            .expect("a batch")
            .expect("a landed batch");
        assert_resident("StreamChunkedSerie::next, identity", &record);
        assert_eq!(xs(&record), expected(ROWS));
    }
    assert!(reader.next_chunk().is_none());
}

#[test]
fn a_row_write_never_settles() {
    installed();
    let mut column = heap(ROWS);
    column.set(0, Scalar::from(-1_i64)).expect("a row in range");
    assert_resident("set", &column);
    column
        .push(Scalar::from(-2_i64))
        .expect("a row of the field");
    assert_resident("push", &column);
    column
        .insert(1, Scalar::from(-3_i64))
        .expect("a row of the field");
    assert_resident("insert", &column);
    column.remove(1).expect("a row in range");
    assert_resident("remove", &column);
    assert_eq!(column.len(), ROWS + 1);
    assert_eq!(column.scalar(0).expect("row 0"), Scalar::from(-1_i64));
    assert_eq!(
        column.scalar(ROWS).expect("the pushed row"),
        Scalar::from(-2_i64)
    );
}

#[test]
fn one_chunk_joined_is_the_chunk_itself_and_never_settles() {
    installed();
    let chunked = ChunkedSerie::from_series(Some(&price()), [heap(ROWS)], ArrowCastOptions::new())
        .expect("one chunk");
    let joined = chunked.into_serie().expect("one column");
    assert_resident("ChunkedSerie::into_serie, one chunk", &joined);
}

#[test]
fn a_column_under_the_bound_is_never_spilled_by_any_door() {
    installed();
    let few = expected(FEW);
    let narrow: ArrayRef = Arc::new(Int32Array::from((0..FEW as i32).collect::<Vec<_>>()));
    let target = Field::new("x", DataType::Float64, false);
    let mut grown = heap(FEW);
    grown.extend_from_serie(&heap(FEW)).expect("appended");
    let mut sorted = heap(FEW);
    sorted.as_sorted(SortOptions::descending()).expect("sorted");
    let mut pushed =
        ChunkedSerie::from_series(Some(&price()), [heap(FEW)], ArrowCastOptions::new())
            .expect("a chunk");
    pushed
        .push_chunk(heap(FEW), ArrowCastOptions::new())
        .expect("a chunk");
    let left = pairs("l", FEW, FEW as i64);
    let right = pairs("r", FEW, FEW as i64);

    let doors: Vec<(&str, Serie)> = vec![
        (
            "from_scalars",
            Serie::from_scalars(price(), few.clone()).expect("rows"),
        ),
        (
            "from_arrow_array, cast",
            Serie::from_arrow_array(Some(&price()), narrow, ArrowCastOptions::new())
                .expect("a cast"),
        ),
        (
            "from_arrow_batch, cast",
            Serie::from_arrow_batch(
                Some(&quote_root()),
                &batch(FEW, true),
                ArrowCastOptions::new(),
            )
            .expect("a cast"),
        ),
        (
            "cast",
            heap(FEW)
                .cast(&target, ArrowCastOptions::new())
                .expect("a cast"),
        ),
        (
            "into_sorted",
            heap(FEW)
                .into_sorted(SortOptions::descending())
                .expect("sorted"),
        ),
        ("into_reversed", heap(FEW).into_reversed()),
        ("into_unique", heap(FEW).into_unique().expect("unique")),
        ("as_sorted", sorted),
        ("extend_from_serie", grown),
        (
            "ChunkedSerie::into_serie",
            pushed.into_serie().expect("one column"),
        ),
        (
            "Serie::from_arrow_reader",
            Serie::from_arrow_reader(None, stream(2, FEW), ArrowCastOptions::new())
                .expect("a column"),
        ),
        (
            "join_with",
            left.join_with(&right, "id", JoinKind::Inner, &JoinOptions::new())
                .expect("a join"),
        ),
    ];
    for (door, column) in doors {
        assert!(column.memory_size() > 0, "{door}");
        assert!(
            u64::try_from(column.memory_size()).expect("a byte count") <= BOUND,
            "{door}: {} bytes",
            column.memory_size()
        );
        assert_resident(door, &column);
    }
    assert!(!pushed.is_spilled(), "push_chunk under the bound");
    assert_eq!(
        Serie::from_scalars(price(), few.clone())
            .expect("rows")
            .rows(),
        few
    );
}

#[test]
fn a_held_join_past_one_output_batch_keeps_a_stated_never_bound() {
    installed();
    // Two output batches: every left row matches two right rows, and the
    // held door joins them into one column under the join's own bound, so
    // `NEVER` keeps the output resident where the process default would
    // have spilled the join before the options were read.
    let rows = 40_000;
    let never =
        JoinOptions::new().with_spill(SpillOptions::new().with_byte_size(SpillOptions::NEVER));
    let left = pairs("l", rows, 1_000);
    let right = pairs("r", 2_000, 1_000);
    let joined = left
        .join_with(&right, "id", JoinKind::Inner, &never)
        .expect("a join of two batches");
    assert_eq!(joined.len(), 2 * rows);
    assert_resident("join_with past one batch under never", &joined);
    let settled = left
        .join_with(&right, "id", JoinKind::Inner, &JoinOptions::new())
        .expect("a join under the default");
    assert!(
        settled.is_spilled(),
        "the same join settles under the process default"
    );
}

/// A write holds its cadence under the bound too: eight batches past it
/// are published whole through an IPC leaf, the rows all there, whatever the
/// window spilled on the way.
#[test]
fn a_write_cadence_held_past_the_bound_publishes_every_row_it_was_handed() {
    installed();
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOMedia, IOMode, Url};

    let batches: Vec<RecordBatch> = (0..8).map(|_| batch(ROWS, false)).collect();
    let schema = batches[0].schema();
    let stream = StreamChunkedSerie::from_arrow_reader(
        Some(&quote_root()),
        yggdryl::arrow::batch_reader(schema, batches),
        ArrowCastOptions::new(),
    )
    .expect("a stream");
    let mut target = Buffer::new().with_media_type(
        Url::from_str("file:///quotes.arrows")
            .expect("a url")
            .media_type(),
    );
    let mut options = target.record_options().expect("the IPC encoding");
    options.set_commit_batch_num(Some(3));
    target
        .write_serie(Serie::from(stream), IOMode::Overwrite, Some(&options))
        .expect("the stream writes in three cadences");
    let rows: usize =
        yggdryl::StreamChunkedSerie::from_serie(target.read_serie(None).expect("the rows read"))
            .expect("native record stream")
            .into_chunks()
            .map(|column| column.expect("a batch").len())
            .sum();
    assert_eq!(rows, 8 * ROWS);
}

#[cfg(feature = "internals")]
mod internal {
    //! The publication window's residency, which no caller can read.

    use super::*;
    use yggdryl::internals::media_options_commit::residency_after;

    #[test]
    fn a_write_cadence_held_past_the_bound_spills_its_heaviest_batches() {
        installed();
        let batches: Vec<RecordBatch> = (0..8).map(|_| batch(ROWS, false)).collect();
        let held = residency_after(batches[0].schema(), batches, 16).expect("the window holds");
        assert_eq!(
            held,
            (0, 8),
            "every eight-kilobyte batch passes a 256-byte bound on its own"
        );

        let few: Vec<RecordBatch> = (0..2).map(|_| batch(FEW, false)).collect();
        let (resident, spilled) =
            residency_after(few[0].schema(), few, 16).expect("the window holds");
        assert_eq!(spilled, 0, "two three-row batches stay under the bound");
        assert!(resident > 0 && resident <= BOUND, "{resident}");

        // A completed cadence is taken, so nothing is held after it.
        let batches: Vec<RecordBatch> = (0..4).map(|_| batch(ROWS, false)).collect();
        assert_eq!(
            residency_after(batches[0].schema(), batches, 4).expect("the window holds"),
            (0, 0)
        );
    }
}

/// A constant column holds its value once whatever its length, so the bound
/// never reaches it: the default a door lays out is a lit, resident and
/// never spilled, and exporting it builds the array without a spill.
#[test]
fn a_constant_column_is_never_spilled_whatever_its_length() {
    installed();
    let defaults = Serie::from_default(price(), ROWS).expect("a default column");
    assert!(
        defaults.as_lit().is_some(),
        "a default is a constant column"
    );
    assert!(!defaults.is_spilled());
    assert!(
        defaults.resident_size() < BOUND as usize,
        "{}",
        defaults.resident_size()
    );
    let constant = || Serie::lit(price(), Scalar::from(7_i64), ROWS).expect("a constant column");
    let exported = constant();
    let array = exported.into_arrow_array().expect("the array builds");
    assert_eq!(array.len(), ROWS);
    assert!(!exported.is_spilled(), "exporting builds, it never spills");
    assert!(
        exported.resident_size() > defaults.resident_size(),
        "the layout is held"
    );
    // The one join of chunks of constants lands as rows the crate laid out
    // and settles like any other column.
    let chunked =
        ChunkedSerie::from_series(Some(&price()), [constant(), constant()], Default::default())
            .expect("two constant chunks");
    assert_eq!(chunked.resident_size(), 2 * defaults.resident_size());
    assert!(
        chunked.into_serie().expect("one join").is_spilled(),
        "the join passes the bound"
    );
}
