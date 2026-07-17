//! The `io::fixed::decimal` family: the self-describing [`Decimal`] value type (arithmetic,
//! ordering, conversions, `Display`/`FromStr`, byte codec, value identity) and the columnar
//! descriptors ([`DecimalType`]/[`DecimalField`]/[`DecimalScalar`]/[`DecimalSerie`]) — plus the
//! `DataType`/`FieldType` drill-down predicates. The zero-copy Arrow interop lives behind
//! `#[cfg(feature = "arrow")]` at the bottom.

use std::collections::HashSet;
use std::str::FromStr;

use yggdryl_core::io::fixed::{
    D128Field, D128Scalar, D128Serie, D128Type, D64Serie, Dec128, Dec256, Dec32, DecimalError,
    D128, D256, D32, D64,
};
use yggdryl_core::io::{DataType, FieldType, ScalarType, SerieType};

// -------------------------------------------------------------------------------------
// Value type — construction, Display, FromStr
// -------------------------------------------------------------------------------------

#[test]
fn new_and_display_place_the_point() {
    assert_eq!(D128::new(12345, 2).unwrap().to_string(), "123.45");
    assert_eq!(D128::new(5, 3).unwrap().to_string(), "0.005"); // pad leading zeros
    assert_eq!(D128::new(-5, 3).unwrap().to_string(), "-0.005");
    assert_eq!(D128::new(42, 0).unwrap().to_string(), "42"); // scale 0 is an integer
    assert_eq!(D128::new(-12300, 2).unwrap().to_string(), "-123.00");
}

#[test]
fn from_str_round_trips_through_display() {
    for text in ["123.45", "-0.005", "42", "-123.00", "0.1", "1000000.000001"] {
        let value = D128::from_str(text).unwrap();
        assert_eq!(value.to_string(), text, "{text}");
    }
    assert_eq!(D128::from_str(".5").unwrap().to_string(), "0.5");
    assert_eq!(D128::from_str("+7").unwrap().to_string(), "7");
    assert!(matches!(
        D128::from_str("1.2.3"),
        Err(DecimalError::ParseError { .. })
    ));
    assert!(matches!(
        D128::from_str("abc"),
        Err(DecimalError::ParseError { .. })
    ));
}

#[test]
fn from_str_accepts_scientific_notation() {
    // Exponent notation (as a Python `decimal.Decimal` emits) parses to the right scale.
    assert_eq!(
        D128::from_str("1.5e3").unwrap(),
        D128::new(1500, 0).unwrap()
    );
    assert_eq!(D128::from_str("1.5E+3").unwrap().to_string(), "1500");
    assert_eq!(D128::from_str("1.5e-2").unwrap().to_string(), "0.015");
    assert_eq!(D128::from_str("25e-1").unwrap(), D128::new(25, 1).unwrap()); // 2.5
    assert!(matches!(
        D128::from_str("1e"), // missing exponent digits
        Err(DecimalError::ParseError { .. })
    ));
}

#[test]
fn coefficient_out_of_range_is_guided() {
    // 3e9 overflows d32's i32 coefficient (i32::MAX is ~2.1e9).
    let error = D32::new(3_000_000_000, 0).unwrap_err();
    assert!(matches!(
        error,
        DecimalError::CoefficientOutOfRange { ty: "d32", .. }
    ));
    assert!(error.to_string().contains("d32"));
    assert!(error.to_string().contains("wider decimal"));
}

// -------------------------------------------------------------------------------------
// Value type — arithmetic
// -------------------------------------------------------------------------------------

#[test]
fn add_and_sub_align_scales() {
    let a = D128::new(12345, 2).unwrap(); // 123.45
    let b = D128::new(617, 2).unwrap(); //     6.17
    assert_eq!((a + b).to_string(), "129.62");
    assert_eq!((a - b).to_string(), "117.28");

    // Mixed scales align to the larger scale.
    let x = D64::new(25, 1).unwrap(); // 2.5
    let y = D64::new(25, 2).unwrap(); // 0.25
    let sum = x.checked_add(&y).unwrap();
    assert_eq!(sum.to_string(), "2.75");
    assert_eq!(sum.scale(), 2);
}

