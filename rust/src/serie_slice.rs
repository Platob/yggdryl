//! SerieSlice: a window over a serie that reads and writes through it.
//!
//! [`Serie::slice`] answers a new serie - zero copy, an Arrow slice of a
//! column or a view of a run's values - and a caller who wants to read or
//! write a stretch of rows where they stand wants no serie at all: a
//! [`SerieSlice`] points at the serie and the window, moving nothing, and
//! every index it takes is window-relative, bounds-checked against the
//! window, then rebased onto the serie, so a leaf's `value(i)` under it is
//! one bounds check and one buffer read as it was. A [`SerieSliceMut`] is
//! the same window over a serie the caller holds mutably, and its writes go
//! through [`Serie::splice`] and [`Serie::set`] on the rebased range - so a
//! primitive leaf's `set` stays one buffer write, and a sort of a uniquely
//! held primitive or boolean buffer sorts the window where it stands. A
//! window never grows or shrinks what it views: a write that would is
//! refused by name.
//!
//! A window of a window is a window of the serie, its offsets summed, so
//! nothing nests: [`SerieSlice::window`] and [`SerieSliceMut::window_mut`]
//! narrow onto the same serie, and [`SerieWindows`] - the windows of equal
//! adjacent keys [`Serie::window_by`] and [`SerieSlice::window_by`] cut -
//! are every one over the serie, at offsets in it.
//!
//! Identity is the window's rows alone, as a serie's is its rows: a window
//! equals, orders as and hashes like the serie of the same rows.
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

use arrow_buffer::BooleanBuffer;
use arrow_buffer::bit_iterator::BitIndexIterator;

use crate::arrow::scalar_memory_size;
use crate::expression::IntoSelector;
use crate::serie::{
    Rows, compare_rows, hash_rows, proven_row, require_range, require_row, require_window,
};
use crate::{DataType, Field, Result, Scalar, Serie, SortOptions};

/// A window over a serie, read through the serie's own implementation.
///
/// `Copy`: two words beside the reference. Every index is window-relative.
#[derive(Clone, Copy)]
pub struct SerieSlice<'a> {
    serie: &'a Serie,
    offset: usize,
    len: usize,
}

/// A window over a serie the caller holds mutably, read and written
/// through the serie's own implementation on the rebased range.
pub struct SerieSliceMut<'a> {
    serie: &'a mut Serie,
    offset: usize,
    len: usize,
}

/// The rows of one window, lent for a run and built one at a time for a
/// column, from either end.
pub struct SerieSliceRows<'a> {
    window: SerieSlice<'a>,
    front: usize,
    back: usize,
}

/// The windows of equal adjacent keys over one serie, in row order: what
/// [`Serie::window_by`] and [`SerieSlice::window_by`] answer.
///
/// It holds the key column and the bitmap of where each window opens, both
/// computed once; each step finds the next set bit, builds the window's key,
/// which is the key column's row at the window's first row, and lends the
/// window over the serie at its offset in it, so a window costs its key and
/// nothing else. Exact-size and fused.
///
/// ```
/// use yggdryl::{Scalar, Serie};
///
/// # fn main() -> yggdryl::Result<()> {
/// let days = Serie::from_scalars(
///     yggdryl::Field::new("day", yggdryl::DataType::Int32, false),
///     [1_i32, 1, 2].map(Scalar::from),
/// )?;
/// let mut windows = days.window_by("day")?;
/// assert_eq!(windows.len(), 2);
/// let (key, window) = windows.next().expect("a first window");
/// assert_eq!(key, Scalar::from_sequence([Scalar::from(1_i32)]));
/// assert_eq!((window.offset(), window.len()), (0, 2));
/// assert_eq!(windows.next().map(|(_, window)| window.offset()), Some(2));
/// assert!(windows.next().is_none());
/// # Ok(())
/// # }
/// ```
#[must_use = "the windows are cut only as the walk reaches them"]
#[derive(Clone, Debug)]
pub struct SerieWindows<'a> {
    serie: &'a Serie,
    /// The serie row the keys' first row stands for.
    offset: usize,
    /// One key row per row windowed.
    keys: Serie,
    /// One bit per key row, set where a window opens.
    starts: BooleanBuffer,
    /// The key row the next window opens at.
    front: usize,
    remaining: usize,
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
    pub fn window(&self, offset: usize, length: usize) -> Result<SerieSlice<'_>> {
        require_window(self.name(), offset, length, self.len())?;
        Ok(SerieSlice {
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
    pub fn window_mut(&mut self, offset: usize, length: usize) -> Result<SerieSliceMut<'_>> {
        require_window(self.name(), offset, length, self.len())?;
        Ok(SerieSliceMut {
            serie: self,
            offset,
            len: length,
        })
    }
}

