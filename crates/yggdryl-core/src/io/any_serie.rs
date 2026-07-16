//! [`AnySerie`] — the **erased, recursive column**: any yggdryl column behind a `Box<dyn AnySerie>`,
//! so a struct column can hold heterogeneous children. It is a *contract*, not a parallel type —
//! every concrete `Serie` (`Serie<T>`, `DecimalSerie<B>`, `ByteSerie<E>`, `FixedSizeSerie<K>`,
//! `NullSerie`, and — in [`nested`](crate::io::nested) — `StructSerie`) implements it, and every
//! method delegates to that `Serie`'s own implementation. It lives at the `io` root because it spans
//! every family.
//!
//! Downcasting is safe: an `AnySerie` reports its [`field`](AnySerie::field) (and hence its exact
//! type), and `dyn AnySerie` offers [`downcast_ref`](AnySerie::downcast_ref) /
//! [`as_serie`](AnySerie::as_serie) to recover the concrete `Serie` — `None` if the assumed type is
//! wrong, so a caller keyed on the linked field never mis-reads.
//!
//! Arrow **recomposition is zero-copy** wherever the wrapped `Serie` is: the fixed-primitive columns
//! (native *and* wide) build their Arrow array from the Serie's shared `Arc` buffer + the id's Arrow
//! type, uniformly and with no per-type code; decimals share their `Arc` too.

use core::any::Any;
use core::fmt::Debug;

use super::field_carrier::any_serie_field_forwarding;
use super::fixed::{
    f16, Date32Serie, Date64Serie, Dec128, Dec256, Dec32, Dec64, DecimalBacking, DecimalField,
    DecimalSerie, Duration32Serie, Duration64Serie, Field, FixedBinarySerie, FixedElement,
    FixedSizeSerie, FixedUtf8Serie, NativeType, NullSerie, Serie, TemporalBacking, TemporalField,
    TemporalSerie, Time32Serie, Time64Serie, Ts32Serie, Ts64Serie, Ts96Serie, I256, I96, U256, U96,
};
use super::var::{BinarySerie, ByteSerie, Utf8Serie, VarElement};
use super::{
    AnyField, AnyScalar, Bytes, DataTypeId, FieldType, Headers, IoError, NodePath, PathError,
    PathSegment,
};

/// The width of one variable-length offset (`i32`).
const OFFSET_WIDTH: usize = core::mem::size_of::<i32>();

/// Clamps a `[offset, offset + len)` request to a column of `column_len` rows, returning the
/// in-bounds `(start, count)` — `start` is capped at `column_len`, `count` at the rows remaining.
/// So an out-of-range or overlong slice yields the largest valid sub-window (possibly empty) rather
/// than panicking. Shared by every [`AnySerie::slice`] implementation.
fn clamp_range(column_len: usize, offset: usize, len: usize) -> (usize, usize) {
    let start = offset.min(column_len);
    let count = len.min(column_len - start);
    (start, count)
}

/// If `probe` is a **leaf** value matching `(type_id, byte_width)` with empty metadata, returns its
/// canonical bytes, so a caller can compare them to a cell's own bytes with no allocation; `None`
/// otherwise. Behind the leaf [`cell_eq`](AnySerie::cell_eq) overrides, so a hot per-cell scan never
/// materializes an owned cell scalar. It matches the value-identity a leaf column's
/// [`value`](AnySerie::value) builds — `(type_id, byte_width, metadata, bytes)`, **excluding** the
/// name and declared nullability, exactly as [`AnyScalar`]'s `PartialEq` does — so `cell_eq` stays
/// cell-for-cell identical to the default `self.value(index) == *probe` (a `value` cell carries no
/// metadata, so a metadata-bearing probe never matches, matching the default).
fn bare_leaf_bytes(probe: &AnyScalar, type_id: DataTypeId, byte_width: usize) -> Option<&[u8]> {
    match probe {
        AnyScalar::Leaf { field, bytes }
            if FieldType::type_id(field) == type_id
                && field.byte_width() == byte_width
                && field.metadata().is_empty() =>
        {
            Some(bytes.as_slice())
        }
        _ => None,
    }
}

/// The guided error for appending an erased value whose type does not match the target column (a
/// [`Null`](AnyScalar::Null) is always accepted, so it never reaches this).
pub(crate) fn append_type_mismatch(expected: DataTypeId, value: &AnyScalar) -> IoError {
    let got = value.type_id().map_or("null", DataTypeId::name);
    IoError::Unsupported {
        what: format!(
            "cannot append a {got} value to a {} column; a present value's type must match the \
             column (a null is always accepted)",
            expected.name()
        ),
    }
}

/// The guided error for concatenating an erased column of a different concrete type.
pub(crate) fn concat_type_mismatch(expected: DataTypeId, other: &dyn AnySerie) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot concat a {} column onto a {} column; concat appends a whole column of the \
             same type",
            other.type_id().name(),
            expected.name()
        ),
    }
}

/// Maps a family error (decimal / temporal) to an [`IoError`] with the same guided text, so the
/// erased grow surface returns one error type.
fn to_io<E: core::fmt::Display>(error: E) -> IoError {
    IoError::Unsupported {
        what: error.to_string(),
    }
}

/// The guided error for overwriting a leaf cell with an erased value whose type does not match the
/// target leaf column (a [`Null`](AnyScalar::Null) is always accepted, so it never reaches this) —
/// the length-preserving [`set_cell`](AnySerie::set_cell) twin of [`append_type_mismatch`].
pub(crate) fn set_cell_type_mismatch(expected: DataTypeId, value: &AnyScalar) -> IoError {
    let got = value.type_id().map_or("null", DataTypeId::name);
    IoError::Unsupported {
        what: format!(
            "cannot set a {got} value into a {} cell; a present value's type must match the leaf \
             column (a null is always accepted)",
            expected.name()
        ),
    }
}

/// The guided error for calling [`set_cell`](AnySerie::set_cell) on a **nested** column — a whole
/// struct / list / map cell has no length-preserving in-place overwrite, so the deep setter reaches
/// only a leaf cell.
pub(crate) fn set_cell_on_nested(kind: DataTypeId) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot overwrite a whole {} cell in place; a deep set targets a LEAF cell — extend the \
             path with the field / index segments that reach a leaf column, and grow a nested column \
             through append_row / concat instead",
            kind.name()
        ),
    }
}

/// The guided error for a [`filter`](AnySerie::filter) whose boolean mask length does not match the
/// column — the mask must carry exactly one flag per row. Shared by every family's typed `filter`.
pub(crate) fn filter_len_mismatch(mask_len: usize, column_len: usize) -> IoError {
    IoError::Unsupported {
        what: format!(
            "filter mask length {mask_len} does not match the column length {column_len}; the mask \
             must carry exactly one boolean per row"
        ),
    }
}

/// The guided error for a [`fill_null`](AnySerie::fill_null) whose fill value's type does not match
/// the target leaf column (a [`Null`](AnyScalar::Null) value is a no-op, so it never reaches this).
pub(crate) fn fill_null_type_mismatch(expected: DataTypeId, value: &AnyScalar) -> IoError {
    let got = value.type_id().map_or("null", DataTypeId::name);
    IoError::Unsupported {
        what: format!(
            "cannot fill the nulls of a {} column with a {got} value; the fill value's type must \
             match the column (or be null for a no-op)",
            expected.name()
        ),
    }
}

/// Maps a [`PathError`] (a bad path string, or a segment that failed to resolve) to an [`IoError`]
/// carrying its guided text, so the deep setter returns one error type while keeping the exact
/// position/segment guidance [`get_by_path`](AnySerie::get_by_path) gives.
fn path_to_io(error: PathError) -> IoError {
    IoError::Unsupported {
        what: error.to_string(),
    }
}

/// A **column of any type**, type-erased — the recursive carrier a struct column's heterogeneous
/// children live in (`Box<dyn AnySerie>`). Implemented by every concrete `Serie`; each method
/// delegates. Build one by boxing a `Serie` (`Box::new(serie) as Box<dyn AnySerie>`) or with the
/// erased reader / Arrow importer in [`nested`](crate::io::nested).
///
/// `Send + Sync` because every concrete column is (its buffers are `Arc`-shared or owned `Vec`s,
/// like Arrow's own `Send + Sync` `ArrayRef`), so an erased column — and a `StructSerie` of them —
/// crosses threads and satisfies the language bindings' thread-safety bound.
pub trait AnySerie: Debug + Send + Sync {
    /// The number of elements.
    fn len(&self) -> usize;

    /// The number of null elements.
    fn null_count(&self) -> usize;

