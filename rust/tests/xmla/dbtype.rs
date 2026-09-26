//! `rust/src/xmla/dbtype.rs`: OLE DB's `DBTYPE_*` indicators - their numbers
//! and names as `oledb.h` spells them, the one mapping from every `DataType`
//! family onto them whether the datatype was built, spelled or imported from
//! Arrow, and the `COLUMN_SIZE`, `UNSIGNED_ATTRIBUTE` and `FIXED_PREC_SCALE`
//! answers `DBSCHEMA_PROVIDER_TYPES` states for each.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use arrow_schema::{
    DataType as ArrowDataType, Field as ArrowField, Fields as ArrowFields, IntervalUnit,
    TimeUnit as ArrowTimeUnit, UnionFields, UnionMode as ArrowUnionMode,
};
use yggdryl::xmla::DbType;
use yggdryl::{
    DataType, DataTypeId, DataTypeKind, Field, StructType, TimeUnit, Timezone, UnionMode,
};

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

/// A dense union of an integer and a string member.
fn dense_union() -> DataType {
    DataType::union(
        [
            (0_i8, item(DataType::Int32)),
            (1_i8, Field::new("text", DataType::utf8(), true)),
        ],
        UnionMode::Dense,
    )
    .expect("a dense union")
}

/// `values` run-end encoded over non-null `int32` run ends.
fn run_end(values: DataType) -> DataType {
    DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", values, true),
    )
    .expect("a run-end encoded datatype")
}

/// Asserts that `DbType::of` states every datatype in `cases` as the
/// indicator beside it, naming the datatype that does not.
fn assert_maps(cases: &[(DataType, DbType)]) {
    for (dtype, expected) in cases {
        assert_eq!(DbType::of(dtype), *expected, "DbType::of({dtype})");
    }
}

/// The values a dictionary or a run-end encoding holds, through every layer
/// of either; any other datatype is its own.
fn encoded(dtype: &DataType) -> &DataType {
    match dtype {
        DataType::Dictionary(dictionary) => encoded(dictionary.value()),
        DataType::RunEndEncoded(encoding) => encoded(encoding.values().dtype()),
        other => other,
    }
}

/// The indicator OLE DB's vocabulary gives `dtype`, derived from the family
/// its `DataTypeId` byte lies in rather than from its variant: an integer or a
/// float by its byte width and its sign, a temporal by which of the five it
/// is, a nested value by whether it is a rowset, a choice or an encoding.
fn by_family(dtype: &DataType) -> DbType {
    let id = dtype.id();
    match id.kind() {
        DataTypeKind::Null => DbType::Empty,
        DataTypeKind::Boolean => DbType::Bool,
        DataTypeKind::Integer => {
            let unsigned = id.as_str().starts_with('u');
            match (id.fixed_byte_width(), unsigned) {
                (Some(1), false) => DbType::I1,
                (Some(2), false) => DbType::I2,
                (Some(4), false) => DbType::I4,
                (Some(8), false) => DbType::I8,
                (Some(1), true) => DbType::Ui1,
                (Some(2), true) => DbType::Ui2,
                (Some(4), true) => DbType::Ui4,
                (Some(8), true) => DbType::Ui8,
                other => panic!("{dtype}: OLE DB has no integer of {other:?}"),
            }
        }
        DataTypeKind::Floating => match id.fixed_byte_width() {
            Some(2 | 4) => DbType::R4,
            Some(8) => DbType::R8,
            other => panic!("{dtype}: OLE DB has no float of {other:?} bytes"),
        },
        DataTypeKind::Decimal => DbType::Numeric,
        DataTypeKind::Temporal => match id.temporal_family() {
            Some("date") => DbType::DbDate,
            Some("time") => DbType::DbTime,
            Some("datetime") => DbType::DbTimestamp,
            // OLE DB has no indicator for an elapsed time or a calendar span,
            // so they travel as the text they are spelled as.
            Some("duration" | "interval") => DbType::Wstr,
            other => panic!("{dtype}: an unknown temporal family {other:?}"),
        },
        DataTypeKind::Text | DataTypeKind::Code => DbType::Wstr,
        DataTypeKind::Bytes | DataTypeKind::Geospatial => DbType::Bytes,
        DataTypeKind::Uuid => DbType::Guid,
        DataTypeKind::Nested => match dtype {
            DataType::Dictionary(_) | DataType::RunEndEncoded(_) => by_family(encoded(dtype)),
            DataType::Union(_, _) | DataType::Variant => DbType::Variant,
            _ => DbType::HChapter,
        },
        other => panic!("{dtype}: a family {other:?} this rule does not know"),
    }
}

