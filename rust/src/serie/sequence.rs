//! The columns a sequence field is stored in: the five list layouts, each
//! an item column under its own cut.
//!
//! [`OffsetListSerie<O>`] is the offsets cut - [`ListSerie`] at 32 bits,
//! [`LargeListSerie`] at 64 - [`OffsetListViewSerie<O>`] the viewed cut
//! with offsets and sizes side by side, and [`FixedSizeListSerie`] no cut
//! at all, every row `width` items. The items are one [`Serie`] of the item
//! field, so a caller reading every item of every row reads one column
//! rather than walking rows, and [`OffsetListSerie::row`] is that column
//! sliced to one row, zero copy. No list leaf has a typed writer: rows are
//! sequences, `splice` is the writer, and the item column is not mutably
//! reachable, which is what keeps the cut and the items aligned.
//!
//! Every cut is rebased - it starts at 0 and ends at the item count - so a
//! column that grows knows where its items end. A write cuts the rows, hands
//! the item column exactly the items they hold over exactly the item range
//! the replaced rows occupied, and splices the validity; a null row is a cut
//! of zero items and a cleared bit, or `width` placeholder items under a
//! fixed-size cut. `check` runs first, over the same cut, so a total the
//! offset type cannot reach or a cell an item column refuses leaves every
//! buffer as it was.

use std::borrow::Cow;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, GenericListArray, GenericListViewArray, OffsetSizeTrait,
    UInt64Array,
};
use arrow_buffer::{ArrowNativeType, MutableBuffer, NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::DataType as ArrowDataType;

use super::{Serie, layout, require_range, require_row, require_window};
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar};

/// The invariant every nested column keeps: its children are aligned, so
/// the Arrow array assembles.
const ALIGNED: &str = "a list column's cut ends at its items: no public path misaligns them";

/// The invariant a write carries in from `check`: it ran on these rows
/// under this field, so a placeholder it built builds again.
const CHECKED: &str = "check ran on these rows: a placeholder it built, write builds again";

/// Which leaves of the root one offset width widens to.
pub trait ListLeaf: OffsetSizeTrait + Send + Sync + 'static {
    /// The offsets leaf's name, as its debug rendering spells it.
    const NAME: &'static str;
    /// The viewed leaf's name, as its debug rendering spells it.
    const VIEW_NAME: &'static str;

    /// Widen an offsets column of this width to the serie root.
    fn into_list_serie(column: OffsetListSerie<Self>) -> Serie;

    /// Narrow a serie root to an offsets column of this width.
    fn from_list_serie(serie: &Serie) -> Option<&OffsetListSerie<Self>>;

    /// Widen a viewed column of this width to the serie root.
    fn into_list_view_serie(column: OffsetListViewSerie<Self>) -> Serie;

    /// Narrow a serie root to a viewed column of this width.
    fn from_list_view_serie(serie: &Serie) -> Option<&OffsetListViewSerie<Self>>;
}

/// The item field a list-shaped field repeats, as Arrow spells it.
fn item_field_ref(field: &Field) -> arrow_schema::FieldRef {
    match field.as_arrow_field_ref().expect(ALIGNED).data_type() {
        ArrowDataType::List(item)
        | ArrowDataType::LargeList(item)
        | ArrowDataType::ListView(item)
        | ArrowDataType::LargeListView(item)
        | ArrowDataType::FixedSizeList(item, _) => Arc::clone(item),
        _ => unreachable!("a sequence field projects to a list layout"),
    }
}

/// The item field a sequence field repeats.
fn item_field(field: &Field) -> &Arc<Field> {
    match field.dtype() {
        DataType::Sequence(sequence) => sequence.item_ref(),
        _ => unreachable!("a sequence column's field is a sequence field: the door paired them"),
    }
}

/// Move or copy one row's cells onto `items`.
fn extend_cells(items: &mut Vec<Scalar>, cells: Cow<'_, [Scalar]>) {
    match cells {
        Cow::Borrowed(held) => items.extend_from_slice(held),
        Cow::Owned(built) => items.extend(built),
    }
}