    /// Whether the column is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the column carries any nulls.
    fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }

    /// The column's element [`DataTypeId`].
    fn type_id(&self) -> DataTypeId;

    /// The column's declared name (from its stored header — empty by default).
    fn name(&self) -> &str;

    /// Overwrites the column's declared name in its stored header — used to name a child before
    /// building a struct/list/map, and to restore a child's header when reading a nested frame.
    fn set_name(&mut self, name: &str);

    /// Overwrites the column's declared nullability in its stored header.
    fn set_nullable(&mut self, nullable: bool);

    /// Overwrites the column's metadata in its stored header (moved in, no clone).
    fn set_metadata(&mut self, metadata: Headers);

    /// The [`AnyField`] naming a column of this type `name`, using the column's stored header
    /// (metadata + **effective** nullability, `declared || has_nulls`) but with the name overridden
    /// by the passed `name`.
    fn field(&self, name: &str) -> AnyField;

    /// The [`AnyField`] this column contributes from its stored header (its stored name) — the no-arg
    /// counterpart of [`field`](AnySerie::field) that keeps the header name.
    fn field_self(&self) -> AnyField {
        self.field(self.name())
    }

    /// The value at `index` as an erased [`AnyScalar`] — null if the element is null or out of range.
    fn value(&self, index: usize) -> AnyScalar;

    /// Whether the cell at `index` equals the erased value `probe` — the **allocation-free**
    /// counterpart of `self.value(index) == *probe`, for a hot per-cell scan (e.g.
    /// [`MapSerie::get_value`](crate::io::nested::MapSerie::get_value), which compares every stored key
    /// in a row against one probe key). The default materializes one owned cell scalar
    /// (`self.value(index) == *probe`); the leaf primitive columns override it to compare **borrowed**
    /// cell bytes against the probe with no per-call allocation. Any override MUST agree with the
    /// default cell-for-cell.
    fn cell_eq(&self, index: usize, probe: &AnyScalar) -> bool {
        self.value(index) == *probe
    }

    /// A **new** erased column holding the elements `[offset, offset + len)` — the range is clamped
    /// to the column (an out-of-range or overlong request yields the in-bounds sub-window, never a
    /// panic). Used to materialize a list row's item sub-column. The result is a fresh column (a
    /// null/OOB-safe copy, Arc-shared where cheap); the original is untouched.
    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie>;

    /// A **new** erased column keeping the rows where `mask[i]` is `true` — the type-erased row
    /// filter. `mask` must have exactly one boolean per row (a guided
    /// [`Unsupported`](IoError::Unsupported) error otherwise). A selected row keeps its value *and*
    /// its null-ness. For a **struct** column every child column is filtered by the same mask (plus
    /// the struct's own row validity); for a **list** / **map** column whole *rows* are kept or
    /// dropped — the selected rows' offset ranges are kept and the offsets rebuilt, so the flattened
    /// child is never filtered element-wise (only whole rows drop out).
    ///
    /// DESIGN: `filter` has no `_unchecked` twin — its one precondition (the mask length) is the
    /// cheap check it always does, so a fast path would save nothing.
    fn filter(&self, mask: &[bool]) -> Result<Box<dyn AnySerie>, IoError>;

    /// A **new** erased column with every null replaced by `value` — the type-erased null fill. On a
    /// **leaf** column `value` must be a leaf of the column's own type (a guided
    /// [`Unsupported`](IoError::Unsupported) error otherwise); a [`Null`](AnyScalar::Null) `value` is
    /// a no-op clone. On a **struct** / **list** / **map** column the fill **recurses to the leaf
    /// children**, replacing nulls in each leaf whose type matches `value` and leaving the rest
    /// unchanged — so a nested fill never errors on a column whose leaves differ from `value` (a
    /// heterogeneous struct fills only the matching columns). A filled leaf drops its validity mask
    /// (it is now fully present); a nested column's own row-nulls are **not** filled (only leaf cells).
    fn fill_null(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError>;

    /// Appends one erased value (a null or a cell of this column's type) — the erased single-row grow
    /// primitive that the nested [`append_row`](crate::io::nested::StructSerie::append_row) routes a
    /// child through. A [`Null`](crate::io::AnyScalar::Null) is always accepted (lenient nullability);
    /// a present value must match this column's type, else a guided
    /// [`Unsupported`](IoError::Unsupported) error. Delegates to the wrapped `Serie`'s own `push`.
    fn append_scalar(&mut self, value: &AnyScalar) -> Result<(), IoError>;

    /// Appends **another whole erased column** of the **same concrete type** to this one — the erased
    /// bulk grow that the nested [`concat`](crate::io::nested::StructSerie::concat) routes each child
    /// through, so a nested concat stays one copy-on-write per child. Errors
    /// [`Unsupported`](IoError::Unsupported) if `other` is a different column type; delegates to the
    /// wrapped `Serie`'s own `concat` (which reconciles any descriptor).
    fn concat(&mut self, other: &dyn AnySerie) -> Result<(), IoError>;

    /// Overwrites the cell at `index` with the erased `value`, **preserving the column length** — the
    /// safe, length-preserving counterpart of [`append_scalar`](AnySerie::append_scalar) and the leaf
    /// primitive the deep [`set_by_path`](AnySerie::set_by_path) writes through. A present value must
    /// match this **leaf** column's type (a [`Null`](crate::io::AnyScalar::Null) is always accepted,
    /// setting the cell null under lenient nullability); it delegates to the wrapped leaf `Serie`'s own
    /// length-preserving `set` / `set_bytes`, so no length ever changes and a nested parent's
    /// equal-length / offset invariants stay intact. A **nested** column (struct / list / map) errors
    /// [`Unsupported`](IoError::Unsupported): a whole nested cell has no length-preserving in-place
    /// overwrite (a list / map cell would resize the flattened child), so grow it through
    /// [`append_row`](crate::io::nested::StructSerie::append_row) / `concat` instead. Errors
    /// [`IndexOutOfBounds`](IoError::IndexOutOfBounds) past the end, or
    /// [`Unsupported`](IoError::Unsupported) on a type / width mismatch.
    fn set_cell(&mut self, index: usize, value: &AnyScalar) -> Result<(), IoError>;

    /// Writes this column to `sink` — delegates to the wrapped `Serie`'s own byte codec.
    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError>;

    /// This column as an Arrow [`ArrayRef`](arrow_array::ArrayRef) (feature `arrow`) — delegates to
    /// the wrapped `Serie`'s own (zero-copy where it is) converter. Fallible because a temporal
    /// column at a resolution Arrow cannot express (`Minute`…`Year`) has no Arrow array.
    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError>;

    /// A boxed clone (value semantics for `Box<dyn AnySerie>`).
    fn clone_box(&self) -> Box<dyn AnySerie>;

    /// Content equality against another erased column (equal type *and* value).
    fn eq_any(&self, other: &dyn AnySerie) -> bool;

    /// This column as `&dyn Any`, for the safe downcast helpers.
    fn as_any(&self) -> &dyn Any;

    /// This column as `&mut dyn Any` — the crate-internal mutable downcast the deep-cell setter uses
    /// to reach a nested column's child column in place while walking a path. Safe: the recovered
    /// `&mut ConcreteSerie` exposes only its **public** (length-preserving) methods; the raw child
    /// accessors (`column_at_mut`, `values_mut`, …) stay `pub(crate)`, so no `&mut` child leaks that
    /// could desync a parent's length.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// This column's canonical bytes (the wrapped `Serie`'s frame), as an owned `Vec`.
    fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Names this (self-describing) column and erases it to a `Box<dyn AnySerie>` — the one-line
    /// shorthand for building a struct / list / map child (`Serie::from_values(&[1i32, 2]).named("x")`).
    /// It replaces the removed `NamedSerie` carrier: the name is written straight into the column's
    /// own header, so the boxed column *is* the self-describing child the builders take.
    fn named(self, name: &str) -> Box<dyn AnySerie>
    where
        Self: Sized + 'static,
    {
        let mut boxed: Box<dyn AnySerie> = Box::new(self);
        boxed.set_name(name);
        boxed
    }

    // ---- unified child access (leaf-safe defaults; only the nested columns override) -----------

    /// The number of **child columns** — a struct's fields, a list's one item child, a map's two
    /// (`key`, `value`) children. A leaf column has none, so the default is `0` and only the nested
    /// columns override it. This is the *schema-structure* fan-out, matching
    /// [`AnyField::num_children`](crate::io::AnyField::num_children).
    fn num_children(&self) -> usize {
        0
    }

    /// The child column at `index`, or `None` if out of range (a leaf has no children). See
    /// [`num_children`](AnySerie::num_children) for the ordering.
    fn child_serie_at(&self, index: usize) -> Option<&(dyn AnySerie + 'static)> {
        let _ = index; // a leaf has no children — only the nested columns override
        None
    }

    /// The child column named `name`, or `None` (a leaf has no children). A struct matches a field
    /// name, a list matches its item name, a map matches its key/value child names (or the canonical
    /// `"key"` / `"value"`).
    fn child_serie_by(&self, name: &str) -> Option<&(dyn AnySerie + 'static)> {
        let _ = name;
        None
    }

    // DESIGN: there is intentionally **no** public `child_serie_at_mut`. Handing out a raw
    // `&mut dyn AnySerie` child let safe code call `append_scalar` / `concat` on **one** child of a
    // struct / list / map, silently desyncing the parent's length invariant (struct: all columns
    // equal len; list/map: `offsets[last] == child len`) or flipping a map key column to nullable —
    // corruption caught only on serialize / Arrow-export, if at all. Public mutation therefore stays
    // length-preserving: grow through the parent's `append_row` / `append_null` / `concat` (which
    // grow every child together), set one leaf cell through `set` / `set_scalar`, and overwrite a
    // deep leaf cell through the length-preserving [`set_by_path`](AnySerie::set_by_path) /
    // [`set_cell`](AnySerie::set_cell). The crate's own internal routing uses the `pub(crate)`
    // inherent accessors (`StructSerie::column_at_mut`, `child_serie_by_mut`, …), reached only via
    // the safe [`as_any_mut`](AnySerie::as_any_mut) mutable downcast.
}

