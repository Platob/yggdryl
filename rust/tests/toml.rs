use std::io::{Cursor, Read, Write};

use yggdryl::text::{self, Format, Limits};
use yggdryl::{TimeUnit, Timezone, Value, toml};

struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

#[test]
fn plain_toml_and_borrowed_entry_points_preserve_order_and_types() {
    let source = r#"
title = "TOML Example"
active = true
count = -42
ratio = -0.0
items = [1, "two", false]

[owner]
name = "José"
"#;
    let value = toml::from_str(source).unwrap();
    assert_eq!(toml::from_slice(source.as_bytes()).unwrap(), value);
    assert_eq!(
        toml::from_str_with_limits(source, Limits::default()).unwrap(),
        value
    );
    assert_eq!(
        toml::from_reader(OneByte(Cursor::new(source.as_bytes()))).unwrap(),
        value
    );
    let keys = value
        .mapping_iter()
        .map(|(key, _)| key.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        ["title", "active", "count", "ratio", "items", "owner"]
    );
    assert_eq!(
        value
            .get_key_str("ratio")
            .unwrap()
            .as_f64()
            .unwrap()
            .to_bits(),
        (-0.0_f64).to_bits()
    );
}

#[test]
fn empty_and_comment_only_documents_are_empty_tables() {
    let empty = Value::from_mapping([]).unwrap();
    for source in ["", " \t\r\n", "# comment\n  # another\r\n"] {
        assert_eq!(toml::from_str(source).unwrap(), empty);
        assert_eq!(toml::from_str_all(source).unwrap(), vec![empty.clone()]);
        assert_eq!(
            toml::Reader::new(Cursor::new(source.as_bytes()))
                .collect::<yggdryl::Result<Vec<_>>>()
                .unwrap(),
            vec![empty.clone()]
        );
    }
    assert!(toml::to_vec(&empty).unwrap().is_empty());
}

#[test]
fn every_shared_value_variant_round_trips_through_typed_envelopes() {
    let arbitrary_mapping = Value::from_mapping([
        (
            Value::from_sequence([Value::I64(1)]),
            Value::from(vec![0, 1, 255]),
        ),
        (Value::Bool(true), Value::Null),
    ])
    .unwrap();
    let value = Value::from_mapping([
        (Value::from("null"), Value::Null),
        (Value::from("bool"), Value::Bool(false)),
        (Value::from("i64"), Value::I64(i64::MIN)),
        (Value::from("u64"), Value::U64(u64::MAX)),
        (Value::from("i128"), Value::I128(i128::MIN)),
        (Value::from("u128"), Value::U128(u128::MAX)),
        (Value::from("float"), Value::from(f64::NAN)),
        (Value::from("string"), Value::from("café")),
        (Value::from("bytes"), Value::from(vec![0, 255])),
        (Value::from("decimal"), Value::decimal(i128::MIN, -128)),
        (Value::from("date"), Value::date(i32::MIN)),
        (
            Value::from("time"),
            Value::time(i64::MAX, TimeUnit::Nanosecond),
        ),
        (
            Value::from("timestamp"),
            // An interval-layout unit has no classic ISO spelling, so this is
            // the timestamp that still rides the typed envelope.
            Value::Timestamp(
                7,
                TimeUnit::YearMonth,
                yggdryl::Timezone::from_str("Asia/Kolkata").unwrap(),
            ),
        ),
        (
            Value::from("duration"),
            Value::duration(i64::MIN, TimeUnit::MonthDayNano),
        ),
        (
            Value::from("sequence"),
            Value::from_sequence([Value::I64(1), Value::from("two")]),
        ),
        (Value::from("mapping"), arbitrary_mapping),
    ])
    .unwrap();

    let encoded = toml::to_vec(&value).unwrap();
    let decoded = toml::from_slice(&encoded).unwrap();
    assert_eq!(decoded, value);
    assert!(matches!(decoded.get_key_str("u64"), Some(Value::U64(_))));
    assert!(matches!(decoded.get_key_str("i128"), Some(Value::I128(_))));
    assert!(matches!(decoded.get_key_str("u128"), Some(Value::U128(_))));
    assert!(matches!(
        decoded.get_key_str("bytes"),
        Some(Value::Bytes(_))
    ));
    assert!(matches!(
        decoded.get_key_str("mapping"),
        Some(Value::Mapping(_))
    ));
    // A limit of the storage keeps both of its parts, not just its number.
    assert_eq!(
        decoded.get_key_str("decimal").and_then(Value::as_decimal),
        Some((i128::MIN, -128))
    );
    assert_eq!(
        decoded.get_key_str("duration").and_then(Value::as_duration),
        Some((i64::MIN, TimeUnit::MonthDayNano))
    );
    assert_eq!(
        decoded
            .get_key_str("timestamp")
            .and_then(Value::as_timestamp),
        Some((7, TimeUnit::YearMonth, Some("Asia/Kolkata")))
    );
}

