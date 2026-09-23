//! `rust/src/datatype_id.rs`: the datatype identifier: its name, its byte,
//! its kind and its ranges.

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
fn every_legacy_name_reads_as_its_identifier_and_is_never_written() {
    let names = [
        ("list", DataTypeId::Serie, "serie"),
        ("list_view", DataTypeId::SerieView, "serie_view"),
        (
            "fixed_size_list",
            DataTypeId::FixedSizeSerie,
            "fixed_size_serie",
        ),
        ("large_list", DataTypeId::LargeSerie, "large_serie"),
        (
            "large_list_view",
            DataTypeId::LargeSerieView,
            "large_serie_view",
        ),
    ];
    assert_eq!(DataTypeId::LEGACY_NAMES.len(), names.len());
    for ((legacy, id), (name, expected, spelled)) in DataTypeId::LEGACY_NAMES.into_iter().zip(names)
    {
        assert_eq!((legacy, id), (name, expected));
        let capitalized = format!("{}{}", legacy[..1].to_uppercase(), &legacy[1..]);
        for text in [legacy.to_owned(), legacy.to_uppercase(), capitalized] {
            assert_eq!(DataTypeId::from_str(&text).unwrap(), id, "{text}");
            assert_eq!(text.parse::<DataTypeId>().unwrap(), id, "{text}");
            assert_eq!(DataTypeId::from_legacy_name(&text), Some(id), "{text}");
            let quoted = format!("\"{text}\"");
            assert_eq!(
                serde_json::from_str::<DataTypeId>(&quoted).unwrap(),
                id,
                "{text}"
            );
        }
        // Read, never written: the identifier spells its own name, and no
        // identifier's name is a legacy one.
        assert_eq!(id.as_str(), spelled);
        assert_eq!(id.to_string(), spelled);
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            format!("\"{spelled}\"")
        );
        assert!(DataTypeId::ALL.iter().all(|other| other.as_str() != legacy));
        assert_eq!(DataTypeId::from_legacy_name(spelled), None);
    }
    assert_eq!(DataTypeId::from_legacy_name("array"), None);
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
    assert_eq!(DataTypeId::ALL.len(), 84);
    for id in [
        DataTypeId::Utf8String,
        DataTypeId::FixedUtf8String,
        DataTypeId::Utf8StringView,
        DataTypeId::LargeUtf8String,
        DataTypeId::LargeUtf8StringView,
        DataTypeId::SizedUtf8String,
        DataTypeId::AsciiString,
        DataTypeId::LargeAsciiString,
        DataTypeId::AsciiStringView,
        DataTypeId::LargeAsciiStringView,
        DataTypeId::FixedAsciiString,
        DataTypeId::SizedAsciiString,
        DataTypeId::Cp1252String,
        DataTypeId::LargeCp1252String,
        DataTypeId::Cp1252StringView,
        DataTypeId::LargeCp1252StringView,
        DataTypeId::FixedCp1252String,
        DataTypeId::SizedCp1252String,
    ] {
        assert_eq!(id.kind(), DataTypeKind::Text);
        assert!(id.is_string());
        // Every string leaf is a parameter of `DataType::String`, so no
        // identifier of the family is a complete datatype on its own.
        assert!(id.is_parameterized());
        // A numbered leaf's width is that parameter, so the identifier
        // names no fixed width.
        assert_eq!(id.fixed_byte_width(), None);
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
    assert_eq!(
        DataTypeId::from_str("UTF8").unwrap(),
        DataTypeId::Utf8String
    );
    assert_eq!(
        DataTypeId::from_str("Fixed_Utf8").unwrap(),
        DataTypeId::FixedUtf8String
    );
    assert_eq!(
        DataTypeId::from_str("Sized_CP1252").unwrap(),
        DataTypeId::SizedCp1252String
    );
    // The identifier is the canonical name alone; the grammar's other
    // spellings of a leaf belong to `DataType`.
    assert!(DataTypeId::from_str("string").is_err());
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
    // The byte `Scalar::write_bytes` and the variant encoding write as a
    // value's tag. Every variant states its number, laid out by family -
    // the family's own number first, then its leaves in the family's range
    // - and this pins every byte so a moved or reused number is a failure
    // rather than a surprise.
    let pinned = [
        (DataTypeId::Null, 0x00),
        (DataTypeId::Boolean, 0x09),
        (DataTypeId::Int8, 0x11),
        (DataTypeId::Int16, 0x12),
        (DataTypeId::Int32, 0x13),
        (DataTypeId::Int64, 0x14),
        (DataTypeId::Int128, 0x15),
        (DataTypeId::UInt8, 0x19),
        (DataTypeId::UInt16, 0x1a),
        (DataTypeId::UInt32, 0x1b),
        (DataTypeId::UInt64, 0x1c),
        (DataTypeId::UInt128, 0x1d),
        (DataTypeId::Float16, 0x21),
        (DataTypeId::Float32, 0x22),
        (DataTypeId::Float64, 0x23),
        (DataTypeId::Decimal32, 0x29),
        (DataTypeId::Decimal64, 0x2a),
        (DataTypeId::Decimal128, 0x2b),
        (DataTypeId::Decimal256, 0x2c),
        (DataTypeId::DateTime64, 0x31),
        (DataTypeId::Date32, 0x32),
        (DataTypeId::Date64, 0x33),
        (DataTypeId::Time32, 0x34),
        (DataTypeId::Time64, 0x35),
        (DataTypeId::Duration32, 0x36),
        (DataTypeId::Duration64, 0x37),
        (DataTypeId::Interval, 0x38),
        (DataTypeId::Binary, 0x41),
        (DataTypeId::LargeBinary, 0x42),
        (DataTypeId::BinaryView, 0x43),
        (DataTypeId::LargeBinaryView, 0x44),
        (DataTypeId::FixedBinary, 0x45),
        (DataTypeId::SizedBinary, 0x46),
        (DataTypeId::Utf8String, 0x51),
        (DataTypeId::LargeUtf8String, 0x52),
        (DataTypeId::Utf8StringView, 0x53),
        (DataTypeId::LargeUtf8StringView, 0x54),
        (DataTypeId::FixedUtf8String, 0x55),
        (DataTypeId::SizedUtf8String, 0x56),
        (DataTypeId::AsciiString, 0x57),
        (DataTypeId::LargeAsciiString, 0x58),
        (DataTypeId::AsciiStringView, 0x59),
        (DataTypeId::LargeAsciiStringView, 0x5a),
        (DataTypeId::FixedAsciiString, 0x5b),
        (DataTypeId::SizedAsciiString, 0x5c),
        (DataTypeId::Cp1252String, 0x5d),
        (DataTypeId::LargeCp1252String, 0x5e),
        (DataTypeId::Cp1252StringView, 0x5f),
        (DataTypeId::LargeCp1252StringView, 0x60),
        (DataTypeId::FixedCp1252String, 0x61),
        (DataTypeId::SizedCp1252String, 0x62),
        (DataTypeId::Version, 0x63),
        (DataTypeId::Url, 0x64),
        (DataTypeId::Urn, 0x65),
        (DataTypeId::Timezone, 0x66),
        (DataTypeId::MimeType, 0x67),
        (DataTypeId::MediaType, 0x68),
        (DataTypeId::Country, 0x71),
        (DataTypeId::Currency, 0x72),
        (DataTypeId::MicCode, 0x73),
        (DataTypeId::CfiCode, 0x74),
        (DataTypeId::Side, 0x75),
        (DataTypeId::State, 0x76),
        (DataTypeId::TimeInForce, 0x77),
        (DataTypeId::IsinCode, 0x78),
        (DataTypeId::CusipCode, 0x79),
        (DataTypeId::SedolCode, 0x7a),
        (DataTypeId::BloombergCode, 0x7b),
        (DataTypeId::FIGICode, 0x7c),
        (DataTypeId::Uuid, 0x81),
        (DataTypeId::Serie, 0x91),
        (DataTypeId::LargeSerie, 0x92),
        (DataTypeId::SerieView, 0x93),
        (DataTypeId::LargeSerieView, 0x94),
        (DataTypeId::FixedSizeSerie, 0x95),
        (DataTypeId::Struct, 0x96),
        (DataTypeId::Map, 0x97),
        (DataTypeId::SortedMap, 0x98),
        (DataTypeId::Union, 0x99),
        (DataTypeId::Dictionary, 0x9a),
        (DataTypeId::RunEndEncoded, 0x9b),
        (DataTypeId::Variant, 0x9c),
        (DataTypeId::Geometry, 0xb1),
        (DataTypeId::Geography, 0xb2),
    ];
    assert_eq!(pinned.len(), DataTypeId::ALL.len());
    for ((id, byte), held) in pinned.into_iter().zip(DataTypeId::ALL) {
        assert_eq!(id, held, "declaration order");
        assert_eq!(id.as_u8(), byte, "{id}");
    }
    assert_eq!(DataTypeId::IsinCode.code_width(), Some(12));
    assert_eq!(DataTypeId::CusipCode.code_width(), Some(9));
    assert_eq!(DataTypeId::SedolCode.code_width(), Some(7));
    assert_eq!(DataTypeId::FIGICode.code_width(), Some(12));
}

