//! `rust/src/xxhash/field.rs`: digest roles on fields, and the component
//! selection a declaration resolves to.

mod xxhash_arrow {
    use arrow_array::cast::AsArray as _;
    use arrow_array::types::{UInt32Type, UInt64Type};
    use arrow_array::{
        Array, ArrayRef, FixedSizeBinaryArray, Int64Array, RecordBatch, StringArray, StructArray,
    };
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use std::sync::Arc;
    use yggdryl::xxhash::Xxh3;
    use yggdryl::xxhash::arrow::row_digests;
    use yggdryl::{DataType, Digest, DigestAlgorithm, Field, Scalar, StructType};

    fn root(fields: impl IntoIterator<Item = Field>) -> Field {
        DataType::from(StructType::from_fields(fields).unwrap()).required_field("row")
    }

    fn holder(name: &str, dtype: DataType) -> Field {
        let mut field = Field::new(name, dtype, false);
        field.as_digest_mut().set_holder().unwrap();
        field
    }

    fn row_digest(value: impl Into<Scalar>, algorithm: DigestAlgorithm) -> Digest {
        Scalar::from_sequence([value.into()]).digest(algorithm)
    }

    /// Read one digest column back as the digests it holds.
    fn digests(array: &ArrayRef, algorithm: DigestAlgorithm) -> Vec<Digest> {
        match algorithm {
            DigestAlgorithm::Xxh32 => array
                .as_primitive::<UInt32Type>()
                .values()
                .iter()
                .map(|value| Digest::new(algorithm, u128::from(*value)))
                .collect(),
            DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => array
                .as_primitive::<UInt64Type>()
                .values()
                .iter()
                .map(|value| Digest::new(algorithm, u128::from(*value)))
                .collect(),
            DigestAlgorithm::Xxh128 => {
                let array = array
                    .as_any()
                    .downcast_ref::<FixedSizeBinaryArray>()
                    .expect("the 128-bit column is fixed-size binary");
                (0..array.len())
                    .map(|index| {
                        Digest::from_bytes(algorithm, array.value(index)).expect("the exact width")
                    })
                    .collect()
            }
            // `DigestAlgorithm` is non-exhaustive outside the crate; a new one
            // reaching here has no column reading stated yet.
            other => panic!("{other} has no digest column reading"),
        }
    }

    fn empty_batch() -> RecordBatch {
        RecordBatch::new_empty(Arc::new(Schema::empty()))
    }

    fn assert_metadata_error(error: yggdryl::arrow::Error, key: &str, holder: &str) {
        match error {
            yggdryl::arrow::Error::Core(yggdryl::Error::InvalidMetadataValue {
                key: actual,
                reason,
            }) => {
                assert_eq!(actual.as_str(), key);
                assert!(reason.contains(holder), "{reason}");
            }
            other => panic!("expected {key} metadata error, got {other}"),
        }
    }

