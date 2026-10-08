//! Native media scans and their generic serie representation.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use yggdryl::csv::{CSVSerie, Csv, CsvOptions};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, Field, MediaSerieValue, Scalar, SerieValue};

fn field() -> Field {
    DataType::from_str("struct<id: int64 not null, value: int64 not null>")
        .unwrap()
        .required_field("rows")
}
fn csv(bytes: &[u8]) -> CSVSerie {
    CSVSerie::new(
        Csv::new(Buffer::from_bytes(bytes.to_vec()))
            .with_options(CsvOptions::new().with_field(field())),
        None,
    )
    .unwrap()
}

#[test]
fn a_row_media_stream_does_not_prebatch_past_the_requested_row() {
    let source = csv(b"id,value\n1,2\n2,invalid\n");
    let mut rows = source.into_stream().unwrap();
    assert_eq!(
        rows.next().unwrap().unwrap(),
        Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from))
    );
    assert!(rows.next().unwrap().is_err());
    assert!(rows.next().is_none());
    assert!(rows.next().is_none());
}

#[test]
fn media_scan_clauses_bind_before_pulls_and_keep_native_options() {
    let source = csv(b"id,value\n1,2\n2,4\n3,6\n");
    assert_eq!(source.field(), &field());
    assert_eq!(source.memory_size(), 0);
    assert_eq!(source.resident_size(), 0);
    assert!(source.clone().with_filter("missing = 2").is_err());
    assert!(source.clone().with_select("missing").is_err());
    let selected = source
        .with_filter("id >= 2")
        .unwrap()
        .with_select("value as qty")
        .unwrap()
        .with_row_range(0, Some(1))
        .unwrap();
    assert_eq!(selected.field().fields()[0].name(), "qty");
    assert_eq!(selected.memory_size(), 0);
    assert_eq!(
        selected.into_stream().unwrap().collect_rows().unwrap(),
        vec![Scalar::from_sequence([4_i64].map(Scalar::from))]
    );
}

#[test]
fn a_generic_media_serie_keeps_the_specialized_leaf_and_native_rows() {
    let source = csv(b"id,value\n1,2\n2,invalid\n");
    let generic = source.into_serie();
    assert!(matches!(&generic, yggdryl::Serie::Csv(_)));
    assert!(!generic.is_held());
    assert_eq!(generic.memory_size(), 0);
    assert_eq!(generic.field(), Some(&field()));
    let mut rows = generic.into_stream().unwrap();
    assert_eq!(
        rows.next().unwrap().unwrap(),
        Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from))
    );
}

#[test]
fn edited_media_rows_are_owned_snapshots_and_refusal_is_atomic() {
    let source = csv(b"id,value\n1,2\n2,4\n");
    let mut edited = source.clone();
    edited
        .set(0, Scalar::from_sequence([8_i64, 16_i64].map(Scalar::from)))
        .unwrap();
    assert_eq!(
        source.scalar(0).unwrap(),
        Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from))
    );
    assert_eq!(
        edited.scalar(0).unwrap(),
        Scalar::from_sequence([8_i64, 16_i64].map(Scalar::from))
    );
    assert!(
        edited
            .set(
                0,
                Scalar::from_sequence([Scalar::Null, Scalar::from(1_i64)])
            )
            .is_err()
    );
    assert_eq!(
        edited.scalar(0).unwrap(),
        Scalar::from_sequence([8_i64, 16_i64].map(Scalar::from))
    );
    let selected = edited
        .with_filter("id = 8")
        .unwrap()
        .into_stream()
        .unwrap()
        .collect_rows()
        .unwrap();
    assert_eq!(
        selected,
        vec![Scalar::from_sequence([8_i64, 16_i64].map(Scalar::from))]
    );
}

