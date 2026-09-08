//! A coded handle presents the decoded bytes and stores the encoded ones.

use super::Coding;
use crate::holder::Buffer;
use crate::{Codec, Level, MimeType, Url};
use crate::{IOBase, IOMedia};

#[derive(Debug)]
struct SharedReads {
    handle: Buffer,
    reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl SharedReads {
    fn new(bytes: Vec<u8>) -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (
            Self {
                handle: Buffer::from_bytes(bytes),
                reads: std::sync::Arc::clone(&reads),
            },
            reads,
        )
    }
}

impl crate::IOMedia for SharedReads {
    crate::impl_default_iomedia!();
}

impl IOBase for SharedReads {
    crate::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate, url, media_type,
        set_media_type, flush, parent, child_by_path, ls, kind, clear, remove, is_atomic,
        is_tabular, is_io);

    fn pread(&self, offset: u64, target: &mut [u8]) -> crate::Result<usize> {
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.handle.pread(offset, target)
    }
}

/// Long enough that framing overhead cannot hide the compression.
const PAYLOAD: &[u8] = b"symbol,price
AAPL,1
AAPL,2
AAPL,3
AAPL,4
AAPL,5
AAPL,6
AAPL,7
AAPL,8
AAPL,9
AAPL,10
AAPL,11
AAPL,12
AAPL,13
AAPL,14
AAPL,15
AAPL,16
AAPL,17
AAPL,18
AAPL,19
AAPL,20
AAPL,21
AAPL,22
AAPL,23
AAPL,24
";

#[test]
fn every_coding_round_trips_through_the_handle() {
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Deflate, Codec::Zstd] {
        let mut handle = Coding::new(Buffer::new(), codec);
        handle.write_all_bytes(PAYLOAD).unwrap();
        handle.flush().unwrap();

        assert_eq!(handle.read_all_bytes().unwrap(), PAYLOAD, "{codec}");
        assert_eq!(handle.size(), PAYLOAD.len() as u64, "{codec}");
        // The wrapped handle holds the encoded form, which is not the payload.
        assert_ne!(handle.handle().as_slice(), PAYLOAD, "{codec}");
    }
}

#[test]
fn an_identity_coding_is_a_pass_through() {
    let mut handle = Coding::new(Buffer::new(), Codec::Identity);
    handle.write_all_bytes(PAYLOAD).unwrap();
    handle.flush().unwrap();

    assert_eq!(handle.handle().as_slice(), PAYLOAD);
}

#[test]
fn positional_reads_and_writes_address_the_decoded_value() {
    let mut handle = Coding::new(Buffer::new(), Codec::Gzip);
    handle.write_all_bytes(PAYLOAD).unwrap();

    let mut head = [0_u8; 6];
    handle.pread(0, &mut head).unwrap();
    assert_eq!(&head, b"symbol");

    // Overwriting in place changes only those bytes.
    handle.pwrite(0, b"ticker").unwrap();
    assert_eq!(&handle.read_range_bytes(0, 6).unwrap(), b"ticker");
    assert_eq!(handle.size(), PAYLOAD.len() as u64);

    // A read past the end is empty rather than an error.
    let mut past = [0_u8; 4];
    assert_eq!(handle.pread(handle.size() + 10, &mut past).unwrap(), 0);
}

#[test]
fn the_media_type_reported_is_the_decoded_one() {
    let inner = Buffer::new().with_media_type(
        Url::from_str("file:///trades.arrows.gz")
            .unwrap()
            .media_type(),
    );
    assert_eq!(inner.media_type().encoding_len(), 1);

    let handle = Coding::new(inner, Codec::Gzip);
    // Wrapping removes the coding, because the wrapper's bytes are decoded.
    assert_eq!(handle.media_type().base(), &MimeType::ARROW_STREAM);
    assert_eq!(handle.media_type().encoding_len(), 0);
    assert!(handle.codec().is_identity() || handle.codec() == Codec::Gzip);
}

#[test]
fn a_missing_resource_reads_as_empty_rather_than_failing() {
    let handle = Coding::new(Buffer::new(), Codec::Zstd);
    assert_eq!(handle.size(), 0);
    assert!(handle.read_all_bytes().unwrap().is_empty());
}

