use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::{UInt32Type, UInt64Type};
use arrow_array::{
    Array, ArrayRef, FixedSizeBinaryArray, Int64Array, RecordBatch, StringArray, StructArray,
    UInt64Array,
};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};

use super::{column_digests, row_digests};
use crate::xxhash::{Xxh3, Xxh32, Xxh64, Xxh128};
use crate::{DataType, Digest, DigestAlgorithm, Field, Scalar, TimeUnit, Timezone};

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from_fields(fields).unwrap().required_field("row")
}

fn batch(fields: &[Field], columns: Vec<ArrayRef>) -> RecordBatch {
    let fields = fields
        .iter()
        .cloned()
        .map(Field::into_arrow)
        .collect::<crate::Result<Vec<_>>>()
        .unwrap();
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap()
}

fn holder(name: &str, dtype: DataType) -> Field {
    let mut field = Field::new(name, dtype, false);
    field.as_digest_mut().set_holder().unwrap();
    field
}

fn row_digest(value: impl Into<Scalar>, algorithm: DigestAlgorithm) -> Digest {
    Scalar::from_sequence([value.into()]).digest(algorithm)
}

fn seeded_xxh64_row(seed: u64, value: impl Into<Scalar>) -> u64 {
    let mut state = Xxh64::with_seed(seed);
    state.write_scalar(&Scalar::from_sequence([value.into()]));
    state.as_u64()
}

/// Read one digest column back as the digests it holds.
fn digests(array: &ArrayRef, algorithm: DigestAlgorithm) -> Vec<Digest> {
    match algorithm {
        DigestAlgorithm::Xxh32 => array
            .as_primitive::<UInt32Type>()
            .values()
            .iter()
            .map(|value| Digest::new(algorithm, u128::from(*value)))
            .collect(),
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => array
            .as_primitive::<UInt64Type>()
            .values()
            .iter()
            .map(|value| Digest::new(algorithm, u128::from(*value)))
            .collect(),
        DigestAlgorithm::Xxh128 => {
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
fn row_digest_roles_exclude_holders_and_explicit_components_narrow_the_input() {
    let symbol = Arc::new(StringArray::from(vec!["AAPL", "MSFT"])) as ArrayRef;
    let quantity = Arc::new(Int64Array::from(vec![100, 250])) as ArrayRef;
    let stored = Arc::new(Int64Array::from(vec![11, 22])) as ArrayRef;

    let plain = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
            ArrowField::new("quantity", ArrowDataType::Int64, false),
        ])),
        vec![Arc::clone(&symbol), Arc::clone(&quantity)],
    )
    .unwrap();

    let mut holder = Field::new("row_digest", DataType::Int64, false);
    holder.as_digest_mut().set_holder().unwrap();
    let fallback = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
            ArrowField::new("quantity", ArrowDataType::Int64, false),
            holder.into_arrow().unwrap(),
        ])),
        vec![
            Arc::clone(&symbol),
            Arc::clone(&quantity),
            Arc::clone(&stored),
        ],
    )
    .unwrap();

    let mut component = Field::new("quantity", DataType::Int64, false);
    component.as_digest_mut().set_component().unwrap();
    let mut holder = Field::new("row_digest", DataType::Int64, false);
    holder.as_digest_mut().set_holder().unwrap();
    let explicit = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
            component.into_arrow().unwrap(),
            holder.into_arrow().unwrap(),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["changed", "too"])),
            Arc::clone(&quantity),
            Arc::new(Int64Array::from(vec![99, 88])),
        ],
    )
    .unwrap();
    let quantity_only = RecordBatch::try_new(
        Arc::new(Schema::new(vec![ArrowField::new(
            "quantity",
            ArrowDataType::Int64,
            false,
        )])),
        vec![Arc::clone(&quantity)],
    )
    .unwrap();

    for algorithm in DigestAlgorithm::ALL {
        assert_eq!(
            digests(&row_digests(&fallback, algorithm).unwrap(), algorithm),
            digests(&row_digests(&plain, algorithm).unwrap(), algorithm),
            "a holder is excluded for {algorithm}"
        );
        assert_eq!(
            digests(&row_digests(&explicit, algorithm).unwrap(), algorithm),
            digests(&row_digests(&quantity_only, algorithm).unwrap(), algorithm),
            "an explicit component is the whole input for {algorithm}"
        );
    }
}

