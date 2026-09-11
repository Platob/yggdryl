//! Record reads over payloads that are not UTF-8.

use arrow_array::{Array as _, StringArray};
use yggdryl::charset::Transcoded;
use yggdryl::holder::Buffer;
use yggdryl::media::text::TextOptions;
use yggdryl::{Charset, IOMedia as _, MediaType};

/// The string column `name` holds, across every batch.
fn strings(batches: &[arrow_array::RecordBatch], name: &str) -> Vec<Option<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            let index = batch.schema().index_of(name).expect("a declared column");
            batch
                .column(index)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("a string column")
                .iter()
                .map(|value| value.map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn read(source: &impl yggdryl::IOBase, options: TextOptions) -> Vec<arrow_array::RecordBatch> {
    source
        .read_arrow_reader(&options.into())
        .expect("a reader")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("batches")
}

#[test]
fn a_capture_is_read_in_the_charset_the_options_declare() {
    let lines = "Zürich first\nLondon second\n";
    let source = Buffer::from_bytes(Charset::Cp1252.encode(lines).unwrap().into_owned())
        .with_media_type(MediaType::from_str("text/plain").unwrap());

    let options = TextOptions::new()
        .try_with_rowheader(r"^(?<city>(?-u:\S)+) ")
        .expect("a header");
    let refused = source
        .read_arrow_reader(&options.clone().into())
        .and_then(|reader| {
            reader
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(yggdryl::Error::from)
        });
    assert!(
        refused.is_err(),
        "the default charset accepted CP1252 bytes"
    );

    let batches = read(&source, options.with_charset(Charset::Cp1252));
    assert_eq!(
        strings(&batches, "city"),
        [Some("Zürich".to_owned()), Some("London".to_owned())]
    );
}

#[test]
fn a_transcoded_handle_needs_no_charset_on_the_options() {
    let lines = "Zürich first\nLondon second\n";
    let source = Buffer::from_bytes(Charset::Cp1252.encode(lines).unwrap().into_owned())
        .with_media_type(MediaType::from_str("text/plain;charset=windows-1252").unwrap());

    // The other door: decode once at the handle, and the record layer is
    // reading UTF-8 like any other payload.
    let handle = Transcoded::infer(source);
    let batches = read(
        &handle,
        TextOptions::new()
            .try_with_rowheader(r"^(?<city>(?-u:\S)+) ")
            .expect("a header"),
    );
    assert_eq!(
        strings(&batches, "city"),
        [Some("Zürich".to_owned()), Some("London".to_owned())]
    );
}

#[test]
fn a_record_body_stays_the_bytes_that_arrived() {
    let lines = "Zürich premièr\n";
    let wire = Charset::Cp1252.encode(lines).unwrap().into_owned();
    let source = Buffer::from_bytes(wire.clone())
        .with_media_type(MediaType::from_str("text/plain").unwrap());

    let batches = read(
        &source,
        TextOptions::new()
            .try_with_rowheader(r"^(?<city>(?-u:\S)+) ")
            .expect("a header")
            .with_charset(Charset::Cp1252),
    );
    assert_eq!(strings(&batches, "city"), [Some("Zürich".to_owned())]);

    let index = batches[0].schema().index_of("body").expect("a body column");
    let body = batches[0]
        .column(index)
        .as_any()
        .downcast_ref::<arrow_array::BinaryArray>()
        .expect("a binary column");
    // A capture is an arrival record: the body is the bytes that were written,
    // one per scalar and undecoded, and only the captures are read as text.
    assert_eq!(body.value(0), b"premi\xe8r");
}