impl<'a> SerieSlice<'a> {
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
    pub fn iter(&self) -> SerieSliceRows<'a> {
        SerieSliceRows {
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

    /// The window's rows grouped by `keys`, a serie as long as the window.
    ///
    /// # Errors
    ///
    /// [`Serie::partition_by`]'s refusal.
    pub fn partition_by(&self, keys: &Serie) -> Result<Vec<(Scalar, Serie)>> {
        self.into_serie().partition_by(keys)
    }

    /// The window's rows cut into windows of equal adjacent keys, as
    /// [`Serie::window_by`] cuts a serie's: the keys are computed over this
    /// window's rows alone, so a row outside it never moves a cut, and every
    /// window answered is over the same serie as this one, at its offset in
    /// that serie. Costs what [`Serie::window_by`] costs over
    /// [`Self::into_serie`], plus that serie - nothing for a whole window.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let days = Serie::from_scalars(
    ///     Field::new("day", DataType::Int32, false),
    ///     [1_i32, 1, 1, 2, 2].map(Scalar::from),
    /// )?;
    /// let tail = days.window(1, 4)?;
    /// let windows: Vec<_> = tail.window_by("day")?.collect();
    /// assert_eq!(windows.len(), 2);
    /// // Offsets are the serie's: the first window starts where the tail does.
    /// assert_eq!((windows[0].1.offset(), windows[0].1.len()), (1, 2));
    /// assert_eq!((windows[1].1.offset(), windows[1].1.len()), (3, 2));
    /// assert!(std::ptr::eq(windows[1].1.serie(), &days));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::window_by`]'s refusals, naming the serie, before any row is
    /// read.
    pub fn window_by(&self, by: impl IntoSelector) -> Result<SerieWindows<'a>> {
        let key = self.serie.window_key(&by.into_selector()?)?;
        let keys = key.apply_serie(&self.into_serie())?;
        Ok(SerieWindows::new(self.serie, self.offset, keys))
    }
}

impl<'a> SerieSliceMut<'a> {
    /// The same window, read only: every read of [`SerieSlice`].
    pub fn as_window(&self) -> SerieSlice<'_> {
        SerieSlice {
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
    /// [`SerieSlice::dtype`]'s refusal.
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
    /// [`SerieSlice::is_null`]'s refusal.
    pub fn is_null(&self, index: usize) -> Result<bool> {
        self.as_window().is_null(index)
    }

    /// Window row `index` as a value.
    ///
    /// # Errors
    ///
    /// [`SerieSlice::scalar`]'s refusal.
    pub fn scalar(&self, index: usize) -> Result<Scalar> {
        self.as_window().scalar(index)
    }

    /// Window row `index`, or `None` past the window.
    pub fn get(&self, index: usize) -> Option<Cow<'_, Scalar>> {
        self.as_window().get(index)
    }

    /// Walk the window's rows.
    pub fn iter(&self) -> SerieSliceRows<'_> {
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

