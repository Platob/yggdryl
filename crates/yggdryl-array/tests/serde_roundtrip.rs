//! With the `serde` feature on, arrays round-trip through JSON and
//! deserialization re-validates every length.

#![cfg(feature = "serde")]

use yggdryl_array::PrimitiveArray;
use yggdryl_schema::{Float64, Int32};

#[test]
fn arrays_roundtrip_through_json() {
    let arrays = [
        PrimitiveArray::from_native(Int32, vec![1, 2, 3]),
        PrimitiveArray::from_options(Int32, vec![Some(1), None]),
        PrimitiveArray::from_native(Int32, vec![]),
    ];
    for array in arrays {
        let json = serde_json::to_string(&array).unwrap();
        assert_eq!(
            serde_json::from_str::<PrimitiveArray<Int32>>(&json).unwrap(),
            array
        );
    }

    let floats = PrimitiveArray::from_options(Float64, vec![Some(f64::NAN), None]);
    let json = serde_json::to_string(&floats).unwrap();
    assert_eq!(
        serde_json::from_str::<PrimitiveArray<Float64>>(&json).unwrap(),
        floats
    );
}

#[test]
fn deserialization_revalidates_lengths() {
    // 2 elements of Int32 need 8 value bytes, not 4.
    let short = serde_json::json!({
        "data_type": null,
        "len": 2,
        "values": [0, 0, 0, 0],
        "validity": null,
    });
    assert!(serde_json::from_value::<PrimitiveArray<Int32>>(short).is_err());
    // A validity bitmap must cover exactly ceil(len / 8) bytes.
    let bad_bits = serde_json::json!({
        "data_type": null,
        "len": 1,
        "values": [0, 0, 0, 0],
        "validity": [0, 0],
    });
    assert!(serde_json::from_value::<PrimitiveArray<Int32>>(bad_bits).is_err());
}