/// Restores a child column's stored header (name / declared nullability / metadata) from the field
/// that describes it — used when reading a nested frame or importing a nested Arrow array, so the
/// child's derived [`field_self`](AnySerie::field_self) round-trips exactly (a nested column is
/// unreconstructable without its child names).
pub(crate) fn apply_field_header(column: &mut Box<dyn AnySerie>, field: &AnyField) {
    column.set_name(field.name());
    column.set_nullable(field.nullable());
    column.set_metadata(field.metadata().clone());
}

impl dyn AnySerie {
    /// The concrete `Serie` behind this erased column, if it is of type `S` — the safe downcast a
    /// caller reaches for after reading the [`field`](AnySerie::field) to know the type. `None` if
    /// the assumed type is wrong.
    pub fn downcast_ref<S: AnySerie + 'static>(&self) -> Option<&S> {
        self.as_any().downcast_ref::<S>()
    }

    /// Whether this erased column is a concrete `S`.
    pub fn is<S: AnySerie + 'static>(&self) -> bool {
        self.as_any().is::<S>()
    }

    /// This column as a fixed-width primitive [`Serie<T>`](Serie), if it is one — the `as_ref`-style
    /// typed view (`any.as_serie::<i32>()`), keyed on the element type the field reports.
    pub fn as_serie<T: NativeType>(&self) -> Option<&Serie<T>> {
        self.downcast_ref::<Serie<T>>()
    }

    /// This column as a [`DecimalSerie<B>`](DecimalSerie), if it is one.
    pub fn as_decimal<B: DecimalBacking>(&self) -> Option<&DecimalSerie<B>>
    where
        B::Coeff: arrow_buffer::ArrowNativeType,
    {
        self.downcast_ref::<DecimalSerie<B>>()
    }

    /// This column as a variable-length [`ByteSerie<E>`](ByteSerie) (`Utf8Serie` / `BinarySerie`).
    pub fn as_bytes_serie<E: VarElement>(&self) -> Option<&ByteSerie<E>> {
        self.downcast_ref::<ByteSerie<E>>()
    }

    /// This column as a [`TemporalSerie<B>`](TemporalSerie) (`Date32Serie` … `Duration64Serie`), if
    /// it is one — keyed on the concept+width marker the field's type reports.
    pub fn as_temporal<B: TemporalBacking>(&self) -> Option<&TemporalSerie<B>> {
        self.downcast_ref::<TemporalSerie<B>>()
    }

    /// Resolves `path` against this column's nested structure, returning the addressed **child
    /// column** — the schema-structure walk symmetric with
    /// [`AnyField::get_by_path`](crate::io::AnyField::get_by_path). Each
    /// [`Name`](crate::io::PathSegment::Name) segment follows [`child_serie_by`](AnySerie::child_serie_by),
    /// each [`Index`](crate::io::PathSegment::Index) segment follows
    /// [`child_serie_at`](AnySerie::child_serie_at); the empty path returns this column. The returned
    /// borrow is tied to `&self` (a transient view, not a stored value).
    ///
    /// # Errors
    /// A [`PathError`] from [`NodePath::parse`](crate::io::NodePath::parse), or a
    /// [`PathError::NoChildNamed`] / [`PathError::ChildIndexOutOfRange`] naming the depth and the
    /// missing segment.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::nested::{ListSerie, StructSerie};
    /// use yggdryl_core::io::{boxed, AnySerie};
    ///
    /// // struct<a: list<struct<{b: i32}>>>
    /// let b = Serie::from_values(&[10i32, 20, 30]).named("b");
    /// let inner = boxed(StructSerie::from_series(vec![b]).unwrap());
    /// let list = ListSerie::from_values(inner, &[0, 2, 3], None).unwrap();
    /// let root = StructSerie::from_named(vec![("a", boxed(list))]).unwrap();
    ///
    /// let b = (&root as &dyn AnySerie).get_by_path("a[0].b").unwrap();
    /// assert_eq!(b.name(), "b");
    /// assert_eq!(b.len(), 3);
    /// ```
    pub fn get_by_path(&self, path: &str) -> Result<&(dyn AnySerie + 'static), PathError> {
        let parsed = NodePath::parse(path)?;
        resolve_serie(self, &parsed)
    }

    /// **Overwrites a single leaf cell** addressed by `path`, in place and **length-preservingly** —
    /// the safe deep-set symmetric with [`get_by_path`](AnySerie::get_by_path). All but the last path
    /// segment navigate to a leaf **column** exactly as `get_by_path` would (each
    /// [`Name`](crate::io::PathSegment::Name) follows the child column's name, each
    /// [`Index`](crate::io::PathSegment::Index) the positional child), walking through the crate's
    /// `pub(crate)` **mutable** child accessors; the final [`Index`](crate::io::PathSegment::Index) is
    /// the **cell** position within that leaf column, overwritten via
    /// [`set_cell`](AnySerie::set_cell). The `value` is type-checked against the leaf's element (a null
    /// [`AnyScalar`](crate::io::AnyScalar) sets a null under lenient nullability).
    ///
    /// DESIGN: this is **overwrite-only** — every hop is a length-preserving overwrite of an existing
    /// cell, so it can never change a column length and therefore never desync a struct's equal-length
    /// or a list / map's `offsets[last] == child len` invariant (unlike a raw `&mut` child, which is
    /// why none is exposed). To **grow** a nested column, use `append_row` / `append_null` / `concat`.
    ///
    /// # Errors
    /// A guided [`Unsupported`](IoError::Unsupported) wrapping a
    /// [`PathError`](crate::io::PathError) for a bad path string or a segment that names / indexes a
    /// child the node does not have; for the empty path or a final **name** segment (a cell is
    /// addressed by index); if the path resolves to a **non-leaf** (nested) column; or an
    /// [`IndexOutOfBounds`](IoError::IndexOutOfBounds) if the final cell index is past the leaf's end.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{Field, Serie};
    /// use yggdryl_core::io::nested::StructSerie;
    /// use yggdryl_core::io::{boxed, AnyScalar, AnySerie, DataTypeId};
    ///
    /// // struct<a: i32> of 3 rows; overwrite row 1 of child `a` with 99.
    /// let mut root = boxed(StructSerie::from_series(vec![
    ///     Serie::from_values(&[10i32, 20, 30]).named("a"),
    /// ])
    /// .unwrap());
    /// let ninety_nine =
    ///     AnyScalar::leaf(Field::of("", DataTypeId::I32, 4, false), 99i32.to_le_bytes().to_vec());
    /// root.set_by_path("a[1]", &ninety_nine).unwrap();
    /// // Read the leaf column back and check the cell changed; the length is unchanged.
    /// let a = root.get_by_path("a").unwrap();
    /// assert_eq!(a.value(1), ninety_nine);
    /// assert_eq!(a.len(), 3);
    /// ```
    pub fn set_by_path(&mut self, path: &str, value: &AnyScalar) -> Result<(), IoError> {
        let parsed = NodePath::parse(path).map_err(path_to_io)?;
        set_by_segments(self, parsed.segments(), value)
    }

    /// **Overwrites a single leaf cell** addressed by a pure-**coordinate** path — the index-only twin
    /// of [`set_by_path`](AnySerie::set_by_path). Each of `coords` is a positional child index; the
    /// leading coordinates navigate to a leaf column (a struct field by position, a list's item child
    /// with `0`, a map's key / value child with `0` / `1`) and the **last** is the cell position within
    /// it. Same length-preserving, overwrite-only guarantee and same errors as `set_by_path`.
    pub fn set_at(&mut self, coords: &[usize], value: &AnyScalar) -> Result<(), IoError> {
        let segments: Vec<PathSegment> = coords
            .iter()
            .map(|&index| PathSegment::Index(index))
            .collect();
        set_by_segments(self, &segments, value)
    }

    /// **Reads a single leaf cell** addressed by `path` — the read-twin of
    /// [`set_by_path`](AnySerie::set_by_path). All but the last path segment navigate to a leaf
    /// **column** exactly as [`get_by_path`](AnySerie::get_by_path) would; the final
    /// [`Index`](crate::io::PathSegment::Index) is the **cell** position within that leaf, read via
    /// [`value`](AnySerie::value).
    ///
    /// Where [`get_by_path`](AnySerie::get_by_path) returns the addressed sub-**column** (a `&dyn
    /// AnySerie`, every segment a column hop), this returns the single [`AnyScalar`] **cell** a
    /// trailing index addresses — the exact value [`set_by_path`](AnySerie::set_by_path) would write,
    /// so `col.get_scalar_by_path(p)` and `col.set_by_path(p, v)` name the same location.
    ///
    /// # Errors
    /// A guided [`Unsupported`](IoError::Unsupported) wrapping a
    /// [`PathError`](crate::io::PathError) for a bad path string or a missing child; for the empty path
    /// or a final **name** segment (a cell is addressed by index); or an
    /// [`IndexOutOfBounds`](IoError::IndexOutOfBounds) if the final cell index is past the leaf's end.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{Field, Serie};
    /// use yggdryl_core::io::nested::StructSerie;
    /// use yggdryl_core::io::{boxed, AnyScalar, AnySerie, DataTypeId};
    ///
    /// let mut root = boxed(StructSerie::from_series(vec![
    ///     Serie::from_values(&[10i32, 20, 30]).named("a"),
    /// ])
    /// .unwrap());
    /// let ninety_nine =
    ///     AnyScalar::leaf(Field::of("", DataTypeId::I32, 4, false), 99i32.to_le_bytes().to_vec());
    /// root.set_by_path("a[1]", &ninety_nine).unwrap();
    /// // The read-twin returns exactly the written cell.
    /// assert_eq!(root.get_scalar_by_path("a[1]").unwrap(), ninety_nine);
    /// ```
    pub fn get_scalar_by_path(&self, path: &str) -> Result<AnyScalar, IoError> {
        let parsed = NodePath::parse(path).map_err(path_to_io)?;
        get_scalar_by_segments(self, parsed.segments())
    }

    /// **Reads a single leaf cell** addressed by a pure-**coordinate** path — the index-only twin of
    /// [`get_scalar_by_path`](AnySerie::get_scalar_by_path) and the read-twin of
    /// [`set_at`](AnySerie::set_at). Each of `coords` is a positional child index; the leading
    /// coordinates navigate to a leaf column and the **last** is the cell position within it, so
    /// `col.get_at(c)` reads exactly what `col.set_at(c, v)` writes.
    pub fn get_at(&self, coords: &[usize]) -> Result<AnyScalar, IoError> {
        let segments: Vec<PathSegment> = coords
            .iter()
            .map(|&index| PathSegment::Index(index))
            .collect();
        get_scalar_by_segments(self, &segments)
    }

    /// A transient `NodeRef` cursor rooted at this column — the crate-internal entry point for the
    /// graph-cursor drill-down. Reserved for later phases (only its own tests use it today).
    #[allow(dead_code)]
    pub(crate) fn root_ref(&self) -> super::node_ref::NodeRef<'_> {
        super::node_ref::NodeRef::new(self)
    }
}

