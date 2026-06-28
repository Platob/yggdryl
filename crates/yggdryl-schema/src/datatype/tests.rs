//! Tests for the schema crate: the [`DataType`](crate::DataType) grammar, the
//! uniform accessors, the conversion / merge lattice and the [`Field`](crate::Field)
//! graph node (plus serde / json / arrow round-trips).

use crate::{
    Charset, DataType, Field, IntervalUnit, MergeStrategy, SchemaError, TypeCategory, UnionMode,
};
use yggdryl_core::{TimeUnit, Timezone};

/// Types whose string form round-trips losslessly (UTF-8 strings only, so the Arrow
/// round-trip is exact too).
fn sample_types() -> Vec<DataType> {
    vec![
        DataType::Any,
        DataType::Null,
        DataType::Boolean,
        DataType::int(8, true),
        DataType::int(16, false),
        DataType::int(32, true),
        DataType::int(64, false),
        DataType::float(16),
        DataType::float(32),
        DataType::float(64),
        DataType::varchar(),
        DataType::varchar_with(Charset::Utf8, true, false, None),
        DataType::varchar_with(Charset::Utf8, false, true, None),
        DataType::binary(),
        DataType::Binary {
            large: true,
            view: false,
            size: None,
        },
        DataType::Binary {
            large: false,
            view: true,
            size: None,
        },
        DataType::fixed_size_binary(16),
        DataType::decimal(38, 10),
        DataType::decimal_with(9, 2, 32),
        DataType::decimal_with(76, 0, 256),
        DataType::date(),
        DataType::Date { large: true },
        DataType::Time {
            unit: TimeUnit::Second,
        },
        DataType::Time {
            unit: TimeUnit::Nanosecond,
        },
        DataType::Duration {
            unit: TimeUnit::Millisecond,
        },
        DataType::Interval {
            unit: IntervalUnit::MonthDayNano,
        },
        DataType::timestamp(TimeUnit::Microsecond, None),
        DataType::timestamp(TimeUnit::Microsecond, Some(Timezone::Utc)),
        DataType::timestamp(
            TimeUnit::Nanosecond,
            Some(Timezone::from_str("America/New_York").unwrap()),
        ),
        DataType::timestamp(TimeUnit::Second, Some(Timezone::Fixed(19_800))),
        // A raw POSIX zone contains commas — its round-trip exercises the
        // first-comma split in the timestamp grammar.
        DataType::timestamp(
            TimeUnit::Microsecond,
            Some(Timezone::from_str("EST5EDT,M3.2.0,M11.1.0").unwrap()),
        ),
        DataType::dictionary(DataType::int(32, true), DataType::varchar()),
        DataType::list(Field::new("item", DataType::varchar(), true)),
        DataType::large_list(Field::new("item", DataType::int(64, true), false)),
        DataType::fixed_size_list(Field::new("item", DataType::float(64), true), 3),
        DataType::List {
            item: Box::new(Field::new("item", DataType::int(8, true), true)),
            large: false,
            view: true,
            size: None,
        },
        DataType::struct_(vec![]),
        DataType::struct_(vec![
            Field::new("id", DataType::int(64, true), false),
            Field::new("name", DataType::varchar(), true),
            Field::new(
                "inner",
                DataType::struct_(vec![Field::new("x", DataType::int(32, true), true)]),
                true,
            ),
        ]),
        DataType::map(DataType::varchar(), DataType::int(64, true), false),
        DataType::map(DataType::varchar(), DataType::int(64, true), true),
        DataType::union(
            vec![
                Field::new("a", DataType::int(32, true), true),
                Field::new("b", DataType::varchar(), true),
            ],
            UnionMode::Sparse,
        ),
        DataType::union(
            vec![Field::new("a", DataType::float(64), true)],
            UnionMode::Dense,
        ),
        DataType::run_end_encoded(DataType::int(32, true), DataType::varchar()),
    ]
}

#[test]
fn string_round_trips_for_every_type() {
    for dt in sample_types() {
        let rendered = dt.to_str();
        let parsed =
            DataType::from_str(&rendered).unwrap_or_else(|e| panic!("re-parse {rendered:?}: {e}"));
        assert_eq!(parsed, dt, "round-trip mismatch for {rendered:?}");
        // bytes round-trip too.
        assert_eq!(DataType::from_bytes(&dt.to_bytes()).unwrap(), dt);
    }
}

