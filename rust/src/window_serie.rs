//! Borrowed windows read and mutate rows through their underlying serie.
//! Clustering answers owned [`crate::KeySeries`]; borrowing remains the in-place door.
//!
//! ```
//! use yggdryl::{Scalar, Serie, SortOptions};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut prices = Serie::new(vec![
//!     Scalar::from(9_i64),
//!     Scalar::from(3_i64),
//!     Scalar::from(1_i64),
//!     Scalar::from(2_i64),
//! ]);
//!
//! // A window reads through the serie, window-relative.
//! let middle = prices.window(1, 2)?;
//! assert_eq!((middle.len(), middle.offset()), (2, 1));
//! assert_eq!(middle.scalar(0)?, Scalar::from(3_i64));
//! assert!(middle.scalar(2).is_err());
//!
//! // And writes through it, in place, never past its edges.
//! prices.window_mut(1, 3)?.as_sorted(SortOptions::default())?;
//! assert_eq!(
//!     prices.rows().to_vec(),
//!     vec![Scalar::from(9_i64), Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(3_i64)]
//! );
//! # Ok(())
//! # }
//! ```

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::iter::FusedIterator;
use std::ops::Range;

use crate::arrow::scalar_memory_size;
use crate::expression::IntoOrderings;
use crate::serie::{
    Rows, StructSerie, compare_rows, hash_rows, proven_row, require_range, require_row,
    require_window,
};
use crate::{DataType, Field, Result, Scalar, Serie, SortOptions};

/// A window over a serie, read through the serie's own implementation.
///
/// `Copy`: one reference, an offset and a length. Every index is window-relative.
#[derive(Clone, Copy)]
pub struct WindowSerie<'a> {
    serie: &'a Serie,
    offset: usize,
    len: usize,
}

const _: () = assert!(size_of::<WindowSerie<'static>>() == 24);

/// Each key cell of row `row` of `keys`, the record column a key computes,
/// in order: the cell's value, or null where the record row is absent.
/// `row` is below the length.
pub(crate) fn key_cells(keys: &StructSerie, row: usize) -> impl ExactSizeIterator<Item = Scalar> {
    let absent = keys.nulls().is_some_and(|nulls| nulls.is_null(row));
    keys.children().iter().map(move |cell| {
        if absent {
            Scalar::Null
        } else {
            proven_row(cell, row)
        }
    })
}

/// The next start after `start`, or the length when this is the last run.
pub(crate) fn window_end(starts: &arrow_buffer::BooleanBuffer, start: usize) -> usize {
    let len = starts.len();
    arrow_buffer::bit_iterator::BitIndexIterator::new(
        starts.values(),
        starts.offset() + start + 1,
        len - start - 1,
    )
    .next()
    .map_or(len, |next| start + 1 + next)
}

/// A window over a serie the caller holds mutably, read and written
/// through the serie's own implementation on the rebased range.
pub struct WindowSerieMut<'a> {
    serie: &'a mut Serie,
    offset: usize,
    len: usize,
}

/// The rows of one window, lent for a run and built one at a time for a
/// column, from either end.
pub struct WindowSerieRows<'a> {
    window: WindowSerie<'a>,
    front: usize,
    back: usize,
}

impl Serie {
    /// The window `offset..offset + length` as a view that reads through
    /// this serie, moving nothing: every index it takes is window-relative.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(3_i64)]);
    /// let tail = prices.window(1, 2)?;
    /// assert_eq!((tail.len(), tail.scalar(0)?), (2, Scalar::from(2_i64)));
    /// assert!(tail.scalar(2).is_err());
    /// assert!(prices.window(2, 2).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when the window
    /// reaches past the end.
    pub fn window(&self, offset: usize, length: usize) -> Result<WindowSerie<'_>> {
        require_window(self.name(), offset, length, self.len())?;
        Ok(WindowSerie {
            serie: self,
            offset,
            len: length,
        })
    }

    /// The window `offset..offset + length` as a view that reads and writes
    /// through this serie, moving nothing; a write never grows or shrinks
    /// what the window views.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(3_i64)]);
    /// let mut tail = prices.window_mut(1, 2)?;
    /// tail.set(1, Scalar::from(9_i64))?;
    /// assert!(tail.splice(0..1, Vec::new()).is_err());
    /// assert_eq!(prices.rows().to_vec(), vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(9_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when the window
    /// reaches past the end.
    pub fn window_mut(&mut self, offset: usize, length: usize) -> Result<WindowSerieMut<'_>> {
        require_window(self.name(), offset, length, self.len())?;
        Ok(WindowSerieMut {
            serie: self,
            offset,
            len: length,
        })
    }
}

