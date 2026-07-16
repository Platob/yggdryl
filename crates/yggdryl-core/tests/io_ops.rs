//! Vectorized arithmetic (Phase 8b) over the erased column and the typed fast path: the two-tier
//! `add` / `sub` / `mul` / `div` / `rem` (serie×serie and serie×scalar). Covers null propagation,
//! integer div/rem-by-zero → null (never a panic), integer overflow wrapping, cross-type casting of
//! the right operand into the left's type (result follows the LEFT), the scalar broadcast, empty +
//! guided-error edges, temporal (routed through the backing integer), and nested (struct field-wise,
//! list element-wise, map value-wise) — asserting the result type_id + values + a codec round-trip.

use yggdryl_core::io::fixed::temporal::{Date32, TimeUnit, Ts64, Tz};
use yggdryl_core::io::fixed::{Date32Kind, Date32Serie, Field, Serie, Ts64Kind, Ts64Serie};
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnyScalar, AnySerie, DataTypeId};

/// An erased leaf scalar of the given native type + bytes.
fn leaf(id: DataTypeId, width: usize, bytes: Vec<u8>) -> AnyScalar {
    AnyScalar::leaf(Field::of("", id, width, false), bytes)
}

// -------------------------------------------------------------------------------------
// typed fast path — same type, the five ops, null propagation, wrap, div/rem by zero.
// -------------------------------------------------------------------------------------

#[test]
fn same_type_arithmetic_covers_the_five_ops() {
    let a = Serie::from_values(&[10i32, 20, 30]);
    let b = Serie::from_values(&[3i32, 4, 5]);
    assert_eq!(
        a.add_unchecked(&b).to_options(),
        [Some(13), Some(24), Some(35)]
    );
    assert_eq!(
        a.sub_unchecked(&b).to_options(),
        [Some(7), Some(16), Some(25)]
    );
    assert_eq!(
        a.mul_unchecked(&b).to_options(),
        [Some(30), Some(80), Some(150)]
    );
    assert_eq!(
        a.div_unchecked(&b).to_options(),
        [Some(3), Some(5), Some(6)]
    );
    assert_eq!(
        a.rem_unchecked(&b).to_options(),
        [Some(1), Some(0), Some(0)]
    );
}

#[test]
fn null_propagates_from_either_operand() {
    let a = Serie::from_options(&[Some(1i64), None, Some(3), Some(4)]);
    let b = Serie::from_options(&[Some(10i64), Some(20), None, Some(40)]);
    // result[i] null iff a[i] OR b[i] null.
    assert_eq!(
        a.add_unchecked(&b).to_options(),
        [Some(11), None, None, Some(44)]
    );
}

#[test]
fn integer_div_and_rem_by_zero_yield_null_no_panic() {
    let a = Serie::from_values(&[6i32, 7, 8]);
    let z = Serie::from_values(&[2i32, 0, 4]); // the 0 divisor -> a null cell
    let div = a.div_unchecked(&z);
    let rem = a.rem_unchecked(&z);
    assert_eq!(div.to_options(), [Some(3), None, Some(2)]);
    assert_eq!(rem.to_options(), [Some(0), None, Some(0)]);
    assert!(div.get(1).is_none() && rem.get(1).is_none()); // the zero-divisor cell is a null
}

#[test]
fn integer_arithmetic_wraps() {
    // i8: 127 + 1 -> -128, MIN - 1 -> 127, MIN / -1 -> MIN (the lone overflow case wraps).
    assert_eq!(
        Serie::from_values(&[127i8, i8::MIN])
            .add_unchecked(&Serie::from_values(&[1i8, -1]))
            .to_options(),
        [Some(-128), Some(127)]
    );
    assert_eq!(
        Serie::from_values(&[i8::MIN])
            .div_unchecked(&Serie::from_values(&[-1i8]))
            .to_options(),
        [Some(i8::MIN)]
    );
}

#[test]
fn float_div_by_zero_is_ieee_not_null() {
    let a = Serie::from_values(&[1.0f64, -1.0, 0.0]);
    let z = Serie::from_values(&[0.0f64, 0.0, 0.0]);
    let out = a.div_unchecked(&z).to_options();
    assert!(out[0].unwrap().is_infinite() && out[0].unwrap() > 0.0);
    assert!(out[1].unwrap().is_infinite() && out[1].unwrap() < 0.0);
    assert!(out[2].unwrap().is_nan()); // 0.0 / 0.0 = NaN, still a present cell
}

// -------------------------------------------------------------------------------------
// erased base op — cross-type cast (result follows the LEFT), scalar broadcast, edges.
// -------------------------------------------------------------------------------------

