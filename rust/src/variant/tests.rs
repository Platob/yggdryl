//! The Variant codec: what round-trips, what the spec's bytes decode to, and
//! what refuses with which position.

use smol_str::SmolStr;

use crate::{DataType, Field, Limits, TimeUnit, Timezone, Value};

use super::{decode_value, decode_value_with_limits, encode_value};

/// The metadata every value without object fields encodes: version 1, the
/// sorted bit, one-byte offsets, an empty dictionary.
/// VariantEncoding.md "Metadata encoding".
const EMPTY_METADATA: [u8; 3] = [0x11, 0x00, 0x00];

/// Encode, decode back, and hand over what came back.
fn round_trip(value: &Value) -> Value {
    let (metadata, bytes) = encode_value(value).expect("the value should encode");
    decode_value(&metadata, &bytes).expect("the encoding should decode")
}

/// Assert one value comes back exactly as it went in.
fn assert_round_trip(value: &Value) {
    assert_eq!(&round_trip(value), value, "{value:?} should round-trip");
}

/// The rendered message of the error a decode must produce.
fn decode_error(metadata: &[u8], value: &[u8]) -> String {
    decode_value(metadata, value)
        .expect_err("the bytes should refuse")
        .to_string()
}

/// The rendered message of the error an encode must produce.
fn encode_error(value: &Value) -> String {
    encode_value(value)
        .expect_err("the value should refuse")
        .to_string()
}

mod round_trips {
    use super::*;

    #[test]
    fn every_scalar_kind_comes_back_identical() {
        for value in [
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::I8(i8::MIN),
            Value::I8(i8::MAX),
            Value::I16(i16::MIN),
            Value::I16(i16::MAX),
            Value::I32(i32::MIN),
            Value::I32(i32::MAX),
            Value::I64(i64::MIN),
            Value::I64(i64::MAX),
            Value::F32(crate::Float32::from_f32(1.5)),
            Value::F32(crate::Float32::from_f32(f32::MIN)),
            Value::F64(crate::Float::from_f64(-0.0)),
            Value::F64(crate::Float::from_f64(f64::MAX)),
            Value::F64(crate::Float::from_f64(f64::NAN)),
            Value::Date(i32::MIN),
            Value::Date(i32::MAX),
            Value::Bytes([].as_slice().into()),
            Value::Bytes(vec![7_u8; 300].into()),
            Value::String(SmolStr::default()),
        ] {
            assert_round_trip(&value);
        }
    }

    #[test]
    fn unsigned_integers_come_back_at_the_next_signed_width() {
        let (metadata, bytes) = encode_value(&Value::U8(u8::MAX)).unwrap();
        assert_eq!(
            decode_value(&metadata, &bytes).unwrap(),
            Value::I16(i16::from(u8::MAX))
        );
        let (metadata, bytes) = encode_value(&Value::U16(u16::MAX)).unwrap();
        assert_eq!(
            decode_value(&metadata, &bytes).unwrap(),
            Value::I32(i32::from(u16::MAX))
        );
        let (metadata, bytes) = encode_value(&Value::U32(u32::MAX)).unwrap();
        assert_eq!(
            decode_value(&metadata, &bytes).unwrap(),
            Value::I64(i64::from(u32::MAX))
        );
    }

    #[test]
    fn wide_integers_come_back_as_int64_when_the_value_fits() {
        for value in [
            Value::U64(u64::try_from(i64::MAX).unwrap()),
            Value::I128(i128::from(i64::MIN)),
            Value::U128(42),
        ] {
            let (metadata, bytes) = encode_value(&value).unwrap();
            let decoded = decode_value(&metadata, &bytes).unwrap();
            assert!(matches!(decoded, Value::I64(_)), "got {decoded:?}");
        }
    }

    #[test]
    fn decimals_round_trip_at_every_physical_width() {
        // The decimal table: precision selects decimal4, decimal8, decimal16.
        for value in [
            Value::Decimal(0, 0),
            Value::Decimal(i128::from(i32::MIN), 9),
            Value::Decimal(i128::from(i32::MAX), 9),
            Value::Decimal(i128::from(i32::MAX) + 1, 18),
            Value::Decimal(i128::from(i64::MIN), 18),
            Value::Decimal(i128::from(i64::MAX) + 1, 38),
            Value::Decimal(super::super::DECIMAL_MAGNITUDE_LIMIT - 1, 38),
            Value::Decimal(1 - super::super::DECIMAL_MAGNITUDE_LIMIT, 0),
        ] {
            assert_round_trip(&value);
        }
    }

