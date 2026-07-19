//! Functional tests for the [`typed`](yggdryl_core::typed) serialization layer — the
//! `Encoder`/`Decoder` round-trip into an `IOBase`, the `FixedScalar` / `FixedSerie` value carriers
//! (nullable and non-nullable), the vectorized `Reduce` aggregations, the `Bit` boolean column,
//! filtering, and the `HeaderField` metadata — plus the edges (empty, all-null, out-of-range, NaN).

use yggdryl_core::datatype_id::DataTypeId;
use yggdryl_core::io::memory::{Heap, IOBase, IoError};
use yggdryl_core::typed::fixedbit::Bit;
use yggdryl_core::typed::fixedbyte::{
    Decimal128, Decimal256, Decimal32, Decimal64, Float64, Int128, Int32, Int64, Int8, UInt128,
    UInt8, I256,
};
use yggdryl_core::typed::{
    Binary, Decimal, Decoder, Encoder, Field, FixedBinary, FixedScalar, FixedSerie, FixedSizeSerie,
    FixedUtf8, FlexibleFromStr, HeaderField, Scalar, Serie, Utf8, VarScalar, VarSerie,
};

// -------------------------------------------------------------------------------------
// Encoder / Decoder — the byte round-trip over any IOBase
// -------------------------------------------------------------------------------------

#[test]
fn encoder_decoder_round_trip_scalar_and_bulk() {
    let mut h = Heap::new();
    Int32::encode(&mut h, 0, -42).unwrap();
    Int32::encode(&mut h, 1, 7).unwrap();
    assert_eq!(Int32::decode(&h, 0).unwrap(), -42);
    assert_eq!(Int32::decode(&h, 1).unwrap(), 7);

    // Bulk: encode a slice at an element offset, decode it back.
    let mut b = Heap::new();
    Int64::encode_slice(&mut b, 0, &[1, -2, 3, -4]).unwrap();
    let mut out = [0i64; 4];
    Int64::decode_slice(&b, 0, &mut out).unwrap();
    assert_eq!(out, [1, -2, 3, -4]);

    // The widest + the byte type both round-trip.
    let mut w = Heap::new();
    UInt128::encode(&mut w, 3, u128::MAX).unwrap();
    assert_eq!(UInt128::decode(&w, 3).unwrap(), u128::MAX);
    let mut bytes = Heap::new();
    UInt8::encode_slice(&mut bytes, 0, &[0xAB, 0xCD, 0xEF]).unwrap();
    assert_eq!(UInt8::decode(&bytes, 2).unwrap(), 0xEF);
}

// -------------------------------------------------------------------------------------
// Flexible string <-> value parsing — Encoder::encode_str / Decoder::decode_str, scalar + bulk
// -------------------------------------------------------------------------------------

#[test]
fn flexible_str_parse_scalar_int_round_trip() {
    // Each tolerant form parses through encode_str and decodes to the expected i64.
    for (text, expected) in [
        ("1,000,000", 1_000_000i64),
        ("  +42 ", 42),
        ("0xFF", 255),
        ("0b1010", 10),
        ("0o17", 15),
        ("1e3", 1000),
        ("-2_500", -2500),
    ] {
        let mut h = Heap::new();
        Int64::encode_str(&mut h, 0, text).unwrap();
        assert_eq!(Int64::decode(&h, 0).unwrap(), expected, "parsing {text:?}");
        // decode_str renders the canonical decimal back.
        assert_eq!(Int64::decode_str(&h, 0).unwrap(), expected.to_string());
    }

    // A fractional value for an integer target is a guided ParseError naming the fix.
    let mut h = Heap::new();
    let err = Int64::encode_str(&mut h, 0, "1.5").unwrap_err();
    assert!(matches!(err, IoError::ParseError { .. }));
    assert!(err.to_string().contains("fractional"));
}

#[test]
fn flexible_str_parse_float_and_bool() {
    // Floats: scientific, thousands separators, and the special values.
    let mut f = Heap::new();
    Float64::encode_str(&mut f, 0, "1.5e3").unwrap();
    assert_eq!(Float64::decode(&f, 0).unwrap(), 1500.0);
    Float64::encode_str(&mut f, 1, "1,234.5").unwrap();
    assert_eq!(Float64::decode(&f, 1).unwrap(), 1234.5);
    Float64::encode_str(&mut f, 2, "inf").unwrap();
    assert!(Float64::decode(&f, 2).unwrap().is_infinite());
    Float64::encode_str(&mut f, 3, "NaN").unwrap();
    assert!(Float64::decode(&f, 3).unwrap().is_nan());

    // bool: case-insensitive words and 1/0, round-tripped through the Bit column encoder.
    let mut b = Heap::new();
    Bit::encode_str(&mut b, 0, "YES").unwrap();
    Bit::encode_str(&mut b, 1, "0").unwrap();
    assert!(Bit::decode(&b, 0).unwrap());
    assert!(!Bit::decode(&b, 1).unwrap());
    assert_eq!(Bit::decode_str(&b, 0).unwrap(), "true");
}

#[test]
fn flexible_str_parse_bulk_and_exact() {
    // Bulk: one vectorized encode of the parsed values, then a bulk decode back to strings.
    let mut h = Heap::new();
    Int64::encode_str_slice(&mut h, 0, &["1", "2_0", "0x3"]).unwrap();
    assert_eq!(
        Int64::decode_str_slice(&h, 0, 3).unwrap(),
        vec!["1".to_string(), "20".to_string(), "3".to_string()]
    );

    // parse_exact refuses a thousands separator that parse_flexible accepts.
    assert_eq!(
        <i64 as FlexibleFromStr>::parse_flexible("1,000").unwrap(),
        1000
    );
    assert!(matches!(
        <i64 as FlexibleFromStr>::parse_exact("1,000").unwrap_err(),
        IoError::ParseError { .. }
    ));
    // The strict bulk encoder surfaces the same error on the comma element.
    let mut strict = Heap::new();
    assert!(Int64::encode_str_exact_slice(&mut strict, 0, &["1", "1,000"]).is_err());
}

