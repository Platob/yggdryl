//! `rust/src/integer.rs`.

use yggdryl::DataType;
use yggdryl::integer;

use super::typed::assert_typed_marker;

#[test]
fn integer_markers_cover_every_signed_and_unsigned_width() {
    assert_typed_marker::<integer::Int8Type>(DataType::Int8);
    assert_typed_marker::<integer::Int16Type>(DataType::Int16);
    assert_typed_marker::<integer::Int32Type>(DataType::Int32);
    assert_typed_marker::<integer::Int64Type>(DataType::Int64);
    assert_typed_marker::<integer::UInt8Type>(DataType::UInt8);
    assert_typed_marker::<integer::UInt16Type>(DataType::UInt16);
    assert_typed_marker::<integer::UInt32Type>(DataType::UInt32);
    assert_typed_marker::<integer::UInt64Type>(DataType::UInt64);
}

mod reading {
    //! The one integer grammar, read through the value door.

    use yggdryl::{DataType, MarketDataKind, Scalar, Side, State};

    #[test]
    fn the_value_door_reads_an_enum_member_as_its_stored_code() {
        // A member is held and stored as its code, so an integer column of
        // a width that fits it takes the member as that code - what a table
        // storing the column as a plain integer hands back - and one too
        // narrow refuses it as it refuses any integer.
        assert_eq!(
            DataType::Int32.scalar(Scalar::State(State::New)).unwrap(),
            Scalar::from(i32::from(State::New.code()))
        );
        assert_eq!(
            DataType::UInt8.scalar(Scalar::from(Side::Buy)).unwrap(),
            Scalar::from(Side::Buy.code())
        );
        assert_eq!(
            DataType::UInt16
                .scalar(Scalar::from(MarketDataKind::Order))
                .unwrap(),
            Scalar::from(u16::from(MarketDataKind::Order.code()))
        );
        assert_eq!(
            DataType::Int64.scalar(Scalar::State(State::New)).unwrap(),
            Scalar::from(i64::from(State::New.code()))
        );
        assert!(DataType::Int8.scalar(Scalar::State(State::New)).is_err());
    }

    #[test]
    fn the_value_door_reads_a_decimal_with_no_fraction_as_the_integer_it_is() {
        // A table with no unsigned type stores a `uint64` as `decimal(20, 0)`
        // (`into_scheme_compat`), so the door takes a whole decimal back as
        // the number it was - at any scale whose fraction is zero - and
        // refuses a fraction rather than rounding it, a negative one into an
        // unsigned width, and a magnitude the width cannot hold.
        assert_eq!(
            DataType::UInt64
                .scalar(Scalar::decimal128(i128::from(u64::MAX), 0))
                .unwrap(),
            Scalar::from(u64::MAX)
        );
        assert_eq!(
            DataType::Int32
                .scalar(Scalar::decimal128(-4_200, 2))
                .unwrap(),
            Scalar::from(-42_i32)
        );
        assert_eq!(
            DataType::UInt8.scalar(Scalar::decimal128(700, 2)).unwrap(),
            Scalar::from(7_u8)
        );
        assert!(
            DataType::Int32
                .scalar(Scalar::decimal128(4_250, 2))
                .is_err()
        );
        assert!(DataType::UInt64.scalar(Scalar::decimal128(-1, 0)).is_err());
        assert!(
            DataType::UInt64
                .scalar(Scalar::decimal128(i128::from(u64::MAX) + 1, 0))
                .is_err()
        );
        assert!(DataType::Int8.scalar(Scalar::decimal128(128, 0)).is_err());
    }

    #[test]
    fn the_value_door_reads_a_signed_whole_number_and_narrows_it_to_the_width() {
        for (text, expected) in [
            (" 42 ", 42_i64),
            ("+7", 7),
            ("-0", 0),
            ("-9", -9),
            ("007", 7),
        ] {
            assert_eq!(
                DataType::Int64.scalar(Scalar::from(text)).expect(text),
                Scalar::from(expected),
                "{text:?}"
            );
        }
        for text in [
            "1.0", "1e3", "0x10", "1_000", "1 000", " ", "-", "+", "ten", "1,5",
        ] {
            assert!(
                DataType::Int64.scalar(Scalar::from(text)).is_err(),
                "{text:?} read as an integer"
            );
        }
        // The empty text is absence at the door, never a spelling the reader
        // sees; the column's nullability is what may refuse it.
        assert_eq!(
            DataType::Int64.scalar(Scalar::from("")).unwrap(),
            Scalar::Null
        );
        // The width refuses what it cannot hold rather than wrapping it.
        assert!(DataType::UInt32.scalar(Scalar::from("4294967296")).is_err());
        assert!(DataType::UInt32.scalar(Scalar::from("-1")).is_err());
        assert_eq!(
            DataType::UInt32.scalar(Scalar::from("4294967295")).unwrap(),
            Scalar::from(u32::MAX)
        );
        assert_eq!(
            DataType::UInt64
                .scalar(Scalar::from("18446744073709551615"))
                .unwrap(),
            Scalar::from(u64::MAX)
        );
    }
}

