//! Key layouts, held and lazy clustering, and generic replay.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{RecordBatchIterator, RecordBatchReader};
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, KeySeries, PartitionOptions, Scalar, Serie,
    StreamChunkedSerie, StreamKeySerie, StreamSerie, StructType,
};

fn root() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::utf8().required_field("venue"),
            DataType::Int64.required_field("qty"),
            DataType::Int32.required_field("day"),
        ])
        .unwrap(),
    )
    .required_field("quote")
}

fn rows(values: &[(&str, i64, i32)]) -> Serie {
    Serie::from_scalars(
        root(),
        values.iter().map(|(venue, qty, day)| {
            Scalar::from_sequence([Scalar::from(*venue), Scalar::from(*qty), Scalar::from(*day)])
        }),
    )
    .unwrap()
}

fn sample() -> Serie {
    rows(&[
        ("a", 1, 1),
        ("a", 2, 2),
        ("b", 3, 1),
        ("a", 4, 2),
        ("b", 5, 1),
    ])
}

fn stream(pieces: Vec<Serie>) -> StreamChunkedSerie {
    StreamChunkedSerie::from_chunked(
        ChunkedSerie::from_series(Some(&root()), pieces, ArrowCastOptions::new()).unwrap(),
    )
    .unwrap()
}

fn keys(items: &KeySeries) -> Vec<Scalar> {
    items.iter().map(|item| item.key().clone()).collect()
}
fn one(value: impl Into<Scalar>) -> Scalar {
    Scalar::from_sequence([value.into()])
}
fn names(field: &Field) -> Vec<&str> {
    field.fields().iter().map(Field::name).collect()
}
fn sizes(stream: StreamChunkedSerie) -> Vec<usize> {
    stream
        .into_chunks()
        .map(|piece| piece.unwrap().len())
        .collect()
}

#[test]
fn a_bare_key_moves_its_child_without_copying_buffers() {
    let rows = sample();
    let groups = rows.partition_by("venue").unwrap();
    assert_eq!(names(groups.key_field()), ["venue"]);
    assert_eq!(names(groups.serie_field()), ["qty", "day"]);
    assert_eq!(names(groups.field()), ["venue", "qty", "day"]);
    assert_eq!(groups.key_paths()[0].as_ref().unwrap().to_string(), "venue");
    assert_eq!(keys(&groups), [one("a"), one("b")]);
    assert_eq!(
        groups
            .iter()
            .map(|item| item.rows().len())
            .collect::<Vec<_>>(),
        [3, 2]
    );
    assert!(groups.iter().all(|item| item.rownum().is_none()));
    let windows = rows.window_by("venue", false).unwrap();
    let original = rows.child("qty").unwrap().require_arrow_array().unwrap();
    let child = windows[0]
        .rows()
        .child("qty")
        .unwrap()
        .require_arrow_array()
        .unwrap();
    assert_eq!(
        original.to_data().buffers()[0].as_ptr(),
        child.to_data().buffers()[0].as_ptr()
    );
}

#[test]
fn layouts_keep_aliases_computed_paths_and_nullable_key_cells() {
    let groups = sample()
        .window_by("venue as desk, day + 1 as tomorrow", false)
        .unwrap();
    assert_eq!(names(groups.key_field()), ["desk", "tomorrow"]);
    assert_eq!(names(groups.serie_field()), ["qty", "day"]);
    assert_eq!(groups.key_paths()[0].as_ref().unwrap().to_string(), "venue");
    assert!(groups.key_paths()[1].is_none());
    for by in ["qty + 1 as qty", "day + 1 as VeNuE"] {
        assert!(
            sample()
                .window_by(by, false)
                .unwrap_err()
                .to_string()
                .contains("alias the key cell")
        );
    }
    assert!(sample().window_by("qty + 1 as quantity", false).is_ok());
}

