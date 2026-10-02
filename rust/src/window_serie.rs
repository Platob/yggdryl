//! WindowSerie: a window over a serie that reads and writes through it.
//!
//! [`Serie::slice`] answers a new serie - zero copy, an Arrow slice of a
//! column or a view of a run's values - and a caller who wants to read or
//! write a stretch of rows where they stand wants no serie at all: a
//! [`WindowSerie`] points at the serie and the window, moving nothing, and
//! every index it takes is window-relative, bounds-checked against the
//! window, then rebased onto the serie, so a leaf's `value(i)` under it is
//! one bounds check and one buffer read as it was. A [`WindowSerieMut`] is
//! the same window over a serie the caller holds mutably, and its writes go
//! through [`Serie::splice`] and [`Serie::set`] on the rebased range - so a
//! primitive leaf's `set` stays one buffer write, and a sort of a uniquely
//! held primitive or boolean buffer sorts the window where it stands. A
//! window never grows or shrinks what it views: a write that would is
//! refused by name.
//!
//! A window of a window is a window of the serie, its offsets summed, so
//! nothing nests: [`WindowSerie::window`] and [`WindowSerieMut::window_mut`]
//! narrow onto the same serie, and [`SerieWindows`] - the windows of equal
//! adjacent keys [`Serie::window_by`] and [`WindowSerie::window_by`] cut -
//! are every one over the serie, at offsets in it, unless `sorted` had to
//! gather the rows into key order: then every window is over that one
//! gathered copy, which the windows value owns.
//!
//! A window [`SerieWindows::iter`] lends states a record of values constant
//! over its rows - [`WindowSerie::static_values`]: the record of the window
//! it was cut from, if that is one, then its key cells, its place among the
//! windows (`windownum`) and the number its first row has in what it was cut
//! from (`rownum`). Nothing else states one: not a window [`Serie::window`]
//! takes, a narrower window, or a window turned back into a serie.
//!
//! Identity is the window's rows alone, as a serie's is its rows: a window
//! equals, orders as and hashes like the serie of the same rows, whatever
//! record it states.
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
use std::sync::OnceLock;

use arrow_buffer::BooleanBuffer;
use arrow_buffer::bit_iterator::BitIndexIterator;

use smol_str::{SmolStr, format_smolstr};

use crate::arrow::scalar_memory_size;
use crate::expression::{IntoSelector, Selector};
use crate::serie::{
    Rows, StructSerie, compare_rows, hash_rows, proven_row, require_indexable, require_range,
    require_row, require_window,
};
use crate::{DataType, Error, Field, FieldScalar, Result, Scalar, Serie, SortOptions, StructType};

/// A window over a serie, read through the serie's own implementation.
///
/// `Copy`: two words and where its record comes from beside the reference -
/// forty bytes. Every index is window-relative.
#[derive(Clone, Copy)]
pub struct WindowSerie<'a> {
    serie: &'a Serie,
    offset: usize,
    len: usize,
    /// Set only on a window [`SerieWindows::iter`] lent: where its record
    /// comes from.
    origin: Option<Origin<'a>>,
}

const _: () = assert!(size_of::<WindowSerie<'static>>() == 40);

/// Where the record of a window [`SerieWindows::iter`] lent comes from: the
/// windows it is one of, and its place among them. Its key row is read off
/// the cuts - the window's offset in the windowed rows when they are in row
/// order, the gathered window's own entry otherwise - so nothing it states
/// is held twice.
#[derive(Clone, Copy)]
struct Origin<'a> {
    windows: &'a SerieWindows<'a>,
    /// The window's place among the windows: its `windownum`.
    index: usize,
}

/// The static value naming a window's place among the windows of what it
/// was cut from, from 0.
const WINDOWNUM: &str = "windownum";

/// The static value naming the number a window's first row has in what it
/// was cut from: the crate's word for a row's number.
const ROWNUM: &str = "rownum";

