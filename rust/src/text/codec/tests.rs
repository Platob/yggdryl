use super::{Json, Jsonl, TextCodec, Toml, Yaml};
use crate::IOBase;
use crate::holder::Buffer;
use crate::{MimeType, Scalar, Url};
use std::sync::atomic::{AtomicUsize, Ordering};

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

fn value() -> Scalar {
    Json.from_utf8(r#"{"price":1.5,"symbol":"AAPL"}"#).unwrap()
}

#[test]
fn text_bytes_and_readers_share_one_contract() {
    let expected = value();
    for encoded in [
        Json.into_utf8(&expected),
        Toml.into_utf8(&expected),
        Yaml.into_utf8(&expected),
    ] {
        assert!(encoded.is_ok());
    }

    let bytes = Json.into_bytes(&expected).unwrap();
    assert_eq!(Json.from_bytes(&bytes).unwrap(), expected);
    assert_eq!(Json.from_reader(bytes.as_slice()).unwrap(), expected);
}

#[test]
fn handles_apply_their_own_content_coding() {
    let expected = value();
    let mut compressed = handle("quote.json.gz");
    Json.into_io(&expected, &mut compressed).unwrap();
    assert_eq!(Json.from_io(&compressed).unwrap(), expected);
    assert_eq!(compressed.read_range_bytes(0, 2).unwrap(), [0x1F, 0x8B]);
}

#[test]
fn newline_delimited_json_holds_one_value_per_line() {
    let values = [
        Json.from_utf8(r#"{"id":1}"#).unwrap(),
        Json.from_utf8(r#"{"id":2}"#).unwrap(),
    ];
    let text = Jsonl.into_utf8_all(&values).unwrap();
    assert_eq!(text.lines().count(), 2);
    assert_eq!(Jsonl.from_utf8_all(&text).unwrap(), values);

    let mut lines = handle("rows.jsonl");
    Jsonl.into_io_all(&values, &mut lines).unwrap();
    assert_eq!(Jsonl.from_io_all(&lines).unwrap(), values);
}

#[test]
fn formats_report_their_media_types() {
    assert_eq!(Json.mime_type(), MimeType::JSON);
    assert_eq!(Jsonl.mime_type(), MimeType::JSON_LINES);
    assert_eq!(Yaml.mime_type(), MimeType::YAML);
    assert_eq!(Toml.mime_type(), MimeType::TOML);
}

/// A source whose size accessor is observable. Structured parsing must use
/// the positional byte stream rather than measuring then materializing it.
struct Measured {
    inner: Buffer,
    size_asks: AtomicUsize,
    reads: AtomicUsize,
}

impl crate::IOMedia for Measured {
    crate::delegate_iomedia!(inner);
}

impl IOBase for Measured {
    crate::delegate_iobase!(inner: pwrite, capacity, reserve, truncate, url, media_type,
        set_media_type, flush, parent, child_by_path, ls, kind, clear, remove, is_atomic,
        is_tabular, is_io);

    fn pread(&self, offset: u64, target: &mut [u8]) -> crate::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.inner.pread(offset, target)
    }

    fn size(&self) -> u64 {
        self.size_asks.fetch_add(1, Ordering::Relaxed);
        self.inner.size()
    }
}

#[test]
fn text_codec_handle_reads_stream_without_measuring_the_payload() {
    let message = "0123456789abcdef".repeat(32 * 1024);
    let expected = Scalar::from_record([("message", Scalar::from(message))]).unwrap();
    let plain = Json.into_bytes(&expected).unwrap();
    let encoded = crate::coding::gzip::dump(&plain).unwrap();
    let source = Measured {
        inner: Buffer::from_bytes(encoded)
            .with_media_type(Url::from_str("file:///large.json.gz").unwrap().media_type()),
        size_asks: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
    };

    assert_eq!(Json.from_io(&source).unwrap(), expected);
    assert_eq!(source.size_asks.load(Ordering::Relaxed), 0);
    assert!(source.reads.load(Ordering::Relaxed) > 1);
}
