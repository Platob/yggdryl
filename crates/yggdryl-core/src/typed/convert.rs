//! [`convert_column`] — the **one place** every any→any column conversion lives.
//!
//! One entry point converts a column of **any** [`DataTypeId`] to **any** other, each arm delegating
//! to an existing **optimized kernel** — never a fresh scalar loop where a vectorized op already
//! exists:
//!
//! | from → to | kernel reused |
//! |---|---|
//! | numeric ↔ numeric (incl. `bool → numeric`, `numeric → bool` as 0/1) | [`resize_dtype`](crate::io::memory::IOBase::resize_dtype) + the bit pack/unpack |
//! | numeric → utf8 (format) / utf8 → numeric (flexible parse) | [`to_string_options`](crate::typed::FixedSerie::to_string_options) / [`from_strings`](crate::typed::FixedSerie::from_strings) |
//! | binary ↔ utf8 (and `Large*` / `Fixed*` among the byte family) | the offsets+data reinterpret (no data-buffer re-copy when the offset width matches) |
//! | decimal ↔ numeric | the decimal [`LogicalType`](crate::typed::LogicalType) — the unscaled physical integer (a same-width `decimal ↔ i128` is a **zero-copy relabel**) |
//! | decimal → utf8 | the scale-aware [`to_decimal_string`](crate::typed::FixedSerie::to_decimal_string) |
//! | Null → X / X → Null / Any (or `to.is_any()`) → X | an all-null `X` / a bufferless [`Null`](Column::Null) run / the column unchanged |
//! | same dtype | a no-op clone (+ optional metadata reshape from `out_field`) |
//!
//! A genuinely unsupported pair (numeric → struct, `decimal256 ↔ numeric`, …) returns a **guided**
//! [`IoError`] naming the pair and the closest supported route. The erased [`Column::cast_field`]
//! routes through this same function, so the field-driven cast the bindings call and the raw dtype
//! change share **one** implementation.

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase, IoError};
use crate::typed::fixedbit::Bit;
use crate::typed::fixedbyte::{
    Decimal128, Decimal16, Decimal256, Decimal32, Decimal64, Decimal8, FixedBinary, FixedUtf8,
    Float16, Float32, Float64, Int128, Int16, Int32, Int64, Int8, UInt128, UInt16, UInt32, UInt64,
    UInt8,
};
use crate::typed::varbyte::{Binary, LargeBinary, LargeUtf8, Utf8};
use crate::typed::{
    Column, ColumnField, Decoder, Encoder, FixedSerie, FixedSizeSerie, FlexibleFromStr, Scalar,
    Serie, VarSerie,
};

/// The **physical** [`DataTypeId`] a logical id decays to for the converter — the runtime companion
/// of the compile-time [`LogicalType::physical_dtype`](crate::typed::LogicalType::physical_dtype)
/// (a test pins the two in lockstep). A decimal decays to its unscaled signed integer; a UTF-8
/// string to the byte-identical binary layout; everything else (including `Decimal256`, whose
/// 256-bit physical has no numeric dtype) is its own physical.
pub(crate) fn physical_dtype(id: DataTypeId) -> DataTypeId {
    match id {
        DataTypeId::Decimal8 => DataTypeId::I8,
        DataTypeId::Decimal16 => DataTypeId::I16,
        DataTypeId::Decimal32 => DataTypeId::I32,
        DataTypeId::Decimal64 => DataTypeId::I64,
        DataTypeId::Decimal128 => DataTypeId::I128,
        DataTypeId::Utf8 => DataTypeId::Binary,
        DataTypeId::LargeUtf8 => DataTypeId::LargeBinary,
        DataTypeId::FixedUtf8 => DataTypeId::FixedBinary,
        other => other,
    }
}

/// Whether `id` participates in the numeric **resize** family — an integer, a float, or a decimal
/// with an id-derivable numeric physical (`Decimal32`/`64`/`128`, but **not** `Decimal256`, whose
/// 256-bit magnitude cannot round-trip through the `f64` carrier the resize uses).
fn resizable(id: DataTypeId) -> bool {
    (id.is_integer() || id.is_float() || id.is_decimal()) && id != DataTypeId::Decimal256
}

/// The guided [`IoError::TypedCast`] for a pair with no supported conversion — names both dtypes and
/// the closest supported route (or why this exact pair has none).
fn unsupported(from: DataTypeId, to: DataTypeId) -> IoError {
    let hint = if from == DataTypeId::Decimal256 || to == DataTypeId::Decimal256 {
        "decimal256 has no i128/f64 physical to reinterpret through — build the target column \
         directly, or go decimal256 → utf8 for a human-readable form"
    } else if from.is_nested() || to.is_nested() {
        "a nested struct/list/map has no flat element reinterpretation — build the target column \
         directly from its rows"
    } else {
        "no supported route for this pair — supported are numeric↔numeric, bool↔numeric, \
         numeric/decimal↔utf8, binary↔utf8 (incl. Large*/Fixed*), decimal↔numeric (unscaled), and \
         null↔any; build the target column directly or route via a supported intermediate type"
    };
    IoError::TypedCast {
        detail: format!("cannot convert a {from} column to {to}: {hint}"),
    }
}