/// The record a window states, typed once for every window of one call:
/// what [`WindowRecord::new`] answers.
#[derive(Clone, Debug)]
pub(crate) struct WindowRecord {
    /// The record's field: the kept cells', the key cells', `windownum`'s
    /// and `rownum`'s, in that order.
    pub(crate) field: Field,
    /// The cells of the record the windowed carrier states but `windownum`
    /// and `rownum` - a window of a window keeps the window's - ahead of the
    /// key cells in every window's row.
    pub(crate) kept: Box<[Scalar]>,
    /// The windowed carrier's own `rownum`, which every window's adds to:
    /// `None` where it is null, 0 where the carrier states no record.
    pub(crate) base: Option<u64>,
}

impl WindowRecord {
    /// The record every window of a carrier states, named `name` - the
    /// carrier's root: the cells of `stated`, the carrier's own record, but
    /// `windownum` and `rownum`; then the key `cells`, each as the key
    /// declares it; then `windownum: uint64`, required, and
    /// `rownum: uint64`, nullable. Keys holding an absent record row
    /// declare their cells nullable after ([`Self::with_absent_keys`]).
    ///
    /// # Errors
    ///
    /// Returns an error naming `path` and both names for a key cell whose
    /// name folds onto a kept cell's or onto `windownum` or `rownum`.
    pub(crate) fn new(
        path: &str,
        name: &str,
        stated: Option<&FieldScalar<'_>>,
        cells: &[Field],
    ) -> Result<Self> {
        let (stated, row) = match stated {
            Some(stated) => (
                stated.field().fields(),
                stated.value().sequence_rows().unwrap_or_default(),
            ),
            None => (&[][..], std::borrow::Cow::Borrowed(&[][..])),
        };
        let cut_from = || stated.iter().zip(row.iter());
        let held = || cut_from().filter(|(field, _)| reserved(field.name()).is_none());
        let base = cut_from()
            .find(|(field, _)| reserved(field.name()) == Some(ROWNUM))
            .map_or(Some(0), |(_, rownum)| match rownum {
                Scalar::UInt64(rownum) => Some(rownum.get()),
                _ => None,
            });
        for cell in cells {
            let collides = held()
                .map(|(field, _)| field.name())
                .chain([WINDOWNUM, ROWNUM])
                .find(|name| name.eq_ignore_ascii_case(cell.name()));
            if let Some(name) = collides {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new(path),
                    reason: format_smolstr!(
                        "the key cell {:?} collides with the static value {name:?}; alias the \
                         key cell (`... as <name>`)",
                        cell.name()
                    ),
                });
            }
        }
        let kept = held().count();
        let mut fields = Vec::with_capacity(kept + cells.len() + 2);
        let mut values = Vec::with_capacity(kept);
        for (field, value) in held() {
            fields.push(field.clone());
            values.push(value.clone());
        }
        fields.extend_from_slice(cells);
        fields.push(DataType::UInt64.required_field(WINDOWNUM));
        fields.push(DataType::UInt64.nullable_field(ROWNUM));
        Ok(Self {
            field: DataType::from(StructType::from_fields(fields)?).required_field(name),
            kept: values.into_boxed_slice(),
            base,
        })
    }

    /// This record with its `cells` key cells - the children after the
    /// kept ones - declared nullable: the record of keys holding an absent
    /// record row, every cell of which [`key_cells`] answers null there.
    /// Only a held record column's rows can be absent; a stream's root is
    /// required, so its keys never hold one and its record states the key
    /// cells as the key declares them - as a held window's does over the
    /// same rows.
    ///
    /// # Errors
    ///
    /// Only what [`StructType::from_fields`] refuses, which these children,
    /// the record's own under the names it already accepted, passed once.
    pub(crate) fn with_absent_keys(mut self, cells: usize) -> Result<Self> {
        let keys = self.kept.len()..self.kept.len() + cells;
        let fields: Vec<Field> = self
            .field
            .fields()
            .iter()
            .enumerate()
            .map(|(index, field)| {
                if keys.contains(&index) {
                    field.clone().with_nullable(true)
                } else {
                    field.clone()
                }
            })
            .collect();
        self.field =
            DataType::from(StructType::from_fields(fields)?).required_field(self.field.name());
        Ok(self)
    }
}

