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
use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder, NullBuffer};

use super::{Serie, layout, require_range, require_row, require_window};
use crate::spill::Backing;
use crate::value::SerieValue;
use crate::{Field, Result, Scalar, SortOptions};

/// One column of booleans: a bitmap of values beside a bitmap of validity.
///
/// The two bitmaps are Arrow's own, held apart and shared rather than
/// copied, so a clone costs pointer bumps and a row read builds no array.
#[derive(Clone)]
pub struct BooleanSerie {
    field: Arc<Field>,
    values: BooleanBuffer,
    nulls: Option<NullBuffer>,
    /// Where `values` and `nulls` live; a write puts them back on the heap.
    backing: Backing,
}

impl BooleanSerie {
    /// Pair a field with the bitmaps that hold its rows, the array taken
    /// apart into them.
    pub(crate) fn new(field: Arc<Field>, values: BooleanArray) -> Self {
        let (values, nulls) = values.into_parts();
        Self {
            field,
            values,
            nulls,
            backing: Backing::Heap,
        }
    }

    /// Borrow the values bitmap, without copying it.
    pub fn values(&self) -> &BooleanBuffer {
        &self.values
    }

    /// Read row `index` off the values bitmap: `None` when the row is absent
    /// or past the end.
    pub fn value(&self, index: usize) -> Option<bool> {
        (index < self.values.len() && self.is_valid(index)).then(|| self.values.value(index))
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// The Arrow array these bitmaps are, rebuilt around them: pointer
    /// bumps, no allocation and no copy.
    pub fn array(&self) -> BooleanArray {
        BooleanArray::new(self.values.clone(), self.nulls.clone())
    }

    /// Where the bitmaps live.
    pub(crate) const fn backing(&self) -> &Backing {
        &self.backing
    }

    /// State where the bitmaps live: a spill names the mapping it laid them
    /// in, a write the heap.
    pub(crate) fn set_backing(&mut self, backing: Backing) {
        self.backing = backing;
    }

    /// Whether row `index`, inside the column, holds a value: read off the
    /// validity bitmap.
    fn is_valid(&self, index: usize) -> bool {
        self.nulls
            .as_ref()
            .is_none_or(|nulls| nulls.is_valid(index))
    }

    /// Hold the bitmaps a write finished as this column's rows, on the heap
    /// whatever the bitmaps they replace lay in.
    fn put(&mut self, values: BooleanBuffer, nulls: Option<NullBuffer>) {
        self.values = values;
        self.nulls = nulls;
        self.set_backing(Backing::Heap);
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

    /// Replace rows `range` by `replacement`: the two bitmaps moved out of
    /// the column and spliced, each in place where this column held it
    /// alone, copied once where it was shared or mapped.
    fn write_array(&mut self, range: Range<usize>, replacement: BooleanArray) {
        let len = self.values.len();
        let present: Vec<bool> = (0..replacement.len())
            .map(|row| replacement.is_valid(row))
            .collect();
        let bits = std::mem::replace(&mut self.values, BooleanBuffer::new_unset(0));
        let bits = layout::splice_bits(bits, len, range.clone(), replacement.values());
        let nulls = layout::splice_nulls(self.nulls.take(), len, range, &present);
        self.put(bits, nulls);
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
        self.write_array(len..len, other.array());
        true
    }

    /// Take both bitmaps out of the column as builders over their own
    /// bytes, copied once where this column does not hold them alone or
    /// they lie in a mapping.
    fn take_bits(&mut self) -> (BooleanBufferBuilder, Option<BooleanBufferBuilder>) {
        let bits = std::mem::replace(&mut self.values, BooleanBuffer::new_unset(0));
        (
            layout::owned_bits(bits),
            self.nulls
                .take()
                .map(|nulls| layout::owned_bits(nulls.into_inner())),
        )
    }

    /// Put both bitmaps back.
    fn put_bits(
        &mut self,
        mut values: BooleanBufferBuilder,
        validity: Option<BooleanBufferBuilder>,
    ) {
        let nulls = validity.map(|mut validity| NullBuffer::new(validity.finish()));
        self.put(values.finish(), nulls);
    }

    /// Sort rows `range` in place under `options`: the present falses and
    /// trues counted and written back as two runs - falses first ascending,
    /// trues first descending - and the absent rows gathered to the end
    /// `options` names, both bitmaps rewritten where they stand.
    ///
    /// No row is built and nothing is copied when the column holds its
    /// bitmaps alone; a shared one is copied once.
    pub(crate) fn sort_in_place(&mut self, range: Range<usize>, options: SortOptions) {
        let (mut values, mut validity) = self.take_bits();
        let (mut trues, mut absent) = (0, 0);
        for index in range.clone() {
            match &validity {
                Some(valid) if !valid.get_bit(index) => absent += 1,
                _ => trues += usize::from(values.get_bit(index)),
            }
        }
        let present = range.len() - absent;
        let (present_start, absent_start) = if options.is_nulls_first() {
            (range.start + absent, range.start)
        } else {
            (range.start, range.start + present)
        };
        let (first, first_count) = if options.is_descending() {
            (true, trues)
        } else {
            (false, present - trues)
        };
        for offset in 0..present {
            values.set_bit(present_start + offset, (offset < first_count) == first);
        }
        if let Some(valid) = validity.as_mut() {
            for index in present_start..present_start + present {
                valid.set_bit(index, true);
            }
            for index in absent_start..absent_start + absent {
                valid.set_bit(index, false);
                values.set_bit(index, false);
            }
        }
        self.put_bits(values, validity);
    }

    /// Reverse rows `range` in place: both bitmaps reversed where they
    /// stand, copied once where this column does not hold them alone.
    pub(crate) fn reverse_in_place(&mut self, range: Range<usize>) {
        let (mut values, mut validity) = self.take_bits();
        layout::reverse_bits(&mut values, range.clone());
        if let Some(valid) = validity.as_mut() {
            layout::reverse_bits(valid, range);
        }
        self.put_bits(values, validity);
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
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.values.len())?;
        Ok(!self.is_valid(index))
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
            nulls: self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
            backing: self.backing,
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
        Arc::new(self.array())
    }

    fn memory_size(&self) -> usize {
        // The typed array is pointer bumps, so nothing is boxed to count it.
        crate::arrow::sliced_size(&self.array())
    }

    fn resident_size(&self) -> usize {
        if self.backing().is_mapped() {
            0
        } else {
            self.memory_size()
        }
    }

    fn into_serie(self) -> Serie {
        super::Leaf::root(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        super::Leaf::narrow_laid(value)
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
    proof: &super::arrow::Proof,
    _budget: &mut crate::budget::MaterializationBudget,
    _resolved: Option<&super::arrow::Resolved>,
) -> crate::arrow::Result<Option<Serie>> {
    let _ = (parent, proof);
    if !matches!(array.data_type(), arrow_schema::DataType::Boolean) {
        return Ok(None);
    }
    Ok(Some(
        BooleanSerie::new(field, super::arrow::held::<BooleanArray>(&array)?).into_serie(),
    ))
}