/// The items canonical `rows` hold, in row order, and how many each row
/// holds: an absent row holds none.
///
/// A canonical row is [`Scalar::Null`] or a sequence - a run, or a column of
/// the item field whose rows are already proven - so its items are read for
/// what they mean and never borrowed for what they happen to store.
fn items_of(rows: &[Scalar]) -> (Vec<usize>, Vec<Scalar>) {
    let mut lengths = Vec::with_capacity(rows.len());
    let mut items = Vec::new();
    for row in rows {
        match row.sequence_rows() {
            Some(cells) => {
                lengths.push(cells.len());
                extend_cells(&mut items, cells);
            }
            None => lengths.push(0),
        }
    }
    (lengths, items)
}

/// Which of canonical `rows` are present.
fn present(rows: &[Scalar]) -> Vec<bool> {
    rows.iter().map(|row| !row.is_null()).collect()
}

/// Which of `len` rows a validity bitmap marks present.
fn validity(nulls: Option<&NullBuffer>, len: usize) -> Vec<bool> {
    (0..len)
        .map(|row| nulls.is_none_or(|nulls| nulls.is_valid(row)))
        .collect()
}

/// `buffer` with `values` on its end: written into its own bytes where the
/// column holds them alone and their pointer was never advanced, copied
/// once where it does not.
fn extended<O: OffsetSizeTrait>(buffer: ScalarBuffer<O>, values: &[O]) -> ScalarBuffer<O> {
    let mut bytes = match buffer.into_inner().into_mutable() {
        Ok(owned) => owned,
        Err(shared) => {
            let mut owned =
                MutableBuffer::with_capacity(shared.len() + std::mem::size_of_val(values));
            owned.extend_from_slice(shared.typed_data::<O>());
            owned
        }
    };
    bytes.extend_from_slice(values);
    ScalarBuffer::from(bytes)
}

// ------------------------------------------------------------------------
// Offsets: one monotone cut over the item column.
// ------------------------------------------------------------------------

/// One column of sequences: the offsets that cut it, and the item column
/// under them.
///
/// The cut is rebased: its first offset is 0 and its last is the item
/// count, so a column that grows knows where its items end.
pub struct OffsetListSerie<O: OffsetSizeTrait> {
    field: Arc<Field>,
    offsets: OffsetBuffer<O>,
    items: Serie,
    nulls: Option<NullBuffer>,
}

/// A column of sequences, 32-bit offsets.
pub type ListSerie = OffsetListSerie<i32>;

/// A column of sequences, 64-bit offsets.
pub type LargeListSerie = OffsetListSerie<i64>;

impl<O: OffsetSizeTrait> OffsetListSerie<O> {
    /// Pair a sequence field with its cut and the column it cuts.
    pub(crate) const fn new(
        field: Arc<Field>,
        offsets: OffsetBuffer<O>,
        items: Serie,
        nulls: Option<NullBuffer>,
    ) -> Self {
        Self {
            field,
            offsets,
            items,
            nulls,
        }
    }

    /// Borrow the item column every row is cut out of.
    pub const fn items(&self) -> &Serie {
        &self.items
    }