/// The reserved static name `name` folds onto, under the binder's ASCII
/// fold.
fn reserved(name: &str) -> Option<&'static str> {
    [WINDOWNUM, ROWNUM]
        .into_iter()
        .find(|reserved| reserved.eq_ignore_ascii_case(name))
}

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

/// The windows of equal adjacent keys over one serie: what
/// [`Serie::window_by`], [`WindowSerie::window_by`] and
/// [`WindowSerieMut::window_by`] answer.
///
/// An owner, lending its windows as often as asked: [`Self::iter`] - or
/// `&windows` in a `for` - walks them as `(key, window)` pairs, each window
/// a [`WindowSerie`] over [`Self::serie`]. Without `sorted`, or with keys
/// already in order, the windows are in row order over the windowed serie
/// itself, borrowed, at their offsets in it; with `sorted` and keys out of
/// order they are in key order over the one copy of the rows gathered in
/// that order, which this value owns.
///
/// It holds the key column and where each window opens, both computed once,
/// and the record every window states, typed once ([`Self::static_field`]);
/// each step builds the window's key - the key column's row at the window's
/// first row - and lends the window, so a window costs its key and nothing
/// else. A window's [record](WindowSerie::static_values) is built only when
/// it is read.
///
/// ```
/// use yggdryl::{DataType, Field, Scalar, Serie};
///
/// # fn main() -> yggdryl::Result<()> {
/// let day = Field::new("day", DataType::Int32, false);
/// let days = Serie::from_scalars(day.clone(), [1_i32, 1, 2].map(Scalar::from))?;
/// let windows = days.window_by("day", false)?;
/// assert_eq!(windows.len(), 2);
/// assert!(std::ptr::eq(windows.serie(), &days));
/// let mut walk = windows.iter();
/// let (key, window) = walk.next().expect("a first window");
/// assert_eq!(key, Scalar::from_sequence([Scalar::from(1_i32)]));
/// assert_eq!((window.offset(), window.len()), (0, 2));
/// assert_eq!(walk.next().map(|(_, window)| window.offset()), Some(2));
/// assert!(walk.next().is_none());
///
/// // Lent again, as often as asked.
/// let mut lengths = Vec::new();
/// for (_, window) in &windows {
///     lengths.push(window.len());
/// }
/// assert_eq!(lengths, [2, 1]);
///
/// // Sorted over keys out of order: the rows gathered once, in key order.
/// let mixed = Serie::from_scalars(day, [2_i32, 1, 2].map(Scalar::from))?;
/// let sorted = mixed.window_by("day", true)?;
/// assert!(!std::ptr::eq(sorted.serie(), &mixed));
/// assert_eq!(sorted.serie().rows().to_vec(), [1_i32, 2, 2].map(Scalar::from));
/// let cuts: Vec<_> = sorted
///     .iter()
///     .map(|(key, window)| (key, window.offset(), window.len()))
///     .collect();
/// assert_eq!(cuts, [
///     (Scalar::from_sequence([Scalar::from(1_i32)]), 0, 1),
///     (Scalar::from_sequence([Scalar::from(2_i32)]), 1, 2),
/// ]);
/// # Ok(())
/// # }
/// ```
#[must_use = "the windows are lent only by walking them"]
#[derive(Clone, Debug)]
pub struct SerieWindows<'a> {
    holder: Cow<'a, Serie>,
    /// The holder row the keys' first row stands for: the window's offset
    /// when borrowed, 0 when gathered.
    offset: usize,
    /// One key row per row windowed, in arrival order: a record column.
    keys: Serie,
    cuts: Cuts,
    /// The record every window states.
    record: WindowRecord,
    /// In row order, the key row each window opens at, indexed by the first
    /// [`Self::get`] and kept for every next one: one `usize` per window,
    /// so a window is reached in constant time however many there are.
    opens: OnceLock<Box<[usize]>>,
}