    #[test]
    fn both_timestamp_units_round_trip_in_both_zonings() {
        for value in [
            Value::Timestamp(i64::MIN, TimeUnit::Microsecond, Timezone::UTC),
            Value::Timestamp(i64::MAX, TimeUnit::Nanosecond, Timezone::UTC),
            Value::DateTime(i64::MIN, TimeUnit::Microsecond),
            Value::DateTime(i64::MAX, TimeUnit::Nanosecond),
            Value::Time(86_399_999_999, TimeUnit::Microsecond),
        ] {
            assert_round_trip(&value);
        }
    }

    #[test]
    fn coarse_temporal_units_widen_to_microseconds() {
        let seconds = Value::Timestamp(2, TimeUnit::Second, Timezone::UTC);
        assert_eq!(
            round_trip(&seconds),
            Value::Timestamp(2_000_000, TimeUnit::Microsecond, Timezone::UTC)
        );
        let millis = Value::DateTime(-3, TimeUnit::Millisecond);
        assert_eq!(
            round_trip(&millis),
            Value::DateTime(-3_000, TimeUnit::Microsecond)
        );
        let whole_micro_nanos = Value::Time(1_000, TimeUnit::Nanosecond);
        assert_eq!(
            round_trip(&whole_micro_nanos),
            Value::Time(1, TimeUnit::Microsecond)
        );
    }

    #[test]
    fn strings_round_trip_across_the_short_string_threshold() {
        // 63 bytes folds into the type byte; 64 takes the string primitive.
        for length in [0, 1, 63, 64, 1_000] {
            assert_round_trip(&Value::String(SmolStr::new("x".repeat(length))));
        }
        assert_round_trip(&Value::String(SmolStr::new("héllo, wörld: 変種")));
    }

    #[test]
    fn empty_containers_round_trip() {
        assert_round_trip(&Value::from_sequence([]));
        assert_round_trip(&Value::from_mapping([]).unwrap());
    }

    #[test]
    fn nested_containers_round_trip_several_levels_deep() {
        let leaf = Value::from_mapping([
            (Value::from("a"), Value::I64(1)),
            (
                Value::from("b"),
                Value::from_sequence([Value::I8(2), Value::Null]),
            ),
        ])
        .unwrap();
        let value = Value::from_mapping([
            (
                Value::from("outer"),
                Value::from_sequence([
                    leaf.clone(),
                    Value::from_sequence([Value::from_sequence([leaf])]),
                ]),
            ),
            (Value::from("z"), Value::String(SmolStr::new("tail"))),
        ])
        .unwrap();
        assert_round_trip(&value);
    }

    #[test]
    fn an_object_comes_back_in_sorted_field_order() {
        let value = Value::from_mapping([
            (Value::from("c"), Value::I8(3)),
            (Value::from("a"), Value::I8(1)),
            (Value::from("b"), Value::I8(2)),
        ])
        .unwrap();
        let decoded = round_trip(&value);
        assert_eq!(decoded.keys(), ["a", "b", "c"]);
        assert_eq!(decoded.get_key_str("c"), Some(&Value::I8(3)));
    }

    #[test]
    fn a_record_encodes_as_an_object_and_decodes_as_its_mapping() {
        let row = Value::record(
            DataType::from_fields([
                Field::new("name", DataType::Utf8, true),
                Field::new("age", DataType::Int64, true),
            ])
            .unwrap(),
            [Value::from("ada"), Value::I64(36)],
        )
        .unwrap();
        let decoded = round_trip(&row);
        assert_eq!(decoded.keys(), ["age", "name"]);
        assert_eq!(decoded.get_key_str("name"), Some(&Value::from("ada")));
    }

    #[test]
    fn a_large_container_takes_the_four_byte_count_and_wide_ids() {
        let sequence = Value::from_sequence((0..300).map(|index| Value::I64(i64::from(index))));
        assert_round_trip(&sequence);

        let mapping = Value::from_mapping(
            (0..300).map(|index| (Value::from(format!("key{index:03}")), Value::I32(index))),
        )
        .unwrap();
        let decoded = round_trip(&mapping);
        assert_eq!(decoded.len(), 300);
        assert_eq!(decoded.get_key_str("key299"), Some(&Value::I32(299)));
    }