#[test]
fn mul_adds_scales_and_div_takes_a_scale() {
    let a = D64::new(25, 1).unwrap(); // 2.5
    let b = D64::new(20, 1).unwrap(); // 2.0
    let product = a * b; //             5.00 (scale 1 + 1)
    assert_eq!(product.to_string(), "5.00");
    assert_eq!(product.scale(), 2);

    let one = D128::new(1, 0).unwrap();
    let three = D128::new(3, 0).unwrap();
    assert_eq!(one.checked_div(&three, 4).unwrap().to_string(), "0.3333");
    assert!(matches!(
        one.checked_div(&D128::zero(), 2),
        Err(DecimalError::DivideByZero { .. })
    ));
}

#[test]
fn neg_and_rem() {
    assert_eq!((-D128::new(12345, 2).unwrap()).to_string(), "-123.45");
    // 7.5 % 2.0 = 1.5
    let r = D64::new(75, 1)
        .unwrap()
        .checked_rem(&D64::new(20, 1).unwrap())
        .unwrap();
    assert_eq!(r.to_string(), "1.5");
}

#[test]
fn checked_arithmetic_reports_overflow() {
    let max = D128::new(i128::MAX, 0).unwrap();
    let one = D128::new(1, 0).unwrap();
    assert!(matches!(
        max.checked_add(&one),
        Err(DecimalError::Overflow {
            ty: "d128",
            op: "add"
        })
    ));
    // Multiplying d32 near its coefficient limit overflows.
    let big = D32::new(999_999_999, 0).unwrap();
    assert!(matches!(
        big.checked_mul(&D32::new(10, 0).unwrap()),
        Err(DecimalError::Overflow { ty: "d32", .. })
    ));
}

#[test]
#[should_panic(expected = "d128 add overflow")]
fn add_operator_panics_on_overflow() {
    let _ = D128::new(i128::MAX, 0).unwrap() + D128::new(1, 0).unwrap();
}

// -------------------------------------------------------------------------------------
// Value type — ordering & identity (equal-by-value, over the normalized form)
// -------------------------------------------------------------------------------------

#[test]
fn equal_values_compare_equal_and_hash_equal_across_scales() {
    let a = D128::new(25, 1).unwrap(); // 2.5
    let b = D128::new(250, 2).unwrap(); // 2.50
    assert_eq!(a, b);
    let mut set = HashSet::new();
    set.insert(a);
    set.insert(b);
    assert_eq!(set.len(), 1, "equal values must hash equal");

    // The byte codec is over the normalized form, so equal values serialize identically.
    assert_eq!(a.serialize_bytes(), b.serialize_bytes());
}

#[test]
fn ordering_is_true_numeric_order() {
    let mut values = [
        D64::new(275, 2).unwrap(),  // 2.75
        D64::new(25, 1).unwrap(),   // 2.5
        D64::new(-1, 0).unwrap(),   // -1
        D64::new(2500, 3).unwrap(), // 2.5 (== the scale-1 one)
    ];
    values.sort();
    assert_eq!(values[0].to_string(), "-1");
    assert_eq!(values[3].to_string(), "2.75");
    // 2.5 (scale 1) and 2.500 (scale 3) are equal by value.
    assert_eq!(D64::new(25, 1).unwrap(), D64::new(2500, 3).unwrap());
    assert!(D64::new(25, 1).unwrap() < D64::new(275, 2).unwrap());
}

// -------------------------------------------------------------------------------------
// Value type — conversions & numeric interop
// -------------------------------------------------------------------------------------

#[test]
fn float_conversions() {
    assert!((D128::new(12345, 2).unwrap().to_f64() - 123.45).abs() < 1e-9);
    assert_eq!(D128::from_f64(123.45, 2).unwrap().to_string(), "123.45");
    assert!(matches!(
        D128::from_f64(f64::NAN, 2),
        Err(DecimalError::NonFinite { .. })
    ));
}