#[test]
fn fixed_serie_from_strings_and_to_strings() {
    // Column-level flexible parse: builds a non-nullable in-heap column.
    let col = FixedSerie::<Int64>::from_strings(&["1,000", "2_0", "0x3"]).unwrap();
    assert_eq!(col.len(), 3);
    assert_eq!(col.null_count(), 0);
    assert_eq!(col.values(), vec![1000, 20, 3]);

    // Round-trip back to strings (validity-ignored, mirrors `values()`).
    assert_eq!(
        col.to_strings().unwrap(),
        vec!["1000".to_string(), "20".to_string(), "3".to_string()]
    );

    // A nullable column: to_string_options is null-aware (None at the null slot).
    let nullable = FixedSerie::<Int64>::from_options(&[Some(7), None, Some(9)]);
    assert_eq!(
        nullable.to_string_options().unwrap(),
        vec![Some("7".to_string()), None, Some("9".to_string())]
    );
    // to_strings ignores validity — the null slot renders its stored default (0).
    assert_eq!(
        nullable.to_strings().unwrap(),
        vec!["7".to_string(), "0".to_string(), "9".to_string()]
    );

    // The strict constructor rejects a thousands separator that from_strings accepts.
    // (`.err().unwrap()` avoids requiring the Ok column type to be Debug.)
    assert!(matches!(
        FixedSerie::<Int64>::from_strings_exact(&["1", "1,000"])
            .err()
            .unwrap(),
        IoError::ParseError { .. }
    ));
}

// -------------------------------------------------------------------------------------
// FixedScalar — the single-element Scalar
// -------------------------------------------------------------------------------------

#[test]
fn fixed_scalar_value_null_and_option() {
    let some = FixedScalar::<Int32>::of(42);
    assert_eq!(some.value(), Some(42));
    assert_eq!(some.len(), 1);
    assert!(some.is_valid(0));
    assert_eq!(some.get(0), Some(42));
    assert_eq!(some.data_type_id(), DataTypeId::I32);

    let null = FixedScalar::<Int32>::null();
    assert_eq!(null.value(), None);
    assert!(null.is_null(0));
    assert_eq!(null.null_count(), 1);

    assert_eq!(FixedScalar::<Int64>::from_option(Some(7)).value(), Some(7));
    assert_eq!(FixedScalar::<Int64>::from_option(None).value(), None);
}

// -------------------------------------------------------------------------------------
// FixedSerie — the typed column
// -------------------------------------------------------------------------------------

#[test]
fn serie_from_values_reads_and_reduces() {
    let col = FixedSerie::<Int64>::from_values(&[4, 8, 15, 16, 23, 42]);
    assert_eq!(col.len(), 6);
    assert!(!col.is_empty());
    assert_eq!(col.null_count(), 0);
    assert_eq!(col.get(0), Some(4));
    assert_eq!(col.get(5), Some(42));
    assert_eq!(col.get(6), None); // out of range
    assert_eq!(col.values(), vec![4, 8, 15, 16, 23, 42]);
    assert_eq!(
        col.to_options(),
        (0..6).map(|i| col.get(i)).collect::<Vec<_>>()
    );

    // Reductions route to the data buffer's vectorized Aggregate kernels.
    assert_eq!(col.sum().unwrap(), 108i128);
    assert_eq!(col.min().unwrap(), Some(4));
    assert_eq!(col.max().unwrap(), Some(42));
    assert_eq!(col.mean().unwrap(), Some(18.0));
}

#[test]
fn serie_nullable_from_options_and_push() {
    let col = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3), None, Some(5)]);
    assert_eq!(col.len(), 5);
    assert_eq!(col.null_count(), 2);
    assert!(col.is_valid(0) && col.is_null(1) && col.is_valid(2) && col.is_null(3));
    assert_eq!(col.get(1), None);
    assert_eq!(col.get(2), Some(3));
    assert_eq!(
        col.to_options(),
        vec![Some(1), None, Some(3), None, Some(5)]
    );

    // Building by push — a null after non-nulls back-fills the validity buffer.
    let mut built = FixedSerie::<Int8>::new();
    built.push(1);
    built.push(-2);
    built.push_null();
    built.push_option(Some(4));
    built.push_option(None);
    assert_eq!(built.len(), 5);
    assert_eq!(built.null_count(), 2);
    assert_eq!(
        built.to_options(),
        vec![Some(1), Some(-2), None, Some(4), None]
    );
}

#[test]
fn serie_empty_edges() {
    let empty = FixedSerie::<Int32>::new();
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.values(), Vec::<i32>::new());
    assert_eq!(empty.to_options(), Vec::<Option<i32>>::new());
    assert_eq!(empty.sum().unwrap(), 0);
    assert_eq!(empty.min().unwrap(), None);
    assert_eq!(empty.max().unwrap(), None);
    assert_eq!(empty.mean().unwrap(), None);

    // An all-null column.
    let all_null = FixedSerie::<Int64>::from_options(&[None, None, None]);
    assert_eq!(all_null.len(), 3);
    assert_eq!(all_null.null_count(), 3);
    assert!(all_null.get(0).is_none() && all_null.get(2).is_none());
}

