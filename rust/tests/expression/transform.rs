//! `rust/src/expression/transform.rs`: the edge cases this module is built
//! to get right.
//!
//! Four properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, and a pruning decision never loses a
//! row.

mod grammar {

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
                Scalar::decimal128(150, 2),
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
                Scalar::decimal128(-25, 2),
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
                Scalar::decimal128(10_000, 2),
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
                Scalar::decimal128(0, 2),
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

    #[test]
    fn a_field_holds_a_selector_and_gives_it_back() {
        let schema = rows_schema();
        let selector: Selector = "i, i + 1 as next, lower(s) as name".parse().unwrap();
        let plan = selector.into_field(&schema).unwrap();
        assert_eq!(plan.fields()[0].get_metadata("TRANSFORM:expression"), None);
        assert_eq!(
            plan.fields()[1].get_metadata("TRANSFORM:expression"),
            Some("i + 1")
        );
        // A call over plain columns is stored as the function and the columns
        // it reads, the shape a signature reads.
        assert_eq!(plan.fields()[2].get_metadata("TRANSFORM:expression"), None);
        assert_eq!(
            plan.fields()[2].get_metadata("TRANSFORM:function"),
            Some("lower")
        );
        assert_eq!(
            plan.fields()[2].get_metadata("TRANSFORM:by"),
            Some(r#"["s"]"#)
        );
        assert!(
            plan.fields()[1..]
                .iter()
                .all(|field| field.as_transform().is_derived())
        );

        // The reading is the `create table` one: every column with its type.
        let read = Selector::from_field(&plan);
        assert_eq!(
            read.to_string(),
            "i int64 null, i + 1 as next int64 null, lower(s) as name utf8 null"
        );
        let shape = |field: &Field| -> Vec<(String, DataType, bool)> {
            field
                .fields()
                .iter()
                .map(|child| {
                    (
                        child.name().to_owned(),
                        child.dtype().clone(),
                        child.is_nullable(),
                    )
                })
                .collect()
        };
        assert_eq!(shape(&read.apply_field(&schema).unwrap()), shape(&plan));
        // Applying the plan derives the same columns the selector computes.
        let batch = batch_of(&schema, &rows());
        let derived = plan.as_transform().apply_arrow_batch(&batch).unwrap();
        let projected = selector.apply_arrow_batch(&batch).unwrap();
        assert_eq!(
            derived.column_by_name("next").unwrap(),
            projected.column_by_name("next").unwrap()
        );
        assert_eq!(
            derived.column_by_name("name").unwrap(),
            projected.column_by_name("name").unwrap()
        );
        // A stored declaration is canonical text, and a malformed one is refused.
        let mut broken = plan.fields()[1].clone();
        broken
            .insert_metadata("TRANSFORM:expression", "i +")
            .unwrap_err();
        assert_eq!(broken.get_metadata("TRANSFORM:expression"), Some("i + 1"));
        broken
            .insert_metadata("TRANSFORM:expression", "I   +  2")
            .unwrap();
        assert_eq!(broken.get_metadata("TRANSFORM:expression"), Some("I + 2"));
    }

    #[test]
    fn a_derived_partition_column_is_a_transform() {
        // A derived entry of `PARTITION:by` materializes as a `TRANSFORM:`
        // column, marked as a partition, and reads back as a declaration.
        let rows = DataType::from(
            StructType::from_fields([DataType::date32().required_field("event")]).unwrap(),
        )
        .required_field("row")
        .with_partition_by(["years(event)".parse().unwrap()])
        .unwrap();
        let year = rows.get_field_by_path("event_year").unwrap().clone();
        assert!(year.is_partition());
        assert_eq!(year.get_metadata("TRANSFORM:function"), Some("years"));
        assert_eq!(year.get_metadata("TRANSFORM:by"), Some(r#"["event"]"#));
        assert!(year.as_transform().is_derived());
        assert_eq!(
            year.as_transform()
                .term()
                .unwrap()
                .map(|term| term.to_string()),
            Some("years(event)".to_owned())
        );
        assert_eq!(
            Selector::from_field(&rows).to_string(),
            "event date32 not null, years(event) as event_year int32 not null with (\"FIELD:partition\" = 'true')"
        );
        // A function with no `by` beside it is an incomplete declaration.
        let mut bare = DataType::Int32.nullable_field("year");
        bare.as_transform_mut().insert("function", "years").unwrap();
        let error = bare.as_transform().term().unwrap_err().to_string();
        assert!(error.contains("TRANSFORM:by"), "{error}");
        // An explicit term answers first, and removing it leaves an ordinary
        // column.
        let mut year = year;
        year.as_transform_mut()
            .set_term(&"years(event) + 1".parse().unwrap())
            .unwrap();
        assert_eq!(year.get_metadata("TRANSFORM:function"), None);
        assert_eq!(
            year.as_transform()
                .term()
                .unwrap()
                .map(|term| term.to_string()),
            Some("years(event) + 1".to_owned())
        );
        assert_eq!(
            year.as_transform_mut().remove_term(),
            Some("years(event) + 1".to_owned())
        );
        assert!(!year.as_transform().is_derived());
    }

    #[test]
    fn a_root_and_a_nested_derivation_fill_every_batch() {
        use std::sync::Arc;

        use arrow_array::{
            Array, ArrayRef, Date32Array, Int32Array, Int64Array, RecordBatch, StructArray,
        };
        use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

        // A derivation at the root and one inside a struct, each batch filled
        // on its own, the empty one included.
        let mut year = DataType::Int32.nullable_field("year");
        year.as_transform_mut()
            .set_term(&"year(event)".parse().unwrap())
            .unwrap();
        let mut twice = DataType::Int64.nullable_field("twice");
        twice
            .as_transform_mut()
            .set_term(&"a * 2".parse().unwrap())
            .unwrap();
        let inner = DataType::from(
            StructType::from_fields([DataType::Int64.nullable_field("a"), twice]).unwrap(),
        )
        .nullable_field("inner");
        let root = DataType::from(
            StructType::from_fields([DataType::date32().required_field("event"), year, inner])
                .unwrap(),
        )
        .required_field("row");

        let batch = |days: &[i32], values: &[Option<i64>]| {
            let a = Arc::new(Int64Array::from(values.to_vec())) as ArrayRef;
            let inner = StructArray::from(vec![(
                Arc::new(ArrowField::new("a", ArrowDataType::Int64, true)),
                a,
            )]);
            RecordBatch::try_from_iter([
                (
                    "event",
                    Arc::new(Date32Array::from(days.to_vec())) as ArrayRef,
                ),
                ("inner", Arc::new(inner) as ArrayRef),
            ])
            .unwrap()
        };
        let expected = [
            (
                batch(&[19_723, 0], &[Some(1), None]),
                vec![Some(2024), Some(1970)],
                vec![Some(2), None],
            ),
            (batch(&[], &[]), vec![], vec![]),
            (
                batch(&[-365], &[Some(-4)]),
                vec![Some(1969)],
                vec![Some(-8)],
            ),
        ];
        for (position, (batch, years, twice)) in expected.into_iter().enumerate() {
            let filled = root.as_transform().apply_arrow_batch(&batch).unwrap();
            assert_eq!(
                filled.column_by_name("year").unwrap().as_ref(),
                &Int32Array::from(years) as &dyn Array,
                "batch {position}"
            );
            let inner = filled.column_by_name("inner").unwrap();
            let inner = inner.as_any().downcast_ref::<StructArray>().unwrap();
            assert_eq!(
                inner.column_by_name("twice").unwrap().as_ref(),
                &Int64Array::from(twice) as &dyn Array,
                "batch {position}"
            );
        }
    }

    /// The partition instant a table root declares: `partunix` derived from
    /// `currunix` by `time_bucket`, filled where it arrives absent or wholly
    /// null and left alone where any row of it was written.
    #[test]
    fn a_time_bucket_term_fills_the_partition_instant_from_currunix() {
        use yggdryl::{ArrowCastOptions, Serie};

        let ns = DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        };
        let term = "time_bucket('15 minutes', currunix)";
        let mut partunix = ns.clone().nullable_field("partunix");
        partunix
            .as_transform_mut()
            .set_term(&term.parse().unwrap())
            .unwrap();
        // A literal argument keeps the whole term under `expression`.
        assert_eq!(partunix.get_metadata("TRANSFORM:expression"), Some(term));
        assert_eq!(partunix.get_metadata("TRANSFORM:function"), None);
        let root = DataType::from(
            StructType::from_fields([ns.clone().required_field("currunix"), partunix]).unwrap(),
        )
        .required_field("row");
        let rows_only = DataType::from(
            StructType::from_fields([ns.clone().required_field("currunix")]).unwrap(),
        )
        .required_field("row");

        let nanos =
            |count: i64| Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::UTC).unwrap();
        let instants = [
            899_999_999_999_i64,
            900_000_000_000,
            -1,
            1_704_067_200_000_000_001,
        ];
        let floored = [
            0_i64,
            900_000_000_000,
            -900_000_000_000,
            1_704_067_200_000_000_000,
        ];
        let partunix_of = |batch: &arrow_array::RecordBatch| -> Vec<Scalar> {
            let filled = root.as_transform().apply_arrow_batch(batch).unwrap();
            let read =
                Serie::from_arrow_batch(Some(&root), &filled, ArrowCastOptions::new()).unwrap();
            (0..read.len())
                .map(|position| read.scalar(position).unwrap().as_sequence().unwrap()[1].clone())
                .collect()
        };
        let expected: Vec<Scalar> = floored.iter().map(|count| nanos(*count)).collect();

        // Absent: the rows carry `currunix` alone.
        let absent = Serie::from_scalars(
            rows_only.clone(),
            instants
                .iter()
                .map(|count| Scalar::from_sequence([nanos(*count)])),
        )
        .unwrap()
        .into_arrow_batch()
        .unwrap();
        assert_eq!(partunix_of(&absent), expected);

        // Present and null in every row: the declaration's default, filled.
        let nulls = Serie::from_scalars(
            root.clone(),
            instants
                .iter()
                .map(|count| Scalar::from_sequence([nanos(*count), Scalar::Null])),
        )
        .unwrap()
        .into_arrow_batch()
        .unwrap();
        assert_eq!(partunix_of(&nulls), expected);

        // Written in any row: the column is the caller's, left as it came,
        // its null rows included.
        let written = Serie::from_scalars(
            root.clone(),
            instants.iter().enumerate().map(|(position, count)| {
                Scalar::from_sequence([
                    nanos(*count),
                    if position == 0 {
                        nanos(7)
                    } else {
                        Scalar::Null
                    },
                ])
            }),
        )
        .unwrap()
        .into_arrow_batch()
        .unwrap();
        assert_eq!(
            partunix_of(&written),
            vec![nanos(7), Scalar::Null, Scalar::Null, Scalar::Null]
        );
    }

    #[test]
    fn a_stream_is_filled_under_one_plan_and_states_its_columns_first() {
        use arrow_array::RecordBatchReader as _;
        use yggdryl::{ArrowCastOptions, Serie};

        let ns = DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        };
        let mut partunix = ns.clone().nullable_field("partunix");
        partunix
            .as_transform_mut()
            .set_term(&"time_bucket('15 minutes', currunix)".parse().unwrap())
            .unwrap();
        let root = DataType::from(
            StructType::from_fields([ns.clone().required_field("currunix"), partunix]).unwrap(),
        )
        .required_field("row");
        let rows_only = DataType::from(
            StructType::from_fields([ns.clone().required_field("currunix")]).unwrap(),
        )
        .required_field("row");
        let nanos =
            |count: i64| Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::UTC).unwrap();
        let batch = |counts: &[i64]| {
            Serie::from_scalars(
                rows_only.clone(),
                counts
                    .iter()
                    .map(|count| Scalar::from_sequence([nanos(*count)])),
            )
            .unwrap()
            .into_arrow_batch()
            .unwrap()
        };

