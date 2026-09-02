//! A coded handle presents the decoded bytes and stores the encoded ones.

use super::Coded;
use crate::io::{Buffer, IOBase, IOMedia};
use crate::{Codec, Level, MimeType, Url};

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

impl crate::io::IOMedia for SharedReads {
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
        let mut handle = Coded::new(Buffer::new(), codec);
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
    let mut handle = Coded::new(Buffer::new(), Codec::Identity);
    handle.write_all_bytes(PAYLOAD).unwrap();
    handle.flush().unwrap();

    assert_eq!(handle.handle().as_slice(), PAYLOAD);
}

#[test]
fn positional_reads_and_writes_address_the_decoded_value() {
    let mut handle = Coded::new(Buffer::new(), Codec::Gzip);
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

    let handle = Coded::new(inner, Codec::Gzip);
    // Wrapping removes the coding, because the wrapper's bytes are decoded.
    assert_eq!(handle.media_type().base(), &MimeType::ARROW_STREAM);
    assert_eq!(handle.media_type().encoding_len(), 0);
    assert!(handle.codec().is_identity() || handle.codec() == Codec::Gzip);
}

#[test]
fn a_missing_resource_reads_as_empty_rather_than_failing() {
    let handle = Coded::new(Buffer::new(), Codec::Zstd);
    assert_eq!(handle.size(), 0);
    assert!(handle.read_all_bytes().unwrap().is_empty());
}

#[test]
fn open_materializes_and_close_publishes() {
    let mut handle = Coded::new(Buffer::new(), Codec::Gzip);
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
    let mut best = Coded::new(Buffer::new(), Codec::Zstd).with_level(Level::BEST);
    let mut fast = Coded::new(Buffer::new(), Codec::Zstd).with_level(Level::FAST);

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
    let mut handle = Coded::new(Buffer::new(), Codec::Zlib);
    handle.write_all_bytes(PAYLOAD).unwrap();

    handle.truncate(6).unwrap();
    assert_eq!(handle.read_all_bytes().unwrap(), b"symbol");

    // Growing zero-fills, exactly as a positional write past the end does.
    handle.truncate(8).unwrap();
    assert_eq!(handle.read_all_bytes().unwrap(), b"symbol\0\0");
}

#[test]
fn an_open_handle_answers_reads_out_of_what_it_holds() {
    use crate::buffered::tests::Counting;

    let mut source = Coded::new(Buffer::new(), Codec::Gzip);
    source.write_all_bytes(PAYLOAD).unwrap();
    source.flush().unwrap();
    let encoded = source.into_handle().unwrap().into_bytes();

    let mut handle = Coded::new(Counting::from_bytes(encoded), Codec::Gzip);
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
        let handle = Coded::new(Buffer::from_bytes(encoded), codec);

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
        let handle = Coded::new(Buffer::from_bytes(encoded), codec);
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
    use crate::buffered::tests::Counting;

    // Deliberately incompressible enough to span many transport reads. The
    // regression was one `pread` per encoded byte when output batches were 1.
    let payload: Vec<u8> = (0..256 * 1024)
        .map(|index| ((index * 131 + index / 251) & 0xff) as u8)
        .collect();
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let encoded = codec.dump(&payload).unwrap();
        let handle = Coded::new(Counting::from_bytes(encoded.clone()), codec);
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
    use crate::buffered::tests::Counting;

    let payload = PAYLOAD.repeat(64);
    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let encoded = codec.dump(&payload).unwrap();
        let handle = Coded::new(Counting::from_bytes(encoded), codec);
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
    use crate::buffered::tests::Counting;

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
        let handle = Coded::new(Counting::from_bytes(encoded.clone()), codec);
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
        let handle = Coded::new(Buffer::from_bytes(codec.dump(&payload).unwrap()), codec);

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
    let direct = crate::gzip::Gzip::new(direct_source);
    assert_eq!(direct.read_all_bytes().unwrap(), payload);
    let direct_reads = direct_reads.load(std::sync::atomic::Ordering::Relaxed);

    let (boxed_source, boxed_reads) = SharedReads::new(encoded);
    let boxed_inner = crate::gzip::Gzip::new(boxed_source);
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

    use crate::generic::{IORecordOptions, RecordOptions};
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
    crate::ipc::overwrite_arrow_reader(
        &mut plain,
        crate::arrow::batch_reader(schema, [batch]),
        &crate::ipc::IpcOptions::new(),
    )
    .unwrap();

    let encoded = Codec::Gzip.dump(plain.as_slice()).unwrap();
    let inner = Buffer::from_bytes(encoded).with_media_type(
        Url::from_str("file:///rows.arrows.gz")
            .unwrap()
            .media_type(),
    );
    let coded = Coded::new(inner, Codec::Gzip);
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
    use crate::buffered::tests::Counting;

    for codec in [Codec::Gzip, Codec::Zlib, Codec::Zstd] {
        let encoded = codec.dump(PAYLOAD).unwrap();
        let mut handle = Coded::new(Counting::from_bytes(encoded), codec);
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
        let empty = Coded::new(Buffer::new(), codec);
        let mut stream = empty.pstream_bytes(0, 3).unwrap();
        assert!(
            stream.next().is_none(),
            "{codec} rejected an absent resource"
        );
        assert!(stream.next().is_none(), "{codec} did not fuse at empty EOF");

        // Invalid framing is portable across decoder implementations, while
        // some DEFLATE readers deliberately accept an omitted checksum at EOF.
        let corrupt = Coded::new(Buffer::from_bytes(vec![0xA5; 32]), codec);
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