/// Converts `src` to the element type `to`, reusing the matching optimized kernel — **the single
/// any→any entry point**. When `out_field` is a leaf field it is applied to the result (nullability
/// / name / precision·scale / annotations); pass `None` to keep the conversion's natural metadata.
///
/// - **`Any` target** (`to.is_any()`) → the column unchanged (a clone).
/// - **same dtype** → a no-op clone, plus the `out_field` metadata reshape when given.
/// - **`X → Null`** → the cheapest bufferless [`Null`](Column::Null) run; **`Null → X`** → an
///   all-null `X` column, both of the same length.
/// - otherwise the dtype is converted through the matrix in the module docs.
///
/// ```
/// use yggdryl_core::datatype_id::DataTypeId;
/// use yggdryl_core::typed::{convert_column, Column, FixedSerie, Scalar, Value};
/// use yggdryl_core::typed::fixedbyte::Int64;
///
/// let col = Column::from(FixedSerie::<Int64>::from_values(&[1, 2, 3]));
/// let as_i32 = convert_column(&col, DataTypeId::I32, None).unwrap();
/// assert_eq!(as_i32.data_type_id(), DataTypeId::I32);
/// assert_eq!(as_i32.get(0), Value::Int32(1));
///
/// let as_text = convert_column(&col, DataTypeId::Utf8, None).unwrap();
/// assert_eq!(as_text.get(2), Value::Utf8("3".into()));
/// ```
pub fn convert_column(
    src: &Column,
    to: DataTypeId,
    out_field: Option<&ColumnField>,
) -> Result<Column, IoError> {
    let from = src.data_type_id();
    // An `Any` target keeps the erased column exactly as-is (it already "holds any type").
    if to.is_any() {
        return Ok(src.clone());
    }
    // Same dtype: a no-op clone, plus an optional metadata reshape from `out_field`.
    if to == from {
        return apply_field(src.clone(), out_field);
    }
    // X → Null: the cheapest bufferless all-null run of the same length.
    if to.is_null_type() {
        return Ok(Column::Null(src.len()));
    }
    // Null → X: an all-null X column of the same length.
    if from.is_null_type() {
        return apply_field(all_null_of(to, src.len()), out_field);
    }
    let converted = dispatch(src, from, to)?;
    apply_field(converted, out_field)
}

/// The **in-place** twin of [`convert_column`] — replaces `*src` with its `to`-typed conversion.
/// A no-op target (`Any`, or a same-dtype cast with no `out_field` to apply) touches nothing;
/// otherwise it delegates to [`convert_column`].
///
// DESIGN: a same-family numeric resize could rewrite `self`'s data buffer with
// `resize_dtype_in_place` (no clone), but the erased [`Column`] does not expose a mutable data
// backing, so the in-place form reuses `convert_column` (correct, one clone) rather than plumbing a
// `data_mut` accessor through every carrier. The cheap no-op fast paths are handled directly.
pub fn convert_column_in_place(
    src: &mut Column,
    to: DataTypeId,
    out_field: Option<&ColumnField>,
) -> Result<(), IoError> {
    let from = src.data_type_id();
    if to.is_any() || (to == from && out_field.is_none()) {
        return Ok(());
    }
    *src = convert_column(src, to, out_field)?;
    Ok(())
}

impl Column {
    /// The erased **field-driven cast** — the binding-facing any→any entry. A **same-dtype** `field`
    /// reshapes metadata (nullability / name / precision·scale / annotations) over the same bytes; a
    /// **different-dtype** `field` converts the element type through the one [`convert_column`]
    /// matrix. The dtype-change logic is **not** duplicated here — this is a thin call into
    /// `convert_column`, so the field cast the bindings expose and a raw [`convert_column`] share the
    /// single implementation.
    ///
    /// ```
    /// use yggdryl_core::datatype_id::DataTypeId;
    /// use yggdryl_core::typed::{Column, ColumnField, FixedSerie, HeaderField, Scalar};
    /// use yggdryl_core::typed::fixedbyte::Int64;
    ///
    /// let col = Column::from(FixedSerie::<Int64>::from_values(&[1, 2, 3]));
    /// // A different-dtype field converts the element type and applies the field metadata.
    /// let field = ColumnField::Leaf(HeaderField::new(Some("n"), DataTypeId::I16, false));
    /// let out = col.cast_field(&field).unwrap();
    /// assert_eq!(out.data_type_id(), DataTypeId::I16);
    /// assert_eq!(out.name(), Some("n"));
    /// assert_eq!(out.get(2), yggdryl_core::typed::Value::Int16(3));
    /// ```
    pub fn cast_field(&self, field: &ColumnField) -> Result<Column, IoError> {
        convert_column(self, field.data_type_id(), Some(field))
    }
}