#[test]
fn integer_conversions() {
    assert_eq!(D128::new(12300, 2).unwrap().to_i128().unwrap(), 123); // 123.00 is integral
    assert!(matches!(
        D128::new(12345, 2).unwrap().to_i128(),
        Err(DecimalError::NotInteger { .. })
    ));
    assert_eq!(D128::from_i128(-9).unwrap().to_string(), "-9");
}

#[test]
fn cast_between_widths() {
    let small = D32::new(12345, 2).unwrap(); // 123.45 as d32
    let wide = small.cast::<Dec128>().unwrap();
    assert_eq!(wide.to_string(), "123.45");
    assert_eq!(wide.type_name(), "d128");

    // Widening d128 -> d256 is loss-free; narrowing an out-of-range value errors.
    let huge = D128::new(i128::MAX, 0).unwrap();
    assert_eq!(huge.cast::<Dec256>().unwrap().to_i128().unwrap(), i128::MAX);
    assert!(matches!(
        huge.cast::<Dec32>(),
        Err(DecimalError::OutOfWidth {
            from: "d128",
            to: "d32"
        })
    ));

    // A d256 coefficient beyond i128 (~1.7e38) casts to itself (via its decimal digits, not an
    // i128 bridge).
    let wide256 = D256::from_coeff_str("1000000000000000000000000000000000000000000", 5).unwrap();
    assert_eq!(wide256.cast::<Dec256>().unwrap(), wide256);
    assert!(matches!(
        wide256.cast::<Dec128>(),
        Err(DecimalError::OutOfWidth {
            from: "d256",
            to: "d128"
        })
    ));
}

// -------------------------------------------------------------------------------------
// Value type — rescale / round / trunc & byte codec
// -------------------------------------------------------------------------------------

#[test]
fn rescale_round_trunc() {
    let v = D64::new(12345, 2).unwrap(); // 123.45
    assert_eq!(v.rescale(4).unwrap().to_string(), "123.4500"); // raise: exact
    assert!(matches!(
        v.rescale(1), // lower would drop the non-zero 5
        Err(DecimalError::InexactRescale { .. })
    ));
    assert_eq!(v.trunc_to_scale(1).unwrap().to_string(), "123.4");
    assert_eq!(v.round_to_scale(1).unwrap().to_string(), "123.5"); // .45 -> .5 (half up)
    assert_eq!(v.trunc().to_string(), "123");
    // A rescale that only strips trailing zeros is exact.
    assert_eq!(
        D64::new(12300, 2).unwrap().rescale(0).unwrap().to_string(),
        "123"
    );
}

#[test]
fn byte_codec_round_trips() {
    for value in [
        D256::new(0, 0).unwrap(),
        D256::new(123_456_789, 4).unwrap(),
        D256::new(-1, 6).unwrap(),
    ] {
        let bytes = value.serialize_bytes();
        assert_eq!(bytes.len(), D256::serialized_len());
        assert_eq!(D256::deserialize_bytes(&bytes).unwrap(), value);
    }
    assert!(matches!(
        D128::deserialize_bytes(&[0u8; 2]), // too short
        Err(DecimalError::ParseError { .. })
    ));
}

// -------------------------------------------------------------------------------------
// Columnar — DecimalType / DecimalField
// -------------------------------------------------------------------------------------

#[test]
fn decimal_type_clamps_and_validates() {
    let dt = D128Type::new(20, 4);
    assert_eq!((dt.precision(), dt.scale()), (20, 4));
    assert_eq!(dt.name(), "d128");
    assert_eq!(dt.to_string(), "d128(20, 4)"); // signature Display
    assert_eq!(D128Type::new(38, 18).to_string(), "d128(38, 18)");
    assert_eq!(dt.byte_width(), 16);
    assert!(dt.is_decimal() && dt.is_numeric() && dt.is_signed());

    // `new` clamps an over-range precision into [1, MAX]; `try_new` rejects it.
    assert_eq!(D128Type::new(200, 4).precision(), 38);
    assert!(D128Type::try_new(200, 4).is_err());
    assert_eq!(D128Type::max_precision(), 38);
}

