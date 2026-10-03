//! `rust/src/serie/join.rs`: the three join verbs - a held serie, a
//! chunked serie and a stream - each one door onto the engine, the stream
//! probed one batch at a time unless the build side passes the spill bound.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{Int64Array, RecordBatch, RecordBatchIterator};
use yggdryl::arrow::BatchReader;
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, JoinKind, JoinOptions, Scalar, Serie,
    SerieReader, SerieSource, SpillOptions, StructType,
};

fn record(name: &str, fields: Vec<Field>) -> Field {
    DataType::from(StructType::from_fields(fields).expect("distinct names")).required_field(name)
}

fn row(cells: impl IntoIterator<Item = Scalar>) -> Scalar {
    Scalar::from_sequence(cells)
}

fn int(value: i64) -> Scalar {
    Scalar::from(value)
}

/// `(id, value)` rows: `id` cycles through `0..keys`, `value` counts up.
fn pairs(name: &str, rows: usize, keys: i64) -> Serie {
    let root = record(
        name,
        vec![
            DataType::Int64.required_field("id"),
            DataType::Int64.required_field(if name == "l" {
                "left_value"
            } else {
                "right_value"
            }),
        ],
    );
    Serie::from_scalars(
        root,
        (0..rows).map(|index| row([int(index as i64 % keys), int(index as i64)])),
    )
    .expect("pairs")
}

#[test]
fn a_held_serie_joins_a_held_serie_into_one_column() {
    let left = pairs("l", 6, 3);
    let right = pairs("r", 4, 2);
    let joined = left
        .join_with(&right, "id", JoinKind::Inner, &JoinOptions::new())
        .expect("a join");
    // ids 0 and 1 appear twice on each side: 2 * 2 * 2 rows; id 2 matches nothing.
    assert_eq!(joined.len(), 8);
    assert_eq!(
        joined
            .require_field()
            .expect("a record")
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["id", "left_value", "right_value"]
    );
    assert!(joined.as_struct().is_some());
    assert!(!joined.is_spilled());
}

#[test]
fn a_chunked_serie_keeps_its_output_batches_apart() {
    let left = pairs("l", 6, 3);
    let left_chunks = ChunkedSerie::from_series(
        None,
        [
            left.slice(0, 2).expect("a slice"),
            left.slice(2, 2).expect("a slice"),
            left.slice(4, 2).expect("a slice"),
        ],
        ArrowCastOptions::new(),
    )
    .expect("three chunks");
    let right = ChunkedSerie::from_serie(pairs("r", 4, 2)).expect("one chunk");
    let joined = left_chunks
        .join_with(&right, "id", JoinKind::Inner, &JoinOptions::new())
        .expect("a join");
    // The right side is smaller and is built; every left chunk probes and
    // answers its own output batch, a chunk with no match answers none.
    assert_eq!(joined.len(), 8);
    assert_eq!(joined.num_chunks(), 3);
    assert_eq!(joined.field().name(), "l");
    assert_eq!(
        joined.rows(),
        left.join_with(
            &pairs("r", 4, 2),
            "id",
            JoinKind::Inner,
            &JoinOptions::new()
        )
        .expect("the one-column join")
        .rows()
        .to_vec()
    );
}

/// A stream of `batches` batches of `rows` rows each, counting its pulls.
fn counted_stream(
    root: &Field,
    batches: usize,
    rows: usize,
    pulls: Arc<AtomicUsize>,
) -> BatchReader {
    let schema = root.clone().into_arrow_schema().expect("a schema");
    let parts: Vec<RecordBatch> = (0..batches)
        .map(|batch| {
            let ids = Int64Array::from_iter_values((0..rows).map(|index| (index % 3) as i64));
            let values =
                Int64Array::from_iter_values((0..rows).map(|index| (batch * rows + index) as i64));
            RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(ids), Arc::new(values)])
                .expect("a batch")
        })
        .collect();
    Box::new(RecordBatchIterator::new(
        parts.into_iter().map(move |batch| {
            pulls.fetch_add(1, Ordering::SeqCst);
            Ok(batch)
        }),
        schema,
    ))
}

#[test]
fn a_stream_probes_one_batch_at_a_time_and_collects_nothing() {
    let root = record(
        "l",
        vec![
            DataType::Int64.required_field("id"),
            DataType::Int64.required_field("left_value"),
        ],
    );
    let pulls = Arc::new(AtomicUsize::new(0));
    let stream = SerieReader::from_arrow_reader(
        Some(&root),
        counted_stream(&root, 4, 3, Arc::clone(&pulls)),
        ArrowCastOptions::new(),
    )
    .expect("a stream");
    let right = pairs("r", 4, 2);
    let mut joined = stream
        .join_with(right, "id", JoinKind::Inner, &JoinOptions::new())
        .expect("a lazy join");
    // Resolving the join pulls nothing: the held side is built, the stream waits.
    assert_eq!(pulls.load(Ordering::SeqCst), 0);
    assert_eq!(joined.field().name(), "l");
    let first = joined
        .next()
        .expect("a first output batch")
        .expect("joined rows");
    // One probe batch pulled, its output answered, the rest of the stream untouched.
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
    // ids 0, 1, 2 in a batch of three: id 0 and 1 each match two right rows.
    assert_eq!(first.len(), 4);
    let rest: Vec<Serie> = joined.map(|batch| batch.expect("joined rows")).collect();
    assert_eq!(pulls.load(Ordering::SeqCst), 4);
    assert_eq!(rest.len(), 3);
    assert_eq!(rest.iter().map(Serie::len).sum::<usize>(), 12);
}

