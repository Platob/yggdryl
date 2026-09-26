//! `rust/src/default.rs`.

mod datatypes {
    use yggdryl::{DataType, Field, Scalar, StructType, TimeUnit, Timezone, UnionMode};

    fn all_variants() -> Vec<DataType> {
        let item = || Field::new("item", DataType::Int32, true);
        vec![
            DataType::Null,
            DataType::Boolean,
            DataType::Int8,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
            DataType::UInt8,
            DataType::UInt16,
            DataType::UInt32,
            DataType::UInt64,
            DataType::Float16,
            DataType::Float32,
            DataType::Float64,
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::UTC,
            },
            DataType::date32(),
            DataType::date64(),
            DataType::Time32(TimeUnit::Millisecond),
            DataType::Time64(TimeUnit::Nanosecond),
            DataType::Duration32(TimeUnit::Second),
            DataType::Duration64(TimeUnit::Second),
            DataType::Interval(TimeUnit::YearMonth),
            DataType::Interval(TimeUnit::DayTime),
            DataType::Interval(TimeUnit::MonthDayNano),
            DataType::binary(),
            DataType::from_str("binary(16)").unwrap(),
            DataType::fixed_binary(3).unwrap(),
            DataType::large_binary(),
            DataType::binary_view(),
            DataType::utf8(),
            DataType::from_str("utf8(32)").unwrap(),
            DataType::large_utf8(),
            DataType::utf8_view(),
            DataType::from_str("large_utf8_view").unwrap(),
            DataType::fixed_utf8(8).unwrap(),
            DataType::from_str("string(windows-1252)").unwrap(),
            DataType::ascii(),
            DataType::from_str("ascii(4)").unwrap(),
            DataType::fixed_ascii(4).unwrap(),
            DataType::fixed_ascii(8).unwrap(),
            DataType::fixed_ascii(16).unwrap(),
            DataType::serie(item()),
            DataType::serie_view(item()),
            DataType::fixed_size_serie(item(), 2).unwrap(),
            DataType::large_serie(item()),
            DataType::large_serie_view(item()),
            StructType::from_fields([
                Field::new("required", DataType::Int32, false),
                Field::new("optional", DataType::utf8(), true),
            ])
            .map(DataType::from)
            .unwrap(),
            DataType::union(
                [
                    (2, Field::new("nothing", DataType::Null, true)),
                    (7, Field::new("number", DataType::Int32, false)),
                ],
                UnionMode::Dense,
            )
            .unwrap(),
            DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap(),
            DataType::decimal32(7, 2).unwrap(),
            DataType::decimal64(12, 3).unwrap(),
            DataType::decimal128(30, 4).unwrap(),
            DataType::decimal256(50, 5).unwrap(),
            DataType::map_of(DataType::utf8(), DataType::Int32, true).unwrap(),
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", DataType::utf8(), true),
            )
            .unwrap(),
        ]
    }

    #[test]
    fn every_datatype_variant_has_a_bounded_valid_default() {
        for dtype in all_variants() {
            let value = dtype
                .default_value()
                .unwrap_or_else(|error| panic!("{} default failed: {error}", dtype.kind()));
            assert!(
                dtype
                    .is_default_value(&value)
                    .unwrap_or_else(|error| panic!(
                        "{} default match failed: {error}",
                        dtype.kind()
                    )),
                "{} did not recognize its canonical default",
                dtype.kind()
            );
            let nullable = matches!(dtype, DataType::Null);
            let field = Field::new("value", dtype.clone(), nullable);
            assert_eq!(
                field.default_value().unwrap_or_else(|error| {
                    panic!("{} Field default failed: {error}", dtype.kind())
                }),
                value
            );
            let root = Field::new(
                "Root",
                DataType::from(StructType::from_fields([field]).unwrap()),
                false,
            );
            let row = Scalar::from_sequence([value]);
            root.validate_value(&row).unwrap_or_else(|error| {
                panic!("{} default did not validate: {error}", dtype.kind())
            });
        }
    }

    #[test]
    fn default_matching_is_allocation_free_for_wide_values_and_exact_for_unions() {
        let wide = DataType::fixed_binary(64 * 1024 * 1024).unwrap();
        assert!(!wide.is_default_value(&Scalar::Null).unwrap());
        assert!(!wide.is_default_value(&Scalar::from(vec![0_u8; 3])).unwrap());

        let union = DataType::union(
            [
                (2, Field::new("nothing", DataType::Null, false)),
                (7, Field::new("number", DataType::Int32, false)),
            ],
            UnionMode::Dense,
        )
        .unwrap();
        let expected = union.default_value().unwrap();
        assert!(union.is_default_value(&expected).unwrap());
        assert!(union.default_union_type_id().unwrap() == Some(7));
        assert!(
            !union
                .is_default_value(&Scalar::from_sequence([Scalar::from(2), Scalar::from(0)]))
                .unwrap()
        );
        assert!(
            !union
                .is_default_value(&Scalar::from_sequence([Scalar::from(7), Scalar::from(1)]))
                .unwrap()
        );

        let fatal = DataType::fixed_binary(64 * 1024 * 1024 + 1).unwrap();
        assert!(fatal.is_default_value(&Scalar::Null).is_err());
    }

    #[test]
    fn nested_defaults_respect_child_field_nullability() {
        let structure = StructType::from_fields([
            Field::new("required", DataType::Int32, false),
            Field::new("optional", DataType::utf8(), true),
        ])
        .map(DataType::from)
        .unwrap();
        assert_eq!(
            structure.default_value().unwrap().as_sequence().unwrap(),
            &[Scalar::from(0), Scalar::Null]
        );

        let fixed =
            DataType::fixed_size_serie(Field::new("item", DataType::Int32, true), 3).unwrap();
        assert_eq!(
            fixed.default_value().unwrap().as_sequence().unwrap(),
            &[Scalar::Null, Scalar::Null, Scalar::Null]
        );
    }

    #[test]
    fn field_defaults_apply_physical_union_and_run_end_nulls() {
        let union = DataType::union(
            [
                (3, Field::new("required", DataType::Int32, false)),
                (9, Field::new("optional", DataType::utf8(), true)),
            ],
            UnionMode::Dense,
        )
        .unwrap();
        let nullable = Field::new("choice", union.clone(), true);
        assert_eq!(
            nullable.default_value().unwrap().as_sequence().unwrap(),
            &[Scalar::from(9), Scalar::Null]
        );
        assert_eq!(
            Field::new("choice", union, false)
                .default_value()
                .unwrap()
                .as_sequence()
                .unwrap(),
            &[Scalar::from(3), Scalar::from(0)]
        );

        let nullable_run = Field::new(
            "runs",
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", DataType::utf8(), true),
            )
            .unwrap(),
            true,
        );
        assert_eq!(nullable_run.default_value().unwrap(), Scalar::Null);

        let required_values = Field::new(
            "runs",
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", DataType::utf8(), false),
            )
            .unwrap(),
            true,
        );
        assert!(required_values.default_value().is_err());
        assert!(
            Field::new("null", DataType::Null, false)
                .default_value()
                .is_err()
        );
    }

    #[test]
    fn defaults_reject_invalid_or_unbounded_caller_constructed_layouts() {
        for invalid in [
            DataType::Time32(TimeUnit::Nanosecond),
            DataType::Time64(TimeUnit::Millisecond),
            DataType::Duration32(TimeUnit::DayTime),
            DataType::Duration64(TimeUnit::DayTime),
            DataType::Interval(TimeUnit::Second),
            DataType::FixedBinary(0),
            DataType::FixedSizeSerie(
                std::sync::Arc::new(Field::new("item", DataType::Int32, false)),
                -1,
            ),
            DataType::Decimal32 {
                precision: 0,
                scale: 0,
            },
            DataType::Decimal64 {
                precision: 19,
                scale: 0,
            },
            DataType::Decimal128 {
                precision: 39,
                scale: 0,
            },
            DataType::Decimal256 {
                precision: 77,
                scale: 0,
            },
        ] {
            assert!(invalid.default_value().is_err(), "{invalid:?}");
        }
        let too_wide = DataType::fixed_binary(64 * 1024 * 1024 + 1).unwrap();
        let error = too_wide.default_value().unwrap_err().to_string();
        assert!(error.contains("byte safety limit"), "{error}");

        let mut maximum = DataType::Int32;
        for _ in 0..DataType::PARSE_RECURSION_LIMIT - 1 {
            maximum = DataType::serie(Field::new("a.b", maximum, false));
        }
        assert!(maximum.default_value().is_ok());
        let overdeep = DataType::serie(Field::new("a.b", maximum, false));
        let error = overdeep.default_value().unwrap_err().to_string();
        assert!(error.contains("hard limit"), "{error}");

        let large_child = Field::new(
            "item",
            DataType::fixed_binary(40 * 1024 * 1024).unwrap(),
            false,
        );
        let multiplicative = DataType::fixed_size_serie(large_child, 2).unwrap();
        let error = multiplicative.default_value().unwrap_err().to_string();
        assert!(error.contains("byte safety limit"), "{error}");
    }

    #[test]
    fn fatal_default_limits_never_fall_back_to_nullable_nulls() {
        let oversized = DataType::fixed_binary(64 * 1024 * 1024 + 1).unwrap();
        let union = DataType::union(
            [(1, Field::new("oversized", oversized.clone(), true))],
            UnionMode::Dense,
        )
        .unwrap();
        let error = union.default_value().unwrap_err().to_string();
        assert!(error.contains("byte safety limit"), "{error}");

        let encoded = DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", oversized, true),
        )
        .unwrap();
        let error = encoded.default_value().unwrap_err().to_string();
        assert!(error.contains("byte safety limit"), "{error}");

        let fallback = DataType::union(
            [
                (1, Field::new("uninhabited", DataType::Null, false)),
                (2, Field::new("present", DataType::Int32, false)),
            ],
            UnionMode::Dense,
        )
        .unwrap();
        assert_eq!(
            fallback.default_value().unwrap(),
            Scalar::from_sequence([Scalar::from(2), Scalar::from(0)])
        );
    }

    #[test]
    fn null_only_nested_layouts_obey_physical_field_constraints() {
        let zero =
            DataType::fixed_size_serie(Field::new("item", DataType::Null, false), 0).unwrap();
        assert_eq!(zero.default_value().unwrap(), Scalar::from_sequence([]));
        let positive =
            DataType::fixed_size_serie(Field::new("item", DataType::Null, false), 1).unwrap();
        assert!(positive.default_value().is_err());

        let required_null = DataType::from(
            StructType::from_fields([Field::new("nothing", DataType::Null, false)]).unwrap(),
        );
        assert!(required_null.default_value().is_err());
        let optional_null = DataType::from(
            StructType::from_fields([Field::new("nothing", DataType::Null, true)]).unwrap(),
        );
        assert_eq!(
            optional_null
                .default_value()
                .unwrap()
                .as_sequence()
                .unwrap(),
            &[Scalar::Null]
        );

        let no_nullable_branch = DataType::union(
            [(1, Field::new("number", DataType::Int32, false))],
            UnionMode::Sparse,
        )
        .unwrap();
        assert!(
            Field::new("choice", no_nullable_branch, true)
                .default_value()
                .is_err()
        );
    }

    #[test]
    fn opaque_nested_types_reject_malformed_construction_before_defaults() {
        assert!(DataType::dictionary(DataType::Float32, DataType::utf8()).is_err());
        assert!(
            DataType::union([], UnionMode::Dense)
                .unwrap()
                .default_value()
                .is_err()
        );
        assert!(DataType::map(Field::new("entries", DataType::utf8(), false), false,).is_err());
        assert!(
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::UInt32, false),
                Field::new("values", DataType::utf8(), true),
            )
            .is_err()
        );
    }

    #[test]
    fn a_column_is_the_default_exactly_where_its_run_is() -> yggdryl::Result<()> {
        let item = || Field::new("item", DataType::Int64, false);
        let fixed = DataType::fixed_size_serie(item(), 2)?;
        let pair = DataType::from(StructType::from_fields([
            DataType::Int64.required_field("a"),
            DataType::Int64.required_field("b"),
        ])?);
        for values in [[0_i64, 0], [0, 1]] {
            let column = Scalar::from(yggdryl::Serie::from_scalars(
                item(),
                values.map(Scalar::from),
            )?);
            let run = Scalar::from_sequence(values.map(Scalar::from));
            for dtype in [&fixed, &pair] {
                assert_eq!(
                    dtype.is_default_value(&column)?,
                    dtype.is_default_value(&run)?,
                    "{dtype} over {values:?}"
                );
            }
        }
        assert!(
            fixed.is_default_value(&Scalar::from(yggdryl::Serie::from_scalars(
                item(),
                [0_i64, 0].map(Scalar::from),
            )?))?
        );
        Ok(())
    }
}

