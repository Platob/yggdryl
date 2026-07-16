//! Phase 3 — the **generic mutator (grow) vocabulary** across every serie family: `extend_values`
//! / `extend_options` / `extend_scalars` / `concat` on the leaves (`Serie<T>`, `DecimalSerie<B>`,
//! `ByteSerie<E>`, `FixedSizeSerie<K>`, `TemporalSerie<B>`, `NullSerie`), and `append_row` /
//! `append_null` / `concat` on the nested columns (`StructSerie` / `ListSerie` / `MapSerie`), plus
//! the erased [`AnySerie::append_scalar`] / [`AnySerie::concat`] hooks.
//!
//! The invariant every test asserts: a **grown** column is byte-for-byte the same as the equivalent
//! column **built from scratch** (`serialize_bytes` equal, `PartialEq` equal) and surfaces the right
//! `field()` (a null anywhere makes it nullable). Descriptor mismatches raise a guided error.

use yggdryl_core::io::fixed::temporal::{TimeUnit, Ts64, Tz};
use yggdryl_core::io::fixed::{D128Scalar, D128Serie};
use yggdryl_core::io::fixed::{
    DecimalError, FixedBinaryScalar, FixedBinarySerie, I32Scalar, I32Serie, NullScalar, NullSerie,
    Serie, Ts64Scalar, Ts64Serie, D128,
};
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::var::{Utf8Scalar, Utf8Serie};
use yggdryl_core::io::{boxed, AnyScalar, AnySerie, IoError};

// =====================================================================================
// Serie<T> — the fixed-width primitive leaf
// =====================================================================================

#[test]
fn fixed_extend_values_options_scalars_grow_like_from_scratch() {
    // extend_values onto a dense column.
    let mut col = I32Serie::from_values(&[1, 2]);
    col.extend_values(&[3, 4, 5]);
    assert_eq!(col, I32Serie::from_values(&[1, 2, 3, 4, 5]));

    // extend_options with a null in the middle materializes the validity mask.
    let mut col = I32Serie::from_values(&[1]);
    col.extend_options(&[Some(2), None, Some(4)]);
    let scratch = I32Serie::from_options(&[Some(1), Some(2), None, Some(4)]);
    assert_eq!(col, scratch);
    assert_eq!(col.serialize_bytes(), scratch.serialize_bytes());
    assert_eq!(col.null_count(), 1);
    assert!(
        col.field().nullable(),
        "a null makes the effective field nullable"
    );

    // extend_scalars mixes present and null.
    let mut col = I32Serie::from_values(&[9]);
    col.extend_scalars(&[I32Scalar::of(8), I32Scalar::null()]);
    assert_eq!(col, I32Serie::from_options(&[Some(9), Some(8), None]));
}

#[test]
fn fixed_concat_equals_from_scratch_and_edges() {
    let mut a = I32Serie::from_options(&[Some(1), None]);
    let b = I32Serie::from_values(&[3, 4]);
    a.concat(&b);
    let scratch = I32Serie::from_options(&[Some(1), None, Some(3), Some(4)]);
    assert_eq!(a, scratch);
    assert_eq!(a.serialize_bytes(), scratch.serialize_bytes());

    // concat onto an empty column yields the source.
    let mut empty = I32Serie::new();
    empty.concat(&b);
    assert_eq!(empty, b);

    // an empty extend / concat is a no-op.
    let mut c = I32Serie::from_values(&[1, 2]);
    let before = c.clone();
    c.extend_values(&[]);
    c.extend_options(&[]);
    c.concat(&I32Serie::new());
    assert_eq!(c, before);

    // a null-heavy extend.
    let mut d = I32Serie::new();
    d.extend_options(&[None, None, Some(1), None]);
    assert_eq!(d.null_count(), 3);
    assert_eq!(d, I32Serie::from_options(&[None, None, Some(1), None]));
}

// =====================================================================================
// DecimalSerie<B> — descriptor (precision, scale) reconciliation on concat
// =====================================================================================