#[test]
fn every_root_value_shape_round_trips() {
    let values = [
        Value::Null,
        Value::Bool(true),
        Value::I64(1),
        Value::U64(1),
        Value::I128(1),
        Value::U128(1),
        Value::from(-0.0_f64),
        Value::from("root"),
        Value::from(vec![1, 2]),
        Value::from_sequence([Value::Null, Value::U64(2)]),
        Value::decimal(1_050, 2),
        // A root temporal TOML spells natively rides inside the `value`
        // envelope; one it cannot spell is the envelope body itself.
        Value::date(3_433),
        Value::time(27_120, TimeUnit::Second),
        Value::timestamp_in(296_638_320, TimeUnit::Second, Some(Timezone::UTC)),
        Value::datetime(296_638_320, TimeUnit::Second),
    ];
    for value in values {
        let decoded = toml::from_slice(&toml::to_vec(&value).unwrap()).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(
            std::mem::discriminant(&decoded),
            std::mem::discriminant(&value)
        );
    }

    // A named zone has no native TOML offset and a duration has no TOML
    // syntax; both go out as their classic ISO strings and come back as
    // strings - the schema layer is what recovers the typed reading.
    let paris = Value::timestamp(296_638_320, TimeUnit::Second, Some("Europe/Paris")).unwrap();
    assert_eq!(
        toml::from_slice(&toml::to_vec(&paris).unwrap()).unwrap(),
        Value::from("1979-05-27T09:32:00+02:00[Europe/Paris]")
    );
    let took = Value::duration(90, TimeUnit::Second);
    assert_eq!(
        toml::from_slice(&toml::to_vec(&took).unwrap()).unwrap(),
        Value::from("PT90S")
    );
}