impl<'a> WindowSerie<'a> {
    /// The number of rows the window holds.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the window holds no row.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The serie row the window starts at.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// The whole serie the window reads through.
    pub const fn serie(&self) -> &'a Serie {
        self.serie
    }

    /// The field every row is typed by, or `None` for a window over a run.
    pub fn field(&self) -> Option<&'a Field> {
        self.serie.field()
    }

    /// The path a refusal names: the serie's.
    fn name(&self) -> &'a str {
        self.serie.name()
    }

    /// The datatype the window materializes into: a column's, read off its
    /// field; a run's, agreed out of the window's rows.
    ///
    /// # Errors
    ///
    /// [`Serie::dtype`]'s refusal, over the window's rows.
    pub fn dtype(&self) -> Result<DataType> {
        match self.serie.field() {
            Some(_) => self.serie.dtype(),
            None => self.into_serie().dtype(),
        }
    }

    /// How many rows of the window hold no value: none where the column
    /// holds no absent row at all, else one validity read per window row,
    /// with no row built; a window over a run walks its values.
    pub fn null_count(&self) -> usize {
        match self.serie {
            Serie::Run(run) => run.as_slice()[self.offset..self.offset + self.len]
                .iter()
                .filter(|value| value.is_null())
                .count(),
            column if column.null_count() == 0 => 0,
            column => (self.offset..self.offset + self.len)
                .filter(|index| column.is_null(*index).unwrap_or(false))
                .count(),
        }
    }

    /// Whether window row `index` holds no value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when `index` is
    /// past the window.
    pub fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.name(), index, self.len)?;
        self.serie.is_null(self.offset + index)
    }

    /// Window row `index` as a value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when `index` is
    /// past the window.
    pub fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.name(), index, self.len)?;
        self.serie.scalar(self.offset + index)
    }

    /// Window row `index`, or `None` past the window: borrowed for a run,
    /// built for a column.
    pub fn get(&self, index: usize) -> Option<Cow<'a, Scalar>> {
        (index < self.len)
            .then(|| self.serie.get(self.offset + index))
            .flatten()
    }

    /// Walk the window's rows: lent for a run, built one at a time for a
    /// column.
    pub fn iter(&self) -> WindowSerieRows<'a> {
        WindowSerieRows {
            window: *self,
            front: 0,
            back: self.len,
        }
    }

    /// The window's rows: lent for a run, built once for a column.
    pub fn rows(&self) -> Cow<'a, [Scalar]> {
        match self.serie {
            Serie::Run(run) => Cow::Borrowed(&run.as_slice()[self.offset..self.offset + self.len]),
            column => Cow::Owned(
                (self.offset..self.offset + self.len)
                    .map(|index| proven_row(column, index))
                    .collect(),
            ),
        }
    }

    /// The bytes the window's rows occupy: a column's as its own slice
    /// counts them, a run's as the row estimator charges them.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64)]);
    /// assert!(prices.window(0, 1)?.memory_size() < prices.memory_size());
    /// # Ok(())
    /// # }
    /// ```
    pub fn memory_size(&self) -> usize {
        match self.serie {
            Serie::Run(run) => run.as_slice()[self.offset..self.offset + self.len]
                .iter()
                .map(scalar_memory_size)
                .sum(),
            _ => self.into_serie().memory_size(),
        }
    }

    /// The bytes the window's rows occupy in memory, read through the serie
    /// it views: a window is never spilled on its own - spill the serie.
    pub fn resident_size(&self) -> usize {
        match self.serie {
            Serie::Run(_) => self.memory_size(),
            _ => self.into_serie().resident_size(),
        }
    }

    /// Whether the window's rows lie in a spill file: the serie's own answer
    /// over the rows it views.
    pub fn is_spilled(&self) -> bool {
        match self.serie {
            Serie::Run(_) => false,
            _ => self.into_serie().is_spilled(),
        }
    }

    /// A narrower window, `offset..offset + length` of this one.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when the window
    /// reaches past this one.
    pub fn window(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.name(), offset, length, self.len)?;
        Ok(Self {
            serie: self.serie,
            offset: self.offset + offset,
            len: length,
        })
    }

    /// The window as a serie of its own: [`Serie::slice`], zero copy - an
    /// Arrow slice of a column, a view of a run's values, and the serie
    /// itself for a window over all of it.
    pub fn into_serie(&self) -> Serie {
        self.serie
            .slice(self.offset, self.len)
            .expect("a window was proven against the serie when it was taken")
    }

    /// Whether the window's rows are in sorted order under `options`, as
    /// [`Serie::is_sorted`] answers for the window's serie.
    pub fn is_sorted(&self, options: SortOptions) -> bool {
        self.into_serie().is_sorted(options)
    }

    /// Whether no two rows of the window hold one value.
    pub fn is_unique(&self) -> bool {
        self.into_serie().is_unique()
    }

    /// How many distinct values the window's rows hold.
    pub fn unique_count(&self) -> usize {
        self.into_serie().unique_count()
    }

    /// The window-relative row positions in sorted order, as
    /// [`Serie::sort_indices`] answers them.
    ///
    /// # Errors
    ///
    /// [`Serie::sort_indices`]'s refusal.
    pub fn sort_indices(&self, options: SortOptions) -> Result<Serie> {
        self.into_serie().sort_indices(options)
    }

    /// The window's rows in sorted order, as a serie of their own.
    ///
    /// # Errors
    ///
    /// [`Serie::into_sorted`]'s refusal.
    pub fn into_sorted(&self, options: SortOptions) -> Result<Serie> {
        self.into_serie().into_sorted(options)
    }

    /// The window-relative row positions in the order the `order by` keys
    /// of `by` state, as [`Serie::sort_indices_by`] answers them: the keys
    /// computed over the window's rows alone, so a row outside it is never
    /// read.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let prices = Serie::from_scalars(DataType::Int64.required_field("price"), [9_i64, 1, 3, 2].map(Scalar::from))?;
    /// let order = prices.window(1, 3)?.sort_indices_by("price desc")?;
    /// assert_eq!(order.rows().to_vec(), [1_u32, 2, 0].map(Scalar::from));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::sort_indices_by`]'s refusals.
    pub fn sort_indices_by(&self, by: impl IntoOrderings) -> Result<Serie> {
        self.into_serie().sort_indices_by(by)
    }

    /// The window's rows in the order the `order by` keys of `by` state, as
    /// a serie of their own: [`Serie::into_sort_by`] over the window.
    ///
    /// # Errors
    ///
    /// [`Serie::into_sort_by`]'s refusals.
    pub fn into_sort_by(&self, by: impl IntoOrderings) -> Result<Serie> {
        self.into_serie().into_sort_by(by)
    }

    /// The first occurrence of every value in the window.
    ///
    /// # Errors
    ///
    /// [`Serie::into_unique`]'s refusal.
    pub fn into_unique(&self) -> Result<Serie> {
        self.into_serie().into_unique()
    }

    /// The window's rows in reverse order, as a serie of their own.
    pub fn into_reversed(&self) -> Serie {
        self.into_serie().into_reversed()
    }

    /// The window's rows `indices` names, window-relative.
    ///
    /// # Errors
    ///
    /// [`Serie::into_taken`]'s refusal.
    pub fn into_taken(&self, indices: &Serie) -> Result<Serie> {
        self.into_serie().into_taken(indices)
    }

    /// The window's rows `mask` keeps.
    ///
    /// # Errors
    ///
    /// [`Serie::into_filtered`]'s refusal.
    pub fn into_filtered(&self, mask: &Serie) -> Result<Serie> {
        self.into_serie().into_filtered(mask)
    }

    /// Cut this view's rows, keeping absolute source positions.
    ///
    /// # Errors
    /// [`Serie::window_by`]'s refusals.
    pub fn window_by(&self, by: impl crate::IntoKeyBy, sorted: bool) -> Result<crate::KeySeries> {
        let mut keys = self.into_serie().window_by(by, sorted)?;
        keys.shift_rownum(self.offset as u64)?;
        Ok(keys)
    }

    /// Group this view's rows by the common key intake.
    ///
    /// # Errors
    /// [`Serie::partition_by`]'s refusals.
    pub fn partition_by(&self, by: impl crate::IntoKeyBy) -> Result<crate::KeySeries> {
        self.into_serie().partition_by(by)
    }
}