// -------------------------------------------------------------------------------------
// The category dispatch — from and to are both concrete, non-Null, and different.
// -------------------------------------------------------------------------------------

fn dispatch(src: &Column, from: DataTypeId, to: DataTypeId) -> Result<Column, IoError> {
    // Decimal → utf8 is scale-aware (checked before the generic numeric → utf8).
    if from.is_decimal() && to.is_utf8() {
        return Ok(build_utf8(to, decimal_string_options(src)));
    }
    // bool target / bool source (bit-packed — handled via the bit pack/unpack, not resize_dtype).
    if to.is_bool() {
        return to_bool(src, from);
    }
    if from.is_bool() {
        return from_bool(src, to);
    }
    // binary ↔ utf8 (and Large* / Fixed*) — a physical-bytes reinterpret.
    if from.is_byte_like() && to.is_byte_like() {
        return reinterpret_bytes(src, from, to);
    }
    // numeric (int/float) → utf8 (format) and utf8 → numeric (flexible parse).
    if resizable(from) && to.is_utf8() {
        return Ok(build_utf8(to, numeric_string_options(src)?));
    }
    if from.is_utf8() && resizable(to) {
        return utf8_to_numeric(src, to);
    }
    // numeric ↔ numeric (incl. decimal32/64/128 via the LogicalType physical).
    if resizable(from) && resizable(to) {
        return numeric_resize(src, from, to);
    }
    Err(unsupported(from, to))
}

// -------------------------------------------------------------------------------------
// numeric ↔ numeric — via resize_dtype over the LogicalType physical
// -------------------------------------------------------------------------------------

/// Clones a resizable numeric/decimal source's `(data, validity, len)`; `None` for any other column.
fn numeric_parts(src: &Column) -> Option<(Heap, Option<Heap>, usize)> {
    macro_rules! parts {
        ($s:expr) => {
            Some(($s.data().clone(), $s.validity().cloned(), $s.len()))
        };
    }
    match src {
        Column::Int8(s) => parts!(s),
        Column::UInt8(s) => parts!(s),
        Column::Int16(s) => parts!(s),
        Column::UInt16(s) => parts!(s),
        Column::Int32(s) => parts!(s),
        Column::UInt32(s) => parts!(s),
        Column::Int64(s) => parts!(s),
        Column::UInt64(s) => parts!(s),
        Column::Int128(s) => parts!(s),
        Column::UInt128(s) => parts!(s),
        Column::Float16(s) => parts!(s),
        Column::Float32(s) => parts!(s),
        Column::Float64(s) => parts!(s),
        Column::Decimal8(s) => parts!(s),
        Column::Decimal16(s) => parts!(s),
        Column::Decimal32(s) => parts!(s),
        Column::Decimal64(s) => parts!(s),
        Column::Decimal128(s) => parts!(s),
        _ => None,
    }
}

/// Wraps a converted `data` buffer as the resizable numeric/decimal column `to`.
fn build_numeric(to: DataTypeId, data: Heap, validity: Option<Heap>, len: usize) -> Column {
    macro_rules! b {
        ($m:ty) => {
            FixedSerie::<$m>::from_data(data, validity, len).into()
        };
    }
    match to {
        DataTypeId::I8 => b!(Int8),
        DataTypeId::U8 => b!(UInt8),
        DataTypeId::I16 => b!(Int16),
        DataTypeId::U16 => b!(UInt16),
        DataTypeId::I32 => b!(Int32),
        DataTypeId::U32 => b!(UInt32),
        DataTypeId::I64 => b!(Int64),
        DataTypeId::U64 => b!(UInt64),
        DataTypeId::I128 => b!(Int128),
        DataTypeId::U128 => b!(UInt128),
        DataTypeId::Float16 => b!(Float16),
        DataTypeId::F32 => b!(Float32),
        DataTypeId::F64 => b!(Float64),
        DataTypeId::Decimal8 => b!(Decimal8),
        DataTypeId::Decimal16 => b!(Decimal16),
        DataTypeId::Decimal32 => b!(Decimal32),
        DataTypeId::Decimal64 => b!(Decimal64),
        DataTypeId::Decimal128 => b!(Decimal128),
        _ => unreachable!("build_numeric only serves the resizable numeric/decimal dtypes"),
    }
}