#[test]
fn parses_canonical_and_aliases() {
    assert_eq!(
        DataType::from_str("int64").unwrap(),
        DataType::int(64, true)
    );
    // SQL semantics: bare `int`/`integer` is 32-bit, `bigint` is 64-bit.
    assert_eq!(DataType::from_str("int").unwrap(), DataType::int(32, true));
    assert_eq!(
        DataType::from_str("bigint").unwrap(),
        DataType::int(64, true)
    );
    assert_eq!(
        DataType::from_str("uint8").unwrap(),
        DataType::int(8, false)
    );
    assert_eq!(DataType::from_str("BOOLEAN").unwrap(), DataType::Boolean);
    assert_eq!(DataType::from_str("string").unwrap(), DataType::varchar());
    assert_eq!(DataType::from_str("double").unwrap(), DataType::float(64));
    assert_eq!(DataType::from_str("date").unwrap(), DataType::date());
    assert_eq!(
        DataType::from_str("decimal[10, 2]").unwrap(),
        DataType::decimal(10, 2)
    );
}

#[test]
fn varchar_charset_grammar() {
    assert_eq!(DataType::varchar().to_str(), "utf8");
    assert_eq!(
        DataType::varchar_with(Charset::Utf8, true, false, None).to_str(),
        "large_utf8"
    );
    let latin = DataType::varchar_with(Charset::Latin1, false, false, None);
    assert_eq!(latin.to_str(), "varchar[latin1]");
    assert_eq!(DataType::from_str("varchar[latin1]").unwrap(), latin);
    let big_latin = DataType::varchar_with(Charset::Latin1, true, false, None);
    assert_eq!(big_latin.to_str(), "varchar[latin1, large]");
    assert_eq!(
        DataType::from_str("varchar[latin1, large]").unwrap(),
        big_latin
    );
    assert_eq!(DataType::from_str("varchar").unwrap(), DataType::varchar());
    assert_eq!(latin.charset(), Some(Charset::Latin1));
}

#[test]
fn uniform_physical_accessors() {
    assert_eq!(DataType::int(32, true).bit_size(), Some(32));
    assert_eq!(DataType::Boolean.bit_size(), Some(1));
    assert_eq!(DataType::float(64).bit_size(), Some(64));
    assert_eq!(DataType::decimal_with(20, 2, 128).bit_size(), Some(128));
    assert_eq!(DataType::fixed_size_binary(10).bit_size(), Some(80));
    assert_eq!(DataType::date().bit_size(), Some(32));
    assert_eq!(DataType::Date { large: true }.bit_size(), Some(64));
    assert_eq!(
        DataType::Time {
            unit: TimeUnit::Second
        }
        .bit_size(),
        Some(32)
    );
    assert_eq!(
        DataType::Time {
            unit: TimeUnit::Nanosecond
        }
        .bit_size(),
        Some(64)
    );
    assert_eq!(
        DataType::Interval {
            unit: IntervalUnit::DayTime
        }
        .bit_size(),
        Some(64)
    );
    assert_eq!(DataType::varchar().bit_size(), None);
    assert_eq!(DataType::int(32, true).byte_size(), Some(4));
    assert_eq!(DataType::Boolean.byte_size(), None);
    // large / view flags.
    assert!(DataType::varchar_with(Charset::Utf8, true, false, None).is_large());
    assert!(DataType::varchar_with(Charset::Utf8, false, true, None).is_view());
    assert!(DataType::Date { large: true }.is_large());
    assert!(!DataType::int(32, true).is_large());
}

#[test]
fn categories_and_checks() {
    assert_eq!(DataType::int(32, true).category(), TypeCategory::Primitive);
    assert_eq!(DataType::varchar().category(), TypeCategory::Primitive);
    assert_eq!(DataType::date().category(), TypeCategory::Logical);
    assert_eq!(DataType::decimal(10, 2).category(), TypeCategory::Logical);
    assert_eq!(DataType::struct_(vec![]).category(), TypeCategory::Nested);
    assert_eq!(DataType::Any.category(), TypeCategory::Any);
    assert!(DataType::int(32, true).is_signed_integer());
    assert!(DataType::int(32, false).is_unsigned_integer());
    assert!(DataType::float(32).is_numeric() && !DataType::decimal(1, 0).is_numeric());
    assert!(DataType::binary().is_binary() && DataType::varchar().is_string());
    assert!(DataType::timestamp(TimeUnit::Second, None).is_temporal());
    assert!(DataType::map(DataType::varchar(), DataType::int(8, true), false).is_map());
    assert_eq!(
        DataType::timestamp(TimeUnit::Microsecond, None).time_unit(),
        Some(TimeUnit::Microsecond)
    );
    assert_eq!(DataType::decimal(10, 2).decimal_parts(), Some((10, 2)));
    assert_eq!(
        DataType::timestamp(TimeUnit::Second, Some(Timezone::Utc)).timezone(),
        Some(&Timezone::Utc)
    );
}

