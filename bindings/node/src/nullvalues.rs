//! The `yggdryl.types` namespace's **null** value layer — Arrow's `Null`: a type whose every
//! value is null, at zero storage. `NullScalar` is the (only) null value; `NullSerie` is a run of
//! nulls stored as just its length. Mirrors `yggdryl_core::io::fixed`'s `NullScalar` / `NullSerie`.
//!
//! Every `NullScalar` is equal (and hashes the same); two `NullSerie`s are equal iff they have the
//! same length. A `NullSerie` grows via `push` / `extend`.

use napi::bindgen_prelude::{Buffer, Null};
use napi::{Env, JsUnknown};
use napi_derive::napi;

use yggdryl_core::io::fixed::{
    NullField, NullScalar as CoreNullScalar, NullSerie as CoreNullSerie,
};
use yggdryl_core::io::DataTypeId;

use crate::types::{DataType, Field};

/// Maps any core error to a thrown JS `Error` (its guided text passes through unchanged).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// One **null** value — the null type's only inhabitant.
#[napi(namespace = "types")]
pub struct NullScalar {
    pub(crate) inner: CoreNullScalar,
}

#[napi(namespace = "types")]
impl NullScalar {
    /// The null value.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: CoreNullScalar::null(),
        }
    }

    /// The null value (the cross-family name).
    #[napi(factory)]
    pub fn null() -> Self {
        Self::new()
    }

    /// Always `true` — the null type has only the null value.
    #[napi(getter)]
    pub fn is_null(&self) -> bool {
        true
    }

    /// Always `false`.
    #[napi]
    pub fn is_valid(&self) -> bool {
        false
    }

    /// The value, always `null`.
    #[napi(getter)]
    pub fn value(&self) -> Null {
        Null
    }

    /// The type name, `"null"`.
    #[napi(getter)]
    pub fn type_name(&self) -> &'static str {
        DataTypeId::Null.name()
    }

    /// This scalar's [`DataType`] (`null`, byte width `0`).
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Null)
    }

    /// A [`Field`] naming a null column (always nullable).
    #[napi]
    pub fn field(&self, name: String) -> Field {
        Field {
            inner: NullField::new(&name).erase(),
        }
    }

    /// This scalar broadcast to a length-1 [`NullSerie`].
    #[napi]
    pub fn to_serie(&self) -> NullSerie {
        NullSerie {
            inner: self.inner.to_serie(),
        }
    }

    /// The scalar's canonical bytes — empty (a null value carries nothing).
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs the null value (any input; there is only one value).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> Self {
        Self {
            inner: CoreNullScalar::deserialize_bytes(&bytes),
        }
    }

    /// Value equality — every null scalar is equal.
    #[napi]
    pub fn equals(&self, _other: &NullScalar) -> bool {
        true
    }

    /// A content hash consistent with [`equals`](Self::equals) — a constant.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        0
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
        "NullScalar()".to_string()
    }
}

impl Default for NullScalar {
    fn default() -> Self {
        Self::new()
    }
}

/// A **null column** — a run of `length` nulls, stored as just the length.
#[napi(namespace = "types")]
pub struct NullSerie {
    pub(crate) inner: CoreNullSerie,
}

#[napi(namespace = "types")]
impl NullSerie {
    /// A null column of `length` nulls (empty by default).
    #[napi(constructor)]
    pub fn new(length: Option<u32>) -> Self {
        Self {
            inner: CoreNullSerie::with_len(length.unwrap_or(0) as usize),
        }
    }

    /// A null column from an array of [`getScalar`](NullSerie::get_scalar)-shaped scalars (each a
    /// null; a `null` / `undefined` item is likewise a null). Its length is the array length.
    #[napi(factory)]
    pub fn from_scalars(scalars: Vec<Option<&NullScalar>>) -> Self {
        let scalars: Vec<CoreNullScalar> = scalars
            .into_iter()
            .map(|slot| {
                slot.map(|scalar| scalar.inner.clone())
                    .unwrap_or_else(CoreNullScalar::null)
            })
            .collect();
        Self {
            inner: CoreNullSerie::from_scalars(&scalars),
        }
    }

    /// Appends one null, growing the column by one.
    #[napi]
    pub fn push(&mut self) {
        self.inner.push();
    }

    /// Grows the column by `count` nulls.
    #[napi]
    pub fn extend(&mut self, count: u32) {
        self.inner.extend(count as usize);
    }

