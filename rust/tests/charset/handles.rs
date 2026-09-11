//! A charset applied to a handle: what a reader downstream of it sees.

use std::io::Read as _;

use yggdryl::charset::Transcoded;
use yggdryl::coding::Coded;
use yggdryl::holder::Buffer;
use yggdryl::{Charset, Codec, IOBase, IOCursor as _, MediaType, Scalar, text};

/// A buffer holding `text` encoded in `charset` and declaring it.
fn declared(charset: Charset, base: &str, body: &str) -> Buffer {
    let media_type = MediaType::from_str(base)
        .expect("a media type")
        .with_charset(charset);
    Buffer::from_bytes(
        charset
            .encode(body)
            .expect("the charset holds it")
            .into_owned(),
    )
    .with_media_type(media_type)
}

#[test]
fn a_transcoded_handle_presents_utf8_to_everything_downstream() {
    let body = "symbol,désk\nAAPL,€1\n";
    let source = declared(Charset::Cp1252, "text/csv", body);
    assert_eq!(source.media_type().charset(), Some(Charset::Cp1252));

    let handle = Transcoded::infer(source);
    assert_eq!(handle.charset(), Charset::Cp1252);
    assert_eq!(handle.read_all_bytes().unwrap(), body.as_bytes());
    // The presented bytes are UTF-8, so the handle no longer declares a
    // charset: it would be a claim about bytes that are no longer there.
    assert_eq!(handle.media_type().charset(), None);
    assert_eq!(handle.size(), body.len() as u64);
}

#[test]
fn a_transcoded_handle_streams_without_materializing_the_value() {
    let body = "é".repeat(64 * 1024);
    let source = declared(Charset::Latin1, "text/plain", &body);
    let handle = Transcoded::new(source, Charset::Latin1);

    let mut streamed = Vec::new();
    handle
        .pstream_bytes(0, 4096)
        .unwrap()
        .read_to_end(&mut streamed)
        .unwrap();
    assert_eq!(streamed, body.as_bytes());
    assert!(!handle.opened(), "a streamed read materialized the value");

    // A positional read answers the same bytes a whole read would.
    let whole = handle.read_all_bytes().unwrap();
    for offset in [0_u64, 1, 2, 100, 4095, 4096] {
        let window = handle.read_range_bytes(offset, 8).unwrap();
        let start = usize::try_from(offset).unwrap();
        assert_eq!(window, &whole[start..start + window.len()], "{offset}");
    }
}

#[test]
fn a_transcoded_handle_round_trips_a_write_through_the_charset() {
    let mut handle = Transcoded::new(Buffer::new(), Charset::Latin9);
    let body = "prix,€\nAAPL,1\n";
    handle.write_all_bytes(body.as_bytes()).unwrap();
    handle.flush().unwrap();

    assert_eq!(handle.read_all_bytes().unwrap(), body.as_bytes());
    let encoded = handle.into_handle().unwrap().read_all_bytes().unwrap();
    assert_eq!(encoded, Charset::Latin9.encode(body).unwrap().as_ref());
    // Latin-9 spells the euro in one byte where UTF-8 needs three.
    assert_eq!(encoded.len(), body.len() - 2);
}

#[test]
fn a_charset_composes_over_a_coding_rather_than_inside_it() {
    let body = "symbol,désk\n";
    let plain = Charset::Cp1252.encode(body).unwrap().into_owned();
    let compressed = Codec::Gzip.dump(&plain).unwrap();

    let source = Buffer::from_bytes(compressed).with_media_type(
        MediaType::from_str("text/csv;charset=windows-1252;encodings=application/gzip").unwrap(),
    );
    // Decompress first, then decode: the coding wraps the encoded bytes.
    let decoded = Coded::infer(source);
    assert_eq!(decoded.read_all_bytes().unwrap(), plain);
    assert_eq!(decoded.media_type().charset(), Some(Charset::Cp1252));

    let text = Transcoded::infer(decoded);
    assert_eq!(text.read_all_bytes().unwrap(), body.as_bytes());
}

#[test]
fn a_declared_charset_reads_a_structured_document_that_is_not_utf8() {
    let document = "{\"desk\":\"Zürich\",\"fee\":\"€1\"}";
    let source = declared(Charset::Cp1252, "application/json", document);

    let value = text::from_io(&source).expect("a declared charset reads the document");
    assert_eq!(
        value.get_key_str("desk").and_then(Scalar::as_str),
        Some("Zürich")
    );
    assert_eq!(
        value.get_key_str("fee").and_then(Scalar::as_str),
        Some("€1")
    );
}