impl<'a> WindowSerieMut<'a> {
    /// Group this view through its immutable row window.
    ///
    /// # Errors
    /// [`Serie::partition_by`]'s refusals.
    pub fn partition_by(&self, by: impl crate::IntoKeyBy) -> Result<crate::KeySeries> {
        self.as_window().partition_by(by)
    }
    /// The same window, read only: every read of [`WindowSerie`].
    pub fn as_window(&self) -> WindowSerie<'_> {
        WindowSerie {
            serie: self.serie,
            offset: self.offset,
            len: self.len,
        }
    }

    /// The number of rows the window holds.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the window holds no row.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The serie row the window starts at.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// The whole serie the window reads and writes through.
    pub fn serie(&self) -> &Serie {
        self.serie
    }

    /// The field every row is typed by, or `None` for a window over a run.
    pub fn field(&self) -> Option<&Field> {
        self.serie.field()
    }

    /// The datatype the window materializes into.
    ///
    /// # Errors
    ///
    /// [`WindowSerie::dtype`]'s refusal.
    pub fn dtype(&self) -> Result<DataType> {
        self.as_window().dtype()
    }

    /// How many rows of the window hold no value.
    pub fn null_count(&self) -> usize {
        self.as_window().null_count()
    }

    /// Whether window row `index` holds no value.
    ///
    /// # Errors
    ///
    /// [`WindowSerie::is_null`]'s refusal.
    pub fn is_null(&self, index: usize) -> Result<bool> {
        self.as_window().is_null(index)
    }

    /// Window row `index` as a value.
    ///
    /// # Errors
    ///
    /// [`WindowSerie::scalar`]'s refusal.
    pub fn scalar(&self, index: usize) -> Result<Scalar> {
        self.as_window().scalar(index)
    }

    /// Window row `index`, or `None` past the window.
    pub fn get(&self, index: usize) -> Option<Cow<'_, Scalar>> {
        self.as_window().get(index)
    }

    /// Walk the window's rows.
    pub fn iter(&self) -> WindowSerieRows<'_> {
        self.as_window().iter()
    }

    /// The window's rows: lent for a run, built once for a column.
    pub fn rows(&self) -> Cow<'_, [Scalar]> {
        self.as_window().rows()
    }

    /// The bytes the window's rows occupy.
    pub fn memory_size(&self) -> usize {
        self.as_window().memory_size()
    }

    /// The bytes the window's rows occupy in memory, read through the serie.
    pub fn resident_size(&self) -> usize {
        self.as_window().resident_size()
    }

    /// Whether the window's rows lie in a spill file.
    pub fn is_spilled(&self) -> bool {
        self.as_window().is_spilled()
    }

    /// A narrower read-only window, `offset..offset + length` of this one.
    ///
    /// # Errors
    ///
    /// [`WindowSerie::window`]'s refusal.
    pub fn window(&self, offset: usize, length: usize) -> Result<WindowSerie<'_>> {
        self.as_window().window(offset, length)
    }

    /// A narrower window, `offset..offset + length` of this one, that reads
    /// and writes through the same serie: the offsets summed, so a window of
    /// a window of a window is one window of the serie, and this one is
    /// borrowed while the narrower one lives.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut prices = Serie::new([1_i64, 2, 3, 4, 5].map(Scalar::from).to_vec());
    /// let mut tail = prices.window_mut(1, 4)?;
    /// let mut middle = tail.window_mut(1, 2)?;
    /// assert_eq!((middle.offset(), middle.len()), (2, 2));
    /// middle.set(0, Scalar::from(9_i64))?;
    /// assert!(middle.window_mut(1, 2).is_err());
    /// assert_eq!(prices.scalar(2)?, Scalar::from(9_i64));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when the window
    /// reaches past this one.
    pub fn window_mut(&mut self, offset: usize, length: usize) -> Result<WindowSerieMut<'_>> {
        require_window(self.name(), offset, length, self.len)?;
        Ok(WindowSerieMut {
            serie: self.serie,
            offset: self.offset + offset,
            len: length,
        })
    }

    /// The window as a serie of its own.
    pub fn into_serie(&self) -> Serie {
        self.as_window().into_serie()
    }

    /// Whether the window's rows are in sorted order under `options`.
    pub fn is_sorted(&self, options: SortOptions) -> bool {
        self.as_window().is_sorted(options)
    }

    /// Whether no two rows of the window hold one value.
    pub fn is_unique(&self) -> bool {
        self.as_window().is_unique()
    }

    /// How many distinct values the window's rows hold.
    pub fn unique_count(&self) -> usize {
        self.as_window().unique_count()
    }

    /// The window-relative row positions in sorted order.
    ///
    /// # Errors
    ///
    /// [`Serie::sort_indices`]'s refusal.
    pub fn sort_indices(&self, options: SortOptions) -> Result<Serie> {
        self.as_window().sort_indices(options)
    }

    /// The window's rows in sorted order, as a serie of their own.
    ///
    /// # Errors
    ///
    /// [`Serie::into_sorted`]'s refusal.
    pub fn into_sorted(&self, options: SortOptions) -> Result<Serie> {
        self.as_window().into_sorted(options)
    }

    /// The window-relative row positions in the order the `order by` keys
    /// of `by` state.
    ///
    /// # Errors
    ///
    /// [`Serie::sort_indices_by`]'s refusals.
    pub fn sort_indices_by(&self, by: impl IntoOrderings) -> Result<Serie> {
        self.as_window().sort_indices_by(by)
    }

    /// The window's rows in the order the `order by` keys of `by` state, as
    /// a serie of their own.
    ///
    /// # Errors
    ///
    /// [`Serie::into_sort_by`]'s refusals.
    pub fn into_sort_by(&self, by: impl IntoOrderings) -> Result<Serie> {
        self.as_window().into_sort_by(by)
    }

    /// The first occurrence of every value in the window.
    ///
    /// # Errors
    ///
    /// [`Serie::into_unique`]'s refusal.
    pub fn into_unique(&self) -> Result<Serie> {
        self.as_window().into_unique()
    }

    /// The window's rows in reverse order, as a serie of their own.
    pub fn into_reversed(&self) -> Serie {
        self.as_window().into_reversed()
    }

    /// The window's rows `indices` names, window-relative.
    ///
    /// # Errors
    ///
    /// [`Serie::into_taken`]'s refusal.
    pub fn into_taken(&self, indices: &Serie) -> Result<Serie> {
        self.as_window().into_taken(indices)
    }

    /// The window's rows `mask` keeps.
    ///
    /// # Errors
    ///
    /// [`Serie::into_filtered`]'s refusal.
    pub fn into_filtered(&self, mask: &Serie) -> Result<Serie> {
        self.as_window().into_filtered(mask)
    }

    /// The path a refusal names: the serie's.
    fn name(&self) -> &str {
        self.serie.name()
    }

    /// The serie's range this window is.
    const fn range(&self) -> Range<usize> {
        self.offset..self.offset + self.len
    }

    // --------------------------------------------------------------------
    // Writes, each through `Serie::splice` or `Serie::set` on the rebased
    // range, so a leaf's cost is what the serie states.
    // --------------------------------------------------------------------

    /// Overwrite window row `index`, through the field's contract where
    /// there is one: [`Serie::set`] on the rebased row, one buffer write on
    /// a primitive or boolean leaf.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the window, or
    /// [`Serie::set`]'s refusal.
    pub fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        require_row(self.name(), index, self.len)?;
        self.serie.set(self.offset + index, value)
    }

    /// Overwrite every window row with `value`, proved once and written as
    /// clones.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is not one the field accepts, or
    /// [`Serie::splice`]'s refusal.
    pub fn fill(&mut self, value: Scalar) -> Result<()> {
        let canonical = match self.serie.field() {
            Some(field) => field.scalar(value)?,
            None => value,
        };
        let rows = vec![canonical; self.len];
        let range = self.range();
        self.serie.check(&range, &rows)?;
        self.serie.write(range.clone(), rows);
        self.serie.keep_or_clear_order(range)?;
        Ok(())
    }

    /// Swap window rows `left` and `right`: two reads and two writes.
    ///
    /// # Errors
    ///
    /// Returns an error when either index is past the window, or
    /// [`Serie::set`]'s refusal.
    pub fn swap(&mut self, left: usize, right: usize) -> Result<()> {
        require_row(self.name(), left, self.len)?;
        require_row(self.name(), right, self.len)?;
        if left == right {
            return Ok(());
        }
        let (first, second) = (self.scalar(left)?, self.scalar(right)?);
        self.serie.set(self.offset + left, second)?;
        self.serie.set(self.offset + right, first)
    }

    /// Overwrite the window with `other`'s rows, row for row: `other` is as
    /// long as this window, and its rows go through this serie's field
    /// contract exactly as [`Serie::extend_from_serie`]'s do.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when `other` is another length, or
    /// [`Serie::splice`]'s refusal.
    pub fn copy_from(&mut self, other: &WindowSerie<'_>) -> Result<()> {
        self.splice(0..self.len, other.rows().into_owned())
    }

    /// Replace window rows `range` by `rows`, which are exactly as many: a
    /// window never grows or shrinks what it views.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when `range` is reversed or past
    /// the window, or when `rows` is another length than `range`; and
    /// [`Serie::splice`]'s refusal.
    pub fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.name(), &range, self.len)?;
        if rows.len() != range.len() {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.name()),
                reason: smol_str::format_smolstr!(
                    "a window of {} never grows or shrinks what it views: {} rows cannot replace {}",
                    self.name(),
                    rows.len(),
                    range.len()
                ),
            });
        }
        let rebased = self.offset + range.start..self.offset + range.end;
        self.serie.splice(rebased, rows)
    }

    /// Sort the window's rows in place under `options`, answering this
    /// window so calls chain: a primitive column holding its buffer alone
    /// sorts the window of its native slice where it stands, a boolean
    /// column its two bitmaps, a run its values, and every other leaf
    /// writes the sorted rows back through [`Serie::splice`].
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie, SortOptions};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut flags = Serie::new(vec![
    ///     Scalar::from(true),
    ///     Scalar::from(true),
    ///     Scalar::Null,
    ///     Scalar::from(false),
    /// ]);
    /// flags.window_mut(1, 3)?.as_sorted(SortOptions::default())?;
    /// assert_eq!(
    ///     flags.rows().to_vec(),
    ///     vec![Scalar::from(true), Scalar::from(false), Scalar::from(true), Scalar::Null]
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::sort_indices`]'s refusal, which leaves the serie as it was.
    pub fn as_sorted(&mut self, options: SortOptions) -> Result<&mut Self> {
        let range = self.range();
        if self.serie.sort_range_in_place(range.clone(), options) {
            return Ok(self);
        }
        let rows = self.into_serie().into_sorted(options)?.rows().into_owned();
        self.serie.splice(range, rows)?;
        Ok(self)
    }

    /// Sort the window's rows in place in the order the `order by` keys of
    /// `by` state, answering this window so calls chain: the keys computed
    /// over the window's rows alone, the sorted rows written back through
    /// [`Serie::splice`] on the rebased range, every row outside the window
    /// untouched and the window never grown or shrunk.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut prices = Serie::from_scalars(DataType::Int64.required_field("price"), [9_i64, 1, 3, 2].map(Scalar::from))?;
    /// prices.window_mut(1, 3)?.as_sort_by("price desc")?;
    /// assert_eq!(prices.rows().to_vec(), [9_i64, 3, 2, 1].map(Scalar::from));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::sort_indices_by`]'s refusals, which leave the serie as it
    /// was.
    pub fn as_sort_by(&mut self, by: impl IntoOrderings) -> Result<&mut Self> {
        let rows = self.into_serie().into_sort_by(by)?.rows().into_owned();
        let range = self.range();
        self.serie.splice(range, rows)?;
        Ok(self)
    }

    /// Reverse the window's rows in place, answering this window: a
    /// primitive column holding its buffer alone reverses the window of
    /// its native slice where it stands, a boolean column its two bitmaps,
    /// a run its values, and every other leaf writes the reversed rows back
    /// through [`Serie::splice`].
    ///
    /// # Errors
    ///
    /// [`Serie::splice`]'s refusal, which cannot arise for rows the serie
    /// already holds.
    pub fn as_reversed(&mut self) -> Result<&mut Self> {
        let range = self.range();
        if self.serie.reverse_range_in_place(range.clone()) {
            return Ok(self);
        }
        let rows = self.into_serie().into_reversed().rows().into_owned();
        self.serie.splice(range, rows)?;
        Ok(self)
    }

    /// Rearrange the window's rows as `indices` names them, window-relative
    /// and exactly as many as the window holds, answering this window.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie when `indices` is another length
    /// than the window, [`Serie::into_taken`]'s refusal, or
    /// [`Serie::splice`]'s.
    pub fn as_taken(&mut self, indices: &Serie) -> Result<&mut Self> {
        if indices.len() != self.len {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.name()),
                reason: smol_str::format_smolstr!(
                    "a window of {} never grows or shrinks what it views: {} indices cannot rearrange {} rows",
                    self.name(),
                    indices.len(),
                    self.len
                ),
            });
        }
        let rows = self.into_serie().into_taken(indices)?.rows().into_owned();
        let range = self.range();
        self.serie.splice(range, rows)?;
        Ok(self)
    }
}

