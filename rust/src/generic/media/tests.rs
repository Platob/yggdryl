//! One enum over every record encoding, exercised through one surface.

use arrow_array::{Int64Array, RecordBatch, StringArray};
use std::sync::Arc;

use super::Media;
use crate::buffered::BufferedOptions;
use crate::generic::{Holder, IORecordOptions, RecordOptions};
use crate::io::{Buffer, IOBase, IOMedia};
use crate::{DataType, Field, MediaType, MimeType, Url};

/// A struct field is the schema of the batches it describes.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row")
}

/// Two rows as one Arrow batch.
fn batch() -> RecordBatch {
    let arrow = crate::arrow::arrow_schema_from_field(&schema()).unwrap();
    RecordBatch::try_new(
        arrow,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )
    .unwrap()
}

/// The batches a write takes: one reader over one two-row batch.
fn reader() -> crate::arrow::BatchReader {
    crate::arrow::batch_reader(
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        [batch()],
    )
}

/// A holder whose media type comes from a name, so the encoding is declared.
fn handle(name: &str) -> Holder {
    Holder::buffer(
        Buffer::new().with_media_type(
            Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        ),
    )
}

#[test]
fn the_name_picks_the_implementation() {
    assert!(matches!(
        Media::open(handle("trades.arrows")).unwrap(),
        Media::Ipc(_)
    ));
    #[cfg(feature = "parquet")]
    assert!(matches!(
        Media::open(handle("trades.parquet")).unwrap(),
        Media::Parquet(_)
    ));
}

#[test]
fn each_explicit_variant_owns_options_over_an_unnamed_buffer() {
    let ipc = Media::ipc(Holder::buffer(Buffer::new())).with_field(schema());
    let options = ipc.record_options().unwrap();
    assert!(matches!(options, RecordOptions::Ipc(_)));
    assert_eq!(options.field(), Some(&schema()));

    let avro = Media::avro(Holder::buffer(Buffer::new())).with_field(schema());
    assert!(matches!(
        avro.record_options().unwrap(),
        RecordOptions::Avro(_)
    ));

    #[cfg(feature = "parquet")]
    {
        let parquet = Media::parquet(Holder::buffer(Buffer::new())).with_field(schema());
        assert!(matches!(
            parquet.record_options().unwrap(),
            RecordOptions::Parquet(_)
        ));
    }

    let text = Media::text(Holder::buffer(Buffer::new()));
    assert!(matches!(
        text.record_options().unwrap(),
        RecordOptions::Text(_)
    ));
}

#[test]
fn an_unimplemented_encoding_is_named_rather_than_guessed() {
    let message = Media::open(handle("trades.csv")).unwrap_err().to_string();
    assert!(message.contains("text/csv"), "{message}");
}

#[test]
fn every_variant_round_trips_batches_through_the_same_calls() {
    let mut names = vec!["trades.arrows", "trades.arrows.gz"];
    if cfg!(feature = "parquet") {
        names.push("trades.parquet");
    }

    for name in names {
        let mut media = Media::open(handle(name)).unwrap().with_field(schema());
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(reader(), &options)
            .unwrap_or_else(|error| panic!("{name}: {error}"));

        let rows = media
            .read_arrow_reader(&options)
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>();
        assert_eq!(rows, 2, "{name}");
        assert_eq!(
            media.read_arrow_field(&options).unwrap(),
            schema(),
            "{name}"
        );
    }
}

#[test]
fn generic_media_preserves_commit_cadence_through_variant_redirection() {
    let mut names = vec!["committed.arrows", "committed.avro"];
    if cfg!(feature = "parquet") {
        names.push("committed.parquet");
    }

    for name in names {
        let mut media = Media::open(handle(name)).unwrap().with_field(schema());
        match &mut media {
            Media::Ipc(ipc) => ipc.options_mut().set_commit_row_size(Some(1)),
            #[cfg(feature = "parquet")]
            Media::Parquet(parquet) => parquet.options_mut().set_commit_row_size(Some(1)),
            Media::Avro(avro) => avro.options_mut().set_commit_row_size(Some(1)),
            Media::Text(_) => unreachable!("the fixture names a binary record encoding"),
        }

        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(reader(), &options)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            media
                .read_arrow_reader(&options)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            2,
            "{name}"
        );
        assert_eq!(
            media.read_arrow_field(&options).unwrap(),
            schema(),
            "{name}"
        );
    }
}

