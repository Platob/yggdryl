use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::{UInt32Type, UInt64Type};
use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};

use super::{column_digests, row_digests};
use crate::{DataType, Digest, DigestAlgorithm, Field, Scalar, TimeUnit, Timezone};

/// Read one digest column back as the digests it holds.
fn digests(array: &ArrayRef, algorithm: DigestAlgorithm) -> Vec<Digest> {
    match algorithm {
        DigestAlgorithm::Xxh32 => array
            .as_primitive::<UInt32Type>()
            .values()
            .iter()
            .map(|value| Digest::new(algorithm, u128::from(*value)))
            .collect(),
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3_64 => array
            .as_primitive::<UInt64Type>()
            .values()
            .iter()
            .map(|value| Digest::new(algorithm, u128::from(*value)))
            .collect(),
        DigestAlgorithm::Xxh3_128 => {
            let array = array
                .as_any()
                .downcast_ref::<FixedSizeBinaryArray>()
                .expect("the 128-bit column is fixed-size binary");
            (0..array.len())
                .map(|index| {
                    Digest::from_bytes(algorithm, array.value(index)).expect("the exact width")
                })
                .collect()
        }
    }
}

/// One column per datatype family, each with the values that exercise it.
///
/// Every family the core can read is here, because the contract is that the
/// buffer path and the fallback answer the same thing on all of them.
fn columns() -> Vec<(Field, Scalar)> {
    let utc = Timezone::UTC;
    vec![
        (
            Field::new("null", DataType::Null, true),
            Scalar::from_sequence([Scalar::Null, Scalar::Null]),
        ),
        (
            Field::new("boolean", DataType::Boolean, true),
            Scalar::from_sequence([Scalar::from(true), Scalar::from(false), Scalar::Null]),
        ),
        (
            Field::new("int8", DataType::Int8, true),
            Scalar::from_sequence([Scalar::from(-1), Scalar::from(0), Scalar::Null]),
        ),
        (
            Field::new("int16", DataType::Int16, false),
            Scalar::from_sequence([Scalar::from(i16::MIN), Scalar::from(7), Scalar::from(0)]),
        ),
        (
            Field::new("int32", DataType::Int32, false),
            Scalar::from_sequence([Scalar::from(i32::MIN), Scalar::from(0x31), Scalar::from(0)]),
        ),
        (
            Field::new("int64", DataType::Int64, true),
            Scalar::from_sequence([Scalar::from(i64::MIN), Scalar::from(187), Scalar::Null]),
        ),
        (
            Field::new("uint8", DataType::UInt8, false),
            Scalar::from_sequence([Scalar::from(0), Scalar::from(0x31), Scalar::from(u8::MAX)]),
        ),
        (
            Field::new("uint16", DataType::UInt16, false),
            Scalar::from_sequence([Scalar::from(0), Scalar::from(7), Scalar::from(u16::MAX)]),
        ),
        (
            Field::new("uint32", DataType::UInt32, false),
            Scalar::from_sequence([Scalar::from(0), Scalar::from(7), Scalar::from(u32::MAX)]),
        ),
        (
            Field::new("uint64", DataType::UInt64, true),
            Scalar::from_sequence([Scalar::from(u64::MAX), Scalar::from(7), Scalar::Null]),
        ),
        (
            Field::new("float16", DataType::Float16, true),
            Scalar::from_sequence([
                Scalar::from_float(1.5, 16).unwrap(),
                Scalar::from_float(f64::NAN, 16).unwrap(),
                Scalar::from_float(-0.0, 16).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("float32", DataType::Float32, true),
            Scalar::from_sequence([
                Scalar::from_float(1.5, 32).unwrap(),
                Scalar::from_float(f64::NAN, 32).unwrap(),
                Scalar::from_float(-0.0, 32).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("float64", DataType::Float64, true),
            Scalar::from_sequence([
                Scalar::from_float(1.5, 64).unwrap(),
                Scalar::from_float(f64::NAN, 64).unwrap(),
                Scalar::from_float(-0.0, 64).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("decimal128", DataType::decimal128(12, 2).unwrap(), true),
            Scalar::from_sequence([
                Scalar::d128(18_723, 2),
                Scalar::d128(-100, 2),
                Scalar::d128(0, 2),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("decimal256", DataType::decimal256(40, 3).unwrap(), true),
            Scalar::from_sequence([Scalar::d128(18_723, 3), Scalar::d128(-1, 3), Scalar::Null]),
        ),
        (
            Field::new("utf8", DataType::Utf8, true),
            Scalar::from_sequence([
                Scalar::from(""),
                Scalar::from("AAPL"),
                Scalar::from("é—wide"),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("large_utf8", DataType::LargeUtf8, true),
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::Null]),
        ),
        (
            Field::new("utf8_view", DataType::Utf8View, true),
            Scalar::from_sequence([
                Scalar::from("a short one"),
                Scalar::from("a long one that will not fit inline in a view buffer"),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("ascii(4)", DataType::FixedAscii(4), true),
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from("F"), Scalar::Null]),
        ),
        (
            Field::new("binary", DataType::Binary, true),
            Scalar::from_sequence([
                Scalar::from(Arc::from(b"".as_slice())),
                Scalar::from(Arc::from(b"\x00\xff".as_slice())),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("large_binary", DataType::LargeBinary, true),
            Scalar::from_sequence([Scalar::from(Arc::from(b"AAPL".as_slice())), Scalar::Null]),
        ),
        (
            Field::new("binary_view", DataType::BinaryView, true),
            Scalar::from_sequence([Scalar::from(Arc::from(b"AAPL".as_slice())), Scalar::Null]),
        ),
        (
            Field::new("fixed_size_binary", DataType::FixedSizeBinary(4), true),
            Scalar::from_sequence([Scalar::from(Arc::from(b"AAPL".as_slice())), Scalar::Null]),
        ),
        (
            Field::new("date32", DataType::Date32, true),
            Scalar::from_sequence([
                Scalar::date32_in(20_000, TimeUnit::Day, Timezone::NAIVE).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("date64", DataType::Date64, true),
            Scalar::from_sequence([
                Scalar::date64_in(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("time32", DataType::time32(TimeUnit::Second).unwrap(), true),
            Scalar::from_sequence([
                Scalar::time32(3_600, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new(
                "time64",
                DataType::time64(TimeUnit::Nanosecond).unwrap(),
                true,
            ),
            Scalar::from_sequence([
                Scalar::time64(1, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new(
                "timestamp_utc",
                DataType::DateTime64 {
                    unit: TimeUnit::Microsecond,
                    timezone: utc,
                },
                true,
            ),
            Scalar::from_sequence([
                Scalar::datetime64(1_700_000_000_000_000, TimeUnit::Microsecond, utc).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new(
                "timestamp_naive",
                DataType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::NAIVE,
                },
                true,
            ),
            Scalar::from_sequence([
                Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new("duration64", DataType::Duration64(TimeUnit::Second), true),
            Scalar::from_sequence([
                Scalar::duration64_in(90, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new(
                "list",
                DataType::list(Field::new("item", DataType::Int64, true)),
                true,
            ),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]),
                Scalar::from_sequence([]),
                Scalar::Null,
            ]),
        ),
        (
            Field::new(
                "struct",
                DataType::from_fields([
                    Field::new("symbol", DataType::Utf8, false),
                    Field::new("quantity", DataType::Int64, true),
                ])
                .unwrap(),
                true,
            ),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100)]),
                Scalar::from_sequence([Scalar::from("MSFT"), Scalar::Null]),
                Scalar::Null,
            ]),
        ),
        (
            Field::new(
                "map",
                DataType::map_of(DataType::Utf8, DataType::Int64, false).unwrap(),
                true,
            ),
            Scalar::from_sequence([
                Scalar::from_mapping([(Scalar::from("AAPL"), Scalar::from(100))]).unwrap(),
                Scalar::from_mapping([]).unwrap(),
                Scalar::Null,
            ]),
        ),
        (
            Field::new(
                "dictionary",
                DataType::from_str("dictionary<int32, utf8>").unwrap(),
                true,
            ),
            Scalar::from_sequence([
                Scalar::from("AAPL"),
                Scalar::from("AAPL"),
                Scalar::from("MSFT"),
                Scalar::Null,
            ]),
        ),
        (
            Field::new(
                "union",
                DataType::from_str("dense_union<a: int64, b: utf8>").unwrap(),
                true,
            ),
            // A union value is the `[type_id, payload]` pair its encoding
            // stores, and its validity lives in the child rather than the
            // parent - which is exactly the case the null shortcut skips.
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from(0), Scalar::from(1)]),
                Scalar::from_sequence([Scalar::from(1), Scalar::from("AAPL")]),
            ]),
        ),
        (
            Field::new("geometry", DataType::from_str("geometry").unwrap(), true),
            Scalar::from_sequence([
                // A minimal little-endian WKB point.
                Scalar::Geospatial(crate::types::Geospatial::Geometry(
                    crate::types::Geometry::new([
                        1_u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    ])
                    .unwrap(),
                )),
                Scalar::Null,
            ]),
        ),
    ]
}

#[test]
fn a_column_digest_equals_the_value_feed_on_every_datatype_family() {
    for (field, values) in columns() {
        let array = crate::arrow::array_from_value(&field, &values)
            .unwrap_or_else(|error| panic!("{}: {error}", field.name()));
        // Read the values back through the shared boundary rather than reusing
        // the input, so a column that canonicalizes on the way in is compared
        // against what it actually stores.
        let stored = crate::arrow::array_to_value(&field, array.as_ref())
            .unwrap_or_else(|error| panic!("{}: {error}", field.name()));
        let stored = stored.as_sequence().expect("a sequence of values");

        for algorithm in DigestAlgorithm::ALL {
            let column = column_digests(array.as_ref(), &field, algorithm)
                .unwrap_or_else(|error| panic!("{}: {error}", field.name()));
            assert_eq!(column.len(), stored.len(), "{}", field.name());
            assert_eq!(
                digests(&column, algorithm),
                stored
                    .iter()
                    .map(|value| value.digest(algorithm))
                    .collect::<Vec<_>>(),
                "{} under {algorithm}",
                field.name()
            );
        }
    }
}

#[test]
fn a_row_digest_equals_the_row_value_feed_on_every_datatype_family() {
    // One batch holding every family at once, so the row framing is exercised
    // across the buffer path and the fallback in the same row.
    let width = columns()
        .iter()
        .map(|(_, values)| values.len())
        .max()
        .expect("the corpus is not empty");
    let mut fields = Vec::new();
    let mut arrays: Vec<ArrayRef> = Vec::new();
    for (field, values) in columns() {
        // Pad every column to the same height, which also puts a null in
        // every family that can hold one. A union stores its validity in the
        // child rather than the parent, so it repeats a real member instead.
        let mut padded = values.as_sequence().expect("a sequence").to_vec();
        let filler = if matches!(field.dtype(), DataType::Union(..)) {
            padded
                .first()
                .cloned()
                .expect("the union column is not empty")
        } else {
            Scalar::Null
        };
        padded.resize(width, filler);
        let field = field.clone().with_nullable(true);
        let array = crate::arrow::array_from_value(&field, &Scalar::from_sequence(padded))
            .unwrap_or_else(|error| panic!("{}: {error}", field.name()));
        fields.push(field);
        arrays.push(array);
    }
    let root = DataType::from_fields(fields).unwrap().required_field("row");
    let arrow_fields: Vec<ArrowField> = root
        .dtype()
        .as_fields()
        .expect("a struct root")
        .iter()
        .map(|field| field.clone().into_arrow())
        .collect::<crate::Result<Vec<_>>>()
        .unwrap();
    let batch = RecordBatch::try_new(Arc::new(Schema::new(arrow_fields)), arrays).unwrap();

    let rows = crate::arrow::batch_to_value(&batch).unwrap();
    let rows = rows.as_sequence().expect("a sequence of rows");
    assert_eq!(rows.len(), width);

    for algorithm in DigestAlgorithm::ALL {
        let column = row_digests(&batch, algorithm).unwrap();
        assert_eq!(column.len(), width);
        assert_eq!(
            digests(&column, algorithm),
            rows.iter()
                .map(|row| row.digest(algorithm))
                .collect::<Vec<_>>(),
            "{algorithm}"
        );
    }
}

#[test]
fn identical_rows_answer_identical_digests() {
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
            ArrowField::new("quantity", ArrowDataType::Int64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])) as ArrayRef,
            Arc::new(Int64Array::from(vec![100, 250, 100])),
        ],
    )
    .unwrap();

    for algorithm in DigestAlgorithm::ALL {
        let column = digests(&row_digests(&batch, algorithm).unwrap(), algorithm);
        assert_eq!(column[0], column[2], "{algorithm}");
        assert_ne!(column[0], column[1], "{algorithm}");
        assert_eq!(column[0].algorithm(), algorithm);
    }
}

#[test]
fn a_null_never_collides_with_an_empty_value() {
    let field = Field::new("symbol", DataType::Utf8, true);
    let values = Scalar::from_sequence([Scalar::Null, Scalar::from("")]);
    let array = crate::arrow::array_from_value(&field, &values).unwrap();
    let column = digests(
        &column_digests(array.as_ref(), &field, DigestAlgorithm::Xxh3_64).unwrap(),
        DigestAlgorithm::Xxh3_64,
    );
    assert_ne!(column[0], column[1]);
    assert_eq!(column[0], Scalar::Null.digest(DigestAlgorithm::Xxh3_64));
    assert_eq!(column[1], Scalar::from("").digest(DigestAlgorithm::Xxh3_64));
}

#[test]
fn the_column_width_follows_the_algorithm() {
    let field = Field::new("quantity", DataType::Int64, false);
    let values = Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]);
    let array = crate::arrow::array_from_value(&field, &values).unwrap();

    let widths = [
        (DigestAlgorithm::Xxh32, ArrowDataType::UInt32),
        (DigestAlgorithm::Xxh64, ArrowDataType::UInt64),
        (DigestAlgorithm::Xxh3_64, ArrowDataType::UInt64),
        (
            DigestAlgorithm::Xxh3_128,
            ArrowDataType::FixedSizeBinary(16),
        ),
    ];
    for (algorithm, expected) in widths {
        let column = column_digests(array.as_ref(), &field, algorithm).unwrap();
        assert_eq!(column.data_type(), &expected, "{algorithm}");
        assert_eq!(column.null_count(), 0, "a digest is never null");
    }
}

#[test]
fn an_empty_batch_answers_an_empty_column() {
    let batch = RecordBatch::new_empty(Arc::new(Schema::new(vec![ArrowField::new(
        "symbol",
        ArrowDataType::Utf8,
        false,
    )])));
    for algorithm in DigestAlgorithm::ALL {
        assert_eq!(row_digests(&batch, algorithm).unwrap().len(), 0);
    }
}

#[test]
fn a_row_is_the_sequence_of_its_columns() {
    // The framing itself: a one-column row is not the same as the bare value,
    // because a row is a sequence and a sequence carries its count.
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![ArrowField::new(
            "quantity",
            ArrowDataType::Int64,
            false,
        )])),
        vec![Arc::new(Int64Array::from(vec![100])) as ArrayRef],
    )
    .unwrap();
    let column = digests(
        &row_digests(&batch, DigestAlgorithm::Xxh3_64).unwrap(),
        DigestAlgorithm::Xxh3_64,
    );
    assert_eq!(
        column[0],
        Scalar::from_sequence([Scalar::from(100)]).digest(DigestAlgorithm::Xxh3_64)
    );
    assert_ne!(
        column[0],
        Scalar::from(100).digest(DigestAlgorithm::Xxh3_64)
    );
}