fn numeric_resize(src: &Column, from: DataTypeId, to: DataTypeId) -> Result<Column, IoError> {
    let (data, validity, len) =
        numeric_parts(src).expect("numeric_resize is only reached for a resizable source");
    let from_phys = physical_dtype(from);
    let to_phys = physical_dtype(to);
    let mut buf = data;
    buf.set_dtype(from_phys);
    // Same physical width (e.g. decimal128 ↔ i128, i32 ↔ decimal32): a zero-copy relabel — the
    // unscaled bytes are identical, so nothing is reinterpreted through the f64 carrier. Otherwise
    // reuse the vectorized, saturating resize_dtype kernel over the physical integers/floats.
    let heap = if from_phys == to_phys {
        buf
    } else {
        buf.resize_dtype(to_phys)?
    };
    Ok(build_numeric(to, heap, validity, len))
}

// -------------------------------------------------------------------------------------
// numeric / decimal / bool → utf8, and utf8 → numeric / bool
// -------------------------------------------------------------------------------------

/// Every element of a numeric (int/float) source formatted null-aware (the bulk
/// `to_string_options`). Never reached for a decimal (routed to [`decimal_string_options`]).
fn numeric_string_options(src: &Column) -> Result<Vec<Option<String>>, IoError> {
    macro_rules! s {
        ($ss:expr) => {
            $ss.to_string_options()
        };
    }
    match src {
        Column::Int8(s) => s!(s),
        Column::UInt8(s) => s!(s),
        Column::Int16(s) => s!(s),
        Column::UInt16(s) => s!(s),
        Column::Int32(s) => s!(s),
        Column::UInt32(s) => s!(s),
        Column::Int64(s) => s!(s),
        Column::UInt64(s) => s!(s),
        Column::Int128(s) => s!(s),
        Column::UInt128(s) => s!(s),
        Column::Float16(s) => s!(s),
        Column::Float32(s) => s!(s),
        Column::Float64(s) => s!(s),
        _ => unreachable!("numeric_string_options only serves int/float sources"),
    }
}

/// Every decimal element rendered **scale-aware** (`unscaled 12345`, scale `2` → `"123.45"`),
/// null-aware — the scale-carrying [`to_decimal_string`](FixedSerie::to_decimal_string).
fn decimal_string_options(src: &Column) -> Vec<Option<String>> {
    macro_rules! s {
        ($ss:expr) => {
            (0..$ss.len()).map(|i| $ss.to_decimal_string(i)).collect()
        };
    }
    match src {
        Column::Decimal8(s) => s!(s),
        Column::Decimal16(s) => s!(s),
        Column::Decimal32(s) => s!(s),
        Column::Decimal64(s) => s!(s),
        Column::Decimal128(s) => s!(s),
        Column::Decimal256(s) => s!(s),
        _ => unreachable!("decimal_string_options only serves decimal sources"),
    }
}

/// The longest byte length among the non-null strings (the fixed-size UTF-8 width).
fn max_len(opts: &[Option<String>]) -> usize {
    opts.iter()
        .filter_map(|o| o.as_ref().map(String::len))
        .max()
        .unwrap_or(0)
}

/// Builds a UTF-8 column of dtype `to` from the null-aware formatted strings.
fn build_utf8(to: DataTypeId, opts: Vec<Option<String>>) -> Column {
    match to {
        DataTypeId::Utf8 => VarSerie::<Utf8>::from_options(&opts).into(),
        DataTypeId::LargeUtf8 => VarSerie::<LargeUtf8>::from_options(&opts).into(),
        DataTypeId::FixedUtf8 => {
            let width = max_len(&opts);
            FixedSizeSerie::<FixedUtf8>::from_options(width, &opts).into()
        }
        _ => unreachable!("build_utf8 only serves the utf8 dtypes"),
    }
}

/// The null-aware owned strings of a UTF-8 source (`Utf8` / `LargeUtf8` / `FixedUtf8`).
fn utf8_string_options(src: &Column) -> Vec<Option<String>> {
    match src {
        Column::Utf8(s) => s.to_options(),
        Column::LargeUtf8(s) => s.to_options(),
        Column::FixedUtf8(s) => s.to_options(),
        _ => unreachable!("utf8_string_options only serves utf8 sources"),
    }
}

/// Parses null-aware strings into a fixed-width column via the **bulk** [`from_strings`], then
/// re-applies the source nulls (a null parses through a `"0"` placeholder, then is cleared).
fn parse_utf8_into<M: Encoder + Decoder>(opts: &[Option<String>]) -> Result<FixedSerie<M>, IoError>
where
    M::Native: FlexibleFromStr,
{
    let strings: Vec<&str> = opts.iter().map(|o| o.as_deref().unwrap_or("0")).collect();
    let mut col = FixedSerie::<M>::from_strings(&strings)?;
    for (index, opt) in opts.iter().enumerate() {
        if opt.is_none() {
            col.set_null(index)?;
        }
    }
    Ok(col)
}

