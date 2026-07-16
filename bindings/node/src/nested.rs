//! The `yggdryl.types` namespace's **nested (composite) layer** — `StructField` (the centralized
//! struct schema) and `StructSerie` (a nullable struct column of heterogeneous child columns),
//! mirroring `yggdryl_core::io::nested`.
//!
//! A `StructField` is a value type (with `equals` / `hashCode` and a byte codec) describing an
//! ordered, named set of child fields (each a `Field` or a nested `StructField`). A `StructSerie`
//! is a struct column whose children are the crate's existing `Serie` columns, erased through the
//! core's `AnySerie`. Because napi cannot accept an arbitrary one-of-many class instance, a
//! `StructSerie` is assembled from a `StructField` **schema** plus each child's canonical
//! `serializeBytes()` frame — the same cross-language wire form used everywhere — so it round-trips
//! byte-for-byte with the Rust core and the Python extension.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::{Buffer, Either, Either4, Null};
use napi::{Env, JsUnknown, ValueType};
use napi_derive::napi;

use yggdryl_core::io::fixed::{f16, Field as CoreField, NativeType, Serie, I256, I96, U256, U96};
use yggdryl_core::io::nested::{
    ListField as CoreListField, ListSerie as CoreListSerie, MapField as CoreMapField,
    MapSerie as CoreMapSerie, StructField as CoreStructField, StructSerie as CoreStructSerie,
};
use yggdryl_core::io::var::{Binary, ByteSerie, Utf8};
use yggdryl_core::io::{
    read_any_column, AnyField, AnyScalar, AnySerie, Bytes, DataTypeId, FieldType, NodePath,
    PathSegment,
};

use crate::types::{DataType, Field};
use crate::values::{
    fixed_js_to_le_bytes, fixed_leaf_to_js, from_unknown, js_int_value, js_u128_value, to_unknown,
};
use crate::varvalues::{var_js_to_bytes, var_leaf_to_js};

/// Names a (self-describing) erased column in place — the one-line replacement for the removed
/// `NamedSerie` carrier (the name goes straight into the column's own header).
fn named_column(mut column: Box<dyn AnySerie>, name: &str) -> Box<dyn AnySerie> {
    column.set_name(name);
    column
}

/// Maps any core error to a thrown JS `Error` (its guided text passes through unchanged).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// A Java-style `i32` content hash, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// Any Node field class (leaf `Field`, nested `StructField` / `ListField` / `MapField`) → an erased
/// [`AnyField`]. A nested child (a struct field, a list item, a map key/value) can itself be any of
/// these, so every nested schema constructor takes this four-way union.
fn to_any_field(field: Either4<&Field, &StructField, &ListField, &MapField>) -> AnyField {
    match field {
        Either4::A(leaf) => AnyField::leaf(leaf.inner.clone()),
        Either4::B(nested) => nested.inner.as_any_field().clone(),
        Either4::C(list) => list.inner.as_any_field().clone(),
        Either4::D(map) => map.inner.as_any_field().clone(),
    }
}

/// An erased [`AnyField`] → its concrete `Field` / `StructField` / `ListField` / `MapField`.
fn from_any_field(field: &AnyField) -> Either4<Field, StructField, ListField, MapField> {
    if field.is_struct() {
        Either4::B(StructField {
            inner: CoreStructField::from_any_field(field.clone())
                .expect("a struct AnyField rebuilds a StructField"),
        })
    } else if field.is_list() {
        Either4::C(ListField {
            inner: CoreListField::from_any_field(field.clone())
                .expect("a list AnyField rebuilds a ListField"),
        })
    } else if field.is_map() {
        Either4::D(MapField {
            inner: CoreMapField::from_any_field(field.clone())
                .expect("a map AnyField rebuilds a MapField"),
        })
    } else {
        Either4::A(Field {
            inner: field
                .as_leaf()
                .expect("a non-nested AnyField is a leaf")
                .clone(),
        })
    }
}

/// Reconstructs one erased child column from its schema `field` and its canonical
/// [`serializeBytes`] frame, via the core's central recursive dispatch — a leaf, struct, list, or
/// map child all round-trip through the same call. This is the byte hand-off napi uses in place of
/// passing a heterogeneous child column instance across the boundary.
fn read_child(field: &AnyField, bytes: &[u8]) -> napi::Result<Box<dyn AnySerie>> {
    read_any_column(field, &mut Bytes::from_slice(bytes)).map_err(to_error)
}

/// An erased column, borrowed from a wrapper's `inner`, for the `dyn AnySerie` deep-navigation
/// surface (`getAt` / `getPath` / `childAt` …). The concrete nested `Serie` types all implement
/// [`AnySerie`], so this coercion is the one-line adapter every deep method routes through.
macro_rules! erased {
    ($self:expr) => {
        &$self.inner as &dyn AnySerie
    };
    (mut $self:expr) => {
        &mut $self.inner as &mut dyn AnySerie
    };
}

// =====================================================================================
// The erased AnyScalar <-> native JS bridge + the shared deep-navigation helpers. Every nested
// column's deep get/set is a 1–3 line delegate to these, mirroring the Python binding
// capability-for-capability (Node has no dunders, so each is a named camelCase method).
// =====================================================================================

/// Whether a JS value is `null` / `undefined` (a null cell / element).
fn is_nullish(value: &JsUnknown) -> napi::Result<bool> {
    Ok(matches!(
        value.get_type()?,
        ValueType::Null | ValueType::Undefined
    ))
}

/// **The keystone read bridge** — an erased [`AnyScalar`] cell as its native JS value. A null is JS
/// `null`; a **list** / **map** cell returns its inner sub-column's `serializeBytes()` `Buffer` (the
/// caller deserializes with the matching class — the Node mirror of Python's live sub-`Serie`); a
/// whole struct row has no native scalar view here (guided error); a **leaf** decodes by
/// [`DataTypeId`] via [`leaf_bytes_to_js`].
fn any_scalar_to_js(env: Env, scalar: &AnyScalar) -> napi::Result<JsUnknown> {
    if scalar.is_null() {
        return to_unknown(env, Null);
    }
    if let Some(items) = scalar.as_list() {
        return to_unknown(env, Buffer::from(items.serialize_bytes()));
    }
    if let Some((entries, _)) = scalar.as_map() {
        return to_unknown(env, Buffer::from(entries.serialize_bytes()));
    }
    if scalar.as_struct().is_some() {
        return Err(to_error(
            "reading a whole struct row as a native value is not supported here; read the row with \
             get(row), or index a leaf cell (e.g. getAt([field_index, row]))",
        ));
    }
    let type_id = scalar.type_id().expect("a non-null scalar reports a type");
    let bytes = scalar.bytes().expect("a leaf carries canonical bytes");
    leaf_bytes_to_js(env, type_id, bytes)
}

/// Decodes a leaf cell's canonical little-endian `bytes` of type `type_id` to its native JS value,
/// **reusing** the per-type marshaling the leaf `Scalar` / `Serie` wrappers already expose. A type
/// with no native cross-language form here (decimal / temporal / fixed-size) is a guided error naming
/// the column-access fallback — same as the Python binding.
fn leaf_bytes_to_js(env: Env, type_id: DataTypeId, bytes: &[u8]) -> napi::Result<JsUnknown> {
    if let Some(js) = fixed_leaf_to_js(env, type_id, bytes)? {
        return Ok(js);
    }
    if let Some(js) = var_leaf_to_js(env, type_id, bytes)? {
        return Ok(js);
    }
    Err(to_error(format!(
        "reading a {} cell as a native value is not supported through deep indexing; read the \
         column with getColumn(path) and index its concrete Serie",
        type_id.name()
    )))
}