#[test]
fn cross_type_casts_the_right_into_the_left_type() {
    // i32.add(i64) -> i32 (the right is range-checked into i32).
    let a = boxed(Serie::from_values(&[1i32, 2, 3]));
    let b = boxed(Serie::from_values(&[10i64, 20, 30]));
    let sum = a.add(b.as_ref()).unwrap();
    assert_eq!(sum.type_id(), DataTypeId::I32);
    assert_eq!(
        sum.as_serie::<i32>().unwrap().to_options(),
        [Some(11), Some(22), Some(33)]
    );

    // f64.add(i32) -> f64.
    let f = boxed(Serie::from_values(&[1.5f64, 2.5]));
    let i = boxed(Serie::from_values(&[2i32, 3]));
    let mixed = f.add(i.as_ref()).unwrap();
    assert_eq!(mixed.type_id(), DataTypeId::F64);
    assert_eq!(
        mixed.as_serie::<f64>().unwrap().to_options(),
        [Some(3.5), Some(5.5)]
    );
}

#[test]
fn out_of_range_right_operand_is_a_guided_error() {
    // i8.add(i32) where a right value 1000 does not fit i8 -> guided range error.
    let a = boxed(Serie::from_values(&[1i8, 2]));
    let b = boxed(Serie::from_values(&[10i32, 1000]));
    let err = a.add(b.as_ref()).unwrap_err().to_string();
    assert!(
        err.contains("out of range") && err.contains("i8"),
        "expected a guided range error naming the value and target, got: {err}"
    );
}

#[test]
fn scalar_broadcast_adds_a_constant() {
    let col = boxed(Serie::from_options(&[Some(1i64), None, Some(3)]));
    let out = col
        .add_scalar(&leaf(DataTypeId::I64, 8, 10i64.to_le_bytes().to_vec()))
        .unwrap();
    assert_eq!(out.type_id(), DataTypeId::I64);
    assert_eq!(
        out.as_serie::<i64>().unwrap().to_options(),
        [Some(11), None, Some(13)]
    );
}

#[test]
fn scalar_broadcast_casts_the_scalar_and_handles_null_and_zero_divisor() {
    // An i32 scalar broadcast into an i64 column (cast up).
    let col = boxed(Serie::from_values(&[5i64, 6, 7]));
    let out = col
        .mul_scalar(&leaf(DataTypeId::I32, 4, 2i32.to_le_bytes().to_vec()))
        .unwrap();
    assert_eq!(
        out.as_serie::<i64>().unwrap().to_options(),
        [Some(10), Some(12), Some(14)]
    );

    // A NULL scalar -> all-null result.
    let all_null = col.add_scalar(&AnyScalar::Null).unwrap();
    assert_eq!(
        all_null.as_serie::<i64>().unwrap().to_options(),
        [None, None, None]
    );

    // Integer divide-by-zero scalar -> every present cell null.
    let divz = col
        .div_scalar(&leaf(DataTypeId::I64, 8, 0i64.to_le_bytes().to_vec()))
        .unwrap();
    assert_eq!(
        divz.as_serie::<i64>().unwrap().to_options(),
        [None, None, None]
    );
}

#[test]
fn empty_columns_op_to_an_empty_column() {
    let a = boxed(Serie::<i32>::new());
    let b = boxed(Serie::<i32>::new());
    let out = a.add(b.as_ref()).unwrap();
    assert_eq!(out.type_id(), DataTypeId::I32);
    assert_eq!(out.len(), 0);
}

#[test]
fn length_mismatch_is_a_guided_error() {
    let a = boxed(Serie::from_values(&[1i32, 2, 3]));
    let b = boxed(Serie::from_values(&[1i32, 2]));
    let err = a.add(b.as_ref()).unwrap_err().to_string();
    assert!(
        err.contains("different lengths") && err.contains('3') && err.contains('2'),
        "expected a guided length error naming both lengths, got: {err}"
    );
}

