//! What two datatypes meet as: the widening lattice `DataType::merge_with`
//! walks, in both directions.
use yggdryl::DataType;
use yggdryl::types::DecimalType;
use yggdryl::types::UuidType;

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
        (
            DataType::Uuid(UuidType::Uuid),
            DataType::fixed_binary(16).unwrap(),
        ),
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
        down(&DataType::Cfi, &DataType::fixed_ascii(6).unwrap()),
        DataType::Cfi
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
    assert_eq!(
        down(&dtype("string(latin1)"), &dtype("string(windows-1252)")),
        dtype("string(latin1)")
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