#[test]
fn open_materializes_and_close_publishes() {
    let mut handle = Coding::new(Buffer::new(), Codec::Gzip);
    assert!(!handle.opened());

    handle.open().unwrap();
    assert!(handle.opened());

    handle.write_all_bytes(PAYLOAD).unwrap();
    handle.close().unwrap();
    assert!(!handle.opened());

    // Closing published the write, so the encoded bytes are there.
    assert_eq!(handle.read_all_bytes().unwrap(), PAYLOAD);
}

#[test]
fn a_higher_level_does_not_change_what_is_read_back() {
    let mut best = Coding::new(Buffer::new(), Codec::Zstd).with_level(Level::BEST);
    let mut fast = Coding::new(Buffer::new(), Codec::Zstd).with_level(Level::FAST);

    best.write_all_bytes(PAYLOAD).unwrap();
    fast.write_all_bytes(PAYLOAD).unwrap();
    best.flush().unwrap();
    fast.flush().unwrap();

    assert_eq!(best.read_all_bytes().unwrap(), PAYLOAD);
    assert_eq!(fast.read_all_bytes().unwrap(), PAYLOAD);
    assert!(best.handle().size() < PAYLOAD.len() as u64);
}

#[test]
fn truncation_shrinks_and_grows_the_decoded_value() {
    let mut handle = Coding::new(Buffer::new(), Codec::Zlib);
    handle.write_all_bytes(PAYLOAD).unwrap();

    handle.truncate(6).unwrap();
    assert_eq!(handle.read_all_bytes().unwrap(), b"symbol");

    // Growing zero-fills, exactly as a positional write past the end does.
    handle.truncate(8).unwrap();
    assert_eq!(handle.read_all_bytes().unwrap(), b"symbol\0\0");
}

#[test]
fn an_open_handle_answers_reads_out_of_what_it_holds() {
    use crate::holder::buffered::tests::Counting;

    let mut source = Coding::new(Buffer::new(), Codec::Gzip);
    source.write_all_bytes(PAYLOAD).unwrap();
    source.flush().unwrap();
    let encoded = source.into_handle().unwrap().into_bytes();

    let mut handle = Coding::new(Counting::from_bytes(encoded), Codec::Gzip);
    handle.open().unwrap();
    let reads = handle.handle().reads();
    let sizes = handle.handle().sizes();

    // Between `open` and `close` the decoded value is held, so a positional
    // read is a range copy out of it: it reaches the wrapped handle for
    // nothing, and it does not copy the whole payload to serve four bytes.
    for offset in [0, 7, 32, PAYLOAD.len() as u64 - 4] {
        let mut target = [0_u8; 4];
        assert_eq!(handle.pread(offset, &mut target).unwrap(), 4);
        let at = offset as usize;
        assert_eq!(&target, &PAYLOAD[at..at + 4]);
    }
    assert_eq!(handle.size(), PAYLOAD.len() as u64);
    assert_eq!(
        handle.handle().reads(),
        reads,
        "an open handle re-reads nothing"
    );
    assert_eq!(handle.handle().sizes(), sizes, "nor re-measures anything");

    // Closing releases it, and the handle keeps working by decoding again.
    handle.close().unwrap();
    let mut head = [0_u8; 6];
    assert_eq!(handle.pread(0, &mut head).unwrap(), 6);
    assert_eq!(&head, &PAYLOAD[..6]);
    assert!(handle.handle().reads() > reads);
}

