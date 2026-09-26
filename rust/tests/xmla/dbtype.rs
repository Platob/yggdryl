//! `rust/src/xmla/dbtype.rs`: OLE DB's `DBTYPE_*` indicators - their numbers
//! and names as `oledb.h` spells them, the one mapping from every `DataType`
//! family onto them, and the `COLUMN_SIZE`, `UNSIGNED_ATTRIBUTE` and
//! `FIXED_PREC_SCALE` answers `DBSCHEMA_PROVIDER_TYPES` states for each.

use std::collections::{BTreeSet, HashSet};

use yggdryl::xmla::DbType;
use yggdryl::{DataType, Field, StructType, TimeUnit, Timezone, UnionMode};

/// Every indicator with the number and the name `oledb.h` gives it, in the
/// numeric order `DbType::ALL` promises.
const OLE_DB: &[(DbType, u16, &str)] = &[
    (DbType::Empty, 0, "DBTYPE_EMPTY"),
    (DbType::I2, 2, "DBTYPE_I2"),
    (DbType::I4, 3, "DBTYPE_I4"),
    (DbType::R4, 4, "DBTYPE_R4"),
    (DbType::R8, 5, "DBTYPE_R8"),
    (DbType::Bool, 11, "DBTYPE_BOOL"),
    (DbType::Variant, 12, "DBTYPE_VARIANT"),
    (DbType::I1, 16, "DBTYPE_I1"),
    (DbType::Ui1, 17, "DBTYPE_UI1"),
    (DbType::Ui2, 18, "DBTYPE_UI2"),
    (DbType::Ui4, 19, "DBTYPE_UI4"),
    (DbType::I8, 20, "DBTYPE_I8"),
    (DbType::Ui8, 21, "DBTYPE_UI8"),
    (DbType::Guid, 72, "DBTYPE_GUID"),
    (DbType::Bytes, 128, "DBTYPE_BYTES"),
    (DbType::Wstr, 130, "DBTYPE_WSTR"),
    (DbType::Numeric, 131, "DBTYPE_NUMERIC"),
    (DbType::DbDate, 133, "DBTYPE_DBDATE"),
    (DbType::DbTime, 134, "DBTYPE_DBTIME"),
    (DbType::DbTimestamp, 135, "DBTYPE_DBTIMESTAMP"),
    (DbType::HChapter, 136, "DBTYPE_HCHAPTER"),
];

/// A nullable `item` field of `dtype`, the child every nested datatype here
/// is built over.
fn item(dtype: DataType) -> Field {
    Field::new("item", dtype, true)
}

/// A record of a string and an integer column.
fn record() -> DataType {
    StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Int64.nullable_field("size"),
    ])
    .map(DataType::from)
    .expect("a valid struct")
}

/// Asserts that `DbType::of` states every datatype in `cases` as the
/// indicator beside it, naming the datatype that does not.
fn assert_maps(cases: &[(DataType, DbType)]) {
    for (dtype, expected) in cases {
        assert_eq!(DbType::of(dtype), *expected, "DbType::of({dtype})");
    }
}

#[test]
fn a_code_no_indicator_carries_reads_as_none() {
    // DBTYPE_NULL, DBTYPE_CY, DBTYPE_DATE, DBTYPE_BSTR, DBTYPE_IDISPATCH,
    // DBTYPE_ERROR, DBTYPE_IUNKNOWN, DBTYPE_DECIMAL, DBTYPE_FILETIME,
    // DBTYPE_STR, DBTYPE_UDT, DBTYPE_PROPVARIANT, DBTYPE_VARNUMERIC,
    // DBTYPE_VECTOR, DBTYPE_ARRAY, DBTYPE_BYREF and a number nothing names.
    for code in [
        1,
        6,
        7,
        8,
        9,
        10,
        13,
        14,
        64,
        129,
        132,
        137,
        138,
        139,
        0x1000,
        0x2000,
        0x4000,
        u16::MAX,
    ] {
        assert_eq!(DbType::from_code(code), None, "DbType::from_code({code})");
    }
}

#[test]
fn from_code_answers_exactly_the_codes_all_lists() {
    let listed = DbType::ALL
        .iter()
        .map(|indicator| indicator.code())
        .collect::<BTreeSet<_>>();
    for code in 0..=u16::MAX {
        let found = DbType::from_code(code);
        assert_eq!(
            found.is_some(),
            listed.contains(&code),
            "DbType::from_code({code}) = {found:?}"
        );
        if let Some(indicator) = found {
            assert_eq!(indicator.code(), code);
        }
    }
}

