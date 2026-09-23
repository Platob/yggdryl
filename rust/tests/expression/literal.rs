//! `rust/src/expression/literal.rs`: the edge cases this module is built to
//! get right.
//!
//! Five properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, a free attribute never costs a backend
//! call, and a pruning decision never loses a row.

mod grammar {

    use yggdryl::expression::{Literal, Term};
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

    #[test]
    fn a_literal_is_converted_once_into_the_column_it_meets() {
        let schema = rows_schema();
        let bound = "d > 100".parse::<Term>().unwrap().bind(&schema).unwrap();
        // The bound term prints the literal in the column's own type, which is
        // how a caller sees that the comparison is exact rather than floating.
        assert_eq!(bound.term().to_string(), "d > decimal128(9,2) '100.00'");
    }

    // ---------------------------------------------------------------------------
    // Literals
    // ---------------------------------------------------------------------------

    #[test]
    fn a_literal_holds_what_its_datatype_stores() {
        let narrowed = Literal::new(DataType::Int32, 7_i64).unwrap();
        assert_eq!(narrowed.dtype(), &DataType::Int32);
        assert_eq!(narrowed.value(), &Scalar::from(7_i32));
        assert!(!narrowed.is_null());
        assert_eq!(narrowed.to_string(), "int32 '7'");
        assert_eq!(
            narrowed.clone().into_parts(),
            (DataType::Int32, Scalar::from(7_i32))
        );

        let refused = Literal::new(DataType::Int8, 1_000_i64)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("int8"), "{refused}");
        assert!(Literal::new(DataType::Int64, "seven").is_err());

        let inferred = Literal::infer(Scalar::from("AAPL")).unwrap();
        assert_eq!(inferred.dtype(), &DataType::utf8());
        assert_eq!(inferred.to_string(), "'AAPL'");
        let mixed = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
        assert!(Literal::infer(mixed.clone()).is_err());
        // The term constructor holds what it cannot type as the null it is.
        assert_eq!(Term::literal(mixed).as_literal(), Some(&Literal::null()));

        let null = Literal::new(DataType::Int64, Scalar::Null).unwrap();
        assert!(null.is_null());
        assert_eq!(null.to_string(), "int64 null");
        assert_eq!(Literal::null().dtype(), &DataType::Null);
    }

    #[test]
    fn literals_order_by_datatype_then_value_and_serialize_as_both_halves() {
        let first = Literal::new(DataType::Int32, 7).unwrap();
        let later_value = Literal::new(DataType::Int32, 8).unwrap();
        let later_type = Literal::new(DataType::Int64, 7).unwrap();
        assert!(first < later_value);
        assert!(first < later_type);
        assert_eq!(first, first.clone());

        let encoded = serde_json::to_vec(&first).unwrap();
        assert_eq!(serde_json::from_slice::<Literal>(&encoded).unwrap(), first);
        let term = Term::Literal(first.clone());
        let encoded = serde_json::to_vec(&term).unwrap();
        assert_eq!(serde_json::from_slice::<Term>(&encoded).unwrap(), term);

        // A literal that never agreed is refused on the way in, not stored.
        let contradiction =
            br#"{"dtype":{"type":"int64"},"value":{"type":"string","value":"seven"}}"#;
        assert!(serde_json::from_slice::<Literal>(contradiction).is_err());
        // A literal is read back through the value contract, so it holds what
        // the datatype stores even when the text spelled it wider.
        let widened = br#"{"dtype":{"type":"int32"},"value":{"type":"i64","value":7}}"#;
        let literal = serde_json::from_slice::<Literal>(widened).unwrap();
        assert_eq!(literal.value(), &Scalar::from(7_i32));
    }
}