/// Walks `path` from `root` through the unified child accessors, returning the addressed child
/// column. Shared by [`get_by_path`](AnySerie::get_by_path) (guided errors) and the `NodeRef`
/// cursor's parent re-resolution.
pub(crate) fn resolve_serie<'a>(
    root: &'a (dyn AnySerie + 'static),
    path: &NodePath,
) -> Result<&'a (dyn AnySerie + 'static), PathError> {
    let mut current = root;
    for (depth, segment) in path.segments().iter().enumerate() {
        current =
            match segment {
                PathSegment::Name(name) => {
                    current
                        .child_serie_by(name)
                        .ok_or_else(|| PathError::NoChildNamed {
                            depth,
                            name: name.clone(),
                            num_children: current.num_children(),
                        })?
                }
                PathSegment::Index(index) => current.child_serie_at(*index).ok_or_else(|| {
                    PathError::ChildIndexOutOfRange {
                        depth,
                        index: *index,
                        num_children: current.num_children(),
                    }
                })?,
            };
    }
    Ok(current)
}

/// Walks `segments` from `root` through the **mutable** child accessors to reach a leaf column, then
/// overwrites the cell the final [`Index`](PathSegment::Index) addresses via
/// [`set_cell`](AnySerie::set_cell). Shared by [`set_by_path`](AnySerie::set_by_path) (parses a path)
/// and [`set_at`](AnySerie::set_at) (pure coordinates). Every hop is length-preserving, so no column
/// length changes and no nested invariant can desync.
fn set_by_segments(
    root: &mut (dyn AnySerie + 'static),
    segments: &[PathSegment],
    value: &AnyScalar,
) -> Result<(), IoError> {
    let Some((last, interior)) = segments.split_last() else {
        return Err(IoError::Unsupported {
            what: "a deep cell set needs a non-empty path to a leaf cell (e.g. `a[1]` or \
                   `[0].x[2]`); the empty path addresses the whole column, not a cell"
                .to_string(),
        });
    };
    // Navigate the interior segments to the final container. Each step first probes existence with the
    // *immutable* child accessor (so the guided error can read `num_children` without holding a `&mut`),
    // then re-resolves mutably — the two are symmetric, so the mutable step cannot then miss.
    let mut container: &mut (dyn AnySerie + 'static) = root;
    for (depth, segment) in interior.iter().enumerate() {
        let num_children = container.num_children();
        let exists = match segment {
            PathSegment::Name(name) => container.child_serie_by(name).is_some(),
            PathSegment::Index(index) => container.child_serie_at(*index).is_some(),
        };
        if !exists {
            return Err(path_to_io(match segment {
                PathSegment::Name(name) => PathError::NoChildNamed {
                    depth,
                    name: name.clone(),
                    num_children,
                },
                PathSegment::Index(index) => PathError::ChildIndexOutOfRange {
                    depth,
                    index: *index,
                    num_children,
                },
            }));
        }
        container = crate::io::nested::child_serie_mut(container, segment)
            .expect("existence verified by the immutable probe above");
    }
    // The final segment is the cell index within the leaf container.
    let cell = match last {
        PathSegment::Index(index) => *index,
        PathSegment::Name(name) => {
            return Err(IoError::Unsupported {
                what: format!(
                    "the final path segment must be a cell index (e.g. `[1]`), but got the name \
                     {name:?}; a name addresses a child column, not a cell — append an index segment \
                     to reach a cell"
                ),
            })
        }
    };
    container.set_cell(cell, value)
}

/// Walks `segments` from `root` through the **immutable** child accessors to reach a leaf column, then
/// reads the cell the final [`Index`](PathSegment::Index) addresses via [`value`](AnySerie::value).
/// Shared by [`get_scalar_by_path`](AnySerie::get_scalar_by_path) (parses a path) and
/// [`get_at`](AnySerie::get_at) (pure coordinates) — the read-twin of [`set_by_segments`].
fn get_scalar_by_segments(
    root: &(dyn AnySerie + 'static),
    segments: &[PathSegment],
) -> Result<AnyScalar, IoError> {
    let Some((last, interior)) = segments.split_last() else {
        return Err(IoError::Unsupported {
            what: "a deep cell read needs a non-empty path to a leaf cell (e.g. `a[1]` or \
                   `[0].x[2]`); the empty path addresses the whole column, not a cell"
                .to_string(),
        });
    };
    let mut container = root;
    for (depth, segment) in interior.iter().enumerate() {
        let num_children = container.num_children();
        container = match segment {
            PathSegment::Name(name) => container.child_serie_by(name).ok_or_else(|| {
                path_to_io(PathError::NoChildNamed {
                    depth,
                    name: name.clone(),
                    num_children,
                })
            })?,
            PathSegment::Index(index) => container.child_serie_at(*index).ok_or_else(|| {
                path_to_io(PathError::ChildIndexOutOfRange {
                    depth,
                    index: *index,
                    num_children,
                })
            })?,
        };
    }
    let cell = match last {
        PathSegment::Index(index) => *index,
        PathSegment::Name(name) => {
            return Err(IoError::Unsupported {
                what: format!(
                    "the final path segment must be a cell index (e.g. `[1]`), but got the name \
                     {name:?}; a name addresses a child column, not a cell — append an index segment \
                     to reach a cell"
                ),
            })
        }
    };
    if cell >= container.len() {
        return Err(IoError::IndexOutOfBounds {
            index: cell,
            len: container.len(),
        });
    }
    Ok(container.value(cell))
}

/// Boxes a concrete `Serie` as an erased [`AnySerie`] column — the ergonomic constructor for a
/// heterogeneous child, e.g. `boxed(Serie::from_values(&[1i32, 2]))`.
pub fn boxed<S: AnySerie + 'static>(serie: S) -> Box<dyn AnySerie> {
    Box::new(serie)
}

impl Clone for Box<dyn AnySerie> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl PartialEq for dyn AnySerie {
    fn eq(&self, other: &Self) -> bool {
        self.eq_any(other)
    }
}

impl Eq for dyn AnySerie {}

/// Writes an `eq_any` in terms of `PartialEq` + a same-type downcast — a value type equals another
/// erased column iff it is the same concrete `Serie` type and compares equal.
macro_rules! eq_via_downcast {
    () => {
        fn eq_any(&self, other: &dyn AnySerie) -> bool {
            other
                .as_any()
                .downcast_ref::<Self>()
                .is_some_and(|other| self == other)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn clone_box(&self) -> Box<dyn AnySerie> {
            // UFCS to clone `Self` (not the `&Self` reference — `self.clone()` would autoref).
            Box::new(<Self as Clone>::clone(self))
        }
    };
}

// -------------------------------------------------------------------------------------
// Fixed-width primitive columns: one blanket impl. Arrow export builds the array from the
// Serie's shared Arc buffer, so it is zero-copy and uniform over native *and* wide integers.
// -------------------------------------------------------------------------------------

impl<T: NativeType> AnySerie for Serie<T> {
    fn len(&self) -> usize {
        Serie::len(self)
    }

    fn null_count(&self) -> usize {
        Serie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        T::TYPE_ID
    }

    any_serie_field_forwarding!();

    fn field(&self, name: &str) -> AnyField {
        self.field().with_name(name)
    }

    fn value(&self, index: usize) -> AnyScalar {
        match self.get(index) {
            Some(value) => {
                let mut scratch = [0u8; 32];
                value.write_le(&mut scratch);
                AnyScalar::leaf(
                    Field::of("", T::TYPE_ID, T::WIDTH, false),
                    scratch[..T::WIDTH].to_vec(),
                )
            }
            None => AnyScalar::Null,
        }
    }

    fn cell_eq(&self, index: usize, probe: &AnyScalar) -> bool {
        // The cell's canonical bytes go into a stack scratch (a fixed primitive is <= 32 bytes), so
        // the compare against the probe's borrowed bytes allocates nothing.
        match self.get(index) {
            Some(value) => match bare_leaf_bytes(probe, T::TYPE_ID, T::WIDTH) {
                Some(probe_bytes) => {
                    let mut scratch = [0u8; 32];
                    value.write_le(&mut scratch);
                    &scratch[..T::WIDTH] == probe_bytes
                }
                None => false,
            },
            None => probe.is_null(),
        }
    }

    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie> {
        let (start, count) = clamp_range(Serie::len(self), offset, len);
        let values: Vec<Option<T>> = (start..start + count)
            .map(|index| self.get(index))
            .collect();
        Box::new(Serie::from_options(&values))
    }

    fn filter(&self, mask: &[bool]) -> Result<Box<dyn AnySerie>, IoError> {
        Ok(Box::new(Serie::filter(self, mask)?))
    }

    fn fill_null(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        match value {
            AnyScalar::Null => Ok(Box::new(self.clone())),
            AnyScalar::Leaf { field, bytes }
                if FieldType::type_id(field) == T::TYPE_ID && bytes.len() == T::WIDTH =>
            {
                Ok(Box::new(Serie::fill_null(self, T::read_le(bytes))))
            }
            other => Err(fill_null_type_mismatch(T::TYPE_ID, other)),
        }
    }

    fn append_scalar(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => {
                Serie::push(self, None);
                Ok(())
            }
            AnyScalar::Leaf { field, bytes }
                if FieldType::type_id(field) == T::TYPE_ID && bytes.len() == T::WIDTH =>
            {
                Serie::push(self, Some(T::read_le(bytes)));
                Ok(())
            }
            other => Err(append_type_mismatch(T::TYPE_ID, other)),
        }
    }

    fn concat(&mut self, other: &dyn AnySerie) -> Result<(), IoError> {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => {
                Serie::concat(self, other);
                Ok(())
            }
            None => Err(concat_type_mismatch(T::TYPE_ID, other)),
        }
    }

    fn set_cell(&mut self, index: usize, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => Serie::set(self, index, None),
            AnyScalar::Leaf { field, bytes }
                if FieldType::type_id(field) == T::TYPE_ID && bytes.len() == T::WIDTH =>
            {
                Serie::set(self, index, Some(T::read_le(bytes)))
            }
            other => Err(set_cell_type_mismatch(T::TYPE_ID, other)),
        }
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        Serie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        let data_type = T::TYPE_ID.to_arrow(T::WIDTH);
        let values = self.arrow_value_buffer();
        let nulls = self
            .validity_bitmap()
            .map(|bitmap| arrow_buffer::Buffer::from(bitmap.as_bytes()));
        let data =
            arrow_data::ArrayData::try_new(data_type, self.len(), nulls, 0, vec![values], vec![])
                .expect("a primitive column's Arc buffer is valid for its Arrow type");
        Ok(arrow_array::make_array(data))
    }

    eq_via_downcast!();
}