    /// The number of elements.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len() as u32
    }

    /// The number of null elements — always [`length`](NullSerie::length).
    #[napi(getter)]
    pub fn null_count(&self) -> u32 {
        self.inner.null_count() as u32
    }

    /// Whether the column carries any nulls — `true` unless empty.
    #[napi(getter)]
    pub fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column is empty.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The element at `index` — always `null`.
    #[napi]
    pub fn get(&self, _index: u32) -> Null {
        Null
    }

    /// Element `index` as a [`NullScalar`] (always null); throws out of range.
    #[napi]
    pub fn get_scalar(&self, index: u32) -> napi::Result<NullScalar> {
        if index as usize >= self.inner.len() {
            return Err(to_error("Serie index out of range"));
        }
        Ok(NullScalar {
            inner: self.inner.get_scalar(index as usize),
        })
    }

    /// This column's [`DataType`] (`null`).
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Null)
    }

    /// A [`Field`] naming this null column.
    #[napi]
    pub fn to_field(&self, name: String) -> Field {
        Field {
            inner: self.inner.to_field(&name).erase(),
        }
    }

    /// The column's canonical bytes — its length as a little-endian `u64`.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a column from [`serializeBytes`](Self::serialize_bytes).
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        CoreNullSerie::deserialize_bytes(&bytes)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Value equality (same length).
    #[napi]
    pub fn equals(&self, other: &NullSerie) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    // ---- Phase 8: reshape + row-selection (no arithmetic on a null column) ---------------

    /// A null column of the rows `mask` keeps (`true` keeps row `i`); throws if `mask`'s length is
    /// not this column's length.
    #[napi]
    pub fn filter(&self, mask: Vec<bool>) -> napi::Result<Self> {
        Ok(Self {
            inner: crate::ops::filter_into(&self.inner, mask)?,
        })
    }

    /// A null column, unchanged — every element is already null, so filling with a `null` /
    /// `undefined` is a no-op; a present `value` has no room in a null column and throws.
    #[napi]
    pub fn fill_null(&self, env: Env, value: JsUnknown) -> napi::Result<Self> {
        Ok(Self {
            inner: crate::ops::fill_null_into(env, &self.inner, value)?,
        })
    }

    /// This column as a one-field [`StructSerie`](crate::nested::StructSerie) named `name` (default
    /// `"value"`).
    #[napi]
    pub fn to_struct(&self, name: Option<String>) -> crate::nested::StructSerie {
        crate::ops::to_struct_wrapper(&self.inner, name)
    }

    /// This column as a list-of-singletons [`ListSerie`](crate::nested::ListSerie).
    #[napi]
    pub fn to_list(&self) -> crate::nested::ListSerie {
        crate::ops::to_list_wrapper(&self.inner)
    }

    /// This column reshaped toward a map, as its `serializeBytes()` frame (unchanged for a null
    /// column; reconstruct with the resulting class's `deserializeBytes`).
    #[napi]
    pub fn to_map(&self) -> napi::Result<Buffer> {
        crate::ops::to_map_frame(&self.inner)
    }

    // ---- Phase 9: random-access set + slice get ------------------------------------------

    /// Replaces the nested child column at `index` with the `Serie` `child` — a **leaf** null column
    /// is not nested, so the core surfaces a guided error. `child` must be a `Serie` wrapper.
    #[napi]
    pub fn set_child_at(&mut self, env: Env, index: u32, child: JsUnknown) -> napi::Result<()> {
        crate::ops::set_child_at_into(env, &mut self.inner, index, child)
    }

    /// Adds or replaces the nested child column named `name` with the `Serie` `child` — a leaf null
    /// column is not nested (guided error). `child` must be a `Serie` wrapper.
    #[napi]
    pub fn set_child_by(&mut self, env: Env, name: String, child: JsUnknown) -> napi::Result<()> {
        crate::ops::set_child_by_into(env, &mut self.inner, name, child)
    }

    /// Overwrites the length-preserving range `[offset, offset + other.length)` with `other`'s cells
    /// (a null column only accepts a null source cell). Throws on an out-of-range offset or an
    /// incompatible source cell. `other` must be a `Serie` wrapper.
    #[napi]
    pub fn set_slice(&mut self, env: Env, offset: u32, other: JsUnknown) -> napi::Result<()> {
        crate::ops::set_slice_into(env, &mut self.inner, offset, other)
    }

    /// A fresh null column over rows `[start, start + length)` (the range clamped to the column, never
    /// throws). The Node named mirror of Python's `serie[start:stop]` slice.
    #[napi]
    pub fn slice(&self, start: u32, length: u32) -> Self {
        Self {
            inner: crate::ops::slice_into(&self.inner, start, length),
        }
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!("NullSerie(len={})", self.inner.len())
    }
}