#[test]
fn every_indicator_round_trips_through_its_code() {
    for indicator in DbType::ALL {
        assert_eq!(
            DbType::from_code(indicator.code()),
            Some(*indicator),
            "{indicator}"
        );
    }
}

#[test]
fn every_indicator_carries_the_number_and_name_ole_db_gives_it() {
    assert_eq!(
        DbType::ALL,
        OLE_DB
            .iter()
            .map(|(indicator, _, _)| *indicator)
            .collect::<Vec<_>>()
            .as_slice()
    );
    for (indicator, code, name) in OLE_DB {
        assert_eq!(indicator.code(), *code, "{name}");
        assert_eq!(indicator.as_str(), *name, "{name}");
    }
}

#[test]
fn all_lists_each_indicator_once_in_numeric_order() {
    let codes = DbType::ALL
        .iter()
        .map(|indicator| indicator.code())
        .collect::<Vec<_>>();
    assert!(
        codes.windows(2).all(|pair| pair[0] < pair[1]),
        "strictly ascending: {codes:?}"
    );
    // The derived order is the numeric one, so a sorted set of indicators
    // lists them as `ALL` does.
    assert!(DbType::ALL.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        DbType::ALL.iter().copied().collect::<HashSet<_>>().len(),
        DbType::ALL.len()
    );
}

#[test]
fn every_name_is_a_distinct_dbtype_spelling() {
    let names = DbType::ALL
        .iter()
        .map(|indicator| indicator.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(names.len(), DbType::ALL.len());
    for name in names {
        let suffix = name
            .strip_prefix("DBTYPE_")
            .unwrap_or_else(|| panic!("{name} names no DBTYPE"));
        assert!(!suffix.is_empty(), "{name}");
        assert!(
            suffix
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()),
            "{name} is upper-case ASCII"
        );
    }
}

#[test]
fn display_writes_the_dbtype_name() {
    for indicator in DbType::ALL {
        assert_eq!(indicator.to_string(), indicator.as_str());
        assert_eq!(format!("{indicator}"), indicator.as_str());
    }
    assert_eq!(DbType::HChapter.to_string(), "DBTYPE_HCHAPTER");
}

#[test]
fn every_indicator_all_lists_is_what_some_datatype_maps_to() {
    let corpus = [
        DataType::Null,
        DataType::Int16,
        DataType::Int32,
        DataType::Float32,
        DataType::Float64,
        DataType::Boolean,
        DataType::Variant,
        DataType::Int8,
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::Int64,
        DataType::UInt64,
        DataType::Uuid,
        DataType::binary(),
        DataType::utf8(),
        DataType::decimal128(38, 10).expect("a decimal"),
        DataType::Date32,
        DataType::time64(TimeUnit::Microsecond).expect("a time"),
        DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).expect("a datetime"),
        record(),
    ];
    let reached = corpus.iter().map(DbType::of).collect::<BTreeSet<_>>();
    let listed = DbType::ALL.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(reached, listed);
}

#[test]
fn null_is_empty_and_boolean_is_bool() {
    assert_maps(&[
        (DataType::Null, DbType::Empty),
        (DataType::Boolean, DbType::Bool),
    ]);
}

#[test]
fn each_integer_width_keeps_its_width_and_its_sign() {
    assert_maps(&[
        (DataType::Int8, DbType::I1),
        (DataType::Int16, DbType::I2),
        (DataType::Int32, DbType::I4),
        (DataType::Int64, DbType::I8),
        (DataType::UInt8, DbType::Ui1),
        (DataType::UInt16, DbType::Ui2),
        (DataType::UInt32, DbType::Ui4),
        (DataType::UInt64, DbType::Ui8),
    ]);
}

#[test]
fn a_half_or_single_float_is_r4_and_a_double_is_r8() {
    assert_maps(&[
        (DataType::Float16, DbType::R4),
        (DataType::Float32, DbType::R4),
        (DataType::Float64, DbType::R8),
    ]);
}

