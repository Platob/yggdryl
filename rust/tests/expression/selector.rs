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
        // A null crossing into a required column of another datatype is
        // refused by both tiers, as one of the column's own datatype is,
        // rather than repaired to the canonical default by the cast.
        let crossing: Selector = "i as wide int32 not null".parse().unwrap();
        let error = crossing.apply_arrow_batch(&batch).unwrap_err().to_string();
        assert!(error.contains("wide"), "{error}");
        assert!(crossing.apply_scalar(&schema, &rows[2]).is_err());
        // So is an empty text cell, which is a null before it is read.
        let empty: Selector = "s as count int64 not null".parse().unwrap();
        let only_empty = batch_of(&schema, &rows[4..]);
        let error = empty
            .apply_arrow_batch(&only_empty)
            .unwrap_err()
            .to_string();
        assert!(error.contains("count"), "{error}");
        assert!(empty.apply_scalar(&schema, &rows[4]).is_err());
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

// ---------------------------------------------------------------------------
// A `*` with appended projections
// ---------------------------------------------------------------------------

mod star {

    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
    use yggdryl::expression::{Projection, Selector};
    use yggdryl::{DataType, Field, StructType};

    fn schema() -> Field {
        StructType::from_fields([
            DataType::Int64.required_field("a"),
            DataType::utf8().nullable_field("b"),
            DataType::utf8().nullable_field("x"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("rows")
    }

    fn batch() -> RecordBatch {
        RecordBatch::try_new(
            schema().into_arrow_schema().unwrap(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
                Arc::new(StringArray::from(vec![Some("p"), None])) as ArrayRef,
                Arc::new(StringArray::from(vec![Some("q"), Some("r")])) as ArrayRef,
            ],
        )
        .unwrap()
    }

    fn names(field: &Field) -> Vec<&str> {
        field.fields().iter().map(Field::name).collect()
    }

    #[test]
    fn a_star_carries_its_exclusions_and_appended_projections_through_every_spelling() {
        for (text, canonical) in [
            (
                "* exclude (a, b), securityids['ISIN'] as isin, upper(x) as y",
                "* exclude (a, b), securityids['ISIN'] as isin, upper(x) as y",
            ),
            ("* except (a), x as z", "* exclude (a), x as z"),
            ("*, upper(x) as y", "*, upper(x) as y"),
            ("* exclude (a)", "* exclude (a)"),
            ("*", "*"),
            ("a, x", "a, x"),
        ] {
            let selector: Selector = text
                .parse()
                .unwrap_or_else(|error| panic!("{text}: {error}"));
            assert_eq!(selector.to_string(), canonical, "{text}");
            assert_eq!(canonical.parse::<Selector>().unwrap(), selector, "{text}");
            let document = selector.clone().into_json().unwrap();
            assert_eq!(
                Selector::from_json(&document).unwrap(),
                selector,
                "{text}: {document}"
            );
            let clause: yggdryl::Expression = format!("select {text}").parse().unwrap();
            assert_eq!(clause.to_string(), format!("select {canonical}"), "{text}");
        }
        // The document writes each part only when it says something.
        assert_eq!(Selector::all().into_json().unwrap(), r#"{"star":true}"#);
        assert_eq!(Selector::from_json("{}").unwrap(), Selector::all());
        assert_eq!(
            Selector::all_except(["a"]).into_json().unwrap(),
            r#"{"star":true,"exclude":["a"]}"#
        );
    }

    #[test]
    fn an_exclusion_without_a_star_is_refused_and_a_trailing_comma_too() {
        let error = Selector::from_json(r#"{"exclude":["a"]}"#)
            .unwrap_err()
            .to_string();
        assert!(error.contains("exclude"), "{error}");
        for text in ["*,", "* exclude (a),", "* exclude ()", "a, *"] {
            assert!(text.parse::<Selector>().is_err(), "{text} parsed");
        }
    }

    #[test]
    fn a_star_answers_what_it_can_name_and_says_it_reads_everything() {
        let starred: Selector = "* exclude (a), upper(x) as y".parse().unwrap();
        assert!(starred.has_star());
        assert!(!starred.is_all());
        assert_eq!(starred.excluded(), ["a"]);
        assert_eq!(starred.len(), 1);
        assert_eq!(starred.names(), ["y"]);
        assert_eq!(starred.columns(), ["x"]);
        assert!(Selector::all().is_all() && Selector::all().has_star());
        assert!(Selector::all_except(["a"]).has_star());
        assert!(!Selector::all_except(["a"]).is_all());
        let plain: Selector = "a, x".parse().unwrap();
        assert!(!plain.has_star() && !plain.is_all());
        // The empty list is `*`, and nothing else spells it.
        assert_eq!(Selector::new([]), Selector::all());
        assert_eq!(
            Selector::from_columns::<[&str; 0], &str>([]),
            Selector::all()
        );
        assert_eq!(plain.without_columns(&["a", "x"]), Selector::all());
        let plan: yggdryl::Plan = "select * exclude (a) where b = 'p'".parse().unwrap();
        assert_eq!(plan.read_columns(), None, "a star reads every column");
        let plan: yggdryl::Plan = "select a where b = 'p'".parse().unwrap();
        assert_eq!(
            plan.read_columns(),
            Some(vec!["b".to_owned(), "a".to_owned()])
        );
    }

    #[test]
    fn an_appended_projection_keeps_the_star_and_its_exclusions() {
        let appended = Selector::all_except(["a"]).with_projection(Projection::column("b"));
        assert_eq!(appended.to_string(), "* exclude (a), b");
        assert_eq!(
            Selector::all()
                .with_projection("upper(x) as y".parse().unwrap())
                .to_string(),
            "*, upper(x) as y"
        );
        let plain: Selector = "a".parse().unwrap();
        assert_eq!(
            plain.with_projection(Projection::column("x")).to_string(),
            "a, x"
        );
        let kept = appended.without_columns(&["b"]);
        assert_eq!(kept, Selector::all_except(["a"]));
    }

    #[test]
    fn a_star_publishes_what_it_keeps_then_what_it_appends() {
        let schema = schema();
        let selector: Selector = "* exclude (B), upper(x) as y".parse().unwrap();
        assert_eq!(
            names(&selector.apply_field(&schema).unwrap()),
            ["a", "x", "y"]
        );
        // A name the schema does not hold excludes nothing.
        let selector: Selector = "* exclude (nothing), a as c".parse().unwrap();
        assert_eq!(
            names(&selector.apply_field(&schema).unwrap()),
            ["a", "b", "x", "c"]
        );
        // Two outputs of one name are refused, a kept column included.
        let twice: Selector = "*, upper(x) as a".parse().unwrap();
        let error = twice.apply_field(&schema).unwrap_err().to_string();
        assert!(error.contains("twice"), "{error}");

        let batch = batch();
        let selector: Selector = "* exclude (b), upper(x) as y".parse().unwrap();
        let out = selector.apply_arrow_batch(&batch).unwrap();
        let published: Vec<&str> = out
            .schema_ref()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        assert_eq!(published, ["a", "x", "y"]);
        assert!(Arc::ptr_eq(out.column(0), batch.column(0)));
        assert!(Arc::ptr_eq(out.column(1), batch.column(2)));
        let upper = out
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(upper.iter().collect::<Vec<_>>(), [Some("Q"), Some("R")]);
    }
}

// ---------------------------------------------------------------------------
// `unnest`: the one projection that multiplies rows
// ---------------------------------------------------------------------------

mod unnest {

    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
    use yggdryl::expression::{Filter, Plan, Selector, Term};
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};

    fn leg_item() -> Field {
        StructType::from_fields([
            DataType::utf8().nullable_field("ccy"),
            DataType::Int64.nullable_field("size"),
        ])
        .map(DataType::from)
        .unwrap()
        .nullable_field("item")
    }

    fn schema() -> Field {
        let int = || DataType::Int64.nullable_field("item");
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("tag"),
            DataType::serie(int()).nullable_field("xs"),
            DataType::serie(leg_item()).nullable_field("legs"),
            DataType::serie_view(int()).nullable_field("views"),
            DataType::large_serie(int()).nullable_field("large"),
            DataType::fixed_size_serie(int(), 2)
                .unwrap()
                .nullable_field("fixed"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("rows")
    }

    fn ints(items: &[Option<i64>]) -> Scalar {
        Scalar::from_sequence(
            items
                .iter()
                .map(|item| item.map_or(Scalar::Null, Scalar::from)),
        )
    }

    fn leg(ccy: &str, size: Option<i64>) -> Scalar {
        Scalar::from_sequence([Scalar::from(ccy), size.map_or(Scalar::Null, Scalar::from)])
    }

    /// Four rows: runs of several elements, an empty run, a null run, a null
    /// element, and a null struct element.
    fn rows() -> Vec<Scalar> {
        vec![
            Scalar::from_sequence([
                Scalar::from(1_i64),
                Scalar::from("a"),
                ints(&[Some(1), Some(2), Some(3)]),
                Scalar::from_sequence([leg("EUR", Some(1)), leg("USD", Some(2))]),
                ints(&[Some(10), Some(20)]),
                ints(&[Some(5)]),
                ints(&[Some(1), Some(2)]),
            ]),
            Scalar::from_sequence([
                Scalar::from(2_i64),
                Scalar::from("b"),
                ints(&[]),
                Scalar::Null,
                ints(&[]),
                ints(&[]),
                Scalar::Null,
            ]),
            Scalar::from_sequence([
                Scalar::from(3_i64),
                Scalar::Null,
                Scalar::Null,
                Scalar::from_sequence([Scalar::Null, leg("GBP", None)]),
                Scalar::Null,
                Scalar::Null,
                ints(&[Some(3), Some(4)]),
            ]),
            Scalar::from_sequence([
                Scalar::from(4_i64),
                Scalar::from("d"),
                ints(&[None, Some(7)]),
                Scalar::from_sequence([]),
                ints(&[Some(30)]),
                ints(&[Some(6), Some(7)]),
                ints(&[Some(5), None]),
            ]),
        ]
    }

    fn batch() -> RecordBatch {
        let schema = schema();
        let rows = rows();
        let columns = schema
            .fields()
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let values: Vec<Scalar> = rows
                    .iter()
                    .map(|row| row.as_sequence().unwrap()[index].clone())
                    .collect();
                Serie::from_scalars(field.clone(), values)
                    .unwrap()
                    .require_arrow_array()
                    .unwrap()
            })
            .collect();
        RecordBatch::try_new(schema.into_arrow_schema().unwrap(), columns).unwrap()
    }

    fn names(field: &Field) -> Vec<&str> {
        field.fields().iter().map(Field::name).collect()
    }

    fn column<'batch>(batch: &'batch RecordBatch, name: &str) -> &'batch ArrayRef {
        batch
            .column_by_name(name)
            .unwrap_or_else(|| panic!("no column {name}"))
    }

    fn int64s(array: &ArrayRef) -> Vec<Option<i64>> {
        array
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .iter()
            .collect()
    }

    fn texts(array: &ArrayRef) -> Vec<Option<&str>> {
        array
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .iter()
            .collect()
    }

    fn drained(reader: yggdryl::arrow::BatchReader) -> yggdryl::Result<RecordBatch> {
        let schema = reader.schema();
        let mut batches = Vec::new();
        for batch in reader {
            batches.push(batch.map_err(yggdryl::arrow::from_reader_error)?);
        }
        Ok(arrow_select::concat::concat_batches(&schema, &batches).unwrap())
    }

    fn apply(text: &str) -> RecordBatch {
        let plan: Plan = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let reader = yggdryl::arrow::batch_reader(batch().schema(), [batch()]);
        drained(plan.apply_arrow_reader(reader).unwrap())
            .unwrap_or_else(|error| panic!("{text}: {error}"))
    }

    #[test]
    fn unnest_anywhere_but_the_whole_term_of_a_projection_is_refused() {
        let schema = schema();
        for text in [
            "unnest(xs) + 1 as y",
            "coalesce(unnest(xs), 0)",
            "[unnest(xs)] as ys",
        ] {
            let selector: Selector = text.parse().unwrap();
            let error = selector.apply_field(&schema).unwrap_err().to_string();
            assert!(error.contains("select-list form"), "{text}: {error}");
            assert!(error.contains("unnest(xs)"), "{text}: {error}");
        }
        let filter: Filter = "unnest(xs) > 1".parse().unwrap();
        let error = filter.bind(&schema).unwrap_err().to_string();
        assert!(error.contains("select-list form"), "{error}");
        let term: Term = "unnest(xs)".parse().unwrap();
        let error = term.bind(&schema).unwrap_err().to_string();
        assert!(error.contains("select-list form"), "{error}");
        let plan: Plan = "select id order by unnest(xs)".parse().unwrap();
        let reader = yggdryl::arrow::batch_reader(batch().schema(), [batch()]);
        let error = plan
            .apply_arrow_reader(reader)
            .and_then(drained)
            .unwrap_err()
            .to_string();
        assert!(error.contains("select-list form"), "{error}");
        // A column declaration is not a select list either.
        let plan: Plan = "create (unnest(xs) as x)".parse().unwrap();
        let error = plan.field_from(&schema).unwrap_err().to_string();
        assert!(error.contains("select-list form"), "{error}");
    }

    #[test]
    fn one_unnest_per_select_over_a_serie_and_nothing_else() {
        let schema = schema();
        let twice: Selector = "unnest(xs) as x, unnest(legs) as leg".parse().unwrap();
        let error = twice.apply_field(&schema).unwrap_err().to_string();
        assert!(
            error.contains("unnest(xs)") && error.contains("unnest(legs)"),
            "{error}"
        );
        let scalar: Selector = "unnest(id)".parse().unwrap();
        let error = scalar.apply_field(&schema).unwrap_err().to_string();
        assert!(
            error.contains("serie") && error.contains("int64"),
            "{error}"
        );
        // The grammar reads one argument, and a tree built by hand with two
        // is refused where it is typed.
        let error = "unnest(xs, legs)"
            .parse::<Selector>()
            .unwrap_err()
            .to_string();
        assert!(error.contains("1 argument"), "{error}");
        let two = Selector::new([yggdryl::expression::Projection::new(Term::call(
            yggdryl::expression::Function::Unnest,
            [Term::column("xs"), Term::column("legs")],
        ))]);
        let error = two.apply_field(&schema).unwrap_err().to_string();
        assert!(error.contains("one serie"), "{error}");
        // One row in cannot answer the rows an unnest publishes.
        let selector: Selector = "id, unnest(xs) as x".parse().unwrap();
        let bound = selector.bind(&schema).unwrap();
        let error = bound.apply_scalar(&rows()[0]).unwrap_err().to_string();
        assert!(error.contains("unnest"), "{error}");
    }

    #[test]
    fn a_struct_item_publishes_one_column_per_child_and_any_other_item_one() {
        let schema = schema();
        for (text, expected) in [
            (
                "id, unnest(legs) as leg, tag",
                vec!["id", "leg.ccy", "leg.size", "tag"],
            ),
            ("unnest(legs)", vec!["legs.ccy", "legs.size"]),
            ("unnest(xs)", vec!["xs"]),
            ("explode(xs) as x", vec!["x"]),
            ("unnest(xs) as x int32", vec!["x"]),
            (
                "* exclude (xs, legs, views, large, fixed), unnest(xs) as x",
                vec!["id", "tag", "x"],
            ),
        ] {
            let selector: Selector = text.parse().unwrap();
            let output = selector
                .apply_field(&schema)
                .unwrap_or_else(|error| panic!("{text}: {error}"));
            assert_eq!(names(&output), expected, "{text}");
            assert_eq!(
                selector.to_string().parse::<Selector>().unwrap(),
                selector,
                "{text}"
            );
            let document = selector.clone().into_json().unwrap();
            assert_eq!(Selector::from_json(&document).unwrap(), selector, "{text}");
        }
        let selector: Selector = "explode(xs) as x".parse().unwrap();
        assert_eq!(selector.to_string(), "unnest(xs) as x");
        let selector: Selector = "id, unnest(legs) as leg".parse().unwrap();
        let output = selector.apply_field(&schema).unwrap();
        assert!(output.fields()[1..].iter().all(Field::is_nullable));
        assert_eq!(output.fields()[2].dtype(), &DataType::Int64);
        let selector: Selector = "unnest(xs) as x int32 not null".parse().unwrap();
        let output = selector.apply_field(&schema).unwrap();
        assert_eq!(output.fields()[0].dtype(), &DataType::Int32);
        assert!(!output.fields()[0].is_nullable());
        // The output schema is answered before any batch, a late filter
        // typed against it.
        let plan: Plan = "select id, unnest(legs) as leg where \"leg.size\" > 1"
            .parse()
            .unwrap();
        let output = plan.field_from(&schema).unwrap();
        assert_eq!(names(&output), ["id", "leg.ccy", "leg.size"]);
        assert_eq!(
            plan.apply_datatype(schema.dtype()).unwrap(),
            output.dtype().clone()
        );
    }

    #[test]
    fn every_element_is_a_row_beside_its_parent_and_an_empty_or_null_run_none() {
        let out = apply("select id, unnest(xs) as x, tag");
        assert_eq!(
            int64s(column(&out, "id")),
            [Some(1), Some(1), Some(1), Some(4), Some(4)]
        );
        assert_eq!(
            int64s(column(&out, "x")),
            [Some(1), Some(2), Some(3), None, Some(7)]
        );
        assert_eq!(
            texts(column(&out, "tag")),
            [Some("a"), Some("a"), Some("a"), Some("d"), Some("d")]
        );

        let out = apply("select id, unnest(legs) as leg");
        assert_eq!(
            int64s(column(&out, "id")),
            [Some(1), Some(1), Some(3), Some(3)]
        );
        assert_eq!(
            texts(column(&out, "leg.ccy")),
            [Some("EUR"), Some("USD"), None, Some("GBP")]
        );
        assert_eq!(
            int64s(column(&out, "leg.size")),
            [Some(1), Some(2), None, None]
        );

        // A layout with no offsets takes the row walk; a large and a fixed
        // run read through their own offsets.
        let out = apply("select id, unnest(views) as v");
        assert_eq!(int64s(column(&out, "id")), [Some(1), Some(1), Some(4)]);
        assert_eq!(int64s(column(&out, "v")), [Some(10), Some(20), Some(30)]);
        let out = apply("select id, unnest(large) as l");
        assert_eq!(int64s(column(&out, "id")), [Some(1), Some(4), Some(4)]);
        assert_eq!(int64s(column(&out, "l")), [Some(5), Some(6), Some(7)]);
        let out = apply("select id, unnest(fixed) as f");
        assert_eq!(
            int64s(column(&out, "id")),
            [Some(1), Some(1), Some(3), Some(3), Some(4), Some(4)]
        );
        assert_eq!(
            int64s(column(&out, "f")),
            [Some(1), Some(2), Some(3), Some(4), Some(5), None]
        );
        // A declared item type is cast into.
        let out = apply("select unnest(xs) as x int32");
        assert_eq!(
            out.schema_ref().field(0).data_type(),
            &arrow_schema::DataType::Int32
        );
    }

    #[test]
    fn a_sliced_batch_unnests_only_the_runs_its_rows_cover() {
        let batch = batch();
        let selector: Selector = "id, unnest(xs) as x".parse().unwrap();
        let out = selector.apply_arrow_batch(&batch.slice(2, 2)).unwrap();
        assert_eq!(int64s(column(&out, "id")), [Some(4), Some(4)]);
        assert_eq!(int64s(column(&out, "x")), [None, Some(7)]);
        let out = selector.apply_arrow_batch(&batch.slice(0, 1)).unwrap();
        assert_eq!(int64s(column(&out, "x")), [Some(1), Some(2), Some(3)]);
        let out = selector.apply_arrow_batch(&batch.slice(1, 0)).unwrap();
        assert_eq!(out.num_rows(), 0);
        assert_eq!(out.num_columns(), 2);
    }

    #[test]
    fn a_where_or_an_order_by_that_names_an_unnested_column_runs_after_it() {
        let out = apply("select id, unnest(legs) as leg where \"leg.ccy\" = 'EUR'");
        assert_eq!(int64s(column(&out, "id")), [Some(1)]);
        let out = apply("select id, unnest(xs) as x where x > 1 order by x desc");
        assert_eq!(int64s(column(&out, "x")), [Some(7), Some(3), Some(2)]);
        assert_eq!(int64s(column(&out, "id")), [Some(4), Some(1), Some(1)]);
        // A where over a stored column still runs first, where it prunes.
        let out = apply("select id, unnest(xs) as x where id = 4");
        assert_eq!(int64s(column(&out, "x")), [None, Some(7)]);
        let out = apply("select id, unnest(xs) as x order by id desc limit 2");
        assert_eq!(int64s(column(&out, "id")), [Some(4), Some(4)]);
    }

    #[test]
    fn records_unnest_through_the_stream_they_widen_into() {
        let schema = schema();
        let selector: Selector = "id, unnest(xs) as x".parse().unwrap();
        let records = selector.apply_records(Some(&schema), rows()).unwrap();
        assert_eq!(names(records.field()), ["id", "x"]);
        let held = records.collect_rows().unwrap();
        assert_eq!(held.len(), 5);
        assert_eq!(
            held[4],
            Scalar::from_sequence([Scalar::from(4_i64), Scalar::from(7_i64)])
        );
        // A struct array keeps one row per struct, which an unnest cannot.
        let array: ArrayRef = Arc::new(arrow_array::StructArray::from(batch()));
        let error = selector.apply_arrow_array(&array).unwrap_err().to_string();
        assert!(error.contains("unnest"), "{error}");
    }
}
