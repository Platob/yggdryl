//! Correctness of the Phase 10 in-place copy-on-write mutation: every `*_assign` twin must equal
//! its return-new sibling **exactly** (byte-identical), `retain` must equal `filter`, `fill_null_mut`
//! must equal `fill_null`, `deep_copy` must equal the original value, and — the load-bearing value
//! guarantee — a shallow copy mutated in place must leave the OTHER side untouched (COW isolation).

use yggdryl_core::io::fixed::{Field, Scalar, Serie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnyScalar, DataTypeId};

// ---- typed Serie<T>: in-place arithmetic == return-new, byte-identical -----------------------

/// Every `*_assign` twin equals the matching return-new `*_unchecked` (value AND serialized bytes),
/// for both operand orders of nulls and the div/rem zero-divisor path.
fn check_binary_i32(a: &Serie<i32>, b: &Serie<i32>) {
    macro_rules! one {
        ($assign:ident, $ret:ident) => {{
            let mut x = a.clone();
            x.$assign(b);
            let expected = a.$ret(b);
            assert_eq!(x, expected, concat!(stringify!($assign), " value"));
            assert_eq!(
                x.serialize_bytes(),
                expected.serialize_bytes(),
                concat!(stringify!($assign), " bytes")
            );
            // The left operand `a` is never mutated by the in-place op on its shallow clone.
        }};
    }
    one!(add_assign, add_unchecked);
    one!(sub_assign, sub_unchecked);
    one!(mul_assign, mul_unchecked);
    one!(div_assign, div_unchecked);
    one!(rem_assign, rem_unchecked);
}

#[test]
fn assign_equals_return_new_across_nulls_and_zero_divisors() {
    // No nulls.
    check_binary_i32(
        &Serie::from_values(&[6, 7, 8, 100]),
        &Serie::from_values(&[2, 3, 4, 7]),
    );
    // Nulls in the left, the right, both.
    check_binary_i32(
        &Serie::from_options(&[Some(6), None, Some(8), None]),
        &Serie::from_options(&[Some(2), Some(3), None, None]),
    );
    // Zero divisors (div/rem → null) mixed with present values.
    check_binary_i32(
        &Serie::from_values(&[6, 7, 8]),
        &Serie::from_values(&[2, 0, 4]),
    );
    // Empty.
    check_binary_i32(&Serie::from_values(&[]), &Serie::from_values(&[]));
}

#[test]
fn assign_wrapping_overflow_matches_return_new() {
    // i8 127 + 1 wraps to -128, both in-place and return-new.
    let a = Serie::from_values(&[127i8, -128, 100]);
    let b = Serie::from_values(&[1i8, -1, 100]);
    let mut x = a.clone();
    x.add_assign(&b);
    assert_eq!(x, a.add_unchecked(&b));
    assert_eq!(x.to_options(), [Some(-128), Some(127), Some(-56)]);

    // i128::MIN / -1 wraps (no panic) — same as return-new.
    let a = Serie::from_values(&[i128::MIN, 10]);
    let b = Serie::from_values(&[-1i128, 3]);
    let mut x = a.clone();
    x.div_assign(&b);
    assert_eq!(x, a.div_unchecked(&b));
}

#[test]
fn scalar_assign_equals_return_new() {
    let col = Serie::from_options(&[Some(1i64), None, Some(3), Some(7)]);
    macro_rules! one {
        ($assign:ident, $ret:ident, $v:expr) => {{
            let mut x = col.clone();
            x.$assign($v);
            assert_eq!(x, col.$ret($v));
            assert_eq!(x.serialize_bytes(), col.$ret($v).serialize_bytes());
        }};
    }
    one!(add_scalar_assign, add_scalar_unchecked, 10);
    one!(sub_scalar_assign, sub_scalar_unchecked, 10);
    one!(mul_scalar_assign, mul_scalar_unchecked, 10);
    one!(div_scalar_assign, div_scalar_unchecked, 2);
    one!(rem_scalar_assign, rem_scalar_unchecked, 2);
    // Zero-divisor broadcast nulls every present cell — same as return-new.
    one!(div_scalar_assign, div_scalar_unchecked, 0);
    one!(rem_scalar_assign, rem_scalar_unchecked, 0);
}

// ---- reshape twins: retain == filter, fill_null_mut == fill_null -----------------------------

