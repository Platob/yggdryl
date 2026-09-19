//! The expression converter an integration test cannot reach.
//!
//! `convert` is the crate-private value cast every `cast` operator and every
//! bound projection goes through; what it answers per target leaf, and what it
//! refuses, is the contract. Everything a caller can observe lives in
//! `tests/expression/grammar.rs`.

use crate::expression::Safety;
use crate::{DataType, DataTypeId, Scalar, Version};

#[test]
fn scalar_casts_return_the_exact_target_leaf() {
    let cases = [
        (
            DataType::decimal32(9, 2).unwrap(),
            Scalar::from(125),
            DataTypeId::Decimal32,
        ),
        (
            DataType::decimal64(18, 2).unwrap(),
            Scalar::from(125),
            DataTypeId::Decimal64,
        ),
        (
            DataType::Float32,
            Scalar::from(1.5_f64),
            DataTypeId::Float32,
        ),
        (
            DataType::large_utf8(),
            Scalar::from("value"),
            DataTypeId::LargeUtf8String,
        ),
        (
            DataType::utf8_view(),
            Scalar::from("value"),
            DataTypeId::Utf8StringView,
        ),
        (
            DataType::large_binary(),
            Scalar::from("value"),
            DataTypeId::LargeBinary,
        ),
        (
            DataType::binary_view(),
            Scalar::from("value"),
            DataTypeId::BinaryView,
        ),
        (
            DataType::fixed_ascii(4).unwrap(),
            Scalar::from("FIX"),
            DataTypeId::FixedAsciiString,
        ),
        (
            DataType::Currency,
            Scalar::from("USD"),
            DataTypeId::Currency,
        ),
    ];

    for (target, input, id) in cases {
        let converted = super::convert(&target, &input, Safety::Strict).unwrap();
        assert_eq!(converted.id(), id, "cast to {target}");
    }
}

#[test]
fn versions_do_not_fall_through_text_or_numeric_expression_paths() {
    let patch2 = Scalar::from("5.0.2".parse::<Version>().unwrap());
    let patch10 = Scalar::from("5.0.10".parse::<Version>().unwrap());

    assert_eq!(
        super::convert(
            &DataType::Version,
            &Scalar::from("005.000.002"),
            Safety::Strict
        )
        .unwrap(),
        patch2
    );
    assert_eq!(
        super::convert(&DataType::Version, &patch2, Safety::Strict).unwrap(),
        patch2
    );
    assert_eq!(
        super::convert(&DataType::utf8(), &patch2, Safety::Strict).unwrap(),
        Scalar::from("5.0.2")
    );
    assert!(super::convert(&DataType::Version, &Scalar::from(5), Safety::Strict).is_err());
    assert_eq!(
        crate::expression::eval::order(&DataType::Version, &patch2, &patch10),
        Some(std::cmp::Ordering::Less)
    );

    let schema = DataType::from_fields([DataType::Version.required_field("v")])
        .unwrap()
        .required_field("row");
    let row = Scalar::from_sequence([patch2]);
    for (text, expected) in [
        ("v < version '5.0.10'", Scalar::from(true)),
        ("length(v)", Scalar::from(5_i64)),
        ("v like '5.0%'", Scalar::from(true)),
    ] {
        let answer = text
            .parse::<crate::expression::Term>()
            .unwrap()
            .bind(&schema)
            .unwrap()
            .eval(&row)
            .unwrap();
        assert_eq!(answer, expected, "{text}");
    }
}