    /// Borrow the offsets buffer that cuts the items, without copying it.
    pub const fn offsets(&self) -> &OffsetBuffer<O> {
        &self.offsets
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Return the item range row `index` occupies: `None` when the row is
    /// absent or past the end.
    pub fn range(&self, index: usize) -> Option<Range<usize>> {
        let start = (*self.offsets.get(index)?).as_usize();
        let end = (*self.offsets.get(index + 1)?).as_usize();
        if self
            .nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index))
        {
            return None;
        }
        Some(start..end)
    }

    /// Return row `index` as the item column sliced to it, zero copy.
    pub fn row(&self, index: usize) -> Option<Serie> {
        let range = self.range(index)?;
        self.items.slice(range.start, range.len()).ok()
    }

    /// The item range rows `range` occupy, an absent row occupying none;
    /// `range` is one `require_range` admitted.
    fn cut(&self, range: &Range<usize>) -> Range<usize> {
        self.offsets[range.start].as_usize()..self.offsets[range.end].as_usize()
    }

    /// Refuse what a write could not do: an item total past the offset
    /// type, and whatever the items refuse over the cut they will receive.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let (_, items) = items_of(rows);
        let replaced = self.cut(range);
        layout::require_offset::<O>(
            self.field.name(),
            self.items.len() - replaced.len() + items.len(),
        )?;
        self.items.check(&replaced, &items)
    }

    /// Write canonical `rows` over a checked `range`: the cut re-cut from
    /// `range.start` on, the items spliced over the item range the replaced
    /// rows occupied, the validity spliced.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let len = self.offsets.len() - 1;
        let present = present(&rows);
        let (lengths, items) = items_of(&rows);
        let offsets = std::mem::replace(&mut self.offsets, OffsetBuffer::new_empty());
        let (offsets, replaced) = layout::splice_offsets(offsets, range.clone(), &lengths);
        self.offsets = offsets;
        self.items.write(replaced, items);
        self.nulls = layout::splice_nulls(self.nulls.take(), len, range, &present);
    }

    /// Append `other`'s cut and items, whose field agrees with this one's,
    /// answering whether the offset type reaches the total and the items
    /// appended; `false` leaves this column as it was, and the root then
    /// reads the rows so the splice refuses by name.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let len = self.offsets.len() - 1;
        let total = self.items.len() + other.items.len();
        if layout::require_offset::<O>(self.field.name(), total).is_err()
            || !self.items.append(&other.items)
        {
            return false;
        }
        let lengths: Vec<usize> = other
            .offsets
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).as_usize())
            .collect();
        let offsets = std::mem::replace(&mut self.offsets, OffsetBuffer::new_empty());
        self.offsets = layout::splice_offsets(offsets, len..len, &lengths).0;
        self.nulls = layout::splice_nulls(
            self.nulls.take(),
            len,
            len..len,
            &validity(other.nulls.as_ref(), other.offsets.len() - 1),
        );
        true
    }
}

impl<O: ListLeaf> SerieValue for OffsetListSerie<O> {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.len())?;
        Ok(self
            .nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index)))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.len())?;
        let Some(range) = self.range(index) else {
            return Ok(Scalar::Null);
        };
        Scalar::try_sequence(range.len(), |item| self.items.scalar(range.start + item))
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.len())?;
        let first = self.offsets[offset];
        let last = self.offsets[offset + length];
        let rebased: Vec<O> = self.offsets[offset..=offset + length]
            .iter()
            .map(|held| *held - first)
            .collect();
        Ok(Self::new(
            Arc::clone(&self.field),
            OffsetBuffer::new(ScalarBuffer::from(rebased)),
            self.items
                .slice(first.as_usize(), (last - first).as_usize())?,
            self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
        ))
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(
            GenericListArray::<O>::try_new(
                item_field_ref(&self.field),
                self.offsets.clone(),
                self.items.into_arrow_array().expect(ALIGNED),
                self.nulls.clone(),
            )
            .expect(ALIGNED),
        )
    }

    fn into_serie(self) -> Serie {
        O::into_list_serie(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        O::from_list_serie(value)
    }
}

impl<O: OffsetSizeTrait> Clone for OffsetListSerie<O> {
    fn clone(&self) -> Self {
        Self::new(
            Arc::clone(&self.field),
            self.offsets.clone(),
            self.items.clone(),
            self.nulls.clone(),
        )
    }
}

impl<O: ListLeaf> fmt::Debug for OffsetListSerie<O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, O::NAME, formatter)
    }
}

serie_leaf!(OffsetListSerie<O: ListLeaf>);

// ------------------------------------------------------------------------
// Views: offsets and sizes side by side, kept compact.
// ------------------------------------------------------------------------

/// One column of sequences held as views: each row an offset and a size
/// into the item column.
///
/// Arrow lets views overlap, reorder and leave items unreached; a column
/// does not. Its views are compact: row `i` starts where row `i - 1` ended,
/// the first at 0, the last ending at the item count, and an absent row
/// views nothing. The door rebases a foreign layout onto exactly the items
/// it reaches, a slice rebases its window, and every write keeps the cut
/// compact - so a write is the offsets cut's write, with the sizes beside
/// it.
pub struct OffsetListViewSerie<O: OffsetSizeTrait> {
    field: Arc<Field>,
    offsets: ScalarBuffer<O>,
    sizes: ScalarBuffer<O>,
    items: Serie,
    nulls: Option<NullBuffer>,
}

