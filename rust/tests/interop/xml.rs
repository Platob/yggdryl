//! The XML exchange with an external implementation.
//!
//! `scripts/check_xml_interop.py` drives this target twice around a round trip
//! through Python's own XML stack: the first run writes a document and a
//! schema that `xml.etree.ElementTree` and `xmlschema` must accept, the second
//! reads the document and the schema those wrote. The reading half prints
//! `SKIPPED` when the external files are absent - the driver fails on that
//! word - so a skipped half can never read as a pass.
//!
//! Self-consistency proves nothing about an exchange format. A document this
//! crate writes and reads back would agree with itself whatever it spelled, so
//! what is asserted here is that an outside parser reads the same facts out of
//! it, and that an outside schema processor agrees the schema written beside
//! it describes it.

use std::path::PathBuf;

// One test writes the document another reads, so they never overlap.
static EXCHANGE: std::sync::Mutex<()> = std::sync::Mutex::new(());

use yggdryl::media::{IORecordOptions, xml};
use yggdryl::types::protocol::XmlKind;
use yggdryl::{DataType, Field, IOMedia, IOMode, Limits, Scalar};

/// Where the exchange files live, shared with the Python driver.
fn exchange_dir() -> PathBuf {
    let mut path = std::env::current_dir().expect("a working directory");
    // Under `cargo test` the working directory is `rust/`.
    path.push("target");
    path.push("xml-interop");
    path
}

/// The row both sides write and both sides read.
///
/// It carries one column of each kind the exchange has to survive: two
/// attributes, three child elements, one of them absent on a row, and one that
/// repeats. Every type maps onto an XSD datatype exactly, so the schema this
/// field writes is one an outside processor can compile without guessing.
fn row_field() -> Field {
    let mut id = DataType::Int32.required_field("id");
    id.as_xml_mut()
        .set_kind(XmlKind::Attribute)
        .expect("a static spelling");
    let mut currency = DataType::utf8().required_field("Ccy");
    currency
        .as_xml_mut()
        .set_kind(XmlKind::Attribute)
        .expect("a static spelling");
    let tag = DataType::list(DataType::utf8().required_field("item")).required_field("tag");
    DataType::from_fields([
        id,
        currency,
        DataType::utf8().required_field("symbol"),
        DataType::Int64.required_field("quantity"),
        DataType::utf8().nullable_field("note"),
        tag,
    ])
    .expect("a row of named columns")
    .required_field("Trade")
}

/// The rows both sides assert, in document order.
///
/// The notes are the escaping the exchange has to survive: the three markup
/// characters, and a carriage return, which a parser rewrites to a newline
/// unless it was written as a reference.
fn expected_rows() -> Scalar {
    Scalar::from_sequence([
        row(1, "EUR", "AAPL", 100, Some("a < b & c"), ["eu", "cash"]),
        row(2, "USD", "MSFT", 25, Some("line\rbreak"), ["us"]),
        row(3, "GBP", "VOD.L", 7, None, ["gb"]),
    ])
}

/// One row of the exchange.
fn row(
    id: i32,
    currency: &str,
    symbol: &str,
    quantity: i64,
    note: Option<&str>,
    tags: impl IntoIterator<Item = &'static str>,
) -> Scalar {
    Scalar::from_record([
        ("id", Scalar::from(id)),
        ("Ccy", Scalar::from(currency)),
        ("symbol", Scalar::from(symbol)),
        ("quantity", Scalar::from(quantity)),
        ("note", note.map_or(Scalar::Null, Scalar::from)),
        (
            "tag",
            Scalar::from_sequence(tags.into_iter().map(Scalar::from)),
        ),
    ])
    .expect("a row of named columns")
}

/// A handle over `path`, named so its media type says XML.
fn handle(path: &std::path::Path) -> yggdryl::holder::Holder {
    yggdryl::holder::Holder::file(path).expect("a local document")
}

