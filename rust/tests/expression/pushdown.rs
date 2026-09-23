//! `rust/src/expression/pushdown.rs`: the edge cases this module is built
//! to get right.
//!
//! Four properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, and a pruning decision never loses a
//! row.

mod grammar {

    use yggdryl::expression::{Bound, Bounds, Term};
    use yggdryl::{DataType, Field, Scalar, StructType, TimeUnit, Timezone};

    // ---------------------------------------------------------------------------
    // The shared fixture
    // ---------------------------------------------------------------------------

    /// A schema that covers one column of every family a comparison can meet.
    fn rows_schema() -> Field {
        Field::new(
            "rows",
            StructType::from_fields([
                Field::new("i", DataType::Int64, true),
                Field::new("f", DataType::Float64, true),
                Field::new("d", DataType::decimal128(9, 2).unwrap(), true),
                Field::new("s", DataType::utf8(), true),
                Field::new("b", DataType::Boolean, true),
                Field::new(
                    "t",
                    DataType::DateTime64 {
                        unit: TimeUnit::Microsecond,
                        timezone: Timezone::UTC,
                    },
                    true,
                ),
                Field::new("n", DataType::Int32, true).with_partition(true),
                Field::new(
                    "nested",
                    DataType::from(
                        StructType::from_fields([Field::new("leg", DataType::utf8(), true)])
                            .unwrap(),
                    ),
                    true,
                ),
                // Temporal text, so a cast into and out of a temporal is one of
                // the pairs the two tiers are compared on.
                Field::new("clock", DataType::utf8(), true),
                // A serie, so a position and a run are compared on both tiers.
                Field::new(
                    "xs",
                    DataType::serie(DataType::Int64.nullable_field("item")),
                    true,
                ),
                // A serie of structs holding a serie of structs, so a predicate
                // segment and one nested in another are compared on both tiers.
                Field::new("legs", DataType::serie(leg_field()), true),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        )
    }

    /// One leg: a currency, a size, and notes that are themselves a serie of
    /// structs.
    fn leg_field() -> Field {
        StructType::from_fields([
            DataType::utf8().nullable_field("ccy"),
            DataType::Int64.nullable_field("size"),
            DataType::serie(
                StructType::from_fields([
                    DataType::utf8().nullable_field("k"),
                    DataType::Int64.nullable_field("v"),
                ])
                .map(DataType::from)
                .unwrap()
                .nullable_field("item"),
            )
            .nullable_field("notes"),
        ])
        .map(DataType::from)
        .unwrap()
        .nullable_field("item")
    }

    fn leg(ccy: Option<&str>, size: Option<i64>, notes: Option<&[(&str, i64)]>) -> Scalar {
        Scalar::from_sequence([
            ccy.map_or(Scalar::Null, Scalar::from),
            size.map_or(Scalar::Null, Scalar::from),
            notes.map_or(Scalar::Null, |notes| {
                Scalar::from_sequence(
                    notes
                        .iter()
                        .map(|(k, v)| Scalar::from_sequence([Scalar::from(*k), Scalar::from(*v)])),
                )
            }),
        ])
    }

    /// Rows chosen so every operator meets a null, a `nan`, and a boundary.
    fn rows() -> Vec<Scalar> {
        let stamp =
            |micros: i64| Scalar::datetime64(micros, TimeUnit::Microsecond, Timezone::UTC).unwrap();
        let nested =
            |leg: Option<&str>| Scalar::from_sequence([leg.map_or(Scalar::Null, Scalar::from)]);
        let serie =
            |items: &[i64]| Scalar::from_sequence(items.iter().map(|item| Scalar::from(*item)));
        vec![
            Scalar::from_sequence([
                Scalar::from(1),
                Scalar::from(1.5_f64),
                Scalar::d128(150, 2),
                Scalar::from("alpha"),
                Scalar::from(true),
                stamp(1_700_000_000_000_000),
                Scalar::from(2024),
                nested(Some("EUR")),
                Scalar::from("10:23:45"),
                serie(&[1, 2, 3]),
                Scalar::from_sequence([
                    leg(Some("EUR"), Some(1), Some(&[("a", 1), ("b", 2)])),
                    leg(Some("USD"), Some(2), Some(&[])),
                    leg(Some("EUR"), Some(3), None),
                ]),
            ]),
            Scalar::from_sequence([
                Scalar::from(-3),
                Scalar::from(f64::NAN),
                Scalar::d128(-25, 2),
                Scalar::from("beta"),
                Scalar::from(false),
                stamp(0),
                Scalar::from(2024),
                nested(None),
                Scalar::from("25:30:00"),
                serie(&[]),
                // An empty serie keeps nothing and is not null.
                Scalar::from_sequence([]),
            ]),
            Scalar::from_sequence([
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::from(2024),
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                // A null serie stays null through every predicate.
                Scalar::Null,
            ]),
            Scalar::from_sequence([
                Scalar::from(100),
                Scalar::from(f64::INFINITY),
                Scalar::d128(10_000, 2),
                Scalar::from("Alpha"),
                Scalar::Null,
                stamp(-1_000_000),
                Scalar::from(2023),
                nested(Some("USD")),
                Scalar::from("99:59:59"),
                serie(&[7]),
                // A null element is dropped; a null size makes a size test unknown.
                Scalar::from_sequence([Scalar::Null, leg(Some("EUR"), None, Some(&[("a", 5)]))]),
            ]),
            Scalar::from_sequence([
                Scalar::from(0),
                Scalar::from(0.0_f64),
                Scalar::d128(0, 2),
                Scalar::from(""),
                Scalar::from(true),
                stamp(1_700_000_000_000_001),
                Scalar::from(2025),
                nested(Some("eur")),
                Scalar::from("00:00:00.500"),
                serie(&[0, -1]),
                Scalar::from_sequence([
                    leg(Some("eur"), Some(10), Some(&[("c", 3)])),
                    leg(Some("GBP"), Some(0), Some(&[("z", 0)])),
                ]),
            ]),
        ]
    }

    /// The predicates the two tiers are compared on, all evaluable per row.
    const AGREEMENT: [&str; 33] = [
        "i = 1",
        "i <> 1",
        "i < 0",
        "i >= 100",
        "i is null",
        "i is not null",
        "i is distinct from 1",
        "i is not distinct from null",
        "f = f",
        "f > 1.0",
        "f is null",
        "d > decimal128(9,2) '1.00'",
        "d <= 0",
        "s = 'alpha'",
        "s like 'a%'",
        "s ilike 'A%'",
        "b",
        "t > timestamp(microsecond,'UTC') '2023-11-14T22:13:20Z'",
        "i in (1, 100)",
        "i between 0 and 100",
        "i = 1 or s = 'beta'",
        "not (i = 1) and s is not null",
        "i = 1 or i = 100 or i = 0",
        "not (i in (1, 100) and s like 'a%')",
        "xs[0] = 1",
        // Text entering a temporal and a temporal leaving as text: the vectorized
        // tier reads and spells with the code the row tier reads and spells with,
        // a zone name and an hour past the end of the day included.
        "cast(clock as time64(microsecond))",
        "cast(clock as duration64(millisecond))",
        "cast(t as string)",
        "try_cast(clock as time32(second))",
        // A predicate segment inside a comparison: the kept serie is counted,
        // an empty match is zero, and a null serie is unknown.
        "size(legs[ccy = 'EUR']) > 1",
        "legs[size > 1][-1].ccy = 'EUR'",
        "legs[notes[v > 1][0].k = 'b'][0].size = 1",
        "size(legs[true]) = size(legs)",
    ];

    /// The statistics the fixture rows actually have, computed rather than guessed.
    fn bounds_of(schema: &Field, rows: &[Scalar]) -> Bounds {
        let mut bounds = Bounds::new(Some(rows.len() as u64));
        for (index, field) in schema.fields().iter().enumerate() {
            let mut minimum: Option<Scalar> = None;
            let mut maximum: Option<Scalar> = None;
            let mut nulls = 0_u64;
            for row in rows {
                let value = &row.as_sequence().unwrap()[index];
                if value.is_null() {
                    nulls += 1;
                    continue;
                }
                let ordered = |held: &Option<Scalar>, keep_greater: bool| match held {
                    None => Some(value.clone()),
                    // The fixture orders values the way the scalar itself does;
                    // the rows below hold one width per column, so the datatype
                    // directed ordering the pushdown uses agrees with it.
                    Some(held) => match Some(value.cmp(held)) {
                        Some(std::cmp::Ordering::Greater) if keep_greater => Some(value.clone()),
                        Some(std::cmp::Ordering::Less) if !keep_greater => Some(value.clone()),
                        _ => None,
                    },
                };
                if let Some(next) = ordered(&minimum, false) {
                    minimum = Some(next);
                }
                if let Some(next) = ordered(&maximum, true) {
                    maximum = Some(next);
                }
            }
            bounds = bounds.with_column(field.name(), minimum, maximum, Some(nulls));
        }
        bounds
    }

    #[test]
    fn a_predicate_segment_costs_a_decode_and_prunes_nothing() {
        let schema = rows_schema();
        let rows = rows();
        // Cheapest first: a bare column comparison runs before the decode of a
        // serie, whichever order they were written in.
        let bound = "size(legs[ccy = 'EUR']) > 1 and i = 1"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(
            bound.term().to_string(),
            "i = 1 and size(legs[ccy = 'EUR']) > 1"
        );
        // Every statistic is known and tight, and still nothing about a
        // predicate segment is proven either way.
        let bounds = bounds_of(&schema, &rows);
        assert_eq!(bound.statistics_certainty(&bounds), None);
        assert!(bound.statistics_prune(&bounds));
        let only = "size(legs[ccy = 'EUR']) > 1"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(only.statistics_certainty(&bounds), None);
        assert!(only.reads_rows());
        let split = only.partition_split();
        assert!(split.is_empty());
        assert!(split.remaining().to_string().contains("legs[ccy = 'EUR']"));
    }

    // ---------------------------------------------------------------------------
    // Pushdown
    // ---------------------------------------------------------------------------

    #[test]
    fn pruning_never_loses_a_row() {
        let schema = rows_schema();
        let rows = rows();
        let bounds = bounds_of(&schema, &rows);
        for text in AGREEMENT {
            let bound: Bound = text.parse::<Term>().unwrap().bind(&schema).unwrap();
            if !bound.is_predicate() {
                continue;
            }
            let any_matches = rows.iter().any(|row| bound.matches(row).unwrap());
            if any_matches {
                assert!(
                    bound.statistics_prune(&bounds),
                    "{text} pruned a container that holds a matching row"
                );
            }
        }
    }

    #[test]
    fn pruning_actually_prunes_what_it_can_prove() {
        let schema = rows_schema();
        let bounds = bounds_of(&schema, &rows());
        for text in [
            "i > 1000",
            "i < -1000",
            "n = 1999",
            "s > 'zzz'",
            "n = 1999 or n = 1998",
            "not (n >= 2000)",
        ] {
            let bound = text.parse::<Term>().unwrap().bind(&schema).unwrap();
            assert!(
                !bound.statistics_prune(&bounds),
                "{text} should have been provably empty"
            );
        }
    }

    #[test]
    fn an_answer_for_every_row_counts_the_rows_that_hold_null() {
        let schema = |nullable: bool| {
            Field::new(
                "rows",
                StructType::from_fields([Field::new("x", DataType::Int64, nullable)])
                    .map(DataType::from)
                    .unwrap(),
                false,
            )
        };
        // Three rows whose one non-null value is 1.
        let certainty = |nullable: bool, text: &str, nulls: Option<u64>| {
            let bounds = Bounds::new(Some(3)).with_column(
                "x",
                Some(Scalar::from(1_i64)),
                Some(Scalar::from(1_i64)),
                nulls,
            );
            text.parse::<Term>()
                .unwrap()
                .bind(&schema(nullable))
                .unwrap()
                .statistics_certainty(&bounds)
        };
        // Every row holds the one value only when no row is null.
        assert_eq!(certainty(true, "x = 1", Some(0)), Some(true));
        assert_eq!(certainty(true, "x = 1", None), None);
        assert_eq!(certainty(true, "x = 1", Some(1)), None);
        assert_eq!(certainty(true, "x is not distinct from 1", Some(1)), None);
        // A required column holds no null, whatever the count says.
        assert_eq!(certainty(false, "x = 1", None), Some(true));
        // A null row is distinct from every value.
        assert_eq!(
            certainty(true, "x is distinct from 1", Some(0)),
            Some(false)
        );
        assert_eq!(certainty(true, "x is distinct from 1", Some(1)), None);
        assert_eq!(certainty(true, "x is distinct from 1", None), None);
        // No row above five leaves the null rows unknown, not true.
        assert_eq!(certainty(true, "not (x > 5)", Some(1)), None);
        assert_eq!(certainty(true, "not (x = 1)", Some(0)), Some(false));
    }

    #[test]
    fn a_partition_path_is_the_tightest_statistic_there_is() {
        let schema = rows_schema();
        let partitions = vec![("n".to_owned(), "2024".to_owned())];
        let bounds = Bounds::from_partitions(&schema, &partitions);
        let keep = "n = 2024".parse::<Term>().unwrap().bind(&schema).unwrap();
        let skip = "n = 2023".parse::<Term>().unwrap().bind(&schema).unwrap();
        assert!(keep.statistics_prune(&bounds));
        assert!(!skip.statistics_prune(&bounds));
    }

    #[test]
    fn cheapest_first_is_stable_when_costs_tie() {
        let schema = rows_schema();
        let bound = "s = 'a' and i = 1"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(bound.term().to_string(), "s = 'a' and i = 1");
    }
}