/// Where the windows of a [`SerieWindows`] lie in its holder.
#[derive(Clone, Debug)]
enum Cuts {
    /// Windows in row order: one bit per key row, set where a window opens,
    /// and the number of set bits.
    Starts(BooleanBuffer, usize),
    /// Windows in key order over the gathered holder: per window, the key
    /// row it is keyed by and where it ends in the holder, the first
    /// starting at 0 and each next one where the one before it ends.
    Gathered(Box<[(u32, u32)]>),
}

/// The walk over the windows a [`SerieWindows`] lends, from its first:
/// [`SerieWindows::iter`]. Exact-size and fused.
#[must_use = "the windows are lent only as the walk reaches them"]
#[derive(Clone, Debug)]
pub struct SerieWindowsIter<'s> {
    windows: &'s SerieWindows<'s>,
    /// The next window's place.
    index: usize,
    /// The key row the next window opens at, in row order; the next
    /// window's place, in key order.
    front: usize,
    /// In key order, the holder row the next window starts at.
    at: usize,
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
    pub fn window(&self, offset: usize, length: usize) -> Result<WindowSerie<'_>> {
        require_window(self.name(), offset, length, self.len())?;
        Ok(WindowSerie {
            serie: self,
            offset,
            len: length,
            origin: None,
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

    /// A narrower window, `offset..offset + length` of this one: a window
    /// of the rows alone, stating no [record](Self::static_values).
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
            origin: None,
        })
    }

    /// The values constant over this window's rows, where
    /// [`SerieWindows::iter`] lent it: one required record, typed by
    /// [`SerieWindows::static_field`] and named as the windowed serie's
    /// root ([`SerieReader::root_of`](crate::SerieReader::root_of)), its
    /// cells in order -
    ///
    /// 1. the cells of the record the windowed window states, when a window
    ///    lent this way was windowed, but its `windownum` and `rownum`;
    /// 2. the key cells at the window's first row, as the key's projections
    ///    declare them, and nullable too where the windowed rows hold an
    ///    absent record row, which keys null in every cell;
    /// 3. `windownum: uint64`, the window's place among the windows, from 0;
    /// 4. `rownum: uint64`, the number the window's first row has in what
    ///    was windowed - absolute through windows of windows, and null where
    ///    `sorted` gathered the rows out of their order.
    ///
    /// Every cell is read through [`FieldScalar`]'s own accessors. `None`
    /// for every other window: one [`Serie::window`] takes, a narrower one
    /// ([`Self::window`]) and a [`WindowSerieMut`]'s. The record is never
    /// the window's identity, and a serie built from the window
    /// ([`Self::into_serie`]) or an Arrow array of it carries rows only.
    ///
    /// Built here, on each call: one run of the cells, plus the text of a
    /// cell past what a value holds inline. A walk that reads no record pays
    /// nothing for it.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, Serie, StructType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = DataType::from(StructType::from_fields([
    ///     DataType::utf8().required_field("venue"),
    ///     DataType::Int64.required_field("price"),
    /// ])?)
    /// .required_field("quote");
    /// let quote = |venue: &str, price: i64| Scalar::from_sequence([Scalar::from(venue), Scalar::from(price)]);
    /// let quotes = Serie::from_scalars(root, [quote("XNAS", 1), quote("XNAS", 2), quote("XNYS", 3)])?;
    ///
    /// let windows = quotes.window_by("venue", false)?;
    /// let (_, xnys) = windows.iter().nth(1).expect("a second window");
    /// let record = xnys.static_values().expect("a window window_by lent");
    /// assert_eq!(record.name(), "quote");
    /// assert_eq!(record.get_key_str("venue"), Some(&Scalar::from("XNYS")));
    /// assert_eq!(record.get_key_str("windownum"), Some(&Scalar::from(1_u64)));
    /// assert_eq!(record.get_key_str("rownum"), Some(&Scalar::from(2_u64)));
    ///
    /// // A window of the rows alone states none.
    /// assert!(xnys.window(0, 1)?.static_values().is_none());
    /// assert!(quotes.window(2, 1)?.static_values().is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn static_values(&self) -> Option<FieldScalar<'a>> {
        let Origin { windows, index } = self.origin?;
        let record = &windows.record;
        let (key_row, rownum) = match (&windows.cuts, record.base) {
            // In row order, a window opens at its key row, and its first row
            // is a row of what was windowed, below its length: what the
            // windowed carrier's own number adds to stays in `uint64`.
            (Cuts::Starts(..), base) => {
                let key_row = self.offset - windows.offset;
                let rownum = base.map_or(Scalar::Null, |base| Scalar::from(base + key_row as u64));
                (key_row, rownum)
            }
            (Cuts::Gathered(cuts), _) => (cuts.get(index)?.0 as usize, Scalar::Null),
        };
        let row = Scalar::from_sequence(
            record
                .kept
                .iter()
                .cloned()
                .chain(key_cells(windows.keys.as_struct()?, key_row))
                .chain([Scalar::from(index as u64), rownum]),
        );
        // Every cell is proven under its child: the kept ones under the
        // record they were read from, the key cells by the key's landing,
        // and the two counts are the crate's own.
        Some(FieldScalar::from_checked(&record.field, row))
    }

    /// The window as a serie of its own: [`Serie::slice`], zero copy - an
    /// Arrow slice of a column, a view of a run's values, and the serie
    /// itself for a window over all of it. Rows only: the window's
    /// [record](Self::static_values) stays with the window.
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
    /// [`Serie::window_by`] cuts a serie's, `sorted` meaning what it means
    /// there: the keys are computed over this window's rows alone, so a row
    /// outside it never moves a cut, and every window answered is over the
    /// same serie as this one, at its offset in that serie - unless `sorted`
    /// gathers rows out of order, and then over the copy of this window's
    /// rows alone, from offset 0. Costs what [`Serie::window_by`] costs, plus
    /// one slice per key cell the key reads where it stands - never this
    /// window as a serie of its own.
    ///
    /// Each window answered states its [record](Self::static_values). Where
    /// this window states one, each keeps that record's cells but
    /// `windownum` and `rownum` ahead of its own key cells, and numbers its
    /// first row from this window's `rownum`, so the number stays absolute
    /// in what was windowed first; elsewhere its `rownum` counts from this
    /// window's first row. Keeping the record costs one run of its cells,
    /// once a call.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let days = Serie::from_scalars(
    ///     Field::new("day", DataType::Int32, false),
    ///     [1_i32, 1, 1, 2, 1].map(Scalar::from),
    /// )?;
    /// let tail = days.window(1, 3)?;
    /// let windows = tail.window_by("day", false)?;
    /// let windows: Vec<_> = windows.iter().collect();
    /// assert_eq!(windows.len(), 2);
    /// // Offsets are the serie's: the first window starts where the tail does.
    /// assert_eq!((windows[0].1.offset(), windows[0].1.len()), (1, 2));
    /// assert_eq!((windows[1].1.offset(), windows[1].1.len()), (3, 1));
    /// assert!(std::ptr::eq(windows[1].1.serie(), &days));
    ///
    /// // Sorted over keys out of order, the gather takes this window's rows
    /// // alone, from offset 0.
    /// let sorted = days.window(2, 3)?.window_by("day", true)?;
    /// assert_eq!(sorted.serie().rows().to_vec(), [1_i32, 1, 2].map(Scalar::from));
    /// let cuts: Vec<_> = sorted
    ///     .iter()
    ///     .map(|(_, window)| (window.offset(), window.len()))
    ///     .collect();
    /// assert_eq!(cuts, [(0, 2), (2, 1)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::window_by`]'s refusals, naming the serie, before any row is
    /// read, and a key cell whose name folds onto a cell this window's
    /// record keeps, naming both - alias it; with `sorted` and keys out of
    /// order, a serie past `u32::MAX` rows, which the gather cannot address,
    /// naming it.
    pub fn window_by(&self, by: impl IntoSelector, sorted: bool) -> Result<SerieWindows<'a>> {
        SerieWindows::new(
            self.serie,
            self.offset,
            self.len,
            &by.into_selector()?,
            self.static_values().as_ref(),
            sorted,
        )
    }
}