#[test]
fn shaping_a_native_row_stream_pulls_only_the_rows_its_result_needs() {
    for length in [8, 1_024] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&pulls);
        let source = yggdryl::StreamSerie::from_rows(
            field(),
            (0..length).map(move |id| {
                count.fetch_add(1, Ordering::Relaxed);
                Ok(Scalar::from_sequence([
                    Scalar::from(id as i64),
                    Scalar::from(id as i64 * 2),
                ]))
            }),
        );
        let options = CsvOptions::new()
            .with_field(field())
            .with_filter("id >= 2")
            .unwrap()
            .with_select("value as qty")
            .unwrap()
            .with_max_row_size(1);
        let mut shaped = options.apply_stream(source).unwrap();
        assert_eq!(pulls.load(Ordering::Relaxed), 0);
        assert_eq!(
            shaped.next().unwrap().unwrap(),
            Scalar::from_sequence([Scalar::from(4_i64)])
        );
        assert_eq!(pulls.load(Ordering::Relaxed), 3, "corpus {length}");
        assert!(shaped.next().is_none());
        assert_eq!(pulls.load(Ordering::Relaxed), 3);
    }
}

#[test]
fn a_media_scalar_access_does_not_hold_or_refuse_later_rows() {
    let source = csv(b"id,value\n1,2\n2,invalid\n");
    assert_eq!(
        source.scalar(0).unwrap(),
        Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from))
    );
    assert_eq!(source.memory_size(), 0);
    let generic = source.into_serie();
    assert_eq!(
        generic.scalar(0).unwrap(),
        Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from))
    );
    assert!(!generic.is_held());
}

#[test]
fn key_only_reads_skip_payload_cell_decoding_at_both_corpus_sizes() {
    for length in [8, 1_024] {
        let mut bytes = String::from("id,value\n");
        for id in 0..length {
            bytes.push_str(&format!("{id},invalid\n"));
        }
        let keys = csv(bytes.as_bytes())
            .key_values("id")
            .unwrap()
            .collect_rows()
            .unwrap();
        assert_eq!(keys.len(), length);
        assert_eq!(
            keys[length - 1],
            Scalar::from_sequence([Scalar::from((length - 1) as i64)])
        );
    }
}

#[test]
fn the_media_field_is_the_native_stream_field() {
    let source = csv(b"id,value\n1,2\n").with_select("value as qty").unwrap();
    let expected = source.field().clone();
    assert_eq!(source.into_stream().unwrap().field(), &expected);
}

#[test]
fn generic_mutation_retains_the_specialized_media_snapshot() {
    let mut source = csv(b"id,value\n1,2\n").into_serie();
    source
        .push(Scalar::from_sequence([2_i64, 4_i64].map(Scalar::from)))
        .unwrap();
    assert!(matches!(source, yggdryl::Serie::Csv(_)));
    assert_eq!(source.len(), 2);
}

#[test]
fn a_specialized_media_refuses_another_encoding_before_reading_rows() {
    let other = yggdryl::ipc::Ipc::new(Buffer::new())
        .with_options(yggdryl::ipc::IpcOptions::new().with_field(field()));
    assert!(CSVSerie::new(other, None).is_err());
}

#[test]
fn a_native_media_window_opens_before_decoding_a_later_bad_row() {
    let mut windows = csv(b"id,value\n1,2\n1,invalid\n")
        .window_by("id", false)
        .unwrap();
    let first = windows.next().unwrap().unwrap();
    assert_eq!(first.key(), &Scalar::from_sequence([Scalar::from(1_i64)]));
    let mut rows = first.into_stream().unwrap();
    assert_eq!(
        rows.next().unwrap().unwrap(),
        Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from))
    );
    assert!(rows.next().unwrap().is_err());
    assert!(rows.next().is_none());
    assert!(windows.next().is_none());
}