#[test]
fn every_decimal_width_is_numeric() {
    assert_maps(&[
        (
            DataType::decimal32(9, 2).expect("decimal32"),
            DbType::Numeric,
        ),
        (
            DataType::decimal64(18, 8).expect("decimal64"),
            DbType::Numeric,
        ),
        (
            DataType::decimal128(38, 10).expect("decimal128"),
            DbType::Numeric,
        ),
        (
            DataType::decimal256(76, 0).expect("decimal256"),
            DbType::Numeric,
        ),
        (
            DataType::decimal(5, -2).expect("a negative scale"),
            DbType::Numeric,
        ),
        (
            DataType::decimal(1, 0).expect("the narrowest"),
            DbType::Numeric,
        ),
    ]);
}

#[test]
fn every_string_leaf_in_every_charset_is_wstr() {
    assert_maps(&[
        (DataType::utf8(), DbType::Wstr),
        (DataType::large_utf8(), DbType::Wstr),
        (DataType::utf8_view(), DbType::Wstr),
        (DataType::large_utf8_view(), DbType::Wstr),
        (DataType::fixed_utf8(8).expect("fixed_utf8"), DbType::Wstr),
        (DataType::sized_utf8(32).expect("sized_utf8"), DbType::Wstr),
        (DataType::ascii(), DbType::Wstr),
        (DataType::large_ascii(), DbType::Wstr),
        (DataType::ascii_view(), DbType::Wstr),
        (DataType::large_ascii_view(), DbType::Wstr),
        (DataType::fixed_ascii(4).expect("fixed_ascii"), DbType::Wstr),
        (DataType::sized_ascii(4).expect("sized_ascii"), DbType::Wstr),
        (DataType::cp1252(), DbType::Wstr),
        (DataType::large_cp1252(), DbType::Wstr),
        (DataType::cp1252_view(), DbType::Wstr),
        (DataType::large_cp1252_view(), DbType::Wstr),
        (
            DataType::fixed_cp1252(8).expect("fixed_cp1252"),
            DbType::Wstr,
        ),
        (
            DataType::sized_cp1252(32).expect("sized_cp1252"),
            DbType::Wstr,
        ),
    ]);
}

#[test]
fn a_string_leaf_read_from_its_spelling_is_wstr() {
    for spelling in [
        "string",
        "varchar",
        "text",
        "char(3)",
        "large_string",
        "string_view",
        "string(windows-1252,32)",
        "utf8(32)",
    ] {
        let dtype = spelling
            .parse::<DataType>()
            .unwrap_or_else(|error| panic!("{spelling}: {error}"));
        assert_eq!(DbType::of(&dtype), DbType::Wstr, "{spelling} as {dtype}");
    }
}

#[test]
fn every_byte_leaf_is_bytes() {
    assert_maps(&[
        (DataType::binary(), DbType::Bytes),
        (DataType::large_binary(), DbType::Bytes),
        (DataType::binary_view(), DbType::Bytes),
        (DataType::large_binary_view(), DbType::Bytes),
        (
            DataType::fixed_binary(16).expect("fixed_binary"),
            DbType::Bytes,
        ),
        (
            DataType::sized_binary(16).expect("sized_binary"),
            DbType::Bytes,
        ),
    ]);
}

#[test]
fn a_uuid_is_guid_while_sixteen_plain_bytes_stay_bytes() {
    assert_maps(&[
        (DataType::Uuid, DbType::Guid),
        (
            DataType::fixed_binary(16).expect("fixed_binary"),
            DbType::Bytes,
        ),
    ]);
}

#[test]
fn a_geospatial_value_is_its_well_known_binary_bytes() {
    assert_maps(&[
        (DataType::geometry(None).expect("a geometry"), DbType::Bytes),
        (
            DataType::geometry(Some("EPSG:4326")).expect("a geometry with a CRS"),
            DbType::Bytes,
        ),
        (
            DataType::geography(None, None).expect("a geography"),
            DbType::Bytes,
        ),
    ]);
}

#[test]
fn a_date_of_either_width_is_dbdate() {
    assert_maps(&[
        (DataType::Date32, DbType::DbDate),
        (DataType::Date64, DbType::DbDate),
    ]);
}