#[test]
fn rows_with_only_digest_holders_hash_as_empty_sequences() {
    let mut holder = Field::new("row_digest", DataType::Int64, false);
    holder.as_digest_mut().set_holder().unwrap();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![holder.into_arrow().unwrap()])),
        vec![Arc::new(Int64Array::from(vec![11, 22]))],
    )
    .unwrap();

    for algorithm in DigestAlgorithm::ALL {
        let expected = Scalar::from_sequence(Vec::<Scalar>::new()).digest(algorithm);
        assert_eq!(
            digests(&row_digests(&batch, algorithm).unwrap(), algorithm),
            [expected, expected],
            "{algorithm}"
        );
    }
}

#[test]
fn a_null_never_collides_with_an_empty_value() {
    let field = Field::new("symbol", DataType::Utf8, true);
    let values = Scalar::from_sequence([Scalar::Null, Scalar::from("")]);
    let array = crate::arrow::array_from_value(&field, &values).unwrap();
    let column = digests(
        &column_digests(array.as_ref(), &field, DigestAlgorithm::Xxh3).unwrap(),
        DigestAlgorithm::Xxh3,
    );
    assert_ne!(column[0], column[1]);
    assert_eq!(column[0], Scalar::Null.digest(DigestAlgorithm::Xxh3));
    assert_eq!(column[1], Scalar::from("").digest(DigestAlgorithm::Xxh3));
}

#[test]
fn the_column_width_follows_the_algorithm() {
    let field = Field::new("quantity", DataType::Int64, false);
    let values = Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]);
    let array = crate::arrow::array_from_value(&field, &values).unwrap();

    let widths = [
        (DigestAlgorithm::Xxh32, ArrowDataType::UInt32),
        (DigestAlgorithm::Xxh64, ArrowDataType::UInt64),
        (DigestAlgorithm::Xxh3, ArrowDataType::UInt64),
        (DigestAlgorithm::Xxh128, ArrowDataType::FixedSizeBinary(16)),
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
        &row_digests(&batch, DigestAlgorithm::Xxh3).unwrap(),
        DigestAlgorithm::Xxh3,
    );
    assert_eq!(
        column[0],
        Scalar::from_sequence([Scalar::from(100)]).digest(DigestAlgorithm::Xxh3)
    );
    assert_ne!(column[0], Scalar::from(100).digest(DigestAlgorithm::Xxh3));
}

#[test]
fn fill_casts_missing_holders_and_preserves_or_forces_existing_values() {
    let value = DataType::Int64.required_field("value");
    let digest = holder("digest", DataType::UInt64);
    let root = root([value.clone(), digest.clone()]);
    let values = Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef;
    let source = batch(std::slice::from_ref(&value), vec![Arc::clone(&values)]);

    let mut state = Xxh64::with_seed(7);
    state.write_bytes(b"prior bytes are not row input");
    let before = state.as_u64();
    let filled = state.fill_arrow_batch(&root, source, false).unwrap();
    assert_eq!(state.as_u64(), before, "the prototype is unchanged");
    assert!(Arc::ptr_eq(filled.column(0), &values));
    assert_eq!(
        filled.column(1).as_primitive::<UInt64Type>().values(),
        &[seeded_xxh64_row(7, 1), seeded_xxh64_row(7, 2)]
    );

    let populated = Arc::new(UInt64Array::from(vec![0, 99])) as ArrayRef;
    let source = batch(
        &[value.clone(), digest],
        vec![Arc::clone(&values), populated],
    );
    let conditional = state
        .fill_arrow_batch(&root, source.clone(), false)
        .unwrap();
    assert_eq!(
        conditional.column(1).as_primitive::<UInt64Type>().values(),
        &[seeded_xxh64_row(7, 1), 99]
    );
    let forced = state.fill_arrow_batch(&root, source, true).unwrap();
    assert_eq!(
        forced.column(1).as_primitive::<UInt64Type>().values(),
        &[seeded_xxh64_row(7, 1), seeded_xxh64_row(7, 2)]
    );

    let empty = batch(
        std::slice::from_ref(&value),
        vec![Arc::new(Int64Array::from(Vec::<i64>::new()))],
    );
    let empty = state.fill_arrow_batch(&root, empty, false).unwrap();
    assert_eq!(empty.num_rows(), 0);
    assert_eq!(empty.num_columns(), 2);
}

