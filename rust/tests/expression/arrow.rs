//! `rust/src/expression/arrow.rs`: the edge cases this module is built to
//! get right.
//!
//! Four properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, and a pruning decision never loses a
//! row.

mod grammar {

    use std::sync::Arc;

    use yggdryl::expression::{Bound, Expression, Term};
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
        // A predicate segment inside a comparison: the kept list is counted,
        // an empty match is zero, and a null list is unknown.
        "size(legs[ccy = 'EUR']) > 1",
        "legs[size > 1][-1].ccy = 'EUR'",
        "legs[notes[v > 1][0].k = 'b'][0].size = 1",
        "size(legs[true]) = size(legs)",
    ];

    /// Evaluate one term on both tiers and assert they agree on every row.
    fn assert_tiers_agree(text: &str, schema: &Field, rows: &[Scalar]) -> Bound {
        let batch = batch_of(schema, rows);
        let bound = text
            .parse::<Term>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .bind(schema)
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let vectorized = bound.evaluate(&batch).unwrap();
        let field = bound.field().clone().with_nullable(true);
        // The vectorized column is laid out exactly as the bound field
        // declares, so reading it back casts nothing.
        assert_eq!(
            vectorized.data_type(),
            field.as_arrow_field_ref().unwrap().data_type(),
            "{text}"
        );
        for (position, row) in rows.iter().enumerate() {
            let scalar = bound.eval(row).unwrap();
            // One row out of the vectorized column, through the public boundary:
            // a one-element slice is the scalar it holds.
            let held = yggdryl::Serie::from_arrow_array(
                Some(&field),
                vectorized.slice(position, 1),
                yggdryl::ArrowCastOptions::default(),
            )
            .unwrap()
            .scalar(0)
            .unwrap();
            assert_eq!(
                scalar, held,
                "{text} disagreed on row {position}: scalar {scalar:?}, vectorized {held:?}"
            );
        }
        bound
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

    #[test]
    fn scalar_and_vectorized_agree() {
        let schema = rows_schema();
        let rows = rows();
        for text in AGREEMENT {
            assert_tiers_agree(text, &schema, &rows);
        }
    }

    #[test]
    fn projections_agree_between_the_tiers() {
        let schema = rows_schema();
        let rows = rows();
        for text in [
            "lower(s)",
            "length(s)",
            "i + 1",
            "d * decimal128(9,2) '2.00'",
            "nested.leg",
            "coalesce(s, 'none')",
            "case when i > 0 then 'up' else 'down' end",
            "year(t)",
            "concat(s, '!')",
            "substring(s, 2, 3)",
            "xs[0]",
            "xs[-1]",
            "xs[5]",
            "xs[1:3]",
            "xs[:1]",
            "xs[1:]",
            "xs[-2:]",
            "slice(xs, 1, null)",
            "get(nested, 'leg')",
            // The predicate segment: an empty match, a null list, a null
            // element, a predicate nested in a predicate, chained predicates, a
            // bare boolean and a null predicate, and a function, arithmetic and
            // a membership test over the element's fields.
            "legs[ccy = 'EUR']",
            "legs[ccy = 'EUR'][0].size",
            "legs[size > 1][-1].ccy",
            "legs[notes[v > 1][0].k = 'b']",
            "legs[ccy = 'EUR'][size >= 3]",
            "legs[ccy = 'EUR'][0].notes[v >= 2]",
            "legs[notes[0].v = 1]",
            "legs[true]",
            "legs[null]",
            "legs[size is null]",
            "legs[lower(ccy) = 'eur']",
            "legs[size + 1 > 2][1:]",
            "legs[ccy in ('EUR', 'GBP') and notes is not null]",
            "legs[size between 1 and 2]",
            "legs[ccy = 'JPY'][0]",
            "size(legs[ccy = 'EUR'])",
        ] {
            assert_tiers_agree(text, &schema, &rows);
        }
    }

    #[test]
    fn a_mask_that_keeps_everything_keeps_the_batch_itself() {
        let schema = rows_schema();
        let batch = batch_of(&schema, &rows());
        let bound = "n = 2024 or n <> 2024 or n is null"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let filtered = bound.filter(&batch).unwrap();
        assert_eq!(filtered.num_rows(), batch.num_rows());
        for (left, right) in batch.columns().iter().zip(filtered.columns()) {
            assert!(
                Arc::ptr_eq(left, right),
                "a filter that dropped nothing still moved a buffer"
            );
        }
    }

    #[test]
    fn a_reader_filters_and_projects_in_one_pass() {
        let schema = rows_schema();
        let batch = batch_of(&schema, &rows());
        let arrow_schema = batch.schema();
        let filter: Expression = "where i is not null".parse().unwrap();
        let selector: Expression = "select i".parse().unwrap();
        let reader = yggdryl::arrow::batch_reader(arrow_schema, [batch]);
        let reader = selector
            .apply_arrow_reader(filter.apply_arrow_reader(reader).unwrap())
            .unwrap();
        assert_eq!(reader.schema().fields().len(), 1);
        let rows: usize = reader.map(|batch| batch.unwrap().num_rows()).sum();
        assert_eq!(rows, 4);
    }

    #[test]
    fn a_bound_cast_answers_every_batch_as_a_fresh_bind_does() {
        use arrow_array::{
            Array, ArrayRef, Int64Array, LargeStringArray, RecordBatch, StringArray,
        };

        // A bound cast holds its plan across batches: each batch - one whose
        // operand storage drifts from the first and back, an empty one, one
        // a strict cast refuses and the one after it - answers exactly what
        // a bind made for that batch alone answers.
        let schema = Field::new(
            "rows",
            StructType::from_fields([Field::new("s", DataType::utf8(), true)])
                .map(DataType::from)
                .unwrap(),
            false,
        );
        let utf8 = |values: &[Option<&str>]| {
            let column = Arc::new(StringArray::from(values.to_vec())) as ArrayRef;
            RecordBatch::try_from_iter([("s", column)]).unwrap()
        };
        let large = |values: &[Option<&str>]| {
            let column = Arc::new(LargeStringArray::from(values.to_vec())) as ArrayRef;
            RecordBatch::try_from_iter([("s", column)]).unwrap()
        };
        let batches = [
            utf8(&[Some("1"), None, Some("3")]),
            utf8(&[Some("40")]),
            large(&[Some("7"), None]),
            utf8(&[]),
            large(&[Some("8")]),
            utf8(&[Some("x")]),
            utf8(&[Some("5")]),
        ];
        for text in [
            "cast(s as int64)",
            "try_cast(s as int64)",
            "cast(s as int64) > 2",
        ] {
            let term: Term = text.parse().unwrap();
            let bound = term.bind(&schema).unwrap();
            for (position, batch) in batches.iter().enumerate() {
                let fresh = term.bind(&schema).unwrap().evaluate(batch);
                match (bound.evaluate(batch), fresh) {
                    (Ok(held), Ok(fresh)) => {
                        assert_eq!(held.as_ref(), fresh.as_ref(), "{text}, batch {position}");
                    }
                    (Err(held), Err(fresh)) => {
                        assert_eq!(
                            held.to_string(),
                            fresh.to_string(),
                            "{text}, batch {position}"
                        );
                    }
                    (held, fresh) => panic!("{text}, batch {position}: {held:?} and {fresh:?}"),
                }
            }
        }

        // The answers themselves, so the agreement is not two wrong ones.
        let bound = "cast(s as int64)"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let ints = |batch: &RecordBatch| {
            let answered = bound.evaluate(batch).unwrap();
            answered
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>()
        };
        assert_eq!(ints(&batches[0]), vec![Some(1), None, Some(3)]);
        assert_eq!(ints(&batches[2]), vec![Some(7), None]);
        assert_eq!(ints(&batches[1]), vec![Some(40)]);
        assert!(bound.evaluate(&batches[5]).is_err());
        assert_eq!(ints(&batches[4]), vec![Some(8)]);
        assert_eq!(ints(&batches[6]), vec![Some(5)]);
    }
}