/// One datatype of every `DataTypeId` a `DataType` can hold, built through
/// the crate's constructors.
fn every_datatype() -> Vec<DataType> {
    vec![
        DataType::Null,
        DataType::Boolean,
        DataType::Int8,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
        DataType::Float16,
        DataType::Float32,
        DataType::Float64,
        DataType::decimal32(9, 2).expect("decimal32"),
        DataType::decimal64(18, 4).expect("decimal64"),
        DataType::decimal128(38, 10).expect("decimal128"),
        DataType::decimal256(76, 0).expect("decimal256"),
        DataType::datetime64(TimeUnit::Millisecond, Timezone::UTC).expect("datetime64"),
        DataType::Date32,
        DataType::Date64,
        DataType::time32(TimeUnit::Millisecond).expect("time32"),
        DataType::time64(TimeUnit::Nanosecond).expect("time64"),
        DataType::duration32(TimeUnit::Second).expect("duration32"),
        DataType::duration64(TimeUnit::Microsecond).expect("duration64"),
        DataType::interval(TimeUnit::MonthDayNano).expect("interval"),
        DataType::binary(),
        DataType::large_binary(),
        DataType::binary_view(),
        DataType::large_binary_view(),
        DataType::fixed_binary(4).expect("fixed_binary"),
        DataType::sized_binary(64).expect("sized_binary"),
        DataType::utf8(),
        DataType::large_utf8(),
        DataType::utf8_view(),
        DataType::large_utf8_view(),
        DataType::fixed_utf8(3).expect("fixed_utf8"),
        DataType::sized_utf8(32).expect("sized_utf8"),
        DataType::ascii(),
        DataType::large_ascii(),
        DataType::ascii_view(),
        DataType::large_ascii_view(),
        DataType::fixed_ascii(4).expect("fixed_ascii"),
        DataType::sized_ascii(4).expect("sized_ascii"),
        DataType::cp1252(),
        DataType::large_cp1252(),
        DataType::cp1252_view(),
        DataType::large_cp1252_view(),
        DataType::fixed_cp1252(8).expect("fixed_cp1252"),
        DataType::sized_cp1252(32).expect("sized_cp1252"),
        DataType::Version,
        DataType::Url,
        DataType::Urn,
        DataType::Timezone,
        DataType::MimeType,
        DataType::MediaType,
        DataType::Country,
        DataType::Ccy,
        DataType::MicCode,
        DataType::CfiCode,
        DataType::Side,
        DataType::State,
        DataType::TimeInForce,
        DataType::IsinCode,
        DataType::CusipCode,
        DataType::SedolCode,
        DataType::BloombergCode,
        DataType::FIGICode,
        DataType::Unit,
        DataType::Uuid,
        DataType::serie(item(DataType::Int32)),
        DataType::large_serie(item(DataType::utf8())),
        DataType::serie_view(item(DataType::Float64)),
        DataType::large_serie_view(item(DataType::Boolean)),
        DataType::fixed_size_serie(item(DataType::Int64), 2).expect("fixed_size_serie"),
        record(),
        DataType::map_of(DataType::utf8(), DataType::Int64, false).expect("a map"),
        DataType::map_of(DataType::utf8(), DataType::Int64, true).expect("a sorted map"),
        dense_union(),
        DataType::dictionary(DataType::Int32, DataType::utf8()).expect("a dictionary"),
        run_end(DataType::Float64),
        DataType::Variant,
        DataType::geometry(None).expect("a geometry"),
        DataType::geography(None, None).expect("a geography"),
    ]
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
fn a_by_reference_or_array_modifier_over_a_listed_code_reads_as_none() {
    // DBTYPE_BYREF | DBTYPE_WSTR, DBTYPE_ARRAY | DBTYPE_I4, DBTYPE_VECTOR |
    // DBTYPE_BYTES: a modifier is never masked off to find the base type.
    for code in [0x4000 | 130, 0x2000 | 3, 0x1000 | 128] {
        assert_eq!(
            DbType::from_code(code),
            None,
            "DbType::from_code({code:#x})"
        );
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
        assert_eq!(DbType::from_code(*code), Some(*indicator), "{name}");
    }
}

#[test]
fn all_lists_every_variant_the_enum_has() {
    // The match names every variant and has no wildcard, so a variant the
    // enum gains fails to compile here before it can go missing from `ALL`.
    let position = |indicator: DbType| match indicator {
        DbType::Empty => 0,
        DbType::I2 => 1,
        DbType::I4 => 2,
        DbType::R4 => 3,
        DbType::R8 => 4,
        DbType::Bool => 5,
        DbType::Variant => 6,
        DbType::I1 => 7,
        DbType::Ui1 => 8,
        DbType::Ui2 => 9,
        DbType::Ui4 => 10,
        DbType::I8 => 11,
        DbType::Ui8 => 12,
        DbType::Guid => 13,
        DbType::Bytes => 14,
        DbType::Wstr => 15,
        DbType::Numeric => 16,
        DbType::DbDate => 17,
        DbType::DbTime => 18,
        DbType::DbTimestamp => 19,
        DbType::HChapter => 20,
    };
    assert_eq!(
        DbType::ALL
            .iter()
            .copied()
            .map(position)
            .collect::<Vec<_>>(),
        (0..21).collect::<Vec<_>>()
    );
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
    assert_eq!(
        DbType::ALL.iter().copied().collect::<HashSet<_>>().len(),
        DbType::ALL.len()
    );
}

#[test]
fn the_derived_order_is_the_numeric_order_for_every_pair() {
    for left in DbType::ALL {
        for right in DbType::ALL {
            assert_eq!(
                left.cmp(right),
                left.code().cmp(&right.code()),
                "{left} against {right}"
            );
            assert_eq!(left == right, left.code() == right.code());
        }
    }
    let sorted = [DbType::HChapter, DbType::Empty, DbType::I1, DbType::R8]
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        sorted.into_iter().collect::<Vec<_>>(),
        [DbType::Empty, DbType::R8, DbType::I1, DbType::HChapter]
    );
}