#[test]
fn serie_wide_and_unsigned_reductions() {
    assert_eq!(
        FixedSerie::<UInt128>::from_values(&[1, 2, 3])
            .sum()
            .unwrap(),
        6u128
    );
    assert_eq!(
        FixedSerie::<Int128>::from_values(&[-5, 10, -20])
            .min()
            .unwrap(),
        Some(-20)
    );
    assert_eq!(
        FixedSerie::<UInt8>::from_values(&[10, 250, 3])
            .max()
            .unwrap(),
        Some(250)
    );
}

#[test]
fn float_serie_min_max_ignore_nan() {
    let col = FixedSerie::<Float64>::from_values(&[1.5, f64::NAN, 2.5, 0.5]);
    assert_eq!(col.min().unwrap(), Some(0.5)); // NaN never poisons min/max
    assert_eq!(col.max().unwrap(), Some(2.5));
    // A sum that includes a NaN is NaN (checked via is_nan, not equality).
    assert!(col.sum().unwrap().is_nan());
    // Over the non-NaN part the sum + mean are exact.
    let clean = FixedSerie::<Float64>::from_values(&[1.5, 2.5, 0.5]);
    assert_eq!(clean.sum().unwrap(), 4.5);
    assert_eq!(clean.mean().unwrap(), Some(1.5));
}

// -------------------------------------------------------------------------------------
// Numeric reductions — the full Reduce set on a FixedSerie (var / std / median / first /
// last / count_ge, over and above sum / min / max / mean)
// -------------------------------------------------------------------------------------

#[test]
fn serie_numeric_full_reductions() {
    // A dataset with an exact population variance / std and an even-count median.
    let col = FixedSerie::<Int64>::from_values(&[2, 4, 4, 4, 5, 5, 7, 9]);
    assert_eq!(col.mean().unwrap(), Some(5.0));
    assert_eq!(col.var().unwrap(), Some(4.0)); // population variance = 32/8
    assert_eq!(col.std().unwrap(), Some(2.0)); // sqrt(var)
    assert_eq!(col.median().unwrap(), Some(4.5)); // even count -> mean of the two middle values
    assert_eq!(col.first().unwrap(), Some(2));
    assert_eq!(col.last().unwrap(), Some(9));
    assert_eq!(col.count_ge(5).unwrap(), 4); // 5, 5, 7, 9

    // Odd count -> the single middle element (median sorts; first / last stay positional).
    let odd = FixedSerie::<Int64>::from_values(&[3, 1, 2]);
    assert_eq!(odd.median().unwrap(), Some(2.0)); // sorted [1, 2, 3] -> 2
    assert_eq!(odd.first().unwrap(), Some(3)); // positional, not sorted
    assert_eq!(odd.last().unwrap(), Some(2));
}

// -------------------------------------------------------------------------------------
// Universal aggregations — the type-agnostic Serie defaults (count / valid_count /
// first_value / last_value / n_unique / min_value / max_value), for every element type
// -------------------------------------------------------------------------------------

#[test]
fn serie_universal_aggregations_across_types() {
    // Integer column: min_value / max_value are ordering-based (numeric order here).
    let ints = FixedSerie::<Int64>::from_values(&[5, 1, 3, 9, 3]);
    assert_eq!(ints.count(), 5);
    assert_eq!(ints.valid_count(), 5);
    assert_eq!(ints.first_value(), Some(5));
    assert_eq!(ints.last_value(), Some(3));
    assert_eq!(ints.min_value(), Some(1));
    assert_eq!(ints.max_value(), Some(9));
    assert_eq!(ints.n_unique(), 4); // {5, 1, 3, 9}

    // Utf8 VarSerie: lexicographic min / max, n_unique with a duplicate.
    let words = VarSerie::<Utf8>::from_values(&[
        "banana".to_string(),
        "apple".to_string(),
        "cherry".to_string(),
        "apple".to_string(),
    ]);
    assert_eq!(words.count(), 4);
    assert_eq!(words.first_value().as_deref(), Some("banana"));
    assert_eq!(words.last_value().as_deref(), Some("apple"));
    assert_eq!(words.min_value().as_deref(), Some("apple")); // lexicographic
    assert_eq!(words.max_value().as_deref(), Some("cherry"));
    assert_eq!(words.n_unique(), 3); // apple counted once

    // Bool column: false < true.
    let bools = FixedSerie::<Bit>::from_values(&[true, false, true, true]);
    assert_eq!(bools.count(), 4);
    assert_eq!(bools.valid_count(), 4);
    assert_eq!(bools.first_value(), Some(true));
    assert_eq!(bools.last_value(), Some(true));
    assert_eq!(bools.min_value(), Some(false));
    assert_eq!(bools.max_value(), Some(true));
    assert_eq!(bools.n_unique(), 2); // {true, false}

    // Nullable column: nulls excluded from n_unique / min_value; valid_count counts non-nulls.
    let nullable = FixedSerie::<Int32>::from_options(&[Some(4), None, Some(4), Some(7), None]);
    assert_eq!(nullable.count(), 5); // total, nulls included
    assert_eq!(nullable.valid_count(), 3); // 4, 4, 7
    assert_eq!(nullable.n_unique(), 2); // {4, 7}
    assert_eq!(nullable.min_value(), Some(4));
    assert_eq!(nullable.max_value(), Some(7));
    assert_eq!(nullable.first_value(), Some(4)); // index 0 is valid
    assert_eq!(nullable.last_value(), None); // index 4 is null -> null-aware None
}