#[test]
fn writes_a_document_and_a_schema_for_the_external_reader() {
    let _exchange = EXCHANGE.lock().expect("exclusive XML exchange fixtures");
    let dir = exchange_dir();
    std::fs::create_dir_all(&dir).expect("the exchange directory");

    let field = row_field();
    let document = dir.join("from-rust.xml");
    let _ = std::fs::remove_file(&document);
    let mut target = handle(&document);
    let options = target
        .record_options()
        .expect("record options")
        .with_field(field.clone());
    target
        .write_arrow(
            yggdryl::arrow::ArrowScalar::from_rows(&field, &expected_rows()).expect("typed rows"),
            IOMode::Overwrite,
            Some(&options),
        )
        .expect("the document writes");

    // The schema is written from the same field the rows crossed, which is
    // the whole claim an outside processor is asked to check.
    std::fs::write(
        dir.join("from-rust.xsd"),
        xml::field_into_xsd(&field, yggdryl::text::Formatting::default()).expect("a schema"),
    )
    .expect("the schema writes");
    println!("xml-interop: wrote");
}

#[test]
fn reads_the_document_the_external_writer_produced() {
    let document = exchange_dir().join("from-python.xml");
    if !document.exists() {
        println!("xml-interop: SKIPPED (no {})", document.display());
        return;
    }
    let schema = exchange_dir().join("from-python.xsd");
    assert!(schema.exists(), "the driver writes both or neither");

    // The schema an outside processor wrote is read as the field it declares,
    // and that field is what types the document beside it.
    let declared = xml::field_from_xsd(
        &std::fs::read(&schema).expect("the schema reads"),
        Limits::default(),
        None,
    )
    .expect("the external schema declares a field");
    assert_eq!(declared.name(), "Trade");
    assert_eq!(
        declared.field("id").expect("the id column").dtype(),
        &DataType::Int32
    );
    assert_eq!(
        declared
            .field("Ccy")
            .expect("the currency column")
            .as_xml()
            .kind()
            .expect("a spelling"),
        XmlKind::Attribute
    );

    let source = handle(&document);
    let options = source
        .record_options()
        .expect("record options")
        .with_field(declared);
    let rows: usize = source
        .read_arrow_reader(&options)
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum();
    assert_eq!(rows, 3);

    // And the document's own mapping, read with nothing declared: every leaf
    // is text, and the repeated child is one column holding both.
    let value = xml::from_bytes(&std::fs::read(&document).expect("the document reads"))
        .expect("the external document decodes");
    let trades = value
        .get_key_str("Trade")
        .and_then(Scalar::as_sequence)
        .expect("three Trade children");
    assert_eq!(trades.len(), 3);
    assert_eq!(
        trades[0].get_key_str("Ccy").and_then(Scalar::as_str),
        Some("EUR")
    );
    assert_eq!(
        trades[0].get_key_str("note").and_then(Scalar::as_str),
        Some("a < b & c")
    );
    assert_eq!(
        trades[0]
            .get_key_str("tag")
            .and_then(Scalar::as_sequence)
            .map(<[Scalar]>::len),
        Some(2)
    );
    println!("xml-interop: read");
}

#[test]
fn reads_the_document_it_wrote_for_the_external_reader() {
    // The writing half above is only meaningful if what it wrote is what this
    // crate meant to write, so the same field reads it back here rather than
    // leaving that to the driver alone.
    let _exchange = EXCHANGE.lock().expect("exclusive XML exchange fixtures");
    let document = exchange_dir().join("from-rust.xml");
    if !document.exists() {
        println!("xml-interop: SKIPPED (no {})", document.display());
        return;
    }
    let field = row_field();
    let source = handle(&document);
    let options = source
        .record_options()
        .expect("record options")
        .with_field(field.clone());
    let value = source.read_arrow(Some(&options)).expect("the rows read");
    let read = value.into_scalar().expect("the rows as a value");
    let read = read.as_sequence().expect("a sequence of rows");
    let written = expected_rows();
    let written = written.as_sequence().expect("a sequence of rows");
    assert_eq!(read.len(), written.len());
    for (index, (actual, wanted)) in read.iter().zip(written).enumerate() {
        // A typed row is the positional value its field declares, so the
        // comparison canonicalizes the written row rather than the read one.
        let wanted = field
            .from_natural_value(wanted.clone())
            .expect("a typed row");
        assert_eq!(actual, &wanted, "row {index}");
    }
}
