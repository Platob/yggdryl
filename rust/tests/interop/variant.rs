//! The Variant exchange with the format's own implementation.
//!
//! Both directions run in process against `parquet-variant`, the Apache
//! crate the Parquet and Iceberg stacks read variants with: this crate
//! encodes a value and that crate reads it, then that crate builds one and
//! this crate reads it back. No external driver and no skipped half - the
//! test either exchanges or fails.

use parquet_variant::{Variant as Reference, VariantBuilder};
use yggdryl::{Scalar, TimeUnit, Timezone, Variant};

/// The value both directions exchange, one leaf per spelling that matters.
fn quote() -> Scalar {
    Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("size", Scalar::from(100_i64)),
        ("price", Scalar::from(1.5_f64)),
        ("filled", Scalar::from(true)),
        ("note", Scalar::Null),
        (
            "tags",
            Scalar::from_sequence([Scalar::from("open"), Scalar::from(2_i32)]),
        ),
    ])
    .expect("the exchange value builds")
}

#[test]
fn what_this_crate_writes_the_format_crate_reads() {
    let variant = Variant::encode(&quote()).expect("the exchange value encodes");
    let read = Reference::try_new(variant.metadata(), variant.value())
        .expect("the format's own reader accepts the bytes");
    let object = read.as_object().expect("an object");
    assert_eq!(object.len(), 6);
    assert_eq!(object.get("symbol"), Some(Reference::from("AAPL")));
    assert_eq!(object.get("size"), Some(Reference::from(100_i64)));
    assert_eq!(object.get("price"), Some(Reference::from(1.5_f64)));
    assert_eq!(object.get("filled"), Some(Reference::BooleanTrue));
    assert_eq!(object.get("note"), Some(Reference::Null));
    let list = object.get("tags").expect("the list field");
    let list = list.as_list().expect("a list");
    assert_eq!(list.len(), 2);
    assert_eq!(list.get(0), Some(Reference::from("open")));
    assert_eq!(list.get(1), Some(Reference::from(2_i32)));
}

#[test]
fn what_the_format_crate_writes_this_crate_reads() {
    let mut builder = VariantBuilder::new();
    let mut object = builder.new_object();
    object.insert("symbol", "AAPL");
    object.insert("size", 100_i64);
    object.insert("price", 1.5_f64);
    object.insert("filled", true);
    object.insert("note", ());
    object.finish();
    let (metadata, value) = builder.finish();

    let variant = Variant::new(metadata, value).expect("the format crate's bytes are a variant");
    let read = variant.scalar().expect("the exchange value decodes");
    assert_eq!(
        read,
        Scalar::from_struct([
            ("symbol", Scalar::from("AAPL")),
            ("size", Scalar::from(100_i64)),
            ("price", Scalar::from(1.5_f64)),
            ("filled", Scalar::from(true)),
            ("note", Scalar::Null),
        ])
        .expect("the expected value builds")
    );
}

#[test]
fn the_typed_leaves_exchange_under_the_types_the_specification_names() {
    let cases: Vec<(Scalar, Reference<'static, 'static>)> = vec![
        (Scalar::from(true), Reference::BooleanTrue),
        (Scalar::from(false), Reference::BooleanFalse),
        (Scalar::Null, Reference::Null),
        (Scalar::from(7_i8), Reference::Int8(7)),
        (Scalar::from(7_i16), Reference::Int16(7)),
        (Scalar::from(7_i32), Reference::Int32(7)),
        (Scalar::from(7_i64), Reference::Int64(7)),
        // An unsigned width takes the next signed width up.
        (Scalar::from(7_u8), Reference::Int16(7)),
        (Scalar::from(7_u16), Reference::Int32(7)),
        (Scalar::from(7_u32), Reference::Int64(7)),
        (Scalar::from(1.5_f32), Reference::Float(1.5)),
        (Scalar::from(1.5_f64), Reference::Double(1.5)),
        (
            Scalar::Uuid(yggdryl::Uuid::from_bytes(&[7; 16]).expect("a uuid")),
            Reference::Uuid(uuid_of([7; 16])),
        ),
        (Scalar::from(&b"\x00\x01"[..]), Reference::Binary(&[0, 1])),
    ];
    for (value, expected) in cases {
        let variant = Variant::encode(&value).expect("the leaf encodes");
        let read = Reference::try_new(variant.metadata(), variant.value())
            .expect("the format's own reader accepts the leaf");
        assert_eq!(read, expected, "{value:?}");
        assert_eq!(variant.scalar().expect("the leaf decodes"), value);
    }
}

