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
        // A call over plain columns is stored as the function and its sources,
        // the shape a signature and a partition spec share.
        assert_eq!(plan.fields()[2].get_metadata("TRANSFORM:expression"), None);
        assert_eq!(
            plan.fields()[2].get_metadata("TRANSFORM:function"),
            Some("lower")
        );
        assert_eq!(
            plan.fields()[2].get_metadata("TRANSFORM:sources"),
            Some(r#"["s"]"#)
        );
        assert!(plan.as_transform().declares_derivation());

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
    fn a_partition_declaration_is_a_transform() {
        let mut year = DataType::Int32.nullable_field("year");
        year.as_partition_mut().set_sources(["event"]).unwrap();
        year.as_partition_mut()
            .set_transform(yggdryl::expression::Function::Year)
            .unwrap();
        assert!(year.as_transform().is_derived());
        assert_eq!(
            year.as_transform()
                .term()
                .unwrap()
                .map(|term| term.to_string()),
            Some("year(event)".to_owned())
        );
        // An explicit term answers first, so a plan can override the pair.
        year.as_transform_mut()
            .set_term(&"year(event) + 1".parse().unwrap())
            .unwrap();
        assert_eq!(
            year.as_transform()
                .term()
                .unwrap()
                .map(|term| term.to_string()),
            Some("year(event) + 1".to_owned())
        );
        assert_eq!(
            year.as_transform_mut().remove_term(),
            Some("year(event) + 1".to_owned())
        );
        assert!(year.as_transform().is_derived());
    }

    #[test]
    fn a_stream_derives_every_batch_as_one_batch_does() {
        use std::sync::Arc;

        use arrow_array::{
            Array, ArrayRef, Date32Array, Int32Array, Int64Array, RecordBatch, StructArray,
        };
        use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

        // A derivation at the root and one inside a struct, applied over a
        // stream: every batch it yields is the batch that batch derives alone.
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
        let batches = vec![
            batch(&[19_723, 0], &[Some(1), None]),
            batch(&[], &[]),
            batch(&[-365], &[Some(-4)]),
        ];
        let stream = yggdryl::arrow::batch_reader(batches[0].schema(), batches.clone());
        let applied = root
            .apply_arrow_reader(stream, false, true, false, yggdryl::ArrowCastOptions::new())
            .unwrap();
        let mut yielded = 0;
        for (position, (streamed, batch)) in applied.zip(&batches).enumerate() {
            let alone = root.as_transform().apply_arrow_batch(batch).unwrap();
            assert_eq!(streamed.unwrap(), alone, "batch {position}");
            yielded += 1;
        }
        assert_eq!(yielded, batches.len());

        let first = root.as_transform().apply_arrow_batch(&batches[0]).unwrap();
        assert_eq!(
            first.column_by_name("year").unwrap().as_ref(),
            &Int32Array::from(vec![2024, 1970]) as &dyn Array
        );
        let inner = first.column_by_name("inner").unwrap();
        let inner = inner.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(
            inner.column_by_name("twice").unwrap().as_ref(),
            &Int64Array::from(vec![Some(2), None]) as &dyn Array
        );
    }
}
