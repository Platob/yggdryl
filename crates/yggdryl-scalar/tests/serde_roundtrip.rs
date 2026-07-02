//! With the `serde` feature on, scalars round-trip through JSON and
//! deserialization re-validates the layout.

#![cfg(feature = "serde")]

use yggdryl_scalar::Scalar;
use yggdryl_schema::{Int32Type, Utf8Type};

#[test]
fn scalars_roundtrip_through_json() {
    for scalar in [Scalar::from_native(Int32Type, 7), Scalar::null(Int32Type)] {
        let json = serde_json::to_string(&scalar).unwrap();
        assert_eq!(
            serde_json::from_str::<Scalar<Int32Type>>(&json).unwrap(),
            scalar
        );
    }
    let name = Scalar::from_string(Utf8Type, "ygg");
    let json = serde_json::to_string(&name).unwrap();
    assert_eq!(
        serde_json::from_str::<Scalar<Utf8Type>>(&json).unwrap(),
        name
    );
}

#[test]
fn deserialization_revalidates_the_layout() {
    let short = serde_json::json!({ "data_type": null, "value": [0, 0, 0] });
    assert!(serde_json::from_value::<Scalar<Int32Type>>(short).is_err());
    let not_utf8 = serde_json::json!({ "data_type": null, "value": [255] });
    assert!(serde_json::from_value::<Scalar<Utf8Type>>(not_utf8).is_err());
}