#[test]
fn decimal_extend_and_matching_concat_are_byte_exact() {
    let a = D128::new(12345, 2).unwrap(); // 123.45
    let b = D128::new(600, 2).unwrap(); // 6.00

    let mut col = D128Serie::from_options(20, 2, &[Some(a), None]).unwrap();
    col.extend_values(&[b]).unwrap();
    col.extend_options(&[None, Some(a)]).unwrap();
    col.extend_scalars(&[D128Scalar::null(20, 2)]).unwrap();
    let scratch =
        D128Serie::from_options(20, 2, &[Some(a), None, Some(b), None, Some(a), None]).unwrap();
    assert_eq!(col, scratch);
    assert_eq!(col.serialize_bytes(), scratch.serialize_bytes());

    // Matching-descriptor concat is the raw-byte memcpy fast path.
    let mut left = D128Serie::from_options(20, 2, &[Some(a), None]).unwrap();
    let right = D128Serie::from_values(20, 2, &[b]).unwrap();
    left.concat(&right).unwrap();
    let scratch = D128Serie::from_options(20, 2, &[Some(a), None, Some(b)]).unwrap();
    assert_eq!(left, scratch);
    assert_eq!(left.serialize_bytes(), scratch.serialize_bytes());
}

#[test]
fn decimal_concat_reexpresses_or_errors_on_descriptor_mismatch() {
    // A source at a coarser scale re-expresses exactly into a finer-scale column.
    let mut col = D128Serie::from_values(20, 2, &[D128::new(100, 2).unwrap()]).unwrap(); // 1.00
    let coarse = D128Serie::from_values(20, 0, &[D128::new(5, 0).unwrap()]).unwrap(); // 5
    col.concat(&coarse).unwrap();
    assert_eq!(col.get(1).unwrap().to_string(), "5.00");

    // A source whose finer digits cannot fit the column's scale is a guided InexactRescale.
    let mut col = D128Serie::from_values(20, 2, &[D128::new(100, 2).unwrap()]).unwrap();
    let fine = D128Serie::from_values(20, 4, &[D128::new(12345, 4).unwrap()]).unwrap(); // 1.2345
    let err = col.concat(&fine).unwrap_err();
    assert!(
        matches!(err, DecimalError::InexactRescale { .. }),
        "got {err:?}"
    );
    // The column is left unchanged on error.
    assert_eq!(col.len(), 1);
}

// =====================================================================================
// TemporalSerie<B> — (unit, tz) reconciliation on concat
// =====================================================================================

#[test]
fn temporal_extend_and_concat_matching_and_reexpress() {
    let a = Ts64::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap();
    let b = Ts64::from_epoch(2_000, TimeUnit::Second, Tz::UTC).unwrap();

    let mut col = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None]).unwrap();
    col.extend_values(&[b]).unwrap();
    col.extend_scalars(&[Ts64Scalar::null(TimeUnit::Second, Tz::UTC)])
        .unwrap();
    let scratch =
        Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None, Some(b), None])
            .unwrap();
    assert_eq!(col, scratch);
    assert_eq!(col.serialize_bytes(), scratch.serialize_bytes());

    // Matching (unit, tz) concat is a memcpy fast path.
    let mut left = Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &[a]).unwrap();
    let right = Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &[b]).unwrap();
    left.concat(&right).unwrap();
    assert_eq!(
        left,
        Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &[a, b]).unwrap()
    );

    // A source at a different unit is re-expressed into the column's unit (second -> millisecond).
    let mut ms = Ts64Serie::from_values(
        TimeUnit::Millisecond,
        Tz::UTC,
        &[Ts64::from_epoch(500, TimeUnit::Millisecond, Tz::UTC).unwrap()],
    )
    .unwrap();
    let secs = Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &[a]).unwrap(); // 1000 s
    ms.concat(&secs).unwrap();
    assert_eq!(ms.get(1).unwrap().epoch_value(), 1_000_000); // 1000 s == 1_000_000 ms
}

