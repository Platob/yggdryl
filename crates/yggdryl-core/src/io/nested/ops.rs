//! Vectorized **element-wise arithmetic** on the erased column — the binding-facing base ops
//! (`add` / `sub` / `mul` / `div` / `rem`, serie×serie and serie×scalar) on `dyn AnySerie`.
//!
//! These are the SAFE tier of the two-tier op pattern: they validate (equal lengths, a numeric /
//! compatible left, a right castable into the left's type), **cast the right operand into the left's
//! element type** (range-checked, guided error on overflow), propagate nulls, and dispatch to the
//! typed [`Serie::add_unchecked`](crate::io::fixed::Serie) fast path. The **result type follows the
//! LEFT operand** — `i32.add(i64)` is `i32` (the right is range-checked into `i32`), matching the
//! scalar rule. Integer arithmetic wraps; integer div/rem by zero yields a null.
//!
//! They live in the `nested` module (like the [`reshape`](super::reshape) coercions) because the
//! nested cases *name* the nested column types — a struct recurses field-wise, a list element-wise,
//! a map on its value child — while a temporal column routes through its **backing integer** and
//! re-wraps as the same temporal type. Keeping them here leaves the root [`AnySerie`] trait free of
//! any dependency on its nested children; they are inherent methods on `dyn AnySerie`, so a single
//! dispatch on the column's `type_id` picks the path with no per-type trait impl.

use crate::io::arith::ArithOp;
use crate::io::fixed::{
    f16, Date32Serie, Date64Serie, Duration32Serie, Duration64Serie, NativeType, Serie,
    TemporalBacking, TemporalSerie, Time32Serie, Time64Serie, Ts32Serie, Ts64Serie, Ts96Serie,
};
use crate::io::{
    boxed, AnyScalar, AnySerie, CastError, Converter, DataTypeId, FieldType, IoError, NumericCast,
};

use super::{ListSerie, MapSerie, StructSerie};