#[test]
fn serie_universal_empty_and_float_edges() {
    // Empty column: every universal aggregation is None / 0.
    let empty = FixedSerie::<Int64>::new();
    assert_eq!(empty.count(), 0);
    assert_eq!(empty.valid_count(), 0);
    assert_eq!(empty.n_unique(), 0);
    assert_eq!(empty.first_value(), None);
    assert_eq!(empty.last_value(), None);
    assert_eq!(empty.min_value(), None);
    assert_eq!(empty.max_value(), None);
    // The numeric reductions are None on empty too.
    assert_eq!(empty.var().unwrap(), None);
    assert_eq!(empty.std().unwrap(), None);
    assert_eq!(empty.median().unwrap(), None);
    assert_eq!(empty.first().unwrap(), None);
    assert_eq!(empty.last().unwrap(), None);
    assert_eq!(empty.count_ge(0).unwrap(), 0);

    // A float column has the numeric mean / std / var / median...
    let floats = FixedSerie::<Float64>::from_values(&[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(floats.mean().unwrap(), Some(2.5));
    assert_eq!(floats.var().unwrap(), Some(1.25)); // population variance = 5/4
    assert_eq!(floats.std().unwrap(), Some(1.25f64.sqrt()));
    assert_eq!(floats.median().unwrap(), Some(2.5)); // sorted [1,2,3,4] -> (2+3)/2
                                                     // ...but NO `min_value` / `max_value`: `f64` is not `Ord`, so those methods do not exist for a
                                                     // float column (uncommenting `floats.min_value()` would fail to compile). It uses the NaN-safe
                                                     // numeric `min` / `max` instead:
    assert_eq!(floats.min().unwrap(), Some(1.0));
    assert_eq!(floats.max().unwrap(), Some(4.0));
}

// -------------------------------------------------------------------------------------
// Bit — the boolean column (bit-granular)
// -------------------------------------------------------------------------------------

#[test]
fn bit_serie_packs_and_reads() {
    let col = FixedSerie::<Bit>::from_values(&[true, false, true, true, false]);
    assert_eq!(col.len(), 5);
    assert_eq!(col.get(0), Some(true));
    assert_eq!(col.get(1), Some(false));
    assert_eq!(col.get(4), Some(false));
    assert_eq!(col.values(), vec![true, false, true, true, false]);
    assert_eq!(col.data_type_id(), DataTypeId::Bool);

    // Nullable booleans.
    let nullable = FixedSerie::<Bit>::from_options(&[Some(true), None, Some(false)]);
    assert_eq!(nullable.null_count(), 1);
    assert_eq!(nullable.to_options(), vec![Some(true), None, Some(false)]);
}

// -------------------------------------------------------------------------------------
// Filter
// -------------------------------------------------------------------------------------

#[test]
fn serie_filter_by_bit_mask() {
    let col = FixedSerie::<Int32>::from_values(&[10, 20, 30, 40, 50]);
    let mut mask = Heap::new();
    for (index, keep) in [true, false, true, false, true].iter().enumerate() {
        mask.pwrite_bit(index as u64, *keep).unwrap();
    }
    let filtered = col.filter(&mask);
    assert_eq!(filtered.len(), 3);
    assert_eq!(filtered.values(), vec![10, 30, 50]);

    // Filtering a nullable column preserves the surviving nulls.
    let nullable = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3), None]);
    let mut keep_all = Heap::new();
    for i in 0..4 {
        keep_all.pwrite_bit(i, true).unwrap();
    }
    let kept = nullable.filter(&keep_all);
    assert_eq!(kept.to_options(), vec![Some(1), None, Some(3), None]);
}

// -------------------------------------------------------------------------------------
// Field — the column metadata (Headers-backed)
// -------------------------------------------------------------------------------------

#[test]
fn header_field_metadata_and_serie_field() {
    let field = HeaderField::new(Some("price"), DataTypeId::I64, true);
    assert_eq!(field.name(), Some("price"));
    assert_eq!(field.data_type_id(), DataTypeId::I64);
    assert!(field.nullable());
    // The metadata really lives in the Headers map.
    assert_eq!(field.headers().type_id(), DataTypeId::I64);
    assert_eq!(field.headers().name(), Some("price"));
    assert!(field.headers().nullable());

    // A serie reports its own field: name from `with_name`, nullable from validity presence.
    let non_null = FixedSerie::<Int64>::from_values(&[1, 2, 3]).with_name("id");
    assert_eq!(non_null.field().name(), Some("id"));
    assert_eq!(non_null.field().data_type_id(), DataTypeId::I64);
    assert!(!non_null.field().nullable());

    let nullable = FixedSerie::<Int32>::from_options(&[Some(1), None]);
    assert!(nullable.field().nullable());
    assert_eq!(nullable.field().data_type_id(), DataTypeId::I32);
}

// -------------------------------------------------------------------------------------
// HeaderField — metadata accessors / mutators + the set_* / with_* trio
// -------------------------------------------------------------------------------------

#[test]
fn header_field_metadata_accessors_and_trio() {
    let mut field = HeaderField::new(Some("price"), DataTypeId::I64, false);

    // set_metadata / metadata_value round-trip (an arbitrary annotation key).
    field.set_metadata("unit", "USD");
    assert_eq!(field.metadata_value("unit").as_deref(), Some("USD"));
    assert_eq!(field.metadata_value("missing"), None);

    // metadata() exposes the whole backing map; metadata_mut() mutates it, and the change reflects.
    assert_eq!(field.metadata().get("unit"), Some("USD"));
    field.metadata_mut().insert("currency", "fiat");
    assert_eq!(field.metadata_value("currency").as_deref(), Some("fiat"));

    // with_metadata is the chainable form.
    let annotated = HeaderField::new(None, DataTypeId::I32, false)
        .with_metadata("a", "1")
        .with_metadata("b", "2");
    assert_eq!(annotated.metadata_value("a").as_deref(), Some("1"));
    assert_eq!(annotated.metadata_value("b").as_deref(), Some("2"));

    // The set_* / with_* trio over the promoted typed fields (name / nullable / data_type_id).
    let built = HeaderField::new(None, DataTypeId::Unknown, false)
        .with_name("id")
        .with_nullable(true)
        .with_data_type_id(DataTypeId::I32);
    assert_eq!(built.name(), Some("id"));
    assert!(built.nullable());
    assert_eq!(built.data_type_id(), DataTypeId::I32);

    let mut mutated = built.clone();
    mutated.set_name("key");
    mutated.set_nullable(false);
    mutated.set_data_type_id(DataTypeId::I64);
    assert_eq!(mutated.name(), Some("key"));
    assert!(!mutated.nullable());
    assert_eq!(mutated.data_type_id(), DataTypeId::I64);
}