#[test]
fn a_time_of_every_unit_and_width_is_dbtime() {
    assert_maps(&[
        (
            DataType::time32(TimeUnit::Second).expect("time32(s)"),
            DbType::DbTime,
        ),
        (
            DataType::time32(TimeUnit::Millisecond).expect("time32(ms)"),
            DbType::DbTime,
        ),
        (
            DataType::time64(TimeUnit::Microsecond).expect("time64(us)"),
            DbType::DbTime,
        ),
        (
            DataType::time64(TimeUnit::Nanosecond).expect("time64(ns)"),
            DbType::DbTime,
        ),
    ]);
}

#[test]
fn a_datetime_naive_or_zoned_is_dbtimestamp() {
    let zoned = Timezone::from_str("America/New_York").expect("a registered zone");
    assert_maps(&[
        (
            DataType::datetime64(TimeUnit::Second, Timezone::NAIVE).expect("naive"),
            DbType::DbTimestamp,
        ),
        (
            DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).expect("UTC"),
            DbType::DbTimestamp,
        ),
        (
            DataType::datetime64(TimeUnit::Nanosecond, zoned).expect("a named zone"),
            DbType::DbTimestamp,
        ),
    ]);
}

#[test]
fn a_duration_or_an_interval_has_no_indicator_and_is_spelled_as_wstr() {
    assert_maps(&[
        (
            DataType::duration32(TimeUnit::Second).expect("duration32(s)"),
            DbType::Wstr,
        ),
        (
            DataType::duration64(TimeUnit::Nanosecond).expect("duration64(ns)"),
            DbType::Wstr,
        ),
        (
            DataType::interval(TimeUnit::YearMonth).expect("interval(year_month)"),
            DbType::Wstr,
        ),
        (
            DataType::interval(TimeUnit::DayTime).expect("interval(day_time)"),
            DbType::Wstr,
        ),
        (
            DataType::interval(TimeUnit::MonthDayNano).expect("interval(month_day_nano)"),
            DbType::Wstr,
        ),
    ]);
}

#[test]
fn every_registered_code_is_wstr() {
    assert_maps(&[
        (DataType::Country, DbType::Wstr),
        (DataType::Ccy, DbType::Wstr),
        (DataType::MicCode, DbType::Wstr),
        (DataType::CfiCode, DbType::Wstr),
        (DataType::IsinCode, DbType::Wstr),
        (DataType::CusipCode, DbType::Wstr),
        (DataType::SedolCode, DbType::Wstr),
        (DataType::BloombergCode, DbType::Wstr),
        (DataType::FIGICode, DbType::Wstr),
        (DataType::Side, DbType::Wstr),
        (DataType::State, DbType::Wstr),
        (DataType::TimeInForce, DbType::Wstr),
        (DataType::Unit, DbType::Wstr),
    ]);
}

#[test]
fn every_spelled_identifier_is_wstr() {
    assert_maps(&[
        (DataType::Version, DbType::Wstr),
        (DataType::Url, DbType::Wstr),
        (DataType::Urn, DbType::Wstr),
        (DataType::Timezone, DbType::Wstr),
        (DataType::MimeType, DbType::Wstr),
        (DataType::MediaType, DbType::Wstr),
    ]);
}

#[test]
fn a_struct_is_a_chapter_whatever_its_children() {
    let empty = StructType::from_fields(Vec::<Field>::new())
        .map(DataType::from)
        .expect("an empty struct");
    let nested = StructType::from_fields([record().required_field("inner")])
        .map(DataType::from)
        .expect("a nested struct");
    assert_maps(&[
        (record(), DbType::HChapter),
        (empty, DbType::HChapter),
        (nested, DbType::HChapter),
    ]);
}

#[test]
fn every_serie_layout_is_a_chapter_not_its_item() {
    assert_maps(&[
        (DataType::serie(item(DataType::Int32)), DbType::HChapter),
        (
            DataType::serie_view(item(DataType::Int32)),
            DbType::HChapter,
        ),
        (
            DataType::fixed_size_serie(item(DataType::Int32), 3).expect("fixed_size_serie"),
            DbType::HChapter,
        ),
        (
            DataType::large_serie(item(DataType::utf8())),
            DbType::HChapter,
        ),
        (
            DataType::large_serie_view(item(DataType::utf8())),
            DbType::HChapter,
        ),
        (DataType::serie(item(record())), DbType::HChapter),
    ]);
}