impl Rows for WindowSerie<'_> {
    fn rows_len(&self) -> usize {
        self.len
    }

    fn row_at(&self, index: usize) -> Cow<'_, Scalar> {
        match self.serie {
            Serie::Run(run) => Cow::Borrowed(&run.as_slice()[self.offset + index]),
            column => Cow::Owned(proven_row(column, self.offset + index)),
        }
    }
}

impl PartialEq for WindowSerie<'_> {
    fn eq(&self, other: &Self) -> bool {
        compare_rows(self, other) == Ordering::Equal
    }
}

impl Eq for WindowSerie<'_> {}

impl PartialEq<Serie> for WindowSerie<'_> {
    fn eq(&self, other: &Serie) -> bool {
        compare_rows(self, other) == Ordering::Equal
    }
}

impl PartialEq<WindowSerie<'_>> for Serie {
    fn eq(&self, other: &WindowSerie<'_>) -> bool {
        compare_rows(self, other) == Ordering::Equal
    }
}

impl Ord for WindowSerie<'_> {
    /// The window's rows, and nothing else: not the serie, not the offset.
    fn cmp(&self, other: &Self) -> Ordering {
        compare_rows(self, other)
    }
}

impl PartialOrd for WindowSerie<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for WindowSerie<'_> {
    /// What `<[Scalar]>::hash` writes for the window's rows, so a window
    /// hashes like the serie of the same rows.
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_rows(self, state);
    }
}