// =====================================================================================
// ByteSerie<E> — variable-length (Utf8)
// =====================================================================================

#[test]
fn var_extend_options_scalars_and_concat() {
    let mut col = Utf8Serie::from_strs(&[Some("a")]);
    col.extend_options(&[Some(b"bc".as_slice()), None]).unwrap();
    col.extend_scalars(&[Utf8Scalar::of("d"), Utf8Scalar::null()])
        .unwrap();
    let scratch = Utf8Serie::from_strs(&[Some("a"), Some("bc"), None, Some("d"), None]);
    assert_eq!(col, scratch);
    assert_eq!(col.serialize_bytes(), scratch.serialize_bytes());

    // concat equals a from-scratch build; concat onto empty yields the source.
    let mut a = Utf8Serie::from_strs(&[Some("a"), None]);
    let b = Utf8Serie::from_strs(&[Some("cd")]);
    a.concat(&b).unwrap();
    let scratch = Utf8Serie::from_strs(&[Some("a"), None, Some("cd")]);
    assert_eq!(a, scratch);
    assert_eq!(a.serialize_bytes(), scratch.serialize_bytes());

    let mut empty = Utf8Serie::new();
    empty.concat(&b).unwrap();
    assert_eq!(empty, b);
}

#[test]
fn var_extend_validates_utf8_and_leaves_column_unchanged_on_error() {
    let mut col = Utf8Serie::from_strs(&[Some("ok")]);
    let err = col.extend_values(&[&[0xff, 0xfe]]).unwrap_err();
    assert!(matches!(err, IoError::InvalidUtf8 { .. }), "got {err:?}");
    assert_eq!(col.len(), 1, "a bad value leaves the column unchanged");
}

// =====================================================================================
// FixedSizeSerie<K> — runtime width validation
// =====================================================================================

#[test]
fn fixed_size_extend_scalars_and_width_checked_concat() {
    let mut col = FixedBinarySerie::from_options(2, &[Some(&b"ab"[..])]).unwrap();
    col.extend_options(&[Some(&b"cd"[..]), None]).unwrap();
    col.extend_scalars(&[FixedBinaryScalar::from_bytes(b"ef").unwrap()])
        .unwrap();
    let scratch = FixedBinarySerie::from_options(
        2,
        &[Some(&b"ab"[..]), Some(&b"cd"[..]), None, Some(&b"ef"[..])],
    )
    .unwrap();
    assert_eq!(col, scratch);
    assert_eq!(col.serialize_bytes(), scratch.serialize_bytes());

    // A wrong-width present value is a guided CorruptLength (column unchanged).
    let mut col = FixedBinarySerie::from_options(2, &[Some(&b"ab"[..])]).unwrap();
    assert!(col.extend_values(&[&b"xyz"[..]]).is_err());
    assert_eq!(col.len(), 1);

    // concat of two same-width columns matches from-scratch; a width mismatch errors.
    let mut a = FixedBinarySerie::from_options(2, &[Some(&b"ab"[..]), None]).unwrap();
    let b = FixedBinarySerie::from_options(2, &[Some(&b"cd"[..])]).unwrap();
    a.concat(&b).unwrap();
    assert_eq!(
        a,
        FixedBinarySerie::from_options(2, &[Some(&b"ab"[..]), None, Some(&b"cd"[..])]).unwrap()
    );
    let three = FixedBinarySerie::from_options(3, &[Some(&b"xyz"[..])]).unwrap();
    let err = a.concat(&three).unwrap_err();
    assert!(matches!(err, IoError::CorruptLength { .. }), "got {err:?}");
}

// =====================================================================================
// NullSerie — length-only grows
// =====================================================================================

#[test]
fn null_extend_and_concat_are_length_only() {
    let mut col = NullSerie::with_len(2);
    col.extend_values(&[(), ()]);
    col.extend_options(&[None, None, None]);
    col.extend_scalars(&[NullScalar::null()]);
    assert_eq!(col.len(), 8);
    assert_eq!(col.null_count(), 8);

    col.concat(&NullSerie::with_len(3));
    assert_eq!(col, NullSerie::with_len(11));
    assert_eq!(
        NullSerie::deserialize_bytes(&col.serialize_bytes()).unwrap(),
        col
    );
}