#[test]
fn the_clock_leaves_exchange_at_the_precision_the_specification_names() {
    let date = Scalar::date32(20_000);
    let variant = Variant::encode(&date).expect("a date encodes");
    let read = Reference::try_new(variant.metadata(), variant.value()).expect("a date reads");
    assert!(matches!(read, Reference::Date(_)), "{read:?}");
    assert_eq!(variant.scalar().expect("a date decodes"), date);

    let instant = Scalar::datetime64(1_700_000_000_000_000, TimeUnit::Microsecond, Timezone::UTC)
        .expect("an instant");
    let variant = Variant::encode(&instant).expect("an instant encodes");
    let read = Reference::try_new(variant.metadata(), variant.value()).expect("an instant reads");
    assert!(matches!(read, Reference::TimestampMicros(_)), "{read:?}");
    assert_eq!(variant.scalar().expect("an instant decodes"), instant);

    // A second-resolution instant is exact in microseconds, which is the
    // precision the specification states.
    let seconds =
        Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::NAIVE).expect("an instant");
    let variant = Variant::encode(&seconds).expect("a second instant encodes");
    let read = Reference::try_new(variant.metadata(), variant.value()).expect("it reads");
    assert!(matches!(read, Reference::TimestampNtzMicros(_)), "{read:?}");
    assert_eq!(
        variant.scalar().expect("it decodes"),
        Scalar::datetime64(
            1_700_000_000_000_000,
            TimeUnit::Microsecond,
            Timezone::NAIVE
        )
        .expect("the same instant in microseconds")
    );

    let time = Scalar::time64(3_600_000_000, TimeUnit::Microsecond, Timezone::NAIVE)
        .expect("a time of day");
    let variant = Variant::encode(&time).expect("a time encodes");
    let read = Reference::try_new(variant.metadata(), variant.value()).expect("a time reads");
    assert!(matches!(read, Reference::Time(_)), "{read:?}");
    assert_eq!(variant.scalar().expect("a time decodes"), time);
}

#[test]
fn a_long_string_and_a_wide_object_exchange_at_their_own_widths() {
    // Past sixty-three bytes a string states its own four-byte size, and
    // past two hundred and fifty-five fields an object states a four-byte
    // count: both are widths the format crate must agree on.
    let long = "x".repeat(1000);
    let fields: Vec<(String, Scalar)> = (0..300_i32)
        .map(|index| (format!("field{index:04}"), Scalar::from(index)))
        .collect();
    let mut entries: Vec<(&str, Scalar)> = fields
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect();
    entries.push(("long", Scalar::from(long.as_str())));
    let wide = Scalar::from_struct(entries).expect("a wide value builds");

    let variant = Variant::encode(&wide).expect("a wide value encodes");
    let read = Reference::try_new(variant.metadata(), variant.value())
        .expect("the format's own reader accepts a wide object");
    let object = read.as_object().expect("an object");
    assert_eq!(object.len(), 301);
    assert_eq!(object.get("field0299"), Some(Reference::from(299_i32)));
    assert_eq!(object.get("long"), Some(Reference::from(long.as_str())));
    assert_eq!(variant.scalar().expect("a wide value decodes"), wide);
}

/// The format crate's UUID value for sixteen bytes.
fn uuid_of(bytes: [u8; 16]) -> parquet_variant::Uuid {
    parquet_variant::Uuid::from_bytes(bytes)
}
