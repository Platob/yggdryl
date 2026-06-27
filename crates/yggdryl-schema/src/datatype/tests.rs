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
        DataType::varchar_with(Charset::Utf8, true, false),
        DataType::varchar_with(Charset::Utf8, false, true),
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
    assert_eq!(DataType::from_str("int").unwrap(), DataType::int(64, true));
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
        DataType::varchar_with(Charset::Utf8, true, false).to_str(),
        "large_utf8"
    );
    let latin = DataType::varchar_with(Charset::Latin1, false, false);
    assert_eq!(latin.to_str(), "varchar[latin1]");
    assert_eq!(DataType::from_str("varchar[latin1]").unwrap(), latin);
    let big_latin = DataType::varchar_with(Charset::Latin1, true, false);
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
    assert!(DataType::varchar_with(Charset::Utf8, true, false).is_large());
    assert!(DataType::varchar_with(Charset::Utf8, false, true).is_view());
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
        D::varchar().common_type(&D::varchar_with(Charset::Utf8, true, false)),
        Some(D::varchar_with(Charset::Utf8, true, false))
    );
    // Differing charsets do not unify.
    assert_eq!(
        D::varchar().common_type(&D::varchar_with(Charset::Latin1, false, false)),
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
