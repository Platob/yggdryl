//! One enum over every coding, wrapping any handle.

use super::Coded as CodedHandle;
use crate::io::{Buffer, IOBase};
use crate::{Codec, Level, Url};

const PAYLOAD: &[u8] = b"symbol,price\nAAPL,1\nAAPL,2\nAAPL,3\nAAPL,4\nAAPL,5\nAAPL,6\n\
AAPL,7\nAAPL,8\nAAPL,9\nAAPL,10\nAAPL,11\nAAPL,12\n";

#[test]
fn every_coding_round_trips_through_the_enum() {
    for codec in Codec::ALL {
        let mut handle = CodedHandle::wrap(Buffer::new(), codec);
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
    let mut handle = CodedHandle::infer(named);
    assert_eq!(handle.codec(), Codec::Zstd);

    handle.write_all_bytes(PAYLOAD).unwrap();
    handle.flush().unwrap();
    assert_eq!(handle.read_all_bytes().unwrap(), PAYLOAD);
    // The wrapped handle holds the compressed form.
    assert!(handle.handle().size() < PAYLOAD.len() as u64);
}

#[test]
fn raw_deflate_wraps_as_its_framed_form() {
    let handle = CodedHandle::wrap(Buffer::new(), Codec::Deflate);
    assert_eq!(handle.codec(), Codec::Zlib);
}

#[test]
fn an_identity_coding_writes_the_payload_unchanged() {
    let mut handle = CodedHandle::wrap(Buffer::new(), Codec::Identity);
    handle.write_all_bytes(PAYLOAD).unwrap();
    handle.flush().unwrap();

    assert_eq!(handle.handle().as_slice(), PAYLOAD);
}

#[test]
fn a_level_reaches_the_encoder_and_the_handle_survives_it() {
    let mut handle = CodedHandle::wrap(Buffer::new(), Codec::Gzip).with_level(Level::BEST);
    handle.write_all_bytes(PAYLOAD).unwrap();

    let inner = handle.into_handle().unwrap();
    assert_eq!(crate::gzip::load(inner.as_slice()).unwrap(), PAYLOAD);
}

#[test]
fn a_missing_resource_reads_as_empty_rather_than_failing() {
    let handle = CodedHandle::wrap(Buffer::new(), Codec::Gzip);
    assert_eq!(handle.size(), 0);
    assert!(handle.read_all_bytes().unwrap().is_empty());
}