// -------------------------------------------------------------------------------------
// Field-driven cast — FixedSerie / FixedScalar cast_field (metadata reshape, element type kept)
// -------------------------------------------------------------------------------------

#[test]
fn serie_cast_field_nullability_name_metadata() {
    // non-nullable -> nullable: adds an all-valid validity buffer (no nulls introduced).
    let base = FixedSerie::<Int64>::from_values(&[1, 2, 3]);
    assert!(!base.field().nullable());
    let nullable = base
        .cast_field(&HeaderField::new(None, DataTypeId::I64, true))
        .unwrap();
    assert!(nullable.field().nullable());
    assert_eq!(nullable.null_count(), 0);
    assert!(nullable.is_valid(0) && nullable.is_valid(1) && nullable.is_valid(2));
    assert_eq!(nullable.values(), vec![1, 2, 3]);

    // nullable (but clean) -> non-nullable: drops the validity buffer.
    let clean = FixedSerie::<Int32>::from_options(&[Some(1), Some(2)]);
    assert!(clean.field().nullable());
    let non_null = clean
        .cast_field(&HeaderField::new(None, DataTypeId::I32, false))
        .unwrap();
    assert!(!non_null.field().nullable());
    assert_eq!(non_null.values(), vec![1, 2]);

    // A name + metadata cast: the new field reports both.
    let named = FixedSerie::<Int64>::from_values(&[10, 20])
        .cast_field(
            &HeaderField::new(Some("price"), DataTypeId::I64, false).with_metadata("unit", "USD"),
        )
        .unwrap();
    assert_eq!(named.field().name(), Some("price"));
    assert_eq!(named.field().metadata_value("unit").as_deref(), Some("USD"));
}

#[test]
fn serie_cast_field_guided_errors_and_noop() {
    // nullable -> non-nullable with a real null: the guided TypedCast error naming the count.
    // (`.err().unwrap()` avoids requiring the Ok column type to be Debug.)
    let with_null = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3)]);
    let err = with_null
        .cast_field(&HeaderField::new(None, DataTypeId::I32, false))
        .err()
        .unwrap();
    assert!(matches!(err, IoError::TypedCast { .. }));
    assert!(err.to_string().contains("nulls"));

    // Different dtype: the guided TypedCast error (the typed column keeps its element type).
    let mut col = FixedSerie::<Int64>::from_values(&[1, 2, 3]);
    let dtype_err = col
        .cast_field_in_place(&HeaderField::new(None, DataTypeId::F64, false))
        .unwrap_err();
    assert!(matches!(dtype_err, IoError::TypedCast { .. }));
    assert!(dtype_err.to_string().contains("resize_dtype"));

    // No-op when the field already matches: the backing bytes are untouched (same allocation).
    let mut named = FixedSerie::<Int64>::from_values(&[4, 5, 6]).with_name("x");
    let same = named.field();
    let ptr_before = named.data().as_slice().as_ptr();
    named.cast_field_in_place(&same).unwrap();
    assert_eq!(named.data().as_slice().as_ptr(), ptr_before); // no reallocation
    assert_eq!(named.values(), vec![4, 5, 6]);

    // The copy front door is a no-op in content and name too.
    let copy = named.cast_field(&same).unwrap();
    assert_eq!(copy.values(), named.values());
    assert_eq!(copy.field().name(), Some("x"));
}

#[test]
fn scalar_cast_field_nullability_name_and_errors() {
    // non-nullable scalar -> nullable field: marks the one element valid, keeps its value + name +
    // annotation.
    let some = FixedScalar::<Int32>::of(42);
    assert!(!some.field().nullable());
    let nullable = some
        .cast_field(
            &HeaderField::new(Some("answer"), DataTypeId::I32, true).with_metadata("src", "quiz"),
        )
        .unwrap();
    assert!(nullable.field().nullable());
    assert_eq!(nullable.field().name(), Some("answer"));
    assert_eq!(
        nullable.field().metadata_value("src").as_deref(),
        Some("quiz")
    );
    assert_eq!(nullable.value(), Some(42));
    assert!(nullable.is_valid(0));

    // a null scalar -> non-nullable field: the guided TypedCast error (a real null).
    let null = FixedScalar::<Int32>::null();
    let err = null
        .cast_field(&HeaderField::new(None, DataTypeId::I32, false))
        .err()
        .unwrap();
    assert!(matches!(err, IoError::TypedCast { .. }));

    // different dtype: the guided TypedCast error (names the FixedScalar container).
    let dtype_err = some
        .cast_field(&HeaderField::new(None, DataTypeId::I64, false))
        .err()
        .unwrap();
    assert!(matches!(dtype_err, IoError::TypedCast { .. }));
    assert!(dtype_err.to_string().contains("FixedScalar"));
}

// -------------------------------------------------------------------------------------
// Wrapping any IOBase as a typed column (zero-copy view)
// -------------------------------------------------------------------------------------

