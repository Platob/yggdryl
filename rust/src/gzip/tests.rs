//! RFC 1952 gzip buffers, streams, and the transparent handle.

use super::{Gzip, dump, dump_with_level, load, reader, writer, writer_with_level};
use crate::Level;
use crate::io::{Buffer, IOBase};
use std::io::{Read, Write};

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
fn whole_buffers_round_trip() {
    let encoded = dump(PAYLOAD).unwrap();
    assert_eq!(load(&encoded).unwrap(), PAYLOAD);
    // A repetitive payload really does get smaller.
    assert!(encoded.len() < PAYLOAD.len(), "{} bytes", encoded.len());
}

#[test]
fn every_level_round_trips() {
    for level in [Level::NONE, Level::FAST, Level::DEFAULT, Level::BEST] {
        let encoded = dump_with_level(PAYLOAD, level).unwrap();
        assert_eq!(load(&encoded).unwrap(), PAYLOAD, "{level}");
    }
}

#[test]
fn streams_round_trip_without_buffering_the_payload() {
    let mut target = Vec::new();
    let mut encoder = writer(&mut target);
    encoder.write_all(PAYLOAD).unwrap();
    encoder.finish().unwrap();

    let mut decoded = Vec::new();
    reader(target.as_slice()).read_to_end(&mut decoded).unwrap();
    assert_eq!(decoded, PAYLOAD);

    // The same at an explicit level.
    let mut target = Vec::new();
    let mut encoder = writer_with_level(&mut target, Level::BEST);
    encoder.write_all(PAYLOAD).unwrap();
    encoder.finish().unwrap();
    assert_eq!(load(&target).unwrap(), PAYLOAD);
}

#[test]
fn a_payload_that_is_not_this_format_is_reported() {
    assert!(load(b"definitely not a compressed payload").is_err());
}

#[test]
fn the_handle_reads_decoded_and_stores_encoded() {
    let mut handle = Gzip::new(Buffer::new());
    handle.write_all_bytes(PAYLOAD).unwrap();
    handle.flush().unwrap();

    assert_eq!(handle.read_all().unwrap(), PAYLOAD);
    assert_eq!(handle.size(), PAYLOAD.len() as u64);
    assert_eq!(load(handle.handle().as_slice()).unwrap(), PAYLOAD);

    // The wrapped handle can be recovered with the write already published.
    let inner = handle.into_handle().unwrap();
    assert_eq!(load(inner.as_slice()).unwrap(), PAYLOAD);
}

#[test]
fn a_handle_level_reaches_the_encoder() {
    let mut handle = Gzip::new(Buffer::new()).with_level(Level::BEST);
    assert_eq!(handle.level(), Level::BEST);

    handle.write_all_bytes(PAYLOAD).unwrap();
    handle.flush().unwrap();
    assert_eq!(handle.read_all().unwrap(), PAYLOAD);
}
