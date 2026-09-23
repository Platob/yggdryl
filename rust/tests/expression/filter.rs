//! `rust/src/expression/filter.rs`: the edge cases this module is built to
//! get right.
//!
//! Five properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, a free attribute never costs a backend
//! call, and a pruning decision never loses a row.

mod grammar {

    use yggdryl::expression::{Expression, Filter, Term};
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
                // A list, so a position and a run are compared on both tiers.
                Field::new(
                    "xs",
                    DataType::list(DataType::Int64.nullable_field("item")),
                    true,
                ),
                // A list of structs holding a list of structs, so a predicate
                // segment and one nested in another are compared on both tiers.
                Field::new("legs", DataType::list(leg_field()), true),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        )
    }

    /// One leg: a currency, a size, and notes that are themselves a list of
    /// structs.
    fn leg_field() -> Field {
        StructType::from_fields([
            DataType::utf8().nullable_field("ccy"),
            DataType::Int64.nullable_field("size"),
            DataType::list(
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
        let list =
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
                list(&[1, 2, 3]),
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
                list(&[]),
                // An empty list keeps nothing and is not null.
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
                // A null list stays null through every predicate.
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
                list(&[7]),
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
                list(&[0, -1]),
                Scalar::from_sequence([
                    leg(Some("eur"), Some(10), Some(&[("c", 3)])),
                    leg(Some("GBP"), Some(0), Some(&[("z", 0)])),
                ]),
            ]),
        ]
    }

    fn batch_of(schema: &Field, rows: &[Scalar]) -> arrow_array::RecordBatch {
        let arrow_schema = schema.clone().into_arrow_schema().unwrap();
        let columns = schema
            .fields()
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let values: Vec<Scalar> = rows
                    .iter()
                    .map(|row| row.as_sequence().unwrap()[index].clone())
                    .collect();
                yggdryl::arrow::array_from_value(field, &yggdryl::Scalar::from_sequence(values))
                    .unwrap()
            })
            .collect();
        arrow_array::RecordBatch::try_new(arrow_schema, columns).unwrap()
    }

    // ---------------------------------------------------------------------------
    // Filters
    // ---------------------------------------------------------------------------

    #[test]
    fn a_filter_keeps_the_rows_it_answers_true_for() {
        let schema = rows_schema();
        let rows = rows();
        let batch = batch_of(&schema, &rows);
        let filter: Filter = "i > 0 or s = ''".parse().unwrap();
        assert_eq!(filter.apply_field(&schema).unwrap(), schema);
        let kept = filter.apply_arrow_batch(&batch).unwrap();
        assert_eq!(kept.num_rows(), 3);
        let expected: Vec<bool> = rows
            .iter()
            .map(|row| filter.apply_scalar(&schema, row).unwrap())
            .collect();
        assert_eq!(expected, vec![true, false, false, true, true]);
        // A filter over a bare batch binds against the batch's own schema.
        let expression: Expression = "where i is null".parse().unwrap();
        assert_eq!(expression.apply_arrow_batch(&batch).unwrap().num_rows(), 1);
        assert_eq!(expression.apply_field(&schema).unwrap(), schema);
    }

    #[test]
    fn a_filter_has_to_be_a_predicate() {
        let schema = rows_schema();
        let filter: Filter = "i + 1".parse().unwrap();
        let error = filter.apply_field(&schema).unwrap_err().to_string();
        assert!(error.contains("boolean"), "{error}");
        assert!(filter.bind(&schema).is_err());
        assert!(Filter::always_true().is_always_true());
        assert!(Filter::always_false().is_always_false());
        assert!(Filter::all([]).is_always_true());
        assert!(Filter::any([]).is_always_false());
    }

    #[test]
    fn partition_equal_filters_keep_literal_dotted_and_reserved_column_names_typed() {
        let schema = StructType::from_fields([
            DataType::Int64.required_field("a.b"),
            DataType::Int64.required_field("null"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let bound = Filter::all_partitions_equal(&schema, [("a.b", "2"), ("null", "3")])
            .bind(&schema)
            .unwrap();

        // Both names are literal fields, and each directory spelling is cast once
        // through its declared integer type before it filters the rows.
        assert!(
            bound
                .matches(&Scalar::from_sequence([
                    Scalar::from(2_i64),
                    Scalar::from(3_i64)
                ]))
                .unwrap()
        );
        assert!(
            !bound
                .matches(&Scalar::from_sequence([
                    Scalar::from(99_i64),
                    Scalar::from(3_i64)
                ]))
                .unwrap()
        );
        assert!(
            !bound
                .matches(&Scalar::from_sequence([
                    Scalar::from(2_i64),
                    Scalar::from(99_i64)
                ]))
                .unwrap()
        );
    }

    #[test]
    fn a_split_conjoins_back_to_what_it_split() {
        let schema = rows_schema();
        let bound = "n = 2024 and i > 1 and s like 'a%'"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let residual = bound.partition_split();
        assert_eq!(residual.answerable().to_string(), "n = int32 '2024'");
        assert!(!residual.is_complete());
        let rejoined = residual
            .answerable()
            .clone()
            .and(residual.remaining().clone());
        let mut left: Vec<Term> = rejoined
            .conjuncts()
            .into_iter()
            .map(Filter::into_term)
            .collect();
        let mut right = bound.term().conjuncts();
        left.sort();
        right.sort();
        assert_eq!(left, right);
    }
}