#[test]
fn retain_equals_filter() {
    for col in [
        Serie::from_values(&[1i32, 2, 3, 4, 5]),
        Serie::from_options(&[Some(1i32), None, Some(3), None, Some(5)]),
    ] {
        for mask in [
            vec![true, true, false, true, false],
            vec![false, false, false, false, false], // drop all
            vec![true, true, true, true, true],      // keep all
        ] {
            let mut retained = col.clone();
            retained.retain(&mask).unwrap();
            assert_eq!(retained, col.filter(&mask).unwrap());
            assert_eq!(
                retained.serialize_bytes(),
                col.filter(&mask).unwrap().serialize_bytes()
            );
        }
    }
    // A length-mismatched mask errors and leaves the column unchanged.
    let mut col = Serie::from_values(&[1i32, 2, 3]);
    assert!(col.retain(&[true, false]).is_err());
    assert_eq!(col.to_options(), [Some(1), Some(2), Some(3)]);
}

#[test]
fn fill_null_mut_equals_fill_null() {
    let col = Serie::from_options(&[Some(1i32), None, Some(3), None]);
    let mut filled = col.clone();
    filled.fill_null_mut(0);
    assert_eq!(filled, col.fill_null(0));
    assert_eq!(filled.null_count(), 0);
    // No-null column: unchanged.
    let dense = Serie::from_values(&[1i32, 2, 3]);
    let mut d = dense.clone();
    d.fill_null_mut(9);
    assert_eq!(d, dense);
}

// ---- ByteSerie reshape twins ----------------------------------------------------------------

#[test]
fn bytes_retain_and_fill_null_mut_equal_return_new() {
    let col = Utf8Serie::from_strs(&[Some("a"), None, Some("cd"), Some("e")]);
    let mask = [true, true, false, true];
    let mut retained = col.clone();
    retained.retain(&mask).unwrap();
    assert_eq!(retained, col.filter(&mask).unwrap());

    let mut filled = col.clone();
    filled.fill_null_mut(b"X").unwrap();
    assert_eq!(filled, col.fill_null_bytes(b"X").unwrap());
    assert_eq!(filled.null_count(), 0);
}

// ---- deep_copy: equal value, fully independent ----------------------------------------------

#[test]
fn deep_copy_equals_original_every_family() {
    let s = Serie::from_options(&[Some(1i32), None, Some(3)]);
    assert_eq!(s.deep_copy(), s);
    let u = Utf8Serie::from_strs(&[Some("a"), None, Some("cd")]);
    assert_eq!(u.deep_copy(), u); // Vec-backed: deep_copy == clone
    assert_eq!(Scalar::of(42i32).deep_copy(), Scalar::of(42));
    assert_eq!(Scalar::<i32>::null().deep_copy(), Scalar::null());
}

#[test]
fn deep_copy_is_independent_of_the_original() {
    let a = Serie::from_values(&[1i32, 2, 3]);
    let mut d = a.deep_copy();
    d.add_scalar_assign(100); // mutate the deep copy in place
    assert_eq!(a.to_options(), [Some(1), Some(2), Some(3)]); // original untouched
    assert_eq!(d.to_options(), [Some(101), Some(102), Some(103)]);
}

// ---- COW isolation: the load-bearing value guarantee ----------------------------------------

#[test]
fn shallow_copy_mutated_in_place_leaves_the_other_side_unchanged() {
    let original = Serie::from_options(&[Some(1i64), None, Some(3), Some(4)]);
    // Two shallow copies sharing the same Arc buffer.
    let keep = original.clone();
    let mut mutated = original.clone();

    // Serie×serie in place.
    mutated.add_assign(&Serie::from_values(&[10i64, 10, 10, 10]));
    assert_eq!(original.to_options(), [Some(1), None, Some(3), Some(4)]);
    assert_eq!(keep.to_options(), [Some(1), None, Some(3), Some(4)]);
    assert_eq!(mutated.to_options(), [Some(11), None, Some(13), Some(14)]);

    // Scalar broadcast, fill_null_mut, retain on a fresh shallow copy — each COWs, never touching
    // the shared original.
    let mut m2 = original.clone();
    m2.mul_scalar_assign(2);
    assert_eq!(original.to_options(), [Some(1), None, Some(3), Some(4)]);

    let mut m3 = original.clone();
    m3.fill_null_mut(0);
    assert_eq!(original.to_options(), [Some(1), None, Some(3), Some(4)]);
    assert_eq!(m3.to_options(), [Some(1), Some(0), Some(3), Some(4)]);

    let mut m4 = original.clone();
    m4.retain(&[true, false, true, false]).unwrap();
    assert_eq!(original.to_options(), [Some(1), None, Some(3), Some(4)]);
    assert_eq!(m4.to_options(), [Some(1), Some(3)]);
}