#[test]
fn non_numeric_operand_is_a_guided_error() {
    // A utf8 left is not numeric.
    let a = boxed(Utf8Serie::from_strs(&[Some("a"), Some("b")]));
    let b = boxed(Serie::from_values(&[1i32, 2]));
    let err = a.add(b.as_ref()).unwrap_err().to_string();
    assert!(
        err.contains("utf8") || err.contains("not supported"),
        "got: {err}"
    );

    // A numeric left with a NON-NUMERIC utf8 right is now COERCED per cell (PART A "absorb
    // anything") — so a genuinely unparseable string is a guided parse error naming the value +
    // target, not a blanket "right operand" rejection (a numeric string would parse fine).
    let c = boxed(Serie::from_values(&[1i32, 2]));
    let d = boxed(Utf8Serie::from_strs(&[Some("a"), Some("b")]));
    let err = c.add(d.as_ref()).unwrap_err().to_string();
    assert!(
        err.contains("cannot parse") && err.contains("i32") && err.contains('a'),
        "got: {err}"
    );

    // A nested right operand has no scalar value to coerce -> a guided right-operand error.
    let s = boxed(
        yggdryl_core::io::nested::StructSerie::from_named(vec![(
            "x",
            boxed(Serie::from_values(&[1i32, 2])),
        )])
        .unwrap(),
    );
    let e = boxed(Serie::from_values(&[1i32, 2]));
    let err = e.add(s.as_ref()).unwrap_err().to_string();
    assert!(err.contains("right operand"), "got: {err}");
}

#[test]
fn result_round_trips_through_the_byte_codec() {
    let a = boxed(Serie::from_options(&[Some(1i32), None, Some(3)]));
    let b = boxed(Serie::from_values(&[10i32, 20, 30]));
    let sum = a.add(b.as_ref()).unwrap();
    let concrete = sum.as_serie::<i32>().unwrap();
    let round = Serie::<i32>::deserialize_bytes(&concrete.serialize_bytes()).unwrap();
    assert_eq!(&round, concrete);
}

// -------------------------------------------------------------------------------------
// temporal — routed through the backing integer, keeping the LEFT temporal type.
// -------------------------------------------------------------------------------------

#[test]
fn temporal_date_add_date_adds_day_counts() {
    let a = boxed(
        Date32Serie::from_values(
            TimeUnit::Day,
            Tz::NAIVE,
            &[Date32::from_days(10), Date32::from_days(20)],
        )
        .unwrap(),
    );
    let b = boxed(
        Date32Serie::from_values(
            TimeUnit::Day,
            Tz::NAIVE,
            &[Date32::from_days(3), Date32::from_days(4)],
        )
        .unwrap(),
    );
    let sum = a.add(b.as_ref()).unwrap();
    assert_eq!(sum.type_id(), DataTypeId::Date32); // result keeps the left temporal type
    let dates = sum.as_temporal::<Date32Kind>().unwrap();
    assert_eq!(dates.get(0).unwrap().days(), 13);
    assert_eq!(dates.get(1).unwrap().days(), 24);
}

#[test]
fn temporal_timestamp_sub_timestamp_is_the_count_diff() {
    let a = boxed(
        Ts64Serie::from_values(
            TimeUnit::Second,
            Tz::UTC,
            &[
                Ts64::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap(),
                Ts64::from_epoch(2_000, TimeUnit::Second, Tz::UTC).unwrap(),
            ],
        )
        .unwrap(),
    );
    let b = boxed(
        Ts64Serie::from_values(
            TimeUnit::Second,
            Tz::UTC,
            &[
                Ts64::from_epoch(100, TimeUnit::Second, Tz::UTC).unwrap(),
                Ts64::from_epoch(500, TimeUnit::Second, Tz::UTC).unwrap(),
            ],
        )
        .unwrap(),
    );
    let diff = a.sub(b.as_ref()).unwrap();
    assert_eq!(diff.type_id(), DataTypeId::Ts64);
    let ts = diff.as_temporal::<Ts64Kind>().unwrap();
    assert_eq!(ts.get(0).unwrap().epoch_value(), 900);
    assert_eq!(ts.get(1).unwrap().epoch_value(), 1_500);
}

#[test]
fn temporal_plus_integer_offsets_the_backing_count() {
    // date32 + i32 -> date32 (the integer offsets the day count).
    let dates = boxed(
        Date32Serie::from_values(TimeUnit::Day, Tz::NAIVE, &[Date32::from_days(10)]).unwrap(),
    );
    let offset = boxed(Serie::from_values(&[5i32]));
    let out = dates.add(offset.as_ref()).unwrap();
    assert_eq!(out.type_id(), DataTypeId::Date32);
    assert_eq!(
        out.as_temporal::<Date32Kind>()
            .unwrap()
            .get(0)
            .unwrap()
            .days(),
        15
    );
}

// -------------------------------------------------------------------------------------
// nested — struct field-wise, list element-wise, map value-wise, shape errors.
// -------------------------------------------------------------------------------------