#[test]
fn holder_paths_are_ordered_override_roles_and_preserve_explicit_empty() {
    let mut a = DataType::Int64.required_field("a");
    a.as_digest_mut().set_component().unwrap();
    let b = DataType::Utf8.required_field("b");
    let mut ordered = holder("ordered", DataType::UInt64);
    ordered.as_digest_mut().set_paths(["b", "a"]).unwrap();
    let mut empty = holder("empty", DataType::UInt64);
    empty.as_digest_mut().set_paths(Vec::<&str>::new()).unwrap();
    let root = root([a, b, ordered, empty]);
    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from(7),
        Scalar::from("AAPL"),
        Scalar::from(0_u64),
        Scalar::from(0_u64),
    ])]);
    let source = crate::arrow::batch_from_value(&root, &rows).unwrap();
    let filled = Xxh3::new().fill_arrow_batch(&root, source, false).unwrap();

    let expected_ordered = Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(7)])
        .digest(DigestAlgorithm::Xxh3)
        .as_u64()
        .unwrap();
    let expected_empty = Scalar::from_sequence(Vec::<Scalar>::new())
        .digest(DigestAlgorithm::Xxh3)
        .as_u64()
        .unwrap();
    assert_eq!(
        filled.column(2).as_primitive::<UInt64Type>().value(0),
        expected_ordered
    );
    assert_eq!(
        filled.column(3).as_primitive::<UInt64Type>().value(0),
        expected_empty
    );
    assert_ne!(
        expected_ordered,
        row_digest(7, DigestAlgorithm::Xxh3).as_u64().unwrap(),
        "paths override the explicit component role"
    );
}

#[test]
fn nested_holders_fill_bottom_up_and_hidden_rows_stay_untouched() {
    let inner_value = DataType::Int64.required_field("value");
    let inner_digest = holder("digest", DataType::UInt64);
    let nested = DataType::from_fields([inner_value, inner_digest])
        .unwrap()
        .nullable_field("nested");
    let outer_digest = holder("digest", DataType::UInt64);
    let root = root([nested, outer_digest]);
    let rows = Scalar::from_sequence([
        Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from(7), Scalar::from(0_u64)]),
            Scalar::from(0_u64),
        ]),
        Scalar::from_sequence([Scalar::Null, Scalar::from(0_u64)]),
    ]);
    let source = crate::arrow::batch_from_value(&root, &rows).unwrap();
    let filled = Xxh3::new().fill_arrow_batch(&root, source, true).unwrap();

    let nested = filled
        .column(0)
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    let inner = nested.column(1).as_primitive::<UInt64Type>();
    let inner_expected = row_digest(7, DigestAlgorithm::Xxh3).as_u64().unwrap();
    assert_eq!(inner.value(0), inner_expected);
    assert_eq!(inner.value(1), 0, "a null parent hides its child holder");

    let outer = filled.column(1).as_primitive::<UInt64Type>();
    assert_eq!(
        outer.value(0),
        row_digest(inner_expected, DigestAlgorithm::Xxh3)
            .as_u64()
            .unwrap(),
        "the containing row consumes the filled nested holder"
    );
    assert_eq!(
        outer.value(1),
        row_digest(Scalar::Null, DigestAlgorithm::Xxh3)
            .as_u64()
            .unwrap(),
        "the shortcut retains the selected Struct's null"
    );
}

