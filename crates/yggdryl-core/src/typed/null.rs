//! [`NullSerie`] — a **first-class all-null column**: `len` null elements with **no data buffer**.
//!
//! Unlike a nullable [`FixedSerie`](crate::typed::FixedSerie) (which stores real values plus a
//! validity bitmap), a `NullSerie` carries **no bytes at all** — every element is null by
//! construction, so it is the cheapest possible column of `n` nulls. Its element type is the typed
//! [`DataTypeId::Null`] (distinct from [`Unknown`](crate::datatype_id::DataTypeId::Unknown), which
//! is *raw bytes*). It is the typed counterpart of the erased [`Column::Null`](crate::typed::Column)
//! arm: [`Column::from(null_serie)`](crate::typed::Column) erases into `Column::Null(len)`, and that
//! column's [`get`](crate::typed::Column::get) yields [`Value::Null`](crate::typed::Value::Null).
//!
//! ```
//! use yggdryl_core::typed::{NullSerie, Scalar, Column, Value};
//!
//! let nulls = NullSerie::new(3);
//! assert_eq!(nulls.len(), 3);
//! assert_eq!(nulls.null_count(), 3);
//! assert_eq!(nulls.get(0), None);          // every slot is null
//! assert!(!nulls.is_valid(0));
//!
//! let column = Column::from(nulls);         // erases to the bufferless Column::Null(3)
//! assert_eq!(column.get(0), Value::Null);
//! ```

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::typed::{HeaderField, Scalar, Serie};

/// An **all-null column** of `len` elements with no backing data buffer — the cheapest column of `n`
/// nulls. Carries only its length plus the optional column `name` / free-form metadata; every
/// element is null ([`is_valid`](Scalar::is_valid) is always `false`, [`get`](Scalar::get) always
/// `None`). Its [`data_type_id`](Scalar::data_type_id) is the typed [`DataTypeId::Null`].
#[derive(Clone, Debug, Default)]
pub struct NullSerie {
    len: usize,
    name: Option<Box<str>>,
    /// Free-form field annotations carried onto the [`field`](NullSerie::field). Empty for a plain
    /// null column (an empty [`Headers`] allocates nothing).
    metadata: Headers,
}

impl NullSerie {
    /// An all-null column of `len` elements.
    pub fn new(len: usize) -> Self {
        NullSerie {
            len,
            name: None,
            metadata: Headers::new(),
        }
    }

    /// Sets the number of null elements — chainable (the null run has no buffer, so this is a plain
    /// length assignment).
    pub fn with_len(mut self, len: usize) -> Self {
        self.len = len;
        self
    }

    /// Sets the column **name** (reported by [`field`](NullSerie::field)) — chainable.
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Appends one **null** element (growing the column by one row) — the only growth a null column
    /// admits, since it holds no values.
    pub fn push_null(&mut self) {
        self.len += 1;
    }

    /// The column **name**, if set — the lightweight accessor (the same value
    /// [`field`](NullSerie::field) reports), read without building a [`HeaderField`].
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The mutable free-form metadata map — annotate the column with any extra headers.
    pub fn metadata_mut(&mut self) -> &mut Headers {
        &mut self.metadata
    }

    /// The column's [`Field`](crate::typed::Field) metadata — its `name`, the [`DataTypeId::Null`]
    /// element type, `nullable = true` (an all-null column is inherently nullable), and any
    /// free-form annotations.
    pub fn field(&self) -> HeaderField {
        let mut field = HeaderField::new(self.name.as_deref(), DataTypeId::Null, true);
        for (name, value) in self.metadata.iter() {
            field.metadata_mut().append_bytes(name, value);
        }
        field
    }
}

impl Scalar for NullSerie {
    /// The element of an all-null column carries no payload — the unit type, always read as `None`.
    type Value = ();

    fn data_type_id(&self) -> DataTypeId {
        DataTypeId::Null
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_valid(&self, _index: usize) -> bool {
        false
    }

    /// Every element is null — `null_count == len` in O(1) (overrides the counting default).
    fn null_count(&self) -> usize {
        self.len
    }

    fn get(&self, _index: usize) -> Option<()> {
        None
    }
}

impl Serie for NullSerie {}