#[test]
fn closed_codings_stream_bounded_chunks_from_decoded_offsets() {
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let encoded = codec.dump(PAYLOAD).unwrap();
        let handle = Coding::new(Buffer::from_bytes(encoded), codec);

        let chunks = handle
            .pstream_bytes(7, 13)
            .unwrap()
            .collect::<crate::Result<Vec<_>>>()
            .unwrap();
        assert!(
            chunks
                .iter()
                .all(|chunk| !chunk.is_empty() && chunk.len() <= 13),
            "{codec} exceeded the requested chunk bound"
        );
        assert!(
            chunks
                .iter()
                .take(chunks.len().saturating_sub(1))
                .all(|chunk| chunk.len() == 13),
            "{codec} returned a short non-final chunk"
        );
        assert_eq!(chunks.concat(), PAYLOAD[7..], "{codec}");
        assert!(
            handle
                .pstream_bytes(PAYLOAD.len() as u64 + 1, 13)
                .unwrap()
                .next()
                .is_none(),
            "{codec} did not end cleanly past the decoded value"
        );
        assert!(!handle.opened(), "{codec} materialized a closed stream");
    }
}

#[test]
fn compressed_headers_and_trailers_may_cross_source_chunks() {
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let encoded = codec.dump(PAYLOAD).unwrap();
        let handle = Coding::new(Buffer::from_bytes(encoded), codec);
        let decoded = handle
            .pstream_bytes(0, 1)
            .unwrap()
            .collect::<crate::Result<Vec<_>>>()
            .unwrap();

        assert!(decoded.iter().all(|chunk| chunk.len() == 1), "{codec}");
        assert_eq!(decoded.concat(), PAYLOAD, "{codec}");
        assert!(!handle.opened(), "{codec} materialized while streaming");
    }
}

#[test]
fn one_byte_decoded_chunks_keep_a_bounded_encoded_transport_window() {
    use crate::holder::buffered::tests::Counting;

    // Deliberately incompressible enough to span many transport reads. The
    // regression was one `pread` per encoded byte when output batches were 1.
    let payload: Vec<u8> = (0..256 * 1024)
        .map(|index| ((index * 131 + index / 251) & 0xff) as u8)
        .collect();
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let encoded = codec.dump(&payload).unwrap();
        let handle = Coding::new(Counting::from_bytes(encoded.clone()), codec);
        let first = handle.pstream_bytes(0, 1).unwrap().next().unwrap().unwrap();

        assert_eq!(first, payload[..1], "{codec}");
        assert!(
            handle.handle().reads() <= 4,
            "{codec} used {} source reads for its first decoded byte",
            handle.handle().reads()
        );
        assert!(
            handle.handle().reads() < encoded.len(),
            "{codec} coupled decoded and encoded chunk sizes"
        );
    }
}

#[test]
fn a_closed_stream_is_lazy_and_never_measures_or_materializes() {
    use crate::holder::buffered::tests::Counting;

    let payload = PAYLOAD.repeat(64);
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let encoded = codec.dump(&payload).unwrap();
        let handle = Coding::new(Counting::from_bytes(encoded), codec);
        let mut stream = handle.pstream_bytes(5, 17).unwrap();

        assert_eq!(handle.handle().reads(), 0, "{codec} read at construction");
        assert_eq!(
            handle.handle().sizes(),
            0,
            "{codec} measured at construction"
        );
        assert_eq!(stream.next().unwrap().unwrap(), payload[5..22], "{codec}");
        drop(stream);

        assert!(
            handle.handle().reads() > 0,
            "{codec} never pulled its source"
        );
        assert_eq!(handle.handle().sizes(), 0, "{codec} measured its source");
        assert!(!handle.opened(), "{codec} retained the decoded value");

        assert_eq!(
            handle.read_range_bytes(9, 31).unwrap(),
            payload[9..40],
            "{codec}"
        );
        assert_eq!(handle.read_all_bytes().unwrap(), payload, "{codec}");
        assert_eq!(handle.handle().sizes(), 0, "{codec} measured a helper read");
        assert!(!handle.opened(), "{codec} cached a helper read");
    }
}

#[test]
fn closed_positional_reads_decode_only_the_requested_prefix() {
    use crate::holder::buffered::tests::Counting;

    let mut state = 0xA537_1D09_u32;
    let payload: Vec<u8> = (0..4 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let encoded = codec.dump(&payload).unwrap();
        let handle = Coding::new(Counting::from_bytes(encoded.clone()), codec);
        let mut first = [0_u8; 4096];

        assert_eq!(handle.pread(0, &mut first).unwrap(), first.len(), "{codec}");
        assert_eq!(first, payload[..first.len()], "{codec}");
        assert!(!handle.opened(), "{codec} retained a decoded payload");
        assert!(
            handle.handle().reads() < encoded.len().div_ceil(8 * 1024) / 4,
            "{codec} drained the complete encoded source for a small prefix"
        );
    }
}