#[test]
fn an_indicator_keys_a_hash_map_by_its_identity() {
    let names = DbType::ALL
        .iter()
        .map(|indicator| (*indicator, indicator.as_str()))
        .collect::<HashMap<_, _>>();
    assert_eq!(names.len(), DbType::ALL.len());
    assert_eq!(names[&DbType::Ui8], "DBTYPE_UI8");
    assert_eq!(names[&DbType::of(&DataType::Uuid)], "DBTYPE_GUID");
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
    for (indicator, _, name) in OLE_DB {
        assert_eq!(indicator.to_string(), *name);
        assert_eq!(format!("{indicator}"), *name);
        assert_eq!(indicator.to_string(), indicator.as_str());
    }
}

#[test]
fn debug_names_the_variant_where_display_names_the_indicator() {
    assert_eq!(format!("{:?}", DbType::DbTimestamp), "DbTimestamp");
    assert_eq!(format!("{}", DbType::DbTimestamp), "DBTYPE_DBTIMESTAMP");
    assert_eq!(format!("{:?}", DbType::Ui1), "Ui1");
    assert_eq!(format!("{}", DbType::Ui1), "DBTYPE_UI1");
}

#[test]
fn every_indicator_all_lists_is_what_some_datatype_maps_to() {
    let reached = every_datatype()
        .iter()
        .map(DbType::of)
        .collect::<BTreeSet<_>>();
    let listed = DbType::ALL.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(reached, listed);
}

#[test]
fn every_datatype_id_a_datatype_holds_is_stated_by_its_family() {
    let corpus = every_datatype();
    let covered = corpus.iter().map(DataType::id).collect::<BTreeSet<_>>();
    // Arrow has no 128-bit integer layout, so no datatype answers the two
    // identifiers the 128-bit scalars carry.
    let uncovered = DataTypeId::ALL
        .iter()
        .copied()
        .filter(|id| !covered.contains(id))
        .collect::<Vec<_>>();
    assert_eq!(uncovered, [DataTypeId::Int128, DataTypeId::UInt128]);
    for dtype in &corpus {
        assert_eq!(
            DbType::of(dtype),
            by_family(dtype),
            "{dtype} ({})",
            dtype.id()
        );
    }
}