#[test]
fn a_map_sorted_or_not_is_a_chapter() {
    let map = DataType::map_of(DataType::utf8(), DataType::Int64, false).expect("a map");
    let sorted = DataType::map_of(DataType::utf8(), DataType::Int64, true).expect("a sorted map");
    assert!(matches!(map, DataType::Map(_)), "{map}");
    assert!(matches!(sorted, DataType::SortedMap(_)), "{sorted}");
    assert_maps(&[(map, DbType::HChapter), (sorted, DbType::HChapter)]);
}

#[test]
fn a_union_of_either_mode_or_a_variant_is_variant() {
    let members = || {
        [
            (0_i8, item(DataType::Int32)),
            (1_i8, Field::new("text", DataType::utf8(), true)),
        ]
    };
    assert_maps(&[
        (
            DataType::union(members(), UnionMode::Dense).expect("a dense union"),
            DbType::Variant,
        ),
        (
            DataType::union(members(), UnionMode::Sparse).expect("a sparse union"),
            DbType::Variant,
        ),
        (
            DataType::dense_union([item(DataType::Float64)]).expect("a dense union"),
            DbType::Variant,
        ),
        (DataType::Variant, DbType::Variant),
    ]);
}

#[test]
fn a_dictionary_is_stated_as_its_values_never_its_keys() {
    assert_maps(&[
        (
            DataType::dictionary(DataType::Int32, DataType::utf8()).expect("a dictionary"),
            DbType::Wstr,
        ),
        (
            DataType::dictionary(DataType::UInt8, DataType::Int64).expect("a dictionary"),
            DbType::I8,
        ),
        (
            DataType::dictionary(DataType::Int16, DataType::Date32).expect("a dictionary"),
            DbType::DbDate,
        ),
        (
            DataType::dictionary(DataType::Int8, DataType::binary()).expect("a dictionary"),
            DbType::Bytes,
        ),
    ]);
}

#[test]
fn a_run_end_encoded_column_is_stated_as_its_values_never_its_run_ends() {
    let encoded = |values: DataType| {
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", values, true),
        )
        .expect("a run-end encoded datatype")
    };
    assert_maps(&[
        (encoded(DataType::Float64), DbType::R8),
        (encoded(DataType::utf8()), DbType::Wstr),
        (encoded(DataType::UInt16), DbType::Ui2),
        (encoded(DataType::Uuid), DbType::Guid),
    ]);
}

#[test]
fn an_encoding_over_an_encoding_reaches_the_innermost_values() {
    let dictionary =
        DataType::dictionary(DataType::Int32, DataType::Float32).expect("a dictionary");
    let encoded = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int64, false),
        Field::new("values", dictionary, true),
    )
    .expect("a run-end encoded dictionary");
    assert_eq!(DbType::of(&encoded), DbType::R4);
    let series = DataType::dictionary(DataType::Int8, DataType::serie(item(DataType::Int32)))
        .expect("a dictionary of series");
    assert_eq!(DbType::of(&series), DbType::HChapter);
}

#[test]
fn column_size_states_each_fixed_indicator_and_leaves_the_rest_to_the_column() {
    for (indicator, expected) in [
        (DbType::Empty, Some(0)),
        (DbType::Bool, Some(1)),
        (DbType::Guid, Some(16)),
        // DBTYPE_NUMERIC: the maximum precision OLE DB's table of numeric
        // precisions gives it.
        (DbType::Numeric, Some(38)),
        // A date, a time and a timestamp: the length of the string
        // representation at nine fractional digits, `yyyy-mm-dd`,
        // `hh:mm:ss.fffffffff` and `yyyy-mm-dd hh:mm:ss.fffffffff`.
        (DbType::DbDate, Some(10)),
        (DbType::DbTime, Some(18)),
        (DbType::DbTimestamp, Some(29)),
        (DbType::Bytes, None),
        (DbType::Wstr, None),
        (DbType::Variant, None),
        (DbType::HChapter, None),
    ] {
        assert_eq!(indicator.column_size(), expected, "{indicator}");
    }
    assert_eq!("yyyy-mm-dd".len(), 10);
    assert_eq!("hh:mm:ss.fffffffff".len(), 18);
    assert_eq!("yyyy-mm-dd hh:mm:ss.fffffffff".len(), 29);
}