#[test]
fn geospatial_values_round_trip_through_the_typed_envelope() {
    // One valid little-endian WKB `POINT (1 2)`.
    let mut wkb = vec![0x01_u8, 0x01, 0x00, 0x00, 0x00];
    wkb.extend_from_slice(&1.0_f64.to_le_bytes());
    wkb.extend_from_slice(&2.0_f64.to_le_bytes());
    let point = Value::Geospatial(wkb.into());

    // At the root, nested in a sequence, and nested in a mapping alike.
    let root = toml::from_slice(&toml::to_vec(&point).unwrap()).unwrap();
    assert_eq!(root, point);
    assert!(matches!(root, Value::Geospatial(_)));

    let nested = Value::from_mapping([
        (Value::from("shape"), point.clone()),
        (
            Value::from("shapes"),
            Value::from_sequence([point.clone(), Value::Bool(true)]),
        ),
    ])
    .unwrap();
    let encoded = toml::to_vec(&nested).unwrap();
    let text = std::str::from_utf8(&encoded).unwrap();
    assert!(text.contains("type = \"geospatial\""), "{text}");
    let decoded = toml::from_slice(&encoded).unwrap();
    assert_eq!(decoded, nested);
    assert!(matches!(
        decoded.get_key_str("shape"),
        Some(Value::Geospatial(_))
    ));

    // A user mapping shaped exactly like the envelope is escaped, so it comes
    // back as the mapping it is rather than as a geometry.
    let body = Value::from_mapping([
        (Value::from("version"), Value::I64(1)),
        (Value::from("type"), Value::from("geospatial")),
        (
            Value::from("value"),
            Value::from("AQEAAAAAAAAAAADwPwAAAAAAAABA"),
        ),
    ])
    .unwrap();
    let collision = Value::from_mapping([(Value::from("$yggdryl"), body)]).unwrap();
    let encoded = toml::to_vec(&collision).unwrap();
    let decoded = toml::from_slice(&encoded).unwrap();
    assert_eq!(decoded, collision);
    assert!(matches!(decoded, Value::Mapping(_)));

    // The recognized envelope still refuses a payload that is not base64.
    assert!(
        toml::from_str(r#""$yggdryl" = { version = 1, type = "geospatial", value = "?" }"#)
            .unwrap_err()
            .to_string()
            .contains("base64")
    );
}

#[test]
fn exact_envelope_collisions_are_escaped_and_non_exact_shapes_stay_plain() {
    let body = Value::from_mapping([
        (Value::from("version"), Value::I64(1)),
        (Value::from("type"), Value::from("bytes")),
        (Value::from("value"), Value::from("AA==")),
    ])
    .unwrap();
    let collision = Value::from_mapping([(Value::from("$yggdryl"), body)]).unwrap();
    let encoded = toml::to_vec(&collision).unwrap();
    assert_eq!(toml::from_slice(&encoded).unwrap(), collision);

    for source in [
        r#""$yggdryl" = { version = 2, type = "bytes", value = "AA==" }"#,
        r#""$yggdryl" = { version = 1, type = "future", value = "AA==" }"#,
        r#""$yggdryl" = { version = 1, type = "bytes", value = "AA==", extra = true }"#,
    ] {
        assert!(matches!(toml::from_str(source).unwrap(), Value::Mapping(_)));
    }
    assert!(
        toml::from_str(r#""$yggdryl" = { version = 1, type = "bytes", value = "?" }"#)
            .unwrap_err()
            .to_string()
            .contains("base64")
    );
}

#[test]
fn upstream_private_datetime_key_is_always_ordinary_user_data() {
    let source = r#"value = { "$__toml_private_datetime" = "1979-05-27" }"#;
    let value = toml::from_str(source).unwrap();
    let nested = value.get_key_str("value").unwrap();
    assert_eq!(
        nested.get_key_str("$__toml_private_datetime"),
        Some(&Value::from("1979-05-27"))
    );
    // Serde's private datetime marker is a user key like any other: it stays a
    // one-entry mapping of text and never becomes a temporal.
    assert!(matches!(nested, Value::Mapping(_)));
    assert_eq!(nested.len(), 1);
}

#[test]
fn each_native_datetime_decodes_to_its_temporal_and_writes_back_in_the_same_syntax() {
    let source = concat!(
        "offset = 1979-05-27T07:32:00-07:00\n",
        "zulu = 1979-05-27T07:32:00.123456789Z\n",
        "local_datetime = 1979-05-27T07:32:00.123\n",
        "local_date = 1979-05-27\n",
        "local_time = 07:32:00\n",
    );
    let value = toml::from_str(source).unwrap();

    // 1979-05-27 is day 3433, so 07:32:00 UTC that day is second 296638320,
    // and the offset reading is that instant seven hours later.
    assert_eq!(
        value.get_key_str("offset"),
        Some(&Value::timestamp(296_663_520, TimeUnit::Second, Some("-07:00")).unwrap())
    );
    assert_eq!(
        value.get_key_str("zulu"),
        Some(&Value::timestamp_in(
            296_638_320_123_456_789,
            TimeUnit::Nanosecond,
            Some(Timezone::UTC)
        ))
    );
    assert_eq!(
        value.get_key_str("local_datetime"),
        Some(&Value::timestamp_in(
            296_638_320_123,
            TimeUnit::Millisecond,
            None
        ))
    );
    assert_eq!(value.get_key_str("local_date"), Some(&Value::date(3_433)));
    assert_eq!(
        value.get_key_str("local_time"),
        Some(&Value::time(27_120, TimeUnit::Second))
    );

    // The unit is the coarsest one that keeps every digit, so a whole second
    // stays seconds and a nine-digit fraction becomes nanoseconds.
    assert_eq!(
        value
            .get_key_str("zulu")
            .and_then(Value::as_timestamp)
            .map(|(_, unit, zone)| (unit, zone)),
        Some((TimeUnit::Nanosecond, Some("UTC")))
    );

    // Each form goes back out in the syntax it arrived in, byte for byte.
    let encoded = String::from_utf8(toml::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        encoded,
        concat!(
            "\"offset\" = 1979-05-27T07:32:00-07:00\n",
            "\"zulu\" = 1979-05-27T07:32:00.123456789Z\n",
            "\"local_datetime\" = 1979-05-27T07:32:00.123\n",
            "\"local_date\" = 1979-05-27\n",
            "\"local_time\" = 07:32:00\n",
        )
    );
    assert_eq!(toml::from_str(&encoded).unwrap(), value);
}

#[test]
fn a_spelling_is_read_for_the_instant_it_names_rather_than_for_its_own_shape() {
    // A space delimiter, trailing fractional zeros, a missing seconds field,
    // and `+00:00` for `Z` are four spellings of readings already above; the
    // value is what survives, so all four decode to the same temporal.
    for (source, expected) in [
        (
            "at = 1979-05-27 07:32:00.123000000\n",
            Value::timestamp_in(296_638_320_123, TimeUnit::Millisecond, None),
        ),
        (
            "at = 1979-05-27T07:32:00+00:00\n",
            Value::timestamp_in(296_638_320, TimeUnit::Second, Some(Timezone::UTC)),
        ),
        ("at = 07:32\n", Value::time(27_120, TimeUnit::Second)),
        (
            // A leap second has no room in a count from the epoch, so it reads
            // as the second that follows it rather than as a second of its own.
            "at = 1979-06-30T23:59:60Z\n",
            Value::timestamp_in(299_635_200, TimeUnit::Second, Some(Timezone::UTC)),
        ),
    ] {
        let value = toml::from_str(source).unwrap();
        assert_eq!(value.get_key_str("at"), Some(&expected), "{source}");
    }

    // One instant written in two units is one value, so a document that
    // changes resolution on the way through still compares equal.
    assert_eq!(
        toml::from_str("at = 1979-05-27T07:32:00.100Z\n").unwrap(),
        toml::from_str("at = 1979-05-27T07:32:00.1Z\n").unwrap()
    );
}

#[test]
fn a_reading_toml_spells_but_the_count_cannot_hold_is_refused_at_its_position() {
    // Year 9999 is inside TOML's syntax, but nanoseconds of it are outside an
    // i64, and refusing beats quietly dropping the fraction.
    let source = "far = 9999-12-31T23:59:59.123456789Z\n";
    match toml::from_str(source).unwrap_err() {
        yggdryl::Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "toml");
            assert_eq!(position, "far = ".len());
            assert!(reason.contains("precision"), "{reason}");
        }
        other => panic!("unexpected error: {other}"),
    }

    // The same instant at a resolution it fits in is ordinary data.
    let whole = toml::from_str("far = 9999-12-31T23:59:59Z\n").unwrap();
    assert_eq!(
        whole.get_key_str("far"),
        Some(&Value::timestamp_in(
            253_402_300_799,
            TimeUnit::Second,
            Some(Timezone::UTC)
        ))
    );
    assert_eq!(
        String::from_utf8(toml::to_vec(&whole).unwrap()).unwrap(),
        "\"far\" = 9999-12-31T23:59:59Z\n"
    );
}