    #[test]
    fn shared_field_names_are_written_to_the_dictionary_once() {
        let entry = |index: i64| {
            Value::from_mapping([(Value::from("repeated"), Value::I64(index))]).unwrap()
        };
        let value = Value::from_sequence((0..10).map(entry));
        let (metadata, _) = encode_value(&value).unwrap();
        // Header, one-name dictionary size, two offsets, then the name once.
        assert_eq!(metadata.len(), 1 + 1 + 2 + "repeated".len());
        assert_round_trip(&value);
    }
}

mod fixtures {
    use super::*;

    #[test]
    fn the_scalar_metadata_is_the_three_byte_empty_dictionary() {
        let (metadata, _) = encode_value(&Value::Null).unwrap();
        assert_eq!(metadata, EMPTY_METADATA);
    }

    #[test]
    fn primitive_headers_spell_the_type_id_above_the_basic_type() {
        // "Value encoding grammar": value_metadata = basic_type | header << 2.
        assert_eq!(encode_value(&Value::Null).unwrap().1, [0x00]);
        assert_eq!(encode_value(&Value::Bool(true)).unwrap().1, [0x04]);
        assert_eq!(encode_value(&Value::Bool(false)).unwrap().1, [0x08]);
        assert_eq!(encode_value(&Value::I8(42)).unwrap().1, [0x0C, 42]);
        assert_eq!(
            encode_value(&Value::Date(17)).unwrap().1,
            [0x2C, 17, 0, 0, 0]
        );
    }

    #[test]
    fn a_short_string_folds_its_length_into_the_header_byte() {
        // "Value Header for Short string": the header is the byte length.
        let (_, bytes) = encode_value(&Value::from("hi")).unwrap();
        assert_eq!(bytes, [0x09, b'h', b'i']);
        assert_eq!(
            decode_value(&EMPTY_METADATA, &bytes).unwrap(),
            Value::from("hi")
        );
    }

    #[test]
    fn a_one_field_object_matches_the_specs_byte_layout() {
        // "Value Data for Object": count, field ids, offsets, then values.
        let value = Value::from_mapping([(Value::from("a"), Value::I8(1))]).unwrap();
        let (metadata, bytes) = encode_value(&value).unwrap();
        assert_eq!(metadata, [0x11, 1, 0, 1, b'a']);
        assert_eq!(bytes, [0x02, 1, 0, 0, 2, 0x0C, 1]);
    }

    #[test]
    fn an_unsorted_dictionary_from_an_older_writer_decodes_by_id() {
        // sorted_strings = 0: readers may assume nothing about order. The
        // dictionary spells ["b", "a"], so name-sorted field ids are [1, 0].
        let metadata = [0x01, 2, 0, 1, 2, b'b', b'a'];
        let bytes = [0x02, 2, 1, 0, 0, 2, 4, 0x0C, 1, 0x0C, 2];
        let decoded = decode_value(&metadata, &bytes).unwrap();
        assert_eq!(decoded.keys(), ["a", "b"]);
        assert_eq!(decoded.get_key_str("a"), Some(&Value::I8(1)));
        assert_eq!(decoded.get_key_str("b"), Some(&Value::I8(2)));
    }

    #[test]
    fn a_two_byte_offset_dictionary_decodes_like_a_one_byte_one() {
        // offset_size_minus_one = 1 in bits 7-6 of the metadata header.
        let metadata = [0x51, 1, 0, 0, 0, 1, 0, b'k'];
        let bytes = [0x02, 1, 0, 0, 1, 0x00];
        let decoded = decode_value(&metadata, &bytes).unwrap();
        assert_eq!(decoded.keys(), ["k"]);
        assert_eq!(decoded.get_key_str("k"), Some(&Value::Null));
    }

    #[test]
    fn an_is_large_array_with_one_element_is_accepted() {
        // "It is valid for an implementation to use a larger value than
        // necessary": is_large = 1 spells the count in four bytes.
        let bytes = [0x13, 1, 0, 0, 0, 0, 2, 0x0C, 7];
        assert_eq!(
            decode_value(&EMPTY_METADATA, &bytes).unwrap(),
            Value::from_sequence([Value::I8(7)])
        );
    }

    #[test]
    fn a_uuid_decodes_as_its_sixteen_big_endian_bytes() {
        // Primitive type 20: 16-byte big-endian; the tree spells it as bytes.
        let mut bytes = vec![0x50];
        bytes.extend(0..16);
        assert_eq!(
            decode_value(&EMPTY_METADATA, &bytes).unwrap(),
            Value::Bytes((0..16).collect())
        );
    }