#[test]
fn a_datatype_read_back_from_its_canonical_spelling_is_stated_as_itself() {
    for dtype in every_datatype() {
        let spelled = dtype.to_string();
        let read = spelled
            .parse::<DataType>()
            .unwrap_or_else(|error| panic!("{spelled}: {error}"));
        assert_eq!(DbType::of(&read), DbType::of(&dtype), "{spelled}");
    }
}

#[test]
fn every_logical_name_is_stated_by_the_family_it_resolves_to() {
    assert!(!DataType::LOGICAL_NAMES.is_empty());
    for (name, dtype) in DataType::LOGICAL_NAMES {
        assert_eq!(DbType::of(dtype), by_family(dtype), "{name} as {dtype}");
        let read = name
            .parse::<DataType>()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(DbType::of(&read), DbType::of(dtype), "{name}");
    }
}

#[test]
fn a_sql_hive_or_spark_spelling_is_stated_by_the_datatype_it_reads_as() {
    for (spelling, expected) in [
        ("void", DbType::Empty),
        ("bool", DbType::Bool),
        ("tinyint", DbType::I1),
        ("utinyint", DbType::Ui1),
        ("smallint", DbType::I2),
        ("usmallint", DbType::Ui2),
        ("int", DbType::I4),
        ("integer", DbType::I4),
        ("uint", DbType::Ui4),
        ("bigint", DbType::I8),
        ("ubigint", DbType::Ui8),
        ("half", DbType::R4),
        ("real", DbType::R4),
        ("float", DbType::R4),
        ("double", DbType::R8),
        ("double precision", DbType::R8),
        ("numeric(10,2)", DbType::Numeric),
        ("decimal(38,18)", DbType::Numeric),
        ("bignumeric(76,38)", DbType::Numeric),
        ("date", DbType::DbDate),
        ("date64", DbType::DbDate),
        ("time", DbType::DbTime),
        ("timestamp", DbType::DbTimestamp),
        ("timestamp_ntz", DbType::DbTimestamp),
        ("timestamp_ltz", DbType::DbTimestamp),
        ("interval year to month", DbType::Wstr),
        ("interval(day_time)", DbType::Wstr),
        ("blob", DbType::Bytes),
        ("bytea", DbType::Bytes),
        ("varbinary(8)", DbType::Bytes),
        ("fixed_size_binary(16)", DbType::Bytes),
        ("uuid", DbType::Guid),
        ("tz", DbType::Wstr),
        ("mime", DbType::Wstr),
        ("content_type", DbType::Wstr),
        ("url", DbType::Wstr),
        ("array<int64>", DbType::HChapter),
        ("list<int64>", DbType::HChapter),
        ("large_list<int64>", DbType::HChapter),
        ("fixed_size_list(int64, 3)", DbType::HChapter),
        ("struct<a:int32,b:string>", DbType::HChapter),
        ("map<string,array<decimal(38,18)>>", DbType::HChapter),
        ("dense_union(sparse:int64)", DbType::Variant),
        ("sparse_union(dense:int64)", DbType::Variant),
        ("variant", DbType::Variant),
        ("dictionary(int32,large_utf8)", DbType::Wstr),
        ("dictionary(int16,binary_view)", DbType::Bytes),
        ("run_end_encoded(int32,array<string>)", DbType::HChapter),
    ] {
        let dtype = spelling
            .parse::<DataType>()
            .unwrap_or_else(|error| panic!("{spelling}: {error}"));
        assert_eq!(DbType::of(&dtype), expected, "{spelling} as {dtype}");
    }
}