#[test]
fn children_accessor() {
    let s = DataType::struct_(vec![
        Field::new("a", DataType::int(32, true), true),
        Field::new("b", DataType::varchar(), true),
    ]);
    assert_eq!(s.children().len(), 2);
    assert_eq!(
        DataType::list(Field::new("item", DataType::int(8, true), true))
            .children()
            .len(),
        1
    );
    assert!(DataType::int(32, true).children().is_empty());
}

#[test]
fn parse_errors() {
    assert_eq!(DataType::from_str(""), Err(SchemaError::Empty));
    assert!(matches!(
        DataType::from_str("notatype"),
        Err(SchemaError::Unknown(_))
    ));
    assert!(matches!(
        DataType::from_str("list[utf8"),
        Err(SchemaError::Invalid(_))
    ));
    assert!(matches!(
        DataType::from_str("int64[3]"),
        Err(SchemaError::Unknown(_))
    ));
    assert!(matches!(
        DataType::from_str("timestamp[nope]"),
        Err(SchemaError::UnknownUnit(_))
    ));
    assert!(matches!(
        DataType::from_str("varchar[klingon]"),
        Err(SchemaError::UnknownUnit(_))
    ));
}

#[test]
fn common_type_numeric() {
    use DataType as D;
    assert_eq!(
        D::int(8, true).common_type(&D::int(32, true)),
        Some(D::int(32, true))
    );
    assert_eq!(
        D::int(8, true).common_type(&D::int(8, false)),
        Some(D::int(16, true))
    );
    assert_eq!(
        D::int(32, true).common_type(&D::int(32, false)),
        Some(D::int(64, true))
    );
    assert_eq!(
        D::int(8, true).common_type(&D::int(64, false)),
        Some(D::float(64))
    );
    assert_eq!(
        D::int(16, true).common_type(&D::float(32)),
        Some(D::float(32))
    );
    assert_eq!(
        D::int(32, true).common_type(&D::float(32)),
        Some(D::float(64))
    );
    // 8 integer digits + max(2,4) fractional digits = precision 12, scale 4.
    assert_eq!(
        D::decimal(10, 2).common_type(&D::decimal(12, 4)),
        Some(D::decimal(12, 4))
    );
    assert_eq!(
        D::decimal(10, 2).common_type(&D::int(32, true)),
        Some(D::decimal(12, 2))
    );
    assert_eq!(
        D::decimal(10, 2).common_type(&D::float(64)),
        Some(D::float(64))
    );
    assert_eq!(D::int(32, true).common_type(&D::varchar()), None);
}

#[test]
fn common_type_strings_temporal_nested() {
    use DataType as D;
    assert_eq!(
        D::varchar().common_type(&D::varchar_with(Charset::Utf8, true, false, None)),
        Some(D::varchar_with(Charset::Utf8, true, false, None))
    );
    // Differing charsets do not unify.
    assert_eq!(
        D::varchar().common_type(&D::varchar_with(Charset::Latin1, false, false, None)),
        None
    );
    assert_eq!(
        D::date().common_type(&D::Date { large: true }),
        Some(D::Date { large: true })
    );
    assert_eq!(
        D::timestamp(TimeUnit::Second, None).common_type(&D::timestamp(TimeUnit::Nanosecond, None)),
        Some(D::timestamp(TimeUnit::Nanosecond, None))
    );
    // list element promotes.
    let a = D::list(Field::new("item", D::int(8, true), true));
    let b = D::list(Field::new("item", D::int(32, true), false));
    assert_eq!(
        a.common_type(&b),
        Some(D::list(Field::new("item", D::int(32, true), true)))
    );
    // struct unions by name.
    let sa = D::struct_(vec![
        Field::new("x", D::int(8, true), false),
        Field::new("y", D::varchar(), true),
    ]);
    let sb = D::struct_(vec![
        Field::new("x", D::int(32, true), false),
        Field::new("z", D::int(64, true), true),
    ]);
    let Some(D::Struct(merged)) = sa.common_type(&sb) else {
        panic!()
    };
    assert_eq!(
        merged.iter().map(|f| f.name()).collect::<Vec<_>>(),
        vec!["x", "y", "z"]
    );
    assert_eq!(merged[0].data_type(), &D::int(32, true));
    assert!(merged[1].is_nullable() && merged[2].is_nullable());
    // Any / Null identity.
    assert_eq!(
        D::Any.common_type(&D::int(32, true)),
        Some(D::int(32, true))
    );
    assert_eq!(D::Null.common_type(&D::varchar()), Some(D::varchar()));
}