// -------------------------------------------------------------------------------------
// Decimal, variable-length, fixed-size, and null columns: each delegates to its own Serie.
// -------------------------------------------------------------------------------------

impl<B: DecimalBacking> AnySerie for DecimalSerie<B>
where
    B::Coeff: arrow_buffer::ArrowNativeType,
{
    fn len(&self) -> usize {
        DecimalSerie::len(self)
    }

    fn null_count(&self) -> usize {
        DecimalSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        B::TYPE_ID
    }

    any_serie_field_forwarding!();

    fn field(&self, name: &str) -> AnyField {
        self.field().with_name(name)
    }

    fn value(&self, index: usize) -> AnyScalar {
        if index >= self.len() || self.get_coeff(index).is_none() {
            return AnyScalar::Null;
        }
        let field = DecimalField::<B>::new("", self.precision(), self.scale(), false).erase();
        let bytes = self.coeff_bytes()[index * B::WIDTH..(index + 1) * B::WIDTH].to_vec();
        AnyScalar::leaf(field, bytes)
    }

    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie> {
        let (start, count) = clamp_range(DecimalSerie::len(self), offset, len);
        let values: Vec<_> = (start..start + count)
            .map(|index| self.get(index))
            .collect();
        Box::new(
            DecimalSerie::<B>::from_options(self.precision(), self.scale(), &values)
                .expect("a column's own values re-fit its own precision/scale exactly"),
        )
    }

    fn filter(&self, mask: &[bool]) -> Result<Box<dyn AnySerie>, IoError> {
        Ok(Box::new(DecimalSerie::filter(self, mask)?))
    }

    fn fill_null(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        match value {
            AnyScalar::Null => Ok(Box::new(self.clone())),
            AnyScalar::Leaf { field, bytes }
                if FieldType::type_id(field) == B::TYPE_ID && bytes.len() == B::WIDTH =>
            {
                // The bytes are the raw coefficient at this column's scale (matching-schema fill).
                Ok(Box::new(self.fill_null_coeff_bytes(bytes)))
            }
            other => Err(fill_null_type_mismatch(B::TYPE_ID, other)),
        }
    }

    fn append_scalar(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => DecimalSerie::push(self, None).map_err(to_io),
            AnyScalar::Leaf { field, bytes }
                if FieldType::type_id(field) == B::TYPE_ID && bytes.len() == B::WIDTH =>
            {
                // The bytes are the raw coefficient at this column's scale (matching-schema append).
                DecimalSerie::append_coeff_bytes(self, bytes);
                Ok(())
            }
            other => Err(append_type_mismatch(B::TYPE_ID, other)),
        }
    }

    fn concat(&mut self, other: &dyn AnySerie) -> Result<(), IoError> {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => DecimalSerie::concat(self, other).map_err(to_io),
            None => Err(concat_type_mismatch(B::TYPE_ID, other)),
        }
    }

    fn set_cell(&mut self, index: usize, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => DecimalSerie::set(self, index, None).map_err(to_io),
            AnyScalar::Leaf { field, bytes }
                if FieldType::type_id(field) == B::TYPE_ID && bytes.len() == B::WIDTH =>
            {
                // The bytes are the raw coefficient at this column's scale (matching-schema set).
                DecimalSerie::set_coeff_bytes(self, index, bytes)
            }
            other => Err(set_cell_type_mismatch(B::TYPE_ID, other)),
        }
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        DecimalSerie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        Ok(std::sync::Arc::new(DecimalSerie::to_arrow_array(self)))
    }

    eq_via_downcast!();
}

impl<E: VarElement> AnySerie for ByteSerie<E> {
    fn len(&self) -> usize {
        ByteSerie::len(self)
    }

    fn null_count(&self) -> usize {
        ByteSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        E::TYPE_ID
    }

    any_serie_field_forwarding!();