// =====================================================================================
// Erased AnySerie::append_scalar / concat (the hooks the nested grows route through)
// =====================================================================================

#[test]
fn erased_append_scalar_and_concat_round_trip() {
    // append_scalar decodes an erased cell back into the concrete leaf column.
    let source = I32Serie::from_options(&[Some(7), None, Some(9)]);
    let mut target: Box<dyn AnySerie> = boxed(I32Serie::new());
    for index in 0..source.len() {
        target.append_scalar(&source.value(index)).unwrap();
    }
    assert!(target.eq_any(&source as &dyn AnySerie));

    // A type-mismatched present value is a guided error; a null is always accepted.
    let mut ints: Box<dyn AnySerie> = boxed(I32Serie::from_values(&[1]));
    let str_cell = boxed(Utf8Serie::from_strs(&[Some("x")])).value(0);
    assert!(ints.append_scalar(&str_cell).is_err());
    ints.append_scalar(&AnyScalar::Null).unwrap();
    assert_eq!(ints.len(), 2);

    // Erased concat routes to the typed concat; a different column type errors.
    let mut a: Box<dyn AnySerie> = boxed(I32Serie::from_values(&[1, 2]));
    a.concat(&I32Serie::from_values(&[3, 4]) as &dyn AnySerie)
        .unwrap();
    assert_eq!(a.len(), 4);
    assert!(a
        .concat(&Utf8Serie::from_strs(&[Some("z")]) as &dyn AnySerie)
        .is_err());
}

// =====================================================================================
// StructSerie — append_row / append_null / concat
// =====================================================================================

fn table(ids: &[i64], names: &[Option<&str>]) -> StructSerie {
    StructSerie::from_named(vec![
        ("id", boxed(Serie::from_values(ids))),
        ("name", boxed(Utf8Serie::from_strs(names))),
    ])
    .unwrap()
}

#[test]
fn struct_append_row_null_and_concat() {
    // append_row: reuse row 0's cell values as a fresh row.
    let mut t = table(&[1, 2, 3], &[Some("a"), Some("b"), Some("c")]);
    let row0 = t.get(0);
    t.append_row(row0.as_struct().unwrap()).unwrap();
    assert_eq!(t.len(), 4);
    assert_eq!(t.get(3), t.get(0));

    // append_null: a null struct row grows validity and every child.
    let mut t = table(&[1, 2], &[Some("a"), Some("b")]);
    t.append_null();
    assert_eq!(t.len(), 3);
    assert_eq!(t.null_count(), 1);
    assert!(t.get(2).is_null());
    assert!(t.column(0).unwrap().value(2).is_null());
    assert!(t.field(0).unwrap().nullable());

    // concat: byte-for-byte the same as building the concatenation from scratch.
    let mut a = table(&[1, 2], &[Some("a"), None]);
    let b = table(&[3, 4], &[Some("c"), Some("d")]);
    a.concat(&b).unwrap();
    let scratch = table(&[1, 2, 3, 4], &[Some("a"), None, Some("c"), Some("d")]);
    assert_eq!(a, scratch);
    assert_eq!(a.serialize_bytes(), scratch.serialize_bytes());
}

#[test]
fn struct_concat_schema_mismatch_is_guided() {
    let mut a = table(&[1], &[Some("a")]);
    // Fewer fields.
    let one_field =
        StructSerie::from_named(vec![("id", boxed(I32Serie::from_values(&[9])))]).unwrap();
    assert!(a.concat(&one_field).is_err());

    // A renamed field.
    let renamed = StructSerie::from_named(vec![
        ("id", boxed(Serie::from_values(&[9i64]))),
        ("label", boxed(Utf8Serie::from_strs(&[Some("z")]))),
    ])
    .unwrap();
    let err = a.concat(&renamed).unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }), "got {err:?}");
}

