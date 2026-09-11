//! The XML exchange with an external implementation.
//!
//! `scripts/check_xml_interop.py` drives this target twice around a Python
//! `xml.etree.ElementTree` round trip: the first run writes a document
//! `ElementTree` must read, the second reads a document `ElementTree` wrote.
//! The reading half prints `SKIPPED` when the external document is absent -
//! the driver fails on that word - so a skipped half can never read as a pass.

use yggdryl::text::xml;
use yggdryl::{Scalar, from_xml_scalar};

/// Where the exchange files live, shared with the Python driver.
fn exchange_dir() -> std::path::PathBuf {
    let mut path = std::env::current_dir().expect("a working directory");
    // Under `cargo test` the working directory is `rust/`.
    path.push("target");
    path.push("xml-interop");
    path
}

/// The document both sides assert.
///
/// It carries every shape the mapping decides: an attribute, character data
/// beside one, a repeated element, an empty element, a prefixed name, and text
/// that only survives as references. The namespace is declared on the document
/// element, which is where the external writer puts one and where this crate
/// then reads it, because neither resolves a prefix into the name it carries.
fn exchanged() -> Scalar {
    Scalar::from_record([(
        "trades",
        Scalar::from_record([
            ("@venue", Scalar::from("XPAR")),
            ("@xmlns:ns", Scalar::from("urn:example")),
            (
                "trade",
                Scalar::from_sequence([
                    Scalar::from_record([
                        ("@id", Scalar::from("1")),
                        ("symbol", Scalar::from("AAPL")),
                        ("note", Scalar::from("a & b <c>")),
                        ("size", Scalar::from("100")),
                    ])
                    .expect("the first trade"),
                    Scalar::from_record([
                        ("@id", Scalar::from("2")),
                        ("symbol", Scalar::from("MSFT")),
                        ("note", Scalar::Null),
                        ("size", Scalar::from("250")),
                    ])
                    .expect("the second trade"),
                ]),
            ),
            ("ns:total", Scalar::from("350")),
        ])
        .expect("the document element"),
    )])
    .expect("the document")
}

#[test]
fn writes_a_document_for_the_external_reader() {
    let dir = exchange_dir();
    std::fs::create_dir_all(&dir).expect("the exchange directory");
    let path = dir.join("from-rust.xml");
    let _ = std::fs::remove_file(&path);

    let encoded = xml::into_bytes(&exchanged()).expect("the document encodes");
    std::fs::write(&path, &encoded).expect("the document writes");

    // Whatever the external reader makes of it, this crate reads its own
    // bytes back as the value it wrote.
    assert_eq!(
        xml::from_bytes(&encoded).expect("the document decodes"),
        exchanged()
    );
}

#[test]
fn reads_the_external_document() {
    let path = exchange_dir().join("from-python.xml");
    let Ok(encoded) = std::fs::read(&path) else {
        println!("SKIPPED: {} is absent", path.display());
        return;
    };

    let value = xml::from_bytes(&encoded).expect("the external document decodes");
    assert_eq!(
        value,
        exchanged(),
        "the external document carries the exchange"
    );

    // And the same bytes this crate would have written for it.
    let encoded = xml::into_utf8(&value).expect("the document encodes");
    assert_eq!(
        from_xml_scalar(&encoded).expect("the document decodes"),
        exchanged()
    );
}