#[test]
fn closed_size_counts_through_a_stream_without_opening() {
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let payload = PAYLOAD.repeat(1_024);
        let handle = Coding::new(Buffer::from_bytes(codec.dump(&payload).unwrap()), codec);

        assert_eq!(handle.size(), payload.len() as u64, "{codec}");
        assert!(
            !handle.opened(),
            "{codec} retained decoded bytes while sizing"
        );
    }
}

#[test]
fn boxed_coding_helpers_keep_the_native_single_stream_path() {
    let payload = PAYLOAD.repeat(8 * 1024);
    let encoded = Codec::Gzip.dump(&payload).unwrap();

    let (direct_source, direct_reads) = SharedReads::new(encoded.clone());
    let direct = crate::coding::gzip::Gzip::new(direct_source);
    assert_eq!(direct.read_all_bytes().unwrap(), payload);
    let direct_reads = direct_reads.load(std::sync::atomic::Ordering::Relaxed);

    let (boxed_source, boxed_reads) = SharedReads::new(encoded);
    let boxed_inner = crate::coding::gzip::Gzip::new(boxed_source);
    let boxed: Box<dyn IOBase> = Box::new(boxed_inner);
    assert_eq!(boxed.read_all_bytes().unwrap(), payload);
    assert!(direct_reads > 0);
    assert_eq!(
        boxed_reads.load(std::sync::atomic::Ordering::Relaxed),
        direct_reads,
        "boxing rebuilt the decoder instead of forwarding the optimized helper"
    );
}

#[cfg(feature = "arrow")]
#[test]
fn a_coded_ipc_view_streams_through_its_owning_reader() {
    use std::sync::Arc;

    use crate::media::{IORecordOptions, RecordOptions};
    use crate::{DataType, MimeType};
    use arrow_array::{Int64Array, RecordBatch};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    let schema = field.clone().into_arrow_schema().unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
    )
    .unwrap();
    let mut plain = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    crate::media::ipc::overwrite_arrow_reader(
        &mut plain,
        crate::arrow::batch_reader(schema, [batch]),
        &crate::media::ipc::IpcOptions::new(),
    )
    .unwrap();

    let encoded = Codec::Gzip.dump(plain.as_slice()).unwrap();
    let inner = Buffer::from_bytes(encoded).with_media_type(
        Url::from_str("file:///rows.arrows.gz")
            .unwrap()
            .media_type(),
    );
    let coded = Coding::new(inner, Codec::Gzip);
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)
        .unwrap()
        .with_field(field);
    let rows: usize = coded
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();

    assert_eq!(rows, 3);
    assert!(!coded.opened());
}

#[test]
fn an_open_stream_reads_only_its_decoded_snapshot() {
    use crate::holder::buffered::tests::Counting;

    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let encoded = codec.dump(PAYLOAD).unwrap();
        let mut handle = Coding::new(Counting::from_bytes(encoded), codec);
        handle.open().unwrap();
        let reads = handle.handle().reads();
        let sizes = handle.handle().sizes();

        let chunks = handle
            .pstream_bytes(11, 19)
            .unwrap()
            .collect::<crate::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(chunks.concat(), PAYLOAD[11..], "{codec}");
        assert_eq!(handle.handle().reads(), reads, "{codec} re-read its source");
        assert_eq!(
            handle.handle().sizes(),
            sizes,
            "{codec} re-measured its source"
        );
    }
}