#[test]
fn media_mirrors_the_bytes_of_its_handle() {
    let mut media = Media::open(handle("trades.arrows"))
        .unwrap()
        .with_field(schema());
    let options = media.record_options().unwrap();
    media.overwrite_arrow_reader(reader(), &options).unwrap();

    // An Arrow IPC stream opens with the continuation marker.
    assert_eq!(media.read_range(0, 4).unwrap(), [0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(media.size(), media.read_all_bytes().unwrap().len() as u64);
    assert_eq!(media.media_type().base(), &MimeType::ARROW_STREAM);
}

#[test]
fn a_compressed_handle_mirrors_the_compressed_bytes() {
    let mut media = Media::open(handle("trades.arrows.gz"))
        .unwrap()
        .with_field(schema());
    let options = media.record_options().unwrap();
    media.overwrite_arrow_reader(reader(), &options).unwrap();

    assert_eq!(media.read_range(0, 2).unwrap(), [0x1F, 0x8B]);
}

#[test]
fn open_and_close_carry_the_cached_schema() {
    let mut media = Media::open(handle("trades.arrows"))
        .unwrap()
        .with_field(schema());
    let options = media.record_options().unwrap();
    media.overwrite_arrow_reader(reader(), &options).unwrap();

    // A reader with no declared schema recovers it once and remembers it.
    let mut reader = Media::ipc(Holder::buffer(Buffer::from_bytes(
        media.read_all_bytes().unwrap(),
    )));
    assert!(!reader.opened());
    reader.open().unwrap();
    assert!(reader.opened());
    let options = reader.record_options().unwrap();
    assert_eq!(reader.read_arrow_field(&options).unwrap(), schema());

    reader.close().unwrap();
    assert!(!reader.opened());
    // Still usable after closing; it simply re-derives.
    assert_eq!(reader.read_arrow_reader(&options).unwrap().count(), 1);
}

#[test]
fn holder_open_retains_the_inferred_media_cache_without_nesting_it() {
    let mut source = Media::open(handle("retained.arrows"))
        .unwrap()
        .with_field(schema());
    let options = source.record_options().unwrap();
    source.overwrite_arrow_reader(reader(), &options).unwrap();

    // A Buffer itself has no opened state. Opening the Holder promotes it to
    // the inferred IPC implementation, whose opened session owns the cached
    // schema and dimensions for subsequent binding calls.
    let buffer = Buffer::from_bytes(source.read_all_bytes().unwrap())
        .with_media_type(MediaType::from(MimeType::ARROW_STREAM));
    let mut holder = Holder::buffer(buffer);
    assert!(!holder.opened());
    holder.open().unwrap();
    assert!(holder.opened());
    assert_eq!(holder.row_size().unwrap(), 2);
    assert_eq!(holder.column_size().unwrap(), 2);
    let options = holder.record_options().unwrap();
    assert_eq!(holder.read_arrow_field(&options).unwrap(), schema());

    // Reopening is idempotent: the retained Media remains one IPC wrapper
    // whose byte handle is the original Buffer, not another Media.
    holder.open().unwrap();
    match &holder {
        Holder::Media(media) => match media.as_ref() {
            Media::Ipc(ipc) => assert!(matches!(ipc.handle(), Holder::Buffer(_))),
            other => panic!("expected retained IPC media, got {other:?}"),
        },
        other => panic!("expected a retained media holder, got {other:?}"),
    }

    holder.close().unwrap();
    assert!(!holder.opened());
    assert!(matches!(holder.into_media(), Holder::Media(_)));
}

#[test]
fn holder_media_promotion_preserves_wrapper_idempotence_and_plain_bytes() {
    let options = BufferedOptions::default().with_page_size(256);
    let promoted = handle("retained.arrows")
        .buffered(options)
        .into_media()
        .into_media()
        .buffered(options);
    match &promoted {
        Holder::Buffered(buffered) => assert!(matches!(
            buffered.handle(),
            Holder::Media(media) if matches!(media.as_ref(), Media::Ipc(_))
        )),
        other => panic!("expected one outer page cache, got {other:?}"),
    }

    let text = handle("retained.txt").into_media().into_media();
    assert!(matches!(text, Holder::Text(_)));

    // Structured text codecs are atomic Scalar documents, not row media. The
    // best-fitting conversion therefore leaves them as ordinary bytes, and
    // opening them cannot manufacture an unsupported-media error.
    let mut json = Holder::buffer(
        Buffer::from_bytes(br#"{"id":1}"#.to_vec())
            .with_media_type(MediaType::from(MimeType::JSON)),
    )
    .into_media();
    assert!(matches!(&json, Holder::Buffer(_)));
    json.open().unwrap();
    assert!(matches!(json, Holder::Buffer(_)));
}

#[test]
fn a_missing_resource_reads_as_empty_rather_than_failing() {
    let media = Media::ipc(Holder::buffer(Buffer::new())).with_field(schema());
    let options = media.record_options().unwrap();
    assert_eq!(media.read_arrow_reader(&options).unwrap().count(), 0);
}