    #[test]
    fn the_long_string_primitive_decodes_like_a_short_one() {
        // Primitive type 16: four-byte size then UTF-8 bytes; semantically
        // identical to the short-string basic type.
        let bytes = [0x40, 2, 0, 0, 0, b'o', b'k'];
        assert_eq!(
            decode_value(&EMPTY_METADATA, &bytes).unwrap(),
            Value::from("ok")
        );
    }

    #[test]
    fn every_timestamp_primitive_decodes_to_its_unit_and_zoning() {
        // Types 12/13 are microseconds, 18/19 nanoseconds; 12/18 UTC-adjusted.
        let payload = 5_i64.to_le_bytes();
        let spell = |id: u8| {
            let mut bytes = vec![id << 2];
            bytes.extend(payload);
            decode_value(&EMPTY_METADATA, &bytes).unwrap()
        };
        assert_eq!(
            spell(12),
            Value::Timestamp(5, TimeUnit::Microsecond, Timezone::UTC)
        );
        assert_eq!(spell(13), Value::DateTime(5, TimeUnit::Microsecond));
        assert_eq!(spell(17), Value::Time(5, TimeUnit::Microsecond));
        assert_eq!(
            spell(18),
            Value::Timestamp(5, TimeUnit::Nanosecond, Timezone::UTC)
        );
        assert_eq!(spell(19), Value::DateTime(5, TimeUnit::Nanosecond));
    }
}

mod decode_errors {
    use super::*;

    #[test]
    fn empty_metadata_reports_the_missing_header() {
        assert_eq!(
            decode_error(&[], &[0x00]),
            "invalid variant data at byte 0: expected 1 byte of metadata header, got 0"
        );
    }

    #[test]
    fn a_foreign_metadata_version_is_refused_by_number() {
        assert_eq!(
            decode_error(&[0x12, 0, 0], &[0x00]),
            "invalid variant data at byte 0: expected metadata version 1, got 2"
        );
        assert_eq!(
            decode_error(&[0x10, 0, 0], &[0x00]),
            "invalid variant data at byte 0: expected metadata version 1, got 0"
        );
    }

    #[test]
    fn metadata_truncated_at_each_structural_boundary_names_the_position() {
        assert_eq!(
            decode_error(&[0x11], &[0x00]),
            "invalid variant data at byte 1: expected 1 byte of dictionary size, got 0"
        );
        assert_eq!(
            decode_error(&[0x11, 2], &[0x00]),
            "invalid variant data at byte 2: expected 3 bytes of dictionary offsets, got 0"
        );
        assert_eq!(
            decode_error(&[0x11, 1, 0, 2, b'a'], &[0x00]),
            "invalid variant data at byte 3: expected a final dictionary offset of 1, got 2"
        );
    }

    #[test]
    fn decreasing_dictionary_offsets_are_refused_before_slicing() {
        // Offsets [0, 2, 1] over one byte of names: the middle offset
        // overruns and the pair decreases.
        assert_eq!(
            decode_error(&[0x11, 2, 0, 2, 1, b'x'], &[0x00]),
            "invalid variant data at byte 3: expected a dictionary offset of at most 1, got 2"
        );
    }

    #[test]
    fn non_utf8_dictionary_bytes_name_the_offending_byte() {
        assert_eq!(
            decode_error(&[0x11, 1, 0, 1, 0xFF], &[0x00]),
            "invalid variant data at byte 4: \
             expected UTF-8 dictionary string bytes, got an invalid sequence"
        );
    }