#[test]
fn a_star_beside_terms_moves_every_retained_source_column() {
    let groups = sample()
        .window_by("* exclude(day), day + 1 as shift", false)
        .unwrap();
    assert_eq!(names(groups.key_field()), ["venue", "qty", "shift"]);
    assert_eq!(names(groups.serie_field()), ["day"]);
    assert_eq!(groups.key_paths().len(), 3);
    assert!(
        groups.key_paths()[0].is_some()
            && groups.key_paths()[1].is_some()
            && groups.key_paths()[2].is_none()
    );
}

#[test]
fn external_keys_are_typed_records_and_name_their_own_cells() {
    let external = Serie::from_scalars(
        DataType::Int32.required_field("desk"),
        [1, 1, 2, 1, 2].map(Scalar::from),
    )
    .unwrap();
    let groups = sample().partition_by(&external).unwrap();
    assert_eq!(names(groups.key_field()), ["desk"]);
    assert_eq!(names(groups.serie_field()), ["venue", "qty", "day"]);
    assert_eq!(keys(&groups), [one(1_i32), one(2_i32)]);
    assert!(groups.key_paths().iter().all(Option::is_none));
    assert!(
        sample()
            .partition_by(&Serie::new(vec![Scalar::Null; 5]))
            .is_err()
    );
    assert!(
        sample()
            .partition_by(&external.slice(0, 1).unwrap())
            .is_err()
    );
    assert!(Serie::new(Vec::new()).partition_by(&external).is_err());
    assert!(
        sample()
            .partition_by(sample().child("venue").unwrap())
            .unwrap_err()
            .to_string()
            .contains("alias the key cell")
    );
}

#[test]
fn windows_cross_chunk_edges_and_sorted_gathers_have_no_source_position() {
    let rows = sample();
    let chunks = ChunkedSerie::from_series(
        Some(&root()),
        [
            rows.slice(0, 1).unwrap(),
            rows.slice(1, 3).unwrap(),
            rows.slice(4, 1).unwrap(),
        ],
        ArrowCastOptions::new(),
    )
    .unwrap();
    for held in [rows.clone(), Serie::from(chunks)] {
        let windows = held.window_by("venue", false).unwrap();
        assert_eq!(keys(&windows), [one("a"), one("b"), one("a"), one("b")]);
        assert_eq!(
            windows
                .iter()
                .map(|item| (item.rownum(), item.rows().len()))
                .collect::<Vec<_>>(),
            [(Some(0), 2), (Some(2), 1), (Some(3), 1), (Some(4), 1)]
        );
        let sorted = held.window_by("venue", true).unwrap();
        assert_eq!(keys(&sorted), [one("a"), one("b")]);
        assert!(sorted.iter().all(|item| item.rownum().is_none()));
        assert_eq!(
            sorted[0].rows().child("qty").unwrap().rows().to_vec(),
            [1_i64, 2, 4].map(Scalar::from)
        );
    }
    let tail = rows
        .window(2, 3)
        .unwrap()
        .window_by("venue", false)
        .unwrap();
    assert_eq!(tail[0].rownum(), Some(2));
}

#[test]
fn nested_keys_bind_global_rows_and_keep_paths_and_absolute_positions() {
    let outer = sample().window_by("venue as desk", false).unwrap();
    let nested = outer[0].window_by("day", false).unwrap();
    assert_eq!(names(nested.key_field()), ["desk", "day"]);
    assert_eq!(nested[1].rownum(), Some(1));
    assert_eq!(nested.key_paths()[0], outer.key_paths()[0]);
    assert_eq!(names(nested.serie_field()), ["qty"]);
    let again = outer[0].window_by("desk as again", false).unwrap();
    assert_eq!(
        again[0].key(),
        &Scalar::from_sequence([Scalar::from("a"), Scalar::from("a")])
    );
    assert!(
        outer[0]
            .window_by("desk", false)
            .unwrap_err()
            .to_string()
            .contains("alias the key cell")
    );
    let grouped = sample()
        .partition_by("venue")
        .unwrap()
        .window_by("day", false)
        .unwrap();
    assert!(grouped.iter().all(|item| item.rownum().is_none()));
}

