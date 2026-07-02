//! With the `serde` feature on, arrays round-trip through JSON and
//! deserialization re-validates every length.

#![cfg(feature = "serde")]

use yggdryl_array::PrimitiveArray;
use yggdryl_schema::{Float64Type, Int32Type};

#[test]
fn arrays_roundtrip_through_json() {
    let arrays = [
        PrimitiveArray::from_native(Int32Type, vec![1, 2, 3]),
        PrimitiveArray::from_options(Int32Type, vec![Some(1), None]),
        PrimitiveArray::from_native(Int32Type, vec![]),
    ];
    for array in arrays {
        let json = serde_json::to_string(&array).unwrap();
        assert_eq!(
            serde_json::from_str::<PrimitiveArray<Int32Type>>(&json).unwrap(),
            array
        );
    }

    let floats = PrimitiveArray::from_options(Float64Type, vec![Some(f64::NAN), None]);
    let json = serde_json::to_string(&floats).unwrap();
    assert_eq!(
        serde_json::from_str::<PrimitiveArray<Float64Type>>(&json).unwrap(),
        floats
    );
}

#[test]
fn deserialization_revalidates_lengths() {
    // 2 elements of Int32Type need 8 value bytes, not 4.
    let short = serde_json::json!({
        "data_type": null,
        "len": 2,
        "values": [0, 0, 0, 0],
        "validity": null,
    });
    assert!(serde_json::from_value::<PrimitiveArray<Int32Type>>(short).is_err());
    // A validity bitmap must cover exactly ceil(len / 8) bytes.
    let bad_bits = serde_json::json!({
        "data_type": null,
        "len": 1,
        "values": [0, 0, 0, 0],
        "validity": [0, 0],
    });
    assert!(serde_json::from_value::<PrimitiveArray<Int32Type>>(bad_bits).is_err());
}