    fn field(&self, name: &str) -> AnyField {
        self.field().with_name(name)
    }

    fn value(&self, index: usize) -> AnyScalar {
        match self.get_bytes(index) {
            Some(bytes) => AnyScalar::leaf(
                Field::of("", E::TYPE_ID, OFFSET_WIDTH, false),
                bytes.to_vec(),
            ),
            None => AnyScalar::Null,
        }
    }

    fn cell_eq(&self, index: usize, probe: &AnyScalar) -> bool {
        // The cell's bytes are a borrow (`get_bytes`), so the compare against the probe allocates
        // nothing.
        match self.get_bytes(index) {
            Some(cell) => bare_leaf_bytes(probe, E::TYPE_ID, OFFSET_WIDTH) == Some(cell),
            None => probe.is_null(),
        }
    }

    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie> {
        let (start, count) = clamp_range(ByteSerie::len(self), offset, len);
        let values: Vec<Option<&[u8]>> = (start..start + count)
            .map(|index| self.get_bytes(index))
            .collect();
        Box::new(
            ByteSerie::<E>::from_options(&values)
                .expect("a column's own values are already valid for its kind"),
        )
    }

    fn filter(&self, mask: &[bool]) -> Result<Box<dyn AnySerie>, IoError> {
        Ok(Box::new(ByteSerie::filter(self, mask)?))
    }

    fn fill_null(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        match value {
            AnyScalar::Null => Ok(Box::new(self.clone())),
            AnyScalar::Leaf { field, bytes } if FieldType::type_id(field) == E::TYPE_ID => {
                Ok(Box::new(ByteSerie::fill_null_bytes(self, bytes)?))
            }
            other => Err(fill_null_type_mismatch(E::TYPE_ID, other)),
        }
    }

    fn append_scalar(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => ByteSerie::push_bytes(self, None),
            AnyScalar::Leaf { field, bytes } if FieldType::type_id(field) == E::TYPE_ID => {
                ByteSerie::push_bytes(self, Some(bytes))
            }
            other => Err(append_type_mismatch(E::TYPE_ID, other)),
        }
    }

    fn concat(&mut self, other: &dyn AnySerie) -> Result<(), IoError> {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => ByteSerie::concat(self, other),
            None => Err(concat_type_mismatch(E::TYPE_ID, other)),
        }
    }

    fn set_cell(&mut self, index: usize, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => ByteSerie::set_bytes(self, index, None),
            AnyScalar::Leaf { field, bytes } if FieldType::type_id(field) == E::TYPE_ID => {
                ByteSerie::set_bytes(self, index, Some(bytes))
            }
            other => Err(set_cell_type_mismatch(E::TYPE_ID, other)),
        }
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        ByteSerie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        Ok(std::sync::Arc::new(ByteSerie::to_arrow_array(self)))
    }

    eq_via_downcast!();
}

impl<K: FixedElement> AnySerie for FixedSizeSerie<K> {
    fn len(&self) -> usize {
        FixedSizeSerie::len(self)
    }

    fn null_count(&self) -> usize {
        FixedSizeSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        K::TYPE_ID
    }

    any_serie_field_forwarding!();

    fn field(&self, name: &str) -> AnyField {
        self.field().with_name(name)
    }

    fn value(&self, index: usize) -> AnyScalar {
        match self.get_bytes(index) {
            Some(bytes) => AnyScalar::leaf(
                Field::of("", K::TYPE_ID, self.width(), false),
                bytes.to_vec(),
            ),
            None => AnyScalar::Null,
        }
    }

    fn cell_eq(&self, index: usize, probe: &AnyScalar) -> bool {
        // The cell's bytes are a borrow (`get_bytes`), so the compare against the probe allocates
        // nothing.
        match self.get_bytes(index) {
            Some(cell) => bare_leaf_bytes(probe, K::TYPE_ID, self.width()) == Some(cell),
            None => probe.is_null(),
        }
    }

    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie> {
        let (start, count) = clamp_range(FixedSizeSerie::len(self), offset, len);
        let values: Vec<Option<&[u8]>> = (start..start + count)
            .map(|index| self.get_bytes(index))
            .collect();
        Box::new(
            FixedSizeSerie::<K>::from_options(self.width(), &values)
                .expect("a column's own values re-fit its own width and kind exactly"),
        )
    }

    fn filter(&self, mask: &[bool]) -> Result<Box<dyn AnySerie>, IoError> {
        Ok(Box::new(FixedSizeSerie::filter(self, mask)?))
    }

    fn fill_null(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        match value {
            AnyScalar::Null => Ok(Box::new(self.clone())),
            AnyScalar::Leaf { field, bytes } if FieldType::type_id(field) == K::TYPE_ID => {
                Ok(Box::new(FixedSizeSerie::fill_null_bytes(self, bytes)?))
            }
            other => Err(fill_null_type_mismatch(K::TYPE_ID, other)),
        }
    }

    fn append_scalar(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => FixedSizeSerie::push(self, None),
            AnyScalar::Leaf { field, bytes } if FieldType::type_id(field) == K::TYPE_ID => {
                FixedSizeSerie::push(self, Some(bytes))
            }
            other => Err(append_type_mismatch(K::TYPE_ID, other)),
        }
    }

    fn concat(&mut self, other: &dyn AnySerie) -> Result<(), IoError> {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => FixedSizeSerie::concat(self, other),
            None => Err(concat_type_mismatch(K::TYPE_ID, other)),
        }
    }

    fn set_cell(&mut self, index: usize, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => FixedSizeSerie::set(self, index, None),
            AnyScalar::Leaf { field, bytes } if FieldType::type_id(field) == K::TYPE_ID => {
                FixedSizeSerie::set(self, index, Some(bytes))
            }
            other => Err(set_cell_type_mismatch(K::TYPE_ID, other)),
        }
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        FixedSizeSerie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        Ok(std::sync::Arc::new(FixedSizeSerie::to_arrow_array(self)))
    }

    eq_via_downcast!();
}

impl AnySerie for NullSerie {
    fn len(&self) -> usize {
        NullSerie::len(self)
    }

    fn null_count(&self) -> usize {
        NullSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Null
    }

    any_serie_field_forwarding!();

    fn field(&self, name: &str) -> AnyField {
        self.field().with_name(name)
    }

    fn value(&self, _index: usize) -> AnyScalar {
        AnyScalar::Null
    }

    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie> {
        let (_, count) = clamp_range(NullSerie::len(self), offset, len);
        Box::new(NullSerie::with_len(count))
    }

    fn filter(&self, mask: &[bool]) -> Result<Box<dyn AnySerie>, IoError> {
        if mask.len() != NullSerie::len(self) {
            return Err(filter_len_mismatch(mask.len(), NullSerie::len(self)));
        }
        // A null column is just its length; keeping the `true` rows keeps their (all-null) count.
        let kept = mask.iter().filter(|&&keep| keep).count();
        Ok(Box::new(NullSerie::with_len(kept)))
    }

    fn fill_null(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        match value {
            // A null column has no element type: filling with a null is the identity; a present value
            // has no room in a null column.
            AnyScalar::Null => Ok(Box::new(self.clone())),
            other => Err(fill_null_type_mismatch(DataTypeId::Null, other)),
        }
    }

    fn append_scalar(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => {
                NullSerie::push(self);
                Ok(())
            }
            other => Err(append_type_mismatch(DataTypeId::Null, other)),
        }
    }

    fn concat(&mut self, other: &dyn AnySerie) -> Result<(), IoError> {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => {
                NullSerie::concat(self, other);
                Ok(())
            }
            None => Err(concat_type_mismatch(DataTypeId::Null, other)),
        }
    }

    fn set_cell(&mut self, index: usize, value: &AnyScalar) -> Result<(), IoError> {
        // A null column holds a null at every existing slot; setting a null is a bounds-checked no-op,
        // and a present value has no room in a null column.
        if index >= NullSerie::len(self) {
            return Err(IoError::IndexOutOfBounds {
                index,
                len: NullSerie::len(self),
            });
        }
        match value {
            AnyScalar::Null => Ok(()),
            other => Err(set_cell_type_mismatch(DataTypeId::Null, other)),
        }
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        NullSerie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        Ok(std::sync::Arc::new(NullSerie::to_arrow_array(self)))
    }

    eq_via_downcast!();
}

// The temporal columns (`Date32Serie` … `Duration64Serie`) — one blanket impl over the
// concept+width marker, delegating to `TemporalSerie<B>`'s own codec / Arrow converter.
impl<B: TemporalBacking> AnySerie for TemporalSerie<B> {
    fn len(&self) -> usize {
        TemporalSerie::len(self)
    }

    fn null_count(&self) -> usize {
        TemporalSerie::null_count(self)
    }

    fn type_id(&self) -> DataTypeId {
        B::TYPE_ID
    }

    any_serie_field_forwarding!();

    fn field(&self, name: &str) -> AnyField {
        self.field().with_name(name)
    }