fn utf8_to_numeric(src: &Column, to: DataTypeId) -> Result<Column, IoError> {
    let opts = utf8_string_options(src);
    macro_rules! b {
        ($m:ty) => {
            Ok(parse_utf8_into::<$m>(&opts)?.into())
        };
    }
    match to {
        DataTypeId::I8 => b!(Int8),
        DataTypeId::U8 => b!(UInt8),
        DataTypeId::I16 => b!(Int16),
        DataTypeId::U16 => b!(UInt16),
        DataTypeId::I32 => b!(Int32),
        DataTypeId::U32 => b!(UInt32),
        DataTypeId::I64 => b!(Int64),
        DataTypeId::U64 => b!(UInt64),
        DataTypeId::I128 => b!(Int128),
        DataTypeId::U128 => b!(UInt128),
        DataTypeId::Float16 => b!(Float16),
        DataTypeId::F32 => b!(Float32),
        DataTypeId::F64 => b!(Float64),
        // A decimal target parses the **unscaled** integer string (the decimal's physical).
        DataTypeId::Decimal8 => b!(Decimal8),
        DataTypeId::Decimal16 => b!(Decimal16),
        DataTypeId::Decimal32 => b!(Decimal32),
        DataTypeId::Decimal64 => b!(Decimal64),
        DataTypeId::Decimal128 => b!(Decimal128),
        _ => unreachable!("utf8_to_numeric only serves the resizable numeric/decimal dtypes"),
    }
}

// -------------------------------------------------------------------------------------
// bool ↔ everything (the bit pack/unpack)
// -------------------------------------------------------------------------------------

/// A resizable numeric/decimal source unpacked to null-aware booleans (`value != 0`); `None` for
/// any non-numeric source.
fn numeric_to_bool_options(src: &Column) -> Option<Vec<Option<bool>>> {
    macro_rules! opts {
        ($s:expr) => {
            Some(
                $s.to_options()
                    .into_iter()
                    .map(|o| o.map(|v| v != Default::default()))
                    .collect(),
            )
        };
    }
    match src {
        Column::Int8(s) => opts!(s),
        Column::UInt8(s) => opts!(s),
        Column::Int16(s) => opts!(s),
        Column::UInt16(s) => opts!(s),
        Column::Int32(s) => opts!(s),
        Column::UInt32(s) => opts!(s),
        Column::Int64(s) => opts!(s),
        Column::UInt64(s) => opts!(s),
        Column::Int128(s) => opts!(s),
        Column::UInt128(s) => opts!(s),
        // A half unpacks to `value != 0` by **value** (through `f32`), so `-0.0` is `false` and NaN
        // is `true` — matching the `f32` / `f64` bool cast (their `!= 0.0` is IEEE, not bitwise).
        Column::Float16(s) => Some(
            s.to_options()
                .into_iter()
                .map(|o| o.map(|v| v.to_f32() != 0.0))
                .collect(),
        ),
        Column::Float32(s) => opts!(s),
        Column::Float64(s) => opts!(s),
        Column::Decimal8(s) => opts!(s),
        Column::Decimal16(s) => opts!(s),
        Column::Decimal32(s) => opts!(s),
        Column::Decimal64(s) => opts!(s),
        Column::Decimal128(s) => opts!(s),
        _ => None,
    }
}

/// Any → bool: a UTF-8 source parses each string as a boolean (`from_strings`); a numeric source
/// packs `value != 0`. Both build a bit-packed [`Bit`] column.
fn to_bool(src: &Column, from: DataTypeId) -> Result<Column, IoError> {
    if from.is_utf8() {
        let opts = utf8_string_options(src);
        return Ok(parse_utf8_into::<Bit>(&opts)?.into());
    }
    if let Some(opts) = numeric_to_bool_options(src) {
        return Ok(FixedSerie::<Bit>::from_options(&opts).into());
    }
    Err(unsupported(from, DataTypeId::Bool))
}

/// bool → any: to UTF-8 renders `"true"`/`"false"`; to a numeric maps the bits to `0`/`1` (built as
/// an `i8` column, then resized to the target width through the numeric kernel).
fn from_bool(src: &Column, to: DataTypeId) -> Result<Column, IoError> {
    let Column::Bool(bits) = src else {
        return Err(unsupported(DataTypeId::Bool, to));
    };
    if to.is_utf8() {
        return Ok(build_utf8(to, bits.to_string_options()?));
    }
    if resizable(to) {
        let opts: Vec<Option<i8>> = bits
            .to_options()
            .into_iter()
            .map(|o| o.map(i8::from))
            .collect();
        let i8_col = Column::Int8(FixedSerie::<Int8>::from_options(&opts));
        if to == DataTypeId::I8 {
            return Ok(i8_col);
        }
        return numeric_resize(&i8_col, DataTypeId::I8, to);
    }
    Err(unsupported(DataTypeId::Bool, to))
}