#[test]
fn serie_from_data_views_an_existing_buffer() {
    // A caller writes i32s into some IOBase (a Heap here; a mapped file / device buffer works the
    // same via the generic `D`), then views it as a typed column without copying.
    let mut buffer = Heap::new();
    buffer.pwrite_i32_array(0, &[100, 200, 300, 400]).unwrap();
    let column: FixedSerie<Int32, Heap> = FixedSerie::from_data(buffer, None, 4);
    assert_eq!(column.len(), 4);
    assert_eq!(column.get(0), Some(100));
    assert_eq!(column.get(3), Some(400));
    assert_eq!(column.values(), vec![100, 200, 300, 400]);
    assert_eq!(column.sum().unwrap(), 1000i64);

    // With a validity buffer the view is null-aware too.
    let mut data = Heap::new();
    data.pwrite_i32_array(0, &[1, 0, 3]).unwrap();
    let mut validity = Heap::new();
    validity.pwrite_bit(0, true).unwrap();
    validity.pwrite_bit(1, false).unwrap();
    validity.pwrite_bit(2, true).unwrap();
    let nullable: FixedSerie<Int32, Heap> = FixedSerie::from_data(data, Some(validity), 3);
    assert_eq!(nullable.to_options(), vec![Some(1), None, Some(3)]);
    assert_eq!(nullable.null_count(), 1);
}

// -------------------------------------------------------------------------------------
// Fixed-point decimals — Decimal32/64/128/256 + the shared Decimal trait + precision/scale
// -------------------------------------------------------------------------------------

#[test]
fn decimal_serie_precision_scale_and_format() {
    // Money as Decimal128 scale 2: the stored value is the unscaled integer; the string is scaled.
    let col = FixedSerie::<Decimal128>::from_values(&[12345, 5, -5, 100000])
        .with_name("price")
        .with_precision_scale(10, 2);
    assert_eq!(col.len(), 4);
    assert_eq!(col.get(0), Some(12345i128)); // the raw unscaled value
    assert_eq!(col.to_decimal_string(0).as_deref(), Some("123.45"));
    assert_eq!(col.to_decimal_string(1).as_deref(), Some("0.05"));
    assert_eq!(col.to_decimal_string(2).as_deref(), Some("-0.05"));
    assert_eq!(col.to_decimal_string(3).as_deref(), Some("1000.00"));
    assert_eq!(col.decimal_scale(), 2);
    assert_eq!(col.decimal_precision(), 10);

    // The field carries the decimal metadata (in its Headers).
    let field = col.field();
    assert_eq!(field.name(), Some("price"));
    assert_eq!(field.data_type_id(), DataTypeId::Decimal128);
    assert_eq!(field.precision(), Some(10));
    assert_eq!(field.scale(), Some(2));
    assert!(field.data_type_id().is_decimal());
}

#[test]
fn decimal_widths_and_trait() {
    assert_eq!(
        FixedSerie::<Decimal32>::from_values(&[999, -1]).get(1),
        Some(-1i32)
    );
    assert_eq!(
        FixedSerie::<Decimal64>::from_values(&[i64::MAX]).get(0),
        Some(i64::MAX)
    );
    // The shared Decimal trait: max precision per width + scale-aware format.
    assert_eq!(Decimal32::MAX_PRECISION, 9);
    assert_eq!(Decimal64::MAX_PRECISION, 18);
    assert_eq!(Decimal128::MAX_PRECISION, 38);
    assert_eq!(Decimal256::MAX_PRECISION, 76);
    assert_eq!(Decimal64::format(-12345, 3), "-12.345");
    assert_eq!(Decimal32::format(7, 0), "7");
}

#[test]
fn decimal256_i256_round_trip_and_format() {
    let col = FixedSerie::<Decimal256>::from_values(&[
        I256::from_i128(12345678901234567890i128),
        I256::from_i128(-1),
        I256::ZERO,
    ])
    .with_precision_scale(40, 4);
    assert_eq!(col.len(), 3);
    assert_eq!(col.get(1), Some(I256::from_i128(-1)));
    assert_eq!(
        col.to_decimal_string(0).as_deref(),
        Some("1234567890123456.7890")
    );
    assert_eq!(col.to_decimal_string(2).as_deref(), Some("0.0000"));

    // I256 native basics: i128 interop, ordering, byte round-trip.
    assert_eq!(I256::from_i128(42).to_i128(), Some(42));
    assert!(I256::from_i128(-5) < I256::from_i128(5));
    assert!(I256::ZERO < I256::from_i128(1));
    let bytes = I256::from_i128(-1).to_le_bytes();
    assert_eq!(I256::from_le_bytes(bytes), I256::from_i128(-1));
    assert_eq!(I256::from_i128(-1).to_string(), "-1");
}

#[test]
fn decimal_encoder_decoder_direct() {
    let mut h = Heap::new();
    Decimal128::encode(&mut h, 0, 999i128).unwrap();
    Decimal128::encode(&mut h, 1, -1i128).unwrap();
    assert_eq!(Decimal128::decode(&h, 0).unwrap(), 999);
    assert_eq!(Decimal128::decode(&h, 1).unwrap(), -1);

    // Decimal256 encodes its 32 LE bytes per element.
    let mut d256 = Heap::new();
    Decimal256::encode_slice(&mut d256, 0, &[I256::from_i128(7), I256::from_i128(-9)]).unwrap();
    assert_eq!(Decimal256::decode(&d256, 0).unwrap(), I256::from_i128(7));
    assert_eq!(Decimal256::decode(&d256, 1).unwrap(), I256::from_i128(-9));

    // A nullable decimal column via from_options.
    let nullable = FixedSerie::<Decimal64>::from_options(&[Some(100), None, Some(250)])
        .with_precision_scale(6, 2);
    assert_eq!(nullable.null_count(), 1);
    assert_eq!(nullable.to_decimal_string(0).as_deref(), Some("1.00"));
    assert_eq!(nullable.to_decimal_string(1), None); // null
    assert_eq!(nullable.to_decimal_string(2).as_deref(), Some("2.50"));
}