#[test]
fn dotted_and_quoted_keys_tables_arrays_and_multiline_strings_are_complete() {
    let source = r#"
root."quoted.key"."".value = 1
multiline_basic = """alpha
beta"""
multiline_literal = '''gamma
delta'''
inline = { one = 1, nested = { two = 2 } }

[[products]]
name = "hammer"

[[products]]
name = "nail"
"#;
    let value = toml::from_str(source).unwrap();
    let keys = value
        .mapping_iter()
        .map(|(key, _)| key.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        [
            "root",
            "multiline_basic",
            "multiline_literal",
            "inline",
            "products"
        ]
    );
    assert_eq!(
        value
            .get_key_str("root")
            .unwrap()
            .get_key_str("quoted.key")
            .unwrap()
            .get_key_str("")
            .unwrap()
            .get_key_str("value"),
        Some(&Value::I64(1))
    );
    assert_eq!(
        value.get_key_str("multiline_basic"),
        Some(&Value::from("alpha\nbeta"))
    );
    assert_eq!(
        value.get_key_str("multiline_literal"),
        Some(&Value::from("gamma\ndelta"))
    );
    let products = value
        .get_key_str("products")
        .unwrap()
        .as_sequence()
        .unwrap();
    assert_eq!(products.len(), 2);
    assert_eq!(
        products[0].get_key_str("name"),
        Some(&Value::from("hammer"))
    );
    assert_eq!(products[1].get_key_str("name"), Some(&Value::from("nail")));
    assert_eq!(
        toml::from_slice(&toml::to_vec(&value).unwrap()).unwrap(),
        value
    );
}