// -------------------------------------------------------------------------------------
// binary ↔ utf8 (and Large* / Fixed*) — the offsets+data reinterpret
// -------------------------------------------------------------------------------------

/// A variable-length byte source's `(offsets, data, validity, len)` — a clone of each backing buffer.
fn var_parts(src: &Column) -> (Heap, Heap, Option<Heap>, usize) {
    macro_rules! p {
        ($s:expr) => {
            (
                $s.offsets().clone(),
                $s.data().clone(),
                $s.validity().cloned(),
                $s.len(),
            )
        };
    }
    match src {
        Column::Binary(s) => p!(s),
        Column::Utf8(s) => p!(s),
        Column::LargeBinary(s) => p!(s),
        Column::LargeUtf8(s) => p!(s),
        _ => unreachable!("var_parts only serves variable-length byte sources"),
    }
}

/// A fixed-size byte source's `(data, validity, len, width)` — a clone of the data + validity.
fn fixed_parts(src: &Column) -> (Heap, Option<Heap>, usize, usize) {
    match src {
        Column::FixedBinary(s) => (s.data().clone(), s.validity().cloned(), s.len(), s.width()),
        Column::FixedUtf8(s) => (s.data().clone(), s.validity().cloned(), s.len(), s.width()),
        _ => unreachable!("fixed_parts only serves fixed-size byte sources"),
    }
}

/// Wraps offsets + data + validity as a variable-length byte column of dtype `to`.
fn build_var(
    to: DataTypeId,
    offsets: Heap,
    data: Heap,
    validity: Option<Heap>,
    len: usize,
) -> Column {
    match to {
        DataTypeId::Binary => VarSerie::<Binary>::from_parts(offsets, data, validity, len).into(),
        DataTypeId::Utf8 => VarSerie::<Utf8>::from_parts(offsets, data, validity, len).into(),
        DataTypeId::LargeBinary => {
            VarSerie::<LargeBinary>::from_parts(offsets, data, validity, len).into()
        }
        DataTypeId::LargeUtf8 => {
            VarSerie::<LargeUtf8>::from_parts(offsets, data, validity, len).into()
        }
        _ => unreachable!("build_var only serves the variable-length byte dtypes"),
    }
}

/// Wraps data + validity as a fixed-size byte column of dtype `to` at `width`.
fn build_fixed(
    to: DataTypeId,
    data: Heap,
    validity: Option<Heap>,
    len: usize,
    width: usize,
) -> Column {
    match to {
        DataTypeId::FixedBinary => {
            FixedSizeSerie::<FixedBinary>::from_parts(data, validity, len, width).into()
        }
        DataTypeId::FixedUtf8 => {
            FixedSizeSerie::<FixedUtf8>::from_parts(data, validity, len, width).into()
        }
        _ => unreachable!("build_fixed only serves the fixed-size byte dtypes"),
    }
}

/// Re-encodes `len + 1` offsets from `from_large`'s width to `to_large`'s width (i32 ↔ i64) through
/// the vectorized array read/write — the only work when the offset widths differ; the data buffer is
/// still cloned exactly once (never re-copied per element).
fn convert_offsets(
    offsets: &Heap,
    len: usize,
    from_large: bool,
    to_large: bool,
) -> Result<Heap, IoError> {
    let count = len + 1;
    let to_width = if to_large { 8 } else { 4 };
    let mut out = Heap::with_capacity(count * to_width);
    if from_large {
        let mut vals = vec![0i64; count];
        offsets.pread_i64_array(0, &mut vals)?;
        if to_large {
            out.pwrite_i64_array(0, &vals)?;
        } else {
            let narrowed: Vec<i32> = vals.iter().map(|&v| v as i32).collect();
            out.pwrite_i32_array(0, &narrowed)?;
        }
    } else {
        let mut vals = vec![0i32; count];
        offsets.pread_i32_array(0, &mut vals)?;
        if to_large {
            let widened: Vec<i64> = vals.iter().map(|&v| v as i64).collect();
            out.pwrite_i64_array(0, &widened)?;
        } else {
            out.pwrite_i32_array(0, &vals)?;
        }
    }
    Ok(out)
}