#[test]
fn empty_and_invalid_closed_streams_have_stable_end_states() {
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let empty = Coding::new(Buffer::new(), codec);
        let mut stream = empty.pstream_bytes(0, 3).unwrap();
        assert!(
            stream.next().is_none(),
            "{codec} rejected an absent resource"
        );
        assert!(stream.next().is_none(), "{codec} did not fuse at empty EOF");

        // Invalid framing is portable across decoder implementations, while
        // some DEFLATE readers deliberately accept an omitted checksum at EOF.
        let corrupt = Coding::new(Buffer::from_bytes(vec![0xA5; 32]), codec);
        let mut stream = corrupt.pstream_bytes(0, 7).unwrap();
        let mut failed = false;
        for item in &mut stream {
            if item.is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed, "{codec} accepted an invalid header");
        assert!(stream.next().is_none(), "{codec} yielded after an error");
        assert!(
            stream.next().is_none(),
            "{codec} did not fuse after an error"
        );
        assert!(!corrupt.opened(), "{codec} cached a failed decode");
    }
}

mod dispatched {
    use super::super::Coded;
    use crate::holder::Buffer;
    use crate::{Codec, IOBase, Level, Url};

    const PAYLOAD: &[u8] = b"symbol,price\nAAPL,1\nAAPL,2\nAAPL,3\nAAPL,4\nAAPL,5\nAAPL,6\n\
    AAPL,7\nAAPL,8\nAAPL,9\nAAPL,10\nAAPL,11\nAAPL,12\n";

    #[test]
    fn every_coding_round_trips_through_the_enum() {
        for codec in Codec::ALL {
            let mut handle = Coded::wrap(Buffer::new(), codec);
            handle.write_all_bytes(PAYLOAD).unwrap();
            handle.flush().unwrap();

            assert_eq!(handle.read_all_bytes().unwrap(), PAYLOAD, "{codec}");
            assert_eq!(handle.size(), PAYLOAD.len() as u64, "{codec}");
        }
    }

    #[test]
    fn the_handles_own_media_type_picks_the_coding() {
        let named = Buffer::new().with_media_type(
            Url::from_str("file:///trades.csv.zst")
                .unwrap()
                .media_type(),
        );
        let mut handle = Coded::infer(named);
        assert_eq!(handle.codec(), Codec::Zstd);

        handle.write_all_bytes(PAYLOAD).unwrap();
        handle.flush().unwrap();
        assert_eq!(handle.read_all_bytes().unwrap(), PAYLOAD);
        assert!(handle.handle().size() < PAYLOAD.len() as u64);
    }

    #[test]
    fn raw_deflate_wraps_as_its_framed_form() {
        let handle = Coded::wrap(Buffer::new(), Codec::Deflate);
        assert_eq!(handle.codec(), Codec::Zlib);
    }

    #[test]
    fn an_identity_coding_writes_the_payload_unchanged() {
        let mut handle = Coded::wrap(Buffer::new(), Codec::Identity);
        handle.write_all_bytes(PAYLOAD).unwrap();
        handle.flush().unwrap();

        assert_eq!(handle.handle().read_all_bytes().unwrap(), PAYLOAD);
    }

    #[test]
    fn a_level_reaches_the_encoder_and_the_handle_survives_it() {
        let mut handle = Coded::wrap(Buffer::new(), Codec::Gzip).with_level(Level::BEST);
        handle.write_all_bytes(PAYLOAD).unwrap();

        let inner = handle.into_handle().unwrap();
        let encoded = inner.read_all_bytes().unwrap();
        assert_eq!(crate::coding::gzip::load(&encoded).unwrap(), PAYLOAD);
    }

    #[test]
    fn a_missing_resource_reads_as_empty_rather_than_failing() {
        let handle = Coded::wrap(Buffer::new(), Codec::Gzip);
        assert_eq!(handle.size(), 0);
        assert!(handle.read_all_bytes().unwrap().is_empty());
    }
}

mod held {
    use crate::holder::buffered::BufferedOptions;
    use crate::holder::{Buffer, Holder};
    use crate::{Codec, IOBase, Level, MimeType, Url};

    const PLAIN: &[u8] = b"[INFO] alpha\n[WARN] beta\n";

    fn named(name: &str, bytes: Vec<u8>) -> Holder {
        Holder::buffer(
            Buffer::from_bytes(bytes).with_media_type(
                Url::from_str(&format!("file:///{name}"))
                    .unwrap()
                    .media_type(),
            ),
        )
    }

