//! `rust/src/variant.rs`: the Variant binary encoding, byte for byte.

use yggdryl::internals::variant::{
    INT32, OBJECT, PRIMITIVE, SHORT_STRING, SHORT_STRING_MAX, STRING,
};
use yggdryl::{Scalar, VARIANT_VERSION, Variant};

#[test]
fn a_short_string_folds_its_length_into_the_header() {
    let variant = Variant::encode(&Scalar::from("abc")).unwrap();
    assert_eq!(variant.metadata(), [0x11, 0, 0]);
    assert_eq!(variant.value(), [SHORT_STRING | (3 << 2), b'a', b'b', b'c']);
    assert_eq!(variant.scalar().unwrap(), Scalar::from("abc"));
}

#[test]
fn a_long_string_states_its_size_in_four_bytes() {
    let long = "x".repeat(SHORT_STRING_MAX + 1);
    let variant = Variant::encode(&Scalar::from(long.as_str())).unwrap();
    assert_eq!(variant.value()[0], PRIMITIVE | (STRING << 2));
    assert_eq!(variant.scalar().unwrap(), Scalar::from(long.as_str()));
}

#[test]
fn an_object_names_its_keys_by_dictionary_index() {
    let quote = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("size", Scalar::from(100_i64)),
    ])
    .unwrap();
    let variant = Variant::encode(&quote).unwrap();
    // The dictionary is sorted: `size` is id 0 and `symbol` id 1.
    assert_eq!(variant.metadata()[0] & 0x0f, VARIANT_VERSION);
    assert_eq!(variant.metadata()[0] >> 4 & 0x01, 1, "sorted_strings");
    assert_eq!(
        &variant.metadata()[variant.metadata().len() - 10..],
        b"sizesymbol"
    );
    assert_eq!(variant.value()[0] & 0x03, OBJECT);
    assert_eq!(variant.scalar().unwrap(), quote);
}

#[test]
fn the_refusals_name_the_byte() {
    let refused = Variant::new(vec![0x02_u8], vec![0_u8])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("version 2"), "{refused}");
    let variant = Variant::new(vec![0x11_u8, 0, 0], vec![PRIMITIVE | (INT32 << 2), 1]).unwrap();
    let refused = variant.scalar().unwrap_err().to_string();
    assert!(refused.contains("4 bytes announced"), "{refused}");
    let variant = Variant::new(vec![0x11_u8, 0, 0], vec![PRIMITIVE | (60 << 2)]).unwrap();
    let refused = variant.scalar().unwrap_err().to_string();
    assert!(refused.contains("primitive type 60"), "{refused}");
}