mod scalars {
    use std::sync::Arc;

    use arrow_array::types::Int8Type;
    use arrow_array::{Array, ArrayRef, DictionaryArray, Int8Array, Int32Array, StringArray};
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, Field, FieldScalar, Scalar, Serie, StructType,
        TimeUnit, Timezone, UnionMode,
    };

    /// Lay one value out as the one-row column `field` types.
    fn lay_out(field: &Field, value: &Scalar) -> yggdryl::Result<ArrayRef> {
        Serie::from_scalars(field.clone(), [value.clone()])?.require_arrow_array()
    }

    /// Read row 0 of an Arrow array back as the value `field` types, the array
    /// taking the door under `options`.
    fn read_back_under(
        field: &Field,
        array: ArrayRef,
        options: ArrowCastOptions,
    ) -> yggdryl::arrow::Result<Scalar> {
        Ok(Serie::from_arrow_array(Some(field), array, options)?.scalar(0)?)
    }

    /// Read row 0 of an Arrow array back under the door's default options.
    fn read_back(field: &Field, array: ArrayRef) -> yggdryl::arrow::Result<Scalar> {
        read_back_under(field, array, ArrowCastOptions::default())
    }

    /// The door's refusing options: a failed conversion is an error rather
    /// than a null.
    fn refusing() -> ArrowCastOptions {
        ArrowCastOptions::new().with_safe(false)
    }

    fn representative_types() -> Vec<DataType> {
        let item = || Field::new("item", DataType::Int32, true);
        vec![
            DataType::Null,
            DataType::Boolean,
            DataType::Int8,
            DataType::UInt64,
            DataType::Float16,
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            },
            DataType::date32(),
            DataType::date64(),
            DataType::Time32(TimeUnit::Second),
            DataType::Time64(TimeUnit::Microsecond),
            DataType::Duration32(TimeUnit::Millisecond),
            DataType::Duration64(TimeUnit::Millisecond),
            DataType::interval(TimeUnit::YearMonth).unwrap(),
            DataType::interval(TimeUnit::DayTime).unwrap(),
            DataType::interval(TimeUnit::MonthDayNano).unwrap(),
            DataType::binary(),
            DataType::fixed_binary(2).unwrap(),
            DataType::large_binary(),
            DataType::binary_view(),
            DataType::from_str("binary(4)").unwrap(),
            DataType::utf8(),
            DataType::large_utf8(),
            DataType::utf8_view(),
            DataType::ascii(),
            DataType::fixed_ascii(4).unwrap(),
            DataType::fixed_utf8(2).unwrap(),
            DataType::from_str("utf8(8)").unwrap(),
            DataType::from_str("string(windows-1252)").unwrap(),
            DataType::large_utf8_view(),
            DataType::ascii_view(),
            DataType::fixed_cp1252(3).unwrap(),
            DataType::from_str("sized_cp1252(8)").unwrap(),
            DataType::serie(item()),
            DataType::serie_view(item()),
            DataType::fixed_size_serie(item(), 2).unwrap(),
            DataType::large_serie(item()),
            DataType::large_serie_view(item()),
            StructType::from_fields([
                Field::new("required", DataType::Int32, false),
                Field::new("optional", DataType::utf8(), true),
            ])
            .map(DataType::from)
            .unwrap(),
            DataType::union(
                [
                    (1, Field::new("nothing", DataType::Null, true)),
                    (4, Field::new("number", DataType::Int32, false)),
                ],
                UnionMode::Dense,
            )
            .unwrap(),
            DataType::dictionary(DataType::Int8, DataType::utf8()).unwrap(),
            DataType::decimal32(7, 2).unwrap(),
            DataType::decimal64(12, 2).unwrap(),
            DataType::decimal128(30, 2).unwrap(),
            DataType::decimal256(50, 2).unwrap(),
            DataType::map_of(DataType::utf8(), DataType::Int32, false).unwrap(),
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", DataType::utf8(), true),
            )
            .unwrap(),
        ]
    }

    #[test]
    fn datatype_defaults_round_trip_through_the_public_scalar_boundary() {
        for dtype in representative_types() {
            let expected = dtype.default_value().unwrap();
            // The default is laid out under the datatype's required field. A
            // required field whose datatype's only default is null has no default
            // - `Field::default_value` refuses it by design - so that datatype's
            // default is laid out under the nullable field, and stays null.
            let holder = if expected.is_null() {
                dtype.clone().nullable_field("value")
            } else {
                dtype.clone().required_field("value")
            };
            let array = Serie::from_default(holder, 1)
                .and_then(|serie| Ok(serie.require_arrow_array()?))
                .unwrap_or_else(|error| panic!("{} Arrow default failed: {error}", dtype.kind()));
            assert_eq!(array.len(), 1);
            // The default reads back through a non-nullable Field: the door lands
            // it under that field, the null datatype's null-only default included.
            let field = Field::new("value", dtype.clone(), false);
            // The Arrow reading spells temporals and decimals with their unit,
            // zone, or scale; the canonical default recognizes both spellings.
            let read = read_back(&field, Arc::clone(&array))
                .unwrap_or_else(|error| panic!("{} read failed: {error}", dtype.kind()));
            assert!(
                dtype.is_default_value(&read).unwrap(),
                "{} read {read:?} is not the default {expected:?}",
                dtype.kind()
            );

            // The same array decodes as a typed pairing under the field, which
            // re-projects to an equal one-row array: the default is closed under
            // both directions. A leaf borrows the field the crate keeps for its
            // datatype; a nested datatype has none and pairs under the local one.
            let shared = dtype.shared_field().unwrap_or(&field);
            let decoded = read_back(shared, Arc::clone(&array))
                .unwrap_or_else(|error| panic!("{} typed decode failed: {error}", dtype.kind()));
            let typed = FieldScalar::new(shared, decoded)
                .unwrap_or_else(|error| panic!("{} typed pairing failed: {error}", dtype.kind()));
            assert_eq!(typed.dtype(), &dtype);
            assert!(
                dtype.is_default_value(typed.value()).unwrap(),
                "{} typed {:?} is not the default {expected:?}",
                dtype.kind(),
                typed.value()
            );
            let reprojected = lay_out(typed.field(), typed.value())
                .unwrap_or_else(|error| panic!("{} reprojection failed: {error}", dtype.kind()));
            assert_eq!(reprojected.as_ref(), array.as_ref());
        }
    }

    #[test]
    fn leaf_defaults_keep_their_declared_physical_identity() {
        let cases = [
            (DataType::decimal32(7, 2).unwrap(), DataTypeId::Decimal32),
            (DataType::decimal64(12, 2).unwrap(), DataTypeId::Decimal64),
            (DataType::large_utf8(), DataTypeId::LargeUtf8String),
            (DataType::utf8_view(), DataTypeId::Utf8StringView),
            (DataType::fixed_binary(3).unwrap(), DataTypeId::FixedBinary),
            (DataType::large_binary(), DataTypeId::LargeBinary),
            (DataType::binary_view(), DataTypeId::BinaryView),
            (DataType::ascii(), DataTypeId::AsciiString),
            (
                DataType::fixed_ascii(4).unwrap(),
                DataTypeId::FixedAsciiString,
            ),
            (DataType::cp1252(), DataTypeId::Cp1252String),
            (
                DataType::fixed_cp1252(3).unwrap(),
                DataTypeId::FixedCp1252String,
            ),
            (DataType::Country, DataTypeId::Country),
            (DataType::Ccy, DataTypeId::Ccy),
            (DataType::MicCode, DataTypeId::MicCode),
            (DataType::CfiCode, DataTypeId::CfiCode),
            (DataType::Uuid, DataTypeId::Uuid),
            (
                DataType::interval(TimeUnit::YearMonth).unwrap(),
                DataTypeId::Interval,
            ),
            (
                DataType::interval(TimeUnit::DayTime).unwrap(),
                DataTypeId::Interval,
            ),
            (
                DataType::interval(TimeUnit::MonthDayNano).unwrap(),
                DataTypeId::Interval,
            ),
            (DataType::geometry(None).unwrap(), DataTypeId::Geometry),
            (
                DataType::geography(None, None).unwrap(),
                DataTypeId::Geography,
            ),
        ];

        for (dtype, id) in cases {
            let value = dtype
                .default_value()
                .unwrap_or_else(|error| panic!("{dtype} default failed: {error}"));
            assert_eq!(value.id(), id, "default for {dtype}");
            assert_eq!(value.dtype().unwrap().id(), id, "inference for {dtype}");
        }
    }

    #[test]
    fn field_defaults_preserve_exact_identity_and_nullability() {
        let field = Field::from_parts(
            "price",
            DataType::decimal128(9, 2).unwrap(),
            true,
            [("ARROW:extension:name", "example.price")],
        )
        .unwrap();
        let array = Serie::from_default(field.clone(), 1)
            .unwrap()
            .require_arrow_array()
            .unwrap();
        assert_eq!(array.len(), 1);
        assert!(array.is_null(0));
        assert_eq!(read_back(&field, array).unwrap(), Scalar::Null);

        assert!(Serie::from_default(Field::new("never", DataType::Null, false), 1).is_err());
    }

    #[test]
    fn foreign_arrays_are_cast_and_refuse_a_missing_row_or_a_required_null() {
        let array = Serie::from_default(DataType::Int32.required_field("value"), 1)
            .unwrap()
            .require_arrow_array()
            .unwrap();
        // Another layout is cast into the field rather than refused: an int32
        // widens into an int64 exactly...
        assert_eq!(
            read_back(
                &Field::new("value", DataType::Int64, false),
                Arc::clone(&array)
            )
            .unwrap(),
            Scalar::from(0_i64)
        );
        // ...and a layout no cast reaches is refused, naming it.
        let record = StructType::from_fields([DataType::Int32.required_field("a")])
            .map(DataType::from)
            .unwrap();
        let error = read_back(&Field::new("value", record, false), Arc::clone(&array))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("unsupported Arrow record datatype struct"),
            "{error}"
        );
        // Zero rows land as the empty column, which holds no row to read.
        let error = read_back(
            &Field::new("value", DataType::Int32, false),
            array.slice(0, 0),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("row 0 is past the 0 rows"), "{error}");

        // A null under a required field is an absent row, refused by path
        // whatever `safe` says - never repaired to the field's default.
        let nullable_child = Serie::from_default(Field::new("child", DataType::Int32, true), 1)
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let required = Field::new("child", DataType::Int32, false);
        for error in [
            read_back_under(&required, Arc::clone(&nullable_child), refusing()),
            read_back(&required, nullable_child),
        ] {
            assert_eq!(
                error.unwrap_err().to_string(),
                "required Arrow field $.child holds 1 null values"
            );
        }
        assert!(lay_out(&required, &Scalar::Null).is_err());
    }

    #[test]
    fn null_valued_wrappers_round_trip_but_a_selected_null_member_does_not() {
        // A dictionary or a run-end pair over the null datatype holds its null
        // default as its one value, and it reads back through a non-nullable
        // Field. A union selecting its null member holds a row the required
        // field does not accept, so under that field it is an absent row,
        // refused by path.
        let dictionary = DataType::dictionary(DataType::Int8, DataType::Null).unwrap();
        let union = DataType::union(
            [(5, Field::new("nothing", DataType::Null, true))],
            UnionMode::Dense,
        )
        .unwrap();
        let encoded = DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int16, false),
            Field::new("values", DataType::Null, true),
        )
        .unwrap();
        let refusal = "required Arrow field $.value holds 1 null values";
        for (dtype, refused) in [(dictionary, None), (union, Some(refusal)), (encoded, None)] {
            let expected = dtype.default_value().unwrap();
            // A required field whose datatype's only default is null has no
            // default - `Field::default_value` refuses it by design - so the
            // null-only default is laid out under the nullable field, where it is
            // logically null.
            assert!(Serie::from_default(dtype.clone().required_field("value"), 1).is_err());
            let array = Serie::from_default(dtype.clone().nullable_field("value"), 1)
                .unwrap()
                .require_arrow_array()
                .unwrap();
            let field = Field::new("value", dtype.clone(), false);
            let holder = field.clone().with_nullable(true);
            assert_eq!(read_back(&holder, Arc::clone(&array)).unwrap(), expected);
            match refused {
                None => assert_eq!(read_back(&field, Arc::clone(&array)).unwrap(), expected),
                Some(refusal) => {
                    for options in [ArrowCastOptions::default(), refusing()] {
                        let error =
                            read_back_under(&field, Arc::clone(&array), options).unwrap_err();
                        assert_eq!(error.to_string(), refusal, "{dtype}");
                    }
                }
            }
            // A typed pairing is the field's own contract: it refuses the
            // null-only default under the non-nullable Field, holds it under the
            // nullable one, and lays out exactly what the default laid out.
            assert!(FieldScalar::new(&field, expected.clone()).is_err());
            let typed = FieldScalar::new(&holder, expected.clone()).unwrap();
            assert_eq!(
                lay_out(typed.field(), typed.value()).unwrap().as_ref(),
                array.as_ref()
            );
        }

        let union = DataType::union(
            [
                (1, Field::new("present", DataType::Int32, false)),
                (9, Field::new("absent", DataType::utf8(), true)),
            ],
            UnionMode::Dense,
        )
        .unwrap();
        let nullable = Field::new("choice", union.clone(), true);
        let selected_null = Serie::from_default(nullable.clone(), 1)
            .unwrap()
            .require_arrow_array()
            .unwrap();
        assert_ne!(
            read_back(&nullable, Arc::clone(&selected_null)).unwrap(),
            union.default_value().unwrap()
        );
        // Under a required field the selected null is an absent row, refused
        // by path - never read back as itself nor as the union's default.
        let required = Field::new("choice", union, false);
        for options in [ArrowCastOptions::default(), refusing()] {
            let error = read_back_under(&required, Arc::clone(&selected_null), options)
                .unwrap_err()
                .to_string();
            assert_eq!(error, "required Arrow field $.choice holds 1 null values");
        }
    }

    #[test]
    fn nullable_dictionary_null_keys_decode_as_native_null() {
        let dtype = DataType::dictionary(DataType::Int8, DataType::utf8()).unwrap();
        let array: ArrayRef = Arc::new(
            DictionaryArray::<Int8Type>::try_new(
                Int8Array::from(vec![None]),
                Arc::new(StringArray::from(vec!["not-null"])),
            )
            .unwrap(),
        );
        assert_eq!(
            read_back(&Field::new("encoded", dtype, true), array).unwrap(),
            Scalar::Null
        );
    }

    #[test]
    fn foreign_arrays_preflight_deep_caller_built_schemas_before_arrow_projection() {
        let mut maximum = DataType::Int32;
        for _ in 0..DataType::PARSE_RECURSION_LIMIT - 1 {
            maximum = DataType::serie(Field::new("item", maximum, false));
        }
        let array = Serie::from_default(maximum.clone().required_field("value"), 1)
            .unwrap()
            .require_arrow_array()
            .unwrap();
        read_back(&Field::new("value", maximum.clone(), false), array).unwrap();

        let overdeep = DataType::serie(Field::new("item", maximum, false));
        let unrelated: ArrayRef = Arc::new(Int32Array::from(vec![0]));
        let error = read_back(&Field::new("value", overdeep, false), unrelated)
            .unwrap_err()
            .to_string();
        assert!(error.contains("hard limit"), "{error}");
    }
}