#[test]
fn struct_op_is_field_wise() {
    let left = boxed(
        StructSerie::from_named(vec![
            ("a", boxed(Serie::from_values(&[1i32, 2, 3]))),
            ("b", boxed(Serie::from_values(&[10i64, 20, 30]))),
        ])
        .unwrap(),
    );
    let right = boxed(
        StructSerie::from_named(vec![
            ("a", boxed(Serie::from_values(&[100i32, 200, 300]))),
            ("b", boxed(Serie::from_values(&[1i64, 2, 3]))),
        ])
        .unwrap(),
    );
    let sum = left.add(right.as_ref()).unwrap();
    assert_eq!(sum.type_id(), DataTypeId::Struct);
    let st = sum.as_any().downcast_ref::<StructSerie>().unwrap();
    // Field names are kept from the LEFT.
    assert_eq!(st.field(0).unwrap().name(), "a");
    assert_eq!(st.field(1).unwrap().name(), "b");
    assert_eq!(
        st.column(0)
            .unwrap()
            .as_serie::<i32>()
            .unwrap()
            .to_options(),
        [Some(101), Some(202), Some(303)]
    );
    assert_eq!(
        st.column(1)
            .unwrap()
            .as_serie::<i64>()
            .unwrap()
            .to_options(),
        [Some(11), Some(22), Some(33)]
    );
}

#[test]
fn struct_with_mismatched_column_count_is_a_guided_error() {
    let left =
        boxed(StructSerie::from_named(vec![("a", boxed(Serie::from_values(&[1i32, 2])))]).unwrap());
    let right = boxed(
        StructSerie::from_named(vec![
            ("a", boxed(Serie::from_values(&[1i32, 2]))),
            ("b", boxed(Serie::from_values(&[1i32, 2]))),
        ])
        .unwrap(),
    );
    let err = left.add(right.as_ref()).unwrap_err().to_string();
    assert!(err.contains("column counts"), "got: {err}");
}

#[test]
fn list_op_is_element_wise_on_matching_shapes() {
    let a = boxed(
        ListSerie::from_values(
            Serie::from_values(&[1i32, 2, 3]).named("item"),
            &[0, 2, 3],
            None,
        )
        .unwrap(),
    );
    let b = boxed(
        ListSerie::from_values(
            Serie::from_values(&[10i32, 20, 30]).named("item"),
            &[0, 2, 3],
            None,
        )
        .unwrap(),
    );
    let sum = a.add(b.as_ref()).unwrap();
    assert_eq!(sum.type_id(), DataTypeId::List);
    let list = sum.as_any().downcast_ref::<ListSerie>().unwrap();
    assert_eq!(list.offsets(), &[0, 2, 3]); // LEFT's offsets reused
    assert_eq!(
        list.values().as_serie::<i32>().unwrap().to_options(),
        [Some(11), Some(22), Some(33)]
    );
}

#[test]
fn list_shape_mismatch_is_a_guided_error() {
    let a = boxed(
        ListSerie::from_values(
            Serie::from_values(&[1i32, 2, 3]).named("item"),
            &[0, 2, 3],
            None,
        )
        .unwrap(),
    );
    let b = boxed(
        ListSerie::from_values(
            Serie::from_values(&[1i32, 2, 3]).named("item"),
            &[0, 1, 3],
            None,
        )
        .unwrap(),
    );
    let err = a.add(b.as_ref()).unwrap_err().to_string();
    assert!(
        err.contains("shapes") || err.contains("offsets"),
        "got: {err}"
    );
}

#[test]
fn map_op_is_value_wise_keeping_keys() {
    let a = boxed(
        MapSerie::from_entries(
            Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key"),
            Serie::from_values(&[1i64, 2]).named("value"),
            &[0, 2],
            None,
            false,
        )
        .unwrap(),
    );
    let b = boxed(
        MapSerie::from_entries(
            Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key"),
            Serie::from_values(&[10i64, 20]).named("value"),
            &[0, 2],
            None,
            false,
        )
        .unwrap(),
    );
    let sum = a.add(b.as_ref()).unwrap();
    assert_eq!(sum.type_id(), DataTypeId::Map);
    let map = sum.as_any().downcast_ref::<MapSerie>().unwrap();
    // Keys and offsets are kept from the LEFT; the value child is summed.
    assert_eq!(map.offsets(), &[0, 2]);
    assert_eq!(map.keys().len(), 2);
    assert!(map.keys().value(0).is_valid() && map.keys().value(1).is_valid());
    assert_eq!(
        map.values().as_serie::<i64>().unwrap().to_options(),
        [Some(11), Some(22)]
    );
}

