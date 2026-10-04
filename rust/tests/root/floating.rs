//! `rust/src/floating.rs`.

use yggdryl::floating;
use yggdryl::{DataType, Scalar};

use super::typed::assert_typed_marker;

#[test]
fn floating_markers_cover_every_width() {
    assert_typed_marker::<floating::Float16Type>(DataType::Float16);
    assert_typed_marker::<floating::Float32Type>(DataType::Float32);
    assert_typed_marker::<floating::Float64Type>(DataType::Float64);
}

#[test]
fn scalar_float_arithmetic_keeps_native_bits() {
    let left = 1.0_f64;
    let right = -(1.0 - 2.0_f64.powi(-52));
    let left_scalar = Scalar::from(left);
    let right_scalar = Scalar::from(right);
    for (actual, expected) in [
        (left_scalar.checked_add(&right_scalar), left + right),
        (left_scalar.checked_sub(&right_scalar), left - right),
        (left_scalar.checked_mul(&right_scalar), left * right),
        (left_scalar.checked_div(&right_scalar), left / right),
        (left_scalar.checked_rem(&right_scalar), left % right),
    ] {
        assert_eq!(
            actual.unwrap().as_f64().unwrap().to_bits(),
            expected.to_bits()
        );
    }
    let tiny = Scalar::from(f64::MIN_POSITIVE);
    assert_eq!(
        tiny.checked_div(&Scalar::from(2.0))
            .unwrap()
            .as_f64()
            .unwrap()
            .to_bits(),
        (f64::MIN_POSITIVE / 2.0).to_bits(),
    );
    assert!(
        Scalar::from(f64::MAX)
            .checked_mul(&Scalar::from(2.0))
            .unwrap()
            .as_f64()
            .unwrap()
            .is_infinite(),
    );
}

mod reading {
    //! The one float grammar, read through the value door.

    use yggdryl::{DataType, Scalar};

    #[test]
    fn the_value_door_reads_a_decimal_fraction_or_an_exponent_and_rounds_to_the_width() {
        for (text, expected) in [
            (" 2.5 ", 2.5_f64),
            ("1e3", 1000.0),
            ("-1.25E-2", -0.0125),
            ("+3", 3.0),
            (".5", 0.5),
            ("5.", 5.0),
            ("0", 0.0),
        ] {
            assert_eq!(
                DataType::Float64.scalar(Scalar::from(text)).expect(text),
                Scalar::from(expected),
                "{text:?}"
            );
        }
        assert_eq!(
            DataType::Float32.scalar(Scalar::from("0.1")).unwrap(),
            Scalar::from(0.1_f32)
        );
        for text in [" ", "1,5", "1_000", "0x10", "1e", "--1", "1.2.3", "two"] {
            assert!(
                DataType::Float64.scalar(Scalar::from(text)).is_err(),
                "{text:?} read as a float"
            );
        }
        // The empty text is absence at the door, never a spelling the reader
        // sees; the column's nullability is what may refuse it.
        assert_eq!(
            DataType::Float64.scalar(Scalar::from("")).unwrap(),
            Scalar::Null
        );
    }
}

#[cfg(feature = "internals")]
mod internal {
    //! The grammar itself, which a setting reads a seconds count through.

    use yggdryl::internals::floating::{FLOAT_SPELLINGS, f64_from_text};

    #[test]
    fn f64_from_text_reads_rusts_float_grammar_trimmed_including_the_non_finite_spellings() {
        assert_eq!(f64_from_text(" 1.5\t"), Some(1.5));
        assert_eq!(f64_from_text("1e300"), Some(1e300));
        assert_eq!(f64_from_text("+3"), Some(3.0));
        assert_eq!(f64_from_text("-0"), Some(-0.0));
        assert_eq!(f64_from_text("inf"), Some(f64::INFINITY));
        assert_eq!(f64_from_text("-Infinity"), Some(f64::NEG_INFINITY));
        assert!(f64_from_text("NaN").is_some_and(f64::is_nan));
        for text in ["", "1_000", "1,5", "0x10", "1e", "1.2.3", "1 0", "two"] {
            assert_eq!(f64_from_text(text), None, "{text:?}");
        }
        assert_eq!(
            FLOAT_SPELLINGS,
            "a number, with an optional fraction or exponent"
        );
    }
}