/// A column of sequence views, 32-bit offsets.
pub type ListViewSerie = OffsetListViewSerie<i32>;

/// A column of sequence views, 64-bit offsets.
pub type LargeListViewSerie = OffsetListViewSerie<i64>;

impl<O: OffsetSizeTrait> OffsetListViewSerie<O> {
    /// Pair a sequence field with its compact views and the column they
    /// view.
    pub(crate) const fn new(
        field: Arc<Field>,
        offsets: ScalarBuffer<O>,
        sizes: ScalarBuffer<O>,
        items: Serie,
        nulls: Option<NullBuffer>,
    ) -> Self {
        Self {
            field,
            offsets,
            sizes,
            items,
            nulls,
        }
    }

    /// Borrow the item column every row views into.
    pub const fn items(&self) -> &Serie {
        &self.items
    }

    /// Borrow the offsets buffer, one per row, without copying it.
    pub const fn offsets(&self) -> &ScalarBuffer<O> {
        &self.offsets
    }

    /// Borrow the sizes buffer, one per row, without copying it.
    pub const fn sizes(&self) -> &ScalarBuffer<O> {
        &self.sizes
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Return the item range row `index` views: `None` when the row is
    /// absent or past the end.
    pub fn range(&self, index: usize) -> Option<Range<usize>> {
        let start = (*self.offsets.get(index)?).as_usize();
        let size = (*self.sizes.get(index)?).as_usize();
        if self
            .nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index))
        {
            return None;
        }
        Some(start..start + size)
    }

    /// Return row `index` as the item column sliced to it, zero copy.
    pub fn row(&self, index: usize) -> Option<Serie> {
        let range = self.range(index)?;
        self.items.slice(range.start, range.len()).ok()
    }

    /// The item range rows `range` occupy: the views are compact, so a row
    /// starts at its offset and the row past the end starts at the item
    /// count; `range` is one `require_range` admitted.
    fn cut(&self, range: &Range<usize>) -> Range<usize> {
        let start = |row: usize| {
            self.offsets
                .get(row)
                .map_or(self.items.len(), |offset| offset.as_usize())
        };
        start(range.start)..start(range.end)
    }

    /// Re-cut rows `range` as `lengths` rows of items from `start` on, the
    /// rows after them following on: appended into the buffers the column
    /// holds at the end, rebuilt from `range.start` elsewhere.
    fn recut(&mut self, range: &Range<usize>, start: usize, lengths: &[usize]) {
        let len = self.offsets.len();
        let mut next = start;
        let mut offsets = Vec::with_capacity(lengths.len());
        let mut sizes = Vec::with_capacity(lengths.len());
        for items in lengths {
            offsets.push(O::usize_as(next));
            sizes.push(O::usize_as(*items));
            next += items;
        }
        if range.start == len {
            self.offsets = extended(std::mem::take(&mut self.offsets), &offsets);
            self.sizes = extended(std::mem::take(&mut self.sizes), &sizes);
            return;
        }
        let kept = len - range.len() + lengths.len();
        let mut rebuilt_offsets: Vec<O> = Vec::with_capacity(kept);
        let mut rebuilt_sizes: Vec<O> = Vec::with_capacity(kept);
        rebuilt_offsets.extend_from_slice(&self.offsets[..range.start]);
        rebuilt_sizes.extend_from_slice(&self.sizes[..range.start]);
        rebuilt_offsets.extend(offsets);
        rebuilt_sizes.extend(sizes);
        for size in &self.sizes[range.end..] {
            rebuilt_offsets.push(O::usize_as(next));
            rebuilt_sizes.push(*size);
            next += size.as_usize();
        }
        self.offsets = ScalarBuffer::from(rebuilt_offsets);
        self.sizes = ScalarBuffer::from(rebuilt_sizes);
    }

    /// Refuse what a write could not do: an item total past the offset
    /// type, and whatever the items refuse over the cut they will receive.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let (_, items) = items_of(rows);
        let replaced = self.cut(range);
        layout::require_offset::<O>(
            self.field.name(),
            self.items.len() - replaced.len() + items.len(),
        )?;
        self.items.check(&replaced, &items)
    }

    /// Write canonical `rows` over a checked `range`, keeping the views
    /// compact: the items spliced over the item range the replaced rows
    /// viewed, the views re-cut from `range.start` on, the validity
    /// spliced.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let len = self.offsets.len();
        let present = present(&rows);
        let (lengths, items) = items_of(&rows);
        let replaced = self.cut(&range);
        self.items.write(replaced.clone(), items);
        self.recut(&range, replaced.start, &lengths);
        self.nulls = layout::splice_nulls(self.nulls.take(), len, range, &present);
    }

    /// Append `other`'s views and items, whose field agrees with this one's,
    /// answering whether the offset type reaches the total and the items
    /// appended; `false` leaves this column as it was.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let len = self.offsets.len();
        let prior = self.items.len();
        if layout::require_offset::<O>(self.field.name(), prior + other.items.len()).is_err()
            || !self.items.append(&other.items)
        {
            return false;
        }
        let shifted: Vec<O> = other
            .offsets
            .iter()
            .map(|offset| *offset + O::usize_as(prior))
            .collect();
        self.offsets = extended(std::mem::take(&mut self.offsets), &shifted);
        self.sizes = extended(std::mem::take(&mut self.sizes), &other.sizes);
        self.nulls = layout::splice_nulls(
            self.nulls.take(),
            len,
            len..len,
            &validity(other.nulls.as_ref(), other.offsets.len()),
        );
        true
    }
}