#[test]
fn can_cast_and_merge_strategies() {
    use DataType as D;
    assert!(D::int(32, true).can_cast_to(&D::int(64, true)));
    assert!(D::int(32, true).can_cast_to(&D::varchar()));
    assert!(D::varchar().can_cast_to(&D::int(32, true)));
    assert!(!D::int(32, true).can_cast_to(&D::binary()));
    assert!(D::Null.can_cast_to(&D::int(32, true)) && !D::int(32, true).can_cast_to(&D::Null));
    assert!(D::date().can_cast_to(&D::int(32, true)));
    // strategies
    assert_eq!(
        D::int(8, true)
            .merge(&D::int(64, true), MergeStrategy::Promote)
            .unwrap(),
        D::int(64, true)
    );
    assert!(D::int(8, true)
        .merge(&D::int(64, true), MergeStrategy::Strict)
        .is_err());
    assert_eq!(
        D::Any
            .merge(&D::int(8, true), MergeStrategy::Strict)
            .unwrap(),
        D::int(8, true)
    );
    assert_eq!(
        D::int(8, true)
            .merge(&D::varchar(), MergeStrategy::Permissive)
            .unwrap(),
        D::Any
    );
    assert_eq!(
        MergeStrategy::from_str("widen").unwrap(),
        MergeStrategy::Promote
    );
    assert_eq!(MergeStrategy::default(), MergeStrategy::Promote);
}

#[test]
fn timestamp_raw_posix_zone_round_trips() {
    // A raw POSIX zone keeps its embedded commas through the string grammar.
    let dt = DataType::from_str("timestamp[us, EST5EDT,M3.2.0,M11.1.0]").unwrap();
    assert_eq!(
        dt.timezone().map(Timezone::name).as_deref(),
        Some("EST5EDT,M3.2.0,M11.1.0")
    );
    assert_eq!(DataType::from_str(&dt.to_str()).unwrap(), dt);
}

#[test]
fn interval_common_type_widens_to_month_day_nano() {
    use DataType as D;
    // Differing interval units must widen to MonthDayNano (the only superset that
    // keeps both months and sub-day components) — never to one that drops a field.
    let ym = D::Interval {
        unit: IntervalUnit::YearMonth,
    };
    let dt = D::Interval {
        unit: IntervalUnit::DayTime,
    };
    assert_eq!(
        ym.common_type(&dt),
        Some(D::Interval {
            unit: IntervalUnit::MonthDayNano
        })
    );
    // Equal units are preserved.
    assert_eq!(ym.common_type(&ym), Some(ym.clone()));
}

#[test]
fn common_decimal_overflow_falls_back_to_float() {
    use DataType as D;
    // 70 integer digits + 10 fractional > the 76-digit decimal cap, so the common
    // type widens to float64 instead of silently clamping (and dropping digits).
    assert_eq!(
        D::decimal(76, 6).common_type(&D::decimal(76, 10)),
        Some(D::float(64))
    );
}

#[test]
fn promote_field_folds_metadata() {
    use DataType as D;
    // A list element's metadata is preserved (first wins, second folded in) when its
    // type promotes — the same rule as Field::merge.
    let a = D::list(Field::new("item", D::int(8, true), true).with_metadata_entry("k", "a"));
    let b = D::list(
        Field::new("item", D::int(32, true), false)
            .with_metadata_entry("k", "b")
            .with_metadata_entry("k2", "c"),
    );
    let Some(D::List { item, .. }) = a.common_type(&b) else {
        panic!("list promotes")
    };
    assert_eq!(item.data_type(), &D::int(32, true));
    assert_eq!(item.get_metadata("k"), Some("a"));
    assert_eq!(item.get_metadata("k2"), Some("c"));
}

#[test]
fn run_end_encoded_is_transparent_to_cast_and_merge() {
    use DataType as D;
    let ree = D::run_end_encoded(D::int(32, true), D::int(8, true));
    // Casting / merging see through run-end encoding to the values type.
    assert!(ree.can_cast_to(&D::int(64, true)));
    assert!(D::int(8, true).can_cast_to(&ree));
    assert_eq!(ree.common_type(&D::int(32, true)), Some(D::int(32, true)));
}

