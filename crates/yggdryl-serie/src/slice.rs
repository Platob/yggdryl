//! [`SliceSerie`] — a zero-copy **child** view of another column, plus the
//! [`child`] / [`child_range`] constructors that build the parent→child graph. A
//! child delegates its data to the slice and remembers the serie it came from
//! ([`parent`](crate::Serie::parent)); [`materialize`](crate::Serie::materialize)
//! detaches it into an independent column.

use std::any::Any;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::ArrayRef;
use yggdryl_schema::Field;

use crate::scalar::Scalar;
use crate::serie::{Serie, SerieRef};

/// A zero-copy slice of a parent column that keeps a link back to it. Built by
/// [`child`] / [`child_range`]; it *is* a [`Serie`], delegating every data accessor to
/// the underlying slice while exposing the [`parent`](Serie::parent).
#[derive(Debug, Clone)]
pub struct SliceSerie {
    inner: SerieRef,
    parent: SerieRef,
}

impl SliceSerie {
    /// Slices `parent` into a child of `length` rows starting at `offset`.
    pub fn new(parent: SerieRef, offset: usize, length: usize) -> SliceSerie {
        let inner = parent.slice(offset, length);
        SliceSerie { inner, parent }
    }

    /// The underlying (parent-less) slice this child wraps.
    pub fn inner(&self) -> &SerieRef {
        &self.inner
    }
}

impl Serie for SliceSerie {
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

    fn parent(&self) -> Option<&SerieRef> {
        Some(&self.parent)
    }

    fn is_materialized(&self) -> bool {
        self.inner.is_materialized()
    }

    fn value_at(&self, index: usize) -> Scalar {
        self.inner.value_at(index)
    }

    /// Detaches the slice from its parent, returning an independent column.
    fn materialize(&self) -> SerieRef {
        self.inner.materialize()
    }

    /// A (parent-less) sub-slice of the underlying data.
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        self.inner.slice(offset, length)
    }
}

/// A zero-copy [`child`](SliceSerie) slice of `parent` — `length` rows from `offset` —
/// linked back to `parent` via [`parent`](Serie::parent).
///
/// ```
/// use yggdryl_serie::{from_array, child, Serie};
/// use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
/// use std::sync::Arc;
///
/// let parent = from_array("n", Arc::new(Int32Array::from(vec![10, 20, 30, 40])) as ArrayRef)?;
/// let view = child(&parent, 1, 2);
/// assert_eq!(view.len(), 2);
/// assert!(view.parent().is_some());
/// # Ok::<(), yggdryl_serie::SerieError>(())
/// ```
pub fn child(parent: &SerieRef, offset: usize, length: usize) -> SerieRef {
    Arc::new(SliceSerie::new(parent.clone(), offset, length))
}

/// A zero-copy [`child`] slice of `parent` addressed by a half-open row `range`.
pub fn child_range(parent: &SerieRef, range: Range<usize>) -> SerieRef {
    child(parent, range.start, range.len())
}