#[test]
fn nested_result_round_trips_through_the_byte_codec() {
    let a = StructSerie::from_named(vec![
        ("a", boxed(Serie::from_values(&[1i32, 2, 3]))),
        ("b", boxed(Serie::from_values(&[10i64, 20, 30]))),
    ])
    .unwrap();
    let b = StructSerie::from_named(vec![
        ("a", boxed(Serie::from_values(&[4i32, 5, 6]))),
        ("b", boxed(Serie::from_values(&[1i64, 1, 1]))),
    ])
    .unwrap();
    let sum = (&a as &dyn AnySerie).add(&b).unwrap();
    let st = sum.as_any().downcast_ref::<StructSerie>().unwrap();
    let round = StructSerie::deserialize_bytes(&st.serialize_bytes()).unwrap();
    assert_eq!(&round, st);
}

// -------------------------------------------------------------------------------------
// Regression — adversarial-pass fixes (cast gate, temporal scalar operand, width guard).
// -------------------------------------------------------------------------------------

#[test]
fn op_cast_promotion_rejects_an_out_of_range_float_right() {
    // REGRESSION (FIX 1): the op casts the right into the left's type, range-checked. A float of
    // exactly 2^63 does NOT fit i64 (the old gate saturated it to i64::MAX) — the op must error.
    let left = boxed(Serie::from_values(&[0i64]));
    let too_big = boxed(Serie::from_values(&[9223372036854775808.0_f64])); // 2^63
    assert!(left.add(too_big.as_ref()).is_err());
    // A right that fits promotes fine (correct behavior unchanged).
    let ok = boxed(Serie::from_values(&[100.0_f64]));
    assert_eq!(
        left.add(ok.as_ref())
            .unwrap()
            .as_serie::<i64>()
            .unwrap()
            .to_options(),
        [Some(100)]
    );
}

#[test]
fn temporal_add_scalar_rejects_a_wrong_type_right_operand() {
    // REGRESSION (FIX 2+5): the temporal scalar broadcast used to accept any temporal type and even
    // wide ints, then mis-read their raw bytes. It must mirror the serie path exactly.
    let date = boxed(
        Date32Serie::from_values(TimeUnit::Day, Tz::NAIVE, &[Date32::from_days(10)]).unwrap(),
    );

    // (a) A DIFFERENT temporal type (ts64) as the scalar right -> the SAME guided error the serie
    //     path returns for date32.add(ts64_col).
    let ts_col = boxed(
        Ts64Serie::from_values(
            TimeUnit::Second,
            Tz::UTC,
            &[Ts64::from_epoch(1, TimeUnit::Second, Tz::UTC).unwrap()],
        )
        .unwrap(),
    );
    let serie_err = date.add(ts_col.as_ref()).unwrap_err().to_string();
    let ts_leaf = leaf(DataTypeId::Ts64, 8, 1i64.to_le_bytes().to_vec());
    let scalar_err = date.add_scalar(&ts_leaf).unwrap_err().to_string();
    assert_eq!(scalar_err, serie_err);

    // (b) A WIDE integer (u128) as the scalar right is now COERCED (PART A) — its LE magnitude is
    //     read CORRECTLY (range-checked into the backing i128), not the old silent byte-misread: an
    //     in-range value offsets the day count, an out-of-range magnitude is a guided error.
    let u128_five = leaf(DataTypeId::U128, 16, 5u128.to_le_bytes().to_vec());
    assert_eq!(
        date.add_scalar(&u128_five)
            .unwrap()
            .as_temporal::<Date32Kind>()
            .unwrap()
            .get(0)
            .unwrap()
            .days(),
        15 // 10 + 5
    );
    let u128_huge = leaf(DataTypeId::U128, 16, u128::MAX.to_le_bytes().to_vec());
    assert!(date.add_scalar(&u128_huge).is_err()); // magnitude exceeds the i128 bridge

    // (c) A small integer (i32) still works -> offsets the day count.
    let i32_leaf = leaf(DataTypeId::I32, 4, 5i32.to_le_bytes().to_vec());
    let out = date.add_scalar(&i32_leaf).unwrap();
    assert_eq!(out.type_id(), DataTypeId::Date32);
    assert_eq!(
        out.as_temporal::<Date32Kind>()
            .unwrap()
            .get(0)
            .unwrap()
            .days(),
        15
    );
}

#[test]
fn add_scalar_with_a_width_mismatched_leaf_is_a_guided_error_not_a_panic() {
    // REGRESSION (FIX 4): a leaf with the right type_id but a wrong byte length (constructible via
    // the public `AnyScalar::leaf`) used to panic in `read_le`; it must now return a guided error.
    let col = boxed(Serie::from_values(&[10i64, 20, 30]));
    let malformed = AnyScalar::leaf(Field::of("", DataTypeId::F64, 8, false), vec![0u8; 2]);
    assert!(col.add_scalar(&malformed).is_err());
}