#[test]
fn grammar_rejects_extra_args_and_unbalanced_brackets() {
    // map accepts only `key, value[, sorted]`.
    assert!(DataType::from_str("map[utf8, int64]").is_ok());
    assert!(DataType::from_str("map[utf8, int64, sorted]").is_ok());
    assert!(matches!(
        DataType::from_str("map[utf8, int64, nope]"),
        Err(SchemaError::Invalid(_))
    ));
    assert!(matches!(
        DataType::from_str("map[utf8, int64, sorted, extra]"),
        Err(SchemaError::Invalid(_))
    ));
    // A stray closing bracket inside a name is rejected, not absorbed.
    assert!(matches!(
        DataType::from_str("struct[a]: int]"),
        Err(SchemaError::Invalid(_))
    ));
    // A quoted name may still legitimately contain a bracket.
    let dt = DataType::from_str("struct[\"a]b\": int32]").unwrap();
    assert_eq!(dt.children()[0].name(), "a]b");
}

#[test]
fn fixed_integer_widths() {
    use crate::{FixedType, Int32, UInt8};
    use DataType as D;
    // Each integer alias resolves to its concrete fixed variant.
    assert_eq!(D::from_str("int8").unwrap(), D::int8());
    assert_eq!(D::from_str("int16").unwrap(), D::int16());
    assert_eq!(D::from_str("int32").unwrap(), D::int32());
    assert_eq!(D::from_str("int64").unwrap(), D::int64());
    assert_eq!(D::from_str("uint8").unwrap(), D::uint8());
    assert_eq!(D::from_str("uint64").unwrap(), D::uint64());
    // Explicit constructors and the width builder agree.
    assert_eq!(D::int8(), D::int(8, true));
    assert_eq!(D::uint32(), D::int(32, false));
    assert_eq!(D::integer(), D::int64());
    // Arbitrary widths are no longer a type — they are unknown, not a generic int.
    assert!(matches!(D::from_str("int24"), Err(SchemaError::Unknown(_))));
    assert!(matches!(
        D::from_str("uint128"),
        Err(SchemaError::Unknown(_))
    ));
    assert!(matches!(D::from_str("int0"), Err(SchemaError::Unknown(_))));
    assert!(matches!(
        D::from_str("intfoo"),
        Err(SchemaError::Unknown(_))
    ));
    // A non-standard width passed to the builder rounds up to the next fixed width.
    assert_eq!(D::int(24, true), D::int32());
    assert_eq!(D::int(128, false), D::uint64());
    // The native Rust storage type is named per variant; the descriptor mirrors it.
    assert_eq!(D::int32().native_name(), Some("i32"));
    assert_eq!(D::uint8().native_name(), Some("u8"));
    assert_eq!(Int32.data_type(), D::int32());
    assert_eq!(DataType::from(UInt8), D::uint8());
}

#[test]
fn fixed_float_widths() {
    use crate::{f16, FixedType, Float16};
    use DataType as D;
    assert_eq!(D::from_str("float16").unwrap(), D::float16());
    assert_eq!(D::from_str("float32").unwrap(), D::float32());
    assert_eq!(D::from_str("double").unwrap(), D::float64());
    assert_eq!(D::float16(), D::float(16));
    assert_eq!(D::floating(), D::float64()); // default width
                                             // Arbitrary float widths are unknown now.
    assert!(matches!(
        D::from_str("float24"),
        Err(SchemaError::Unknown(_))
    ));
    assert!(matches!(
        D::from_str("float128"),
        Err(SchemaError::Unknown(_))
    ));
    // The half float is backed by the created `f16` native type.
    assert_eq!(D::float16().native_name(), Some("f16"));
    assert_eq!(Float16.data_type(), D::float16());
    assert_eq!(f16::from_f32(0.5).to_f32(), 0.5);
}

#[test]
fn fixed_decimal_widths_and_native_types() {
    use crate::{i256, Decimal128, FixedType};
    use DataType as D;
    // Each decimal width is a concrete variant carrying precision/scale.
    assert_eq!(D::from_str("decimal32[9, 2]").unwrap(), D::decimal32(9, 2));
    assert_eq!(
        D::from_str("decimal64[18, 4]").unwrap(),
        D::decimal64(18, 4)
    );
    assert_eq!(
        D::from_str("decimal128[38, 10]").unwrap(),
        D::decimal128(38, 10)
    );
    assert_eq!(
        D::from_str("decimal256[76, 0]").unwrap(),
        D::decimal256(76, 0)
    );
    // The bare `decimal[..]` alias is the 128-bit decimal.
    assert_eq!(D::from_str("decimal[10, 2]").unwrap(), D::decimal128(10, 2));
    // Native storage names: i32/i64/i128/i256.
    assert_eq!(D::decimal32(9, 2).native_name(), Some("i32"));
    assert_eq!(D::decimal128(10, 2).native_name(), Some("i128"));
    assert_eq!(D::decimal256(76, 0).native_name(), Some("i256"));
    // The descriptor struct mirrors the variant.
    assert_eq!(Decimal128::new(10, 2).data_type(), D::decimal128(10, 2));
    // The created 256-bit native type round-trips a value beyond i128.
    assert_eq!(i256::from_i128(-5).to_str(), "-5");
}