fn avro_bytes() -> Vec<u8> {
    fn long(out: &mut Vec<u8>, value: i64) {
        let mut bits = ((value << 1) ^ (value >> 63)) as u64;
        while bits >= 128 {
            out.push(bits as u8 | 128);
            bits >>= 7;
        }
        out.push(bits as u8);
    }
    fn bytes(out: &mut Vec<u8>, value: &[u8]) {
        long(out, value.len() as i64);
        out.extend_from_slice(value);
    }
    let schema = br#"{"type":"record","name":"rows","fields":[{"name":"id","type":"long"},{"name":"value","type":"string"}]}"#;
    let mut out = b"Obj\x01".to_vec();
    long(&mut out, 2);
    bytes(&mut out, b"avro.schema");
    bytes(&mut out, schema);
    bytes(&mut out, b"avro.codec");
    bytes(&mut out, b"null");
    long(&mut out, 0);
    out.extend_from_slice(&[7; 16]);
    // Independent wire fixture: (1, "ok"), then (2, invalid UTF-8).
    long(&mut out, 2);
    bytes(&mut out, &[2, 4, b'o', b'k', 4, 2, 255]);
    out.extend_from_slice(&[7; 16]);
    out
}

#[test]
fn an_avro_row_stream_yields_its_prefix_before_a_bad_datum_in_the_same_block() {
    let out = avro_bytes();
    let source = yggdryl::avro::Avro::new(Buffer::from_bytes(out));
    let mut rows = yggdryl::IOMedia::read_serie(&source, None)
        .unwrap()
        .into_stream()
        .unwrap();
    assert_eq!(
        rows.next().unwrap().unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("ok")])
    );
    assert!(rows.next().unwrap().is_err());
    assert!(rows.next().is_none());
    assert!(rows.next().is_none());
}

struct RowMedia {
    handle: Buffer,
    pulls: Arc<AtomicUsize>,
    length: usize,
}
impl yggdryl::IOMedia for RowMedia {
    fn as_io_base(&self) -> &dyn yggdryl::IOBase {
        self
    }
    fn as_io_base_mut(&mut self) -> &mut dyn yggdryl::IOBase {
        self
    }
    fn record_options(&self) -> yggdryl::Result<yggdryl::media::RecordOptions> {
        Ok(CsvOptions::new().with_field(field()).into())
    }
    fn read_arrow_field(&self, _: &yggdryl::media::RecordOptions) -> yggdryl::Result<Field> {
        Ok(field())
    }
    fn read_serie(
        &self,
        _: Option<&yggdryl::media::RecordOptions>,
    ) -> yggdryl::Result<yggdryl::Serie> {
        let count = Arc::clone(&self.pulls);
        Ok(yggdryl::Serie::from(yggdryl::StreamSerie::from_rows(
            field(),
            (0..self.length).map(move |id| {
                count.fetch_add(1, Ordering::Relaxed);
                Ok(Scalar::from_sequence([
                    Scalar::from((id / 3) as i64),
                    Scalar::from(id as i64),
                ]))
            }),
        )))
    }
    fn overwrite_serie(
        &mut self,
        _: yggdryl::Serie,
        _: Option<&yggdryl::media::RecordOptions>,
    ) -> yggdryl::Result<yggdryl::IOResult> {
        unreachable!("a read-only test medium")
    }
}
impl yggdryl::IOBase for RowMedia {
    yggdryl::delegate_iobase!(handle);
}

#[test]
fn native_media_windows_and_key_row_conversion_pull_one_row_at_a_time() {
    for length in [8, 1_024] {
        let count = Arc::new(AtomicUsize::new(0));
        let media = RowMedia {
            handle: Buffer::new(),
            pulls: Arc::clone(&count),
            length,
        };
        let source = yggdryl::media::GenericMediaSerie::new(media, None).unwrap();
        let mut windows = source.window_by("id", false).unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        let first = windows.next().unwrap().unwrap();
        assert_eq!(
            count.load(Ordering::Relaxed),
            1,
            "opening a window, corpus {length}"
        );
        let mut rows = first.into_stream().unwrap();
        assert_eq!(
            rows.next().unwrap().unwrap(),
            Scalar::from_sequence([0_i64, 0_i64].map(Scalar::from))
        );
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert_eq!(
            rows.next().unwrap().unwrap(),
            Scalar::from_sequence([0_i64, 1_i64].map(Scalar::from))
        );
        assert_eq!(count.load(Ordering::Relaxed), 2);
        assert_eq!(rows.count(), 1);
        assert_eq!(
            count.load(Ordering::Relaxed),
            4,
            "one lookahead at the key edge"
        );
        assert_eq!(
            windows.next().unwrap().unwrap().key(),
            &Scalar::from_sequence([Scalar::from(1_i64)])
        );
    }
}