impl dyn AnySerie {
    /// Element-wise `self + other` — the safe, binding-facing add. Validates equal lengths, **casts
    /// `other` into this column's element type** (range-checked; a guided error on overflow), and
    /// dispatches to the typed fast path. The **result type follows `self`** (the left operand).
    /// Integer addition wraps; a result cell is null iff either input cell is null. A temporal
    /// column adds through its backing integer (keeping the left temporal type); a struct recurses
    /// field-wise, a list element-wise, a map on its value child.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::{boxed, AnySerie};
    ///
    /// let a = boxed(Serie::from_values(&[1i64, 2, 3]));
    /// let b = boxed(Serie::from_values(&[10i32, 20, 30])); // i32, cast into the left's i64
    /// let sum = a.add(b.as_ref()).unwrap();
    /// assert_eq!(sum.type_id(), a.type_id()); // result follows the left (i64)
    /// assert_eq!(sum.as_serie::<i64>().unwrap().to_options(), [Some(11), Some(22), Some(33)]);
    /// ```
    pub fn add(&self, other: &(dyn AnySerie + 'static)) -> Result<Box<dyn AnySerie>, IoError> {
        binary(self, other, ArithOp::Add)
    }

    /// Element-wise `self - other` — see [`add`](AnySerie::add) for the checking + casting rules.
    pub fn sub(&self, other: &(dyn AnySerie + 'static)) -> Result<Box<dyn AnySerie>, IoError> {
        binary(self, other, ArithOp::Sub)
    }

    /// Element-wise `self * other` — see [`add`](AnySerie::add) for the checking + casting rules.
    pub fn mul(&self, other: &(dyn AnySerie + 'static)) -> Result<Box<dyn AnySerie>, IoError> {
        binary(self, other, ArithOp::Mul)
    }

    /// Element-wise `self / other` — see [`add`](AnySerie::add). Integer division by a **zero**
    /// divisor writes a null cell (never a panic); a float divides to IEEE `±∞` / `NaN`.
    pub fn div(&self, other: &(dyn AnySerie + 'static)) -> Result<Box<dyn AnySerie>, IoError> {
        binary(self, other, ArithOp::Div)
    }

    /// Element-wise `self % other` — see [`add`](AnySerie::add). Integer remainder by a **zero**
    /// divisor writes a null cell (never a panic).
    pub fn rem(&self, other: &(dyn AnySerie + 'static)) -> Result<Box<dyn AnySerie>, IoError> {
        binary(self, other, ArithOp::Rem)
    }

    /// Broadcasts the scalar `value` over every element as `self + value` — casts `value` into this
    /// column's element type, then applies it to each cell (null cells stay null). A **null** scalar
    /// yields an all-null result. Nested columns broadcast into each leaf child; a temporal column
    /// broadcasts through its backing integer.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{Field, Serie};
    /// use yggdryl_core::io::{boxed, AnyScalar, AnySerie, DataTypeId};
    ///
    /// let col = boxed(Serie::from_options(&[Some(1i64), None, Some(3)]));
    /// let one = AnyScalar::leaf(Field::of("", DataTypeId::I64, 8, false), 1i64.to_le_bytes().to_vec());
    /// let out = col.add_scalar(&one).unwrap();
    /// assert_eq!(out.as_serie::<i64>().unwrap().to_options(), [Some(2), None, Some(4)]);
    /// ```
    pub fn add_scalar(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        scalar(self, value, ArithOp::Add)
    }

    /// Broadcasts `value` as `self - value` — see [`add_scalar`](AnySerie::add_scalar).
    pub fn sub_scalar(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        scalar(self, value, ArithOp::Sub)
    }

    /// Broadcasts `value` as `self * value` — see [`add_scalar`](AnySerie::add_scalar).
    pub fn mul_scalar(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        scalar(self, value, ArithOp::Mul)
    }

    /// Broadcasts `value` as `self / value` — see [`add_scalar`](AnySerie::add_scalar). A **zero**
    /// integer `value` makes every present cell null (no panic).
    pub fn div_scalar(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        scalar(self, value, ArithOp::Div)
    }

    /// Broadcasts `value` as `self % value` — see [`add_scalar`](AnySerie::add_scalar). A **zero**
    /// integer `value` makes every present cell null (no panic).
    pub fn rem_scalar(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        scalar(self, value, ArithOp::Rem)
    }
}

// -------------------------------------------------------------------------------------
// serie × serie
// -------------------------------------------------------------------------------------

/// The serie×serie dispatch shared by all five ops: length-check, then route on the left's type_id
/// (numeric leaf / temporal / nested), casting the right into the left's type where it is a leaf.
fn binary(
    left: &(dyn AnySerie + 'static),
    right: &(dyn AnySerie + 'static),
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    if left.len() != right.len() {
        return Err(length_mismatch(left.len(), right.len()));
    }
    let id = left.type_id();
    if id.is_temporal() {
        return temporal_binary(left, right, op);
    }
    if id.is_nested() {
        return nested_binary(left, right, op);
    }
    macro_rules! num {
        ($t:ty) => {{
            let l = left
                .as_serie::<$t>()
                .expect("type_id names this concrete Serie");
            let r = cast_into::<$t>(right)?;
            Ok(boxed(l.arith_unchecked(&r, op)))
        }};
    }
    match id {
        DataTypeId::U8 => num!(u8),
        DataTypeId::U16 => num!(u16),
        DataTypeId::U32 => num!(u32),
        DataTypeId::U64 => num!(u64),
        DataTypeId::I8 => num!(i8),
        DataTypeId::I16 => num!(i16),
        DataTypeId::I32 => num!(i32),
        DataTypeId::I64 => num!(i64),
        DataTypeId::I128 => num!(i128),
        DataTypeId::F16 => num!(f16),
        DataTypeId::F32 => num!(f32),
        DataTypeId::F64 => num!(f64),
        other => Err(non_numeric_left(other)),
    }
}

/// Casts an erased column into a typed `Serie<U>` (range-checked, nulls preserved) — the right
/// operand of a binary op whose result type is `U` (the left's type). A guided error if `other` is
/// not one of the twelve numeric leaf types, or a value does not fit `U`.
fn cast_into<U: NumericCast>(other: &(dyn AnySerie + 'static)) -> Result<Serie<U>, IoError> {
    macro_rules! cast {
        ($t:ty) => {
            other
                .as_serie::<$t>()
                .expect("type_id names this concrete Serie")
                .cast::<U>()
                .map_err(cast_error)
        };
    }
    match other.type_id() {
        DataTypeId::U8 => cast!(u8),
        DataTypeId::U16 => cast!(u16),
        DataTypeId::U32 => cast!(u32),
        DataTypeId::U64 => cast!(u64),
        DataTypeId::I8 => cast!(i8),
        DataTypeId::I16 => cast!(i16),
        DataTypeId::I32 => cast!(i32),
        DataTypeId::I64 => cast!(i64),
        DataTypeId::I128 => cast!(i128),
        DataTypeId::F16 => cast!(f16),
        DataTypeId::F32 => cast!(f32),
        DataTypeId::F64 => cast!(f64),
        other_id => Err(right_not_numeric(other_id, U::NAME)),
    }
}

// -------------------------------------------------------------------------------------
// serie × scalar
// -------------------------------------------------------------------------------------

/// The serie×scalar dispatch shared by all five broadcast ops.
fn scalar(
    left: &(dyn AnySerie + 'static),
    value: &AnyScalar,
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    let id = left.type_id();
    if id.is_temporal() {
        return temporal_scalar(left, value, op);
    }
    if id.is_nested() {
        return nested_scalar(left, value, op);
    }
    macro_rules! num {
        ($t:ty) => {{
            let l = left.as_serie::<$t>().expect("type_id names this concrete Serie");
            match any_scalar_into::<$t>(value)? {
                Some(v) => Ok(boxed(l.arith_scalar_unchecked(v, op))),
                None => Ok(boxed(Serie::<$t>::from_options(&vec![None; l.len()]))), // null scalar -> all null
            }
        }};
    }
    match id {
        DataTypeId::U8 => num!(u8),
        DataTypeId::U16 => num!(u16),
        DataTypeId::U32 => num!(u32),
        DataTypeId::U64 => num!(u64),
        DataTypeId::I8 => num!(i8),
        DataTypeId::I16 => num!(i16),
        DataTypeId::I32 => num!(i32),
        DataTypeId::I64 => num!(i64),
        DataTypeId::I128 => num!(i128),
        DataTypeId::F16 => num!(f16),
        DataTypeId::F32 => num!(f32),
        DataTypeId::F64 => num!(f64),
        other => Err(non_numeric_left(other)),
    }
}

/// Casts an erased scalar into a typed `U` value (range-checked), or `None` for a null scalar. A
/// guided error if the scalar is a non-numeric leaf / nested value, or the value does not fit `U`.
fn any_scalar_into<U: NumericCast>(value: &AnyScalar) -> Result<Option<U>, IoError> {
    let (field, bytes) = match value {
        AnyScalar::Null => return Ok(None),
        AnyScalar::Leaf { field, bytes } => (field, bytes),
        other => return Err(scalar_not_numeric(other.type_id())),
    };
    macro_rules! conv {
        ($t:ty) => {{
            // Width guard BEFORE `read_le`: a hand-built leaf (via the public `AnyScalar::leaf`) can
            // carry the right type_id but a wrong-length byte payload, which would panic in
            // `read_le`. Reject it with a guided error instead — no panic from any public op.
            let need = <$t as NativeType>::WIDTH;
            if bytes.len() != need {
                return Err(scalar_width_mismatch(
                    FieldType::type_id(field),
                    bytes.len(),
                    need,
                ));
            }
            <$t as Converter<U>>::cast_value(<$t as NativeType>::read_le(bytes))
                .map(Some)
                .map_err(cast_error)
        }};
    }
    match FieldType::type_id(field) {
        DataTypeId::U8 => conv!(u8),
        DataTypeId::U16 => conv!(u16),
        DataTypeId::U32 => conv!(u32),
        DataTypeId::U64 => conv!(u64),
        DataTypeId::I8 => conv!(i8),
        DataTypeId::I16 => conv!(i16),
        DataTypeId::I32 => conv!(i32),
        DataTypeId::I64 => conv!(i64),
        DataTypeId::I128 => conv!(i128),
        DataTypeId::F16 => conv!(f16),
        DataTypeId::F32 => conv!(f32),
        DataTypeId::F64 => conv!(f64),
        other => Err(scalar_not_numeric(Some(other))),
    }
}

// -------------------------------------------------------------------------------------
// temporal — route through the backing integer, re-wrap as the same temporal type.
//
// DESIGN: temporal arithmetic is semantically unusual (a date is not a number), enabled on request:
// it falls out of the shared integer path by reading the column's raw physical counts as its backing
// integer (i32 / i64 / i96), running the wrapping integer op, and re-wrapping the result at the
// backing width — so `date32 + date32` adds day counts and wraps at i32, `ts64 - ts64` is the micros
// diff as the left ts64 type. The result keeps the LEFT column's `(unit, tz)` temporal type. A right
// operand is either the SAME temporal type (its counts read directly) or an integer column (cast into
// i128 as the backing count).
// -------------------------------------------------------------------------------------

fn temporal_binary(
    left: &(dyn AnySerie + 'static),
    right: &(dyn AnySerie + 'static),
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    macro_rules! t {
        ($serie:ty) => {
            temporal_backing(
                left.downcast_ref::<$serie>().expect("temporal type_id"),
                right,
                op,
            )
        };
    }
    match left.type_id() {
        DataTypeId::Date32 => t!(Date32Serie),
        DataTypeId::Date64 => t!(Date64Serie),
        DataTypeId::Time32 => t!(Time32Serie),
        DataTypeId::Time64 => t!(Time64Serie),
        DataTypeId::Ts32 => t!(Ts32Serie),
        DataTypeId::Ts64 => t!(Ts64Serie),
        DataTypeId::Ts96 => t!(Ts96Serie),
        DataTypeId::Duration32 => t!(Duration32Serie),
        DataTypeId::Duration64 => t!(Duration64Serie),
        other => Err(non_numeric_left(other)),
    }
}

fn temporal_backing<B: TemporalBacking>(
    left: &TemporalSerie<B>,
    right: &(dyn AnySerie + 'static),
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    let len = left.len();
    let right_counts: Vec<Option<i128>> = if right.type_id() == B::TYPE_ID {
        // Same temporal type — read its raw physical counts directly.
        let other = right
            .downcast_ref::<TemporalSerie<B>>()
            .expect("matching temporal type_id");
        (0..len).map(|index| other.get_count(index)).collect()
    } else if right.type_id().is_integer() {
        // An integer operand — cast into the backing count (i128).
        let ints = cast_into::<i128>(right)?;
        (0..len).map(|index| ints.get(index)).collect()
    } else {
        return Err(temporal_right(B::NAME, Some(right.type_id())));
    };
    let width = B::WIDTH;
    let counts: Vec<Option<i128>> = (0..len)
        .map(|index| match (left.get_count(index), right_counts[index]) {
            (Some(a), Some(b)) => op
                .apply_i128_wrapping(a, b)
                .map(|value| wrap_to_width(value, width)),
            _ => None, // null propagation
        })
        .collect();
    Ok(boxed(TemporalSerie::<B>::from_result_counts(
        left.unit(),
        left.timezone(),
        &counts,
    )))
}

fn temporal_scalar(
    left: &(dyn AnySerie + 'static),
    value: &AnyScalar,
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    macro_rules! t {
        ($serie:ty) => {
            temporal_scalar_backing(
                left.downcast_ref::<$serie>().expect("temporal type_id"),
                value,
                op,
            )
        };
    }
    match left.type_id() {
        DataTypeId::Date32 => t!(Date32Serie),
        DataTypeId::Date64 => t!(Date64Serie),
        DataTypeId::Time32 => t!(Time32Serie),
        DataTypeId::Time64 => t!(Time64Serie),
        DataTypeId::Ts32 => t!(Ts32Serie),
        DataTypeId::Ts64 => t!(Ts64Serie),
        DataTypeId::Ts96 => t!(Ts96Serie),
        DataTypeId::Duration32 => t!(Duration32Serie),
        DataTypeId::Duration64 => t!(Duration64Serie),
        other => Err(non_numeric_left(other)),
    }
}

fn temporal_scalar_backing<B: TemporalBacking>(
    left: &TemporalSerie<B>,
    value: &AnyScalar,
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    // Mirror the serie path `temporal_backing` exactly: the right operand is accepted only when it
    // is the SAME temporal type (its raw counts read directly) or one of the twelve numeric leaf
    // types routed through the range-checked integer path (`any_scalar_into::<i128>`, the scalar
    // twin of `cast_into::<i128>`). A DIFFERENT temporal type or a WIDE integer (u96/u128/u256/
    // i96/i256 — not `NumericCast`) returns a guided error, never a silently mis-read value.
    let rhs: Option<i128> = match value {
        AnyScalar::Null => None, // a null scalar -> all-null result
        AnyScalar::Leaf { field, bytes } => {
            let id = FieldType::type_id(field);
            if id == B::TYPE_ID {
                // Same temporal type — read its raw physical counts directly (width-guarded, so a
                // malformed leaf errors instead of mis-reading). Temporal counts are signed.
                if bytes.len() != B::WIDTH {
                    return Err(temporal_scalar_width(B::NAME, bytes.len(), B::WIDTH));
                }
                Some(le_bytes_to_i128(bytes, true))
            } else if id.is_integer() {
                // An integer operand — cast into the backing count (i128), range-checked. A wide
                // integer is `is_integer()` but not `NumericCast`, so this rejects it with a guided
                // error, exactly as the serie path's `cast_into::<i128>` does.
                any_scalar_into::<i128>(value)?
            } else {
                return Err(temporal_right(B::NAME, Some(id)));
            }
        }
        other => return Err(temporal_right(B::NAME, other.type_id())),
    };
    let (len, width) = (left.len(), B::WIDTH);
    let counts: Vec<Option<i128>> = (0..len)
        .map(|index| match (left.get_count(index), rhs) {
            (Some(a), Some(b)) => op
                .apply_i128_wrapping(a, b)
                .map(|value| wrap_to_width(value, width)),
            _ => None,
        })
        .collect();
    Ok(boxed(TemporalSerie::<B>::from_result_counts(
        left.unit(),
        left.timezone(),
        &counts,
    )))
}

/// Wraps an `i128` to a `width`-byte two's-complement integer (sign-extended) — the backing-integer
/// wrap a temporal op applies, so `date32 + date32` wraps at `i32` exactly like the integer path.
fn wrap_to_width(value: i128, width: usize) -> i128 {
    if width >= 16 {
        return value;
    }
    let bits = width * 8;
    let mask = (1i128 << bits) - 1;
    let truncated = value & mask;
    let sign_bit = 1i128 << (bits - 1);
    if truncated & sign_bit != 0 {
        truncated | !mask // negative: fill the high bits
    } else {
        truncated
    }
}

/// Reads `bytes` (little-endian) as an `i128`, sign-extending when `signed` and the top bit is set,
/// else zero-extending — the erased read of an integer / temporal scalar's backing count.
fn le_bytes_to_i128(bytes: &[u8], signed: bool) -> i128 {
    let negative = signed && bytes.last().is_some_and(|&byte| byte & 0x80 != 0);
    let mut buf = if negative { [0xffu8; 16] } else { [0u8; 16] };
    let n = bytes.len().min(16);
    buf[..n].copy_from_slice(&bytes[..n]);
    i128::from_le_bytes(buf)
}

// -------------------------------------------------------------------------------------
// nested — recurse to the leaves (struct field-wise, list element-wise, map value-wise).
// -------------------------------------------------------------------------------------

fn nested_binary(
    left: &(dyn AnySerie + 'static),
    right: &(dyn AnySerie + 'static),
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    match left.type_id() {
        DataTypeId::Struct => struct_binary(left, right, op),
        DataTypeId::List => list_binary(left, right, op),
        DataTypeId::Map => map_binary(left, right, op),
        other => Err(non_numeric_left(other)),
    }
}

/// struct.op(struct): equal column count + pairwise op of the children (which recurse the same
/// dispatch), keeping the LEFT struct's field names and row validity.
fn struct_binary(
    left: &(dyn AnySerie + 'static),
    right: &(dyn AnySerie + 'static),
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    let l = left
        .as_any()
        .downcast_ref::<StructSerie>()
        .expect("struct type_id");
    let r = right
        .as_any()
        .downcast_ref::<StructSerie>()
        .ok_or_else(|| nested_shape("struct", right.type_id()))?;
    if l.num_columns() != r.num_columns() {
        return Err(struct_column_count(l.num_columns(), r.num_columns()));
    }
    let mut columns: Vec<Box<dyn AnySerie>> = Vec::with_capacity(l.num_columns());
    for index in 0..l.num_columns() {
        let lc = l.column(index).expect("index < num_columns");
        let rc = r.column(index).expect("index < num_columns");
        columns.push(binary(lc, rc, op)?);
    }
    // Rebuild with the LEFT struct's schema (names + metadata) and row validity. Pass the LEFT's
    // explicit row count so a **field-less** struct (no child column to derive the length from)
    // keeps its rows instead of collapsing to length 0.
    let present = row_present(left);
    Ok(boxed(StructSerie::from_columns_with_len(
        l.fields(),
        columns,
        l.len(),
        present.as_deref(),
    )?))
}

/// list.op(list): the offsets must match exactly (same shape), then op the flattened child series,
/// reusing the LEFT list's offsets and row validity.
fn list_binary(
    left: &(dyn AnySerie + 'static),
    right: &(dyn AnySerie + 'static),
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    let l = left
        .as_any()
        .downcast_ref::<ListSerie>()
        .expect("list type_id");
    let r = right
        .as_any()
        .downcast_ref::<ListSerie>()
        .ok_or_else(|| nested_shape("list", right.type_id()))?;
    if l.offsets() != r.offsets() {
        return Err(list_shape());
    }
    let mut child = binary(l.values(), r.values(), op)?;
    child.set_name(l.values().name());
    let present = row_present(left);
    Ok(boxed(ListSerie::from_values(
        child,
        l.offsets(),
        present.as_deref(),
    )?))
}

/// map.op(map): the keys and offsets must match, then op the VALUE child, keeping the LEFT map's
/// keys / offsets / row validity / keys_sorted flag.
fn map_binary(
    left: &(dyn AnySerie + 'static),
    right: &(dyn AnySerie + 'static),
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    let l = left
        .as_any()
        .downcast_ref::<MapSerie>()
        .expect("map type_id");
    let r = right
        .as_any()
        .downcast_ref::<MapSerie>()
        .ok_or_else(|| nested_shape("map", right.type_id()))?;
    if l.offsets() != r.offsets() || !l.keys().eq_any(r.keys()) {
        return Err(map_keys());
    }
    let mut values = binary(l.values(), r.values(), op)?;
    values.set_name(l.values().name());
    let mut keys = l.keys().clone_box();
    keys.set_name(l.keys().name());
    let present = row_present(left);
    Ok(boxed(MapSerie::from_entries(
        keys,
        values,
        l.offsets(),
        present.as_deref(),
        l.keys_sorted(),
    )?))
}

fn nested_scalar(
    left: &(dyn AnySerie + 'static),
    value: &AnyScalar,
    op: ArithOp,
) -> Result<Box<dyn AnySerie>, IoError> {
    match left.type_id() {
        DataTypeId::Struct => {
            let l = left
                .as_any()
                .downcast_ref::<StructSerie>()
                .expect("struct type_id");
            let mut columns: Vec<Box<dyn AnySerie>> = Vec::with_capacity(l.num_columns());
            for index in 0..l.num_columns() {
                columns.push(scalar(
                    l.column(index).expect("index < num_columns"),
                    value,
                    op,
                )?);
            }
            // Explicit row count so a field-less struct keeps its rows (see `struct_binary`).
            let present = row_present(left);
            Ok(boxed(StructSerie::from_columns_with_len(
                l.fields(),
                columns,
                l.len(),
                present.as_deref(),
            )?))
        }
        DataTypeId::List => {
            let l = left
                .as_any()
                .downcast_ref::<ListSerie>()
                .expect("list type_id");
            let mut child = scalar(l.values(), value, op)?;
            child.set_name(l.values().name());
            let present = row_present(left);
            Ok(boxed(ListSerie::from_values(
                child,
                l.offsets(),
                present.as_deref(),
            )?))
        }
        DataTypeId::Map => {
            let l = left
                .as_any()
                .downcast_ref::<MapSerie>()
                .expect("map type_id");
            let mut values = scalar(l.values(), value, op)?;
            values.set_name(l.values().name());
            let mut keys = l.keys().clone_box();
            keys.set_name(l.keys().name());
            let present = row_present(left);
            Ok(boxed(MapSerie::from_entries(
                keys,
                values,
                l.offsets(),
                present.as_deref(),
                l.keys_sorted(),
            )?))
        }
        other => Err(non_numeric_left(other)),
    }
}

/// The nested column's per-row **present** flags (LEFT's row validity), or `None` when the column
/// has no null rows — the mask a nested op re-applies to its result. Reads each row's logical value
/// (`Null` for a null row); nested ops are not a hot path, so this materializes per-row cell values.
fn row_present(column: &(dyn AnySerie + 'static)) -> Option<Vec<bool>> {
    if column.null_count() == 0 {
        return None;
    }
    Some(
        (0..column.len())
            .map(|i| column.value(i).is_valid())
            .collect(),
    )
}

// -------------------------------------------------------------------------------------
// Guided errors — each names how to fix it.
// -------------------------------------------------------------------------------------

fn length_mismatch(left: usize, right: usize) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot combine columns of different lengths ({left} and {right}); an element-wise op \
             needs both operands the same length"
        ),
    }
}

fn non_numeric_left(id: DataTypeId) -> IoError {
    IoError::Unsupported {
        what: format!(
            "arithmetic is not supported on a {} column; the left operand must be a numeric column \
             (u8..i64, i128, f16/f32/f64), a temporal column (routed through its backing integer), \
             or a nested column of those",
            id.name()
        ),
    }
}

fn right_not_numeric(id: DataTypeId, target: &str) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot use a {} column as the right operand of an arithmetic op producing `{target}`; \
             the right operand must be a numeric column (u8..i64, i128, f16/f32/f64) castable into \
             the left's element type",
            id.name()
        ),
    }
}

fn cast_error(error: CastError) -> IoError {
    IoError::Unsupported {
        what: format!(
            "the right operand does not fit the result type ({error}); the right column is cast \
             into the left column's element type, range-checked"
        ),
    }
}

fn scalar_not_numeric(id: Option<DataTypeId>) -> IoError {
    let got = id.map_or("null", DataTypeId::name);
    IoError::Unsupported {
        what: format!(
            "cannot broadcast a {got} scalar in an arithmetic op; the scalar must be a numeric \
             value castable into the column's element type (or null for an all-null result)"
        ),
    }
}

fn scalar_width_mismatch(id: DataTypeId, got: usize, need: usize) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot broadcast a {} scalar of {got} bytes in an arithmetic op; a {} value's bytes \
             must be exactly {need} long",
            id.name(),
            id.name()
        ),
    }
}

fn temporal_scalar_width(left: &str, got: usize, need: usize) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot broadcast a {left} scalar of {got} bytes; a {left} value's raw counts must be \
             exactly {need} bytes long"
        ),
    }
}

fn temporal_right(left: &str, right: Option<DataTypeId>) -> IoError {
    let got = right.map_or("null", DataTypeId::name);
    IoError::Unsupported {
        what: format!(
            "cannot combine a {left} column with a {got} operand; a temporal arithmetic takes \
             either the same temporal type or an integer (cast into its backing integer)"
        ),
    }
}

fn nested_shape(kind: &str, right: DataTypeId) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot combine a {kind} column with a {} column; both operands must be {kind} columns \
             of the same shape",
            right.name()
        ),
    }
}

fn struct_column_count(left: usize, right: usize) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot combine structs with different column counts ({left} and {right}); a \
             field-wise op needs the same columns in order"
        ),
    }
}

fn list_shape() -> IoError {
    IoError::Unsupported {
        what: "cannot combine lists of different shapes; the two list columns must have identical \
               offsets (same per-row lengths) for an element-wise op"
            .to_string(),
    }
}

fn map_keys() -> IoError {
    IoError::Unsupported {
        what: "cannot combine maps with different keys or shape; the two map columns must have \
               identical offsets and key columns for a value-wise op"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::io::nested::StructSerie;
    use crate::io::{boxed, DataTypeId};

    #[test]
    fn field_less_struct_arithmetic_preserves_the_row_count() {
        // REGRESSION (FIX 7): a struct with NO child columns but 3 rows. `from_columns` derives the
        // length from the (absent) first column -> 0, so the op must carry the operands' explicit
        // len instead. The result must keep the 3 rows.
        let left = StructSerie::from_columns_with_len(vec![], vec![], 3, None).unwrap();
        let right = StructSerie::from_columns_with_len(vec![], vec![], 3, None).unwrap();
        assert_eq!(left.len(), 3);

        let sum = boxed(left).add(boxed(right).as_ref()).unwrap();
        assert_eq!(sum.type_id(), DataTypeId::Struct);
        assert_eq!(sum.len(), 3);
    }
}