#[test]
fn numeric_trait_bits_and_signed() {
    use crate::Numeric;
    use DataType as D;
    // numeric_bits + signed are mutualised across int / float / decimal.
    assert_eq!(D::int(32, false).numeric_bits(), Some(32));
    assert_eq!(D::int(32, false).signed(), Some(false));
    assert_eq!(D::int(64, true).signed(), Some(true));
    assert_eq!(D::float(64).numeric_bits(), Some(64));
    assert_eq!(D::float(64).signed(), Some(true)); // floats are always signed
    assert_eq!(D::decimal_with(20, 2, 128).numeric_bits(), Some(128));
    assert_eq!(D::decimal(10, 2).signed(), Some(true));
    assert!(D::int(8, true).is_numeric_kind() && D::float(16).is_numeric_kind());
    // Non-numeric types report None.
    assert_eq!(D::varchar().numeric_bits(), None);
    assert_eq!(D::varchar().signed(), None);
    assert!(!D::date().is_numeric_kind());
}

#[test]
fn json_bson_and_physical_types() {
    use DataType as D;
    assert_eq!(D::from_str("json").unwrap(), D::json());
    assert_eq!(D::from_str("jsonb").unwrap(), D::json());
    assert_eq!(D::from_str("bson").unwrap(), D::bson());
    assert_eq!(D::json().to_str(), "json");
    assert_eq!(D::bson().to_str(), "bson");
    assert!(D::json().is_json() && D::json().is_logical());
    assert!(D::bson().is_bson() && D::bson().is_logical());
    assert_eq!(D::json().category(), TypeCategory::Logical);
    assert_eq!(D::from_bytes(&D::json().to_bytes()).unwrap(), D::json());
    // physical (storage) types.
    assert_eq!(D::json().physical_type(), D::varchar());
    assert_eq!(D::bson().physical_type(), D::binary());
    assert_eq!(D::date().physical_type(), D::int(32, true));
    assert_eq!(D::Date { large: true }.physical_type(), D::int(64, true));
    assert_eq!(
        D::timestamp(TimeUnit::Microsecond, None).physical_type(),
        D::int(64, true)
    );
    assert_eq!(
        D::decimal_with(10, 2, 128).physical_type(),
        D::fixed_size_binary(16)
    );
    assert_eq!(
        D::dictionary(D::int(16, true), D::varchar()).physical_type(),
        D::int(16, true)
    );
    assert_eq!(D::int(32, true).physical_type(), D::int(32, true)); // identity
                                                                    // json/bson cast + merge with their physical type.
    assert!(D::json().can_cast_to(&D::varchar()) && D::varchar().can_cast_to(&D::json()));
    assert!(D::bson().can_cast_to(&D::binary()));
    assert_eq!(D::json().common_type(&D::varchar()), Some(D::varchar()));
}

#[test]
fn timezone_logical_type() {
    use DataType as D;
    assert_eq!(D::from_str("timezone").unwrap(), D::Timezone);
    assert_eq!(D::from_str("tz").unwrap(), D::Timezone);
    assert_eq!(D::Timezone.to_str(), "timezone");
    assert!(D::Timezone.is_timezone() && D::Timezone.is_logical());
    assert_eq!(D::Timezone.category(), TypeCategory::Logical);
    assert_eq!(D::Timezone.physical_type(), D::varchar());
    assert_eq!(D::from_bytes(&D::Timezone.to_bytes()).unwrap(), D::Timezone);
    // casts / merges with its string physical type, like json.
    assert!(D::Timezone.can_cast_to(&D::varchar()) && D::varchar().can_cast_to(&D::Timezone));
    assert_eq!(D::Timezone.common_type(&D::varchar()), Some(D::varchar()));
    // a Timezone *value* type is distinct from a Timestamp's display timezone attribute.
    assert!(D::timestamp(TimeUnit::Microsecond, None)
        .timezone()
        .is_none());
}

#[test]
fn datatype_interning() {
    use std::sync::Arc;
    use DataType as D;
    let a = D::int(32, true).interned();
    let b = D::int(32, true).interned();
    assert!(Arc::ptr_eq(&a, &b)); // same shared allocation
    assert_eq!(*a, D::int(32, true));
    // a different type interns to its own shared allocation, stable across calls.
    let c = D::varchar().interned();
    let d = D::varchar().interned();
    assert!(Arc::ptr_eq(&c, &d));
    assert!(!Arc::ptr_eq(&a, &c));
    assert_eq!(*c, D::varchar());
}