#[test]
fn native_media_partitions_close_without_prebatching_later_keys() {
    for length in [8, 1_024] {
        let count = Arc::new(AtomicUsize::new(0));
        let media = RowMedia {
            handle: Buffer::new(),
            pulls: Arc::clone(&count),
            length,
        };
        let source = yggdryl::media::GenericMediaSerie::new(media, None).unwrap();
        let mut groups = source
            .partition_by("id", yggdryl::PartitionOptions::new().with_max_open(1))
            .unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        let first = groups.next().unwrap().unwrap();
        assert_eq!(first.key(), &Scalar::from_sequence([Scalar::from(0_i64)]));
        assert_eq!(count.load(Ordering::Relaxed), 4, "corpus {length}");
        assert_eq!(first.into_stream().unwrap().count(), 3);
        assert_eq!(count.load(Ordering::Relaxed), 4);
    }
}

#[test]
fn a_constant_projection_skips_all_csv_payload_columns() {
    let mut rows = csv(b"id,value\ninvalid,invalid\n")
        .with_select("1 as k")
        .unwrap()
        .into_stream()
        .unwrap();
    assert_eq!(
        rows.next().unwrap().unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64)])
    );
    assert!(rows.next().is_none());
}

#[test]
fn a_constant_projection_skips_all_avro_payload_columns() {
    let source = yggdryl::avro::Avro::new(Buffer::from_bytes(avro_bytes())).with_options(
        yggdryl::avro::AvroOptions::new()
            .with_select("1 as k")
            .unwrap(),
    );
    let rows = yggdryl::IOMedia::read_serie(&source, None)
        .unwrap()
        .into_stream()
        .unwrap()
        .collect_rows()
        .unwrap();
    assert_eq!(rows, vec![Scalar::from_sequence([Scalar::from(1_i64)]); 2]);
}

#[test]
fn avro_native_rows_keep_the_declared_column_order() {
    let expected = DataType::from_str("struct<value: string not null, id: int64 not null>")
        .unwrap()
        .required_field("result");
    let source = yggdryl::avro::Avro::new(Buffer::from_bytes(avro_bytes()))
        .with_options(yggdryl::avro::AvroOptions::new().with_field(expected.clone()));
    let mut rows = yggdryl::IOMedia::read_serie(&source, None)
        .unwrap()
        .into_stream()
        .unwrap();
    assert_eq!(rows.field(), &expected);
    assert_eq!(
        rows.next().unwrap().unwrap(),
        Scalar::from_sequence([Scalar::from("ok"), Scalar::from(1_i64)])
    );
}

#[test]
fn native_row_windows_verify_the_order_the_source_declares() {
    let mut ordered = field();
    ordered
        .as_sort_mut()
        .set_by(["value".parse::<yggdryl::expression::Ordering>().unwrap()])
        .unwrap();
    let source = CSVSerie::new(
        Csv::new(Buffer::from_bytes(b"id,value\n1,2\n1,1\n".to_vec()))
            .with_options(CsvOptions::new().with_field(ordered)),
        None,
    )
    .unwrap();
    let mut windows = source.window_by("id", false).unwrap();
    let mut rows = windows.next().unwrap().unwrap().into_stream().unwrap();
    assert!(rows.next().unwrap().is_ok());
    assert!(rows.next().unwrap().is_err());
    assert!(rows.next().is_none());
    assert!(windows.next().is_none());
}