// =====================================================================================
// ListSerie — append_row / append_null / concat
// =====================================================================================

fn list_of(flat: &[i32], offsets: &[i32], present: Option<&[bool]>) -> ListSerie {
    ListSerie::from_values(Serie::from_values(flat).named("item"), offsets, present).unwrap()
}

#[test]
fn list_append_row_null_and_concat() {
    // append_row appends one sub-list, growing the flat child + offsets.
    let mut list = list_of(&[1, 2], &[0, 2], None);
    list.append_row(boxed(Serie::from_values(&[3i32, 4, 5])))
        .unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list.get_scalar(1).len(), 3);

    // append_null: a zero-width null row (the flat child is untouched).
    let mut list = list_of(&[1, 2], &[0, 2], None);
    let child_before = list.values().len();
    list.append_null();
    assert_eq!(list.len(), 2);
    assert_eq!(list.null_count(), 1);
    assert!(list.get(1).is_null());
    assert_eq!(
        list.values().len(),
        child_before,
        "a null row adds no items"
    );

    // concat equals a from-scratch build (offsets rebased, child concatenated).
    let mut a = list_of(&[1, 2, 3], &[0, 2, 3], None); // rows [1,2], [3]
    let b = list_of(&[4, 5], &[0, 1, 2], None); // rows [4], [5]
    a.concat(&b).unwrap();
    let scratch = list_of(&[1, 2, 3, 4, 5], &[0, 2, 3, 4, 5], None);
    assert_eq!(a, scratch);
    assert_eq!(a.serialize_bytes(), scratch.serialize_bytes());

    // Item-type mismatch is a guided error.
    let strings = ListSerie::from_values(
        Utf8Serie::from_strs(&[Some("x")]).named("item"),
        &[0, 1],
        None,
    )
    .unwrap();
    assert!(a.concat(&strings).is_err());
}

// =====================================================================================
// MapSerie — append_row / append_null / concat
// =====================================================================================

fn map_of(keys: &[&str], vals: &[i64], offsets: &[i32]) -> MapSerie {
    let key_opts: Vec<Option<&str>> = keys.iter().map(|k| Some(*k)).collect();
    MapSerie::from_entries(
        Utf8Serie::from_strs(&key_opts).named("key"),
        Serie::from_values(vals).named("value"),
        offsets,
        None,
        false,
    )
    .unwrap()
}

#[test]
fn map_append_row_null_and_concat() {
    // append_row appends one row of entries.
    let mut map = map_of(&["a"], &[1], &[0, 1]);
    map.append_row(
        boxed(Utf8Serie::from_strs(&[Some("b"), Some("c")])),
        boxed(Serie::from_values(&[2i64, 3])),
    )
    .unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get_scalar(1).len(), 2);

    // A null key in an appended row is rejected (map invariant).
    let mut map = map_of(&["a"], &[1], &[0, 1]);
    let err = map
        .append_row(
            boxed(Utf8Serie::from_strs(&[None])),
            boxed(Serie::from_values(&[9i64])),
        )
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }), "got {err:?}");

    // append_null: a zero-width null map row.
    let mut map = map_of(&["a", "b"], &[1, 2], &[0, 2]);
    map.append_null();
    assert_eq!(map.len(), 2);
    assert_eq!(map.null_count(), 1);
    assert!(map.get(1).is_null());

    // concat equals a from-scratch build.
    let mut a = map_of(&["a", "b"], &[1, 2], &[0, 1, 2]); // rows {a->1}, {b->2}
    let b = map_of(&["c"], &[3], &[0, 1]); // row {c->3}
    a.concat(&b).unwrap();
    let scratch = map_of(&["a", "b", "c"], &[1, 2, 3], &[0, 1, 2, 3]);
    assert_eq!(a, scratch);
    assert_eq!(a.serialize_bytes(), scratch.serialize_bytes());
}
