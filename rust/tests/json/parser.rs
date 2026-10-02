//! `rust/src/json/parser.rs`.

use yggdryl::Scalar;

fn read(document: &str) -> yggdryl::Result<Scalar> {
    yggdryl::json::from_bytes(document.as_bytes())
}

/// An integer token is the narrowest of `u64`, `i64`, `u128` and `i128` its
/// sign and size take, at every digit count either side of the widths, and
/// one past `u128` or below `i128` is refused.
#[test]
fn an_integer_token_reads_at_the_width_its_digits_take() {
    for digits in 1..=40 {
        let text = "9".repeat(digits);
        let expected = text
            .parse::<u64>()
            .map(Scalar::from)
            .or_else(|_| text.parse::<u128>().map(Scalar::from));
        match expected {
            Ok(expected) => assert_eq!(read(&text).unwrap(), expected, "{digits} nines"),
            Err(_) => assert!(read(&text).is_err(), "{digits} nines"),
        }
        let negative = format!("-{text}");
        let expected = negative
            .parse::<i64>()
            .map(Scalar::from)
            .or_else(|_| negative.parse::<i128>().map(Scalar::from));
        match expected {
            Ok(expected) => assert_eq!(read(&negative).unwrap(), expected, "-{digits} nines"),
            Err(_) => assert!(read(&negative).is_err(), "-{digits} nines"),
        }
    }
    for (text, expected) in [
        ("0", Scalar::from(0_u64)),
        ("-1", Scalar::from(-1_i64)),
        ("18446744073709551615", Scalar::from(u64::MAX)),
        (
            "18446744073709551616",
            Scalar::from(u128::from(u64::MAX) + 1),
        ),
        ("-9223372036854775808", Scalar::from(i64::MIN)),
        (
            "-9223372036854775809",
            Scalar::from(i128::from(i64::MIN) - 1),
        ),
        (
            "-999999999999999999",
            Scalar::from(-999_999_999_999_999_999_i64),
        ),
        (
            "340282366920938463463374607431768211455",
            Scalar::from(u128::MAX),
        ),
        (
            "-170141183460469231731687303715884105728",
            Scalar::from(i128::MIN),
        ),
    ] {
        assert_eq!(read(text).unwrap(), expected, "{text}");
    }
    assert!(read("340282366920938463463374607431768211456").is_err());
    assert!(read("-170141183460469231731687303715884105729").is_err());
}

/// `-0` is the one integer spelling that is a float, and a leading zero is
/// no spelling at all.
#[test]
fn negative_zero_is_a_float_and_a_leading_zero_is_refused() {
    let zero = read("-0").unwrap();
    assert_eq!(zero.as_f64().map(f64::to_bits), Some((-0.0_f64).to_bits()));
    assert_eq!(
        format!("{:?}", read("[-0, 0]").unwrap()),
        format!("{:?}", read("[-0.0, 0]").unwrap())
    );
    for refused in ["01", "-01", "00", "-", "1.", "1e", "-a"] {
        assert!(read(refused).is_err(), "{refused}");
    }
}

/// A string reads as its characters whether the document spells them as
/// they are or escapes them; a raw control character, bytes that are not
/// UTF-8, a lone surrogate and an unterminated string are refused.
#[test]
fn a_string_reads_as_its_characters_and_refuses_what_json_does_not_spell() {
    for (document, expected) in [
        (r#""plain""#, "plain"),
        (r#""café ☃""#, "café ☃"),
        (r#""a\"b\\c\/d\né😀""#, "a\"b\\c/d\né😀"),
        (r#""""#, ""),
        (r#""\u0000""#, "\0"),
    ] {
        assert_eq!(
            read(document).unwrap(),
            Scalar::from(expected),
            "{document}"
        );
    }
    for refused in [
        "\"a\nb\"",
        "\"a\u{1}b\"",
        r#""\ud800""#,
        r#""\q""#,
        "\"unterminated",
        "\"tail\\\"",
    ] {
        assert!(read(refused).is_err(), "{refused:?}");
    }
    assert!(yggdryl::json::from_bytes(b"\"\xff\"").is_err());
    assert!(yggdryl::json::from_bytes(b"\"ok\xc3\"").is_err());
}

/// Two keys that unescape alike are one key twice, which an object refuses.
#[test]
fn keys_that_unescape_alike_are_a_duplicate() {
    assert!(read(r#"{"a":1,"a":2}"#).is_err());
    assert!(read(r#"{"a":1,"b":2}"#).is_ok());
}