#[test]
fn a_media_can_explicitly_write_its_own_native_rows_without_locking_itself() {
    let source = csv(b"id,value\n1,2\n2,4\n");
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let input = source.clone().into_serie();
        send.send(source.write_serie(input, yggdryl::IOMode::Overwrite, None))
            .unwrap();
    });
    let result = receive
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("source opening must happen before the target write lock");
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn exact_key_predicates_keep_source_terms_casts_and_null_cells() {
    let source = csv(b"id,value\n1,invalid\n2,invalid\n");
    let selected = source
        .clone()
        .with_key(
            "id as key string not null",
            Scalar::from_sequence([Scalar::from("2")]),
        )
        .unwrap();
    assert_eq!(selected.read_options().filter().columns(), ["id"]);
    assert_eq!(
        selected.key_values("id").unwrap().collect_rows().unwrap(),
        [Scalar::from_sequence([Scalar::from(2_i64)])]
    );
    assert!(
        source
            .with_key("*", Scalar::from_sequence([Scalar::from(1_i64)]))
            .is_err()
    );

    let nullable = DataType::from_str("struct<id: int64, value: int64 not null>")
        .unwrap()
        .required_field("rows");
    let source = CSVSerie::new(
        Csv::new(Buffer::from_bytes(
            b"id,value\n,invalid\n2,invalid\n".to_vec(),
        ))
        .with_options(CsvOptions::new().with_field(nullable)),
        None,
    )
    .unwrap();
    let keys = source
        .with_key("id as key", Scalar::from_sequence([Scalar::Null]))
        .unwrap()
        .key_values("id")
        .unwrap()
        .collect_rows()
        .unwrap();
    assert_eq!(keys, [Scalar::from_sequence([Scalar::Null])]);
}

#[test]
fn specialized_narrowings_are_lazy_and_mutate_an_owned_snapshot() {
    let original = csv(b"id,value\n1,2\n");
    let mut generic = yggdryl::Serie::from(original.clone());
    assert_eq!(generic.as_csv().unwrap().memory_size(), 0);
    generic
        .get_csv_mut()
        .unwrap()
        .set(0, Scalar::from_sequence([3_i64, 6_i64].map(Scalar::from)))
        .unwrap();
    assert_eq!(
        original.scalar(0).unwrap(),
        Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from))
    );
    assert_eq!(
        generic.scalar(0).unwrap(),
        Scalar::from_sequence([3_i64, 6_i64].map(Scalar::from))
    );
    assert!(generic.as_text().is_none());
}

#[test]
fn a_public_scan_state_cannot_bypass_the_specialized_encoding_refusal() {
    let media: Box<dyn yggdryl::IOBase> = Box::new(
        yggdryl::ipc::Ipc::new(Buffer::new())
            .with_options(yggdryl::ipc::IpcOptions::new().with_field(field())),
    );
    let state = yggdryl::MediaSerieState::new(media, None).unwrap();
    assert!(CSVSerie::from_media_state(state).is_err());
}

#[test]
fn a_refused_media_write_does_not_pull_its_source() {
    let count = Arc::new(AtomicUsize::new(0));
    let observe = Arc::clone(&count);
    let input = yggdryl::StreamSerie::from_rows(
        field(),
        std::iter::once_with(move || {
            observe.fetch_add(1, Ordering::Relaxed);
            Ok(Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from)))
        }),
    );
    assert!(
        csv(b"id,value\n")
            .write_serie(yggdryl::Serie::from(input), yggdryl::IOMode::Random, None)
            .is_err()
    );
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

#[test]
fn explicit_native_row_batch_bounds_are_kept_by_the_arrow_adapter() {
    let media = Csv::new(Buffer::from_bytes(
        b"id,value\n1,2\n2,4\n3,6\n4,8\n5,10\n".to_vec(),
    ));
    let options = CsvOptions::new()
        .with_field(field())
        .with_batch_row_size(2)
        .into();
    let reader = yggdryl::IOMedia::read_arrow_reader(&media, &options).unwrap();
    let lengths = reader
        .map(|batch| batch.unwrap().num_rows())
        .collect::<Vec<_>>();
    assert_eq!(lengths, [2, 2, 1]);
}