#[test]
fn mixed_holder_widths_resolve_algorithms_per_holder() {
    let value = DataType::Int64.required_field("value");
    let h32 = holder("h32", DataType::UInt32);
    let h64 = holder("h64", DataType::UInt64);
    let mut explicit = holder("explicit", DataType::UInt64);
    explicit
        .as_digest_mut()
        .set_algorithm(DigestAlgorithm::Xxh3)
        .unwrap();
    let h128 = holder("h128", DataType::FixedSizeBinary(16));
    let root = root([value.clone(), h32, h64, explicit, h128]);
    let source = batch(
        std::slice::from_ref(&value),
        vec![Arc::new(Int64Array::from(vec![17]))],
    );

    let mut state = Xxh64::with_seed(9);
    state.write_bytes(b"ignored");
    let before = state.as_u64();
    let filled = state.fill_arrow_batch(&root, source, false).unwrap();
    assert_eq!(state.as_u64(), before);
    assert_eq!(
        filled.column(1).as_primitive::<UInt32Type>().value(0),
        row_digest(17, DigestAlgorithm::Xxh32).as_u32().unwrap(),
        "mismatched uint32 selects unseeded xxh32"
    );
    assert_eq!(
        filled.column(2).as_primitive::<UInt64Type>().value(0),
        seeded_xxh64_row(9, 17),
        "a matching receiver contributes its seed"
    );
    assert_eq!(
        filled.column(3).as_primitive::<UInt64Type>().value(0),
        row_digest(17, DigestAlgorithm::Xxh3).as_u64().unwrap(),
        "an explicit different algorithm is fresh and unseeded"
    );
    let wide = filled
        .column(4)
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(
        wide.value(0),
        &*row_digest(17, DigestAlgorithm::Xxh128).into_bytes(),
        "mismatched fixed-size binary selects unseeded xxh3-128"
    );
}

#[test]
fn every_concrete_state_and_the_dispatcher_expose_batch_fill() {
    let empty_root = root(Vec::<Field>::new());
    let empty = RecordBatch::new_empty(Arc::new(Schema::empty()));
    assert_eq!(
        Xxh32::new()
            .fill_arrow_batch(&empty_root, empty.clone(), false)
            .unwrap()
            .num_rows(),
        0
    );
    assert_eq!(
        Xxh64::new()
            .fill_arrow_batch(&empty_root, empty.clone(), false)
            .unwrap()
            .num_rows(),
        0
    );
    assert_eq!(
        Xxh3::new()
            .fill_arrow_batch(&empty_root, empty.clone(), false)
            .unwrap()
            .num_rows(),
        0
    );
    assert_eq!(
        Xxh128::new()
            .fill_arrow_batch(&empty_root, empty.clone(), false)
            .unwrap()
            .num_rows(),
        0
    );
    assert_eq!(
        DigestAlgorithm::Xxh3
            .digester()
            .fill_arrow_batch(&empty_root, empty, false)
            .unwrap()
            .num_rows(),
        0
    );
}

fn empty_batch() -> RecordBatch {
    RecordBatch::new_empty(Arc::new(Schema::empty()))
}

fn assert_metadata_error(error: crate::arrow::Error, key: &str, holder: &str) {
    match error {
        crate::arrow::Error::Core(crate::Error::InvalidMetadataValue {
            key: actual,
            reason,
        }) => {
            assert_eq!(actual.as_str(), key);
            assert!(reason.contains(holder), "{reason}");
        }
        other => panic!("expected {key} metadata error, got {other}"),
    }
}

#[test]
fn invalid_holder_algorithms_and_metadata_ownership_are_rejected() {
    let wrong_width = Field::from_parts(
        "digest",
        DataType::UInt32,
        false,
        [("digest:role", "holder"), ("digest:algorithm", "xxh3-64")],
    )
    .unwrap();
    let error = Xxh3::new()
        .fill_arrow_batch(&root([wrong_width]), empty_batch(), false)
        .unwrap_err();
    assert_metadata_error(error, "digest:algorithm", "$.digest");

    let non_holder_algorithm = Field::from_parts(
        "value",
        DataType::UInt64,
        false,
        [("digest:algorithm", "xxh3-64")],
    )
    .unwrap();
    let error = Xxh3::new()
        .fill_arrow_batch(&root([non_holder_algorithm]), empty_batch(), false)
        .unwrap_err();
    assert_metadata_error(error, "digest:algorithm", "$.value");

    let non_holder_paths =
        Field::from_parts("value", DataType::UInt64, false, [("digest:paths", "[]")]).unwrap();
    let error = Xxh3::new()
        .fill_arrow_batch(&root([non_holder_paths]), empty_batch(), false)
        .unwrap_err();
    assert_metadata_error(error, "digest:paths", "$.value");

    let non_struct_root = DataType::Int64.required_field("value");
    let error = Xxh3::new()
        .fill_arrow_batch(&non_struct_root, empty_batch(), false)
        .unwrap_err();
    assert!(
        matches!(error, crate::arrow::Error::IncompatibleSchema(_)),
        "an invalid batch root remains a schema error"
    );
}