    #[test]
    fn value_bytes_truncated_at_each_structural_boundary_name_the_position() {
        // No header byte at all.
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[]),
            "invalid variant data at byte 0: expected 1 byte of value header, got 0"
        );
        // An int32 with two of its four data bytes.
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[0x14, 1, 0]),
            "invalid variant data at byte 1: expected 4 bytes of an int32 value, got 2"
        );
        // A short string shorter than its folded length.
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[0x09, b'h']),
            "invalid variant data at byte 1: expected 2 bytes of short string, got 1"
        );
        // A binary size with no data behind it.
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[0x3C, 9, 0, 0, 0]),
            "invalid variant data at byte 5: expected 9 bytes of binary data, got 0"
        );
        // An object promising two fields into one remaining byte.
        assert_eq!(
            decode_error(&[0x11, 1, 0, 1, b'a'], &[0x02, 2, 0]),
            "invalid variant data at byte 2: expected 7 bytes of object fields, got 1"
        );
        // An array whose offsets are cut off.
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[0x03, 2, 0]),
            "invalid variant data at byte 2: expected 5 bytes of array elements, got 1"
        );
        // An array whose field region is shorter than its final offset.
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[0x03, 1, 0, 9, 0x00]),
            "invalid variant data at byte 4: expected 9 bytes of container field data, got 1"
        );
    }

    #[test]
    fn a_field_id_past_the_dictionary_is_refused_with_both_numbers() {
        assert_eq!(
            decode_error(&[0x11, 1, 0, 1, b'a'], &[0x02, 1, 5, 0, 2, 0x0C, 1]),
            "invalid variant data at byte 2: \
             expected a field id below the dictionary size 1, got 5"
        );
    }

    #[test]
    fn a_field_offset_past_the_region_is_refused_with_both_numbers() {
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[0x03, 1, 9, 2, 0x0C, 7]),
            "invalid variant data at byte 2: expected a field offset of at most 2, got 9"
        );
    }

    #[test]
    fn trailing_bytes_after_the_value_are_refused() {
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[0x00, 0xFF]),
            "invalid variant data at byte 1: expected the end of the buffer, got 1 trailing byte"
        );
    }

    #[test]
    fn an_unknown_primitive_type_id_is_refused_by_number() {
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[21 << 2]),
            "invalid variant data at byte 0: expected a primitive type id between 0 and 20, got 21"
        );
        assert_eq!(
            decode_error(&EMPTY_METADATA, &[63 << 2]),
            "invalid variant data at byte 0: expected a primitive type id between 0 and 20, got 63"
        );
    }

    #[test]
    fn a_decimal_scale_past_the_specification_is_refused() {
        let mut bytes = vec![0x20, 39];
        bytes.extend(1_i32.to_le_bytes());
        assert_eq!(
            decode_error(&EMPTY_METADATA, &bytes),
            "invalid variant data at byte 1: expected a decimal scale between 0 and 38, got 39"
        );
    }

    #[test]
    fn duplicate_object_field_names_are_refused_even_across_distinct_ids() {
        // Two dictionary entries spell the same name; the object uses both.
        let metadata = [0x01, 2, 0, 1, 2, b'a', b'a'];
        let bytes = [0x02, 2, 0, 1, 0, 2, 4, 0x0C, 1, 0x0C, 2];
        assert_eq!(
            decode_error(&metadata, &bytes),
            "invalid variant data at byte 4: expected unique object field names, got \"a\" twice"
        );
    }
}

mod limit_errors {
    use super::*;

    /// Wrap `bytes` in `levels` nested one-element arrays.
    fn nest(mut bytes: Vec<u8>, levels: usize) -> Vec<u8> {
        for _ in 0..levels {
            let length = u8::try_from(bytes.len()).expect("the fixture stays small");
            let mut outer = vec![0x03, 1, 0, length];
            outer.append(&mut bytes);
            bytes = outer;
        }
        bytes
    }

    #[test]
    fn nesting_past_the_depth_limit_is_refused_at_the_container_header() {
        let limits = Limits::new(4, 1 << 20, 1 << 20, 1);
        let shallow = nest(vec![0x00], 4);
        let mut expected = Value::Null;
        for _ in 0..4 {
            expected = Value::from_sequence([expected]);
        }
        assert_eq!(
            decode_value_with_limits(&EMPTY_METADATA, &shallow, limits).unwrap(),
            expected
        );
        // The fifth container header sits past four enclosing arrays.
        let deep = nest(vec![0x00], 5);
        assert_eq!(
            decode_value_with_limits(&EMPTY_METADATA, &deep, limits)
                .unwrap_err()
                .to_string(),
            "invalid variant data at byte 16: expected a value at most 4 levels deep"
        );
    }

    #[test]
    fn a_value_of_too_many_nodes_is_refused() {
        let limits = Limits::new(8, 1 << 20, 3, 1);
        let bytes = [0x03, 3, 0, 1, 2, 3, 0x00, 0x00, 0x00];
        assert_eq!(
            decode_value_with_limits(&EMPTY_METADATA, &bytes, limits)
                .unwrap_err()
                .to_string(),
            "invalid variant data at byte 8: expected a value of at most 3 nodes"
        );
    }

