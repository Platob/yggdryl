//! The datatype identifier: its name, its byte, its kind and its ranges.

use yggdryl::{DataType, DataTypeId, DataTypeKind};

#[test]
fn names_round_trip_case_insensitively() {
    for id in DataTypeId::ALL {
        assert_eq!(DataTypeId::from_str(id.as_str()).unwrap(), id);
        assert_eq!(
            DataTypeId::from_str(&id.as_str().to_uppercase()).unwrap(),
            id
        );
    }
}

#[test]
fn names_are_unique() {
    let mut names: Vec<_> = DataTypeId::ALL.iter().map(|id| id.as_str()).collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(names.len(), total);
}

#[test]
fn every_kind_is_reachable() {
    for kind in DataTypeKind::ALL {
        assert!(
            DataTypeId::ALL.iter().any(|id| id.kind() == kind),
            "no identifier maps to {kind}"
        );
    }
}

#[test]
fn the_strings_and_the_codes_are_text() {
    assert_eq!(DataTypeId::ALL.len(), 68);
    for id in [
        DataTypeId::String,
        DataTypeId::FixedString,
        DataTypeId::StringView,
        DataTypeId::LargeString,
        DataTypeId::LargeStringView,
    ] {
        assert_eq!(id.kind(), DataTypeKind::Text);
        assert!(id.is_string());
        // Every string carries a charset and a bound, so none of the
        // layouts is a complete datatype on its own.
        assert!(id.is_parameterized());
    }
    for (_, dtype, width) in DataType::CODES {
        let id = dtype.id();
        assert_eq!(id.kind(), DataTypeKind::Code);
        assert!(id.is_string());
        assert!(!id.is_parameterized());
        // A code's width bounds its values; it is not a layout, so the
        // identifier names no fixed width.
        assert_eq!(id.code_width(), Some(*width));
        assert_eq!(id.fixed_byte_width(), None);
    }
    assert_eq!(DataTypeId::from_str("STRING").unwrap(), DataTypeId::String);
    assert_eq!(
        DataTypeId::from_str("Fixed_String").unwrap(),
        DataTypeId::FixedString
    );
    assert_eq!(
        DataTypeId::from_str("Currency").unwrap(),
        DataTypeId::Currency
    );
}

#[test]
fn the_two_arrow_less_integer_widths_are_named_here_and_nowhere_in_datatype() {
    // Arrow has no 128-bit integer layout, so these two identifiers name a
    // width `Scalar` stores and `DataType` cannot. Every integer predicate
    // still has to place them, and `DataType::id` still has to be able to
    // produce every *other* identifier.
    for id in [DataTypeId::Int128, DataTypeId::UInt128] {
        assert!(id.is_integer());
        assert_eq!(id.kind(), DataTypeKind::Integer);
        assert_eq!(id.fixed_byte_width(), Some(16));
        assert!(!id.is_parameterized());
    }
    assert!(DataTypeId::Int128.is_signed_integer());
    assert!(DataTypeId::UInt128.is_unsigned_integer());
    assert_eq!(DataTypeId::from_str("int128").unwrap(), DataTypeId::Int128);
    assert_eq!(
        DataTypeId::from_str("UINT128").unwrap(),
        DataTypeId::UInt128
    );
    // The datatype grammar does not accept them, because no Arrow layout
    // holds one.
    assert!(DataType::from_str("int128").is_err());
}