    fn compressed(name: &str, codec: Codec) -> Holder {
        named(name, codec.dump(PLAIN).unwrap())
    }

    #[test]
    fn a_held_coding_comes_from_the_name_and_presents_decoded_bytes() {
        for (name, codec) in [("app.log.gz", Codec::Gzip), ("app.log.zst", Codec::Zstd)] {
            let source = compressed(name, codec);
            assert_eq!(source.read_all_bytes().unwrap(), codec.dump(PLAIN).unwrap());

            let decoded = source.into_coded();
            assert!(matches!(decoded, Holder::Coded(_)), "{name}");
            assert_eq!(decoded.read_all_bytes().unwrap(), PLAIN, "{name}");
            assert_eq!(decoded.size(), PLAIN.len() as u64, "{name}");
            assert_eq!(decoded.media_type().base(), &MimeType::PLAIN_TEXT, "{name}");
            assert!(decoded.media_type().encodings().is_empty(), "{name}");
        }
    }

    #[test]
    fn a_name_declaring_no_coding_passes_its_bytes_through() {
        let decoded = named("app.log", PLAIN.to_vec()).into_coded();
        assert_eq!(decoded.read_all_bytes().unwrap(), PLAIN);
        assert_eq!(decoded.media_type().base(), &MimeType::PLAIN_TEXT);
    }

    #[test]
    fn repeating_the_conversion_never_decodes_twice() {
        let decoded = compressed("app.log.gz", Codec::Gzip)
            .into_coded()
            .into_coded()
            .into_coded_with(Codec::Zstd, Level::BEST);
        assert_eq!(decoded.read_all_bytes().unwrap(), PLAIN);
    }

    #[test]
    fn a_page_cache_stays_outside_the_coding() {
        let decoded = compressed("app.log.gz", Codec::Gzip)
            .buffered(BufferedOptions::default())
            .into_coded();
        match &decoded {
            Holder::Buffered(buffered) => {
                assert!(matches!(buffered.handle(), Holder::Coded(_)));
            }
            other => panic!("expected a cache outside the coding, got {other:?}"),
        }
        assert_eq!(decoded.read_all_bytes().unwrap(), PLAIN);
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn a_coded_holder_reads_its_text_records_through_the_decoded_view() {
        use crate::IOMedia as _;

        let decoded = compressed("app.log.gz", Codec::Gzip).into_coded();
        let options = decoded.record_options().unwrap();
        assert!(matches!(options, crate::media::RecordOptions::Text(_)));

        let bodies = decoded
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| {
                let batch = batch.unwrap();
                let index = batch.schema().index_of("body").unwrap();
                arrow_array::cast::as_generic_binary_array::<i32>(batch.column(index))
                    .iter()
                    .map(|value| value.unwrap().to_vec())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(bodies, [b"[INFO] alpha".to_vec(), b"[WARN] beta".to_vec()]);
        assert_eq!(decoded.row_size().unwrap(), 2);
    }
}

/// What a read costs the store underneath a coded handle.
///
/// The decoded side and the transport side are separate budgets: a stream is
/// read to its end and fetches whole windows, while a positional read wants
/// its own few bytes and must not pull a window to answer them.
mod transport {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::Coding;
    use crate::holder::Buffer;
    use crate::{Codec, DEFAULT_FETCH_BYTE_SIZE, DEFAULT_STREAM_BATCH_SIZE, IOBase, IOMedia, Url};

    /// A handle that records how many bytes each read asks it for.
    #[derive(Debug)]
    struct Asked {
        handle: Buffer,
        requests: Arc<AtomicUsize>,
        bytes: Arc<AtomicUsize>,
    }

    impl IOMedia for Asked {
        crate::impl_default_iomedia!();
    }

    impl IOBase for Asked {
        crate::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate, url,
            media_type, set_media_type, flush, parent, child_by_path, ls, kind, clear, remove,
            is_atomic, is_tabular, is_io);

        fn pread(&self, offset: u64, target: &mut [u8]) -> crate::Result<usize> {
            self.requests.fetch_add(1, Ordering::Relaxed);
            self.bytes.fetch_add(target.len(), Ordering::Relaxed);
            self.handle.pread(offset, target)
        }
    }

    /// A payload gzip cannot shrink away, larger than one fetch window.
    fn coded() -> (Coding<Asked>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut state = 0x1357_9BDF_u32;
        let plain: Vec<u8> = (0..2 * DEFAULT_FETCH_BYTE_SIZE)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                ALPHABET[state as usize % ALPHABET.len()]
            })
            .collect();
        let encoded = Codec::Gzip.dump(&plain).unwrap();
        assert!(encoded.len() > DEFAULT_FETCH_BYTE_SIZE);