#[test]
fn digest_paths_reject_peer_outputs_ambiguity_duplicates_and_collection_descent() {
    let peer = holder("peer", DataType::UInt64);
    let mut selecting_peer = holder("digest", DataType::UInt64);
    selecting_peer.as_digest_mut().set_paths(["peer"]).unwrap();
    let error = Xxh3::new()
        .fill_arrow_batch(&root([peer, selecting_peer]), empty_batch(), false)
        .unwrap_err();
    assert_metadata_error(error, "digest:paths", "$.digest");

    let nested_value = DataType::Int64.required_field("value");
    let nested_holder = holder("digest", DataType::UInt64);
    let nested = DataType::from_fields([nested_value, nested_holder])
        .unwrap()
        .required_field("nested");
    let mut duplicate = holder("digest", DataType::UInt64);
    duplicate
        .as_digest_mut()
        .set_paths(["nested", "nested.digest"])
        .unwrap();
    let error = Xxh3::new()
        .fill_arrow_batch(&root([nested.clone(), duplicate]), empty_batch(), false)
        .unwrap_err();
    assert_metadata_error(error, "digest:paths", "$.digest");

    let nested = DataType::from_fields([
        holder("left", DataType::UInt64),
        holder("right", DataType::UInt64),
    ])
    .unwrap()
    .required_field("nested");
    let mut ambiguous = holder("digest", DataType::UInt64);
    ambiguous.as_digest_mut().set_paths(["nested"]).unwrap();
    let error = Xxh3::new()
        .fill_arrow_batch(&root([nested, ambiguous]), empty_batch(), false)
        .unwrap_err();
    assert_metadata_error(error, "digest:paths", "$.digest");

    let items = DataType::from_str("array<struct<value:int64>>")
        .unwrap()
        .required_field("items");
    let mut collection = holder("digest", DataType::UInt64);
    collection
        .as_digest_mut()
        .set_paths(["items.value"])
        .unwrap();
    let error = Xxh3::new()
        .fill_arrow_batch(&root([items, collection]), empty_batch(), false)
        .unwrap_err();
    assert_metadata_error(error, "digest:paths", "$.digest");
}

#[test]
fn digest_paths_try_later_literal_prefixes_and_allow_terminal_collections() {
    let scalar_prefix = DataType::Int64.required_field("a");
    let dotted_prefix = DataType::from_fields([DataType::Int64.required_field("c")])
        .unwrap()
        .required_field("a.b");
    let items = DataType::from_str("array<int64>")
        .unwrap()
        .required_field("items");
    let mut digest = holder("digest", DataType::UInt64);
    digest
        .as_digest_mut()
        .set_paths(["a.b.c", "items"])
        .unwrap();
    let root = root([scalar_prefix, dotted_prefix, items, digest]);
    let item_value = Scalar::from_sequence([Scalar::from(3), Scalar::from(4)]);
    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from(1),
        Scalar::from_sequence([Scalar::from(2)]),
        item_value.clone(),
        Scalar::from(0_u64),
    ])]);
    let source = crate::arrow::batch_from_value(&root, &rows).unwrap();
    let filled = Xxh3::new().fill_arrow_batch(&root, source, false).unwrap();
    let expected = Scalar::from_sequence([Scalar::from(2), item_value])
        .digest(DigestAlgorithm::Xxh3)
        .as_u64()
        .unwrap();
    assert_eq!(
        filled.column(3).as_primitive::<UInt64Type>().value(0),
        expected
    );
}