#[test]
fn an_arrow_datatype_is_stated_as_the_datatype_it_imports_as() {
    let child = || Arc::new(ArrowField::new("item", ArrowDataType::Int32, true));
    let entries = ArrowField::new(
        "entries",
        ArrowDataType::Struct(ArrowFields::from(vec![
            ArrowField::new("key", ArrowDataType::Utf8, false),
            ArrowField::new("value", ArrowDataType::Float64, true),
        ])),
        false,
    );
    let members = UnionFields::try_new(
        [0_i8, 1],
        [
            ArrowField::new("number", ArrowDataType::Int64, true),
            ArrowField::new("text", ArrowDataType::Utf8, true),
        ],
    )
    .expect("union members");
    for (arrow, expected) in [
        (ArrowDataType::Null, DbType::Empty),
        (ArrowDataType::Boolean, DbType::Bool),
        (ArrowDataType::Int8, DbType::I1),
        (ArrowDataType::Int16, DbType::I2),
        (ArrowDataType::Int32, DbType::I4),
        (ArrowDataType::Int64, DbType::I8),
        (ArrowDataType::UInt8, DbType::Ui1),
        (ArrowDataType::UInt16, DbType::Ui2),
        (ArrowDataType::UInt32, DbType::Ui4),
        (ArrowDataType::UInt64, DbType::Ui8),
        (ArrowDataType::Float16, DbType::R4),
        (ArrowDataType::Float32, DbType::R4),
        (ArrowDataType::Float64, DbType::R8),
        (ArrowDataType::Decimal32(9, 2), DbType::Numeric),
        (ArrowDataType::Decimal64(18, 0), DbType::Numeric),
        (ArrowDataType::Decimal128(38, 10), DbType::Numeric),
        (ArrowDataType::Decimal256(76, 0), DbType::Numeric),
        (ArrowDataType::Date32, DbType::DbDate),
        (ArrowDataType::Date64, DbType::DbDate),
        (ArrowDataType::Time32(ArrowTimeUnit::Second), DbType::DbTime),
        (
            ArrowDataType::Time64(ArrowTimeUnit::Nanosecond),
            DbType::DbTime,
        ),
        (
            ArrowDataType::Timestamp(ArrowTimeUnit::Microsecond, None),
            DbType::DbTimestamp,
        ),
        (
            ArrowDataType::Timestamp(ArrowTimeUnit::Nanosecond, Some("UTC".into())),
            DbType::DbTimestamp,
        ),
        (
            ArrowDataType::Duration(ArrowTimeUnit::Millisecond),
            DbType::Wstr,
        ),
        (
            ArrowDataType::Interval(IntervalUnit::MonthDayNano),
            DbType::Wstr,
        ),
        (ArrowDataType::Utf8, DbType::Wstr),
        (ArrowDataType::LargeUtf8, DbType::Wstr),
        (ArrowDataType::Utf8View, DbType::Wstr),
        (ArrowDataType::Binary, DbType::Bytes),
        (ArrowDataType::LargeBinary, DbType::Bytes),
        (ArrowDataType::BinaryView, DbType::Bytes),
        (ArrowDataType::FixedSizeBinary(16), DbType::Bytes),
        (ArrowDataType::List(child()), DbType::HChapter),
        (ArrowDataType::LargeList(child()), DbType::HChapter),
        (ArrowDataType::ListView(child()), DbType::HChapter),
        (ArrowDataType::LargeListView(child()), DbType::HChapter),
        (ArrowDataType::FixedSizeList(child(), 3), DbType::HChapter),
        (
            ArrowDataType::Struct(ArrowFields::from(vec![ArrowField::new(
                "a",
                ArrowDataType::Int32,
                true,
            )])),
            DbType::HChapter,
        ),
        (
            ArrowDataType::Map(Arc::new(entries), false),
            DbType::HChapter,
        ),
        (
            ArrowDataType::Union(members, ArrowUnionMode::Dense),
            DbType::Variant,
        ),
        (
            ArrowDataType::Dictionary(
                Box::new(ArrowDataType::Int32),
                Box::new(ArrowDataType::Utf8),
            ),
            DbType::Wstr,
        ),
        (
            ArrowDataType::Dictionary(
                Box::new(ArrowDataType::UInt16),
                Box::new(ArrowDataType::Date32),
            ),
            DbType::DbDate,
        ),
        (
            ArrowDataType::RunEndEncoded(
                Arc::new(ArrowField::new("run_ends", ArrowDataType::Int32, false)),
                Arc::new(ArrowField::new("values", ArrowDataType::Float64, true)),
            ),
            DbType::R8,
        ),
    ] {
        let dtype = DataType::from_arrow_datatype(&arrow)
            .unwrap_or_else(|error| panic!("{arrow}: {error}"));
        assert_eq!(DbType::of(&dtype), expected, "{arrow} as {dtype}");
    }
}