impl<'a> WindowSerieMut<'a> {
    /// The same window, read only: every read of [`WindowSerie`]. A
    /// mutable window states no [record](WindowSerie::static_values), and
    /// neither does this one.
    pub fn as_window(&self) -> WindowSerie<'_> {
        WindowSerie {
            serie: self.serie,
            offset: self.offset,
            len: self.len,
            origin: None,
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

    /// The window's rows cut into windows of equal adjacent keys, read only:
    /// [`WindowSerie::window_by`] over [`Self::as_window`] - each window
    /// answered states its [record](WindowSerie::static_values), its
    /// `rownum` counted from this window's first row.
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
    ///     .window_by("day", false)?
    ///     .iter()
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
    /// [`WindowSerie::window_by`]'s refusals, naming the serie.
    pub fn window_by(&self, by: impl IntoSelector, sorted: bool) -> Result<SerieWindows<'_>> {
        self.as_window().window_by(by, sorted)
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

impl<'a> SerieWindows<'a> {
    /// The windows `by` cuts the rows `offset..offset + len` of `holder`
    /// into: the key bound and the record every window states typed - its
    /// head the cells of `cut_from`, the record of the window windowed -
    /// before a row is read, then the key computed once and where the
    /// windows open read once, and with `sorted` and a descent in the keys
    /// the rows gathered into key order once, into a holder this value owns.
    ///
    /// # Errors
    ///
    /// [`Serie::window_by`]'s refusals and [`WindowRecord::new`]'s, before a
    /// row is read; then the key's own; with `sorted` and a descent, rows
    /// past what one `uint32` position addresses - in the keys or in
    /// `holder` - naming them.
    pub(crate) fn new(
        holder: &'a Serie,
        offset: usize,
        len: usize,
        by: &Selector,
        cut_from: Option<&FieldScalar<'_>>,
        sorted: bool,
    ) -> Result<Self> {
        let key = holder.window_key(by)?;
        let cells = key.output().fields();
        let mut record = WindowRecord::new(holder.name(), key.schema().name(), cut_from, cells)?;
        let keys = key.apply_serie_window(holder, offset, len)?;
        debug_assert!(keys.as_struct().is_some(), "a key lands as a record column");
        // The windowed rows' absent record rows are the keys' own, counted
        // on their validity bitmap: only then is a key cell null where the
        // key declares none.
        if keys.null_count() > 0 {
            record = record.with_absent_keys(cells.len())?;
        }
        let cut = keys.window_starts(sorted)?;
        let Some(regrouped) = cut.regrouped else {
            let count = cut.starts.count_set_bits();
            return Ok(Self {
                holder: Cow::Borrowed(holder),
                offset,
                keys,
                cuts: Cuts::Starts(cut.starts, count),
                record,
                opens: OnceLock::new(),
            });
        };
        require_indexable(holder)?;
        let mut order = regrouped.order;
        // `holder` addresses every row as a `uint32`, and the keys' rows lie
        // in it from `offset`, so each shifted position is one of its rows.
        let shift = offset as u32;
        if shift != 0 {
            for position in &mut order {
                *position += shift;
            }
        }
        Ok(Self {
            holder: Cow::Owned(holder.taken(&order)?),
            offset: 0,
            keys,
            cuts: Cuts::Gathered(regrouped.windows),
            record,
            opens: OnceLock::new(),
        })
    }

    /// How many windows there are.
    pub fn len(&self) -> usize {
        match &self.cuts {
            Cuts::Starts(_, count) => *count,
            Cuts::Gathered(windows) => windows.len(),
        }
    }

    /// Whether there is no window: no row was windowed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The serie every window views: the windowed serie, unless `sorted`
    /// gathered its rows into key order, then that gathered copy, which this
    /// value owns.
    pub fn serie(&self) -> &Serie {
        &self.holder
    }

    /// The record every window [states](WindowSerie::static_values), known
    /// before any window is walked: a required record named as the windowed
    /// serie's root, its children the cells the windowed window's record
    /// keeps, the key cells as the key declares them - each nullable where
    /// the windowed rows hold an absent record row - `windownum: uint64`
    /// and `rownum: uint64`, nullable.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let days = Serie::from_scalars(
    ///     Field::new("day", DataType::Int32, false),
    ///     [1_i32, 1, 2].map(Scalar::from),
    /// )?;
    /// let windows = days.window_by("day as date", false)?;
    /// let record = windows.static_field();
    /// // A column that is not a record windows under the `row` root.
    /// assert_eq!((record.name(), record.is_nullable()), ("row", false));
    /// let names: Vec<&str> = record.fields().iter().map(Field::name).collect();
    /// assert_eq!(names, ["date", "windownum", "rownum"]);
    /// assert!(record.fields()[2].is_nullable());
    /// # Ok(())
    /// # }
    /// ```
    pub fn static_field(&self) -> &Field {
        &self.record.field
    }

    /// These windows holding what they view: the windowed serie cloned -
    /// its buffers shared, no row copied - where they borrowed it, so a
    /// holder that outlives the borrow (a binding's window object) keeps
    /// every window and its record, and windows of a window keep it too.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, Serie, SerieWindows};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let windows: SerieWindows<'static> = {
    ///     let days = Serie::from_scalars(
    ///         Field::new("day", DataType::Int32, false),
    ///         [1_i32, 1, 2].map(Scalar::from),
    ///     )?;
    ///     days.window_by("day", false)?.into_owned()
    /// };
    /// let (key, window) = windows.iter().nth(1).expect("a second window");
    /// assert_eq!((key, window.offset()), (Scalar::from_sequence([Scalar::from(2_i32)]), 2));
    /// let record = window.static_values().expect("a window window_by lent");
    /// assert_eq!(record.get_key_str("windownum"), Some(&Scalar::from(1_u64)));
    /// # Ok(())
    /// # }
    /// ```
    pub fn into_owned(self) -> SerieWindows<'static> {
        SerieWindows {
            holder: Cow::Owned(self.holder.into_owned()),
            offset: self.offset,
            keys: self.keys,
            cuts: self.cuts,
            record: self.record,
            opens: self.opens,
        }
    }

    /// The window `index` places from the first, as [`Self::iter`] lends it:
    /// its key, and the window stating its
    /// [record](WindowSerie::static_values); `None` past the last. Constant
    /// time: in row order the first call indexes where every window opens,
    /// once, and every call after it reads that index; in key order the cuts
    /// are the index.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let days = Serie::from_scalars(
    ///     Field::new("day", DataType::Int32, false),
    ///     [1_i32, 1, 2, 3].map(Scalar::from),
    /// )?;
    /// let windows = days.window_by("day", false)?;
    /// let (key, window) = windows.get(2).expect("a third window");
    /// assert_eq!((key, window.offset(), window.len()), (Scalar::from_sequence([Scalar::from(3_i32)]), 3, 1));
    /// let record = window.static_values().expect("a window window_by lent");
    /// assert_eq!(record.get_key_str("windownum"), Some(&Scalar::from(2_u64)));
    /// assert!(windows.get(3).is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn get(&self, index: usize) -> Option<(Scalar, WindowSerie<'_>)> {
        let (key_row, offset, len) = match &self.cuts {
            Cuts::Starts(starts, count) => {
                if index >= *count {
                    return None;
                }
                let opens = self.opens.get_or_init(|| {
                    // Sized by the count: one allocation, never regrown.
                    let mut opens = Vec::with_capacity(*count);
                    opens.extend(starts.set_indices());
                    opens.into_boxed_slice()
                });
                let start = *opens.get(index)?;
                let end = opens.get(index + 1).copied().unwrap_or(starts.len());
                (start, self.offset + start, end - start)
            }
            Cuts::Gathered(cuts) => {
                let (key_row, end) = *cuts.get(index)?;
                let start = index
                    .checked_sub(1)
                    .and_then(|before| cuts.get(before))
                    .map_or(0, |&(_, end)| end as usize);
                (key_row as usize, start, end as usize - start)
            }
        };
        Some((
            proven_row(&self.keys, key_row),
            WindowSerie {
                serie: &self.holder,
                offset,
                len,
                origin: Some(Origin {
                    windows: self,
                    index,
                }),
            },
        ))
    }