#[test]
fn a_temporal_or_decimal_toml_cannot_spell_takes_its_classic_string_or_envelope() {
    let value = Value::from_mapping([
        // A zone that names a place is not an offset, and writing the offset
        // that place happens to be at would throw the name away - so the
        // classic string carries both, offset and bracketed name.
        (
            Value::from("zoned"),
            Value::timestamp(296_638_320, TimeUnit::Second, Some("Europe/Paris")).unwrap(),
        ),
        // Elapsed time has no TOML syntax, but it has a classic ISO spelling.
        (
            Value::from("elapsed"),
            Value::duration(90, TimeUnit::Second),
        ),
        // An exact decimal has neither, so it keeps the typed envelope.
        (Value::from("price"), Value::decimal(-1_050, 2)),
        // A year outside four digits, and a clock reading outside one day,
        // are outside both syntaxes rather than outside the value.
        (Value::from("far"), Value::date(2_932_897)),
        (
            Value::from("overflowing"),
            Value::time(86_400, TimeUnit::Second),
        ),
    ])
    .unwrap();

    let encoded = toml::to_vec(&value).unwrap();
    let source = String::from_utf8(encoded.clone()).unwrap();
    for expected in [
        r#""zoned" = "1979-05-27T09:32:00+02:00[Europe/Paris]""#,
        r#""elapsed" = "PT90S""#,
        r#""price" = { "$yggdryl" = { version = 1, type = "decimal", value = ["-1050", 2] } }"#,
        r#""far" = { "$yggdryl" = { version = 1, type = "date", value = 2932897 } }"#,
        r#""overflowing" = { "$yggdryl" = { version = 1, type = "time", value = ["s", 86400] } }"#,
    ] {
        assert!(source.contains(expected), "{source}");
    }

    let decoded = toml::from_slice(&encoded).unwrap();
    // The classic strings come back as strings - loose typing is the deal -
    // and the enveloped readings come back typed.
    assert_eq!(
        decoded.get_key_str("zoned"),
        Some(&Value::from("1979-05-27T09:32:00+02:00[Europe/Paris]"))
    );
    assert_eq!(decoded.get_key_str("elapsed"), Some(&Value::from("PT90S")));
    // Equality alone would accept a rescaled decimal, so pin the scale too:
    // it is data, not a spelling.
    assert_eq!(
        decoded.get_key_str("price").and_then(Value::as_decimal),
        Some((-1_050, 2))
    );
    assert_eq!(
        decoded.get_key_str("far").and_then(Value::as_date),
        Some(2_932_897)
    );
}

