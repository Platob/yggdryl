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

use core::str::FromStr;

use crate::io::arith::ArithOp;
use crate::io::fixed::{
    f16, Date32Serie, Date64Serie, Duration32Serie, Duration64Serie, Field, NativeType, Serie,
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

    // ---- in-place arithmetic twins — mutate self's buffer through copy-on-write -----------------
    //
    // The erased mirror of the [`Serie::add_assign`](crate::io::fixed::Serie) twins: `other` is cast
    // into self's element type (the cast-anything path, the only place an allocation may happen),
    // then self is mutated IN PLACE through the copy-on-write [`Buffer::with_values_mut`] — self is
    // **never** cloned, so an owned column copies no payload and a shared one pays exactly one COW.
    // The **result follows the LEFT** (self keeps its type). Scoped to the **numeric leaf** columns
    // (the in-place COW families); temporal / decimal / nested columns have no zero-copy in-place
    // form, so route through the return-new [`add`](AnySerie::add) (a guided error names this).

    /// In-place `self += other` — casts `other` into self's element type, then mutates self through
    /// copy-on-write. See the [module note](self) for the two-tier / result-follows-left rules.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::{boxed, AnySerie};
    ///
    /// let mut a = boxed(Serie::from_values(&[1i64, 2, 3]));
    /// let b = boxed(Serie::from_values(&[10i32, 20, 30])); // i32, cast into the left's i64
    /// a.add_assign(b.as_ref()).unwrap();
    /// assert_eq!(a.as_serie::<i64>().unwrap().to_options(), [Some(11), Some(22), Some(33)]);
    /// ```
    pub fn add_assign(&mut self, other: &(dyn AnySerie + 'static)) -> Result<(), IoError> {
        binary_assign(self, other, ArithOp::Add)
    }

    /// In-place `self -= other` — see [`add_assign`](AnySerie::add_assign).
    pub fn sub_assign(&mut self, other: &(dyn AnySerie + 'static)) -> Result<(), IoError> {
        binary_assign(self, other, ArithOp::Sub)
    }

    /// In-place `self *= other` — see [`add_assign`](AnySerie::add_assign).
    pub fn mul_assign(&mut self, other: &(dyn AnySerie + 'static)) -> Result<(), IoError> {
        binary_assign(self, other, ArithOp::Mul)
    }

    /// In-place `self /= other` — see [`add_assign`](AnySerie::add_assign). Integer division by a
    /// **zero** divisor writes a null cell (never a panic).
    pub fn div_assign(&mut self, other: &(dyn AnySerie + 'static)) -> Result<(), IoError> {
        binary_assign(self, other, ArithOp::Div)
    }

    /// In-place `self %= other` — see [`add_assign`](AnySerie::add_assign). Integer remainder by a
    /// **zero** divisor writes a null cell (never a panic).
    pub fn rem_assign(&mut self, other: &(dyn AnySerie + 'static)) -> Result<(), IoError> {
        binary_assign(self, other, ArithOp::Rem)
    }

    /// In-place `self += value` (broadcast) — casts `value` into self's element type, then mutates
    /// self through copy-on-write. A **null** scalar makes every cell null.
    pub fn add_scalar_assign(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        scalar_assign(self, value, ArithOp::Add)
    }

    /// In-place `self -= value` (broadcast) — see [`add_scalar_assign`](AnySerie::add_scalar_assign).
    pub fn sub_scalar_assign(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        scalar_assign(self, value, ArithOp::Sub)
    }

    /// In-place `self *= value` (broadcast) — see [`add_scalar_assign`](AnySerie::add_scalar_assign).
    pub fn mul_scalar_assign(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        scalar_assign(self, value, ArithOp::Mul)
    }

    /// In-place `self /= value` (broadcast) — see [`add_scalar_assign`](AnySerie::add_scalar_assign).
    /// A **zero** integer `value` makes every present cell null (no panic).
    pub fn div_scalar_assign(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        scalar_assign(self, value, ArithOp::Div)
    }

    /// In-place `self %= value` (broadcast) — see [`add_scalar_assign`](AnySerie::add_scalar_assign).
    /// A **zero** integer `value` makes every present cell null (no panic).
    pub fn rem_scalar_assign(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        scalar_assign(self, value, ArithOp::Rem)
    }
}

// -------------------------------------------------------------------------------------
// in-place serie × serie / serie × scalar — numeric leaves, mutated through copy-on-write
// -------------------------------------------------------------------------------------

/// The in-place serie×serie dispatch: length-check, cast `right` into the left's element type, then
/// mutate the left leaf IN PLACE (copy-on-write). Numeric leaves only — a temporal / decimal /
/// nested left is a guided error (no zero-copy in-place form; use the return-new `add`).
fn binary_assign(
    left: &mut (dyn AnySerie + 'static),
    right: &(dyn AnySerie + 'static),
    op: ArithOp,
) -> Result<(), IoError> {
    if left.len() != right.len() {
        return Err(length_mismatch(left.len(), right.len()));
    }
    macro_rules! num {
        ($t:ty) => {{
            // Cast the right operand into the left's type first (may allocate — the only allocation);
            // then mutate the left leaf in place, never cloning it.
            let r = cast_into::<$t>(right)?;
            left.as_any_mut()
                .downcast_mut::<Serie<$t>>()
                .expect("type_id names this concrete Serie")
                .arith_assign_unchecked(&r, op);
            Ok(())
        }};
    }
    match left.type_id() {
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
        other => Err(inplace_unsupported_left(other)),
    }
}

/// The in-place serie×scalar dispatch — the broadcast twin of [`binary_assign`].
fn scalar_assign(
    left: &mut (dyn AnySerie + 'static),
    value: &AnyScalar,
    op: ArithOp,
) -> Result<(), IoError> {
    macro_rules! num {
        ($t:ty) => {{
            // Coerce the scalar into the left's type BEFORE the mutable downcast (it reads nothing
            // from `left`), then mutate in place. A null scalar nulls every cell.
            let coerced = any_scalar_into::<$t>(value)?;
            let serie = left
                .as_any_mut()
                .downcast_mut::<Serie<$t>>()
                .expect("type_id names this concrete Serie");
            match coerced {
                Some(v) => serie.arith_scalar_assign_unchecked(v, op),
                None => serie.set_all_null(), // null scalar -> all-null, in place
            }
            Ok(())
        }};
    }
    match left.type_id() {
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
        other => Err(inplace_unsupported_left(other)),
    }
}

/// The guided error for an in-place arithmetic op whose left column has no copy-on-write in-place
/// twin (temporal / decimal / nested / non-numeric) — directs the caller to the return-new form.
fn inplace_unsupported_left(id: DataTypeId) -> IoError {
    IoError::Unsupported {
        what: format!(
            "in-place arithmetic (add_assign / sub_assign / …) is only supported on a numeric leaf \
             column (u8..i64, i128, f16/f32/f64); a {} column has no zero-copy in-place twin — use \
             the return-new `add` / `sub` / … (which also covers temporal and nested columns)",
            id.name()
        ),
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
/// operand of a binary op whose result type is `U` (the left's type). Per the "absorb anything" aim
/// it coerces a convertible operand of **any** leaf type into `U`, not just the twelve numeric ones:
///
/// - the **twelve numeric** leaves take the bulk [`Serie::cast`] (range-checked, one pass) — the
///   fast path a numeric source keeps;
/// - **utf8 / binary**, the **decimal** and **temporal** families, and the **wide integers**
///   (`u96`/`u128`/`u256`/`i96`/`i256`) are coerced **element-wise** through the shared per-cell
///   bridge [`any_scalar_into`] (a numeric-string parse, a decimal's `f64` value, a temporal's
///   backing count, a wide magnitude) — nulls preserved.
///
/// Only a genuinely non-convertible operand errors, guided: a **nested** column (no scalar value to
/// coerce), a non-numeric string, or an out-of-range magnitude.
fn cast_into<U: NumericCast + FromStr>(
    other: &(dyn AnySerie + 'static),
) -> Result<Serie<U>, IoError> {
    macro_rules! cast {
        ($t:ty) => {
            other
                .as_serie::<$t>()
                .expect("type_id names this concrete Serie")
                .cast::<U>()
                .map_err(cast_error)
        };
    }
    let id = other.type_id();
    match id {
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
        // A nested column has no scalar value to coerce into a number.
        _ if id.is_nested() => Err(right_not_convertible(id, U::NAME)),
        // Every other leaf family (utf8/binary, decimal, temporal, wide ints) is coerced per cell
        // through the shared `any_scalar_into` bridge; a numeric source already took the bulk cast.
        // DESIGN: per-cell (each cell's `value` is one erased scalar) is unavoidable for a string /
        // decimal / temporal source, and this is not the hot path; the numeric branch stays bulk.
        _ => {
            let len = other.len();
            let mut out: Vec<Option<U>> = Vec::with_capacity(len);
            for index in 0..len {
                out.push(any_scalar_into::<U>(&other.value(index))?);
            }
            Ok(Serie::from_options(&out))
        }
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

/// Coerces an erased scalar into a typed `U` value (range-checked), or `None` for a null scalar —
/// the shared per-cell bridge for both the scalar broadcast and the [`cast_into`] element-wise path.
/// Per the "absorb anything" aim it accepts a convertible leaf of **any** family, not just the
/// twelve numeric ones:
///
/// - the **twelve numeric** leaves: a width-guarded read + the exact [`Converter`] (fast, lossless
///   for integers);
/// - the **wide integers** (`u96`/`u128`/`u256`/`i96`/`i256`, not `NumericCast`): the LE magnitude
///   range-checked into `i128`, then the shared `Converter` into `U`;
/// - **utf8** (var + fixed): parse the numeric string (the [`Utf8Scalar::parse_to`] bridge);
/// - **binary** (var + fixed): reinterpret the raw LE bytes (the [`BinaryScalar::read_to`] bridge);
/// - **decimal**: its numeric value (`coeff × 10^-scale`) cast into `U` through `f64`;
/// - **temporal**: its backing integer **count** cast into `U`.
///
/// A guided error if the scalar is a nested value, a genuinely non-numeric string, or a value out of
/// `U`'s range.
///
/// [`Utf8Scalar::parse_to`]: crate::io::var::Utf8Scalar::parse_to
/// [`BinaryScalar::read_to`]: crate::io::var::BinaryScalar::read_to
fn any_scalar_into<U: NumericCast + FromStr>(value: &AnyScalar) -> Result<Option<U>, IoError> {
    let (field, bytes) = match value {
        AnyScalar::Null => return Ok(None),
        AnyScalar::Leaf { field, bytes } => (field, bytes),
        other => return Err(scalar_not_convertible(other.type_id())),
    };
    let id = FieldType::type_id(field);
    macro_rules! conv {
        ($t:ty) => {{
            // Width guard BEFORE `read_le`: a hand-built leaf (via the public `AnyScalar::leaf`) can
            // carry the right type_id but a wrong-length byte payload, which would panic in
            // `read_le`. Reject it with a guided error instead — no panic from any public op.
            let need = <$t as NativeType>::WIDTH;
            if bytes.len() != need {
                return Err(scalar_width_mismatch(id, bytes.len(), need));
            }
            <$t as Converter<U>>::cast_value(<$t as NativeType>::read_le(bytes))
                .map(Some)
                .map_err(cast_error)
        }};
    }
    match id {
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
        DataTypeId::U96
        | DataTypeId::U128
        | DataTypeId::U256
        | DataTypeId::I96
        | DataTypeId::I256 => wide_bytes_into::<U>(id, bytes),
        DataTypeId::Utf8 | DataTypeId::LargeUtf8 | DataTypeId::FixedUtf8 => {
            utf8_bytes_into::<U>(bytes)
        }
        DataTypeId::Binary | DataTypeId::LargeBinary | DataTypeId::FixedBinary => {
            binary_bytes_into::<U>(bytes)
        }
        DataTypeId::D32 | DataTypeId::D64 | DataTypeId::D128 | DataTypeId::D256 => {
            decimal_bytes_into::<U>(field, bytes)
        }
        other if other.is_temporal() => temporal_bytes_into::<U>(other, bytes),
        other => Err(scalar_not_convertible(Some(other))),
    }
}

// -------------------------------------------------------------------------------------
// Per-family value bridges — each converts one non-numeric leaf's bytes into the result type `U`,
// reusing the crate's existing conversion surface (`Converter`, the utf8/binary bridges).
// -------------------------------------------------------------------------------------

/// utf8 leaf → `U`: parse the trimmed numeric string — the [`Utf8Scalar::parse_to`] bridge, inlined
/// so a bulk serie coercion does not allocate one scalar per cell.
///
/// DESIGN: routes through `U`'s own `FromStr`, so an integer string is **exact**
/// (`"9223372036854775807"` → `i64` losslessly, unlike an `f64` bridge) and a fractional string
/// (`"2.5"`) reaches a float target. A non-numeric string is a guided parse error.
///
/// [`Utf8Scalar::parse_to`]: crate::io::var::Utf8Scalar::parse_to
fn utf8_bytes_into<U: NumericCast + FromStr>(bytes: &[u8]) -> Result<Option<U>, IoError> {
    let text =
        core::str::from_utf8(bytes).map_err(|_| not_parseable("<non-utf8 bytes>", U::NAME))?;
    text.trim()
        .parse::<U>()
        .map(Some)
        .map_err(|_| not_parseable(text, U::NAME))
}

/// binary leaf → `U`: reinterpret the raw little-endian bytes — the [`BinaryScalar::read_to`]
/// bridge. Errors, guided, when the byte width is not `U`'s.
///
/// [`BinaryScalar::read_to`]: crate::io::var::BinaryScalar::read_to
fn binary_bytes_into<U: NumericCast>(bytes: &[u8]) -> Result<Option<U>, IoError> {
    if bytes.len() == U::WIDTH {
        Ok(Some(U::read_le(bytes)))
    } else {
        Err(binary_width_mismatch(bytes.len(), U::WIDTH, U::NAME))
    }
}

/// Rejects a hand-built leaf (via the public [`AnyScalar::leaf`](crate::io::AnyScalar::leaf)) whose
/// byte payload length disagrees with its type's canonical fixed width — the same defense the numeric
/// `conv!` arm applies, so a malformed wide / temporal / decimal leaf scalar yields the guided width
/// error instead of silently misreading a wrong-length payload into a wrong value. A type with no
/// fixed width (utf8/binary handle their own length) is passed through.
fn guard_leaf_width(id: DataTypeId, got: usize) -> Result<(), IoError> {
    match id.fixed_byte_width() {
        Some(need) if got != need => Err(scalar_width_mismatch(id, got, need)),
        _ => Ok(()),
    }
}

/// wide-integer leaf → `U`: read the wide little-endian (two's-complement for the signed widths)
/// magnitude, range-check it into `i128`, then the shared `Converter` casts `i128` into `U`.
///
/// DESIGN: the wide byte-newtypes (`u96`/`u128`/`u256`/`i96`/`i256`) are not `NumericCast`, so this
/// is their numeric bridge — routed through `i128`, the crate's integer intermediate. A magnitude
/// beyond `i128` (already beyond every integer target's range) is a guided out-of-range error.
fn wide_bytes_into<U: NumericCast>(id: DataTypeId, bytes: &[u8]) -> Result<Option<U>, IoError> {
    guard_leaf_width(id, bytes.len())?;
    let magnitude = wide_le_to_i128(bytes, id.is_signed_integer())
        .ok_or_else(|| wide_out_of_range(id, U::NAME))?;
    <i128 as Converter<U>>::cast_value(magnitude)
        .map(Some)
        .map_err(cast_error)
}

/// temporal leaf → `U`: the backing integer **count** (sign-extended from its little-endian bytes)
/// cast into `U` through the shared `Converter` — so `i64.add(date32_col)` coerces the day count.
fn temporal_bytes_into<U: NumericCast>(id: DataTypeId, bytes: &[u8]) -> Result<Option<U>, IoError> {
    guard_leaf_width(id, bytes.len())?;
    let count = le_bytes_to_i128(bytes, true); // temporal counts are signed two's-complement
    <i128 as Converter<U>>::cast_value(count)
        .map(Some)
        .map_err(cast_error)
}

/// decimal leaf → `U`: its numeric value (`coefficient × 10^-scale`, the scale read from the field's
/// reserved metadata) cast into `U` through `f64`.
///
/// DESIGN: routed through `f64` (lossy beyond f64's 53-bit mantissa, exactly like `Decimal::to_f64`)
/// so an integer target truncates toward zero and a float target keeps the value — one total path. A
/// `d256` coefficient beyond `i128` is a guided out-of-range error, not a silent misread.
fn decimal_bytes_into<U: NumericCast>(field: &Field, bytes: &[u8]) -> Result<Option<U>, IoError> {
    guard_leaf_width(FieldType::type_id(field), bytes.len())?;
    let scale = field
        .metadata()
        .get(DataTypeId::SCALE_METADATA_KEY)
        .and_then(|value| value.parse::<i8>().ok())
        .unwrap_or(0);
    let coeff = wide_le_to_i128(bytes, true)
        .ok_or_else(|| wide_out_of_range(FieldType::type_id(field), U::NAME))?;
    let value = coeff as f64 / 10f64.powi(scale as i32);
    <f64 as Converter<U>>::cast_value(value)
        .map(Some)
        .map_err(cast_error)
}

/// Reads `bytes` (little-endian; `signed` two's-complement, else unsigned) as an `i128`, or `None`
/// if the value does **not** fit `i128`'s range — the range-checked numeric bridge for a wide
/// integer / a wide decimal coefficient. Sign/zero-extends a `≤ 16`-byte value; for a wider value
/// it must be exactly the sign extension in the high bytes (and the low 16 bytes must carry the
/// right sign), else the magnitude is out of range.
fn wide_le_to_i128(bytes: &[u8], signed: bool) -> Option<i128> {
    let n = bytes.len();
    if n == 0 {
        return Some(0);
    }
    let negative = signed && bytes[n - 1] & 0x80 != 0;
    if n <= 16 {
        let mut buf = if negative { [0xffu8; 16] } else { [0u8; 16] };
        buf[..n].copy_from_slice(bytes);
        // A full-width unsigned value with the top bit set exceeds i128::MAX.
        if !signed && n == 16 && buf[15] & 0x80 != 0 {
            return None;
        }
        Some(i128::from_le_bytes(buf))
    } else {
        // More than 16 bytes: every high byte must equal the sign extension, and the low 16 bytes'
        // own sign bit must match, else the magnitude overflows i128.
        let fill = if negative { 0xff } else { 0x00 };
        if bytes[16..].iter().any(|&byte| byte != fill) {
            return None;
        }
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&bytes[..16]);
        if (buf[15] & 0x80 != 0) != negative {
            return None;
        }
        Some(i128::from_le_bytes(buf))
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
    // is the SAME temporal type (its raw counts read directly) or an INTEGER leaf routed through the
    // range-checked integer path (`any_scalar_into::<i128>`, the scalar twin of `cast_into::<i128>`).
    // That integer path now also coerces a WIDE integer (u96/u128/u256/i96/i256) correctly — its LE
    // magnitude range-checked into i128 — so a wide offset applies in range and errors out of range,
    // never a silent misread. A DIFFERENT temporal type (or a non-integer leaf) still errors, guided.
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
                // An integer operand — coerced into the backing count (i128), range-checked, exactly
                // as the serie path's `cast_into::<i128>`. A wide integer (`u96`/`u128`/… — not
                // `NumericCast`) is read correctly through the broadened bridge (its LE magnitude
                // range-checked into i128), so an in-range wide offset applies and an out-of-range
                // one is a guided error — never the old silent byte-misread.
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

fn right_not_convertible(id: DataTypeId, target: &str) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot use a {} column as the right operand of an arithmetic op producing `{target}`; \
             the right operand must be convertible into the left's element type — a numeric column, \
             a numeric-string utf8 column, a binary column of {target}-width values, a decimal, a \
             temporal, or a wide-integer column (a nested column has no scalar value to coerce)",
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

fn scalar_not_convertible(id: Option<DataTypeId>) -> IoError {
    let got = id.map_or("null", DataTypeId::name);
    IoError::Unsupported {
        what: format!(
            "cannot broadcast a {got} scalar in an arithmetic op; the scalar must be convertible \
             into the column's element type — a numeric value, a numeric string, a decimal, a \
             temporal, or a wide integer (or null for an all-null result)"
        ),
    }
}

/// A utf8 operand cell could not be parsed as the result type — guided (naming the text + target).
fn not_parseable(text: &str, target: &str) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot parse {text:?} as `{target}` for an arithmetic op; a utf8 operand must hold a \
             number the target type can represent (an integer literal for an integer target, a \
             decimal literal for a float target)"
        ),
    }
}

/// A binary operand cell's byte width does not match the result type's — guided.
fn binary_width_mismatch(got: usize, need: usize, target: &str) -> IoError {
    IoError::Unsupported {
        what: format!(
            "a binary operand is {got} bytes, but the result type `{target}` reads exactly {need} \
             little-endian bytes; a binary value is reinterpreted as the target type's raw bytes"
        ),
    }
}

/// A wide integer / a wide decimal coefficient whose magnitude exceeds the `i128` bridge — guided.
fn wide_out_of_range(id: DataTypeId, target: &str) -> IoError {
    IoError::Unsupported {
        what: format!(
            "a {} value is out of range for `{target}`; its magnitude exceeds the i128 integer \
             bridge the arithmetic coercion routes a wide operand through",
            id.name()
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