    /// Walk the windows from the first, each as its key and the window over
    /// [`Self::serie`]; as often as asked.
    pub fn iter(&self) -> SerieWindowsIter<'_> {
        SerieWindowsIter {
            windows: self,
            index: 0,
            front: 0,
            at: 0,
            remaining: self.len(),
        }
    }
}

impl<'s, 'a: 's> IntoIterator for &'s SerieWindows<'a> {
    type Item = (Scalar, WindowSerie<'s>);
    type IntoIter = SerieWindowsIter<'s>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// The row the window opening at `start` ends before: the next row `starts`
/// opens a window at, else its length. `start` is below the length.
pub(crate) fn window_end(starts: &BooleanBuffer, start: usize) -> usize {
    let len = starts.len();
    BitIndexIterator::new(
        starts.values(),
        starts.offset() + start + 1,
        len - start - 1,
    )
    .next()
    .map_or(len, |next| start + 1 + next)
}

impl<'s> Iterator for SerieWindowsIter<'s> {
    type Item = (Scalar, WindowSerie<'s>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let windows = self.windows;
        let (key_row, offset, len) = match &windows.cuts {
            Cuts::Starts(starts, _) => {
                let start = self.front;
                let end = window_end(starts, start);
                self.front = end;
                (start, windows.offset + start, end - start)
            }
            Cuts::Gathered(cuts) => {
                let (key_row, end) = *cuts.get(self.front)?;
                let (start, end) = (self.at, end as usize);
                self.front += 1;
                self.at = end;
                (key_row as usize, start, end - start)
            }
        };
        self.remaining -= 1;
        let index = self.index;
        self.index += 1;
        Some((
            proven_row(&windows.keys, key_row),
            WindowSerie {
                serie: &windows.holder,
                offset,
                len,
                origin: Some(Origin { windows, index }),
            },
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }

    /// Skip `skip` windows without building their keys - one walk of the
    /// set bits in row order, one read of the cuts in key order - then lend
    /// the next.
    fn nth(&mut self, skip: usize) -> Option<Self::Item> {
        if skip >= self.remaining {
            self.remaining = 0;
            return None;
        }
        if skip > 0 {
            match &self.windows.cuts {
                Cuts::Starts(starts, _) => {
                    // The bit at `front` is set: the window `skip` on opens
                    // at the set bit `skip` places past it.
                    let past = BitIndexIterator::new(
                        starts.values(),
                        starts.offset() + self.front,
                        starts.len() - self.front,
                    )
                    .nth(skip)?;
                    self.front += past;
                }
                Cuts::Gathered(cuts) => {
                    self.at = cuts.get(self.front + skip - 1)?.1 as usize;
                    self.front += skip;
                }
            }
            self.index += skip;
            self.remaining -= skip;
        }
        self.next()
    }
}

impl ExactSizeIterator for SerieWindowsIter<'_> {
    fn len(&self) -> usize {
        self.remaining
    }
}

impl FusedIterator for SerieWindowsIter<'_> {}