#[test]
fn every_discriminant_is_stated_and_pinned() {
    // The byte `Scalar::write_bytes` writes as a value's tag. Every
    // variant states its number, a retired one leaves its number unused
    // (58 was `msgdirection`, since retired), and this pins every byte so a
    // moved or reused number is a failure rather than a surprise.
    let pinned = [
        (DataTypeId::Null, 0),
        (DataTypeId::Boolean, 1),
        (DataTypeId::Int8, 2),
        (DataTypeId::Int16, 3),
        (DataTypeId::Int32, 4),
        (DataTypeId::Int64, 5),
        (DataTypeId::UInt8, 6),
        (DataTypeId::UInt16, 7),
        (DataTypeId::UInt32, 8),
        (DataTypeId::UInt64, 9),
        (DataTypeId::Int128, 10),
        (DataTypeId::UInt128, 11),
        (DataTypeId::Float16, 12),
        (DataTypeId::Float32, 13),
        (DataTypeId::Float64, 14),
        (DataTypeId::DateTime64, 15),
        (DataTypeId::Date32, 16),
        (DataTypeId::Date64, 17),
        (DataTypeId::Time32, 18),
        (DataTypeId::Time64, 19),
        (DataTypeId::Duration32, 20),
        (DataTypeId::Duration64, 21),
        (DataTypeId::Interval, 22),
        (DataTypeId::Binary, 23),
        (DataTypeId::FixedSizeBinary, 24),
        (DataTypeId::LargeBinary, 25),
        (DataTypeId::BinaryView, 26),
        (DataTypeId::String, 27),
        (DataTypeId::FixedString, 28),
        (DataTypeId::StringView, 29),
        (DataTypeId::LargeString, 30),
        (DataTypeId::LargeStringView, 31),
        (DataTypeId::Country, 32),
        (DataTypeId::Currency, 33),
        (DataTypeId::Mic, 34),
        (DataTypeId::Cfi, 35),
        (DataTypeId::Uuid, 36),
        (DataTypeId::List, 37),
        (DataTypeId::ListView, 38),
        (DataTypeId::FixedSizeList, 39),
        (DataTypeId::LargeList, 40),
        (DataTypeId::LargeListView, 41),
        (DataTypeId::Struct, 42),
        (DataTypeId::Union, 43),
        (DataTypeId::Dictionary, 44),
        (DataTypeId::Decimal32, 45),
        (DataTypeId::Decimal64, 46),
        (DataTypeId::Decimal128, 47),
        (DataTypeId::Decimal256, 48),
        (DataTypeId::Map, 49),
        (DataTypeId::RunEndEncoded, 50),
        (DataTypeId::Variant, 51),
        (DataTypeId::Geometry, 52),
        (DataTypeId::Geography, 53),
        (DataTypeId::Version, 54),
        (DataTypeId::Side, 55),
        (DataTypeId::State, 56),
        (DataTypeId::TimeInForce, 57),
        (DataTypeId::Url, 59),
        (DataTypeId::Isin, 60),
        (DataTypeId::Timezone, 61),
        (DataTypeId::MimeType, 62),
        (DataTypeId::MediaType, 63),
        (DataTypeId::Cusip, 64),
        (DataTypeId::Sedol, 65),
        (DataTypeId::Bloomberg, 66),
        (DataTypeId::SortedMap, 67),
        (DataTypeId::Struct2, 68),
    ];
    assert_eq!(pinned.len(), DataTypeId::ALL.len());
    for ((id, byte), held) in pinned.into_iter().zip(DataTypeId::ALL) {
        assert_eq!(id, held, "declaration order");
        assert_eq!(id.as_u8(), byte, "{id}");
    }
    assert!(
        DataTypeId::ALL.iter().all(|id| id.as_u8() != 58),
        "58 is retired and never reused"
    );
    assert_eq!(DataTypeId::Isin.code_width(), Some(12));
    assert_eq!(DataTypeId::Cusip.code_width(), Some(9));
    assert_eq!(DataTypeId::Sedol.code_width(), Some(7));
}

#[test]
fn unknown_name_reports_the_input() {
    let error = DataTypeId::from_str("int33").unwrap_err();
    assert!(error.to_string().contains("\"int33\""), "{error}");
}

#[test]
fn integer_predicates_partition_the_family() {
    for id in DataTypeId::ALL.into_iter().filter(|id| id.is_integer()) {
        assert_ne!(id.is_signed_integer(), id.is_unsigned_integer());
    }
}

#[test]
fn fixed_widths_match_their_layout() {
    assert_eq!(DataTypeId::Int32.fixed_byte_width(), Some(4));
    assert_eq!(DataTypeId::Decimal256.fixed_byte_width(), Some(32));
    assert_eq!(DataTypeId::Uuid.fixed_byte_width(), Some(16));
    // A fixed string's width is a parameter, so the identifier alone has
    // none: `DataType::fixed_byte_width` is what answers for one value.
    assert_eq!(DataTypeId::FixedString.fixed_byte_width(), None);
    assert_eq!(DataTypeId::String.fixed_byte_width(), None);
    assert_eq!(DataTypeId::Struct.fixed_byte_width(), None);
}

#[test]
fn a_code_width_is_a_bound_and_never_a_layout() {
    assert_eq!(DataTypeId::Currency.code_width(), Some(3));
    assert_eq!(DataTypeId::Cfi.code_width(), Some(6));
    assert_eq!(DataTypeId::Currency.fixed_byte_width(), None);
    assert_eq!(DataTypeId::Cfi.fixed_byte_width(), None);
    // Only a code has one: a width that is a layout is not this fact.
    assert_eq!(DataTypeId::Uuid.code_width(), None);
    assert_eq!(DataTypeId::FixedString.code_width(), None);
    assert_eq!(DataTypeId::Int32.code_width(), None);
}
