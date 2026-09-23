//! `rust/src/expression/selector.rs`: the edge cases this module is built
//! to get right.
//!
//! Four properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, and a pruning decision never loses a
//! row.

mod grammar {

    use std::sync::Arc;

    use yggdryl::expression::Selector;
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
                yggdryl::Serie::from_scalars(field.clone(), values)
                    .unwrap()
                    .require_arrow_array()
                    .unwrap()
            })
            .collect();
        arrow_array::RecordBatch::try_new(arrow_schema, columns).unwrap()
    }

    // ---------------------------------------------------------------------------
    // Selectors
    // ---------------------------------------------------------------------------

    #[test]
    fn a_selector_publishes_the_field_it_computes() {
        let schema = rows_schema();
        let selector: Selector = "i, i + 1 as next, s as name string not null, nested.leg"
            .parse()
            .unwrap();
        let output = selector.apply_field(&schema).unwrap();
        assert_eq!(output.name(), "rows");
        let names: Vec<&str> = output.fields().iter().map(Field::name).collect();
        assert_eq!(names, vec!["i", "next", "name", "leg"]);
        assert_eq!(output.fields()[1].dtype(), &DataType::Int64);
        assert!(!output.fields()[2].is_nullable());
        assert_eq!(output.fields()[3].dtype(), &DataType::utf8());
        assert!(output.fields()[3].is_nullable());
        assert_eq!(selector.columns(), vec!["i", "s", "nested"]);
        assert_eq!(Selector::all().apply_field(&schema).unwrap(), schema);

        let twice: Selector = "i, i".parse().unwrap();
        let error = twice.apply_field(&schema).unwrap_err().to_string();
        assert!(error.contains("twice"), "{error}");
        let missing: Selector = "nope".parse().unwrap();
        assert!(missing.apply_field(&schema).is_err());
    }

    #[test]
    fn a_selector_computes_rows_on_both_tiers() {
        let schema = rows_schema();
        let rows = rows();
        let batch = batch_of(&schema, &rows);
        let selector: Selector =
            "i * 2 as doubled, coalesce(s, '-') as name string not null, n int64"
                .parse()
                .unwrap();
        let projected = selector.apply_arrow_batch(&batch).unwrap();
        let output = selector.apply_field(&schema).unwrap();
        assert_eq!(
            projected.schema().as_ref(),
            output.clone().into_arrow_schema().unwrap().as_ref()
        );
        let bound = selector.bind(&schema).unwrap();
        for (position, row) in rows.iter().enumerate() {
            let scalar = bound.apply_scalar(row).unwrap();
            let held: Vec<Scalar> = output
                .fields()
                .iter()
                .zip(projected.columns())
                .map(|(field, column)| {
                    yggdryl::Serie::from_arrow_array(
                        Some(&field.clone().with_nullable(true)),
                        column.slice(position, 1),
                        yggdryl::ArrowCastOptions::default(),
                    )
                    .unwrap()
                    .scalar(0)
                    .unwrap()
                })
                .collect();
            assert_eq!(scalar, Scalar::from_sequence(held), "row {position}");
        }
        // The declared type is what the column carries, on both tiers.
        assert_eq!(
            bound.apply_scalar(&rows[0]).unwrap(),
            Scalar::from_sequence([
                Scalar::from(2_i64),
                Scalar::from("alpha"),
                Scalar::from(2024_i64)
            ])
        );
    }

    #[test]
    fn a_projection_reorders_without_touching_a_buffer() {
        let schema = rows_schema();
        let batch = batch_of(&schema, &rows());
        let selector: Selector = "select s, i".parse().unwrap();
        let projected = selector.apply_arrow_batch(&batch).unwrap();
        assert_eq!(projected.num_columns(), 2);
        assert!(Arc::ptr_eq(projected.column(0), batch.column(3)));
        assert!(Arc::ptr_eq(projected.column(1), batch.column(0)));
        let everything = Selector::all().apply_arrow_batch(&batch).unwrap();
        for (left, right) in batch.columns().iter().zip(everything.columns()) {
            assert!(Arc::ptr_eq(left, right));
        }
    }

    #[test]
    fn a_declared_column_is_enforced_when_it_is_applied() {
        let schema = rows_schema();
        let rows = rows();
        let batch = batch_of(&schema, &rows);
        // A required column that computes a null is refused by name.
        let required: Selector = "s as name string not null".parse().unwrap();
        let error = required.apply_arrow_batch(&batch).unwrap_err().to_string();
        assert!(error.contains("name"), "{error}");
        assert!(required.apply_scalar(&schema, &rows[2]).is_err());
        assert_eq!(
            required.apply_scalar(&schema, &rows[0]).unwrap(),
            Scalar::from_sequence([Scalar::from("alpha")])
        );
        // A declared type the value does not fit answers null - the best-effort
        // reading - unless the column is required, where the value is refused
        // by name.
        let narrow: Selector = "n as small int8".parse().unwrap();
        assert_eq!(
            narrow
                .apply_arrow_batch(&batch)
                .unwrap()
                .column(0)
                .null_count(),
            batch.num_rows()
        );
        assert_eq!(
            narrow.apply_scalar(&schema, &rows[0]).unwrap(),
            Scalar::from_sequence([Scalar::Null])
        );
        let required: Selector = "n as small int8 not null".parse().unwrap();
        let error = required.apply_arrow_batch(&batch).unwrap_err().to_string();
        assert!(error.contains("small"), "{error}");
        assert!(required.apply_scalar(&schema, &rows[0]).is_err());
        let fits: Selector = "i as small int8".parse().unwrap();
        assert_eq!(
            fits.apply_scalar(&schema, &rows[0]).unwrap(),
            Scalar::from_sequence([Scalar::from(1_i8)])
        );
    }

    #[test]
    fn a_bound_selector_casts_every_batch_as_a_fresh_bind_does() {
        // One bound selector holds one cast per declared projection across a
        // stream: every batch answers what a bind made for that batch alone
        // answers, a value the column cannot hold and a refusal included.
        let schema = rows_schema();
        let rows = rows();
        let batches: Vec<_> = rows
            .chunks(2)
            .map(|chunk| batch_of(&schema, chunk))
            .collect();
        let same = |held: yggdryl::Result<arrow_array::RecordBatch>,
                    fresh: yggdryl::Result<arrow_array::RecordBatch>,
                    context: &str| match (held, fresh) {
            (Ok(held), Ok(fresh)) => assert_eq!(held, fresh, "{context}"),
            (Err(held), Err(fresh)) => assert_eq!(held.to_string(), fresh.to_string(), "{context}"),
            (held, fresh) => panic!("{context}: {held:?} and {fresh:?}"),
        };
        for text in [
            "i as small int8 not null",
            "n as small int8",
            "f as whole int32",
            "i * 2 as doubled int8, s as name string not null",
        ] {
            let selector: Selector = text.parse().unwrap();
            let bound = selector.bind(&schema).unwrap();
            for (position, batch) in batches.iter().enumerate() {
                same(
                    bound.apply_arrow_batch(batch),
                    selector.bind(&schema).unwrap().apply_arrow_batch(batch),
                    &format!("{text}, batch {position}"),
                );
            }
            let stream = yggdryl::arrow::batch_reader(batches[0].schema(), batches.clone());
            let streamed = selector.apply_arrow_reader(stream).unwrap();
            for (position, (held, batch)) in streamed.zip(&batches).enumerate() {
                same(
                    held.map_err(|error| yggdryl::arrow::from_reader_error(error).into()),
                    selector.apply_arrow_batch(batch),
                    &format!("{text}, streamed batch {position}"),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Intake from a column
// ---------------------------------------------------------------------------

mod intake {

    use yggdryl::expression::{Projection, Selector};
    use yggdryl::{DataType, Field, Scalar, Serie};

    /// A utf8 column of `texts`, the shape a binding's columnar input lands as.
    fn texts(texts: &[&str]) -> Scalar {
        Scalar::from(
            Serie::from_scalars(
                Field::new("item", DataType::utf8(), false),
                texts.iter().map(|text| Scalar::from(*text)),
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_term_alias_pair_reads_the_same_from_a_column() {
        assert_eq!(
            Projection::from_scalar(&texts(&["price", "p"])).unwrap(),
            "price as p".parse::<Projection>().unwrap()
        );
    }

    #[test]
    fn a_column_of_projections_reads_as_the_run_does() {
        let spelled = ["i", "i + 1 as next"];
        let run = Scalar::from_sequence(spelled.iter().map(|text| Scalar::from(*text)));
        assert_eq!(
            Selector::from_scalar(&texts(&spelled)).unwrap(),
            Selector::from_scalar(&run).unwrap()
        );
    }
}