#[test]
fn every_leaf_sits_in_its_familys_range_and_no_leaf_takes_the_familys_number() {
    use yggdryl::DataTypeKind;

    for kind in DataTypeKind::ALL {
        // The null family's own number is its one leaf: a null has no leaf
        // to tell from another.
        let placeholder = if kind == DataTypeKind::Null {
            Some(DataTypeId::Null)
        } else {
            None
        };
        assert_eq!(
            DataTypeId::from_u8(kind.id()),
            placeholder,
            "{kind}: a family's own number is a placeholder"
        );
        assert_eq!(DataTypeKind::of_u8(kind.id()), Some(kind), "{kind}");
    }
    for id in DataTypeId::ALL {
        let family = id.kind();
        // The null leaf is the family's own number: a null has no leaf to
        // tell from another.
        if id == DataTypeId::Null {
            assert_eq!(id.as_u8(), family.id());
        } else {
            assert!(id.as_u8() > family.id(), "{id}");
        }
        assert_eq!(DataTypeKind::of_u8(id.as_u8()), Some(family), "{id}");
        assert_eq!(DataTypeId::from_u8(id.as_u8()), Some(id), "{id}");
    }
    // Every family starts after the last one ends.
    let mut starts: Vec<u8> = DataTypeKind::ALL.iter().map(|kind| kind.id()).collect();
    starts.sort_unstable();
    starts.dedup();
    assert_eq!(starts.len(), DataTypeKind::ALL.len());
    // A byte past every family names nothing.
    assert_eq!(DataTypeKind::of_u8(0xff), None);
    assert_eq!(DataTypeId::from_u8(0xff), None);
}