#[test]
fn decimal_field_reports_its_type() {
    let field = D128Field::new("amount", 20, 4, true);
    assert_eq!(field.name(), "amount");
    assert_eq!(field.type_name(), "d128");
    assert_eq!(field.byte_width(), 16);
    assert!(field.nullable() && field.is_decimal());
    assert_eq!(
        FieldType::type_id(&field),
        yggdryl_core::io::DataTypeId::D128
    );
    // Erasing keeps precision/scale in metadata so the descriptor is not lost.
    let erased = field.erase();
    assert_eq!(erased.type_name(), "d128");
    assert!(erased.metadata().get("scale").is_some());
}

// -------------------------------------------------------------------------------------
// Columnar — DecimalScalar / DecimalSerie
// -------------------------------------------------------------------------------------

#[test]
fn decimal_scalar_carries_value_and_round_trips() {
    let scalar = D128Scalar::of(D128::new(12345, 2).unwrap());
    assert!(!scalar.is_null());
    assert_eq!(scalar.value().unwrap().to_string(), "123.45");
    assert_eq!(scalar.scale(), 2);
    assert!(D128Scalar::null(10, 2).is_null());
    assert!(ScalarType::is_valid(&scalar));

    // with_precision_scale rescales + validates precision.
    let strict = D128Scalar::with_precision_scale(D128::new(5, 0).unwrap(), 4, 2).unwrap();
    assert_eq!(strict.value().unwrap().to_string(), "5.00");
    assert!(matches!(
        D128Scalar::with_precision_scale(D128::new(123456, 0).unwrap(), 3, 0),
        Err(DecimalError::PrecisionExceeded { .. })
    ));

    // Byte round-trip through a cursor.
    let mut sink = yggdryl_core::io::Bytes::new();
    scalar.write_to(&mut sink).unwrap();
    yggdryl_core::io::IOCursor::rewind(&mut sink);
    assert_eq!(D128Scalar::read_from(&mut sink).unwrap(), scalar);
}

#[test]
fn decimal_serie_builds_reads_and_round_trips() {
    let values = [
        Some(D128::new(12345, 2).unwrap()),
        None,
        Some(D128::new(600, 2).unwrap()),
    ];
    let col = D128Serie::from_options(20, 2, &values).unwrap();
    assert_eq!(col.len(), 3);
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get(0).unwrap().to_string(), "123.45");
    assert_eq!(col.get(1), None);
    assert_eq!(SerieType::get(&col, 2).unwrap().to_string(), "6.00");

    // Serialization round-trips through a byte sink.
    let mut sink = yggdryl_core::io::Bytes::new();
    col.write_to(&mut sink).unwrap();
    yggdryl_core::io::IOCursor::rewind(&mut sink);
    assert_eq!(D128Serie::read_from(&mut sink).unwrap(), col);
}

#[test]
fn decimal_serie_push_validates_scale_and_precision() {
    let mut col = D64Serie::new(4, 2); // precision 4, scale 2
                                       // A value with a non-zero digit past scale 2 cannot be stored exactly.
    assert!(matches!(
        col.push(Some(D64::new(12345, 3).unwrap())), // 12.345
        Err(DecimalError::InexactRescale { .. })
    ));
    // A value needing 5 significant digits exceeds precision 4.
    assert!(matches!(
        col.push(Some(D64::new(12345, 2).unwrap())), // 123.45 -> 5 digits
        Err(DecimalError::PrecisionExceeded { .. })
    ));
    // A fitting value pushes fine.
    col.push(Some(D64::new(1234, 2).unwrap())).unwrap(); // 12.34
    assert_eq!(col.get(0).unwrap().to_string(), "12.34");
}

// -------------------------------------------------------------------------------------
// Value type — canonical normalized form (identity == bytes == hash, all classes)
// -------------------------------------------------------------------------------------