#[test]
fn an_arrow_extension_field_is_stated_as_its_identity_not_its_storage() {
    let extension = |name: &str, storage: ArrowDataType, extension: Option<&str>| {
        let field = ArrowField::new(name, storage, true);
        let field = match extension {
            Some(extension) => field
                .with_metadata([("ARROW:extension:name".to_owned(), extension.to_owned())].into()),
            None => field,
        };
        Field::from_arrow_field(&field).unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let id = extension("id", ArrowDataType::FixedSizeBinary(16), Some("arrow.uuid"));
    assert_eq!(id.dtype(), &DataType::Uuid, "{id}");
    assert_eq!(DbType::of(id.dtype()), DbType::Guid);
    let bare = extension("digest", ArrowDataType::FixedSizeBinary(16), None);
    assert_eq!(DbType::of(bare.dtype()), DbType::Bytes, "{bare}");
    let currency = extension("ccy", ArrowDataType::Utf8, Some("yggdryl.ccy"));
    assert_eq!(currency.dtype(), &DataType::Ccy, "{currency}");
    assert_eq!(DbType::of(currency.dtype()), DbType::Wstr);
    let shape = extension("shape", ArrowDataType::Binary, Some("geoarrow.wkb"));
    assert!(matches!(shape.dtype(), DataType::Geometry(_)), "{shape}");
    assert_eq!(DbType::of(shape.dtype()), DbType::Bytes);
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
        "LARGE-UTF8",
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
    let of_one_union = StructType::from_fields([dense_union().nullable_field("choice")])
        .map(DataType::from)
        .expect("a struct of a union");
    assert_maps(&[
        (record(), DbType::HChapter),
        (empty, DbType::HChapter),
        (nested, DbType::HChapter),
        (of_one_union, DbType::HChapter),
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
        (DataType::serie(item(dense_union())), DbType::HChapter),
        (DataType::serie(item(DataType::Uuid)), DbType::HChapter),
        (
            DataType::serie(item(DataType::serie(item(DataType::Int8)))),
            DbType::HChapter,
        ),
    ]);
}

#[test]
fn a_map_sorted_or_not_is_a_chapter() {
    let map = DataType::map_of(DataType::utf8(), DataType::Int64, false).expect("a map");
    let sorted = DataType::map_of(DataType::utf8(), DataType::Int64, true).expect("a sorted map");
    let of_unions = DataType::map_of(DataType::Int32, dense_union(), false).expect("a map");
    assert!(matches!(map, DataType::Map(_)), "{map}");
    assert!(matches!(sorted, DataType::SortedMap(_)), "{sorted}");
    assert_maps(&[
        (map, DbType::HChapter),
        (sorted, DbType::HChapter),
        (of_unions, DbType::HChapter),
    ]);
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
        (
            DataType::union(
                [(0_i8, Field::new("row", record(), true))],
                UnionMode::Sparse,
            )
            .expect("a union of a struct"),
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
        (
            DataType::dictionary(DataType::UInt64, DataType::Ccy).expect("a dictionary"),
            DbType::Wstr,
        ),
    ]);
}

#[test]
fn a_dictionary_over_a_choice_a_rowset_or_nothing_is_that_value() {
    assert_maps(&[
        (
            DataType::dictionary(DataType::Int32, dense_union()).expect("a union dictionary"),
            DbType::Variant,
        ),
        (
            DataType::dictionary(DataType::Int32, DataType::Variant).expect("a variant dictionary"),
            DbType::Variant,
        ),
        (
            DataType::dictionary(DataType::Int32, record()).expect("a struct dictionary"),
            DbType::HChapter,
        ),
        (
            DataType::dictionary(DataType::Int32, DataType::Null).expect("a null dictionary"),
            DbType::Empty,
        ),
        (
            DataType::dictionary(DataType::Int32, DataType::Uuid).expect("a uuid dictionary"),
            DbType::Guid,
        ),
    ]);
}

#[test]
fn a_run_end_encoded_column_is_stated_as_its_values_never_its_run_ends() {
    assert_maps(&[
        (run_end(DataType::Float64), DbType::R8),
        (run_end(DataType::utf8()), DbType::Wstr),
        (run_end(DataType::UInt16), DbType::Ui2),
        (run_end(DataType::Uuid), DbType::Guid),
        (run_end(DataType::Boolean), DbType::Bool),
        (run_end(record()), DbType::HChapter),
        (run_end(dense_union()), DbType::Variant),
        (run_end(DataType::Null), DbType::Empty),
        (
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int64, false),
                Field::new("values", DataType::Int16, true),
            )
            .expect("int64 run ends"),
            DbType::I2,
        ),
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
    let twice = DataType::dictionary(
        DataType::Int16,
        DataType::dictionary(DataType::Int8, DataType::Date64).expect("the inner dictionary"),
    )
    .expect("a dictionary of a dictionary");
    assert_eq!(DbType::of(&twice), DbType::DbDate);
    assert_eq!(DbType::of(&run_end(run_end(DataType::UInt32))), DbType::Ui4);
}