// ---- erased dyn AnySerie mirror -------------------------------------------------------------

fn i32_scalar(v: i32) -> AnyScalar {
    AnyScalar::leaf(
        Field::of("", DataTypeId::I32, 4, false),
        v.to_le_bytes().to_vec(),
    )
}

#[test]
fn erased_assign_matches_return_new_and_follows_left_type() {
    // i64 += i32 (right cast into the left's i64); result follows the left.
    let mut a = boxed(Serie::from_values(&[1i64, 2, 3]));
    let b = boxed(Serie::from_values(&[10i32, 20, 30]));
    let expected = a.add(b.as_ref()).unwrap();
    a.add_assign(b.as_ref()).unwrap();
    assert_eq!(a.type_id(), DataTypeId::I64);
    assert!(a.eq_any(expected.as_ref()));

    // Every serie×serie op through the erased surface equals the return-new erased op, across nulls.
    let base = || boxed(Serie::from_options(&[Some(6i32), None, Some(8)]));
    let rhs = boxed(Serie::from_values(&[2i32, 3, 4]));
    macro_rules! same {
        ($assign:ident, $ret:ident) => {{
            let mut lhs = base();
            lhs.$assign(rhs.as_ref()).unwrap();
            let expected = base().$ret(rhs.as_ref()).unwrap();
            assert!(lhs.eq_any(expected.as_ref()), stringify!($assign));
        }};
    }
    same!(add_assign, add);
    same!(sub_assign, sub);
    same!(mul_assign, mul);
    same!(div_assign, div);
    same!(rem_assign, rem);
}

#[test]
fn erased_div_assign_matches_return_new_with_zero_and_nulls() {
    let base = || boxed(Serie::from_options(&[Some(6i32), None, Some(8), Some(9)]));
    let rhs = boxed(Serie::from_options(&[Some(2i32), Some(3), Some(0), None]));
    let expected = base().div(rhs.as_ref()).unwrap();
    let mut lhs = base();
    lhs.div_assign(rhs.as_ref()).unwrap();
    assert!(lhs.eq_any(expected.as_ref()));
}

#[test]
fn erased_scalar_assign_matches_return_new_including_null_scalar() {
    let base = || boxed(Serie::from_options(&[Some(1i32), None, Some(3)]));
    let expected = base().add_scalar(&i32_scalar(10)).unwrap();
    let mut lhs = base();
    lhs.add_scalar_assign(&i32_scalar(10)).unwrap();
    assert!(lhs.eq_any(expected.as_ref()));

    // A null scalar → all-null, matching the return-new form.
    let expected = base().add_scalar(&AnyScalar::Null).unwrap();
    let mut lhs = base();
    lhs.add_scalar_assign(&AnyScalar::Null).unwrap();
    assert!(lhs.eq_any(expected.as_ref()));
    assert_eq!(lhs.null_count(), 3);
}

#[test]
fn erased_fill_null_mut_and_retain_match_return_new() {
    let base = || boxed(Serie::from_options(&[Some(1i32), None, Some(3), None]));
    let expected = base().fill_null(&i32_scalar(0)).unwrap();
    let mut lhs = base();
    lhs.fill_null_mut(&i32_scalar(0)).unwrap();
    assert!(lhs.eq_any(expected.as_ref()));

    let mask = [true, true, false, true];
    let expected = base().filter(&mask).unwrap();
    let mut lhs = base();
    lhs.retain(&mask).unwrap();
    assert!(lhs.eq_any(expected.as_ref()));
}

#[test]
fn erased_deep_copy_equals_and_is_independent() {
    let a = boxed(Serie::from_values(&[1i32, 2, 3]));
    let d = a.deep_copy();
    assert!(d.eq_any(a.as_ref()));
    // Mutate the deep copy in place; the original is untouched (independent payload).
    let mut d = a.deep_copy();
    d.add_scalar_assign(&i32_scalar(5)).unwrap();
    assert_eq!(
        a.as_serie::<i32>().unwrap().to_options(),
        [Some(1), Some(2), Some(3)]
    );
}

#[test]
fn erased_in_place_arith_rejects_a_non_numeric_left_with_a_guided_error() {
    let mut utf8 = boxed(Utf8Serie::from_strs(&[Some("a"), Some("b")]));
    let other = boxed(Serie::from_values(&[1i32, 2]));
    let err = utf8.add_assign(other.as_ref()).unwrap_err().to_string();
    assert!(err.contains("in-place arithmetic"), "guided: {err}");
    assert!(err.contains("return-new"), "names the fix: {err}");
}