// -------------------------------------------------------------------------------------
// Variable-length: Binary / Utf8 (offsets + data layout)
// -------------------------------------------------------------------------------------

#[test]
fn var_binary_serie() {
    let col =
        VarSerie::<Binary>::from_values(&[b"hello".to_vec(), b"".to_vec(), b"world!".to_vec()]);
    assert_eq!(col.len(), 3);
    assert_eq!(col.get(0), Some(b"hello".to_vec()));
    assert_eq!(col.get(1), Some(b"".to_vec())); // an empty (zero-length) element
    assert_eq!(col.get(2), Some(b"world!".to_vec()));
    assert_eq!(col.get(3), None); // out of range
    assert_eq!(col.data_type_id(), DataTypeId::Binary);
    assert_eq!(col.bytes_at(2), Some(b"world!".to_vec()));

    // push + nullable + raw bytes front door.
    let mut built = VarSerie::<Binary>::new();
    built.push(&b"a".to_vec());
    built.push_null();
    built.push_bytes(b"bc");
    assert_eq!(built.len(), 3);
    assert_eq!(built.null_count(), 1);
    assert_eq!(built.get(1), None);
    assert_eq!(
        built.to_options(),
        vec![Some(b"a".to_vec()), None, Some(b"bc".to_vec())]
    );
}

#[test]
fn var_utf8_serie() {
    let col = VarSerie::<Utf8>::from_options(&[
        Some("héllo".to_string()),
        None,
        Some(String::new()),
        Some("世界".to_string()),
    ]);
    assert_eq!(col.len(), 4);
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get(0).as_deref(), Some("héllo")); // multibyte
    assert_eq!(col.get(1), None);
    assert_eq!(col.get(3).as_deref(), Some("世界"));
    assert_eq!(col.data_type_id(), DataTypeId::Utf8);
    assert_eq!(col.field().data_type_id(), DataTypeId::Utf8);
    assert!(col.field().nullable());
    // values() ignores validity: the null slot (index 1) is a zero-length span -> "".
    assert_eq!(col.values(), vec!["héllo", "", "", "世界"]);
}

#[test]
fn var_scalar() {
    let some = VarScalar::<Utf8>::of("hi".to_string());
    assert_eq!(some.value(), Some(&"hi".to_string()));
    assert_eq!(some.len(), 1);
    assert!(some.is_valid(0));
    assert_eq!(some.get(0).as_deref(), Some("hi"));

    let null = VarScalar::<Binary>::null();
    assert_eq!(null.value(), None);
    assert!(null.is_null(0));
    assert_eq!(
        VarScalar::<Binary>::from_option(Some(vec![1, 2])).get(0),
        Some(vec![1, 2])
    );
}

// -------------------------------------------------------------------------------------
// Fixed-size (parameterized width): FixedBinary / FixedUtf8
// -------------------------------------------------------------------------------------

#[test]
fn fixed_binary_serie_padded_and_truncated() {
    let col = FixedSizeSerie::<FixedBinary>::from_values(
        4,
        &[b"ab".to_vec(), b"cdef".to_vec(), b"ghijk".to_vec()],
    );
    assert_eq!(col.width(), 4);
    assert_eq!(col.len(), 3);
    assert_eq!(col.get(0), Some(b"ab\0\0".to_vec())); // zero-padded to 4
    assert_eq!(col.get(1), Some(b"cdef".to_vec())); // exact width
    assert_eq!(col.get(2), Some(b"ghij".to_vec())); // truncated to 4
    assert_eq!(col.data_type_id(), DataTypeId::FixedBinary);

    let field = col.field();
    assert_eq!(field.byte_width(), Some(4)); // the parameterized length rides the field metadata
    assert!(field.data_type_id().is_binary());
}

#[test]
fn fixed_utf8_serie_nullable() {
    let col = FixedSizeSerie::<FixedUtf8>::from_options(
        3,
        &[Some("ab".to_string()), None, Some("xyz".to_string())],
    )
    .with_name("code");
    assert_eq!(col.width(), 3);
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get(0).as_deref(), Some("ab\0")); // zero-padded to 3
    assert_eq!(col.get(1), None);
    assert_eq!(col.get(2).as_deref(), Some("xyz"));
    assert_eq!(col.field().name(), Some("code"));
    assert_eq!(col.field().byte_width(), Some(3));
    assert!(col.field().data_type_id().is_utf8());
}

// -------------------------------------------------------------------------------------
// Element + range mutators (set / set_checked / set_null / set_range / set_range_serie)
// and the read-range slice — the in-place edit surface on the typed columns.
// -------------------------------------------------------------------------------------

#[test]
fn serie_set_and_set_checked_replace_elements() {
    let mut col = FixedSerie::<Int32>::from_values(&[10, 20, 30, 40]);
    col.set(1, 99).unwrap();
    assert_eq!(col.get(1), Some(99));
    assert_eq!(col.values(), vec![10, 99, 30, 40]);

    // set_checked — the unchecked fast path (caller pre-validated index < len).
    col.set_checked(3, -7);
    assert_eq!(col.get(3), Some(-7));
    assert_eq!(
        col.to_options(),
        vec![Some(10), Some(99), Some(30), Some(-7)]
    );

    // Out-of-range set returns the guided (window-past-end) error.
    let err = col.set(4, 0).unwrap_err();
    assert!(matches!(err, IoError::SliceOutOfBounds { .. }));
}