impl<O: ListLeaf> SerieValue for OffsetListViewSerie<O> {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.offsets.len()
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.len())?;
        Ok(self
            .nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index)))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.len())?;
        let Some(range) = self.range(index) else {
            return Ok(Scalar::Null);
        };
        Scalar::try_sequence(range.len(), |item| self.items.scalar(range.start + item))
    }

    /// The window's views rebased onto the items they reach, which are
    /// contiguous because the views are compact.
    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.len())?;
        let reached = self.cut(&(offset..offset + length));
        let rebased: Vec<O> = self.offsets[offset..offset + length]
            .iter()
            .map(|held| *held - O::usize_as(reached.start))
            .collect();
        Ok(Self::new(
            Arc::clone(&self.field),
            ScalarBuffer::from(rebased),
            self.sizes.slice(offset, length),
            self.items.slice(reached.start, reached.len())?,
            self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
        ))
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(
            GenericListViewArray::<O>::try_new(
                item_field_ref(&self.field),
                self.offsets.clone(),
                self.sizes.clone(),
                self.items.into_arrow_array().expect(ALIGNED),
                self.nulls.clone(),
            )
            .expect(ALIGNED),
        )
    }

    fn into_serie(self) -> Serie {
        O::into_list_view_serie(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        O::from_list_view_serie(value)
    }
}

impl<O: OffsetSizeTrait> Clone for OffsetListViewSerie<O> {
    fn clone(&self) -> Self {
        Self::new(
            Arc::clone(&self.field),
            self.offsets.clone(),
            self.sizes.clone(),
            self.items.clone(),
            self.nulls.clone(),
        )
    }
}

impl<O: ListLeaf> fmt::Debug for OffsetListViewSerie<O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, O::VIEW_NAME, formatter)
    }
}

serie_leaf!(OffsetListViewSerie<O: ListLeaf>);