    /// A narrower read-only window, `offset..offset + length` of this one.
    ///
    /// # Errors
    ///
    /// [`SerieSlice::window`]'s refusal.
    pub fn window(&self, offset: usize, length: usize) -> Result<SerieSlice<'_>> {
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
    pub fn window_mut(&mut self, offset: usize, length: usize) -> Result<SerieSliceMut<'_>> {
        require_window(self.name(), offset, length, self.len)?;
        Ok(SerieSliceMut {
            serie: self.serie,
            offset: self.offset + offset,
            len: length,
        })
    }

    /// The window's rows cut into windows of equal adjacent keys, read only:
    /// [`SerieSlice::window_by`] over [`Self::as_window`].
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut days = Serie::from_scalars(
    ///     Field::new("day", DataType::Int32, false),
    ///     [1_i32, 1, 1, 2, 2].map(Scalar::from),
    /// )?;
    /// let tail = days.window_mut(1, 4)?;
    /// let cuts: Vec<_> = tail
    ///     .window_by("day")?
    ///     .map(|(key, window)| (key, window.offset(), window.len()))
    ///     .collect();
    /// // Offsets are the serie's: the first window starts where the tail does.
    /// assert_eq!(cuts, [
    ///     (Scalar::from_sequence([Scalar::from(1_i32)]), 1, 2),
    ///     (Scalar::from_sequence([Scalar::from(2_i32)]), 3, 2),
    /// ]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::window_by`]'s refusals, naming the serie.
    pub fn window_by(&self, by: impl IntoSelector) -> Result<SerieWindows<'_>> {
        self.as_window().window_by(by)
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

    /// The window's rows grouped by `keys`.
    ///
    /// # Errors
    ///
    /// [`Serie::partition_by`]'s refusal.
    pub fn partition_by(&self, keys: &Serie) -> Result<Vec<(Scalar, Serie)>> {
        self.as_window().partition_by(keys)
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
        self.serie.write(range, rows);
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
    pub fn copy_from(&mut self, other: &SerieSlice<'_>) -> Result<()> {
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

impl Rows for SerieSlice<'_> {
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

impl PartialEq for SerieSlice<'_> {
    fn eq(&self, other: &Self) -> bool {
        compare_rows(self, other) == Ordering::Equal
    }
}

impl Eq for SerieSlice<'_> {}

impl PartialEq<Serie> for SerieSlice<'_> {
    fn eq(&self, other: &Serie) -> bool {
        compare_rows(self, other) == Ordering::Equal
    }
}

impl PartialEq<SerieSlice<'_>> for Serie {
    fn eq(&self, other: &SerieSlice<'_>) -> bool {
        compare_rows(self, other) == Ordering::Equal
    }
}

impl Ord for SerieSlice<'_> {
    /// The window's rows, and nothing else: not the serie, not the offset.
    fn cmp(&self, other: &Self) -> Ordering {
        compare_rows(self, other)
    }
}

impl PartialOrd for SerieSlice<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for SerieSlice<'_> {
    /// What `<[Scalar]>::hash` writes for the window's rows, so a window
    /// hashes like the serie of the same rows.
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_rows(self, state);
    }
}

impl fmt::Display for SerieSlice<'_> {
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

impl fmt::Debug for SerieSlice<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SerieSlice")
            .field("serie", &self.name())
            .field("offset", &self.offset)
            .field("len", &self.len)
            .field("nulls", &self.null_count())
            .finish()
    }
}

impl fmt::Display for SerieSliceMut<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.as_window(), formatter)
    }
}

impl fmt::Debug for SerieSliceMut<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SerieSliceMut")
            .field("serie", &self.name())
            .field("offset", &self.offset)
            .field("len", &self.len)
            .field("nulls", &self.null_count())
            .finish()
    }
}

impl<'a> IntoIterator for &SerieSlice<'a> {
    type Item = Cow<'a, Scalar>;
    type IntoIter = SerieSliceRows<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> Iterator for SerieSliceRows<'a> {
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

impl DoubleEndedIterator for SerieSliceRows<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        self.back -= 1;
        self.window.get(self.back)
    }
}

impl ExactSizeIterator for SerieSliceRows<'_> {
    fn len(&self) -> usize {
        self.back - self.front
    }
}

impl FusedIterator for SerieSliceRows<'_> {}

impl<'a> SerieWindows<'a> {
    /// The windows `keys` cuts, one key row per row of `serie` from
    /// `offset`: where they open is read once, here.
    pub(crate) fn new(serie: &'a Serie, offset: usize, keys: Serie) -> Self {
        let starts = keys.window_starts();
        Self {
            serie,
            offset,
            remaining: starts.count_set_bits(),
            keys,
            starts,
            front: 0,
        }
    }
}

impl<'a> Iterator for SerieWindows<'a> {
    type Item = (Scalar, SerieSlice<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let start = self.front;
        let len = self.starts.len();
        let end = BitIndexIterator::new(
            self.starts.values(),
            self.starts.offset() + start + 1,
            len - start - 1,
        )
        .next()
        .map_or(len, |next| start + 1 + next);
        self.front = end;
        self.remaining -= 1;
        Some((
            proven_row(&self.keys, start),
            SerieSlice {
                serie: self.serie,
                offset: self.offset + start,
                len: end - start,
            },
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for SerieWindows<'_> {
    fn len(&self) -> usize {
        self.remaining
    }
}

impl FusedIterator for SerieWindows<'_> {}