#[test]
fn fixed_size_string_and_binary() {
    use DataType as D;
    // char(n) is fixed; varchar(n) stays variable (the length is a max hint).
    let fixed = D::from_str("char[10]").unwrap();
    assert_eq!(fixed, D::fixed_size_varchar(10));
    assert!(fixed.is_fixed_size());
    assert_eq!(fixed.to_str(), "char[10]");
    assert_eq!(
        D::from_str("char(255)").unwrap(),
        D::fixed_size_varchar(255)
    );
    assert_eq!(D::from_str("varchar(255)").unwrap(), D::varchar()); // still variable
    assert!(!D::varchar().is_fixed_size());
    assert!(!D::binary().is_fixed_size());
    assert!(D::fixed_size_binary(16).is_fixed_size());
    assert!(D::int(32, true).is_fixed_size());
    // Every (charset, large, view, size) combo round-trips through the `char[..]` form.
    for dt in [
        D::varchar_with(Charset::Latin1, false, false, Some(8)),
        D::varchar_with(Charset::Utf8, true, false, Some(8)),
        D::varchar_with(Charset::Utf8, false, true, Some(8)),
    ] {
        assert_eq!(
            D::from_str(&dt.to_str()).unwrap(),
            dt,
            "round-trip {}",
            dt.to_str()
        );
    }
    // common_type keeps a shared fixed size, else falls back to variable.
    assert_eq!(
        D::fixed_size_varchar(8).common_type(&D::fixed_size_varchar(8)),
        Some(D::fixed_size_varchar(8))
    );
    assert_eq!(
        D::fixed_size_varchar(8).common_type(&D::fixed_size_varchar(4)),
        Some(D::varchar())
    );
}

// ---- Field ----

#[test]
fn field_surface() {
    let f = Field::new("id", DataType::int(64, true), false).with_comment("primary key");
    assert_eq!(f.name(), "id");
    assert!(!f.is_nullable());
    assert_eq!(f.comment(), Some("primary key"));
    assert_eq!(f.to_str(), "id: int64 not null");
    assert_eq!(
        Field::from_str("id: int64 not null").unwrap(),
        f.clone().without_metadata()
    );
    // builders are non-mutating.
    let g = f.clone().with_nullable(true).with_name("ident");
    assert_eq!((g.name(), g.is_nullable()), ("ident", true));
    assert_eq!(f.name(), "id");
    // metadata getters/setters.
    let mut m = f.clone();
    m.set_metadata("unit", "count");
    assert_eq!(m.get_metadata("unit"), Some("count"));
    assert_eq!(m.remove_metadata("unit"), Some("count".to_string()));
    // mapping round-trip incl. comment.
    assert_eq!(Field::from_mapping(&f.to_mapping()).unwrap(), f);
    assert_eq!(
        Field::from_bytes(&f.to_bytes()).unwrap(),
        f.clone().without_metadata()
    );
}

#[test]
fn field_merge() {
    let a = Field::new("x", DataType::int(8, true), false).with_metadata_entry("k", "a");
    let b = Field::new("x", DataType::int(32, true), true)
        .with_metadata_entry("k", "b")
        .with_metadata_entry("k2", "c");
    let merged = a.merge(&b, MergeStrategy::Promote).unwrap();
    assert_eq!(merged.data_type(), &DataType::int(32, true));
    assert!(merged.is_nullable());
    assert_eq!(merged.get_metadata("k"), Some("a")); // this field wins
    assert_eq!(merged.get_metadata("k2"), Some("c"));
    // name mismatch errors.
    assert!(matches!(
        a.merge(
            &Field::new("y", DataType::int(8, true), true),
            MergeStrategy::Promote
        ),
        Err(SchemaError::NameMismatch { .. })
    ));
}