#[test]
fn a_lazy_layout_is_available_before_pulls_and_a_skipped_payload_refuses_once() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let batches = stream(vec![sample()]).into_arrow_reader();
    let schema = batches.schema();
    let counter = Arc::clone(&pulls);
    let batches = RecordBatchIterator::new(
        batches.inspect(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
        schema,
    );
    let mut groups =
        StreamChunkedSerie::from_arrow_reader(None, Box::new(batches), ArrowCastOptions::new())
            .unwrap()
            .window_by("venue", false)
            .unwrap();
    assert_eq!(names(groups.key_field()), ["venue"]);
    assert_eq!(names(groups.serie_field()), ["qty", "day"]);
    assert_eq!(pulls.load(Ordering::SeqCst), 0);
    let first = groups.next().unwrap().unwrap();
    assert_eq!(first.rownum(), Some(0));
    assert!(
        first
            .window_by("day", false)
            .unwrap_err()
            .to_string()
            .contains("StreamKeySerie")
    );
    let second = groups.next().unwrap().unwrap();
    let mut skipped = first
        .rows()
        .clone()
        .into_chunked_stream(None, None)
        .unwrap();
    assert!(skipped.next_chunk().unwrap().is_err());
    assert!(skipped.next_chunk().is_none());
    assert_eq!(second.rows().len(), 1);
}

#[test]
fn stream_partitions_keep_closing_orders_and_chunked_payloads() {
    let grouped = stream(vec![
        sample().slice(0, 3).unwrap(),
        sample().slice(3, 2).unwrap(),
    ])
    .partition_by("venue", PartitionOptions::new().with_threads(1))
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert_eq!(
        grouped
            .iter()
            .map(|item| item.key().clone())
            .collect::<Vec<_>>(),
        [one("a"), one("b")]
    );
    assert_eq!(
        grouped
            .iter()
            .map(|item| item.rows().len())
            .collect::<Vec<_>>(),
        [3, 2]
    );
    assert!(
        grouped
            .iter()
            .all(|item| item.rownum().is_none() && item.rows().as_chunked().is_some())
    );
}

#[test]
fn generic_key_stream_clones_replay_all_items_once_and_keep_boundaries() {
    let windows = stream(vec![
        sample().slice(0, 1).unwrap(),
        sample().slice(1, 4).unwrap(),
    ])
    .window_by("venue", false)
    .unwrap();
    let generic = Serie::from(windows);
    assert!(!generic.is_held());
    assert_eq!(generic.memory_size(), 0);
    let clone = generic.clone();
    assert_eq!(
        sizes(generic.into_chunked_stream(Some(100), None).unwrap()),
        [2, 1, 1, 1]
    );
    assert_eq!(clone.len(), 5);
    assert_eq!(
        sizes(clone.into_chunked_stream(Some(100), None).unwrap()),
        [2, 1, 1, 1]
    );
}

#[test]
fn all_kinds_convert_and_key_conversions_never_join_across_items() {
    let held = sample();
    let expected = held.rows().to_vec();
    for kind in [
        held.clone(),
        Serie::from(held.window_by("venue", false).unwrap()),
        Serie::from(stream(vec![held.clone()])),
        Serie::from(held.clone().into_stream().unwrap()),
        Serie::from(
            stream(vec![held.clone()])
                .window_by("venue", false)
                .unwrap(),
        ),
    ] {
        assert_eq!(
            kind.into_stream().unwrap().collect_rows().unwrap(),
            expected
        );
    }
    let keys = held.window_by("venue", false).unwrap();
    assert_eq!(
        sizes(keys.into_chunked_stream(Some(100), None).unwrap()),
        [2, 1, 1, 1]
    );
}