#[test]
fn a_stream_probing_a_build_side_past_the_spill_bound_is_read_whole_before_the_first_batch() {
    let root = record(
        "l",
        vec![
            DataType::Int64.required_field("id"),
            DataType::Int64.required_field("left_value"),
        ],
    );
    let stream = |pulls: &Arc<AtomicUsize>| {
        SerieReader::from_arrow_reader(
            Some(&root),
            counted_stream(&root, 4, 3, Arc::clone(pulls)),
            ArrowCastOptions::new(),
        )
        .expect("a stream")
    };
    let right = pairs("r", 4, 2);
    let options = JoinOptions::new().with_spill(SpillOptions::new().with_byte_size(1));
    let pulls = Arc::new(AtomicUsize::new(0));
    let mut joined = stream(&pulls)
        .join_with(right.clone(), "id", JoinKind::Inner, &options)
        .expect("a partitioned join");
    // Resolving the join still pulls nothing: the held side is partitioned,
    // the stream waits.
    assert_eq!(pulls.load(Ordering::SeqCst), 0);
    let first = joined
        .next()
        .expect("a first output batch")
        .expect("joined rows");
    // The build side does not fit, so the probe is read whole and
    // partitioned before the first partition is joined.
    assert_eq!(pulls.load(Ordering::SeqCst), 4);
    assert!(first.is_spilled());
    let mut rows: Vec<Scalar> = first.rows().to_vec();
    rows.extend(joined.flat_map(|batch| batch.expect("joined rows").rows().to_vec()));
    let mut streamed: Vec<Scalar> = stream(&Arc::new(AtomicUsize::new(0)))
        .join_with(right, "id", JoinKind::Inner, &JoinOptions::new())
        .expect("a streamed join")
        .flat_map(|batch| batch.expect("joined rows").rows().to_vec())
        .collect();
    rows.sort();
    streamed.sort();
    assert_eq!(rows.len(), 16);
    assert_eq!(rows, streamed);
}

#[test]
fn a_stream_against_a_stream_holds_the_right_one() {
    let root = record(
        "l",
        vec![
            DataType::Int64.required_field("id"),
            DataType::Int64.required_field("left_value"),
        ],
    );
    let right_root = record(
        "r",
        vec![
            DataType::Int64.required_field("id"),
            DataType::Int64.required_field("right_value"),
        ],
    );
    let (left_pulls, right_pulls) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let left = SerieReader::from_arrow_reader(
        Some(&root),
        counted_stream(&root, 2, 3, Arc::clone(&left_pulls)),
        ArrowCastOptions::new(),
    )
    .expect("a stream");
    let right = SerieReader::from_arrow_reader(
        Some(&right_root),
        counted_stream(&right_root, 2, 3, Arc::clone(&right_pulls)),
        ArrowCastOptions::new(),
    )
    .expect("a stream");
    let joined = left
        .join_with(
            SerieSource::Reader(right),
            "id",
            JoinKind::Left,
            &JoinOptions::new(),
        )
        .expect("a join of two streams");
    // The right stream is the build side: drained before the first pull of the left.
    assert_eq!(right_pulls.load(Ordering::SeqCst), 2);
    assert_eq!(left_pulls.load(Ordering::SeqCst), 0);
    let rows: usize = joined.map(|batch| batch.expect("rows").len()).sum();
    // Every left id 0, 1, 2 matches two right rows per id: 6 left rows * 2.
    assert_eq!(rows, 12);
    assert_eq!(left_pulls.load(Ordering::SeqCst), 2);
}

#[test]
fn the_three_verbs_agree_on_the_rows() {
    let left = pairs("l", 7, 3);
    let right = pairs("r", 5, 2);
    let held = left
        .join_with(&right, "id", JoinKind::Full, &JoinOptions::new())
        .expect("held");
    let chunked = ChunkedSerie::from_serie(left.clone())
        .expect("one chunk")
        .join_with(
            &ChunkedSerie::from_serie(right.clone()).expect("one chunk"),
            "id",
            JoinKind::Full,
            &JoinOptions::new(),
        )
        .expect("chunked");
    let streamed: Vec<Scalar> = SerieReader::from_serie(left)
        .expect("a stream")
        .join_with(right, "id", JoinKind::Full, &JoinOptions::new())
        .expect("streamed")
        .flat_map(|batch| batch.expect("rows").rows().to_vec())
        .collect();
    assert_eq!(held.rows().to_vec(), chunked.rows());
    assert_eq!(held.rows().to_vec(), streamed);
}