/// Tie one offset width to the two leaves it widens through.
macro_rules! list_leaf {
    ($offset:ty, $list:ident, $view:ident) => {
        impl ListLeaf for $offset {
            const NAME: &'static str = stringify!($list);
            const VIEW_NAME: &'static str = stringify!($view);

            fn into_list_serie(column: OffsetListSerie<Self>) -> Serie {
                Serie::Sequence(Arc::new(super::SequenceSerie::$list(column)))
            }

            fn from_list_serie(serie: &Serie) -> Option<&OffsetListSerie<Self>> {
                match serie {
                    Serie::Sequence(family) => match family.as_ref() {
                        super::SequenceSerie::$list(column) => Some(column),
                        _ => None,
                    },
                    _ => None,
                }
            }

            fn into_list_view_serie(column: OffsetListViewSerie<Self>) -> Serie {
                Serie::Sequence(Arc::new(super::SequenceSerie::$view(column)))
            }

            fn from_list_view_serie(serie: &Serie) -> Option<&OffsetListViewSerie<Self>> {
                match serie {
                    Serie::Sequence(family) => match family.as_ref() {
                        super::SequenceSerie::$view(column) => Some(column),
                        _ => None,
                    },
                    _ => None,
                }
            }
        }
    };
}

list_leaf!(i32, List, ListView);
list_leaf!(i64, LargeList, LargeListView);

// ------------------------------------------------------------------------
// Fixed size: no cut, every row `width` items.
// ------------------------------------------------------------------------

/// One column of fixed-size sequences: `width` items per row, and no cut.
///
/// The items hold `len * width` rows; a null row holds `width` placeholder
/// items and a cleared bit.
#[derive(Clone)]
pub struct FixedSizeListSerie {
    field: Arc<Field>,
    width: usize,
    items: Serie,
    nulls: Option<NullBuffer>,
    rows: usize,
}

impl FixedSizeListSerie {
    /// Pair a fixed-size sequence field with the column its rows tile.
    pub(crate) const fn new(
        field: Arc<Field>,
        width: usize,
        items: Serie,
        nulls: Option<NullBuffer>,
        rows: usize,
    ) -> Self {
        Self {
            field,
            width,
            items,
            nulls,
            rows,
        }
    }

    /// Return how many items every row holds.
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Borrow the item column every row tiles.
    pub const fn items(&self) -> &Serie {
        &self.items
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Return the item range row `index` occupies: `None` when the row is
    /// absent or past the end.
    pub fn range(&self, index: usize) -> Option<Range<usize>> {
        if index >= self.rows
            || self
                .nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(index))
        {
            return None;
        }
        Some(index * self.width..(index + 1) * self.width)
    }

    /// Return row `index` as the item column sliced to it, zero copy.
    pub fn row(&self, index: usize) -> Option<Serie> {
        let range = self.range(index)?;
        self.items.slice(range.start, range.len()).ok()
    }

    /// The items canonical `rows` tile, in row order: a row's own `width`
    /// items, or `width` placeholders under an absent row.
    ///
    /// The placeholder is the crate's one answer to what occupies a slot a
    /// parent null hides, built once and only when a row is absent.
    fn tiles_of(&self, rows: &[Scalar]) -> crate::arrow::Result<Vec<Scalar>> {
        let placeholder = rows
            .iter()
            .any(Scalar::is_null)
            .then(|| crate::arrow::value::physical_placeholder_for_field(item_field(&self.field)))
            .transpose()?
            .unwrap_or(Scalar::Null);
        let mut items = Vec::with_capacity(rows.len() * self.width);
        for row in rows {
            match row.sequence_rows() {
                Some(cells) => extend_cells(&mut items, cells),
                None => items.extend(std::iter::repeat_n(placeholder.clone(), self.width)),
            }
        }
        Ok(items)
    }

    /// Refuse what a write could not do: whatever the items refuse over the
    /// tiles they will receive.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let tiles = self.tiles_of(rows)?;
        self.items
            .check(&(range.start * self.width..range.end * self.width), &tiles)
    }

    /// Write canonical `rows` over a checked `range`: `width` items per row
    /// spliced over the tiles the replaced rows occupied, the validity
    /// spliced, the count moved.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let present = present(&rows);
        let tiles = self.tiles_of(&rows).expect(CHECKED);
        self.items
            .write(range.start * self.width..range.end * self.width, tiles);
        self.nulls = layout::splice_nulls(self.nulls.take(), self.rows, range.clone(), &present);
        self.rows = self.rows - range.len() + rows.len();
    }

    /// Append `other`'s items, whose field agrees with this one's, answering
    /// whether they appended; `false` leaves this column as it was.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        if !self.items.append(&other.items) {
            return false;
        }
        self.nulls = layout::splice_nulls(
            self.nulls.take(),
            self.rows,
            self.rows..self.rows,
            &validity(other.nulls.as_ref(), other.rows),
        );
        self.rows += other.rows;
        true
    }
}