        // Two batches of one layout: the derived column is in the reader's
        // schema before either is pulled, and every batch carries it.
        let (first, second) = (batch(&[1, 900_000_000_001]), batch(&[1_800_000_000_000]));
        let reader = yggdryl::arrow::batch_reader(first.schema(), [first, second]);
        let filled = root.as_transform().apply_arrow_reader(reader).unwrap();
        let schema = filled.schema();
        assert_eq!(
            schema
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>(),
            ["currunix", "partunix"]
        );
        let mut read = Vec::new();
        for filled in filled {
            let filled = filled.unwrap();
            assert_eq!(filled.schema(), schema);
            let rows =
                Serie::from_arrow_batch(Some(&root), &filled, ArrowCastOptions::new()).unwrap();
            for position in 0..rows.len() {
                read.push(rows.scalar(position).unwrap().as_sequence().unwrap()[1].clone());
            }
        }
        assert_eq!(
            read,
            vec![nanos(0), nanos(900_000_000_000), nanos(1_800_000_000_000)]
        );

        // A schema deriving nothing hands the reader back: its batches are
        // the caller's own.
        let plain = batch(&[5]);
        let reader = yggdryl::arrow::batch_reader(plain.schema(), [plain.clone()]);
        let mut same = rows_only.as_transform().apply_arrow_reader(reader).unwrap();
        assert_eq!(same.schema(), plain.schema());
        assert_eq!(same.next().unwrap().unwrap(), plain);

        // Rows that cannot answer the term are refused before a batch is
        // pulled, naming the column the term reads.
        let other = Serie::from_scalars(
            DataType::from(
                StructType::from_fields([DataType::Int64.required_field("id")]).unwrap(),
            )
            .required_field("row"),
            [Scalar::from_sequence([Scalar::from(1_i64)])],
        )
        .unwrap()
        .into_arrow_batch()
        .unwrap();
        let reader = yggdryl::arrow::batch_reader(other.schema(), [other]);
        let Err(error) = root.as_transform().apply_arrow_reader(reader) else {
            panic!("a stream without the term's column was filled");
        };
        assert!(error.to_string().contains("currunix"), "{error}");
    }
}