/// Raw element bytes of any byte source, null-aware, plus the longest element (the fixed-size width).
fn byte_values(src: &Column) -> (Vec<Option<Vec<u8>>>, usize) {
    macro_rules! v {
        ($s:expr) => {{
            let len = $s.len();
            let mut out = Vec::with_capacity(len);
            let mut width = 0usize;
            for index in 0..len {
                if $s.is_valid(index) {
                    let bytes = $s.bytes_at(index).unwrap_or_default();
                    width = width.max(bytes.len());
                    out.push(Some(bytes));
                } else {
                    out.push(None);
                }
            }
            (out, width)
        }};
    }
    match src {
        Column::Binary(s) => v!(s),
        Column::Utf8(s) => v!(s),
        Column::LargeBinary(s) => v!(s),
        Column::LargeUtf8(s) => v!(s),
        Column::FixedBinary(s) => v!(s),
        Column::FixedUtf8(s) => v!(s),
        _ => unreachable!("byte_values only serves byte-like sources"),
    }
}

/// Builds a byte column of dtype `to` by re-pushing the raw element bytes — the fallback for a
/// variable ↔ fixed-size reshape, which has no contiguous vectorized reinterpretation.
fn build_byte_from_values(to: DataTypeId, values: Vec<Option<Vec<u8>>>, width: usize) -> Column {
    macro_rules! var {
        ($m:ty) => {{
            let mut col = VarSerie::<$m>::new();
            for value in &values {
                match value {
                    Some(bytes) => col.push_bytes(bytes),
                    None => col.push_null(),
                }
            }
            col.into()
        }};
    }
    macro_rules! fixed {
        ($m:ty) => {{
            let mut col = FixedSizeSerie::<$m>::new(width);
            for value in &values {
                match value {
                    Some(bytes) => col.push_bytes(bytes),
                    None => col.push_null(),
                }
            }
            col.into()
        }};
    }
    match to {
        DataTypeId::Binary => var!(Binary),
        DataTypeId::Utf8 => var!(Utf8),
        DataTypeId::LargeBinary => var!(LargeBinary),
        DataTypeId::LargeUtf8 => var!(LargeUtf8),
        DataTypeId::FixedBinary => fixed!(FixedBinary),
        DataTypeId::FixedUtf8 => fixed!(FixedUtf8),
        _ => unreachable!("build_byte_from_values only serves byte-like dtypes"),
    }
}

fn reinterpret_bytes(src: &Column, from: DataTypeId, to: DataTypeId) -> Result<Column, IoError> {
    // Both fixed-size: clone the data + validity at the same byte stride (no offsets).
    if from.is_fixed_size() && to.is_fixed_size() {
        let (data, validity, len, width) = fixed_parts(src);
        return Ok(build_fixed(to, data, validity, len, width));
    }
    // Both variable-length: reuse the offsets buffer as-is when the width matches (a zero-copy
    // relabel of the same physical bytes), else re-encode only the offsets; the data buffer is
    // cloned exactly once either way.
    if from.is_variable_length() && to.is_variable_length() {
        let (offsets, data, validity, len) = var_parts(src);
        let new_offsets = if from.is_large() == to.is_large() {
            offsets
        } else {
            convert_offsets(&offsets, len, from.is_large(), to.is_large())?
        };
        return Ok(build_var(to, new_offsets, data, validity, len));
    }
    // Mixed variable ↔ fixed-size: rebuild through the raw element bytes (no vectorized path).
    let (values, width) = byte_values(src);
    Ok(build_byte_from_values(to, values, width))
}

// -------------------------------------------------------------------------------------
// Null → X and the out_field metadata reshape
// -------------------------------------------------------------------------------------

/// An all-null column of dtype `to` with `len` elements — the `Null → X` conversion. A dtype with no
/// leaf carrier (`Any` / `Unknown` / a nested composite) degrades to the bufferless
/// [`Null`](Column::Null) run.
fn all_null_of(to: DataTypeId, len: usize) -> Column {
    macro_rules! num {
        ($m:ty) => {
            FixedSerie::<$m>::from_options(&vec![None; len]).into()
        };
    }
    macro_rules! var {
        ($m:ty) => {
            VarSerie::<$m>::from_options(&vec![None; len]).into()
        };
    }
    match to {
        DataTypeId::Bool => num!(Bit),
        DataTypeId::I8 => num!(Int8),
        DataTypeId::U8 => num!(UInt8),
        DataTypeId::I16 => num!(Int16),
        DataTypeId::U16 => num!(UInt16),
        DataTypeId::I32 => num!(Int32),
        DataTypeId::U32 => num!(UInt32),
        DataTypeId::I64 => num!(Int64),
        DataTypeId::U64 => num!(UInt64),
        DataTypeId::I128 => num!(Int128),
        DataTypeId::U128 => num!(UInt128),
        DataTypeId::Float16 => num!(Float16),
        DataTypeId::F32 => num!(Float32),
        DataTypeId::F64 => num!(Float64),
        DataTypeId::Decimal8 => num!(Decimal8),
        DataTypeId::Decimal16 => num!(Decimal16),
        DataTypeId::Decimal32 => num!(Decimal32),
        DataTypeId::Decimal64 => num!(Decimal64),
        DataTypeId::Decimal128 => num!(Decimal128),
        DataTypeId::Decimal256 => num!(Decimal256),
        DataTypeId::Binary => var!(Binary),
        DataTypeId::Utf8 => var!(Utf8),
        DataTypeId::LargeBinary => var!(LargeBinary),
        DataTypeId::LargeUtf8 => var!(LargeUtf8),
        DataTypeId::FixedBinary => {
            FixedSizeSerie::<FixedBinary>::from_options(0, &vec![None; len]).into()
        }
        DataTypeId::FixedUtf8 => {
            FixedSizeSerie::<FixedUtf8>::from_options(0, &vec![None; len]).into()
        }
        // Null (already), Any, Unknown, and the nested composites have no all-null leaf carrier.
        _ => Column::Null(len),
    }
}