#[cfg(feature = "internals")]
mod internal {
    //! The native-width reader and the byte-count table, which no caller
    //! names until a setting reads through them.

    use yggdryl::Scalar;
    #[cfg(feature = "http")]
    use yggdryl::internals::integer::{BYTE_COUNT_SPELLINGS, byte_count_from_text};
    use yggdryl::internals::integer::{
        INTEGER_SPELLINGS, integer_from_scalar_as, integer_from_text_as,
    };

    #[test]
    fn a_scalar_reads_as_the_number_it_is_or_the_digits_it_spells_and_never_wraps() {
        // The number, at every width a document or a column holds it, and
        // the digits of one, read by the one text grammar.
        for value in [
            Scalar::from(3599_u64),
            Scalar::from(3599_i64),
            Scalar::from(3599_u16),
            Scalar::from(3599_i128),
            Scalar::from("3599"),
            Scalar::from(" +3599 "),
        ] {
            assert_eq!(
                integer_from_scalar_as::<u64>(&value),
                Some(3599),
                "{value:?}"
            );
        }
        // Narrowing is exact either way.
        assert_eq!(integer_from_scalar_as::<u64>(&Scalar::from(-1_i64)), None);
        assert_eq!(integer_from_scalar_as::<u64>(&Scalar::from("-1")), None);
        assert_eq!(integer_from_scalar_as::<u8>(&Scalar::from(256_u32)), None);
        assert_eq!(integer_from_scalar_as::<u8>(&Scalar::from("256")), None);
        assert_eq!(
            integer_from_scalar_as::<i8>(&Scalar::from(-128_i64)),
            Some(i8::MIN)
        );
        assert_eq!(
            integer_from_scalar_as::<u64>(&Scalar::from(u128::MAX)),
            None
        );
        assert_eq!(
            integer_from_scalar_as::<u128>(&Scalar::from(u128::MAX)),
            Some(u128::MAX)
        );
        assert_eq!(
            integer_from_scalar_as::<i128>(&Scalar::from(u128::MAX)),
            None
        );
        // Nothing else is an integer: a fraction, a boolean, an absence,
        // words.
        for value in [
            Scalar::from(3599.0_f64),
            Scalar::from(1.5_f64),
            Scalar::from(true),
            Scalar::Null,
            Scalar::from("soon"),
            Scalar::from(""),
            Scalar::from("1.5"),
        ] {
            assert_eq!(integer_from_scalar_as::<u64>(&value), None, "{value:?}");
        }
    }

    #[test]
    fn a_native_width_reads_the_one_grammar_and_never_wraps() {
        assert_eq!(integer_from_text_as::<u32>(" +3 "), Some(3));
        assert_eq!(integer_from_text_as::<u32>("-0"), Some(0));
        assert_eq!(integer_from_text_as::<u32>("4294967295"), Some(u32::MAX));
        assert_eq!(integer_from_text_as::<u32>("4294967296"), None);
        assert_eq!(integer_from_text_as::<u32>("-1"), None);
        assert_eq!(integer_from_text_as::<i8>("-128"), Some(i8::MIN));
        assert_eq!(integer_from_text_as::<i8>("128"), None);
        assert_eq!(
            integer_from_text_as::<u128>("340282366920938463463374607431768211455"),
            Some(u128::MAX)
        );
        assert_eq!(
            integer_from_text_as::<i128>("340282366920938463463374607431768211455"),
            None
        );
        assert_eq!(integer_from_text_as::<usize>("1"), Some(1));
        for text in ["1.5", "1e3", "1_000", "0x10", "many", "", "-", "1 0"] {
            assert_eq!(integer_from_text_as::<u64>(text), None, "{text:?}");
        }
        assert_eq!(INTEGER_SPELLINGS, "a whole number");
    }

    // Every byte count a setting states is the HTTP client's or an object
    // store's, so the table exists under that feature.
    #[cfg(feature = "http")]
    #[test]
    fn a_byte_count_reads_one_unit_table_in_powers_of_1024() {
        for (text, expected) in [
            ("8MiB", 8 << 20),
            ("32 MB", 32 << 20),
            ("1kib", 1 << 10),
            ("2GB", 2 << 30),
            ("512b", 512),
            ("7k", 7 << 10),
            ("3m", 3 << 20),
            ("1G", 1 << 30),
            ("100", 100),
            (" 4 KiB ", 4 << 10),
            ("0gib", 0),
        ] {
            assert_eq!(byte_count_from_text(text), Some(expected), "{text:?}");
        }
        // A count past u64 is a refusal, never a silently unbounded limit.
        assert_eq!(
            byte_count_from_text("17179869183GiB"),
            Some(u64::MAX - (1 << 30) + 1)
        );
        assert_eq!(byte_count_from_text("17179869184GiB"), None);
        for text in [
            "1.5MiB", "1e3", "-1", "lots", "64 pages", "KiB", "", "1 tb", "1 mibs",
        ] {
            assert_eq!(byte_count_from_text(text), None, "{text:?}");
        }
        assert_eq!(
            BYTE_COUNT_SPELLINGS,
            "a byte count, with an optional KiB, MiB or GiB suffix"
        );
    }
}
