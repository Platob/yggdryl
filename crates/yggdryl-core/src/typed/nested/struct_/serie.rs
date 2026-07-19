//! [`StructSerie`] — the **struct "table"**: an ordered set of heterogeneous, equal-length child
//! [`Column`]s under one name + metadata, with an optional row-level validity buffer. It is the
//! project's centralized table-like holder and the first nested carrier.
//!
//! It implements [`Scalar`] / [`Serie`] (its element is a [`StructScalar`] row), so a struct is
//! itself a column and nests inside another struct — navigation flows **downward** through
//! [`column_path`](StructSerie::column_path) into inner children, and a
//! [`column_by_name_mut`](StructSerie::column_by_name_mut) hands back a `&mut Column` so a caller
//! **deep-mutates an inner series in place, no copy** (matching the public [`Column`] variant to
//! recover the concrete series).
//!
//! ```
//! use yggdryl_core::typed::fixedbyte::Int64;
//! use yggdryl_core::typed::varbyte::Utf8;
//! use yggdryl_core::typed::{Column, FixedSerie, StructSerie, Value, VarSerie};
//!
//! let id = FixedSerie::<Int64>::from_values(&[1, 2, 3]).with_name("id");
//! let name = VarSerie::<Utf8>::from_values(&["ada".into(), "bo".into(), "cy".into()])
//!     .with_name("name");
//! let table = StructSerie::from_columns(vec![Column::from(id), Column::from(name)]).unwrap();
//!
//! assert_eq!(table.num_columns(), 2);
//! assert_eq!(table.len(), 3);
//! let row = table.row(1).unwrap();
//! assert_eq!(row.get_by_name("id"), Some(&Value::Int64(2)));
//! assert_eq!(row.get_by_name("name"), Some(&Value::Utf8("bo".into())));
//! ```

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::io::memory::{Heap, IOBase, IoError};
use crate::typed::nested::{Column, ColumnField, StructField, StructScalar};
use crate::typed::{Scalar, Serie};

/// The guided [`IoError`] for a struct whose child columns are not all the same length — names the
/// offending child `index`, its length, the struct's length, and the concrete fix. Shared by
/// [`from_columns`](StructSerie::from_columns) and [`with_column`](StructSerie::with_column) so both
/// messages read identically.
fn length_mismatch_error(index: usize, len: usize, expected: usize) -> IoError {
    IoError::TypedCast {
        detail: format!(
            "struct child column {index} has {len} rows but the struct has {expected}: every child \
             must share the struct's length — build the column to {expected} rows (pad or truncate) \
             before combining"
        ),
    }
}

/// A **struct column** — the table. Its `children` are equal-length erased [`Column`]s; `validity`,
/// when present, is the row-level null bitmap (LSB-first, `1` = valid); `len` is the shared row count.
pub struct StructSerie {
    children: Vec<Column>,
    validity: Option<Heap>,
    len: usize,
    name: Option<Box<str>>,
    metadata: Headers,
}

impl StructSerie {
    /// An empty, non-nullable struct named `name` (no children yet — add them with
    /// [`with_column`](StructSerie::with_column)).
    pub fn new(name: &str) -> Self {
        StructSerie {
            children: Vec::new(),
            validity: None,
            len: 0,
            name: Some(name.into()),
            metadata: Headers::new(),
        }
    }

    /// A struct from `children` — **errors** (guided) if the columns are not all the same length. The
    /// struct's row count is the shared child length; it is non-nullable (no null rows) until a
    /// [`push_null`](StructSerie::push_null). The struct is unnamed; name it with
    /// [`with_name`](StructSerie::with_name).
    pub fn from_columns(children: Vec<Column>) -> Result<Self, IoError> {
        let len = children.first().map_or(0, Column::len);
        for (index, child) in children.iter().enumerate() {
            if child.len() != len {
                return Err(length_mismatch_error(index, child.len(), len));
            }
        }
        Ok(StructSerie {
            children,
            validity: None,
            len,
            name: None,
            metadata: Headers::new(),
        })
    }