/// Applies a leaf `out_field`'s metadata to the (already dtype-converted) column — nullability, name,
/// annotations, and (for a decimal) precision·scale — through the concrete carrier's `cast_field`.
///
// DESIGN: only the fixed-width [`FixedSerie`] carriers implement `cast_field` (a same-dtype metadata
// reshape). A byte / nested / null column keeps the nullability + values the conversion already
// produced; a metadata-only reshape of those carriers is out of `convert_column`'s scope, so they
// pass through unchanged (name it at build time with the carrier's `with_name`).
fn apply_field(col: Column, field: Option<&ColumnField>) -> Result<Column, IoError> {
    let Some(ColumnField::Leaf(hf)) = field else {
        return Ok(col);
    };
    macro_rules! reshape {
        ($s:expr) => {{
            let mut carrier = $s;
            carrier.cast_field_in_place(hf)?;
            carrier.into()
        }};
    }
    macro_rules! reshape_decimal {
        ($s:expr) => {{
            let mut carrier = $s;
            carrier.cast_field_in_place(hf)?;
            let carrier = match (hf.precision(), hf.scale()) {
                (Some(precision), Some(scale)) => carrier.with_precision_scale(precision, scale),
                _ => carrier,
            };
            carrier.into()
        }};
    }
    Ok(match col {
        Column::Int8(s) => reshape!(s),
        Column::UInt8(s) => reshape!(s),
        Column::Int16(s) => reshape!(s),
        Column::UInt16(s) => reshape!(s),
        Column::Int32(s) => reshape!(s),
        Column::UInt32(s) => reshape!(s),
        Column::Int64(s) => reshape!(s),
        Column::UInt64(s) => reshape!(s),
        Column::Int128(s) => reshape!(s),
        Column::UInt128(s) => reshape!(s),
        Column::Float16(s) => reshape!(s),
        Column::Float32(s) => reshape!(s),
        Column::Float64(s) => reshape!(s),
        Column::Bool(s) => reshape!(s),
        Column::Decimal8(s) => reshape_decimal!(s),
        Column::Decimal16(s) => reshape_decimal!(s),
        Column::Decimal32(s) => reshape_decimal!(s),
        Column::Decimal64(s) => reshape_decimal!(s),
        Column::Decimal128(s) => reshape_decimal!(s),
        Column::Decimal256(s) => reshape_decimal!(s),
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::physical_dtype;
    use crate::datatype_id::DataTypeId;
    use crate::typed::fixedbyte::{Decimal128, Decimal256, Decimal32, Decimal64, FixedUtf8};
    use crate::typed::varbyte::{LargeUtf8, Utf8};
    use crate::typed::LogicalType;

    /// The runtime [`physical_dtype`] must agree with every [`LogicalType`] impl, arm for arm — the
    /// two are the same mapping expressed for the id-dispatch and the compile-time paths.
    #[test]
    fn runtime_physical_matches_logical_type() {
        assert_eq!(
            physical_dtype(Decimal32::LOGICAL_ID),
            Decimal32::physical_dtype()
        );
        assert_eq!(
            physical_dtype(Decimal64::LOGICAL_ID),
            Decimal64::physical_dtype()
        );
        assert_eq!(
            physical_dtype(Decimal128::LOGICAL_ID),
            Decimal128::physical_dtype()
        );
        assert_eq!(
            physical_dtype(Decimal256::LOGICAL_ID),
            Decimal256::physical_dtype()
        );
        assert_eq!(physical_dtype(Utf8::LOGICAL_ID), Utf8::physical_dtype());
        assert_eq!(
            physical_dtype(LargeUtf8::LOGICAL_ID),
            LargeUtf8::physical_dtype()
        );
        assert_eq!(
            physical_dtype(FixedUtf8::LOGICAL_ID),
            FixedUtf8::physical_dtype()
        );
        // A plain numeric is its own physical.
        assert_eq!(physical_dtype(DataTypeId::I64), DataTypeId::I64);
    }
}