impl SerieValue for FixedSizeListSerie {
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
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.rows)?;
        Ok(self
            .nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index)))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.rows)?;
        let Some(range) = self.range(index) else {
            return Ok(Scalar::Null);
        };
        Scalar::try_sequence(range.len(), |item| self.items.scalar(range.start + item))
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.rows)?;
        Ok(Self::new(
            Arc::clone(&self.field),
            self.width,
            self.items.slice(offset * self.width, length * self.width)?,
            self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
            length,
        ))
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
        let width = i32::try_from(self.width).expect(ALIGNED);
        Arc::new(
            FixedSizeListArray::try_new(
                item_field_ref(&self.field),
                width,
                self.items.into_arrow_array().expect(ALIGNED),
                self.nulls.clone(),
            )
            .expect(ALIGNED),
        )
    }

    fn into_serie(self) -> Serie {
        Serie::Sequence(Arc::new(super::SequenceSerie::FixedSizeList(self)))
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Sequence(family) => match family.as_ref() {
                super::SequenceSerie::FixedSizeList(column) => Some(column),
                _ => None,
            },
            _ => None,
        }
    }
}

impl fmt::Debug for FixedSizeListSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, "FixedSizeListSerie", formatter)
    }
}

serie_leaf!(FixedSizeListSerie);

impl super::SequenceSerie {
    /// Borrow the item column under whichever cut this leaf is.
    pub fn items(&self) -> &Serie {
        match self {
            Self::List(column) => column.items(),
            Self::LargeList(column) => column.items(),
            Self::ListView(column) => column.items(),
            Self::LargeListView(column) => column.items(),
            Self::FixedSizeList(column) => column.items(),
        }
    }
}

// ------------------------------------------------------------------------
// The door: each layout's buffers taken as they are, its cut rebased.
// ------------------------------------------------------------------------

/// The span the rows' views run over when each starts where the one before
/// ended, an absent row viewing nothing there; `None` when one does not.
fn contiguous_span<O: OffsetSizeTrait>(
    offsets: &[O],
    sizes: &[O],
    nulls: Option<&NullBuffer>,
) -> Option<Range<usize>> {
    let mut first = None;
    let mut next = 0;
    for (row, (offset, size)) in offsets.iter().zip(sizes).enumerate() {
        let (offset, size) = (offset.as_usize(), size.as_usize());
        if nulls.is_some_and(|nulls| nulls.is_null(row)) && size != 0 {
            return None;
        }
        match first {
            None => {
                first = Some(offset);
                next = offset + size;
            }
            Some(_) if next == offset => next += size,
            Some(_) => return None,
        }
    }
    Some(first.unwrap_or(0)..next)
}

/// The views as one compact cut: contiguous from 0, an absent row viewing
/// nothing, ending at exactly the items the rows reach.
///
/// Untouched when they already are. A cut that is contiguous but starts
/// past zero - a sliced array - slices the items to its span and shifts,
/// copying nothing. Anything else - rows overlapping, out of order or
/// leaving items unreached - gathers the reached items in row order once,
/// because a column that grows has to know where its items end.
fn rebased_views<O: OffsetSizeTrait>(
    offsets: &ScalarBuffer<O>,
    sizes: &ScalarBuffer<O>,
    nulls: Option<&NullBuffer>,
    values: &ArrayRef,
) -> crate::arrow::Result<(ScalarBuffer<O>, ScalarBuffer<O>, ArrayRef)> {
    match contiguous_span(offsets, sizes, nulls) {
        Some(span) if span == (0..values.len()) => {
            Ok((offsets.clone(), sizes.clone(), Arc::clone(values)))
        }
        Some(span) => {
            let shifted: Vec<O> = offsets
                .iter()
                .map(|offset| *offset - O::usize_as(span.start))
                .collect();
            Ok((
                ScalarBuffer::from(shifted),
                sizes.clone(),
                values.slice(span.start, span.len()),
            ))
        }
        None => {
            let mut indices: Vec<u64> = Vec::new();
            let mut rebuilt_offsets = Vec::with_capacity(offsets.len());
            let mut rebuilt_sizes = Vec::with_capacity(offsets.len());
            for (row, (offset, size)) in offsets.iter().zip(sizes.iter()).enumerate() {
                rebuilt_offsets.push(O::usize_as(indices.len()));
                let size = if nulls.is_some_and(|nulls| nulls.is_null(row)) {
                    0
                } else {
                    size.as_usize()
                };
                let start = offset.as_usize();
                indices.extend((start..start + size).map(u64::usize_as));
                rebuilt_sizes.push(O::usize_as(size));
            }
            let gathered =
                arrow_select::take::take(values.as_ref(), &UInt64Array::from(indices), None)?;
            Ok((
                ScalarBuffer::from(rebuilt_offsets),
                ScalarBuffer::from(rebuilt_sizes),
                gathered,
            ))
        }
    }
}

