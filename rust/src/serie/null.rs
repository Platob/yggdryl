//! The column a null field is stored in: a length, and no buffer at all.
//!
//! Every row of a null column is absent, so there is nothing to lay out and
//! nothing to prove: the leaf keeps a count, builds Arrow's `NullArray` on
//! demand, and its typed writer is a count too.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, NullArray};
use arrow_buffer::NullBuffer;

use super::{Serie, require_range, require_row, require_window};
use crate::value::SerieValue;
use crate::{Field, Result, Scalar};

/// One column of nulls: a length, and no buffer at all.
#[derive(Clone)]
pub struct NullSerie {
    field: Arc<Field>,
    rows: usize,
}

impl NullSerie {
    /// Pair a field with the count of rows it holds.
    pub(crate) const fn new(field: Arc<Field>, rows: usize) -> Self {
        Self { field, rows }
    }

    /// Build the Arrow array this column is: a null column is a length, so
    /// there is no buffer to lend and the array is built on demand.
    pub fn array(&self) -> NullArray {
        NullArray::new(self.rows)
    }

    /// Append one row.
    pub const fn push_value(&mut self) {
        self.rows += 1;
    }

    /// Replace rows `range` by `count` rows.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when `range` is reversed or
    /// reaches past the end.
    pub fn splice_values(&mut self, range: Range<usize>, count: usize) -> Result<()> {
        require_range(self.field.name(), &range, self.rows)?;
        self.rows = self.rows - range.len() + count;
        Ok(())
    }

    /// Refuse what a write could not do: nothing, for a count.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let _ = (range, rows);
        Ok(())
    }

    /// Write canonical `rows` over a checked `range`: every row is absent,
    /// so only the count moves.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        self.rows = self.rows - range.len() + rows.len();
    }

    /// Append `other`'s rows, whose field agrees with this one's; a count
    /// reaches any total.
    pub(crate) const fn append(&mut self, other: &Self) -> bool {
        self.rows += other.rows;
        true
    }
}

impl SerieValue for NullSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.rows
    }

    fn null_count(&self) -> usize {
        self.rows
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.rows)?;
        Ok(true)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.rows)?;
        Ok(Scalar::Null)
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.rows)?;
        Ok(Self {
            field: Arc::clone(&self.field),
            rows: length,
        })
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.rows)?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.array())
    }

    fn into_serie(self) -> Serie {
        Serie::Null(Arc::new(self))
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Null(column) => Some(column.as_ref()),
            _ => None,
        }
    }
}

impl fmt::Debug for NullSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, "NullSerie", formatter)
    }
}

serie_leaf!(NullSerie);

/// Build the column `field` types out of a null array, or answer `None` for
/// a layout that is not one.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proven: bool,
) -> crate::arrow::Result<Option<Serie>> {
    let _ = (parent, proven);
    if !matches!(array.data_type(), arrow_schema::DataType::Null) {
        return Ok(None);
    }
    Ok(Some(NullSerie::new(field, array.len()).into_serie()))
}