#[test]
fn a_family_is_the_range_of_bytes_it_owns_and_the_ranges_tile_the_identifiers() {
    use yggdryl::DataTypeKind as K;

    // Each family's range, stated here rather than read out of the crate, so
    // a moved bound is a failure: the ranges are what `kind`, `contains` and
    // `of_u8` all answer from.
    let ranges = [
        (K::Null, 0x00, 0x07),
        (K::Boolean, 0x08, 0x0f),
        (K::Integer, 0x10, 0x1f),
        (K::Floating, 0x20, 0x27),
        (K::Decimal, 0x28, 0x2f),
        (K::Temporal, 0x30, 0x3f),
        (K::Bytes, 0x40, 0x4f),
        (K::Text, 0x50, 0x6f),
        (K::Code, 0x70, 0x7f),
        (K::Uuid, 0x80, 0x8f),
        (K::Nested, 0x90, 0xaf),
        (K::Geospatial, 0xb0, 0xbf),
    ];
    assert_eq!(ranges.len(), K::ALL.len());
    for (kind, first, last) in ranges {
        assert_eq!(kind.id(), first, "{kind}");
        assert_eq!(kind.last(), last, "{kind}");
        assert_eq!(kind.range(), first..=last, "{kind}");
    }
    // Every byte up to the last family's end belongs to exactly one family,
    // and none past it.
    for byte in 0_u8..=0xff {
        let owners: Vec<K> = ranges
            .iter()
            .filter(|(_, first, last)| (*first..=*last).contains(&byte))
            .map(|(kind, ..)| *kind)
            .collect();
        assert!(owners.len() <= 1, "{byte:#04x}: {owners:?}");
        assert_eq!(
            DataTypeKind::of_u8(byte),
            owners.first().copied(),
            "{byte:#04x}"
        );
        assert_eq!(owners.is_empty(), byte > 0xbf, "{byte:#04x}");
    }

    // Each identifier's family, listed independently of the byte table, so
    // a bound typo that moved a leaf into its neighbour is caught.
    let members: [(K, &[DataTypeId]); 12] = [
        (K::Null, &[DataTypeId::Null]),
        (K::Boolean, &[DataTypeId::Boolean]),
        (
            K::Integer,
            &[
                DataTypeId::Int8,
                DataTypeId::Int16,
                DataTypeId::Int32,
                DataTypeId::Int64,
                DataTypeId::Int128,
                DataTypeId::UInt8,
                DataTypeId::UInt16,
                DataTypeId::UInt32,
                DataTypeId::UInt64,
                DataTypeId::UInt128,
            ],
        ),
        (
            K::Floating,
            &[
                DataTypeId::Float16,
                DataTypeId::Float32,
                DataTypeId::Float64,
            ],
        ),
        (
            K::Decimal,
            &[
                DataTypeId::Decimal32,
                DataTypeId::Decimal64,
                DataTypeId::Decimal128,
                DataTypeId::Decimal256,
            ],
        ),
        (
            K::Temporal,
            &[
                DataTypeId::DateTime64,
                DataTypeId::Date32,
                DataTypeId::Date64,
                DataTypeId::Time32,
                DataTypeId::Time64,
                DataTypeId::Duration32,
                DataTypeId::Duration64,
                DataTypeId::Interval,
            ],
        ),
        (
            K::Bytes,
            &[
                DataTypeId::Binary,
                DataTypeId::LargeBinary,
                DataTypeId::BinaryView,
                DataTypeId::LargeBinaryView,
                DataTypeId::FixedBinary,
                DataTypeId::SizedBinary,
            ],
        ),
        (
            K::Text,
            &[
                DataTypeId::Utf8String,
                DataTypeId::LargeUtf8String,
                DataTypeId::Utf8StringView,
                DataTypeId::LargeUtf8StringView,
                DataTypeId::FixedUtf8String,
                DataTypeId::SizedUtf8String,
                DataTypeId::AsciiString,
                DataTypeId::LargeAsciiString,
                DataTypeId::AsciiStringView,
                DataTypeId::LargeAsciiStringView,
                DataTypeId::FixedAsciiString,
                DataTypeId::SizedAsciiString,
                DataTypeId::Cp1252String,
                DataTypeId::LargeCp1252String,
                DataTypeId::Cp1252StringView,
                DataTypeId::LargeCp1252StringView,
                DataTypeId::FixedCp1252String,
                DataTypeId::SizedCp1252String,
                DataTypeId::Version,
                DataTypeId::Url,
                DataTypeId::Urn,
                DataTypeId::Timezone,
                DataTypeId::MimeType,
                DataTypeId::MediaType,
            ],
        ),
        (
            K::Code,
            &[
                DataTypeId::Country,
                DataTypeId::Currency,
                DataTypeId::MicCode,
                DataTypeId::CfiCode,
                DataTypeId::Side,
                DataTypeId::State,
                DataTypeId::TimeInForce,
                DataTypeId::IsinCode,
                DataTypeId::CusipCode,
                DataTypeId::SedolCode,
                DataTypeId::BloombergCode,
                DataTypeId::FIGICode,
            ],
        ),
        (K::Uuid, &[DataTypeId::Uuid]),
        (
            K::Nested,
            &[
                DataTypeId::Serie,
                DataTypeId::LargeSerie,
                DataTypeId::SerieView,
                DataTypeId::LargeSerieView,
                DataTypeId::FixedSizeSerie,
                DataTypeId::Struct,
                DataTypeId::Map,
                DataTypeId::SortedMap,
                DataTypeId::Union,
                DataTypeId::Dictionary,
                DataTypeId::RunEndEncoded,
                DataTypeId::Variant,
            ],
        ),
        (
            K::Geospatial,
            &[DataTypeId::Geometry, DataTypeId::Geography],
        ),
    ];
    let listed: usize = members.iter().map(|(_, ids)| ids.len()).sum();
    assert_eq!(
        listed,
        DataTypeId::ALL.len(),
        "every identifier is listed once"
    );
    for (kind, ids) in members {
        for &id in ids {
            assert_eq!(id.kind(), kind, "{id}");
            for other in K::ALL {
                assert_eq!(other.contains(id), other == kind, "{id} in {other}");
            }
        }
    }
}