impl fmt::Display for WindowSerie<'_> {
    /// The serie's name - `$` for a run - and the window's rows.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}[", self.name())?;
        for index in 0..self.len {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{:?}", self.row_at(index))?;
        }
        formatter.write_str("]")
    }
}

impl fmt::Debug for WindowSerie<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowSerie")
            .field("serie", &self.name())
            .field("offset", &self.offset)
            .field("len", &self.len)
            .field("nulls", &self.null_count())
            .finish()
    }
}

impl fmt::Display for WindowSerieMut<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.as_window(), formatter)
    }
}

impl fmt::Debug for WindowSerieMut<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowSerieMut")
            .field("serie", &self.name())
            .field("offset", &self.offset)
            .field("len", &self.len)
            .field("nulls", &self.null_count())
            .finish()
    }
}

impl<'a> IntoIterator for &WindowSerie<'a> {
    type Item = Cow<'a, Scalar>;
    type IntoIter = WindowSerieRows<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> Iterator for WindowSerieRows<'a> {
    type Item = Cow<'a, Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        let row = self.window.get(self.front);
        self.front += 1;
        row
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = self.back - self.front;
        (length, Some(length))
    }
}

impl DoubleEndedIterator for WindowSerieRows<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        self.back -= 1;
        self.window.get(self.back)
    }
}

impl ExactSizeIterator for WindowSerieRows<'_> {
    fn len(&self) -> usize {
        self.back - self.front
    }
}

impl FusedIterator for WindowSerieRows<'_> {}
