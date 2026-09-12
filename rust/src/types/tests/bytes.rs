use super::super::{Bytes, BytesLayout, BytesParameters, DataType};
use crate::{Field, Scalar, Scheme};

/// Bytes bounded to `max` on the `binary` layout.
fn bounded_binary(max: u32) -> DataType {
    DataType::bytes(
        BytesParameters::new(BytesLayout::Binary)
            .try_with_bound(max)
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn serde_and_the_structural_value_round_trip() {
    // One `binary` tag for every byte datatype: the layout and the bound are
    // written only where they are not the default.
    for (dtype, json) in [
        (DataType::binary(), r#"{"type":"binary"}"#),
        (
            DataType::large_binary(),
            r#"{"type":"binary","layout":"large_binary"}"#,
        ),
        (
            DataType::binary_view(),
            r#"{"type":"binary","layout":"binary_view"}"#,
        ),
        (bounded_binary(16), r#"{"type":"binary","max":16}"#),
        (
            DataType::fixed_size_binary(4).unwrap(),
            r#"{"type":"binary","layout":"fixed_size_binary","fixed":4}"#,
        ),
    ] {
        assert_eq!(dtype.clone().into_json().unwrap(), json);
        assert_eq!(DataType::from_json(json).unwrap(), dtype);
        let value = dtype.clone().into_value();
        assert_eq!(
            value.get_key_str("type").and_then(Scalar::as_str),
            Some("binary")
        );
        assert_eq!(DataType::from_value(value).unwrap(), dtype);
        let field = Field::new("payload", dtype.clone(), true);
        assert_eq!(
            Field::from_json(&field.clone().into_json().unwrap()).unwrap(),
            field
        );
    }

    // The retired tags name nothing.
    for tag in ["fixed_size_binary", "large_binary", "binary_view"] {
        assert!(
            DataType::from_json(&format!(r#"{{"type":"{tag}","width":4}}"#)).is_err(),
            "{tag}"
        );
    }
    // A fixed layout with no width, a bound of zero, and a bound under the
    // wrong key are refused.
    assert!(DataType::from_json(r#"{"type":"binary","layout":"fixed_size_binary"}"#).is_err());
    assert!(DataType::from_json(r#"{"type":"binary","max":0}"#).is_err());
    assert!(DataType::from_json(r#"{"type":"binary","fixed":4}"#).is_err());
    assert!(
        DataType::from_json(r#"{"type":"binary","layout":"fixed_size_binary","max":4}"#).is_err()
    );
    let refused = DataType::from_value(
        DataType::binary()
            .into_value()
            .with_key("fixed", Scalar::from(4_i64))
            .unwrap(),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("$.fixed"), "{refused}");
}

#[test]
fn only_plain_binary_crosses_a_foreign_target_unchanged() {
    let schema = DataType::from_fields([
        DataType::binary().required_field("plain"),
        bounded_binary(16).required_field("bounded"),
        DataType::large_binary().required_field("large"),
        DataType::binary_view().required_field("view"),
        DataType::fixed_size_binary(4)
            .unwrap()
            .required_field("fixed"),
    ])
    .unwrap()
    .required_field("row");
    for scheme in [Scheme::SPARK, Scheme::POLARS, Scheme::PANDAS] {
        let compat = schema.clone().into_scheme_compat(&scheme).unwrap();
        for name in ["plain", "bounded", "large", "view", "fixed"] {
            assert_eq!(compat[name].dtype(), &DataType::binary(), "{name}");
        }
    }
    // Iceberg names `fixed[n]`, so a fixed width stays.
    let compat = schema.clone().into_scheme_compat(&Scheme::ICEBERG).unwrap();
    for name in ["plain", "bounded", "large", "view"] {
        assert_eq!(compat[name].dtype(), &DataType::binary(), "{name}");
    }
    assert_eq!(
        compat["fixed"].dtype(),
        &DataType::fixed_size_binary(4).unwrap()
    );
    assert_eq!(
        schema.clone().into_scheme_compat(&Scheme::ARROW).unwrap(),
        schema
    );
}

#[test]
fn the_default_is_the_empty_payload_or_the_zero_filled_slot() {
    for dtype in [
        DataType::binary(),
        bounded_binary(16),
        DataType::large_binary(),
        DataType::binary_view(),
    ] {
        let value = dtype.default_value().unwrap();
        assert_eq!(value.as_bytes(), Some(&[][..]), "{dtype}");
        // A value never carries a maximum.
        assert_eq!(
            value.dtype().unwrap(),
            DataType::Bytes(dtype.bytes_parameters().unwrap().without_max())
        );
        assert!(dtype.is_default_value(&value).unwrap());
    }
    let fixed = DataType::fixed_size_binary(4).unwrap();
    let value = fixed.default_value().unwrap();
    assert_eq!(value, Scalar::Bytes(Bytes::new([0_u8; 4])));
    assert_eq!(value.dtype().unwrap(), fixed);
    assert!(fixed.is_default_value(&value).unwrap());
    assert!(
        !fixed
            .is_default_value(&Scalar::Bytes(Bytes::new([0_u8; 3])))
            .unwrap()
    );
}

#[test]
fn a_bound_and_a_layout_are_what_a_diff_reports() {
    assert_eq!(
        DataType::binary().show_diff(&bounded_binary(16), true, false),
        "≠ $.bound: None → Some(16)"
    );
    assert_eq!(
        DataType::fixed_size_binary(4).unwrap().show_diff(
            &DataType::fixed_size_binary(8).unwrap(),
            true,
            false
        ),
        "≠ $.bound: Some(4) → Some(8)"
    );
    assert_eq!(
        DataType::binary().show_diff(&DataType::large_binary(), true, false),
        "≠ $.kind: binary → large_binary"
    );
}