#[test]
fn a_coded_text_read_keeps_the_native_row_kind() {
    let bytes = yggdryl::Codec::Gzip.dump(b"first\nsecond\n").unwrap();
    let coded = yggdryl::coding::Coding::new(Buffer::from_bytes(bytes), yggdryl::Codec::Gzip);
    let options = yggdryl::text::TextOptions::new().into();
    let rows = yggdryl::IOMedia::read_serie(&coded, Some(&options)).unwrap();
    assert!(matches!(rows, yggdryl::Serie::Stream(_)));
}

#[test]
fn an_iomedia_csv_read_keeps_the_native_row_kind() {
    let media = Csv::new(Buffer::from_bytes(b"id,value\n1,2\n2,invalid\n".to_vec()))
        .with_options(CsvOptions::new().with_field(field()));
    let rows = yggdryl::IOMedia::read_serie(&media, None).unwrap();
    assert!(matches!(rows, yggdryl::Serie::Stream(_)));
    let mut rows = rows.into_stream().unwrap();
    assert!(rows.next().unwrap().is_ok());
    assert!(rows.next().unwrap().is_err());
}

#[test]
fn a_sliced_media_snapshot_publishes_its_current_write_field() {
    let snapshot = csv(b"id,value\n1,2\n2,4\n")
        .with_select("id")
        .unwrap()
        .slice(0, 1)
        .unwrap();
    assert_eq!(
        snapshot.read_options().field().as_ref(),
        Some(snapshot.field())
    );
}

#[test]
fn native_key_pruning_composes_with_a_filter_on_selected_aliases() {
    let source = csv(b"id,value\n1,2\n2,4\n3,6\n")
        .with_select("id as key, value")
        .unwrap()
        .with_filter("key > 1")
        .unwrap()
        .with_key("id", Scalar::from_sequence([Scalar::from(2_i64)]))
        .unwrap();
    assert_eq!(
        source.into_stream().unwrap().collect_rows().unwrap(),
        [Scalar::from_sequence([2_i64, 4_i64].map(Scalar::from))]
    );
}

#[test]
fn text_key_projection_skips_unreadable_unused_event_facts() {
    let mut options = yggdryl::text::TextOptions::new();
    options.set_rowheader(Some(r"^(?P<mtime>\S+)\s+")).unwrap();
    let source = yggdryl::text::TextSerie::new(
        yggdryl::text::Text::new(Buffer::from_bytes(b"not-a-date payload\n".to_vec()))
            .with_options(options),
        None,
    )
    .unwrap();
    let rows = source.key_values("body").unwrap().collect_rows().unwrap();
    assert_eq!(rows, [Scalar::from_sequence([Scalar::from("payload")])]);
}

#[test]
fn ipc_native_rows_keep_the_published_media_root_name() {
    let mut media = yggdryl::ipc::Ipc::new(Buffer::new())
        .with_options(yggdryl::ipc::IpcOptions::new().with_name("scan"));
    yggdryl::IOMedia::overwrite_serie(
        &mut media,
        yggdryl::Serie::from_scalars(
            field(),
            [Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from))],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let source = yggdryl::ipc::IpcSerie::new(media, None).unwrap();
    let published = source.field().clone();
    assert_eq!(published.name(), "scan");
    let stream = source.into_stream().unwrap();
    assert_eq!(stream.field(), &published);
}

#[cfg(feature = "parquet")]
#[test]
fn parquet_native_rows_keep_the_published_media_root_name() {
    let mut media = yggdryl::parquet::Parquet::new(Buffer::new())
        .with_options(yggdryl::parquet::ParquetOptions::new().with_name("scan"));
    yggdryl::IOMedia::overwrite_serie(
        &mut media,
        yggdryl::Serie::from_scalars(
            field(),
            [Scalar::from_sequence([1_i64, 2_i64].map(Scalar::from))],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let source = yggdryl::parquet::ParquetSerie::new(media, None).unwrap();
    let published = source.field().clone();
    assert_eq!(published.name(), "scan");
    assert_eq!(source.into_stream().unwrap().field(), &published);
}