#[test]
fn normalized_is_one_canonical_form_per_numeric_value() {
    // Each group holds numerically-equal decimals written at DIFFERENT scales. Every member must
    // compare equal, serialize to identical canonical bytes, and hash into a single HashSet slot —
    // so PartialEq / Hash / serialize_bytes (all built over `normalized()`) can never disagree.
    let groups: [&[D128]; 3] = [
        &[
            D128::new(0, 0).unwrap(),
            D128::new(0, 1).unwrap(),
            D128::new(0, -3).unwrap(),
        ],
        &[D128::new(10, 0).unwrap(), D128::new(1, -1).unwrap()],
        &[
            D128::new(100, 2).unwrap(),
            D128::new(1, 0).unwrap(),
            D128::new(10, 1).unwrap(),
        ],
    ];
    for group in groups {
        let first = group[0];
        let bytes = first.serialize_bytes();
        let mut set = HashSet::new();
        for &value in group {
            assert_eq!(value, first);
            assert_eq!(
                value.serialize_bytes(),
                bytes,
                "equal values must serialize to identical canonical bytes"
            );
            set.insert(value);
        }
        assert_eq!(set.len(), 1, "equal values must hash into one slot");
        // The canonical bytes deserialize back to an equal value.
        assert_eq!(D128::deserialize_bytes(&bytes).unwrap(), first);
    }
}

#[test]
fn arithmetic_zero_and_negative_scale_are_canonical() {
    // `2.5 - 2.5` produces a coefficient-0 value at scale 1 (non-canonical); it must collapse to the
    // canonical zero for equality, bytes, and hashing — reachable purely through arithmetic.
    let zero_scaled = D128::new(25, 1).unwrap() - D128::new(25, 1).unwrap();
    let zero = D128::new(0, 0).unwrap();
    assert_eq!(zero_scaled, zero);
    assert_eq!(zero_scaled.serialize_bytes(), zero.serialize_bytes());
    let mut set = HashSet::new();
    set.insert(zero);
    assert!(
        set.contains(&zero_scaled),
        "a HashSet must find the equal zero member built at a different scale"
    );

    // `10` at scale 0 normalizes to `(1, -1)` — a NEGATIVE scale. serialize writes it as `i8 -> u8`;
    // deserialize must recover the negative scale and an equal, correctly-displaying value.
    let ten = D128::new(10, 0).unwrap();
    let bytes = ten.serialize_bytes();
    assert_eq!(
        bytes[0],
        (-1i8) as u8,
        "scale byte is the normalized negative scale"
    );
    let back = D128::deserialize_bytes(&bytes).unwrap();
    assert_eq!(back, ten);
    assert_eq!(back.to_string(), "10");
    assert_eq!(
        back.serialize_bytes(),
        bytes,
        "already canonical -> byte-stable"
    );
}

#[test]
fn decimal_serie_scale_is_declared_not_value_normalized() {
    // The columnar scale is the column's declared scale, independent of any value's normalized
    // scale (`6.00` normalizes to `(6, 0)` as a value, but the column stays scale 2) — so the
    // canonical-form change to the value type leaves the columnar Arrow `(precision, scale)` intact.
    let col = D128Serie::from_options(20, 2, &[Some(D128::new(600, 2).unwrap())]).unwrap();
    assert_eq!(col.scale(), 2);
    assert_eq!(col.get(0).unwrap().to_string(), "6.00");
}

#[test]
fn decimal_scalar_serie_singular_broadcast_round_trips() {
    // Scalar::to_serie -> a length-1 column carrying the scalar's own (precision, scale).
    let scalar = D128Scalar::of(D128::new(12345, 2).unwrap()); // 123.45
    let col = scalar.to_serie().unwrap();
    assert_eq!(col.len(), 1);
    assert_eq!(col.get(0).unwrap().to_string(), "123.45");

    // Serie::as_scalar -> the single element back; None for a multi-element column.
    assert_eq!(col.as_scalar(), Some(scalar.clone()));
    let two = D128Serie::from_options(20, 2, &[Some(D128::new(1, 0).unwrap()), None]).unwrap();
    assert_eq!(two.as_scalar(), None);

    // Serie::from_scalar is the inverse of as_scalar; a null scalar broadcasts to a null column.
    assert_eq!(D128Serie::from_scalar(scalar).unwrap(), col);
    let null_col = D128Serie::from_scalar(D128Scalar::null(20, 2)).unwrap();
    assert_eq!(null_col.len(), 1);
    assert_eq!(null_col.get(0), None);
}