    /// Sets the struct **name**, chainable.
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Replaces the **row-level validity** buffer (`Some` bitmap — LSB-first, `1` = valid — or `None`
    /// for an all-valid, non-nullable struct), chainable. The counterpart of the offsets-plus-validity
    /// constructors on [`ListSerie`](super::super::ListSerie) /
    /// [`MapSerie`](super::super::MapSerie): the front door for restoring a struct's null rows when
    /// rebuilding it from external buffers (e.g. an Arrow `StructArray`'s null buffer). The caller is
    /// responsible for sizing the bitmap to the struct's row count.
    pub fn with_validity(mut self, validity: Option<Heap>) -> Self {
        self.validity = validity;
        self
    }

    /// Appends a child `column`, chainable — the first column fixes the struct's row count; a later
    /// column of a different length is the guided [`length_mismatch_error`].
    ///
    // DESIGN: the child's **name** rides on the column's own field (set on the underlying series with
    // `with_name`), so `with_column` takes only the `Column` — an erased column cannot be renamed in
    // place (its name lives in the concrete carrier), so a separate `name` argument would be a no-op.
    pub fn with_column(mut self, column: Column) -> Result<Self, IoError> {
        if self.children.is_empty() {
            self.len = column.len();
        } else if column.len() != self.len {
            return Err(length_mismatch_error(
                self.children.len(),
                column.len(),
                self.len,
            ));
        }
        self.children.push(column);
        Ok(self)
    }

    /// Ensures a row-level validity buffer exists, back-filling every existing row as valid (created
    /// lazily on the first null row).
    fn ensure_validity(&mut self) {
        if self.validity.is_none() {
            let mut validity = Heap::new();
            for index in 0..self.len as u64 {
                validity
                    .pwrite_bit(index, true)
                    .expect("bit write into a fresh heap never fails");
            }
            self.validity = Some(validity);
        }
    }

    /// Appends a **null row** — grows every child by one null slot and clears the new row's validity
    /// bit (back-filling the validity buffer on the first null).
    pub fn push_null(&mut self) {
        self.ensure_validity();
        for child in &mut self.children {
            child.push_null();
        }
        self.validity
            .as_mut()
            .expect("validity ensured")
            .pwrite_bit(self.len as u64, false)
            .expect("bit write into a heap never fails");
        self.len += 1;
    }

    // ---- graph discovery ----------------------------------------------------------------

    /// The number of child columns.
    pub fn num_columns(&self) -> usize {
        self.children.len()
    }

    /// The child column at `index`, if present.
    pub fn column(&self, index: usize) -> Option<&Column> {
        self.children.get(index)
    }

    /// The first child column named `name`, if any.
    pub fn column_by_name(&self, name: &str) -> Option<&Column> {
        self.children
            .iter()
            .find(|child| child.name() == Some(name))
    }

    /// The child columns (borrowed).
    pub fn columns(&self) -> &[Column] {
        &self.children
    }

    /// The child column at `index`, **mutable** — the front door to a deep, in-place edit: match the
    /// returned `&mut Column`'s public variant to recover the concrete series and mutate it (e.g.
    /// `if let Column::Int64(serie) = column { serie.set(0, 999)?; }`).
    pub fn column_mut(&mut self, index: usize) -> Option<&mut Column> {
        self.children.get_mut(index)
    }

    /// The first child column named `name`, **mutable** — see [`column_mut`](StructSerie::column_mut).
    pub fn column_by_name_mut(&mut self, name: &str) -> Option<&mut Column> {
        self.children
            .iter_mut()
            .find(|child| child.name() == Some(name))
    }

    /// Descends a **dotted path** (`"address.city"`) into nested struct children, returning the
    /// addressed column. Each segment but the last must name a child that is itself a
    /// [`Column::Struct`]; a missing segment or a non-struct mid-path yields `None`.
    pub fn column_path(&self, path: &str) -> Option<&Column> {
        match path.split_once('.') {
            None => self.column_by_name(path),
            Some((head, rest)) => match self.column_by_name(head)? {
                Column::Struct(inner) => inner.column_path(rest),
                _ => None,
            },
        }
    }

    /// The **mutable** twin of [`column_path`](StructSerie::column_path): descends the dotted `path`
    /// into nested struct children and returns a `&mut Column` at the leaf — so a caller
    /// **deep-mutates an inner series in place, no copy** (match the returned variant to recover the
    /// concrete series).
    pub fn column_path_mut(&mut self, path: &str) -> Option<&mut Column> {
        match path.split_once('.') {
            None => self.column_by_name_mut(path),
            Some((head, rest)) => match self.column_by_name_mut(head)? {
                Column::Struct(inner) => inner.column_path_mut(rest),
                _ => None,
            },
        }
    }

    // ---- rows ---------------------------------------------------------------------------

    /// The **row** at `index` — a [`StructScalar`] of each child's element (erased to a
    /// [`Value`](crate::typed::Value)) with its name, and the row-level validity — or `None` when
    /// `index` is out of range.
    pub fn row(&self, index: usize) -> Option<StructScalar> {
        if index >= self.len {
            return None;
        }
        let mut names = Vec::with_capacity(self.children.len());
        let mut values = Vec::with_capacity(self.children.len());
        for child in &self.children {
            names.push(child.name().unwrap_or("").into());
            values.push(child.get(index));
        }
        Some(StructScalar::new(names, values, self.is_valid(index)))
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the struct has no rows.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the **row** at `index` is valid (non-null). Out of range is `false`.
    pub fn is_valid(&self, index: usize) -> bool {
        index < self.len
            && self
                .validity
                .as_ref()
                .is_none_or(|bits| bits.pread_bit(index as u64).unwrap_or(false))
    }

    /// How many rows are null.
    pub fn null_count(&self) -> usize {
        match &self.validity {
            None => 0,
            Some(bits) => (0..self.len)
                .filter(|&index| !bits.pread_bit(index as u64).unwrap_or(false))
                .count(),
        }
    }

    /// The struct's name, if set.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The free-form metadata map (borrowed).
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// The free-form metadata map (mutable).
    pub fn metadata_mut(&mut self) -> &mut Headers {
        &mut self.metadata
    }

    /// The struct's [`StructField`] schema — derived from the children's fields plus the struct's
    /// name, nullability (whether a validity buffer is present), and metadata.
    pub fn field(&self) -> StructField {
        let children: Vec<ColumnField> = self.children.iter().map(Column::field).collect();
        let mut field = StructField::new(self.name.as_deref(), children);
        field.set_nullable(self.validity.is_some());
        *field.metadata_mut() = self.metadata.clone();
        field
    }
}

// The typed-layer traits — a struct is itself a column whose element is a row. The `Scalar` methods
// delegate to the inherent implementations (fully-qualified so the path resolves to the inherent
// method, never back to the trait — no recursion).
impl Scalar for StructSerie {
    type Value = StructScalar;

    fn data_type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }

    fn len(&self) -> usize {
        StructSerie::len(self)
    }

    fn is_valid(&self, index: usize) -> bool {
        StructSerie::is_valid(self, index)
    }

    fn null_count(&self) -> usize {
        StructSerie::null_count(self)
    }

    fn get(&self, index: usize) -> Option<StructScalar> {
        self.row(index)
    }
}

impl Serie for StructSerie {
    // DESIGN: nested owns its children **downward** — `children()` is the only graph edge; there is
    // no `parent()` up-pointer (a child column does not know the struct that contains it).
    fn children(&self) -> Vec<&Column> {
        self.children.iter().collect()
    }
}
