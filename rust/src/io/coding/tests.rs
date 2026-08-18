//! A coded handle presents the decoded bytes and stores the encoded ones.

use super::Coded;
use crate::io::{Buffer, IOBase};
use crate::{Codec, Level, MimeType, Url};

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

        assert_eq!(handle.read_all().unwrap(), PAYLOAD, "{codec}");
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
    assert_eq!(&handle.read_range(0, 6).unwrap(), b"ticker");
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
    assert!(handle.read_all().unwrap().is_empty());
}

#[test]
fn open_materializes_and_close_publishes() {
    let mut handle = Coded::new(Buffer::new(), Codec::Gzip);
    assert!(!handle.is_open());

    handle.open().unwrap();
    assert!(handle.is_open());

    handle.write_all_bytes(PAYLOAD).unwrap();
    handle.close().unwrap();
    assert!(!handle.is_open());

    // Closing published the write, so the encoded bytes are there.
    assert_eq!(handle.read_all().unwrap(), PAYLOAD);
}

#[test]
fn a_higher_level_does_not_change_what_is_read_back() {
    let mut best = Coded::new(Buffer::new(), Codec::Zstd).with_level(Level::BEST);
    let mut fast = Coded::new(Buffer::new(), Codec::Zstd).with_level(Level::FAST);

    best.write_all_bytes(PAYLOAD).unwrap();
    fast.write_all_bytes(PAYLOAD).unwrap();
    best.flush().unwrap();
    fast.flush().unwrap();

    assert_eq!(best.read_all().unwrap(), PAYLOAD);
    assert_eq!(fast.read_all().unwrap(), PAYLOAD);
    assert!(best.handle().size() < PAYLOAD.len() as u64);
}

#[test]
fn truncation_shrinks_and_grows_the_decoded_value() {
    let mut handle = Coded::new(Buffer::new(), Codec::Zlib);
    handle.write_all_bytes(PAYLOAD).unwrap();

    handle.truncate(6).unwrap();
    assert_eq!(handle.read_all().unwrap(), b"symbol");

    // Growing zero-fills, exactly as a positional write past the end does.
    handle.truncate(8).unwrap();
    assert_eq!(handle.read_all().unwrap(), b"symbol\0\0");
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