#[test]
fn column_size_of_an_integer_is_its_maximum_decimal_precision() {
    // DBSCHEMA_PROVIDER_TYPES.COLUMN_SIZE: "If the data type is numeric, this
    // is the upper bound on the maximum precision of the data type" - OLE
    // DB's table of numeric precisions: the digits of the widest value, as
    // DBTYPE_NUMERIC's 38 already is.
    for (indicator, digits) in [
        (DbType::I1, i128::from(i8::MIN)),
        (DbType::Ui1, i128::from(u8::MAX)),
        (DbType::I2, i128::from(i16::MIN)),
        (DbType::Ui2, i128::from(u16::MAX)),
        (DbType::I4, i128::from(i32::MIN)),
        (DbType::Ui4, i128::from(u32::MAX)),
        (DbType::I8, i128::from(i64::MIN)),
        (DbType::Ui8, i128::from(u64::MAX)),
    ]
    .map(|(indicator, widest)| (indicator, widest.unsigned_abs().to_string().len()))
    {
        assert_eq!(
            indicator.column_size(),
            Some(u32::try_from(digits).expect("a digit count")),
            "{indicator}"
        );
    }
}

#[test]
fn a_fixed_column_size_is_stated_for_every_indicator_but_the_open_four() {
    let open = DbType::ALL
        .iter()
        .copied()
        .filter(|indicator| indicator.column_size().is_none())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        open,
        BTreeSet::from([
            DbType::Variant,
            DbType::Bytes,
            DbType::Wstr,
            DbType::HChapter
        ])
    );
}

#[test]
fn is_unsigned_answers_for_numbers_alone() {
    for (indicator, expected) in [
        (DbType::Empty, None),
        (DbType::I2, Some(false)),
        (DbType::I4, Some(false)),
        (DbType::R4, Some(false)),
        (DbType::R8, Some(false)),
        (DbType::Bool, None),
        (DbType::Variant, None),
        (DbType::I1, Some(false)),
        (DbType::Ui1, Some(true)),
        (DbType::Ui2, Some(true)),
        (DbType::Ui4, Some(true)),
        (DbType::I8, Some(false)),
        (DbType::Ui8, Some(true)),
        (DbType::Guid, None),
        (DbType::Bytes, None),
        (DbType::Wstr, None),
        (DbType::Numeric, Some(false)),
        (DbType::DbDate, None),
        (DbType::DbTime, None),
        (DbType::DbTimestamp, None),
        (DbType::HChapter, None),
    ] {
        assert_eq!(indicator.is_unsigned(), expected, "{indicator}");
    }
}

#[test]
fn an_unsigned_integer_datatype_is_stated_unsigned_and_a_signed_one_signed() {
    for dtype in [
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
    ] {
        assert_eq!(DbType::of(&dtype).is_unsigned(), Some(true), "{dtype}");
    }
    for dtype in [
        DataType::Int8,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::Float16,
        DataType::Float64,
        DataType::decimal64(18, 2).expect("a decimal"),
    ] {
        assert_eq!(DbType::of(&dtype).is_unsigned(), Some(false), "{dtype}");
    }
}

#[test]
fn is_fixed_precision_holds_for_integers_and_numeric_alone() {
    for (indicator, expected) in [
        (DbType::Empty, false),
        (DbType::I2, true),
        (DbType::I4, true),
        (DbType::R4, false),
        (DbType::R8, false),
        (DbType::Bool, false),
        (DbType::Variant, false),
        (DbType::I1, true),
        (DbType::Ui1, true),
        (DbType::Ui2, true),
        (DbType::Ui4, true),
        (DbType::I8, true),
        (DbType::Ui8, true),
        (DbType::Guid, false),
        (DbType::Bytes, false),
        (DbType::Wstr, false),
        (DbType::Numeric, true),
        (DbType::DbDate, false),
        (DbType::DbTime, false),
        (DbType::DbTimestamp, false),
        (DbType::HChapter, false),
    ] {
        assert_eq!(indicator.is_fixed_precision(), expected, "{indicator}");
    }
}

#[test]
fn every_fixed_precision_indicator_is_a_number_with_a_stated_size() {
    for indicator in DbType::ALL
        .iter()
        .filter(|indicator| indicator.is_fixed_precision())
    {
        assert!(indicator.is_unsigned().is_some(), "{indicator}");
        assert!(indicator.column_size().is_some(), "{indicator}");
    }
}