/// The viewed column of one offset width: its views rebased, its items
/// through the door with the item field and no parent.
fn view_column<O: ListLeaf>(
    field: Arc<Field>,
    item: Arc<Field>,
    array: &ArrayRef,
    proven: bool,
) -> crate::arrow::Result<Serie> {
    let views = super::arrow::held::<GenericListViewArray<O>>(array)?;
    let (offsets, sizes, values) = rebased_views(
        views.offsets(),
        views.sizes(),
        views.nulls(),
        views.values(),
    )?;
    let items = super::arrow::column_of(item, values, None, proven)?;
    Ok(OffsetListViewSerie::new(field, offsets, sizes, items, views.nulls().cloned()).into_serie())
}

/// Build the column `field` types out of a list-layout array, or answer
/// `None` for a layout that is not one.
///
/// An offsets cut is rebased onto exactly the items it reaches, and a
/// viewed cut made compact; their items take the door with the item field
/// and no parent, because an absent row reaches no item. A fixed-size cut
/// hides `width` item slots under every absent row - the placeholders a
/// write tiles there - so its items take the list's validity, expanded by
/// `width`, as their parent and a required item is judged only where the
/// row is present.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proven: bool,
) -> crate::arrow::Result<Option<Serie>> {
    use super::arrow::{held, rebased};

    let _ = parent;
    let Some(sequence) = field.dtype().as_sequence_type() else {
        return Ok(None);
    };
    let item = Arc::clone(sequence.item_ref());
    Ok(Some(match array.data_type() {
        ArrowDataType::List(_) => {
            let lists = held::<GenericListArray<i32>>(&array)?;
            let (offsets, values) = rebased(lists.offsets(), lists.values());
            let items = super::arrow::column_of(item, values, None, proven)?;
            OffsetListSerie::new(field, offsets, items, lists.nulls().cloned()).into_serie()
        }
        ArrowDataType::LargeList(_) => {
            let lists = held::<GenericListArray<i64>>(&array)?;
            let (offsets, values) = rebased(lists.offsets(), lists.values());
            let items = super::arrow::column_of(item, values, None, proven)?;
            OffsetListSerie::new(field, offsets, items, lists.nulls().cloned()).into_serie()
        }
        ArrowDataType::ListView(_) => view_column::<i32>(field, item, &array, proven)?,
        ArrowDataType::LargeListView(_) => view_column::<i64>(field, item, &array, proven)?,
        ArrowDataType::FixedSizeList(_, width) => {
            let lists = held::<FixedSizeListArray>(&array)?;
            let width = usize::try_from(*width).map_err(|_| crate::arrow::Error::Internal {
                site: "serie::sequence::width",
            })?;
            let values = lists
                .values()
                .slice(lists.offset() * width, lists.len() * width);
            let hidden = lists.nulls().map(|nulls| nulls.expand(width));
            let items = super::arrow::column_of(item, values, hidden.as_ref(), proven)?;
            FixedSizeListSerie::new(field, width, items, lists.nulls().cloned(), lists.len())
                .into_serie()
        }
        _ => return Ok(None),
    }))
}