// -------------------------------------------------------------------------------------
// Arrow interop (feature `arrow`) — zero-copy columns + schema round-trips
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
mod arrow {
    use arrow_array::Array;
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl_core::io::fixed::{
        D128Field, D128Serie, D256Serie, D32Serie, D64Serie, Field, D128, D256, D32, D64,
    };
    use yggdryl_core::io::FieldType;

    #[test]
    fn serie_to_from_arrow_is_zero_copy_and_carries_precision_scale() {
        let col = D128Serie::from_options(
            20,
            2,
            &[
                Some(D128::new(12345, 2).unwrap()),
                None,
                Some(D128::new(600, 2).unwrap()),
            ],
        )
        .unwrap();
        let array = col.to_arrow_array();
        assert_eq!(array.len(), 3);
        assert_eq!(array.precision(), 20);
        assert_eq!(array.scale(), 2);
        assert_eq!(array.value_as_string(0), "123.45");
        assert!(array.is_null(1));

        // Round-trip back sharing the SAME values allocation (an Arc bump, not a copy).
        let back = D128Serie::from_arrow_array(&array);
        assert_eq!(back, col);
        assert_eq!(back.get(2).unwrap().to_string(), "6.00");
    }

    #[test]
    fn all_widths_map_to_their_arrow_decimal() {
        let d32 = D32Serie::from_values(9, 2, &[D32::new(12345, 2).unwrap()]).unwrap();
        assert!(matches!(
            d32.to_arrow_array().data_type(),
            ArrowDataType::Decimal32(9, 2)
        ));
        let d64 = D64Serie::from_values(18, 3, &[D64::new(1, 3).unwrap()]).unwrap();
        assert!(matches!(
            d64.to_arrow_array().data_type(),
            ArrowDataType::Decimal64(18, 3)
        ));
        let d256 = D256Serie::from_values(50, 4, &[D256::new(1, 4).unwrap()]).unwrap();
        assert!(matches!(
            d256.to_arrow_array().data_type(),
            ArrowDataType::Decimal256(50, 4)
        ));
    }

    #[test]
    fn field_arrow_round_trip_preserves_precision_scale() {
        let field = D128Field::new("amount", 20, 4, true);
        let arrow = field.to_arrow();
        assert!(matches!(
            arrow.data_type(),
            ArrowDataType::Decimal128(20, 4)
        ));
        let back = D128Field::from_arrow(&arrow).unwrap();
        assert_eq!(back, field);
    }

    #[test]
    fn erased_field_round_trips_precision_scale_through_arrow() {
        let arrow = D128Field::new("amount", 20, 4, false).erase().to_arrow();
        assert!(matches!(
            arrow.data_type(),
            ArrowDataType::Decimal128(20, 4)
        ));
        // The erased field recovers as a d128 (via the logical-type tag) with its p/s.
        let erased = Field::from_arrow(&arrow).unwrap();
        assert_eq!(erased.type_name(), "d128");
        assert_eq!(
            erased.to_arrow().data_type(),
            &ArrowDataType::Decimal128(20, 4)
        );
    }

    #[test]
    fn d32_field_does_not_need_a_tag() {
        // Decimal32 is unambiguous, so no metadata tag is needed to recover d32.
        use yggdryl_core::io::fixed::D32Field;
        let arrow = D32Field::new("rate", 9, 4, true).to_arrow();
        assert!(arrow.metadata().is_empty());
        assert_eq!(D32Field::from_arrow(&arrow).unwrap().type_name(), "d32");
    }
}