/// **The keystone cast** — a JS value into an erased leaf [`AnyScalar`] of type `target` (width
/// `width`), **reusing** the leaf wrappers' own marshaling. A type with no native input form here is
/// a guided error; the core `set_*` then re-validates. (A JS `null` never reaches here — the caller
/// builds the null cell.)
fn js_to_any_scalar(
    env: Env,
    value: &JsUnknown,
    target: DataTypeId,
    width: usize,
) -> napi::Result<AnyScalar> {
    if let Some(bytes) = fixed_js_to_le_bytes(env, value, target)? {
        return Ok(AnyScalar::leaf(
            CoreField::of("", target, width, false),
            bytes,
        ));
    }
    if let Some(bytes) = var_js_to_bytes(env, value, target)? {
        return Ok(AnyScalar::leaf(
            CoreField::of("", target, width, false),
            bytes,
        ));
    }
    Err(to_error(format!(
        "setting a {} cell through deep indexing is not supported; set the column's concrete Serie \
         cell directly (a decimal / temporal / fixed-size cell has no native scalar form here)",
        target.name()
    )))
}

/// The `(type_id, byte_width)` of the leaf **column** addressed by the cell path `cell_path` (its
/// parent), so a value set into a cell — **even a currently-null one** — casts to the leaf's actual
/// type. Reuses the core's `get_by_path` navigation on the parent path (matching the Python binding).
fn cell_target_type(
    root: &(dyn AnySerie + 'static),
    cell_path: &NodePath,
) -> napi::Result<(DataTypeId, usize)> {
    let column = match cell_path.parent() {
        Some(parent) => root.get_by_path(&parent.to_string()).map_err(to_error)?,
        None => root,
    };
    let id = column.type_id();
    Ok((id, id.fixed_byte_width().unwrap_or(0)))
}

/// Builds the erased cell value to write at `cell_path`: a JS `null` → a null cell; otherwise the
/// value cast to the addressed leaf column's type.
fn build_cell_scalar(
    env: Env,
    root: &(dyn AnySerie + 'static),
    cell_path: &NodePath,
    value: &JsUnknown,
) -> napi::Result<AnyScalar> {
    if is_nullish(value)? {
        return Ok(AnyScalar::null());
    }
    let (target, width) = cell_target_type(root, cell_path)?;
    js_to_any_scalar(env, value, target, width)
}

/// Coerces JS coordinate numbers to the `usize` slice the core deep-navigation takes, **validating**
/// each is a finite, non-negative, whole number within `usize` — else a guided error (the Node mirror
/// of Python's `extract_coords` message). Accepting `Vec<f64>` (still `number[]` in TS — no JS-visible
/// API change) is what lets us reject a negative / fractional / oversized coordinate instead of
/// silently `ToUint32`-wrapping it into a confusing error or a wrong cell.
fn coords_usize(coords: Vec<f64>) -> napi::Result<Vec<usize>> {
    coords
        .into_iter()
        .map(|coord| {
            if !coord.is_finite()
                || coord.fract() != 0.0
                || coord < 0.0
                || coord > usize::MAX as f64
            {
                return Err(to_error("nested coordinates must be non-negative integers"));
            }
            Ok(coord as usize)
        })
        .collect()
}

/// The deep cell at `coords` as a native JS value (`get_at` → the bridge).
fn deep_cell_by_coords(
    env: Env,
    root: &(dyn AnySerie + 'static),
    coords: &[usize],
) -> napi::Result<JsUnknown> {
    let scalar = root.get_at(coords).map_err(to_error)?;
    any_scalar_to_js(env, &scalar)
}

/// The deep cell at an index-terminal `path` as a native JS value (`get_scalar_by_path` → the
/// bridge). A name-terminal path (a column, not a cell) surfaces the core's guided error.
fn cell_by_path(env: Env, root: &(dyn AnySerie + 'static), path: &str) -> napi::Result<JsUnknown> {
    let scalar = root.get_scalar_by_path(path).map_err(to_error)?;
    any_scalar_to_js(env, &scalar)
}

/// A str key: an index-terminal path reads a **cell** (native), a name-terminal path reads a
/// **sub-column** (its `serializeBytes()` frame — the Node mirror of Python's live `Serie` wrapper).
fn cell_or_column_by_path(
    env: Env,
    root: &(dyn AnySerie + 'static),
    path: &str,
) -> napi::Result<JsUnknown> {
    let parsed = NodePath::parse(path).map_err(to_error)?;
    match parsed.segments().last() {
        Some(PathSegment::Index(_)) => cell_by_path(env, root, path),
        _ => to_unknown(
            env,
            Buffer::from(root.get_by_path(path).map_err(to_error)?.serialize_bytes()),
        ),
    }
}

/// Sets the deep cell at `coords` to `value` (a JS `null` writes a null) — the leaf type is read from
/// the addressed **column** (via `get_by_path` on the parent path), so a value writes even into a
/// currently-null cell.
fn set_cell_by_coords(
    env: Env,
    root: &mut (dyn AnySerie + 'static),
    coords: &[usize],
    value: &JsUnknown,
) -> napi::Result<()> {
    let cell_path = NodePath::from_segments(
        coords
            .iter()
            .map(|&index| PathSegment::Index(index))
            .collect(),
    );
    let scalar = build_cell_scalar(env, &*root, &cell_path, value)?;
    root.set_at(coords, &scalar).map_err(to_error)
}

/// Sets the deep cell at an index-terminal `path` to `value` (a JS `null` writes a null). A
/// name-terminal path addresses a whole column, which has no in-place assignment (guided error).
fn set_cell_by_path(
    env: Env,
    root: &mut (dyn AnySerie + 'static),
    path: &str,
    value: &JsUnknown,
) -> napi::Result<()> {
    let cell_path = NodePath::parse(path).map_err(to_error)?;
    if !matches!(cell_path.segments().last(), Some(PathSegment::Index(_))) {
        return Err(to_error(
            "a str cell assignment must address a leaf cell (an index-terminal path like \"a[1]\"); \
             a name-terminal path addresses a whole column, which has no in-place assignment",
        ));
    }
    let scalar = build_cell_scalar(env, &*root, &cell_path, value)?;
    root.set_by_path(path, &scalar).map_err(to_error)
}

/// `getCell(key)` — a cell key (a coords `number[]` or an index-terminal str path) to a native value
/// only.
fn get_cell_by_key(
    env: Env,
    root: &(dyn AnySerie + 'static),
    key: Either<Vec<f64>, String>,
) -> napi::Result<JsUnknown> {
    match key {
        Either::A(coords) => deep_cell_by_coords(env, root, &coords_usize(coords)?),
        Either::B(path) => cell_by_path(env, root, &path),
    }
}

/// `setCell(key, value)` — a cell key (a coords `number[]` or an index-terminal str path) set to a
/// value (a JS `null` writes a null).
fn set_cell_by_key(
    env: Env,
    root: &mut (dyn AnySerie + 'static),
    key: Either<Vec<f64>, String>,
    value: &JsUnknown,
) -> napi::Result<()> {
    match key {
        Either::A(coords) => set_cell_by_coords(env, root, &coords_usize(coords)?, value),
        Either::B(path) => set_cell_by_path(env, root, &path, value),
    }
}

/// The positional child column at `index` as its `serializeBytes()` frame, or `null` (a leaf has no
/// children) — the shared `childAt`.
fn child_at_bytes(column: &dyn AnySerie, index: u32) -> Option<Buffer> {
    column
        .child_serie_at(index as usize)
        .map(|child| child.serialize_bytes().into())
}

/// The named child column as its `serializeBytes()` frame, or `null` — the shared `childNamed`.
fn child_named_bytes(column: &dyn AnySerie, name: String) -> Option<Buffer> {
    column
        .child_serie_by(&name)
        .map(|child| child.serialize_bytes().into())
}

// =====================================================================================
// Generic inference factory — `yggdryl.types.column(values, dtype?)`. A thin inference over the
// existing typed leaf columns, mirroring the Python `yggdryl.types.column` table exactly. Because
// napi cannot return a heterogeneous one-of-many wrapper, it returns the built column's canonical
// `serializeBytes()` frame; the companion `columnType(values, dtype?)` returns the inferred
// `DataType` so the caller picks the matching `Serie.deserializeBytes` class.
// =====================================================================================

/// The leaf families a JS array may contain, tallied while scanning.
///
/// DESIGN: unlike the Python binding there is no `int_overflow` state — JS has one numeric type, so a
/// whole-valued `number` beyond the `i128` range is classified as a **float** (`saw_float`), not an
/// integer overflow error (Python has a real `int` and errors). See [`infer_column_id`].
#[derive(Default)]
struct Inferred {
    saw_int: bool,
    saw_float: bool,
    saw_str: bool,
    saw_bytes: bool,
    min: i128,
    max: i128,
}

impl Inferred {
    /// Folds one integer value into the running `[min, max]` (and marks an int was seen).
    fn record_int(&mut self, value: i128) {
        if self.saw_int {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        } else {
            self.min = value;
            self.max = value;
        }
        self.saw_int = true;
    }
}

/// The smallest **signed** integer type that holds `[min, max]` (widening to `i128` at the top) —
/// identical to the Python binding.
fn sized_signed_int(min: i128, max: i128) -> DataTypeId {
    if min >= i8::MIN as i128 && max <= i8::MAX as i128 {
        DataTypeId::I8
    } else if min >= i16::MIN as i128 && max <= i16::MAX as i128 {
        DataTypeId::I16
    } else if min >= i32::MIN as i128 && max <= i32::MAX as i128 {
        DataTypeId::I32
    } else if min >= i64::MIN as i128 && max <= i64::MAX as i128 {
        DataTypeId::I64
    } else {
        DataTypeId::I128
    }
}

/// Scans `values` and infers one leaf column [`DataTypeId`]: all-int → the smallest signed int that
/// holds them (`i64` when empty / all-null); any float → `f64`; str → `utf8`; bytes → `binary`; a
/// JS `boolean` counts as the integer `0` / `1`; a `null` / `undefined` is a nullable slot. A mix
/// that shares no leaf type is a guided error naming the offending families.
fn infer_column_id(env: Env, values: &[JsUnknown]) -> napi::Result<DataTypeId> {
    let mut info = Inferred::default();
    for value in values {
        match value.get_type()? {
            ValueType::Null | ValueType::Undefined => continue,
            // A JS boolean counts as the integer 0 / 1 (mirroring Python's `bool` is-an-`int`).
            ValueType::Boolean => {
                info.record_int(if from_unknown::<bool>(env, value)? { 1 } else { 0 })
            }
            ValueType::Number => {
                let number: f64 = from_unknown(env, value)?;
                if !number.is_finite() || number.fract() != 0.0 {
                    // A fractional / non-finite Number is a float.
                    info.saw_float = true;
                } else if number >= i128::MIN as f64 && number < 2f64.powi(127) {
                    // A whole Number in the *strict* `i128` range is an integer. The top guard is
                    // `< 2^127` (not `<= i128::MAX as f64`, which rounds UP to `2^127`) so the
                    // saturating `as i128` can never silently clamp `2^127` to `i128::MAX`.
                    info.record_int(number as i128);
                } else {
                    // DESIGN: a whole-valued Number beyond the `i128` range has no integer type here;
                    // JS has one numeric type, so classify it as a float — `column([1e40])` and
                    // `column([2 ** 127])` infer `f64` (like Python's large float), never a silent
                    // integer clamp or an overflow error.
                    info.saw_float = true;
                }
            }
            ValueType::String => info.saw_str = true,
            ValueType::Object if from_unknown::<Buffer>(env, value).is_ok() => info.saw_bytes = true,
            _ => {
                return Err(to_error(
                    "column() cannot infer a type from this value; the supported element types are \
                     number, string, Buffer, and null — pass an explicit dtype (a DataType or a name \
                     like \"i64\") for anything else",
                ))
            }
        }
    }
    resolve_inferred(&info)
}

/// Resolves the tallied families to one column type, or a guided ambiguity error (mirroring Python).
fn resolve_inferred(info: &Inferred) -> napi::Result<DataTypeId> {
    let numeric = info.saw_int || info.saw_float;
    match (numeric, info.saw_str, info.saw_bytes) {
        // All-null or empty: default to i64 (holds any small integer; nullable via the null slots).
        (false, false, false) => Ok(DataTypeId::I64),
        (true, false, false) => {
            if info.saw_float {
                Ok(DataTypeId::F64)
            } else {
                Ok(sized_signed_int(info.min, info.max))
            }
        }
        (false, true, false) => Ok(DataTypeId::Utf8),
        (false, false, true) => Ok(DataTypeId::Binary),
        _ => {
            let mut families = Vec::new();
            if info.saw_int {
                families.push("int");
            }
            if info.saw_float {
                families.push("float");
            }
            if info.saw_str {
                families.push("str");
            }
            if info.saw_bytes {
                families.push("bytes");
            }
            Err(to_error(format!(
                "column() cannot infer a single column type for a mix of {} values; these do not \
                 share a leaf type — pass an explicit dtype (a DataType or a name like \"utf8\") to \
                 disambiguate",
                families.join(", ")
            )))
        }
    }
}

/// Resolves an explicit `dtype` (a [`DataType`] object or a type-name string) to a [`DataTypeId`].
fn resolve_dtype_id(dtype: Either<&DataType, String>) -> napi::Result<DataTypeId> {
    match dtype {
        Either::A(data_type) => Ok(data_type.type_id()),
        Either::B(name) => DataTypeId::from_name(&name)
            .ok_or_else(|| to_error(format!("unknown data type name: {name:?}"))),
    }
}

/// A JS value as a float, or `None` for null — a `number` or a `boolean` (0.0 / 1.0).
fn float_value(env: Env, value: &JsUnknown) -> napi::Result<Option<f64>> {
    Ok(match value.get_type()? {
        ValueType::Null | ValueType::Undefined => None,
        ValueType::Number => Some(from_unknown(env, value)?),
        ValueType::Boolean => Some(if from_unknown::<bool>(env, value)? {
            1.0
        } else {
            0.0
        }),
        other => {
            return Err(to_error(format!(
                "expected a number for a float column, got a {other:?} value"
            )))
        }
    })
}

/// The guided error for a `column(dtype=…)` type that has no plain-array builder (decimal / temporal
/// / fixed-size need extra parameters, so they use their typed constructor) — mirroring Python.
fn unbuildable_column_error(id: DataTypeId) -> napi::Error {
    to_error(format!(
        "column() cannot build a {} column from a plain array; construct its Serie directly (a \
         decimal / temporal / fixed-size column needs extra parameters like precision, scale, a \
         unit, or a width)",
        id.name()
    ))
}

/// Builds the canonical `serializeBytes()` frame of a leaf column of type `id` from the JS `values`,
/// mirroring Python's `build_column` type set (`u8`…`i256`, `f16`…`f64`, `utf8`, `binary`). A type
/// whose column needs extra parameters (decimal / temporal / fixed-size) is a guided error.
fn build_column_frame(env: Env, id: DataTypeId, values: &[JsUnknown]) -> napi::Result<Vec<u8>> {
    // Signed / unsigned integers that fit through `i128` (range-checked per element).
    macro_rules! build_int {
        ($t:ty) => {{
            let mut options: Vec<Option<$t>> = Vec::with_capacity(values.len());
            for value in values {
                match js_int_value(env, value)? {
                    None => options.push(None),
                    Some(number) => options.push(Some(<$t>::try_from(number).map_err(|_| {
                        to_error(format!(
                            "the value {number} is out of range for a {} column",
                            id.name()
                        ))
                    })?)),
                }
            }
            Serie::<$t>::from_options(&options).serialize_bytes()
        }};
    }
    // `u128` needs its own path: its `[0, u128::MAX]` range exceeds `i128`, which the `build_int!`
    // coefficient (an `i128`) cannot represent — so a value above `i128::MAX` (e.g. `"2e38"`) must
    // parse/validate as a `u128`, not fail. Mirrors the deep-set `u128` arm (a decimal string).
    macro_rules! build_u128 {
        () => {{
            let mut options: Vec<Option<u128>> = Vec::with_capacity(values.len());
            for value in values {
                options.push(js_u128_value(env, value)?);
            }
            Serie::<u128>::from_options(&options).serialize_bytes()
        }};
    }
    // Wide integers (96 / 256-bit) with no numeric JS form — a `Buffer` of little-endian bytes.
    macro_rules! build_wide {
        ($t:ty) => {{
            let mut options: Vec<Option<$t>> = Vec::with_capacity(values.len());
            for value in values {
                if is_nullish(value)? {
                    options.push(None);
                } else {
                    let bytes = fixed_js_to_le_bytes(env, value, id)?.expect("a wide fixed id");
                    options.push(Some(<$t as NativeType>::read_le(&bytes)));
                }
            }
            Serie::<$t>::from_options(&options).serialize_bytes()
        }};
    }
    macro_rules! build_float {
        ($t:ty, $conv:expr) => {{
            let mut options: Vec<Option<$t>> = Vec::with_capacity(values.len());
            for value in values {
                options.push(float_value(env, value)?.map($conv));
            }
            Serie::<$t>::from_options(&options).serialize_bytes()
        }};
    }
    macro_rules! build_utf8 {
        () => {{
            let mut owned: Vec<Option<Vec<u8>>> = Vec::with_capacity(values.len());
            for value in values {
                if is_nullish(value)? {
                    owned.push(None);
                } else {
                    owned.push(Some(from_unknown::<String>(env, value)?.into_bytes()));
                }
            }
            let refs: Vec<Option<&[u8]>> = owned.iter().map(|slot| slot.as_deref()).collect();
            ByteSerie::<Utf8>::from_options(&refs)
                .map_err(to_error)?
                .serialize_bytes()
        }};
    }
    macro_rules! build_binary {
        () => {{
            let mut owned: Vec<Option<Vec<u8>>> = Vec::with_capacity(values.len());
            for value in values {
                if is_nullish(value)? {
                    owned.push(None);
                } else {
                    owned.push(Some(from_unknown::<Buffer>(env, value)?.to_vec()));
                }
            }
            let refs: Vec<Option<&[u8]>> = owned.iter().map(|slot| slot.as_deref()).collect();
            ByteSerie::<Binary>::from_options(&refs)
                .map_err(to_error)?
                .serialize_bytes()
        }};
    }
    Ok(match id {
        DataTypeId::U8 => build_int!(u8),
        DataTypeId::U16 => build_int!(u16),
        DataTypeId::U32 => build_int!(u32),
        DataTypeId::U64 => build_int!(u64),
        DataTypeId::U128 => build_u128!(),
        DataTypeId::I8 => build_int!(i8),
        DataTypeId::I16 => build_int!(i16),
        DataTypeId::I32 => build_int!(i32),
        DataTypeId::I64 => build_int!(i64),
        DataTypeId::I128 => build_int!(i128),
        DataTypeId::U96 => build_wide!(U96),
        DataTypeId::U256 => build_wide!(U256),
        DataTypeId::I96 => build_wide!(I96),
        DataTypeId::I256 => build_wide!(I256),
        DataTypeId::F16 => build_float!(f16, |value| f16::from_f32(value as f32)),
        DataTypeId::F32 => build_float!(f32, |value| value as f32),
        DataTypeId::F64 => build_float!(f64, |value| value),
        DataTypeId::Utf8 => build_utf8!(),
        DataTypeId::Binary => build_binary!(),
        other => return Err(unbuildable_column_error(other)),
    })
}

/// The [`DataTypeId`] a `column(values, dtype?)` call resolves to — an explicit `dtype`, else the
/// inference over `values`.
fn resolved_column_id(
    env: Env,
    values: &[JsUnknown],
    dtype: Option<Either<&DataType, String>>,
) -> napi::Result<DataTypeId> {
    match dtype {
        Some(dtype) => resolve_dtype_id(dtype),
        None => infer_column_id(env, values),
    }
}

/// The generic inference factory — infers a leaf column type from a JS array (or uses an explicit
/// `dtype`), builds the matching column, and returns its canonical `serializeBytes()` frame.
/// Reconstruct the column with the matching class (its type is [`columnType`](column_type)).
///
/// Inference table (identical to the Python `yggdryl.types.column`):
///
/// | array contents (non-null) | inferred column |
/// | --- | --- |
/// | all integers (a `boolean` counts as `0` / `1`) | the **smallest signed int** over `[min, max]` (`i8`…`i128`) |
/// | empty or all-null | `i64` |
/// | any fractional `number` | `f64` |
/// | all `string`s | `utf8` |
/// | all `Buffer`s | `binary` |
///
/// A `null` / `undefined` is a nullable slot (type unaffected). An unshared mix (int + str, str +
/// bytes, …) is a guided error. An explicit `dtype` (a `DataType` or a name string) uses that type's
/// builder; a decimal / temporal / fixed-size type is a guided error (it needs extra parameters — use
/// its typed constructor).
///
/// DESIGN: JavaScript has no separate integer type, so a whole-valued `number` **within the `i128`
/// range** (`1`, `2.0`) is an **integer** here (Python's `1.0` would infer `f64`); a fractional
/// `number`, **and a whole-valued `number` beyond the `i128` range** (`1e40`, `2 ** 127`), infer
/// `f64` — matching Python's large-`float` classification and never a silent integer clamp or an
/// overflow error. An explicit integer `dtype` still rejects an out-of-range value with a guided
/// error (matching Python's out-of-range `int`).
#[napi(namespace = "types")]
pub fn column(
    env: Env,
    values: Vec<JsUnknown>,
    dtype: Option<Either<&DataType, String>>,
) -> napi::Result<Buffer> {
    let id = resolved_column_id(env, &values, dtype)?;
    Ok(build_column_frame(env, id, &values)?.into())
}

/// The [`DataType`] that [`column`](column) infers for `values` (or the explicit `dtype`) — the
/// companion that names which `Serie` class to reconstruct the frame with.
#[napi(namespace = "types")]
pub fn column_type(
    env: Env,
    values: Vec<JsUnknown>,
    dtype: Option<Either<&DataType, String>>,
) -> napi::Result<DataType> {
    Ok(DataType::of(resolved_column_id(env, &values, dtype)?))
}

/// The **centralized struct schema** — a name, nullability, metadata, and an ordered list of child
/// fields (each a `Field` or nested `StructField`).
#[napi(namespace = "types")]
pub struct StructField {
    pub(crate) inner: CoreStructField,
}

#[napi(namespace = "types")]
impl StructField {
    /// A struct schema from a name, its ordered child fields, and its nullability (default `true`).
    #[napi(constructor)]
    pub fn new(
        name: String,
        fields: Vec<Either4<&Field, &StructField, &ListField, &MapField>>,
        nullable: Option<bool>,
    ) -> Self {
        let children = fields.into_iter().map(to_any_field).collect();
        Self {
            inner: CoreStructField::new(&name, children, nullable.unwrap_or(true)),
        }
    }

    /// The struct's name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Whether the struct column admits nulls.
    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"struct"`).
    #[napi(getter)]
    pub fn type_name(&self) -> &'static str {
        "struct"
    }

    /// This schema's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Struct)
    }

    /// The number of child fields.
    #[napi(getter)]
    pub fn num_fields(&self) -> u32 {
        self.inner.num_fields() as u32
    }

    /// The child field at `index` as a `Field` / `StructField` / `ListField` / `MapField`; throws
    /// out of range.
    #[napi]
    pub fn field(
        &self,
        index: u32,
    ) -> napi::Result<Either4<Field, StructField, ListField, MapField>> {
        self.inner
            .field(index as usize)
            .map(from_any_field)
            .ok_or_else(|| to_error("StructField index out of range"))
    }

    /// The child field named `name`, or `null`.
    #[napi]
    pub fn field_named(
        &self,
        name: String,
    ) -> Option<Either4<Field, StructField, ListField, MapField>> {
        self.inner.field_named(&name).map(from_any_field)
    }

    /// The 0-based index of the child field named `name`, or `null`.
    #[napi]
    pub fn index_of(&self, name: String) -> Option<u32> {
        self.inner.index_of(&name).map(|index| index as u32)
    }

    /// The child fields, in order, as `Field` / `StructField` / `ListField` / `MapField`.
    #[napi]
    pub fn fields(&self) -> Vec<Either4<Field, StructField, ListField, MapField>> {
        self.inner.fields().iter().map(from_any_field).collect()
    }

    /// A fresh schema renamed to `name`.
    #[napi]
    pub fn with_name(&self, name: String) -> Self {
        Self {
            inner: self.inner.with_name(&name),
        }
    }

    /// A fresh schema with `nullable` set.
    #[napi]
    pub fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh schema with one more child field appended.
    #[napi]
    pub fn with_field(&self, field: Either4<&Field, &StructField, &ListField, &MapField>) -> Self {
        Self {
            inner: self.inner.with_field(to_any_field(field)),
        }
    }

    /// A fresh schema with one extra `key = value` metadata entry.
    #[napi]
    pub fn with_metadata_entry(&self, key: String, value: String) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(&key, &value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.as_any_field().serialize_bytes().into()
    }

    /// Reconstructs a schema from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        let field = AnyField::deserialize_bytes(&bytes).map_err(to_error)?;
        CoreStructField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| to_error("the bytes did not decode to a struct field"))
    }

    /// Value equality (content, metadata included).
    #[napi]
    pub fn equals(&self, other: &StructField) -> bool {
        self.inner == other.inner
    }

    /// A content hash (equal schemas hash equal).
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "StructField(name={:?}, numFields={}, nullable={})",
            self.inner.name(),
            self.inner.num_fields(),
            self.inner.nullable()
        )
    }
}

/// A **nullable struct column** — one child column per field (all the same length), an ordered
/// schema, and an optional top-level validity mask (a null struct row).
#[napi(namespace = "types")]
pub struct StructSerie {
    pub(crate) inner: CoreStructSerie,
}

#[napi(namespace = "types")]
impl StructSerie {
    /// A struct column from a `schema` and each child column's `serializeBytes()` frame, in field
    /// order. (napi cannot accept an arbitrary one-of-many `Serie` instance, so a child crosses as
    /// its canonical bytes — build them with `serie.serializeBytes()` / `serie.toField(name)`.)
    #[napi(factory)]
    pub fn from_columns(schema: &StructField, columns: Vec<Buffer>) -> napi::Result<Self> {
        let fields = schema.inner.fields();
        if fields.len() != columns.len() {
            return Err(to_error(format!(
                "the schema has {} fields but {} column frames were given",
                fields.len(),
                columns.len()
            )));
        }
        let mut cols: Vec<Box<dyn AnySerie>> = Vec::with_capacity(fields.len());
        for (field, bytes) in fields.iter().zip(&columns) {
            cols.push(read_child(field, bytes)?);
        }
        CoreStructSerie::from_columns(fields.to_vec(), cols, None)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// The number of rows.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len() as u32
    }

    /// The number of child columns (fields).
    #[napi(getter)]
    pub fn num_columns(&self) -> u32 {
        self.inner.num_columns() as u32
    }

    /// The number of null struct rows.
    #[napi(getter)]
    pub fn null_count(&self) -> u32 {
        CoreStructSerie::null_count(&self.inner) as u32
    }

    /// Whether any struct row is null.
    #[napi(getter)]
    pub fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// This column's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Struct)
    }

    /// A [`StructField`] naming this struct column (nullability inferred from its null rows).
    #[napi]
    pub fn to_field(&self, name: String) -> StructField {
        StructField {
            inner: self.inner.to_field(&name),
        }
    }

    /// The child field at `index` as a `Field` / `StructField` / `ListField` / `MapField`; throws
    /// out of range.
    #[napi]
    pub fn field(
        &self,
        index: u32,
    ) -> napi::Result<Either4<Field, StructField, ListField, MapField>> {
        self.inner
            .field(index as usize)
            .map(|field| from_any_field(&field))
            .ok_or_else(|| to_error("StructSerie field index out of range"))
    }

    /// The child column at `index` as its canonical bytes — reconstruct it with the matching
    /// `Serie.deserializeBytes(...)` (its type is `field(index).typeName`). Throws out of range.
    #[napi]
    pub fn column_bytes(&self, index: u32) -> napi::Result<Buffer> {
        self.inner
            .column(index as usize)
            .map(|column| column.serialize_bytes().into())
            .ok_or_else(|| to_error("StructSerie column index out of range"))
    }

    /// The child column named `name` as its canonical bytes, or `null`.
    #[napi]
    pub fn column_bytes_named(&self, name: String) -> Option<Buffer> {
        self.inner
            .column_named(&name)
            .map(|column| column.serialize_bytes().into())
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][children]` frame,
    /// identical across Rust / Python / Node.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a struct column from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        CoreStructSerie::deserialize_bytes(&bytes)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Value equality (content, nulls included).
    #[napi]
    pub fn equals(&self, other: &StructSerie) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// A deep leaf cell by positional coordinates `[field, …, cell]` — its native JS value
    /// (`number` / `string` / `Buffer`), `null`, or a nested list / map cell's `serializeBytes()`
    /// frame. A decimal / temporal / fixed-size leaf, or a whole struct row, is a guided error.
    #[napi]
    pub fn get_at(&self, env: Env, coords: Vec<f64>) -> napi::Result<JsUnknown> {
        deep_cell_by_coords(env, erased!(self), &coords_usize(coords)?)
    }

    /// Overwrites the deep leaf cell at `coords` with `value` (`null` clears it) — the target leaf
    /// type is read from the addressed column, so a value writes even into a currently-null cell.
    #[napi]
    pub fn set_at(&mut self, env: Env, coords: Vec<f64>, value: JsUnknown) -> napi::Result<()> {
        set_cell_by_coords(env, erased!(mut self), &coords_usize(coords)?, &value)
    }

    /// A deep cell by an index-terminal path (e.g. `a[1]`) as its native value, or a sub-column by a
    /// name-terminal path (e.g. `a.b`) as its `serializeBytes()` frame.
    #[napi]
    pub fn get_path(&self, env: Env, path: String) -> napi::Result<JsUnknown> {
        cell_or_column_by_path(env, erased!(self), &path)
    }

    /// Overwrites the deep leaf cell addressed by an index-terminal `path` (e.g. `a[1]`) with `value`.
    #[napi]
    pub fn set_path(&mut self, env: Env, path: String, value: JsUnknown) -> napi::Result<()> {
        set_cell_by_path(env, erased!(mut self), &path, &value)
    }

    /// The deep cell at `key` (a coords `number[]` or an index-terminal str path) as a native value —
    /// like `getAt` / `getPath` but never a sub-column.
    #[napi]
    pub fn get_cell(&self, env: Env, key: Either<Vec<f64>, String>) -> napi::Result<JsUnknown> {
        get_cell_by_key(env, erased!(self), key)
    }

    /// Sets the deep cell at `key` (a coords `number[]` or an index-terminal str path) to `value`
    /// (a JS `null` writes a null).
    #[napi]
    pub fn set_cell(
        &mut self,
        env: Env,
        key: Either<Vec<f64>, String>,
        value: JsUnknown,
    ) -> napi::Result<()> {
        set_cell_by_key(env, erased!(mut self), key, &value)
    }

    /// The sub-column addressed by `path` as its `serializeBytes()` frame — reconstruct it with the
    /// matching `Serie.deserializeBytes(...)`.
    #[napi]
    pub fn get_column(&self, path: String) -> napi::Result<Buffer> {
        let column = erased!(self).get_by_path(&path).map_err(to_error)?;
        Ok(column.serialize_bytes().into())
    }

    /// The number of child columns (a struct's fields).
    #[napi]
    pub fn num_children(&self) -> u32 {
        erased!(self).num_children() as u32
    }

    /// The positional child column's `serializeBytes()` frame, or `null` out of range.
    #[napi]
    pub fn child_at(&self, index: u32) -> Option<Buffer> {
        child_at_bytes(erased!(self), index)
    }

    /// The named child column's `serializeBytes()` frame, or `null`.
    #[napi]
    pub fn child_named(&self, name: String) -> Option<Buffer> {
        child_named_bytes(erased!(self), name)
    }

    /// The `index`-th struct row as a **one-row struct column** `serializeBytes()` frame —
    /// reconstruct it with `StructSerie.deserializeBytes(...)`. Throws out of range.
    #[napi]
    pub fn get(&self, index: u32) -> napi::Result<Buffer> {
        let column = erased!(self);
        if index as usize >= column.len() {
            return Err(to_error("StructSerie row index out of range"));
        }
        Ok(column.slice(index as usize, 1).serialize_bytes().into())
    }

    /// A fresh sub-column over rows `[start, start + length)` as its `serializeBytes()` frame —
    /// reconstruct it with `StructSerie.deserializeBytes(...)`. The range is clamped to the column
    /// (never throws). The Node named mirror of Python's nested `s[start:stop]` slice.
    #[napi]
    pub fn slice(&self, start: u32, length: u32) -> Buffer {
        erased!(self)
            .slice(start as usize, length as usize)
            .serialize_bytes()
            .into()
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "StructSerie(len={}, numColumns={}, nullCount={})",
            self.inner.len(),
            self.inner.num_columns(),
            CoreStructSerie::null_count(&self.inner)
        )
    }
}

/// The **centralized list schema** — a name, nullability, metadata, and a single element (item)
/// field (a `Field` or a nested `StructField` / `ListField` / `MapField`).
#[napi(namespace = "types")]
pub struct ListField {
    pub(crate) inner: CoreListField,
}

#[napi(namespace = "types")]
impl ListField {
    /// A list schema from a name, its element (item) field, and its nullability (default `true`).
    #[napi(constructor)]
    pub fn new(
        name: String,
        item: Either4<&Field, &StructField, &ListField, &MapField>,
        nullable: Option<bool>,
    ) -> Self {
        Self {
            inner: CoreListField::new(&name, to_any_field(item), nullable.unwrap_or(true)),
        }
    }

    /// The list's name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Whether the list column admits nulls.
    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"list"`).
    #[napi(getter)]
    pub fn type_name(&self) -> &'static str {
        "list"
    }

    /// This schema's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::List)
    }

    /// The element (item) field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[napi(getter)]
    pub fn item(&self) -> Either4<Field, StructField, ListField, MapField> {
        from_any_field(self.inner.item())
    }

    /// A fresh list schema renamed to `name`.
    #[napi]
    pub fn with_name(&self, name: String) -> Self {
        Self {
            inner: self.inner.with_name(&name),
        }
    }

    /// A fresh list schema with `nullable` set.
    #[napi]
    pub fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh list schema with a new element (item) field.
    #[napi]
    pub fn with_item(&self, item: Either4<&Field, &StructField, &ListField, &MapField>) -> Self {
        Self {
            inner: self.inner.with_item(to_any_field(item)),
        }
    }

    /// A fresh list schema with one extra `key = value` metadata entry.
    #[napi]
    pub fn with_metadata_entry(&self, key: String, value: String) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(&key, &value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.as_any_field().serialize_bytes().into()
    }

    /// Reconstructs a schema from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        let field = AnyField::deserialize_bytes(&bytes).map_err(to_error)?;
        CoreListField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| to_error("the bytes did not decode to a list field"))
    }

    /// Value equality (content, metadata included).
    #[napi]
    pub fn equals(&self, other: &ListField) -> bool {
        self.inner == other.inner
    }

    /// A content hash (equal schemas hash equal).
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "ListField(name={:?}, item={}, nullable={})",
            self.inner.name(),
            self.inner.item().type_name(),
            self.inner.nullable()
        )
    }
}

/// A **nullable list column** — `i32` offsets over one flattened child column, plus an optional
/// top-level validity mask (a null list row). Row `i` is the child sub-range
/// `child[offsets[i] .. offsets[i + 1]]`.
#[napi(namespace = "types")]
pub struct ListSerie {
    pub(crate) inner: CoreListSerie,
}

#[napi(namespace = "types")]
impl ListSerie {
    /// A list column from its element (item) `field`, the flattened child column's
    /// `serializeBytes()` frame (`itemBytes`), the row `offsets` (`len + 1` entries into the child),
    /// and an optional per-row **present** mask (`present[i] === false` marks row `i` a null list).
    /// (napi cannot accept an arbitrary one-of-many `Serie` instance, so the child crosses as its
    /// canonical bytes — build them with `serie.serializeBytes()` / `serie.toField(name)`.)
    #[napi(factory)]
    pub fn from_parts(
        item_field: Either4<&Field, &StructField, &ListField, &MapField>,
        item_bytes: Buffer,
        offsets: Vec<i32>,
        present: Option<Vec<bool>>,
    ) -> napi::Result<Self> {
        let item = to_any_field(item_field);
        let column = read_child(&item, &item_bytes)?;
        let items = named_column(column, item.name());
        CoreListSerie::from_values(items, &offsets, present.as_deref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// The number of rows.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len() as u32
    }

    /// The number of null list rows.
    #[napi(getter)]
    pub fn null_count(&self) -> u32 {
        self.inner.null_count() as u32
    }

    /// Whether any list row is null.
    #[napi(getter)]
    pub fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// This column's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::List)
    }

    /// The row offsets (`len + 1` entries into the flattened child).
    #[napi(getter)]
    pub fn offsets(&self) -> Vec<i32> {
        self.inner.offsets().to_vec()
    }

    /// The flattened child column as its canonical bytes — reconstruct it with the matching
    /// `Serie.deserializeBytes(...)` (its schema is `toField(name).item`).
    #[napi]
    pub fn item_bytes(&self) -> Buffer {
        self.inner.values().serialize_bytes().into()
    }

    /// A [`ListField`] naming this list column (nullability inferred from its null rows).
    #[napi]
    pub fn to_field(&self, name: String) -> ListField {
        ListField {
            inner: self.inner.to_field(&name),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][offsets][child]`
    /// frame, identical across Rust / Python / Node.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a list column from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        CoreListSerie::deserialize_bytes(&bytes)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Value equality (content, nulls included).
    #[napi]
    pub fn equals(&self, other: &ListSerie) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// A deep leaf cell by positional coordinates `[0, …, cell]` (a list's item child is child `0`) —
    /// its native JS value, `null`, or a nested list / map cell's `serializeBytes()` frame.
    #[napi]
    pub fn get_at(&self, env: Env, coords: Vec<f64>) -> napi::Result<JsUnknown> {
        deep_cell_by_coords(env, erased!(self), &coords_usize(coords)?)
    }

    /// Overwrites the deep leaf cell at `coords` with `value` (`null` clears it) — the target leaf
    /// type is read from the addressed column, so a value writes even into a currently-null cell.
    #[napi]
    pub fn set_at(&mut self, env: Env, coords: Vec<f64>, value: JsUnknown) -> napi::Result<()> {
        set_cell_by_coords(env, erased!(mut self), &coords_usize(coords)?, &value)
    }

    /// A deep cell by an index-terminal path (e.g. `[0][1]`) as its native value, or a sub-column by a
    /// name-terminal path as its `serializeBytes()` frame.
    #[napi]
    pub fn get_path(&self, env: Env, path: String) -> napi::Result<JsUnknown> {
        cell_or_column_by_path(env, erased!(self), &path)
    }

    /// Overwrites the deep leaf cell addressed by an index-terminal `path` (e.g. `[0][1]`).
    #[napi]
    pub fn set_path(&mut self, env: Env, path: String, value: JsUnknown) -> napi::Result<()> {
        set_cell_by_path(env, erased!(mut self), &path, &value)
    }

    /// The deep cell at `key` (a coords `number[]` or an index-terminal str path) as a native value.
    #[napi]
    pub fn get_cell(&self, env: Env, key: Either<Vec<f64>, String>) -> napi::Result<JsUnknown> {
        get_cell_by_key(env, erased!(self), key)
    }

    /// Sets the deep cell at `key` (a coords `number[]` or an index-terminal str path) to `value`.
    #[napi]
    pub fn set_cell(
        &mut self,
        env: Env,
        key: Either<Vec<f64>, String>,
        value: JsUnknown,
    ) -> napi::Result<()> {
        set_cell_by_key(env, erased!(mut self), key, &value)
    }

    /// The sub-column addressed by `path` as its `serializeBytes()` frame.
    #[napi]
    pub fn get_column(&self, path: String) -> napi::Result<Buffer> {
        let column = erased!(self).get_by_path(&path).map_err(to_error)?;
        Ok(column.serialize_bytes().into())
    }

    /// The number of child columns (a list's single item child).
    #[napi]
    pub fn num_children(&self) -> u32 {
        erased!(self).num_children() as u32
    }

    /// The positional child column's `serializeBytes()` frame, or `null` out of range.
    #[napi]
    pub fn child_at(&self, index: u32) -> Option<Buffer> {
        child_at_bytes(erased!(self), index)
    }

    /// The named child column's `serializeBytes()` frame, or `null`.
    #[napi]
    pub fn child_named(&self, name: String) -> Option<Buffer> {
        child_named_bytes(erased!(self), name)
    }

    /// The `index`-th list row's item sub-column as its `serializeBytes()` frame, or `null` if the
    /// row is null — reconstruct it with the item type's `Serie.deserializeBytes(...)`. Throws out of
    /// range.
    #[napi]
    pub fn get(&self, index: u32) -> napi::Result<Option<Buffer>> {
        let column = erased!(self);
        if index as usize >= column.len() {
            return Err(to_error("ListSerie row index out of range"));
        }
        Ok(column
            .value(index as usize)
            .as_list()
            .map(|items| items.serialize_bytes().into()))
    }

    /// A fresh sub-column over rows `[start, start + length)` as its `serializeBytes()` frame —
    /// reconstruct it with `ListSerie.deserializeBytes(...)`. The range is clamped to the column
    /// (never throws). The Node named mirror of Python's nested `s[start:stop]` slice.
    #[napi]
    pub fn slice(&self, start: u32, length: u32) -> Buffer {
        erased!(self)
            .slice(start as usize, length as usize)
            .serialize_bytes()
            .into()
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "ListSerie(len={}, nullCount={})",
            self.inner.len(),
            self.inner.null_count()
        )
    }
}

/// The **centralized map schema** — a name, nullability, metadata, a `keysSorted` flag, and the
/// `key` / `value` fields (each a `Field` or a nested `StructField` / `ListField` / `MapField`).
#[napi(namespace = "types")]
pub struct MapField {
    pub(crate) inner: CoreMapField,
}

#[napi(namespace = "types")]
impl MapField {
    /// A map schema from a name, its `key` and `value` fields, its nullability (default `true`), and
    /// whether the entries are sorted by key (default `false`).
    #[napi(constructor)]
    pub fn new(
        name: String,
        key: Either4<&Field, &StructField, &ListField, &MapField>,
        value: Either4<&Field, &StructField, &ListField, &MapField>,
        nullable: Option<bool>,
        keys_sorted: Option<bool>,
    ) -> Self {
        Self {
            inner: CoreMapField::new(
                &name,
                to_any_field(key),
                to_any_field(value),
                nullable.unwrap_or(true),
                keys_sorted.unwrap_or(false),
            ),
        }
    }

    /// The map's name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// Whether the map column admits nulls.
    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"map"`).
    #[napi(getter)]
    pub fn type_name(&self) -> &'static str {
        "map"
    }

    /// This schema's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Map)
    }

    /// The key field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[napi(getter)]
    pub fn key(&self) -> Either4<Field, StructField, ListField, MapField> {
        from_any_field(self.inner.key())
    }

    /// The value field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[napi(getter)]
    pub fn value(&self) -> Either4<Field, StructField, ListField, MapField> {
        from_any_field(self.inner.value())
    }

    /// Whether the entries are sorted by key.
    #[napi(getter)]
    pub fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// A fresh map schema renamed to `name`.
    #[napi]
    pub fn with_name(&self, name: String) -> Self {
        Self {
            inner: self.inner.with_name(&name),
        }
    }

    /// A fresh map schema with `nullable` set.
    #[napi]
    pub fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh map schema with the `keysSorted` flag set.
    #[napi]
    pub fn with_keys_sorted(&self, keys_sorted: bool) -> Self {
        Self {
            inner: self.inner.with_keys_sorted(keys_sorted),
        }
    }

    /// A fresh map schema with one extra `key = value` metadata entry.
    #[napi]
    pub fn with_metadata_entry(&self, key: String, value: String) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(&key, &value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.as_any_field().serialize_bytes().into()
    }

    /// Reconstructs a schema from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        let field = AnyField::deserialize_bytes(&bytes).map_err(to_error)?;
        CoreMapField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| to_error("the bytes did not decode to a map field"))
    }

    /// Value equality (content, metadata included).
    #[napi]
    pub fn equals(&self, other: &MapField) -> bool {
        self.inner == other.inner
    }

    /// A content hash (equal schemas hash equal).
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "MapField(name={:?}, key={}, value={}, nullable={}, keysSorted={})",
            self.inner.name(),
            self.inner.key().type_name(),
            self.inner.value().type_name(),
            self.inner.nullable(),
            self.inner.keys_sorted()
        )
    }
}

/// A **nullable map column** — the optimized alias of `List<Struct<{key, value}>>`: `i32` offsets
/// over a flattened two-column entries store (keys non-null, values nullable), an optional top-level
/// validity mask, and a `keysSorted` flag. Row `i` is the entries `key[j] -> value[j]` for `j` in
/// `[offsets[i], offsets[i + 1])`.
#[napi(namespace = "types")]
pub struct MapSerie {
    pub(crate) inner: CoreMapSerie,
}

#[napi(namespace = "types")]
impl MapSerie {
    /// A map column from its `key` / `value` fields, each flattened child column's
    /// `serializeBytes()` frame (`keyBytes` / `valueBytes`), the row `offsets` (`len + 1` entries
    /// into the entries), an optional per-row **present** mask (`present[i] === false` marks row `i`
    /// a null map), and whether the entries are sorted by key (default `false`). A map key is never
    /// null (Arrow's Map invariant): the key column must not carry nulls.
    #[napi(factory)]
    pub fn from_parts(
        key_field: Either4<&Field, &StructField, &ListField, &MapField>,
        key_bytes: Buffer,
        value_field: Either4<&Field, &StructField, &ListField, &MapField>,
        value_bytes: Buffer,
        offsets: Vec<i32>,
        present: Option<Vec<bool>>,
        keys_sorted: Option<bool>,
    ) -> napi::Result<Self> {
        let key = to_any_field(key_field);
        let value = to_any_field(value_field);
        let keys = named_column(read_child(&key, &key_bytes)?, key.name());
        let values = named_column(read_child(&value, &value_bytes)?, value.name());
        CoreMapSerie::from_entries(
            keys,
            values,
            &offsets,
            present.as_deref(),
            keys_sorted.unwrap_or(false),
        )
        .map(|inner| Self { inner })
        .map_err(to_error)
    }

    /// The number of rows.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len() as u32
    }

    /// The number of null map rows.
    #[napi(getter)]
    pub fn null_count(&self) -> u32 {
        self.inner.null_count() as u32
    }

    /// Whether any map row is null.
    #[napi(getter)]
    pub fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether the entries are sorted by key.
    #[napi(getter)]
    pub fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// This column's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Map)
    }

    /// The row offsets (`len + 1` entries into the flattened entries).
    #[napi(getter)]
    pub fn offsets(&self) -> Vec<i32> {
        self.inner.offsets().to_vec()
    }

    /// The flattened key column (entries column 0) as its canonical bytes — reconstruct it with the
    /// matching `Serie.deserializeBytes(...)`.
    #[napi]
    pub fn keys(&self) -> Buffer {
        self.inner.keys().serialize_bytes().into()
    }

    /// The flattened value column (entries column 1) as its canonical bytes.
    #[napi]
    pub fn values(&self) -> Buffer {
        self.inner.values().serialize_bytes().into()
    }

    /// The value mapped to a probe key in row `row`, as the value's canonical little-endian bytes,
    /// or `null` if the row is null / out of range or the key is absent. The probe `keyBytes` are a
    /// leaf key's canonical bytes (what `Serie` cells serialize to); the lookup is the core's
    /// allocation-free [`MapSerie::get_value`]. Throws for a nested (non-leaf) key type.
    #[napi]
    pub fn get_value_bytes(&self, row: u32, key_bytes: Buffer) -> napi::Result<Option<Buffer>> {
        let key_field = self.inner.key_field();
        if key_field.is_struct() || key_field.is_list() || key_field.is_map() {
            return Err(to_error(
                "getValueBytes supports only a leaf map key; a nested key is not a byte-probe key",
            ));
        }
        // Rebuild the probe as the bare-leaf scalar a leaf column's `value()` produces, so the core's
        // allocation-free `cell_eq` compares canonical bytes directly (name `""`, non-null, empty
        // metadata, the key's type id + byte width).
        let probe = AnyScalar::leaf(
            CoreField::of("", key_field.type_id(), key_field.byte_width(), false),
            key_bytes.to_vec(),
        );
        Ok(self
            .inner
            .get_value(row as usize, &probe)
            .and_then(|value| value.bytes().map(|bytes| bytes.to_vec().into())))
    }

    /// A [`MapField`] naming this map column (nullability inferred from its null rows).
    #[napi]
    pub fn to_field(&self, name: String) -> MapField {
        MapField {
            inner: self.inner.to_field(&name),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][offsets][entries]`
    /// frame, identical across Rust / Python / Node.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a map column from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        CoreMapSerie::deserialize_bytes(&bytes)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Value equality (content, nulls included).
    #[napi]
    pub fn equals(&self, other: &MapSerie) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// A deep leaf cell by positional coordinates `[child, …, cell]` (a map's key child is `0`, value
    /// child `1`) — its native JS value, `null`, or a nested cell's `serializeBytes()` frame.
    #[napi]
    pub fn get_at(&self, env: Env, coords: Vec<f64>) -> napi::Result<JsUnknown> {
        deep_cell_by_coords(env, erased!(self), &coords_usize(coords)?)
    }

    /// Overwrites the deep leaf cell at `coords` with `value` (`null` clears it) — the target leaf
    /// type is read from the addressed column, so a value writes even into a currently-null cell.
    #[napi]
    pub fn set_at(&mut self, env: Env, coords: Vec<f64>, value: JsUnknown) -> napi::Result<()> {
        set_cell_by_coords(env, erased!(mut self), &coords_usize(coords)?, &value)
    }

    /// A deep cell by an index-terminal path (e.g. `[1][0]`) as its native value, or a sub-column by a
    /// name-terminal path (e.g. `key`) as its `serializeBytes()` frame.
    #[napi]
    pub fn get_path(&self, env: Env, path: String) -> napi::Result<JsUnknown> {
        cell_or_column_by_path(env, erased!(self), &path)
    }

    /// Overwrites the deep leaf cell addressed by an index-terminal `path` (e.g. `[1][0]`).
    #[napi]
    pub fn set_path(&mut self, env: Env, path: String, value: JsUnknown) -> napi::Result<()> {
        set_cell_by_path(env, erased!(mut self), &path, &value)
    }

    /// The deep cell at `key` (a coords `number[]` or an index-terminal str path) as a native value.
    #[napi]
    pub fn get_cell(&self, env: Env, key: Either<Vec<f64>, String>) -> napi::Result<JsUnknown> {
        get_cell_by_key(env, erased!(self), key)
    }

    /// Sets the deep cell at `key` (a coords `number[]` or an index-terminal str path) to `value`.
    #[napi]
    pub fn set_cell(
        &mut self,
        env: Env,
        key: Either<Vec<f64>, String>,
        value: JsUnknown,
    ) -> napi::Result<()> {
        set_cell_by_key(env, erased!(mut self), key, &value)
    }

    /// The sub-column addressed by `path` as its `serializeBytes()` frame (e.g. `getColumn("value")`).
    #[napi]
    pub fn get_column(&self, path: String) -> napi::Result<Buffer> {
        let column = erased!(self).get_by_path(&path).map_err(to_error)?;
        Ok(column.serialize_bytes().into())
    }

    /// The number of child columns (a map's key and value children).
    #[napi]
    pub fn num_children(&self) -> u32 {
        erased!(self).num_children() as u32
    }

    /// The positional child column's `serializeBytes()` frame, or `null` out of range (`0` = keys,
    /// `1` = values).
    #[napi]
    pub fn child_at(&self, index: u32) -> Option<Buffer> {
        child_at_bytes(erased!(self), index)
    }

    /// The named child column's `serializeBytes()` frame, or `null`.
    #[napi]
    pub fn child_named(&self, name: String) -> Option<Buffer> {
        child_named_bytes(erased!(self), name)
    }

    /// The `index`-th map row's `key → value` entries sub-column (a `StructSerie` of `[keys, values]`)
    /// as its `serializeBytes()` frame, or `null` if the row is null — reconstruct it with
    /// `StructSerie.deserializeBytes(...)`. Throws out of range.
    #[napi]
    pub fn get(&self, index: u32) -> napi::Result<Option<Buffer>> {
        let column = erased!(self);
        if index as usize >= column.len() {
            return Err(to_error("MapSerie row index out of range"));
        }
        Ok(column
            .value(index as usize)
            .as_map()
            .map(|(entries, _)| entries.serialize_bytes().into()))
    }

    /// A fresh sub-column over rows `[start, start + length)` as its `serializeBytes()` frame —
    /// reconstruct it with `MapSerie.deserializeBytes(...)`. The range is clamped to the column
    /// (never throws). The Node named mirror of Python's nested `s[start:stop]` slice.
    #[napi]
    pub fn slice(&self, start: u32, length: u32) -> Buffer {
        erased!(self)
            .slice(start as usize, length as usize)
            .serialize_bytes()
            .into()
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "MapSerie(len={}, nullCount={}, keysSorted={})",
            self.inner.len(),
            self.inner.null_count(),
            self.inner.keys_sorted()
        )
    }
}