#[test]
fn field_children_and_parent_graph() {
    let schema = Field::new(
        "rec",
        DataType::struct_(vec![
            Field::new("Id", DataType::int(64, true), false),
            Field::new("Name", DataType::varchar(), true),
            Field::new(
                "addr",
                DataType::struct_(vec![Field::new("City", DataType::varchar(), true)]),
                true,
            ),
        ]),
        false,
    );
    // child accessors: case-insensitive by name, and by index.
    assert_eq!(schema.child_count(), 3);
    assert_eq!(schema.child("id").unwrap().name(), "Id");
    assert_eq!(schema.child("NAME").unwrap().name(), "Name");
    assert!(schema.child_exact("id").is_none()); // case-sensitive
    assert_eq!(schema.child_index("name"), Some(1));
    assert_eq!(schema.child_at(2).unwrap().name(), "addr");
    assert!(Field::new("x", DataType::int(8, true), true)
        .children()
        .is_empty());
    // parent graph after linking.
    let linked = schema.clone().with_linked_children();
    let addr = linked.child("addr").unwrap();
    assert_eq!(addr.parent().unwrap().name(), "rec");
    let city = addr.child("city").unwrap();
    assert_eq!(city.parent().unwrap().name(), "addr");
    assert_eq!(city.root().name(), "rec"); // walk up to the top
                                           // identity ignores parent (still equal to the unlinked schema).
    assert_eq!(linked, schema);
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trips_structurally() {
    for dt in sample_types() {
        let json = serde_json::to_string(&dt).unwrap();
        assert_eq!(
            serde_json::from_str::<DataType>(&json).unwrap(),
            dt,
            "serde {dt}"
        );
    }
    // A field with metadata is lossless through serde; parent is dropped.
    let f = Field::new("id", DataType::int(64, true), false)
        .with_comment("pk")
        .with_parent(Field::new("root", DataType::struct_(vec![]), false));
    let back: Field = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
    assert_eq!(back, f);
    assert_eq!(back.comment(), Some("pk"));
    assert!(back.parent().is_none());
}

#[cfg(feature = "json")]
#[test]
fn json_helpers_round_trip() {
    let dt = DataType::struct_(vec![Field::new("id", DataType::int(64, true), false)]);
    assert_eq!(DataType::from_json(&dt.to_json()).unwrap(), dt);
    let f = Field::new("id", DataType::int(64, true), false).with_comment("c");
    assert_eq!(Field::from_json(&f.to_json()).unwrap(), f);
}

#[cfg(feature = "arrow")]
#[test]
fn arrow_round_trips_every_concrete_type() {
    for dt in sample_types() {
        if dt.is_any() {
            assert!(dt.to_arrow().is_err());
            continue;
        }
        let arrow = dt
            .to_arrow()
            .unwrap_or_else(|e| panic!("to_arrow {dt}: {e}"));
        assert_eq!(
            DataType::from_arrow(&arrow),
            dt,
            "arrow round-trip for {dt}"
        );
    }
}

#[cfg(feature = "arrow")]
#[test]
fn arrow_maps_every_fixed_numeric_width() {
    use arrow_schema::DataType as A;
    // Every concrete fixed width has a direct Arrow equivalent (no width is rejected).
    assert_eq!(DataType::int8().to_arrow().unwrap(), A::Int8);
    assert_eq!(DataType::int(16, false).to_arrow().unwrap(), A::UInt16);
    assert_eq!(DataType::float16().to_arrow().unwrap(), A::Float16);
    assert_eq!(DataType::float64().to_arrow().unwrap(), A::Float64);
    assert_eq!(
        DataType::decimal32(9, 2).to_arrow().unwrap(),
        A::Decimal32(9, 2)
    );
    assert_eq!(
        DataType::decimal256(76, 0).to_arrow().unwrap(),
        A::Decimal256(76, 0)
    );
    // ... and back.
    assert_eq!(DataType::from_arrow(&A::UInt64), DataType::uint64());
    assert_eq!(DataType::from_arrow(&A::Float16), DataType::float16());
}

#[cfg(feature = "arrow")]
#[test]
fn arrow_field_and_schema() {
    use arrow_schema::DataType as A;
    assert_eq!(DataType::int(64, true).to_arrow().unwrap(), A::Int64);
    assert_eq!(DataType::from_arrow(&A::Utf8), DataType::varchar());
    // A struct field -> Arrow Schema and back.
    let schema = Field::new(
        "rec",
        DataType::struct_(vec![Field::new("id", DataType::int(64, true), false)]),
        false,
    )
    .with_metadata_entry("source", "test");
    let arrow = schema.to_arrow_schema().unwrap();
    assert_eq!(arrow.fields().len(), 1);
    assert_eq!(arrow.metadata().get("source"), Some(&"test".to_string()));
    let back = Field::from_arrow_schema("rec", &arrow, false);
    assert_eq!(back.children().len(), 1);
    assert_eq!(back.get_metadata("source"), Some("test"));
    // a non-struct field cannot become a schema.
    assert!(Field::new("x", DataType::int(8, true), true)
        .to_arrow_schema()
        .is_err());
    // Field metadata survives the field round-trip.
    let f = Field::new("c", DataType::varchar(), true).with_comment("hi");
    assert_eq!(Field::from_arrow(&f.to_arrow().unwrap()), f);
}
