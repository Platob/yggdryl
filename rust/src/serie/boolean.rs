//! The column a boolean field is stored in: a bitmap of values beside a
//! bitmap of validity.
//!
//! A boolean is fixed width but not a values buffer, so it is not a
//! [`PrimitiveSerie`](crate::PrimitiveSerie): Arrow hands no builder back
//! for a bitmap, and the two bitmaps are written through
//! `BooleanBufferBuilder` instead - in place where the column holds them
//! alone, copied once where it does not. The native domain is the whole
//! domain, so a native `bool` is a writer here, validating nullability alone.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray};
use arrow_buffer::{BooleanBuffer, NullBuffer};

use super::{Serie, layout, require_range, require_row, require_window};
use crate::value::SerieValue;
use crate::{Field, Result, Scalar};

/// One column of booleans: a bitmap of values beside a bitmap of validity.
#[derive(Clone)]
pub struct BooleanSerie {
    field: Arc<Field>,
    values: BooleanArray,
}

impl BooleanSerie {
    /// Pair a field with the bitmaps that hold its rows.
    pub(crate) const fn new(field: Arc<Field>, values: BooleanArray) -> Self {
        Self { field, values }
    }

    /// Borrow the values bitmap, without copying it.
    pub fn values(&self) -> &BooleanBuffer {
        self.values.values()
    }

    /// Read row `index` off the values bitmap: `None` when the row is absent
    /// or past the end.
    pub fn value(&self, index: usize) -> Option<bool> {
        (index < self.values.len() && self.values.is_valid(index)).then(|| self.values.value(index))
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.values.nulls()
    }

    /// Borrow the Arrow array these bitmaps are.
    pub const fn array(&self) -> &BooleanArray {
        &self.values
    }

    /// Refuse an absent native row under a field that admits none, naming
    /// the column.
    fn require_present(&self, values: &[Option<bool>]) -> Result<()> {
        if self.field.is_nullable() {
            return Ok(());
        }
        match values.iter().position(Option::is_none) {
            None => Ok(()),
            Some(at) => Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "{} is not null, got an absent value at {at} of the {} written",
                    self.field.name(),
                    values.len()
                ),
            }),
        }
    }

    /// Append one native row, without building a value for it.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when the row is absent and the
    /// field admits no absent row.
    pub fn push_value(&mut self, value: Option<bool>) -> Result<()> {
        let len = self.values.len();
        self.splice_values(len..len, vec![value])
    }

    /// Overwrite row `index` with one native value.
    ///
    /// A present row overwritten by a present value is one bit write; a row
    /// that gains or loses its absence rebuilds the validity bitmap.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when `index` is past the end, or
    /// when the row is absent and the field admits no absent row.
    pub fn set_value(&mut self, index: usize, value: Option<bool>) -> Result<()> {
        require_row(self.field.name(), index, self.values.len())?;
        self.splice_values(index..index + 1, vec![value])
    }

    /// Replace rows `range` by native `values`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when `range` is reversed or
    /// reaches past the end, or when a value is absent and the field admits
    /// no absent row.
    pub fn splice_values(&mut self, range: Range<usize>, values: Vec<Option<bool>>) -> Result<()> {
        require_range(self.field.name(), &range, self.values.len())?;
        self.require_present(&values)?;
        self.write_array(range, BooleanArray::from(values));
        Ok(())
    }

    /// Replace rows `range` by `replacement`: the two bitmaps spliced, each
    /// in place where this column holds it alone.
    fn write_array(&mut self, range: Range<usize>, replacement: BooleanArray) {
        let len = self.values.len();
        let present: Vec<bool> = (0..replacement.len())
            .map(|row| replacement.is_valid(row))
            .collect();
        let (bits, nulls) =
            std::mem::replace(&mut self.values, BooleanArray::new_null(0)).into_parts();
        let bits = layout::splice_bits(bits, len, range.clone(), replacement.values());
        let nulls = layout::splice_nulls(nulls, len, range, &present);
        self.values = BooleanArray::new(bits, nulls);
    }

    /// Refuse what a write could not do: nothing, for a bitmap.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let _ = (range, rows);
        Ok(())
    }

    /// Write canonical `rows` over a checked `range`.
    ///
    /// A canonical row is [`Scalar::Null`] or a boolean, so the bits are
    /// read straight off the rows.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let replacement: BooleanArray = rows.iter().map(Scalar::as_bool).collect();
        self.write_array(range, replacement);
    }

    /// Append `other`'s bitmaps, whose field agrees with this one's; a
    /// bitmap reaches any total.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let len = self.values.len();
        self.write_array(len..len, other.values.clone());
        true
    }
}

impl SerieValue for BooleanSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn null_count(&self) -> usize {
        self.values.null_count()
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(self.values.is_null(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(match self.value(index) {
            Some(value) => Scalar::from(value),
            None => Scalar::Null,
        })
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.values.len())?;
        Ok(Self {
            field: Arc::clone(&self.field),
            values: self.values.slice(offset, length),
        })
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.values.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.values.clone())
    }

    fn into_serie(self) -> Serie {
        super::Leaf::root(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        super::Leaf::narrow(value)
    }
}

impl fmt::Debug for BooleanSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, "BooleanSerie", formatter)
    }
}

serie_leaf!(BooleanSerie);

/// Build the column `field` types out of a boolean array, or answer `None`
/// for a layout that is not one.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proven: bool,
) -> crate::arrow::Result<Option<Serie>> {
    let _ = (parent, proven);
    if !matches!(array.data_type(), arrow_schema::DataType::Boolean) {
        return Ok(None);
    }
    Ok(Some(
        BooleanSerie::new(field, super::arrow::held::<BooleanArray>(&array)?).into_serie(),
    ))
}