#[test]
fn a_reserved_toml_name_is_now_ordinary_application_data() {
    // The four `toml:` tags were a vocabulary this codec understood, and the
    // `tag` envelope was the only syntax that spelled one. Both are gone, so a
    // document still carrying that exact text is data with no meaning here: it
    // decodes as the mapping it is written as and never lowers to native
    // date-time syntax again.
    let source = concat!(
        r#"day = { "$yggdryl" = { version = 1, type = "tag", "#,
        r#"tag = "toml:local-date", value = "1979-05-27" } }"#,
        "\n",
    );
    let value = toml::from_str(source).unwrap();
    let body = value
        .get_key_str("day")
        .and_then(|day| day.get_key_str("$yggdryl"))
        .expect("the marker stays an ordinary key");
    assert_eq!(
        body.get_key_str("tag"),
        Some(&Value::from("toml:local-date"))
    );
    assert_eq!(body.get_key_str("value"), Some(&Value::from("1979-05-27")));

    let encoded = toml::to_vec(&value).unwrap();
    let written = String::from_utf8(encoded.clone()).unwrap();
    assert!(!written.contains("= 1979-05-27\n"), "{written}");
    assert!(!written.contains(r#"type = "tag""#), "{written}");
    assert_eq!(toml::from_slice(&encoded).unwrap(), value);
}

#[test]
fn integer_float_and_toml_1_1_semantics_are_exact() {
    let source = concat!(
        "minimum = -9223372036854775808\n",
        "maximum = 9223372036854775807\n",
        "hex = 0x7fff_ffff\n",
        "negative_zero = -0.0\n",
        "positive_infinity = inf\n",
        "negative_infinity = -inf\n",
        "not_a_number = nan\n",
        "escape = \"\\e\"\n",
        "byte_escape = \"\\x41\"\n",
    );
    let value = toml::from_str(source).unwrap();
    assert_eq!(value.get_key_str("minimum"), Some(&Value::I64(i64::MIN)));
    assert_eq!(value.get_key_str("maximum"), Some(&Value::I64(i64::MAX)));
    assert_eq!(value.get_key_str("hex"), Some(&Value::I64(0x7fff_ffff)));
    assert_eq!(
        value
            .get_key_str("negative_zero")
            .unwrap()
            .as_f64()
            .unwrap()
            .to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(value.get_key_str("escape"), Some(&Value::from("\u{001b}")));
    assert_eq!(value.get_key_str("byte_escape"), Some(&Value::from("A")));
    assert!(toml::from_str("too_large = 9223372036854775808").is_err());
    assert!(toml::from_str("too_small = -9223372036854775809").is_err());
}

/// Emit one float as the sole entry of a document and return just its spelling.
fn emitted_float(value: f64) -> String {
    let document = Value::from_mapping([(Value::from("f"), Value::from(value))]).unwrap();
    let encoded = String::from_utf8(toml::to_vec(&document).unwrap()).unwrap();
    encoded
        .trim_end_matches('\n')
        .strip_prefix("\"f\" = ")
        .expect("a plain float entry")
        .to_owned()
}

#[test]
fn floats_are_written_in_the_shortest_spelling_that_reads_back_bit_identically() {
    for value in [
        1e300_f64,
        5e-324_f64,
        f64::MAX,
        f64::MIN,
        6.626e-34_f64,
        -0.0_f64,
        0.1_f64,
        1.234_567_890_123_456_7_f64,
        1.0_f64,
    ] {
        let spelling = emitted_float(value);
        let source = format!("f = {spelling}\n");
        let decoded = toml::from_str(&source).unwrap();
        assert_eq!(
            decoded
                .get_key_str("f")
                .unwrap()
                .as_f64()
                .unwrap()
                .to_bits(),
            value.to_bits(),
            "{source}"
        );
    }

    // A magnitude far from one takes exponent form, so it stays a handful of
    // characters instead of the three hundred and two digits that Rust's
    // `Display` spells 1e300 as. The exact digits belong to the formatter, but
    // the length and the shape of the spelling are the point of the fix.
    for (value, longest) in [
        (1e300_f64, 20),
        (5e-324_f64, 20),
        (6.626e-34_f64, 20),
        (f64::MAX, 25),
        (f64::MIN, 26),
    ] {
        let spelling = emitted_float(value);
        assert!(spelling.len() < longest, "{spelling}");
        assert!(spelling.contains('e'), "{spelling}");
        // An exponent already makes the spelling a float to the TOML grammar,
        // so it must not also collect the trailing fractional part.
        assert!(!spelling.ends_with(".0"), "{spelling}");
    }

    // A spelling near one stays in plain decimal form, and an integral one
    // still takes the fractional part that keeps it from reading as an integer.
    assert_eq!(emitted_float(1.0), "1.0");
    assert_eq!(emitted_float(-0.0), "-0.0");
    assert_eq!(emitted_float(0.1), "0.1");
    assert_eq!(emitted_float(1.234_567_890_123_456_7), "1.2345678901234567");
}

#[test]
fn float_overflow_is_rejected_while_underflow_rounds_to_a_signed_zero() {
    for source in ["f = 1e400\n", "f = -1e400\n", "f = 1.8e308\n"] {
        match toml::from_str(source).unwrap_err() {
            yggdryl::Error::Codec {
                format,
                position,
                reason,
            } => {
                assert_eq!(format, "toml");
                assert_eq!(position, "f = ".len());
                assert!(reason.contains("f64 range"), "{reason}");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    for (source, expected) in [
        ("f = 1e-400\n", 0.0_f64),
        ("f = -1e-400\n", -0.0_f64),
        ("f = 1e-99999999999999999999\n", 0.0_f64),
        ("f = 4.9e-324\n", 5e-324_f64),
    ] {
        let decoded = toml::from_str(source).unwrap();
        assert_eq!(
            decoded
                .get_key_str("f")
                .unwrap()
                .as_f64()
                .unwrap()
                .to_bits(),
            expected.to_bits(),
            "{source}"
        );
    }
}

#[test]
fn all_ascii_controls_and_del_round_trip_in_keys_and_values() {
    let controls = (0_u8..=0x1f)
        .chain(std::iter::once(0x7f))
        .map(char::from)
        .collect::<String>();
    let value =
        Value::from_mapping([(Value::from(controls.clone()), Value::from(controls))]).unwrap();
    let encoded = toml::to_vec(&value).unwrap();
    assert_eq!(toml::from_slice(&encoded).unwrap(), value);
}

#[test]
fn duplicate_keys_and_invalid_utf8_report_original_byte_positions() {
    let duplicate = "ok = 0\nnested = { a = 1, a = 2 }\n";
    let expected = duplicate.rfind("a = 2").unwrap();
    match toml::from_str(duplicate).unwrap_err() {
        yggdryl::Error::Codec {
            format,
            position,
            reason,
        } => {
            assert_eq!(format, "toml");
            assert_eq!(position, expected);
            assert!(reason.contains("duplicate"), "{reason}");
        }
        other => panic!("unexpected error: {other}"),
    }

    match toml::from_slice(b"ok = 1\nname = \xff").unwrap_err() {
        yggdryl::Error::Codec { position, .. } => assert_eq!(position, b"ok = 1\nname = ".len()),
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn node_depth_byte_and_document_limits_apply_at_exact_boundaries() {
    let empty = Limits::new(1, 0, 1, 1);
    assert_eq!(toml::from_str_with_limits("", empty).unwrap().len(), 0);
    assert!(toml::from_str_with_limits("", Limits::new(0, 0, 1, 1)).is_err());
    assert!(toml::from_str_with_limits("", Limits::new(1, 0, 0, 1)).is_err());
    assert!(toml::from_str_with_limits("", Limits::new(1, 0, 1, 0)).is_err());

    assert!(toml::from_str_with_limits("a = 1", Limits::new(1, 5, 3, 1)).is_ok());
    let nodes = toml::from_str_with_limits("a = 1", Limits::new(1, 5, 2, 1)).unwrap_err();
    assert!(nodes.to_string().contains("node limit"));
    assert!(toml::from_str_with_limits("a = []", Limits::new(1, 6, 3, 1)).is_err());
    assert!(toml::from_str_with_limits("a = []", Limits::new(2, 6, 3, 1)).is_ok());
    assert!(toml::from_str_with_limits("a = 1 ", Limits::new(1, 5, 3, 1)).is_err());
}

#[test]
fn parser_and_encoder_share_the_published_hard_depth_cap() {
    fn nested(depth: usize) -> Value {
        (0..depth).fold(Value::I64(0), |value, _| Value::from_sequence([value]))
    }

    let at_limit =
        Value::from_mapping([(Value::from("value"), nested(toml::MAX_PARSER_DEPTH - 1))]).unwrap();
    let encoded = toml::to_vec(&at_limit).unwrap();
    let limits = Limits::new(
        toml::MAX_PARSER_DEPTH + 10,
        encoded.len(),
        toml::MAX_PARSER_DEPTH + 10,
        1,
    );
    assert_eq!(
        toml::from_slice_with_limits(&encoded, limits).unwrap(),
        at_limit
    );

    let over_limit =
        Value::from_mapping([(Value::from("value"), nested(toml::MAX_PARSER_DEPTH))]).unwrap();
    assert!(
        toml::to_vec(&over_limit)
            .unwrap_err()
            .to_string()
            .contains("hard limit")
    );

    struct PanicWriter;
    impl Write for PanicWriter {
        fn write(&mut self, _input: &[u8]) -> std::io::Result<usize> {
            panic!("TOML preflight must finish before writing")
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(toml::validate_for_write(&over_limit).is_err());
    assert!(toml::to_writer(PanicWriter, &over_limit).is_err());

    // A mapping TOML cannot key costs four levels on the wire per level of its
    // own - the wrapper table, the envelope body, the entry array, and the pair
    // array - so a chain of them is four times as deep as the value looks. The
    // preflight has to measure the projection rather than the value, and this
    // boundary measures exactly that charge.
    let arbitrary = |depth| {
        (0..depth).fold(Value::from("payload"), |value, _| {
            Value::from_mapping([(Value::Bool(true), value)]).unwrap()
        })
    };
    let runtime_limits = Limits::new(48, 1024, 1024, 1);
    assert!(toml::validate_for_write_with_limits(&arbitrary(12), runtime_limits).is_ok());
    assert!(toml::validate_for_write(&arbitrary(13)).is_ok());
    assert!(
        toml::validate_for_write_with_limits(&arbitrary(13), runtime_limits)
            .unwrap_err()
            .to_string()
            .contains("depth limit")
    );

    let source = format!(
        "value = {}0{}",
        "[".repeat(toml::MAX_PARSER_DEPTH),
        "]".repeat(toml::MAX_PARSER_DEPTH)
    );
    assert!(
        toml::from_str_with_limits(&source, Limits::new(1_000, source.len(), 1_000, 1),)
            .unwrap_err()
            .to_string()
            .contains("hard limit")
    );
}

#[test]
fn the_depth_a_value_costs_on_the_way_out_is_the_depth_it_costs_on_the_way_back() {
    /// Return the smallest depth budget that both writes and reads one value,
    /// or `None` when the preflight and the parser disagree about its cost.
    fn budget(value: Value) -> Option<usize> {
        let document = Value::from_mapping([(Value::from("value"), value)]).unwrap();
        let limits = |depth| Limits::new(depth, 4_096, 4_096, 1);
        let encoded = toml::to_vec(&document).unwrap();
        let written = (1..=16).find(|depth| {
            toml::validate_for_write_with_limits(&document, limits(*depth)).is_ok()
        })?;
        let read = (1..=16)
            .find(|depth| toml::from_slice_with_limits(&encoded, limits(*depth)).is_ok())?;
        (written == read).then_some(written)
    }

    // A value TOML spells itself is a leaf, whichever kind it is - and so is
    // a temporal that goes out as its classic ISO string.
    for value in [
        Value::from("text"),
        Value::date(3_433),
        Value::time(27_120, TimeUnit::Second),
        Value::timestamp_in(296_638_320, TimeUnit::Second, Some(Timezone::UTC)),
        Value::timestamp(0, TimeUnit::Second, Some("Europe/Paris")).unwrap(),
        Value::duration(90, TimeUnit::Second),
    ] {
        assert_eq!(budget(value.clone()), Some(1), "{value:?}");
    }

    // A value TOML has no syntax for gains the envelope table and its body.
    for value in [
        Value::Null,
        Value::U64(1),
        Value::from(vec![0_u8]),
        Value::date(2_932_897),
    ] {
        assert_eq!(budget(value.clone()), Some(3), "{value:?}");
    }

    // ... and one level more when that body carries an array rather than a
    // scalar, which is what a unit, a count, and a scale travel in.
    for value in [
        Value::decimal(1_050, 2),
        Value::duration(1, TimeUnit::YearMonth),
        Value::time(86_400, TimeUnit::Second),
    ] {
        assert_eq!(budget(value.clone()), Some(4), "{value:?}");
    }
}

#[test]
fn readers_are_bounded_single_document_iterators_and_zero_documents_do_not_read() {
    struct PanicReader;
    impl Read for PanicReader {
        fn read(&mut self, _output: &mut [u8]) -> std::io::Result<usize> {
            panic!("document limit must be checked before reading")
        }
    }

    let mut denied = toml::Reader::with_limits(PanicReader, Limits::new(1, 1, 1, 0));
    assert!(denied.next().unwrap().is_err());
    assert!(denied.next().is_none());

    let limits = Limits::new(8, 5, 16, 1);
    let mut reader = toml::Reader::with_limits(Cursor::new(b"a = 1"), limits);
    assert!(reader.next().unwrap().is_ok());
    assert_eq!(reader.byte_offset(), 5);
    assert!(reader.next().is_none());
    assert!(toml::from_reader_with_limits(Cursor::new(b"a = 1 "), limits).is_err());
}

#[test]
fn writers_accept_short_writes_and_all_requires_exactly_one_value_before_output() {
    #[derive(Default)]
    struct ShortWriter(Vec<u8>);
    impl Write for ShortWriter {
        fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
            let length = input.len().min(2);
            self.0.extend_from_slice(&input[..length]);
            Ok(length)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let value = Value::from_mapping([(Value::from("key"), Value::from("value"))]).unwrap();
    let expected = toml::to_vec(&value).unwrap();
    let mut writer = ShortWriter::default();
    toml::to_writer(&mut writer, &value).unwrap();
    assert_eq!(writer.0, expected);

    let mut output = Vec::new();
    assert!(toml::to_writer_all(&mut output, std::iter::empty::<Value>()).is_err());
    assert!(output.is_empty());
    assert!(toml::to_writer_all(&mut output, [value.clone(), value.clone()]).is_err());
    assert!(output.is_empty());
    toml::to_writer_all(&mut output, std::iter::once(&value)).unwrap();
    assert_eq!(output, expected);
    assert_eq!(toml::from_slice_all(&output).unwrap(), vec![value]);
}

#[test]
fn generic_dispatch_and_inference_are_deterministic_and_single_pass_compatible() {
    let source = "name = \"yggdryl\"\n";
    let expected = toml::from_str(source).unwrap();
    assert_eq!(text::from_str(source, Format::Toml).unwrap(), expected);
    assert_eq!(
        text::from_str_all(source, Format::Toml).unwrap(),
        vec![expected.clone()]
    );
    assert_eq!(text::infer_format(source.as_bytes()).unwrap(), Format::Toml);
    assert_eq!(
        text::from_str_inferred(source).unwrap(),
        (Format::Toml, expected)
    );
    assert_eq!(
        text::infer_format(br#"{"name":"yggdryl"}"#).unwrap(),
        Format::Json
    );
    assert_eq!(
        text::infer_format(b"name: yggdryl\n").unwrap(),
        Format::Yaml
    );
    assert_eq!(text::infer_format(b"").unwrap(), Format::Yaml);
    assert_eq!(text::infer_format(b"# comment\n").unwrap(), Format::Yaml);
}
