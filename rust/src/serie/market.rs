//! The column of a registered market kind: [`MarketSerie`], one variant per
//! storage a kind can declare, each the leaf of that storage under the
//! kind's field.
//!
//! The storage leaf already reads its cells as the kind's values: the
//! landing resolved its reading from the field once - an enum's codes
//! through the member reading - and the kind rides in the field's datatype,
//! so no cell reads the register. This column therefore holds nothing of its
//! own and forwards every verb to that leaf, so a row read here and one read
//! through [`Serie::as_uint8`] or [`Serie::as_uint16`] cannot disagree; what
//! it adds is its place in the root, so a slice, a clone and an append stay
//! one market column.

use std::ops::Range;
use std::sync::Arc;

use arrow_array::ArrayRef;

use super::{Leaf, Serie};
use crate::value::SerieValue;
use crate::{Field, MarketSerie, Result, Scalar};

/// Run one verb on the storage leaf holding the rows.
macro_rules! storage {
    ($self:expr, $column:ident => $answer:expr) => {
        match $self {
            MarketSerie::Code8($column) => $answer,
            MarketSerie::Code16($column) => $answer,
        }
    };
}

impl MarketSerie {
    /// Append `other`'s buffers where both hold one storage and it reaches
    /// the total, answering whether they did; `false` leaves this column as
    /// it was. The caller has agreed the fields, which is what keeps one
    /// kind's column from taking another kind's codes.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        match (self, other) {
            (Self::Code8(mine), Self::Code8(theirs)) => Arc::make_mut(mine).append(theirs),
            (Self::Code16(mine), Self::Code16(theirs)) => Arc::make_mut(mine).append(theirs),
            _ => false,
        }
    }
}

impl SerieValue for MarketSerie {
    fn field(&self) -> &Field {
        storage!(self, column => SerieValue::field(column.as_ref()))
    }

    fn field_ref(&self) -> &Arc<Field> {
        storage!(self, column => SerieValue::field_ref(column.as_ref()))
    }

    fn len(&self) -> usize {
        storage!(self, column => SerieValue::len(column.as_ref()))
    }

    fn null_count(&self) -> usize {
        storage!(self, column => SerieValue::null_count(column.as_ref()))
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        storage!(self, column => SerieValue::is_null(column.as_ref(), index))
    }

    /// The cell read through the reading its storage leaf resolved from the
    /// kind's field where it landed: an enum's code as its member, which
    /// allocates nothing.
    fn scalar(&self, index: usize) -> Result<Scalar> {
        storage!(self, column => SerieValue::scalar(column.as_ref(), index))
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        Ok(match self {
            Self::Code8(column) => Self::Code8(Arc::new(column.slice(offset, length)?)),
            Self::Code16(column) => Self::Code16(Arc::new(column.slice(offset, length)?)),
        })
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        storage!(self, column => SerieValue::splice(Arc::make_mut(column), range, rows))
    }

    fn into_arrow_array(&self) -> ArrayRef {
        storage!(self, column => SerieValue::into_arrow_array(column.as_ref()))
    }

    fn memory_size(&self) -> usize {
        storage!(self, column => SerieValue::memory_size(column.as_ref()))
    }

    fn resident_size(&self) -> usize {
        storage!(self, column => SerieValue::resident_size(column.as_ref()))
    }

    fn into_serie(self) -> Serie {
        Leaf::root(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        Leaf::narrow_laid(value)
    }
}

impl Leaf for MarketSerie {
    fn root(self) -> Serie {
        Serie::Market(self)
    }

    fn narrow(serie: &Serie) -> Option<&Self> {
        match serie {
            Serie::Market(held) => Some(held),
            _ => None,
        }
    }

    /// The storage leaf is copied once by the write that reaches it, so
    /// narrowing lends the column as it stands.
    fn narrow_mut(serie: &mut Serie) -> Option<&mut Self> {
        match serie {
            Serie::Market(held) => Some(held),
            _ => None,
        }
    }
}

serie_leaf!(MarketSerie);
