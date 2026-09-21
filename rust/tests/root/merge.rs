//! `rust/src/merge.rs`: the exact widening lattice under `merge_with`.
//!
//! The two pins that reach `DataType::merge_exact`, which is private: it is
//! the same lattice as [`DataType::merge_with`] up to the point where that
//! one would answer text or bytes for a pair that is neither, and an
//! integration test cannot see it. Every other merge contract lives in
//! `rust/tests/types/merge.rs`.

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