    #[test]
    fn oversized_input_is_refused_before_any_parse() {
        let limits = Limits::new(8, 4, 1 << 20, 1);
        assert_eq!(
            decode_value_with_limits(&EMPTY_METADATA, &[0x00, 0x00], limits)
                .unwrap_err()
                .to_string(),
            "invalid variant data at byte 0: expected at most 4 input bytes, got 5"
        );
    }

    #[test]
    fn encoding_past_the_default_depth_is_refused_with_its_path() {
        let mut value = Value::Null;
        for _ in 0..Limits::default().max_depth() {
            value = Value::from_sequence([value]);
        }
        // Exactly the limit's worth of containers still encodes.
        assert_round_trip(&value);
        let too_deep = Value::from_sequence([value]);
        let message = encode_error(&too_deep);
        assert!(
            message.starts_with("invalid record value at $[0]["),
            "{message}"
        );
        assert!(
            message.ends_with("expected a value at most 128 levels deep"),
            "{message}"
        );
    }
}

mod encode_refusals {
    use super::*;

    #[test]
    fn a_duration_is_refused_by_name() {
        assert_eq!(
            encode_error(&Value::Duration(5, TimeUnit::Second)),
            "invalid record value at $: expected a Variant-encodable value, got a duration of 5"
        );
    }

    #[test]
    fn a_geospatial_value_is_refused_rather_than_respelled_as_bytes() {
        assert_eq!(
            encode_error(&Value::Geospatial([1, 1, 0, 0, 0].as_slice().into())),
            "invalid record value at $: expected a Variant-encodable value, got a geospatial value"
        );
    }

    #[test]
    fn a_zoned_timestamp_outside_utc_is_refused_with_its_zone() {
        let paris = Timezone::from_str("Europe/Paris").unwrap();
        assert_eq!(
            encode_error(&Value::Timestamp(0, TimeUnit::Microsecond, paris)),
            "invalid record value at $: expected a UTC timestamp, got zone \"Europe/Paris\""
        );
    }

    #[test]
    fn integers_past_int64_are_refused_with_the_value() {
        assert_eq!(
            encode_error(&Value::U64(u64::MAX)),
            "invalid record value at $: \
             expected an integer within the Variant int64 range, got 18446744073709551615"
        );
        assert!(
            encode_error(&Value::I128(i128::from(i64::MAX) + 1)).contains("9223372036854775808")
        );
        assert!(
            encode_error(&Value::U128(u128::MAX))
                .contains("got 340282366920938463463374607431768211455")
        );
    }

    #[test]
    fn decimals_outside_the_specification_range_are_refused() {
        assert_eq!(
            encode_error(&Value::Decimal(1, -2)),
            "invalid record value at $: expected a decimal scale between 0 and 38, got -2"
        );
        assert_eq!(
            encode_error(&Value::Decimal(1, 39)),
            "invalid record value at $: expected a decimal scale between 0 and 38, got 39"
        );
        assert_eq!(
            encode_error(&Value::Decimal(super::super::DECIMAL_MAGNITUDE_LIMIT, 0)),
            "invalid record value at $: expected a decimal unscaled value of at most 38 digits, \
             got 100000000000000000000000000000000000000"
        );
    }

    #[test]
    fn temporal_counts_that_cannot_widen_are_refused() {
        assert_eq!(
            encode_error(&Value::Timestamp(i64::MAX, TimeUnit::Second, Timezone::UTC)),
            "invalid record value at $: \
             expected a count representable in microseconds, got 9223372036854775807 s"
        );
        assert_eq!(
            encode_error(&Value::Time(1_500, TimeUnit::Nanosecond)),
            "invalid record value at $: expected a time in whole microseconds, got 1500 nanoseconds"
        );
    }

    #[test]
    fn a_non_string_mapping_key_is_refused_at_its_path() {
        let value = Value::from_mapping([(Value::I64(1), Value::Null)]).unwrap();
        assert_eq!(
            encode_error(&value),
            "invalid record value at $[0].key: expected a string object key, got i64"
        );
    }

    #[test]
    fn a_nested_refusal_reports_the_full_path() {
        let value = Value::from_mapping([(
            Value::from("outer"),
            Value::from_sequence([Value::Duration(1, TimeUnit::Second)]),
        )])
        .unwrap();
        assert_eq!(
            encode_error(&value),
            "invalid record value at $.outer[0]: \
             expected a Variant-encodable value, got a duration of 1"
        );
    }
}