#[test]
fn column_size_states_each_fixed_indicator_and_leaves_the_rest_to_the_column() {
    for (indicator, expected) in [
        (DbType::Empty, Some(0)),
        // The byte width of a boolean and of a GUID.
        (DbType::Bool, Some(1)),
        (DbType::Guid, Some(16)),
        // DBTYPE_NUMERIC: the maximum precision OLE DB's table of numeric
        // precisions gives it.
        (DbType::Numeric, Some(38)),
        // A date and a timestamp: the length of their text at the maximum
        // fractional precision each holds - `yyyy-mm-dd`, and
        // `yyyy-mm-dd hh:mm:ss.fffffffff` since DBTIMESTAMP's fraction counts
        // billionths.
        (DbType::DbDate, Some(10)),
        (DbType::DbTimestamp, Some(29)),
        (DbType::Bytes, None),
        (DbType::Wstr, None),
        (DbType::Variant, None),
        (DbType::HChapter, None),
    ] {
        assert_eq!(indicator.column_size(), expected, "{indicator}");
    }
    assert_eq!("yyyy-mm-dd".len(), 10);
    assert_eq!("yyyy-mm-dd hh:mm:ss.fffffffff".len(), 29);
}

#[test]
fn column_size_of_a_dbtime_is_the_length_of_hh_mm_ss_since_dbtime_holds_no_fraction() {
    // `oledb.h`'s DBTIME is `{ USHORT hour; USHORT minute; USHORT second; }`
    // - no fraction, unlike DBTIMESTAMP's `ULONG fraction` - and OLE DB's
    // COLUMN_SIZE for a datetime indicator is "the length of the string
    // representation (assuming the maximum allowed precision of the
    // fractional seconds component)", which for DBTIME is none.
    assert_eq!(DbType::DbTime.column_size(), Some("hh:mm:ss".len() as u32));
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
    // The two the rustdoc names by number.
    assert_eq!(DbType::I4.column_size(), Some(10));
    assert_eq!(DbType::Ui8.column_size(), Some(20));
}

#[test]
fn column_size_of_a_float_is_its_decimal_precision() {
    // A single holds seven significant digits and, as the rustdoc states, a
    // double fifteen: the precision SQL Server's own provider states for
    // `real` and `float`.
    assert_eq!(DbType::R4.column_size(), Some(7));
    assert_eq!(DbType::R8.column_size(), Some(15));
    assert_eq!(DbType::of(&DataType::Float16).column_size(), Some(7));
    assert_eq!(DbType::of(&DataType::Float64).column_size(), Some(15));
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
fn a_number_a_boolean_a_uuid_or_an_instant_is_never_stated_with_an_open_width() {
    // An open width is the column's to state; a datatype of a fixed width
    // never reaches an indicator that leaves it open.
    for dtype in every_datatype() {
        let values = encoded(&dtype);
        let fixed = match values.kind() {
            DataTypeKind::Null
            | DataTypeKind::Boolean
            | DataTypeKind::Integer
            | DataTypeKind::Floating
            | DataTypeKind::Decimal
            | DataTypeKind::Uuid => true,
            DataTypeKind::Temporal => matches!(
                values.id().temporal_family(),
                Some("date" | "time" | "datetime")
            ),
            _ => false,
        };
        assert_eq!(
            DbType::of(&dtype).column_size().is_some(),
            fixed,
            "{dtype} as {}",
            DbType::of(&dtype)
        );
    }
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
fn a_number_is_exactly_what_answers_a_sign() {
    // A sign is answered for the integer, floating and decimal families and
    // for nothing else a column can hold.
    for dtype in every_datatype() {
        let number = matches!(
            by_family(&dtype),
            DbType::I1
                | DbType::I2
                | DbType::I4
                | DbType::I8
                | DbType::Ui1
                | DbType::Ui2
                | DbType::Ui4
                | DbType::Ui8
                | DbType::R4
                | DbType::R8
                | DbType::Numeric
        );
        assert_eq!(
            DbType::of(&dtype).is_unsigned().is_some(),
            number,
            "{dtype}"
        );
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