#[test]
fn a_byte_order_mark_is_framing_rather_than_the_first_character() {
    // A UTF-16 document declares nothing and is recognized by its own mark.
    let mut bytes = Charset::Utf16Le.bom().expect("a marked form").to_vec();
    bytes.extend_from_slice(&Charset::Utf16Le.encode("{\"id\":1}").unwrap());
    let source =
        Buffer::from_bytes(bytes).with_media_type(MediaType::from_str("application/json").unwrap());

    let value = text::from_io(&source).expect("the mark names the charset");
    assert_eq!(value.get_key_str("id").and_then(Scalar::as_i64), Some(1));
}

#[test]
fn a_declared_charset_wins_over_a_mark_the_payload_carries() {
    // The mark is still framing and comes off, but it does not overrule what
    // the handle was told the bytes are.
    let mut bytes = Charset::Utf8.bom().expect("a marked form").to_vec();
    bytes.extend_from_slice(&Charset::Cp1252.encode("{\"fee\":\"€1\"}").unwrap());
    let source = Buffer::from_bytes(bytes).with_media_type(
        MediaType::from_str("application/json")
            .unwrap()
            .with_charset(Charset::Cp1252),
    );

    let value = text::from_io(&source).expect("the declaration decides");
    assert_eq!(
        value.get_key_str("fee").and_then(Scalar::as_str),
        Some("€1")
    );
}

#[test]
fn a_structured_write_encodes_in_the_charset_the_handle_declares() {
    let mut target = Buffer::new().with_media_type(
        MediaType::from_str("application/json")
            .unwrap()
            .with_charset(Charset::Latin1),
    );
    let value = Scalar::from_record([("desk", Scalar::from("Zürich"))]).unwrap();
    text::into_io(&value, &mut target).unwrap();

    let written = target.read_all_bytes().unwrap();
    assert_eq!(
        Charset::Latin1.decode(&written).unwrap(),
        "{\"desk\":\"Zürich\"}"
    );
    // One byte per scalar on the wire, two in UTF-8.
    assert_eq!(written.len(), "{\"desk\":\"Zürich\"}".len() - 1);
    assert_eq!(text::from_io(&target).unwrap(), value);
}

#[test]
fn a_decoded_handle_is_a_random_byte_accessor() {
    // Many strides of payload, so a read near the end is a genuine seek and
    // not an accident of the value fitting in one batch.
    let body = "symbol,désk,prix €\nAAPL,London,187.23\n".repeat(8 * 1024);
    let source = declared(Charset::Cp1252, "text/csv", &body);
    let handle = Transcoded::infer(source);

    let whole = handle.read_all_bytes().unwrap();
    assert_eq!(whole, body.as_bytes());
    assert_eq!(handle.size(), body.len() as u64);

    // Every window a caller could ask for answers the same bytes a whole read
    // does, wherever it lands and whatever it straddles.
    let size = whole.len();
    for offset in [
        0,
        1,
        2,
        63,
        64,
        size / 4,
        size / 2,
        size - 1024,
        size - 3,
        size - 1,
        size,
        size + 8,
    ] {
        let window = handle.read_range_bytes(offset as u64, 129).unwrap();
        let expected = whole
            .get(offset..(offset + 129).min(size))
            .unwrap_or_default();
        assert_eq!(window, expected, "range at {offset}");

        let mut buffer = [0_u8; 129];
        let read = handle.pread(offset as u64, &mut buffer).unwrap();
        assert_eq!(&buffer[..read], &expected[..read], "pread at {offset}");
    }

    // A cursor over the same handle walks it the same way, because a cursor
    // is the shared one every `IOBase` gets rather than a second accessor.
    let mut cursor = handle.cursor_at(10);
    let mut window = [0_u8; 16];
    let read = cursor.read_next(&mut window).unwrap();
    assert_eq!(&window[..read], &whole[10..10 + read]);
    assert_eq!(cursor.tell(), 10 + read as u64);
    cursor.seek_to(size as u64 - 4);
    let read = cursor.read_next(&mut window).unwrap();
    assert_eq!(&window[..read], &whole[size - 4..]);
}

#[test]
fn a_write_drops_the_index_a_read_built() {
    let mut handle = Transcoded::new(Buffer::new(), Charset::Latin1);
    handle.write_all_bytes("prémier\n".as_bytes()).unwrap();
    handle.flush().unwrap();
    assert_eq!(handle.size(), "prémier\n".len() as u64);
    assert_eq!(
        handle.read_range_bytes(2, 3).unwrap(),
        "émi".as_bytes()[..3]
    );

    handle.close().unwrap();
    handle
        .write_all_bytes("deuxième plus long\n".as_bytes())
        .unwrap();
    handle.flush().unwrap();
    handle.close().unwrap();

    // The index the first value built says nothing about the second.
    assert_eq!(handle.size(), "deuxième plus long\n".len() as u64);
    assert_eq!(
        handle.read_all_bytes().unwrap(),
        "deuxième plus long\n".as_bytes()
    );
    assert_eq!(
        handle.read_range_bytes(4, 6).unwrap(),
        &"deuxième plus long\n".as_bytes()[4..10]
    );
}