    fn value(&self, index: usize) -> AnyScalar {
        if index >= self.len() || self.get_count(index).is_none() {
            return AnyScalar::Null;
        }
        let field = TemporalField::<B>::new("", self.unit(), self.timezone(), false).erase();
        let bytes = self.count_bytes()[index * B::WIDTH..(index + 1) * B::WIDTH].to_vec();
        AnyScalar::leaf(field, bytes)
    }

    fn slice(&self, offset: usize, len: usize) -> Box<dyn AnySerie> {
        let (start, count) = clamp_range(TemporalSerie::len(self), offset, len);
        let values: Vec<_> = (start..start + count)
            .map(|index| self.get(index))
            .collect();
        Box::new(
            TemporalSerie::<B>::from_options(self.unit(), self.timezone(), &values)
                .expect("a column's own values re-fit its own unit exactly"),
        )
    }

    fn filter(&self, mask: &[bool]) -> Result<Box<dyn AnySerie>, IoError> {
        Ok(Box::new(TemporalSerie::filter(self, mask)?))
    }

    fn fill_null(&self, value: &AnyScalar) -> Result<Box<dyn AnySerie>, IoError> {
        match value {
            AnyScalar::Null => Ok(Box::new(self.clone())),
            AnyScalar::Leaf { field, bytes }
                if FieldType::type_id(field) == B::TYPE_ID && bytes.len() == B::WIDTH =>
            {
                // The bytes are the raw count at this column's (unit, tz) (matching-schema fill).
                Ok(Box::new(self.fill_null_count_bytes(bytes)))
            }
            other => Err(fill_null_type_mismatch(B::TYPE_ID, other)),
        }
    }

    fn append_scalar(&mut self, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => TemporalSerie::push(self, None).map_err(to_io),
            AnyScalar::Leaf { field, bytes }
                if FieldType::type_id(field) == B::TYPE_ID && bytes.len() == B::WIDTH =>
            {
                // The bytes are the raw count at this column's (unit, tz) (matching-schema append).
                TemporalSerie::append_count_bytes(self, bytes);
                Ok(())
            }
            other => Err(append_type_mismatch(B::TYPE_ID, other)),
        }
    }

    fn concat(&mut self, other: &dyn AnySerie) -> Result<(), IoError> {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => TemporalSerie::concat(self, other).map_err(to_io),
            None => Err(concat_type_mismatch(B::TYPE_ID, other)),
        }
    }

    fn set_cell(&mut self, index: usize, value: &AnyScalar) -> Result<(), IoError> {
        match value {
            AnyScalar::Null => TemporalSerie::set(self, index, None).map_err(to_io),
            AnyScalar::Leaf { field, bytes }
                if FieldType::type_id(field) == B::TYPE_ID && bytes.len() == B::WIDTH =>
            {
                // The bytes are the raw count at this column's (unit, tz) (matching-schema set).
                TemporalSerie::set_count_bytes(self, index, bytes)
            }
            other => Err(set_cell_type_mismatch(B::TYPE_ID, other)),
        }
    }

    fn write_to(&self, sink: &mut Bytes) -> Result<(), IoError> {
        TemporalSerie::write_to(self, sink)
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_array(&self) -> Result<arrow_array::ArrayRef, IoError> {
        TemporalSerie::to_arrow_array(self)
    }

    eq_via_downcast!();
}

// -------------------------------------------------------------------------------------
// Erased leaf construction — the one dispatch on `type_id` that has to name the concrete `T`
// (nested `StructSerie` extends it in the `nested` module).
// -------------------------------------------------------------------------------------

/// Reads a **leaf** column of the type named by `field` from `source` (the bytes a
/// [`Serie::write_to`](Serie) produced). Errors for a nested field — the [`nested`](crate::io::nested)
/// module's reader handles those recursively.
pub fn read_any_leaf(field: &AnyField, source: &mut Bytes) -> Result<Box<dyn AnySerie>, IoError> {
    let leaf = field.as_leaf().ok_or_else(|| IoError::Unsupported {
        what: "read_any_leaf was given a nested field; use the nested reader".to_string(),
    })?;
    Ok(match FieldType::type_id(leaf) {
        DataTypeId::Null => Box::new(NullSerie::read_from(source)?),
        DataTypeId::U8 => Box::new(Serie::<u8>::read_from(source)?),
        DataTypeId::U16 => Box::new(Serie::<u16>::read_from(source)?),
        DataTypeId::U32 => Box::new(Serie::<u32>::read_from(source)?),
        DataTypeId::U64 => Box::new(Serie::<u64>::read_from(source)?),
        DataTypeId::U96 => Box::new(Serie::<U96>::read_from(source)?),
        DataTypeId::U128 => Box::new(Serie::<u128>::read_from(source)?),
        DataTypeId::U256 => Box::new(Serie::<U256>::read_from(source)?),
        DataTypeId::I8 => Box::new(Serie::<i8>::read_from(source)?),
        DataTypeId::I16 => Box::new(Serie::<i16>::read_from(source)?),
        DataTypeId::I32 => Box::new(Serie::<i32>::read_from(source)?),
        DataTypeId::I64 => Box::new(Serie::<i64>::read_from(source)?),
        DataTypeId::I96 => Box::new(Serie::<I96>::read_from(source)?),
        DataTypeId::I128 => Box::new(Serie::<i128>::read_from(source)?),
        DataTypeId::I256 => Box::new(Serie::<I256>::read_from(source)?),
        DataTypeId::F16 => Box::new(Serie::<f16>::read_from(source)?),
        DataTypeId::F32 => Box::new(Serie::<f32>::read_from(source)?),
        DataTypeId::F64 => Box::new(Serie::<f64>::read_from(source)?),
        DataTypeId::D32 => Box::new(DecimalSerie::<Dec32>::read_from(source)?),
        DataTypeId::D64 => Box::new(DecimalSerie::<Dec64>::read_from(source)?),
        DataTypeId::D128 => Box::new(DecimalSerie::<Dec128>::read_from(source)?),
        DataTypeId::D256 => Box::new(DecimalSerie::<Dec256>::read_from(source)?),
        DataTypeId::Utf8 => Box::new(Utf8Serie::read_from(source)?),
        DataTypeId::Binary => Box::new(BinarySerie::read_from(source)?),
        DataTypeId::FixedBinary => Box::new(FixedBinarySerie::read_from(source)?),
        DataTypeId::FixedUtf8 => Box::new(FixedUtf8Serie::read_from(source)?),
        DataTypeId::Date32 => Box::new(Date32Serie::read_from(source)?),
        DataTypeId::Date64 => Box::new(Date64Serie::read_from(source)?),
        DataTypeId::Time32 => Box::new(Time32Serie::read_from(source)?),
        DataTypeId::Time64 => Box::new(Time64Serie::read_from(source)?),
        DataTypeId::Ts32 => Box::new(Ts32Serie::read_from(source)?),
        DataTypeId::Ts64 => Box::new(Ts64Serie::read_from(source)?),
        DataTypeId::Ts96 => Box::new(Ts96Serie::read_from(source)?),
        DataTypeId::Duration32 => Box::new(Duration32Serie::read_from(source)?),
        DataTypeId::Duration64 => Box::new(Duration64Serie::read_from(source)?),
        other => {
            return Err(IoError::Unsupported {
                what: format!("cannot deserialize a leaf column of type {}", other.name()),
            })
        }
    })
}

