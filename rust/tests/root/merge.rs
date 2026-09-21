//! `rust/src/merge.rs`: the exact widening lattice under `merge_with`.
//!
//! The two pins that reach `DataType::merge_exact`, which is private: it is
//! the same lattice as [`DataType::merge_with`] up to the point where that
//! one would answer text or bytes for a pair that is neither, and an
//! integration test cannot see it. Every other merge contract lives beside
//! them here.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::DataType;
    use yggdryl::Widening;
    use yggdryl::internals::merge::merge_exact;

    #[test]
    fn fixed_widths_meet_text_by_width_in_the_direction_asked_for() {
        let up = |left: &DataType, right: &DataType| left.merge_with(right, true).unwrap();
        let down = |left: &DataType, right: &DataType| left.merge_with(right, false).unwrap();

        assert_eq!(
            up(&DataType::fixed_ascii(4).unwrap(), &DataType::utf8()),
            DataType::utf8()
        );
        assert_eq!(
            down(&DataType::fixed_ascii(4).unwrap(), &DataType::utf8()),
            DataType::fixed_ascii(4).unwrap()
        );
        // A value is never padded to a wider slot, so two widths that
        // disagree meet at the wider one as a maximum.
        assert_eq!(
            up(
                &DataType::fixed_ascii(4).unwrap(),
                &DataType::fixed_ascii(8).unwrap()
            ),
            DataType::sized_ascii(8).unwrap()
        );
        assert_eq!(
            down(
                &DataType::fixed_ascii(4).unwrap(),
                &DataType::fixed_ascii(8).unwrap()
            ),
            DataType::fixed_ascii(4).unwrap()
        );
        assert_eq!(
            up(&DataType::large_utf8(), &DataType::fixed_ascii(16).unwrap()),
            DataType::large_utf8()
        );
        assert_eq!(
            down(&DataType::large_utf8(), &DataType::fixed_ascii(16).unwrap()),
            DataType::fixed_ascii(16).unwrap()
        );
        assert_eq!(
            merge_exact(
                &DataType::fixed_ascii(4).unwrap(),
                &DataType::utf8(),
                Widening::Up
            )
            .unwrap(),
            DataType::utf8()
        );
    }

    #[test]
    fn a_fixed_string_absorbs_a_number_at_no_less_than_utf8_and_only_when_allowed() {
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .merge_with(&DataType::Int32, true)
                .unwrap(),
            DataType::utf8()
        );
        assert_eq!(
            DataType::Int32
                .merge_with(&DataType::fixed_ascii(4).unwrap(), false)
                .unwrap(),
            DataType::utf8()
        );
        let refused = merge_exact(
            &DataType::fixed_ascii(4).unwrap(),
            &DataType::Int32,
            Widening::Up,
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("fixed_ascii(4)"), "{refused}");
        assert!(refused.contains("int32"), "{refused}");
    }
}

mod nested {
    use yggdryl::{DataType, Field, StructType};