        let requests = Arc::new(AtomicUsize::new(0));
        let bytes = Arc::new(AtomicUsize::new(0));
        let handle = Asked {
            handle: Buffer::from_bytes(encoded).with_media_type(
                Url::from_str("file:///payload.bin.gz")
                    .unwrap()
                    .media_type(),
            ),
            requests: Arc::clone(&requests),
            bytes: Arc::clone(&bytes),
        };
        (Coding::new(handle, Codec::Gzip), requests, bytes)
    }

    #[test]
    fn a_positional_read_fetches_what_it_needs_rather_than_a_whole_window() {
        let (coded, requests, bytes) = coded();

        let mut target = [0_u8; 8];
        assert_eq!(coded.pread(0, &mut target).unwrap(), 8);
        assert!(
            bytes.load(Ordering::Relaxed) <= DEFAULT_STREAM_BATCH_SIZE,
            "eight decoded bytes asked the store for {} encoded bytes",
            bytes.load(Ordering::Relaxed)
        );

        // A short range is the same shape, and neither read is free of the
        // decode that has to reach the offset first.
        requests.store(0, Ordering::Relaxed);
        bytes.store(0, Ordering::Relaxed);
        assert_eq!(coded.read_range_bytes(4, 12).unwrap().len(), 12);
        assert!(
            bytes.load(Ordering::Relaxed) <= DEFAULT_STREAM_BATCH_SIZE,
            "a twelve-byte range asked the store for {} encoded bytes",
            bytes.load(Ordering::Relaxed)
        );
    }

    #[test]
    fn a_stream_fetches_whole_windows() {
        let (coded, requests, bytes) = coded();

        let read = coded.read_all_bytes().unwrap();
        assert_eq!(read.len(), 2 * DEFAULT_FETCH_BYTE_SIZE);
        // The whole encoded object is read, and asking for it a window at a
        // time is what keeps the request count proportional to its size.
        assert!(
            requests.load(Ordering::Relaxed)
                <= 2 * bytes.load(Ordering::Relaxed) / DEFAULT_FETCH_BYTE_SIZE + 2,
            "{} requests for {} bytes",
            requests.load(Ordering::Relaxed),
            bytes.load(Ordering::Relaxed)
        );
    }

    #[test]
    fn a_stream_nobody_reads_touches_nothing() {
        let (coded, requests, bytes) = coded();

        let stream = coded.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE).unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 0);
        assert_eq!(bytes.load(Ordering::Relaxed), 0);
        drop(stream);
        assert_eq!(requests.load(Ordering::Relaxed), 0);
    }
}

/// Restart points: where a decoder may begin an encoded stream partway in.
mod restarts {
    use std::io::Write as _;

    use crate::{Codec, Level};

    /// Three segments of a payload, each begun at a restart point.
    fn segmented(codec: Codec) -> (Vec<u8>, Vec<Vec<u8>>) {
        let segments: Vec<Vec<u8>> = (0..3)
            .map(|part| format!("part {part}: symbol,price\nAAPL,187.23\n").repeat(64))
            .map(String::into_bytes)
            .collect();
        let mut encoded = Vec::new();
        let mut encoder = codec.writer_with_level(&mut encoded, Level::DEFAULT);
        for (index, segment) in segments.iter().enumerate() {
            if index > 0 {
                encoder.restart().expect("the coding restarts");
            }
            encoder.write_all(segment).expect("the segment encodes");
        }
        encoder.finish().expect("the stream finishes");
        (encoded, segments)
    }