/// Builds a **leaf** erased column from an Arrow array + its [`Field`](arrow_schema::Field) (feature
/// `arrow`), delegating to the matching `Serie`'s own zero-copy `from_arrow_array`. Errors for a
/// nested or unmodeled field.
#[cfg(feature = "arrow")]
pub fn from_arrow_any_leaf(
    array: &dyn arrow_array::Array,
    field: &arrow_schema::Field,
) -> Result<Box<dyn AnySerie>, IoError> {
    let leaf = Field::from_arrow(field).ok_or_else(|| unsupported(field))?;
    Ok(match FieldType::type_id(&leaf) {
        DataTypeId::Null => Box::new(NullSerie::with_len(array.len())),
        DataTypeId::U8 => Box::new(Serie::<u8>::from_arrow_array(down::<
            arrow_array::UInt8Array,
        >(array, field)?)),
        DataTypeId::U16 => Box::new(Serie::<u16>::from_arrow_array(down::<
            arrow_array::UInt16Array,
        >(array, field)?)),
        DataTypeId::U32 => Box::new(Serie::<u32>::from_arrow_array(down::<
            arrow_array::UInt32Array,
        >(array, field)?)),
        DataTypeId::U64 => Box::new(Serie::<u64>::from_arrow_array(down::<
            arrow_array::UInt64Array,
        >(array, field)?)),
        DataTypeId::I8 => Box::new(Serie::<i8>::from_arrow_array(
            down::<arrow_array::Int8Array>(array, field)?,
        )),
        DataTypeId::I16 => Box::new(Serie::<i16>::from_arrow_array(down::<
            arrow_array::Int16Array,
        >(array, field)?)),
        DataTypeId::I32 => Box::new(Serie::<i32>::from_arrow_array(down::<
            arrow_array::Int32Array,
        >(array, field)?)),
        DataTypeId::I64 => Box::new(Serie::<i64>::from_arrow_array(down::<
            arrow_array::Int64Array,
        >(array, field)?)),
        DataTypeId::F16 => Box::new(Serie::<f16>::from_arrow_array(down::<
            arrow_array::Float16Array,
        >(array, field)?)),
        DataTypeId::F32 => Box::new(Serie::<f32>::from_arrow_array(down::<
            arrow_array::Float32Array,
        >(array, field)?)),
        DataTypeId::F64 => Box::new(Serie::<f64>::from_arrow_array(down::<
            arrow_array::Float64Array,
        >(array, field)?)),
        DataTypeId::U96 => Box::new(wide_from_arrow::<U96>(array)),
        DataTypeId::U128 => Box::new(wide_from_arrow::<u128>(array)),
        DataTypeId::U256 => Box::new(wide_from_arrow::<U256>(array)),
        DataTypeId::I96 => Box::new(wide_from_arrow::<I96>(array)),
        DataTypeId::I128 => Box::new(wide_from_arrow::<i128>(array)),
        DataTypeId::I256 => Box::new(wide_from_arrow::<I256>(array)),
        DataTypeId::D32 => Box::new(DecimalSerie::<Dec32>::from_arrow_array(down::<
            arrow_array::Decimal32Array,
        >(
            array, field
        )?)),
        DataTypeId::D64 => Box::new(DecimalSerie::<Dec64>::from_arrow_array(down::<
            arrow_array::Decimal64Array,
        >(
            array, field
        )?)),
        DataTypeId::D128 => Box::new(DecimalSerie::<Dec128>::from_arrow_array(down::<
            arrow_array::Decimal128Array,
        >(
            array, field
        )?)),
        DataTypeId::D256 => Box::new(DecimalSerie::<Dec256>::from_arrow_array(down::<
            arrow_array::Decimal256Array,
        >(
            array, field
        )?)),
        DataTypeId::Utf8 => Box::new(Utf8Serie::from_arrow_array(down::<
            arrow_array::StringArray,
        >(array, field)?)?),
        DataTypeId::Binary => Box::new(BinarySerie::from_arrow_array(down::<
            arrow_array::BinaryArray,
        >(array, field)?)?),
        DataTypeId::FixedBinary => Box::new(FixedBinarySerie::from_arrow_array(down::<
            arrow_array::FixedSizeBinaryArray,
        >(
            array, field
        )?)?),
        DataTypeId::FixedUtf8 => Box::new(FixedUtf8Serie::from_arrow_array(down::<
            arrow_array::FixedSizeBinaryArray,
        >(
            array, field
        )?)?),
        DataTypeId::Date32 => Box::new(Date32Serie::from_arrow_array(array, field)?),
        DataTypeId::Date64 => Box::new(Date64Serie::from_arrow_array(array, field)?),
        DataTypeId::Time32 => Box::new(Time32Serie::from_arrow_array(array, field)?),
        DataTypeId::Time64 => Box::new(Time64Serie::from_arrow_array(array, field)?),
        DataTypeId::Ts32 => Box::new(Ts32Serie::from_arrow_array(array, field)?),
        DataTypeId::Ts64 => Box::new(Ts64Serie::from_arrow_array(array, field)?),
        DataTypeId::Ts96 => Box::new(Ts96Serie::from_arrow_array(array, field)?),
        DataTypeId::Duration32 => Box::new(Duration32Serie::from_arrow_array(array, field)?),
        DataTypeId::Duration64 => Box::new(Duration64Serie::from_arrow_array(array, field)?),
        _ => return Err(unsupported(field)),
    })
}

/// Downcasts a `PrimitiveArray<A>` (or other concrete array) or errors.
#[cfg(feature = "arrow")]
fn down<'a, A: 'static>(
    array: &'a dyn arrow_array::Array,
    field: &arrow_schema::Field,
) -> Result<&'a A, IoError> {
    array
        .as_any()
        .downcast_ref::<A>()
        .ok_or_else(|| unsupported(field))
}

/// Rebuilds a wide (non-Arrow-native) `Serie<T>` from an imported Arrow array's flat value bytes,
/// reading its **logical** window (offset-aware) and zeroing bytes under null slots.
#[cfg(feature = "arrow")]
fn wide_from_arrow<T: NativeType>(array: &dyn arrow_array::Array) -> Serie<T> {
    let width = T::WIDTH;
    let len = array.len();
    let data = array.to_data();
    let src = data.buffers()[0].as_slice();
    let base = data.offset() * width;
    let mut values = vec![0u8; len * width];
    let mut validity = None;
    for index in 0..len {
        if array.is_null(index) {
            validity
                .get_or_insert_with(|| crate::io::bitmap::Bitmap::all_present(len))
                .set(index, false);
        } else {
            let start = base + index * width;
            values[index * width..(index + 1) * width].copy_from_slice(&src[start..start + width]);
        }
    }
    Serie::from_byte_slice(values, validity, len)
}

/// The guided "Arrow type not modeled" error for a field the crate cannot import.
#[cfg(feature = "arrow")]
fn unsupported(field: &arrow_schema::Field) -> IoError {
    IoError::Unsupported {
        what: format!(
            "Arrow field {:?} of type {:?} is not a yggdryl-modeled column type",
            field.name(),
            field.data_type()
        ),
    }
}

#[cfg(test)]
mod temporal_tests {
    use super::*;
    use crate::io::fixed::temporal::{TimeUnit, Ts64, Tz};
    use crate::io::fixed::{Ts64Kind, Ts64Serie};
    use crate::io::nested::StructSerie;

    #[test]
    fn temporal_leaf_round_trips_through_read_any_leaf_and_as_temporal() {
        let a = Ts64::from_epoch(1_700_000_000, TimeUnit::Second, Tz::UTC).unwrap();
        let col = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(a), None]).unwrap();

        // Erase, name the field, and read it back through the erased leaf reader.
        let erased = boxed(col.clone());
        let field = erased.field("t");
        let bytes = erased.serialize_bytes();
        let back = read_any_leaf(&field, &mut Bytes::from_slice(&bytes)).unwrap();
        assert!(back.eq_any(erased.as_ref()));

        // Downcast the erased column back to its concrete `TemporalSerie`, keyed on the marker.
        let recovered = erased.as_temporal::<Ts64Kind>().expect("Ts64 downcast");
        assert_eq!(*recovered, col);
        assert!(erased
            .as_temporal::<crate::io::fixed::Date32Kind>()
            .is_none());
    }

    #[test]
    fn temporal_child_round_trips_through_struct_serialize_bytes() {
        let a = Ts64::from_epoch(10, TimeUnit::Second, Tz::UTC).unwrap();
        let b = Ts64::from_epoch(20, TimeUnit::Second, Tz::UTC).unwrap();
        let col = Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &[a, b]).unwrap();
        let table = StructSerie::from_named(vec![("t", boxed(col))]).unwrap();
        let back = StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap();
        assert_eq!(back, table);
    }
}

#[cfg(test)]
mod slice_tests {
    use super::*;
    use crate::io::fixed::Serie;
    use crate::io::var::Utf8Serie;

    #[test]
    fn slice_of_a_primitive_column_is_its_sub_window() {
        // The documented example: [1,2,3,4].slice(1, 2) == [2, 3].
        let sliced = boxed(Serie::from_values(&[1i32, 2, 3, 4])).slice(1, 2);
        let expected = boxed(Serie::from_values(&[2i32, 3]));
        assert!(sliced.eq_any(expected.as_ref()));
        assert_eq!(sliced.len(), 2);
    }

    #[test]
    fn slice_clamps_out_of_range_and_preserves_nulls() {
        let column = boxed(Serie::from_options(&[
            Some(1i32),
            None,
            Some(3),
            None,
            Some(5),
        ]));
        // A window that runs past the end is clamped to the rows that remain.
        let tail = column.slice(3, 100);
        let expected = boxed(Serie::from_options(&[None, Some(5i32)]));
        assert!(tail.eq_any(expected.as_ref()));
        // A wholly out-of-range offset yields an empty column.
        assert_eq!(column.slice(9, 4).len(), 0);
    }

    #[test]
    fn slice_of_a_var_column_copies_the_window() {
        let column = boxed(Utf8Serie::from_strs(&[
            Some("a"),
            None,
            Some("cd"),
            Some("e"),
        ]));
        let sliced = column.slice(1, 2);
        let expected = boxed(Utf8Serie::from_strs(&[None, Some("cd")]));
        assert!(sliced.eq_any(expected.as_ref()));
    }
}