#[test]
fn rechunking_closes_on_the_bound_without_an_extra_pull_and_passes_alone_chunks() {
    let first = sample().slice(0, 2).unwrap();
    let original = first.child("qty").unwrap().clone();
    let mut passed = stream(vec![first])
        .into_chunked_stream(Some(1), None)
        .unwrap();
    let output = passed.next_chunk().unwrap().unwrap();
    let (Serie::Int64(original), Serie::Int64(output)) = (&original, output.child("qty").unwrap())
    else {
        panic!("an int64 child")
    };
    assert!(Arc::ptr_eq(original, output));
    let pulls = Arc::new(AtomicUsize::new(0));
    let batches = stream(vec![
        sample().slice(0, 1).unwrap(),
        sample().slice(1, 3).unwrap(),
        sample().slice(4, 1).unwrap(),
    ])
    .into_arrow_reader();
    let schema = batches.schema();
    let count = Arc::clone(&pulls);
    let reader = RecordBatchIterator::new(
        batches.inspect(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        }),
        schema,
    );
    let mut chunks =
        StreamChunkedSerie::from_arrow_reader(None, Box::new(reader), ArrowCastOptions::new())
            .unwrap()
            .into_chunked_stream(Some(3), None)
            .unwrap();
    assert_eq!(chunks.next_chunk().unwrap().unwrap().len(), 3);
    assert_eq!(pulls.load(Ordering::SeqCst), 2);
    assert_eq!(chunks.next_chunk().unwrap().unwrap().len(), 2);
}

#[test]
fn key_types_are_send_and_sync_and_empty_layouts_still_bind() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StreamKeySerie>();
    assert_send_sync::<yggdryl::KeySerie>();
    assert_send_sync::<Serie>();
    let groups = rows(&[]).window_by("venue", false).unwrap();
    assert!(groups.is_empty());
    assert_eq!(names(groups.serie_field()), ["qty", "day"]);
    assert!(groups.window_by("missing", false).is_err());
    assert!(rows(&[]).window_by("*", false).is_err());
    assert!(
        StreamSerie::from_rows(DataType::Int64.required_field("qty"), [])
            .into_stream()
            .is_err()
    );
}

#[test]
fn a_generic_row_stream_checks_its_record_field_before_conversion() {
    let raw = Serie::from(StreamSerie::from_rows(
        DataType::Int64.required_field("qty"),
        [],
    ));
    assert!(raw.into_stream().is_err());
}

#[test]
fn a_row_stream_keeps_the_prefix_before_a_row_its_field_refuses() {
    let first = sample().scalar(0).unwrap();
    let held = Serie::from(StreamSerie::from_rows(
        root(),
        [Ok(first), Ok(Scalar::from(3_i64))],
    ));
    assert_eq!(held.len(), 1);
    assert!(held.scalar(0).is_err());
}

#[test]
fn a_shared_row_stream_verifies_declared_order_at_chunk_edges() {
    let ordered = sample().into_sort_by("qty").unwrap();
    let field = ordered.require_field().unwrap().clone();
    let first = ordered.scalar(0).unwrap();
    let count = yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE;
    let rows = std::iter::repeat_with(move || Ok(first.clone()))
        .take(count)
        .chain([Ok(Scalar::from_sequence([
            Scalar::from("a"),
            Scalar::from(0_i64),
            Scalar::from(1_i32),
        ]))]);
    let held = Serie::from(StreamSerie::from_rows(field, rows));
    assert_eq!(held.len(), count);
    assert!(held.scalar(0).is_err());
}

#[test]
fn a_held_key_collection_refuses_absent_record_rows_before_conversion() {
    let nullable = Serie::from_scalars(
        root().with_nullable(true),
        [Scalar::Null, sample().scalar(0).unwrap()],
    )
    .unwrap();
    let groups = nullable.window_by("venue", false).unwrap();
    assert_eq!(groups[0].key(), &one(Scalar::Null));
    assert!(groups.key_field().fields()[0].is_nullable());
    assert_eq!(groups[0].rows().scalar(0).unwrap(), Scalar::Null);
    assert_eq!(
        Serie::from(groups[0].clone()).scalar(0).unwrap(),
        Scalar::Null
    );
    assert!(groups.into_stream().is_err());
}