    #[test]
    fn row_digest_roles_exclude_holders_and_nothing_else_narrows_the_input() {
        let symbol = Arc::new(StringArray::from(vec!["AAPL", "MSFT"])) as ArrayRef;
        let quantity = Arc::new(Int64Array::from(vec![100, 250])) as ArrayRef;
        let stored = Arc::new(Int64Array::from(vec![11, 22])) as ArrayRef;

        let plain = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                ArrowField::new("symbol", ArrowDataType::Utf8, false),
                ArrowField::new("quantity", ArrowDataType::Int64, false),
            ])),
            vec![Arc::clone(&symbol), Arc::clone(&quantity)],
        )
        .unwrap();

        let mut holder = Field::new("row_digest", DataType::Int64, false);
        holder.as_digest_mut().set_holder().unwrap();
        let fallback = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                ArrowField::new("symbol", ArrowDataType::Utf8, false),
                ArrowField::new("quantity", ArrowDataType::Int64, false),
                holder.into_arrow_field().unwrap(),
            ])),
            vec![
                Arc::clone(&symbol),
                Arc::clone(&quantity),
                Arc::clone(&stored),
            ],
        )
        .unwrap();

        for algorithm in DigestAlgorithm::ALL {
            assert_eq!(
                digests(&row_digests(&fallback, algorithm).unwrap(), algorithm),
                digests(&row_digests(&plain, algorithm).unwrap(), algorithm),
                "a holder is excluded for {algorithm}"
            );
        }
    }

    #[test]
    fn holder_sources_are_ordered_and_preserve_explicit_empty() {
        // A source states nothing on the field it names: `a` and `b` stay ordinary
        // columns, and only the holder carries metadata.
        let a = DataType::Int64.required_field("a");
        assert!(a.as_digest().is_empty());
        let b = DataType::utf8().required_field("b");
        let mut ordered = holder("ordered", DataType::UInt64);
        ordered.as_digest_mut().set_sources(["b", "a"]).unwrap();
        let mut empty = holder("empty", DataType::UInt64);
        empty
            .as_digest_mut()
            .set_sources(Vec::<&str>::new())
            .unwrap();
        let root = root([a, b, ordered, empty]);
        let rows = Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from(7),
            Scalar::from("AAPL"),
            Scalar::from(0_u64),
            Scalar::from(0_u64),
        ])]);
        let source = yggdryl::arrow::batch_from_value(&root, &rows).unwrap();
        let filled = Xxh3::new().apply_arrow_batch(&root, source, false).unwrap();

        let expected_ordered = Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(7)])
            .digest(DigestAlgorithm::Xxh3)
            .as_u64()
            .unwrap();
        let expected_empty = Scalar::from_sequence(Vec::<Scalar>::new())
            .digest(DigestAlgorithm::Xxh3)
            .as_u64()
            .unwrap();
        assert_eq!(
            filled.column(2).as_primitive::<UInt64Type>().value(0),
            expected_ordered
        );
        assert_eq!(
            filled.column(3).as_primitive::<UInt64Type>().value(0),
            expected_empty
        );
        assert_ne!(
            expected_ordered,
            row_digest(7, DigestAlgorithm::Xxh3).as_u64().unwrap(),
            "paths override the explicit component role"
        );
    }

    #[test]
    fn digest_metadata_under_a_collection_is_refused_with_the_same_reach() {
        // The same reach decides the metadata-ownership rules: `DIGEST:sources` on
        // a field that is not a holder is refused at the top level, so it cannot
        // be accepted one layout down.
        let source = Field::from_parts(
            "value",
            DataType::Int64,
            true,
            [("DIGEST:sources", "[\"other\"]")],
        )
        .unwrap();
        let element = DataType::from(
            StructType::from_fields([source, DataType::Int64.nullable_field("other")]).unwrap(),
        );
        let root = root([
            DataType::list(element.required_field("item")).nullable_field("events"),
            holder("row_digest", DataType::UInt64),
        ]);

        let error = root
            .as_digest()
            .apply_arrow_batch(&empty_batch())
            .expect_err("digest metadata belongs to a holder, at any depth")
            .to_string();
        assert!(error.contains("events.item.value"), "{error}");
    }

    #[test]
    fn invalid_holder_algorithms_and_metadata_ownership_are_rejected() {
        let wrong_width = Field::from_parts(
            "digest",
            DataType::UInt32,
            false,
            [("DIGEST:role", "holder"), ("DIGEST:algorithm", "xxh3-64")],
        )
        .unwrap();
        let error = Xxh3::new()
            .apply_arrow_batch(&root([wrong_width]), empty_batch(), false)
            .unwrap_err();
        assert_metadata_error(error, "DIGEST:algorithm", "$.digest");

        let non_holder_algorithm = Field::from_parts(
            "value",
            DataType::UInt64,
            false,
            [("DIGEST:algorithm", "xxh3-64")],
        )
        .unwrap();
        let error = Xxh3::new()
            .apply_arrow_batch(&root([non_holder_algorithm]), empty_batch(), false)
            .unwrap_err();
        assert_metadata_error(error, "DIGEST:algorithm", "$.value");

        let non_holder_paths =
            Field::from_parts("value", DataType::UInt64, false, [("DIGEST:sources", "[]")])
                .unwrap();
        let error = Xxh3::new()
            .apply_arrow_batch(&root([non_holder_paths]), empty_batch(), false)
            .unwrap_err();
        assert_metadata_error(error, "DIGEST:sources", "$.value");

        let non_struct_root = DataType::Int64.required_field("value");
        let error = Xxh3::new()
            .apply_arrow_batch(&non_struct_root, empty_batch(), false)
            .unwrap_err();
        assert!(
            matches!(error, yggdryl::arrow::Error::IncompatibleSchema(_)),
            "an invalid batch root remains a schema error"
        );
    }

    #[test]
    fn digest_sources_reject_peer_outputs_ambiguity_duplicates_and_collection_descent() {
        let peer = holder("peer", DataType::UInt64);
        let mut selecting_peer = holder("digest", DataType::UInt64);
        selecting_peer
            .as_digest_mut()
            .set_sources(["peer"])
            .unwrap();
        let error = Xxh3::new()
            .apply_arrow_batch(&root([peer, selecting_peer]), empty_batch(), false)
            .unwrap_err();
        assert_metadata_error(error, "DIGEST:sources", "$.digest");

        let nested_value = DataType::Int64.required_field("value");
        let nested_holder = holder("digest", DataType::UInt64);
        let nested = StructType::from_fields([nested_value, nested_holder])
            .map(DataType::from)
            .unwrap()
            .required_field("nested");
        let mut duplicate = holder("digest", DataType::UInt64);
        duplicate
            .as_digest_mut()
            .set_sources(["nested", "nested.digest"])
            .unwrap();
        let error = Xxh3::new()
            .apply_arrow_batch(&root([nested.clone(), duplicate]), empty_batch(), false)
            .unwrap_err();
        assert_metadata_error(error, "DIGEST:sources", "$.digest");

        let nested = StructType::from_fields([
            holder("left", DataType::UInt64),
            holder("right", DataType::UInt64),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("nested");
        let mut ambiguous = holder("digest", DataType::UInt64);
        ambiguous.as_digest_mut().set_sources(["nested"]).unwrap();
        let error = Xxh3::new()
            .apply_arrow_batch(&root([nested, ambiguous]), empty_batch(), false)
            .unwrap_err();
        assert_metadata_error(error, "DIGEST:sources", "$.digest");

        let items = DataType::from_str("array<struct<value:int64>>")
            .unwrap()
            .required_field("items");
        let mut collection = holder("digest", DataType::UInt64);
        collection
            .as_digest_mut()
            .set_sources(["items.value"])
            .unwrap();
        let error = Xxh3::new()
            .apply_arrow_batch(&root([items, collection]), empty_batch(), false)
            .unwrap_err();
        assert_metadata_error(error, "DIGEST:sources", "$.digest");
    }

    #[test]
    fn digest_sources_try_later_literal_prefixes_and_allow_terminal_collections() {
        let scalar_prefix = DataType::Int64.required_field("a");
        let dotted_prefix = StructType::from_fields([DataType::Int64.required_field("c")])
            .map(DataType::from)
            .unwrap()
            .required_field("a.b");
        let items = DataType::from_str("array<int64>")
            .unwrap()
            .required_field("items");
        let mut digest = holder("digest", DataType::UInt64);
        digest
            .as_digest_mut()
            .set_sources(["a.b.c", "items"])
            .unwrap();
        let root = root([scalar_prefix, dotted_prefix, items, digest]);
        let item_value = Scalar::from_sequence([Scalar::from(3), Scalar::from(4)]);
        let rows = Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from(1),
            Scalar::from_sequence([Scalar::from(2)]),
            item_value.clone(),
            Scalar::from(0_u64),
        ])]);
        let source = yggdryl::arrow::batch_from_value(&root, &rows).unwrap();
        let filled = Xxh3::new().apply_arrow_batch(&root, source, false).unwrap();
        let expected = Scalar::from_sequence([Scalar::from(2), item_value])
            .digest(DigestAlgorithm::Xxh3)
            .as_u64()
            .unwrap();
        assert_eq!(
            filled.column(3).as_primitive::<UInt64Type>().value(0),
            expected
        );
    }

    #[test]
    fn the_digest_view_answers_the_seedless_state_and_walks_nested_holders() {
        let inner_value = DataType::Int64.required_field("value");
        let inner_digest = holder("inner_digest", DataType::UInt64);
        let nested = StructType::from_fields([inner_value.clone(), inner_digest])
            .map(DataType::from)
            .unwrap()
            .required_field("nested");
        let root = root([nested, holder("row_digest", DataType::UInt64)]);

        let inner = StructArray::from(vec![(
            Arc::new(inner_value.into_arrow_field().unwrap()),
            Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
        )]);
        let source = RecordBatch::try_from_iter([("nested", Arc::new(inner) as ArrayRef)]).unwrap();

        let applied = root.as_digest().apply_arrow_batch(&source).unwrap();

        // The view is the seedless state: no configuration crosses into it.
        assert_eq!(
            applied,
            Xxh3::new()
                .apply_arrow_batch(&root, source.clone(), false)
                .unwrap()
        );
        let nested = applied.column(0).as_struct();
        assert_eq!(nested.num_columns(), 2);
        assert_eq!(nested.column(1).null_count(), 0);
        assert_eq!(applied.column(1).null_count(), 0);

        // Every holder now carries a written value, so a second pass changes none.
        assert_eq!(
            root.as_digest().apply_arrow_batch(&applied).unwrap(),
            applied
        );
    }

    #[test]
    fn the_star_source_is_the_same_selection_as_naming_none() {
        let a = DataType::Int64.required_field("a");
        let b = DataType::utf8().required_field("b");
        let mut starred = holder("digest", DataType::UInt64);
        starred.as_digest_mut().set_sources(["*"]).unwrap();
        let implied = holder("digest", DataType::UInt64);

        let rows = Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from(7),
            Scalar::from("AAPL"),
            Scalar::from(0_u64),
        ])]);
        let starred_root = root([a.clone(), b.clone(), starred]);
        let implied_root = root([a, b, implied]);
        let starred_source = yggdryl::arrow::batch_from_value(&starred_root, &rows).unwrap();
        let implied_source = yggdryl::arrow::batch_from_value(&implied_root, &rows).unwrap();

        let starred_filled = starred_root
            .as_digest()
            .apply_arrow_batch(&starred_source)
            .unwrap();
        let implied_filled = implied_root
            .as_digest()
            .apply_arrow_batch(&implied_source)
            .unwrap();

        let expected = Scalar::from_sequence([Scalar::from(7), Scalar::from("AAPL")])
            .digest(DigestAlgorithm::Xxh3)
            .as_u64()
            .unwrap();
        assert_eq!(
            starred_filled
                .column(2)
                .as_primitive::<UInt64Type>()
                .value(0),
            expected
        );
        assert_eq!(
            implied_filled
                .column(2)
                .as_primitive::<UInt64Type>()
                .value(0),
            expected
        );
    }
}
