//! [`IndexSerie`] — a column used as a **row index**: the labels that address a
//! frame's rows. It wraps any backing [`Serie`] and adds index operations
//! (label ↔ position lookup), **defaulting to a lazy `uint64` range index** `[0, 1, …,
//! len-1]` (a [`RangeSerie`]) — the implicit index a frame carries when no explicit one
//! is set, computed on demand rather than stored.

use std::any::Any;
use std::sync::Arc;

use arrow_array::ArrayRef;
use yggdryl_schema::Field;

use crate::error::SerieResult;
use crate::lazy::RangeSerie;
use crate::scalar::Scalar;
use crate::serie::{Serie, SerieRef};

/// A row index — a [`Serie`] of labels, defaulting to a lazy monotonic `uint64` range.
///
/// `IndexSerie` is itself a [`Serie`] (an index *is* a column), so it slots anywhere a
/// column is expected; it delegates the base accessors to its backing serie and adds
/// the index-specific [`at`](IndexSerie::at) (label at a row) and
/// [`position`](IndexSerie::position) (row of a label) lookups. The default
/// [`range`](IndexSerie::range) index is a lazy `uint64` [`RangeSerie`].
///
/// ```
/// use yggdryl_serie::{IndexSerie, DataType, Serie};
///
/// let index = IndexSerie::range(4);                 // lazy [0, 1, 2, 3] as uint64
/// assert_eq!(index.len(), 4);
/// assert!(index.is_range());
/// assert!(!index.is_materialized());                // computed on demand
/// assert_eq!(index.data_type(), &DataType::int(64, false));
/// assert_eq!(index.at(2), Some(2));                 // label at row 2
/// assert_eq!(index.position(3), Some(3));           // row of label 3
/// assert!(!index.contains(4));
/// ```
#[derive(Debug, Clone)]
pub struct IndexSerie {
    inner: SerieRef,
    /// Whether this is the implicit `0..len` range index (enables the O(1) lookups).
    range: bool,
}

impl IndexSerie {
    /// The default index over `len` rows: a **lazy** monotonic `uint64` range `[0, 1,
    /// …, len-1]`, named `"index"`.
    pub fn range(len: usize) -> IndexSerie {
        IndexSerie {
            inner: Arc::new(RangeSerie::indices(len)),
            range: true,
        }
    }

    /// Wraps an existing column as an index (not a range index — its labels are
    /// whatever the column holds).
    pub fn from_serie(serie: SerieRef) -> IndexSerie {
        IndexSerie {
            inner: serie,
            range: false,
        }
    }

    /// Builds an index from a [`Field`] and an Arrow array (via the
    /// [factory](crate::from_arrow)).
    pub fn from_arrow(field: Field, array: ArrayRef) -> SerieResult<IndexSerie> {
        Ok(IndexSerie::from_serie(crate::from_arrow(field, array)?))
    }

    /// Builds an index named `name` from an Arrow array (via the
    /// [factory](crate::from_array)).
    pub fn from_array(name: impl Into<String>, array: ArrayRef) -> SerieResult<IndexSerie> {
        Ok(IndexSerie::from_serie(crate::from_array(name, array)?))
    }

    /// Whether this is the implicit lazy `0..len` `uint64` range index.
    pub fn is_range(&self) -> bool {
        self.range
    }

    /// The backing column.
    pub fn inner(&self) -> &SerieRef {
        &self.inner
    }

    /// Consumes the index, returning the backing column.
    pub fn into_inner(self) -> SerieRef {
        self.inner
    }

    /// The integer label at row `index`, when the index is integer-valued (the
    /// default), or `None` when out of bounds, null, or non-integer.
    pub fn at(&self, index: usize) -> Option<u64> {
        if index >= self.len() {
            return None;
        }
        match self.value_at(index) {
            Scalar::Int(value) => u64::try_from(value).ok(),
            _ => None,
        }
    }

    /// The first row whose label equals `value`, or `None`. O(1) for the default range
    /// index; a linear scan otherwise; `None` for a non-integer index.
    pub fn position(&self, value: u64) -> Option<usize> {
        if self.range {
            return (value < self.len() as u64).then_some(value as usize);
        }
        (0..self.len()).find(|&i| self.at(i) == Some(value))
    }

    /// Whether `value` is one of the index labels.
    pub fn contains(&self, value: u64) -> bool {
        self.position(value).is_some()
    }
}

impl Default for IndexSerie {
    /// The empty default (lazy `uint64` range) index.
    fn default() -> IndexSerie {
        IndexSerie::range(0)
    }
}

impl Serie for IndexSerie {
    fn field(&self) -> &Field {
        self.inner.field()
    }

    fn array(&self) -> ArrayRef {
        self.inner.array()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        self.inner.is_null(index)
    }

    fn is_materialized(&self) -> bool {
        self.inner.is_materialized()
    }

    fn value_at(&self, index: usize) -> Scalar {
        self.inner.value_at(index)
    }

    /// Materialises the (lazy) index into an in-memory one, preserving its range
    /// fast-path flag.
    fn materialize(&self) -> SerieRef {
        Arc::new(IndexSerie {
            inner: self.inner.materialize(),
            range: self.range,
        })
    }

    /// A slice of the index — still an [`IndexSerie`], but no longer the implicit
    /// range (its labels start at `offset`).
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        Arc::new(IndexSerie {
            inner: self.inner.slice(offset, length),
            range: false,
        })
    }
}