#[test]
fn key_conversions_normalize_nullable_roots_with_present_rows() {
    let nullable =
        Serie::from_scalars(root().with_nullable(true), sample().rows().into_owned()).unwrap();
    let groups = nullable.window_by("venue", false).unwrap();
    let stream = groups.into_stream().unwrap();
    assert!(!stream.field().is_nullable());
    assert_eq!(stream.collect_rows().unwrap(), sample().rows().into_owned());
}

#[test]
fn streamed_keys_compose_paths_and_absolute_positions() {
    let groups = stream(vec![sample()])
        .window_by("venue as desk", false)
        .unwrap()
        .window_by("day", false)
        .unwrap();
    let rows = groups
        .map(|item| {
            let item = item.unwrap();
            assert_eq!(names(item.key_field()), ["desk", "day"]);
            assert_eq!(item.key_paths()[0].as_ref().unwrap().to_string(), "venue");
            let key = item.key().clone();
            let at = item.rownum();
            let rows = item.into_stream().unwrap().collect_rows().unwrap();
            (key, at, rows.len())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rows.iter()
            .map(|(_, at, len)| (*at, *len))
            .collect::<Vec<_>>(),
        [
            (Some(0), 1),
            (Some(1), 1),
            (Some(2), 1),
            (Some(3), 1),
            (Some(4), 1)
        ]
    );
}

#[test]
fn streamed_composition_refuses_an_empty_match_key_before_pulls() {
    let groups = stream(vec![sample()]).window_by("venue", false).unwrap();
    assert!(groups.window_by("*", false).is_err());
}

#[test]
fn external_stream_keys_are_refused_without_pulling_them() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&pulls);
    let keys = Serie::from(StreamSerie::from_rows(
        DataType::from(StructType::from_fields([DataType::Int64.required_field("group")]).unwrap())
            .required_field("keys"),
        [Ok(one(1_i64))].into_iter().inspect(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        }),
    ));
    assert!(stream(vec![sample()]).window_by(&keys, false).is_err());
    assert_eq!(pulls.load(Ordering::SeqCst), 0);
}

#[test]
fn composed_keys_keep_the_remaining_payload_order() {
    let rows = sample().into_sort_by("day, qty").unwrap();
    let held = rows
        .window_by("venue", false)
        .unwrap()
        .window_by("qty as amount", false)
        .unwrap();
    assert_eq!(
        held.serie_field().get_metadata("SORT:by"),
        Some(r#"["day"]"#)
    );
    let streamed = StreamChunkedSerie::from_serie(rows)
        .unwrap()
        .window_by("venue", false)
        .unwrap()
        .window_by("qty as amount", false)
        .unwrap();
    assert_eq!(
        streamed.serie_field().get_metadata("SORT:by"),
        Some(r#"["day"]"#)
    );
    assert!(streamed.field().get_metadata("SORT:by").is_none());
}

#[test]
fn a_default_row_conversion_uses_the_byte_bound_without_a_row_ceiling() {
    let field = DataType::from_str("struct<value: int32 not null>")
        .unwrap()
        .required_field("rows");
    let row = Scalar::from_sequence([Scalar::from(1_i32)]);
    let count = yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE + 1;
    let stream = StreamSerie::from_rows(
        field,
        std::iter::repeat_with(move || Ok(row.clone())).take(count),
    );
    let lengths = stream
        .into_chunked_stream(None, None)
        .unwrap()
        .into_chunks()
        .map(|piece| piece.unwrap().len())
        .collect::<Vec<_>>();
    assert_eq!(lengths, [count]);
}