#[test]
fn a_temporal_identifier_names_its_family_and_no_other_identifier_does() {
    let families = [
        (DataTypeId::Date32, "date"),
        (DataTypeId::Date64, "date"),
        (DataTypeId::Time32, "time"),
        (DataTypeId::Time64, "time"),
        (DataTypeId::DateTime64, "datetime"),
        (DataTypeId::Duration32, "duration"),
        (DataTypeId::Duration64, "duration"),
        (DataTypeId::Interval, "interval"),
    ];
    for id in DataTypeId::ALL {
        let expected = families
            .iter()
            .find(|(temporal, _)| *temporal == id)
            .map(|(_, family)| *family);
        assert_eq!(id.temporal_family(), expected, "{id}");
        assert_eq!(
            expected.is_some(),
            yggdryl::DataTypeKind::Temporal.contains(id),
            "{id}"
        );
    }
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
    assert_eq!(DataTypeId::FixedUtf8String.fixed_byte_width(), None);
    assert_eq!(DataTypeId::Utf8String.fixed_byte_width(), None);
    assert_eq!(DataTypeId::Struct.fixed_byte_width(), None);
}

#[test]
fn a_code_width_is_a_bound_and_never_a_layout() {
    assert_eq!(DataTypeId::Currency.code_width(), Some(3));
    assert_eq!(DataTypeId::CfiCode.code_width(), Some(6));
    assert_eq!(DataTypeId::Currency.fixed_byte_width(), None);
    assert_eq!(DataTypeId::CfiCode.fixed_byte_width(), None);
    // Only a code has one: a width that is a layout is not this fact.
    assert_eq!(DataTypeId::Uuid.code_width(), None);
    assert_eq!(DataTypeId::FixedUtf8String.code_width(), None);
    assert_eq!(DataTypeId::Int32.code_width(), None);
}