    #[test]
    fn a_restarted_stream_decodes_whole_and_from_every_restart() {
        for codec in [Codec::Deflate, Codec::Zstd] {
            let (encoded, segments) = segmented(codec);
            let whole: Vec<u8> = segments.concat();
            assert_eq!(codec.load(&encoded).expect("the whole stream"), whole);

            let mut offsets = Vec::new();
            codec.restart_scan().push(&encoded, &mut offsets);
            assert_eq!(offsets.len(), 2, "{codec}");

            // Each restart begins the segment that follows it, and everything
            // after that segment comes with it.
            let mut behind = whole.len();
            for (index, offset) in offsets.iter().enumerate() {
                let start = usize::try_from(*offset).expect("an in-memory offset");
                let decoded = codec.load(&encoded[start..]).expect("the tail decodes");
                let expected: Vec<u8> = segments[index + 1..].concat();
                assert_eq!(decoded, expected, "{codec} restart {index}");
                assert!(decoded.len() < behind);
                behind = decoded.len();
            }
        }
    }

    #[test]
    fn a_restart_marker_split_across_chunks_is_still_found() {
        for codec in [Codec::Deflate, Codec::Zstd] {
            let (encoded, _) = segmented(codec);
            let mut whole = Vec::new();
            codec.restart_scan().push(&encoded, &mut whole);

            for step in [1_usize, 2, 3, 4, 5, 7] {
                let mut scan = codec.restart_scan();
                let mut split = Vec::new();
                for chunk in encoded.chunks(step) {
                    scan.push(chunk, &mut split);
                }
                assert_eq!(split, whole, "{codec} in {step}-byte chunks");
                assert_eq!(scan.consumed(), encoded.len() as u64);
            }
        }
    }

    #[test]
    fn a_coding_whose_framing_wraps_the_payload_refuses_a_restart() {
        for codec in [Codec::Gzip, Codec::Zlib] {
            assert!(!codec.has_restarts());
            let mut encoded = Vec::new();
            let mut encoder = codec.writer_with_level(&mut encoded, Level::DEFAULT);
            encoder.write_all(b"symbol,price\n").expect("the payload");
            let refused = encoder.restart().expect_err("no restart point");
            assert!(
                refused.to_string().contains(codec.as_str()),
                "{refused} names {codec}"
            );
            encoder.finish().expect("the stream still finishes");
            assert_eq!(
                codec.load(&encoded).expect("the payload"),
                b"symbol,price\n"
            );

            // A refused restart leaves nothing to find.
            let mut offsets = Vec::new();
            codec.restart_scan().push(&encoded, &mut offsets);
            assert!(offsets.is_empty());
        }
    }

    #[test]
    fn an_identity_stream_restarts_at_every_offset_and_reports_none() {
        assert!(Codec::Identity.has_restarts());
        let mut encoded = Vec::new();
        let mut encoder = Codec::Identity.writer(&mut encoded);
        encoder.write_all(b"symbol").expect("the payload");
        encoder.restart().expect("identity always restarts");
        encoder.write_all(b",price").expect("the payload");
        encoder.finish().expect("the stream finishes");
        assert_eq!(encoded, b"symbol,price");

        let mut offsets = Vec::new();
        Codec::Identity.restart_scan().push(&encoded, &mut offsets);
        assert!(offsets.is_empty());
    }

    #[test]
    fn restarting_costs_size_and_nothing_else() {
        let payload = b"symbol,price\nAAPL,187.23\n".repeat(512);
        let plain = Codec::Deflate.dump(&payload).expect("the plain stream");

        let mut restarted = Vec::new();
        let mut encoder = Codec::Deflate.writer(&mut restarted);
        for chunk in payload.chunks(1024) {
            encoder.restart().expect("the coding restarts");
            encoder.write_all(chunk).expect("the chunk encodes");
        }
        encoder.finish().expect("the stream finishes");

        assert_eq!(
            Codec::Deflate.load(&restarted).expect("the payload"),
            payload
        );
        assert!(restarted.len() > plain.len());
    }
}