#[test]
fn serie_set_re_validates_null_and_set_null_nulls() {
    let mut col = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3), None, Some(5)]);
    assert_eq!(col.null_count(), 2);

    // A `set` past a nullable column's null re-validates it (flips the slot to present).
    col.set(1, 22).unwrap();
    assert_eq!(col.get(1), Some(22));
    assert!(col.is_valid(1));
    assert_eq!(col.null_count(), 1);

    // set_null nulls a valid slot; null_count updates.
    col.set_null(0).unwrap();
    assert!(col.is_null(0));
    assert_eq!(col.get(0), None);
    assert_eq!(col.null_count(), 2);
    assert_eq!(
        col.to_options(),
        vec![None, Some(22), Some(3), None, Some(5)]
    );

    // set_null on a previously non-nullable column back-fills a validity buffer.
    let mut plain = FixedSerie::<Int64>::from_values(&[7, 8, 9]);
    assert_eq!(plain.null_count(), 0);
    plain.set_null(1).unwrap();
    assert_eq!(plain.null_count(), 1);
    assert_eq!(plain.to_options(), vec![Some(7), None, Some(9)]);

    // Out-of-range set_null is the guided error.
    assert!(matches!(
        plain.set_null(3).unwrap_err(),
        IoError::SliceOutOfBounds { .. }
    ));
}

#[test]
fn serie_set_range_and_set_range_serie() {
    let mut col = FixedSerie::<Int64>::from_values(&[1, 2, 3, 4, 5, 6]);
    col.set_range(2, &[30, 40, 50]).unwrap();
    assert_eq!(col.values(), vec![1, 2, 30, 40, 50, 6]);

    // Unchecked bulk twin.
    col.set_range_checked(0, &[-1, -2]);
    assert_eq!(col.values(), vec![-1, -2, 30, 40, 50, 6]);

    // Out-of-range set_range returns the guided error.
    let err = col.set_range(4, &[0, 0, 0]).unwrap_err();
    assert!(matches!(err, IoError::SliceOutOfBounds { .. }));

    // set_range_serie copies values AND validity from another column (nullable source makes the
    // target nullable, back-filling a validity buffer).
    let other = FixedSerie::<Int64>::from_options(&[Some(70), None, Some(90)]);
    col.set_range_serie(2, &other).unwrap();
    assert_eq!(
        col.to_options(),
        vec![Some(-1), Some(-2), Some(70), None, Some(90), Some(6)]
    );
    assert_eq!(col.null_count(), 1);

    // set_range_serie from a non-nullable source into a non-nullable target stays non-nullable.
    let mut plain = FixedSerie::<Int32>::from_values(&[0, 0, 0, 0]);
    let src = FixedSerie::<Int32>::from_values(&[11, 22]);
    plain.set_range_serie(1, &src).unwrap();
    assert_eq!(plain.values(), vec![0, 11, 22, 0]);
    assert_eq!(plain.null_count(), 0);
}

#[test]
fn serie_slice_numeric_and_clamp() {
    let col = FixedSerie::<Int32>::from_values(&[10, 20, 30, 40, 50]);

    let mid = col.slice(1, 3);
    assert_eq!(mid.len(), 3);
    assert_eq!(mid.values(), vec![20, 30, 40]);

    // Clamp an over-long window -> a short column.
    let tail = col.slice(3, 10);
    assert_eq!(tail.len(), 2);
    assert_eq!(tail.values(), vec![40, 50]);

    // A start past the end -> an empty column (never an error).
    assert_eq!(col.slice(9, 4).len(), 0);

    // A nullable slice carries the matching validity bits.
    let nullable = FixedSerie::<Int64>::from_options(&[Some(1), None, Some(3), None, Some(5)]);
    let sub = nullable.slice(1, 3);
    assert_eq!(sub.to_options(), vec![None, Some(3), None]);
    assert_eq!(sub.null_count(), 2);
}

#[test]
fn var_and_fixed_size_slice_and_set() {
    // Utf8 VarSerie slice rebuilds offsets/data and carries validity.
    let words = VarSerie::<Utf8>::from_options(&[
        Some("alpha".to_string()),
        None,
        Some("gamma".to_string()),
        Some("delta".to_string()),
    ]);
    let sub = words.slice(1, 2);
    assert_eq!(sub.len(), 2);
    assert_eq!(sub.get(0), None);
    assert_eq!(sub.get(1).as_deref(), Some("gamma"));
    assert_eq!(sub.null_count(), 1);
    // Clamp past the end.
    assert_eq!(words.slice(3, 9).len(), 1);
    assert_eq!(words.slice(2, 9).get(1).as_deref(), Some("delta"));

    // FixedSizeSerie slice copies the fixed-stride block + validity.
    let codes = FixedSizeSerie::<FixedUtf8>::from_options(
        3,
        &[
            Some("ab".to_string()),
            None,
            Some("xyz".to_string()),
            Some("qrs".to_string()),
        ],
    );
    let block = codes.slice(1, 2);
    assert_eq!(block.width(), 3);
    assert_eq!(block.len(), 2);
    assert_eq!(block.get(0), None);
    assert_eq!(block.get(1).as_deref(), Some("xyz"));

    // FixedSizeSerie in-place set (zero-pad / truncate to the fixed width) + out-of-range guard.
    let mut fx =
        FixedSizeSerie::<FixedBinary>::from_values(4, &[b"aaaa".to_vec(), b"bbbb".to_vec()]);
    fx.set(0, b"cd").unwrap();
    assert_eq!(fx.get(0), Some(b"cd\0\0".to_vec())); // zero-padded to width 4
    fx.set_checked(1, b"zzzzzz"); // truncated to width 4
    assert_eq!(fx.get(1), Some(b"zzzz".to_vec()));
    assert!(matches!(
        fx.set(2, b"x").unwrap_err(),
        IoError::SliceOutOfBounds { .. }
    ));
}