    #[test]
    fn two_datatypes_meet_at_the_one_that_holds_both() {
        let up = |left: &DataType, right: &DataType| left.merge_with(right, true).unwrap();
        let down = |left: &DataType, right: &DataType| left.merge_with(right, false).unwrap();

        // Width resolves in the direction asked for, and only in that direction.
        assert_eq!(up(&DataType::Int32, &DataType::Int64), DataType::Int64);
        assert_eq!(down(&DataType::Int32, &DataType::Int64), DataType::Int32);
        assert_eq!(
            up(&DataType::Float32, &DataType::Float64),
            DataType::Float64
        );
        assert_eq!(
            down(&DataType::utf8(), &DataType::large_utf8()),
            DataType::utf8()
        );

        // A null column has no shape, so it takes the other's in either position
        // and in either direction.
        assert_eq!(up(&DataType::Null, &DataType::utf8()), DataType::utf8());
        assert_eq!(down(&DataType::Int64, &DataType::Null), DataType::Int64);

        // Bytes hold every other encoding, and text holds all but bytes.
        assert_eq!(
            up(&DataType::utf8(), &DataType::binary()),
            DataType::binary()
        );
        assert_eq!(
            up(&DataType::Int64, &DataType::binary()),
            DataType::binary()
        );
        assert_eq!(up(&DataType::Int64, &DataType::utf8()), DataType::utf8());
        assert_eq!(up(&DataType::Boolean, &DataType::utf8()), DataType::utf8());
        assert_eq!(up(&DataType::date32(), &DataType::utf8()), DataType::utf8());

        // A pair with no meeting point that is not a re-encoding is refused, and
        // the refusal names both sides.
        let refused = DataType::Boolean
            .merge_with(&DataType::Int64, true)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("boolean") || refused.contains("int64"),
            "{refused}"
        );
        assert!(
            DataType::Float64
                .merge_with(&DataType::decimal128(10, 2).unwrap(), true)
                .is_err(),
            "an exact number and an approximate one have no honest meeting point"
        );
    }

    #[test]
    fn merging_structs_takes_the_union_of_their_fields() {
        let left = StructType::from_fields([
            DataType::Int32.required_field("id"),
            DataType::utf8().required_field("venue"),
        ])
        .map(DataType::from)
        .unwrap();
        let right = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Float64.required_field("price"),
        ])
        .map(DataType::from)
        .unwrap();

        let merged = left.merge_with(&right, true).unwrap();
        assert_eq!(merged.field_len(), 3);

        // A name both carry merges, and stays required because both require it.
        assert_eq!(merged["id"].dtype(), &DataType::Int64);
        assert!(!merged["id"].is_nullable());

        // A name only one side carries becomes nullable: the rows the other side
        // described do not have it.
        assert!(merged["venue"].is_nullable());
        assert!(merged["price"].is_nullable());

        // Order is the receiver's, with additions appended, so a merge never
        // reorders columns a caller already depends on.
        assert_eq!(merged[0].name(), "id");
        assert_eq!(merged[1].name(), "venue");
        assert_eq!(merged[2].name(), "price");

        // Merging a schema with itself changes nothing.
        assert_eq!(merged.merge_with(&merged, true).unwrap(), merged);
    }

    #[test]
    fn merging_reaches_into_every_nested_layout() {
        // Lists merge their item.
        assert_eq!(
            DataType::list(DataType::Int32.nullable_field("item"))
                .merge_with(
                    &DataType::list(DataType::Int64.nullable_field("item")),
                    true
                )
                .unwrap(),
            DataType::list(DataType::Int64.nullable_field("item")),
        );

        // Maps merge through their entries.
        assert_eq!(
            DataType::map_of(DataType::utf8(), DataType::Int32, true)
                .unwrap()
                .merge_with(
                    &DataType::map_of(DataType::utf8(), DataType::Int64, true).unwrap(),
                    true,
                )
                .unwrap(),
            DataType::map_of(DataType::utf8(), DataType::Int64, true).unwrap(),
        );

        // And the recursion goes all the way down.
        let deep = |inner: DataType| {
            StructType::from_fields([StructType::from_fields([inner.required_field("n")])
                .map(DataType::from)
                .unwrap()
                .required_field("in")])
            .map(DataType::from)
            .unwrap()
        };
        let merged = deep(DataType::Int32)
            .merge_with(&deep(DataType::Int64), true)
            .unwrap();
        assert_eq!(merged[r#""in""#]["n"].dtype(), &DataType::Int64);
    }

    #[test]
    fn merging_fields_unions_metadata_and_keeps_the_receivers_value() {
        let mut left = Field::new("price", DataType::Int32, false);
        left.set_metadata([("owner", "left"), ("only_left", "1")])
            .unwrap();
        let mut right = Field::new("price", DataType::Int64, true);
        right
            .set_metadata([("owner", "right"), ("only_right", "2")])
            .unwrap();

        let merged = left.merge_with(&right, true).unwrap();
        assert_eq!(merged.dtype(), &DataType::Int64);

        // Either side being nullable carries over: a value absent from one of two
        // sources is absent from their union.
        assert!(merged.is_nullable());

        // Every key arrives, and the receiver wins the one they disagree on.
        assert_eq!(merged.get_metadata("owner"), Some("left"));
        assert_eq!(merged.get_metadata("only_left"), Some("1"));
        assert_eq!(merged.get_metadata("only_right"), Some("2"));

        // Merging a field with itself changes nothing.
        assert_eq!(merged.merge_with(&merged, true).unwrap(), merged);
    }
}

mod lattice {
    use yggdryl::DataType;
    use yggdryl::DecimalType;

    #[test]
    fn bytes_win_over_text_and_keep_only_an_identical_fixed_width() {
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .merge_with(&DataType::fixed_binary(4).unwrap(), true)
                .unwrap(),
            DataType::fixed_binary(4).unwrap()
        );
        assert_eq!(
            DataType::fixed_binary(8)
                .unwrap()
                .merge_with(&DataType::fixed_ascii(4).unwrap(), true)
                .unwrap(),
            DataType::binary()
        );
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .merge_with(&DataType::binary(), true)
                .unwrap(),
            DataType::binary()
        );
        // Text merges into bytes, and the byte side keeps the maximum it
        // declares: the text side states no width to widen it past.
        assert_eq!(
            DataType::from_str("sized_binary(16)")
                .unwrap()
                .merge_with(&DataType::utf8(), true)
                .unwrap(),
            DataType::sized_binary(16).unwrap()
        );
    }

    #[test]
    fn a_number_never_shares_a_fixed_byte_width() {
        // Four bytes of `int32` are an encoding, not a slot the bytes side
        // stores, so the pair is variable bytes in either direction.
        let fixed = DataType::fixed_binary(4).unwrap();
        assert_eq!(
            DataType::Int32.merge_with(&fixed, true).unwrap(),
            DataType::binary()
        );
        assert_eq!(
            fixed.merge_with(&DataType::Int32, false).unwrap(),
            DataType::binary()
        );
    }

    #[test]
    fn two_byte_types_meet_parameter_by_parameter() {
        let up = |left: &str, right: &str| {
            DataType::from_str(left)
                .unwrap()
                .merge_with(&DataType::from_str(right).unwrap(), true)
                .unwrap()
                .to_string()
        };
        let down = |left: &str, right: &str| {
            DataType::from_str(left)
                .unwrap()
                .merge_with(&DataType::from_str(right).unwrap(), false)
                .unwrap()
                .to_string()
        };
        // Widening: the wider offsets, a view only beside a view, the
        // variable layout over a fixed one, no bound over a bound.
        assert_eq!(up("binary", "large_binary"), "large_binary");
        assert_eq!(up("binary_view", "binary"), "binary");
        assert_eq!(up("binary_view", "large_binary"), "large_binary");
        assert_eq!(up("fixed_binary(4)", "binary_view"), "binary_view");
        assert_eq!(up("fixed_binary(4)", "sized_binary(2)"), "sized_binary(2)");
        assert_eq!(up("fixed_binary(4)", "fixed_binary(8)"), "sized_binary(8)");
        assert_eq!(up("sized_binary(16)", "binary"), "binary");
        assert_eq!(
            up("sized_binary(16)", "sized_binary(32)"),
            "sized_binary(32)"
        );
        // Narrowing: the mirror.
        assert_eq!(down("binary", "large_binary"), "binary");
        assert_eq!(down("binary_view", "large_binary"), "binary");
        assert_eq!(down("fixed_binary(4)", "binary_view"), "fixed_binary(4)");
        assert_eq!(
            down("fixed_binary(4)", "fixed_binary(8)"),
            "fixed_binary(4)"
        );
        assert_eq!(down("sized_binary(16)", "binary"), "sized_binary(16)");
        assert_eq!(
            down("sized_binary(16)", "sized_binary(32)"),
            "sized_binary(16)"
        );
    }

    #[test]
    fn narrowing_keeps_the_type_that_constrains_a_shared_fixed_width() {
        // The storage is the same either way, so the direction is free to
        // answer the tighter of the two types.
        for (left, right) in [
            (
                DataType::fixed_ascii(4).unwrap(),
                DataType::fixed_binary(4).unwrap(),
            ),
            (DataType::Uuid, DataType::fixed_binary(16).unwrap()),
        ] {
            assert_eq!(
                left.merge_with(&right, false).unwrap(),
                left,
                "narrowing answers {left}"
            );
            assert_eq!(
                right.merge_with(&left, false).unwrap(),
                left,
                "in either position"
            );
            assert_eq!(
                left.merge_with(&right, true).unwrap(),
                right,
                "widening answers the bytes"
            );
        }

        // A width neither side shares is variable bytes, as before.
        assert_eq!(
            DataType::uuid()
                .merge_with(&DataType::fixed_binary(8).unwrap(), true)
                .unwrap(),
            DataType::binary()
        );

        // A code shares no fixed width with anything: its own width bounds
        // variable text, so bytes beside it are variable bytes either way.
        for how in [true, false] {
            assert_eq!(
                DataType::Currency
                    .merge_with(&DataType::fixed_binary(3).unwrap(), how)
                    .unwrap(),
                DataType::binary()
            );
        }
    }

    #[test]
    fn narrowing_keeps_a_registered_code_over_the_width_it_stores_in() {
        let up = |left: &DataType, right: &DataType| left.merge_with(right, true).unwrap();
        let down = |left: &DataType, right: &DataType| left.merge_with(right, false).unwrap();

        // The code is the tighter type, so narrowing answers it in either
        // position and widening answers the shape that holds both.
        for other in [
            DataType::fixed_ascii(3).unwrap(),
            DataType::ascii(),
            DataType::utf8(),
            DataType::large_utf8(),
        ] {
            assert_eq!(down(&DataType::Currency, &other), DataType::Currency);
            assert_eq!(down(&other, &DataType::Currency), DataType::Currency);
            assert_ne!(up(&DataType::Currency, &other), DataType::Currency);
        }
        assert_eq!(
            down(&DataType::CfiCode, &DataType::fixed_ascii(6).unwrap()),
            DataType::CfiCode
        );

        // A side narrower than the code still outranks it: narrowing is the
        // tightest type that names both, not the most specific one.
        assert_eq!(
            down(&DataType::Currency, &DataType::fixed_ascii(2).unwrap()),
            DataType::fixed_ascii(2).unwrap()
        );

        // Two different codes are the pair neither direction answers with a
        // code: neither standard names the other's values, so the answer is
        // the bounded ASCII text both store as.
        assert_eq!(
            down(&DataType::Currency, &DataType::Country),
            DataType::from_str("ascii(2)").unwrap()
        );
        assert_eq!(
            up(&DataType::Currency, &DataType::Country),
            DataType::from_str("ascii(3)").unwrap()
        );

        // A number's rendering does not fit a code, so absorbing one is still
        // no less than `utf8`.
        assert_eq!(
            down(&DataType::Currency, &DataType::Int32),
            DataType::utf8()
        );
    }

    #[test]
    fn widening_a_decimal_keeps_the_widest_backing_either_side_declared() {
        let decimal128 = DataType::decimal128(10, 2).unwrap();
        let decimal256 = DataType::Decimal(DecimalType::Decimal256 {
            precision: 20,
            scale: 2,
        });

        // The merged precision fits a narrower backing, but re-encoding the
        // storage is not something a widening merge may impose.
        assert_eq!(
            decimal128.merge_with(&DataType::Int16, true).unwrap(),
            decimal128
        );
        assert_eq!(
            decimal256
                .merge_with(&DataType::decimal128(20, 2).unwrap(), true)
                .unwrap(),
            decimal256
        );
        assert_eq!(
            decimal256.merge_with(&DataType::Int32, true).unwrap(),
            decimal256
        );

        // A precision wider than either backing still moves up to the one
        // that holds it.
        assert_eq!(
            DataType::decimal32(9, 2)
                .unwrap()
                .merge_with(&DataType::Int32, true)
                .unwrap(),
            DataType::decimal64(12, 2).unwrap()
        );

        // Narrowing wants the tightest type, so it takes what precision needs.
        assert_eq!(
            decimal128.merge_with(&DataType::Int16, false).unwrap(),
            DataType::decimal32(8, 0).unwrap()
        );
        assert_eq!(
            decimal256
                .merge_with(&DataType::decimal128(20, 2).unwrap(), false)
                .unwrap(),
            DataType::decimal128(20, 2).unwrap()
        );
    }

    #[test]
    fn strings_meet_parameter_by_parameter() {
        let up = |left: &DataType, right: &DataType| left.merge_with(right, true).unwrap();
        let down = |left: &DataType, right: &DataType| left.merge_with(right, false).unwrap();
        let dtype = |spelling: &str| DataType::from_str(spelling).unwrap();

        // Offsets widen and narrow; a view stays a view only beside a view.
        assert_eq!(
            up(&DataType::utf8(), &DataType::large_utf8()),
            DataType::large_utf8()
        );
        assert_eq!(
            down(&DataType::utf8(), &DataType::large_utf8()),
            DataType::utf8()
        );
        assert_eq!(
            up(&DataType::utf8_view(), &DataType::utf8()),
            DataType::utf8()
        );
        assert_eq!(
            up(&DataType::utf8_view(), &dtype("large_utf8_view")),
            dtype("large_utf8_view")
        );
        assert_eq!(
            down(&DataType::utf8_view(), &dtype("large_utf8_view")),
            DataType::utf8_view()
        );

        // Two charsets widen to UTF-8 and narrow to the smaller repertoire.
        assert_eq!(
            up(&DataType::ascii(), &dtype("string(windows-1252)")),
            DataType::utf8()
        );
        assert_eq!(
            down(&DataType::utf8(), &dtype("string(windows-1252)")),
            dtype("string(windows-1252)")
        );
        assert_eq!(
            down(&dtype("string(windows-1252)"), &DataType::ascii()),
            DataType::ascii()
        );

        // No bound beats a maximum widening; the smaller bound wins narrowing.
        assert_eq!(up(&dtype("utf8(32)"), &DataType::utf8()), DataType::utf8());
        assert_eq!(up(&dtype("utf8(32)"), &dtype("utf8(8)")), dtype("utf8(32)"));
        assert_eq!(
            down(&dtype("utf8(32)"), &DataType::utf8()),
            dtype("utf8(32)")
        );
        assert_eq!(
            down(&dtype("utf8(32)"), &dtype("utf8(8)")),
            dtype("utf8(8)")
        );
        assert_eq!(
            up(&dtype("fixed_utf8(40)"), &dtype("utf8(32)")),
            dtype("utf8(40)")
        );
        assert_eq!(
            down(&dtype("fixed_utf8(40)"), &dtype("utf8(32)")),
            dtype("fixed_utf8(32)")
        );
    }
}
