//! Every column whose rows hold other rows.
//!
//! A nested layout in Arrow is its own buffers plus the columns under them,
//! so that is what these hold: [`StructSerie`] keeps one [`Serie`] per child
//! field, [`ListSerie`] keeps the offsets and the item column they cut, and
//! [`MapSerie`] keeps the offsets and the entry column. Reaching a child is
//! therefore a borrow rather than a projection, and adding or dropping one
//! is one column rather than one row.

use std::fmt;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, LargeListArray, ListArray, MapArray, StructArray, new_empty_array,
};
use arrow_buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::Fields;

use super::Serie;
use crate::value::SerieValue;
use crate::{Field, Result, Scalar};

/// The validity bitmap `nulls` is, with row `index` marked present or absent.
///
/// A record's own validity is the one buffer a null row writes: Arrow leaves
/// the children's slots unspecified under it, so nothing below is touched.
fn with_validity(
    nulls: Option<&NullBuffer>,
    rows: usize,
    index: usize,
    present: bool,
) -> Option<NullBuffer> {
    if present && nulls.is_none() {
        return None;
    }
    let mut kept: Vec<bool> = (0..rows)
        .map(|row| nulls.is_none_or(|nulls| nulls.is_valid(row)))
        .collect();
    if let Some(slot) = kept.get_mut(index) {
        *slot = present;
    }
    Some(NullBuffer::from(kept))
}

/// The validity bitmap `nulls` is, with one more row on the end.
fn pushed_validity(nulls: Option<&NullBuffer>, rows: usize, present: bool) -> Option<NullBuffer> {
    if present && nulls.is_none() {
        return None;
    }
    let mut kept: Vec<bool> = (0..rows)
        .map(|row| nulls.is_none_or(|nulls| nulls.is_valid(row)))
        .collect();
    kept.push(present);
    Some(NullBuffer::from(kept))
}

/// One column of records: the field that types them, and one column per
/// child field.
///
/// The children are columns, not projections, so
/// [`child`](Self::child) is a borrow and
/// [`without_child`](Self::without_child) keeps every other child's buffers
/// exactly as they were.
#[derive(Clone)]
pub struct StructSerie {
    field: Arc<Field>,
    children: Vec<Serie>,
    nulls: Option<NullBuffer>,
    rows: usize,
}

impl StructSerie {
    /// Pair a record field with one column per child.
    pub(crate) const fn new(
        field: Arc<Field>,
        children: Vec<Serie>,
        nulls: Option<NullBuffer>,
        rows: usize,
    ) -> Self {
        Self {
            field,
            children,
            nulls,
            rows,
        }
    }

    /// Borrow every child column, in the field's own order.
    pub fn children(&self) -> &[Serie] {
        &self.children
    }

    /// Borrow one child column by name.
    pub fn child(&self, name: &str) -> Option<&Serie> {
        self.children.get(self.field.index_of(name)?)
    }

    /// Borrow child `index`.
    pub fn child_at(&self, index: usize) -> Option<&Serie> {
        self.children.get(index)
    }

    /// Borrow one child column by name, to write into it.
    pub fn child_mut(&mut self, name: &str) -> Option<&mut Serie> {
        let index = self.field.index_of(name)?;
        self.children.get_mut(index)
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Return this column with `child` added, or replacing the child of its
    /// name.
    ///
    /// Every other child's buffers are shared, not rebuilt, and no row is
    /// read.
    ///
    /// # Errors
    ///
    /// Returns an error when `child` does not hold exactly as many rows as
    /// this column does, or when the field refuses it.
    pub fn with_child(&self, child: &Serie) -> Result<Self> {
        if child.len() != self.rows {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "a child of {} rows does not fit a column of {}",
                    child.len(),
                    self.rows
                ),
            });
        }
        let name = child.field().name().to_owned();
        let mut field = self.field.as_ref().clone();
        field.set_field(name.as_str(), child.field().clone())?;
        let mut children = self.children.clone();
        match field.index_of(name.as_str()) {
            Some(index) if index < children.len() => children[index] = child.clone(),
            Some(_) | None => children.push(child.clone()),
        }
        Ok(Self::new(
            Arc::new(field),
            children,
            self.nulls.clone(),
            self.rows,
        ))
    }

    /// Return this column with the child of `name` dropped.
    ///
    /// # Errors
    ///
    /// Returns an error when no child is named `name`.
    pub fn without_child(&self, name: &str) -> Result<Self> {
        let mut field = self.field.as_ref().clone();
        let index = field
            .index_of(name)
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!("no child named {name:?} in this column"),
            })?;
        field.remove_field_at(index)?;
        let mut children = self.children.clone();
        children.remove(index);
        Ok(Self::new(
            Arc::new(field),
            children,
            self.nulls.clone(),
            self.rows,
        ))
    }

    /// The cells of one canonical row, refused where they do not fit.
    fn cells(&self, row: &Scalar) -> Result<Vec<Scalar>> {
        let cells = row.as_sequence().unwrap_or_default();
        if cells.len() != self.children.len() {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "a row of {} cells does not fit a record of {}",
                    cells.len(),
                    self.children.len()
                ),
            });
        }
        Ok(cells.to_vec())
    }

    /// The Arrow child fields this column projects to.
    fn arrow_fields(&self) -> Result<Fields> {
        match crate::field::arrow_field_ref_from_shared(Arc::clone(&self.field))?.data_type() {
            arrow_schema::DataType::Struct(fields) => Ok(fields.clone()),
            other => Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!("expected a struct datatype, got {other}"),
            }),
        }
    }
}

impl SerieValue for StructSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.rows
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.rows
            || self
                .nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        if self.is_null(index) {
            return Ok(Scalar::Null);
        }
        let cells = self
            .children
            .iter()
            .map(|child| child.scalar(index))
            .collect::<Result<Vec<Scalar>>>()?;
        Ok(Scalar::from_sequence(cells))
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.rows)?;
        let row = self.field.scalar(value)?;
        if row.is_null() {
            self.nulls = with_validity(self.nulls.as_ref(), self.rows, index, false);
            return Ok(());
        }
        let cells = self.cells(&row)?;
        for (child, cell) in self.children.iter_mut().zip(cells) {
            child.set(index, cell)?;
        }
        self.nulls = with_validity(self.nulls.as_ref(), self.rows, index, true);
        Ok(())
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let row = self.field.scalar(value)?;
        // A null record row still occupies one slot in every child, because
        // Arrow's children are as long as the record is; what it holds there
        // is unspecified, so the child's own default is what goes in.
        let present = !row.is_null();
        let cells = if present {
            self.cells(&row)?
        } else {
            Vec::new()
        };
        for (index, child) in self.children.iter_mut().enumerate() {
            match cells.get(index) {
                Some(cell) => child.push(cell.clone())?,
                None => {
                    let filler = child.field().default_value()?;
                    child.push(filler)?;
                }
            }
        }
        self.nulls = pushed_validity(self.nulls.as_ref(), self.rows, present);
        self.rows += 1;
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        let Ok(fields) = self.arrow_fields() else {
            return new_empty_array(&arrow_schema::DataType::Null);
        };
        let columns: Vec<ArrayRef> = self
            .children
            .iter()
            .map(SerieValue::into_arrow_array)
            .collect();
        if columns.is_empty() {
            return Arc::new(StructArray::new_empty_fields(self.rows, self.nulls.clone()));
        }
        StructArray::try_new(fields, columns, self.nulls.clone()).map_or_else(
            |_| new_empty_array(&arrow_schema::DataType::Null),
            |array| Arc::new(array) as ArrayRef,
        )
    }

    fn into_serie(self) -> Serie {
        Serie::Struct(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Struct(column) => Some(column),
            _ => None,
        }
    }
}

impl fmt::Debug for StructSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "StructSerie", formatter)
    }
}

serie_leaf!(StructSerie);

/// The offsets a list column cuts its item column by.
///
/// Arrow spells them at two widths and a column keeps whichever it was given,
/// because rewriting them would copy every row to say the same thing.
#[derive(Clone, Debug)]
pub(crate) enum ListOffsets {
    /// 32-bit offsets.
    Small(OffsetBuffer<i32>),
    /// 64-bit offsets.
    Large(OffsetBuffer<i64>),
}

impl ListOffsets {
    /// The rows this cut names.
    fn len(&self) -> usize {
        match self {
            Self::Small(offsets) => offsets.len().saturating_sub(1),
            Self::Large(offsets) => offsets.len().saturating_sub(1),
        }
    }

    /// The item range row `index` occupies.
    fn range(&self, index: usize) -> Option<(usize, usize)> {
        match self {
            Self::Small(offsets) => {
                let start = usize::try_from(*offsets.get(index)?).ok()?;
                let end = usize::try_from(*offsets.get(index + 1)?).ok()?;
                Some((start, end))
            }
            Self::Large(offsets) => {
                let start = usize::try_from(*offsets.get(index)?).ok()?;
                let end = usize::try_from(*offsets.get(index + 1)?).ok()?;
                Some((start, end))
            }
        }
    }

    /// The same cut with one more row of `items` items on the end.
    fn appended(&self, items: usize) -> Self {
        match self {
            Self::Small(offsets) => {
                let mut held: Vec<i32> = offsets.iter().copied().collect();
                let last = held.last().copied().unwrap_or_default();
                held.push(last + i32::try_from(items).unwrap_or_default());
                Self::Small(OffsetBuffer::new(ScalarBuffer::from(held)))
            }
            Self::Large(offsets) => {
                let mut held: Vec<i64> = offsets.iter().copied().collect();
                let last = held.last().copied().unwrap_or_default();
                held.push(last + i64::try_from(items).unwrap_or_default());
                Self::Large(OffsetBuffer::new(ScalarBuffer::from(held)))
            }
        }
    }
}

/// One column of lists: the offsets that cut it, and the item column under
/// them.
///
/// The items are one column of the item field, so a caller reading every
/// item of every row reads one buffer rather than walking rows.
#[derive(Clone)]
pub struct ListSerie {
    field: Arc<Field>,
    offsets: ListOffsets,
    items: Box<Serie>,
    nulls: Option<NullBuffer>,
}

impl ListSerie {
    /// Pair a list field with its cut and the column it cuts.
    pub(crate) fn new(
        field: Arc<Field>,
        offsets: ListOffsets,
        items: Serie,
        nulls: Option<NullBuffer>,
    ) -> Self {
        Self {
            field,
            offsets,
            items: Box::new(items),
            nulls,
        }
    }

    /// Borrow the item column every row is cut out of.
    pub fn items(&self) -> &Serie {
        &self.items
    }

    /// Borrow the item column, to write into it.
    pub fn items_mut(&mut self) -> &mut Serie {
        &mut self.items
    }

    /// Return the item range row `index` occupies.
    pub fn range(&self, index: usize) -> Option<(usize, usize)> {
        self.offsets.range(index)
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }
}

impl SerieValue for ListSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.offsets.len()
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.len()
            || self
                .nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        if self.is_null(index) {
            return Ok(Scalar::Null);
        }
        let Some((start, end)) = self.offsets.range(index) else {
            return Ok(Scalar::Null);
        };
        let items = (start..end)
            .map(|item| self.items.scalar(item))
            .collect::<Result<Vec<Scalar>>>()?;
        Ok(Scalar::from_sequence(items))
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        let rows = self.len();
        super::require_row(self.field.name(), index, rows)?;
        let row = self.field.scalar(value)?;
        if row.is_null() {
            self.nulls = with_validity(self.nulls.as_ref(), rows, index, false);
            return Ok(());
        }
        let Some((start, end)) = self.offsets.range(index) else {
            return Ok(());
        };
        let items = row.as_sequence().unwrap_or_default().to_vec();
        if items.len() != end - start {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "a row of {} items does not fit the {} this cut names",
                    items.len(),
                    end - start
                ),
            });
        }
        for (offset, item) in (start..end).zip(items) {
            self.items.set(offset, item)?;
        }
        self.nulls = with_validity(self.nulls.as_ref(), rows, index, true);
        Ok(())
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let rows = self.len();
        let row = self.field.scalar(value)?;
        // An absent row is a cut of no items and a cleared validity bit; a
        // present empty row is the same cut, which is why the bitmap is the
        // only thing that tells the two apart.
        let present = !row.is_null();
        let items = row.as_sequence().unwrap_or_default().to_vec();
        let count = items.len();
        for item in items {
            self.items.push(item)?;
        }
        self.offsets = self.offsets.appended(count);
        self.nulls = pushed_validity(self.nulls.as_ref(), rows, present);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        let items = self.items.into_arrow_array();
        let Ok(item) = self.items.field().clone().into_arrow_field_ref() else {
            return new_empty_array(&arrow_schema::DataType::Null);
        };
        match &self.offsets {
            ListOffsets::Small(offsets) => Arc::new(ListArray::new(
                item,
                offsets.clone(),
                items,
                self.nulls.clone(),
            )),
            ListOffsets::Large(offsets) => Arc::new(LargeListArray::new(
                item,
                offsets.clone(),
                items,
                self.nulls.clone(),
            )),
        }
    }

    fn into_serie(self) -> Serie {
        Serie::List(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::List(column) => Some(column),
            _ => None,
        }
    }
}

impl fmt::Debug for ListSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "ListSerie", formatter)
    }
}

serie_leaf!(ListSerie);

/// One column of mappings: the offsets that cut it, and the entry column
/// under them.
///
/// Arrow lays a mapping out as a list of non-null key-value entry records,
/// so the entries are a [`StructSerie`] and reaching the keys or the values
/// is reaching one of its children.
#[derive(Clone)]
pub struct MapSerie {
    field: Arc<Field>,
    entries: Box<Serie>,
    offsets: OffsetBuffer<i32>,
    nulls: Option<NullBuffer>,
}

impl MapSerie {
    /// Pair a mapping field with its cut and the entries it cuts.
    pub(crate) fn new(
        field: Arc<Field>,
        entries: Serie,
        offsets: OffsetBuffer<i32>,
        nulls: Option<NullBuffer>,
    ) -> Self {
        Self {
            field,
            entries: Box::new(entries),
            offsets,
            nulls,
        }
    }

    /// Borrow the entry column every row is cut out of.
    pub fn entries(&self) -> &Serie {
        &self.entries
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Return the entry range row `index` occupies.
    pub fn range(&self, index: usize) -> Option<(usize, usize)> {
        let start = usize::try_from(*self.offsets.get(index)?).ok()?;
        let end = usize::try_from(*self.offsets.get(index + 1)?).ok()?;
        Some((start, end))
    }
}

impl SerieValue for MapSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.len()
            || self
                .nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        if self.is_null(index) {
            return Ok(Scalar::Null);
        }
        let Some((start, end)) = self.range(index) else {
            return Ok(Scalar::Null);
        };
        let mut entries = Vec::with_capacity(end - start);
        for entry in start..end {
            let pair = self.entries.scalar(entry)?;
            let cells = pair.as_sequence().unwrap_or_default();
            let [key, value] = cells else {
                return Err(crate::Error::InvalidRecord {
                    path: smol_str::SmolStr::new(self.field.name()),
                    reason: smol_str::SmolStr::new_static("a map entry holds a key and a value"),
                });
            };
            entries.push((key.clone(), value.clone()));
        }
        Scalar::from_mapping(entries)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        let rows = self.len();
        super::require_row(self.field.name(), index, rows)?;
        let row = self.field.scalar(value)?;
        if row.is_null() {
            self.nulls = with_validity(self.nulls.as_ref(), rows, index, false);
            return Ok(());
        }
        let Some((start, end)) = self.range(index) else {
            return Ok(());
        };
        let pairs = row.as_mapping().unwrap_or_default().to_vec();
        if pairs.len() != end - start {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "a row of {} entries does not fit the {} this cut names",
                    pairs.len(),
                    end - start
                ),
            });
        }
        for (offset, (key, held)) in (start..end).zip(pairs) {
            self.entries
                .set(offset, Scalar::from_sequence([key, held]))?;
        }
        self.nulls = with_validity(self.nulls.as_ref(), rows, index, true);
        Ok(())
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let rows = self.len();
        let row = self.field.scalar(value)?;
        let present = !row.is_null();
        let pairs = row.as_mapping().unwrap_or_default().to_vec();
        for (key, held) in &pairs {
            self.entries
                .push(Scalar::from_sequence([key.clone(), held.clone()]))?;
        }
        let mut held: Vec<i32> = self.offsets.iter().copied().collect();
        let last = held.last().copied().unwrap_or_default();
        held.push(last + i32::try_from(pairs.len()).unwrap_or_default());
        self.offsets = OffsetBuffer::new(ScalarBuffer::from(held));
        self.nulls = pushed_validity(self.nulls.as_ref(), rows, present);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        let entries = self.entries.into_arrow_array();
        let Some(records) = entries.as_any().downcast_ref::<StructArray>() else {
            return new_empty_array(&arrow_schema::DataType::Null);
        };
        let Ok(entry) = self.entries.field().clone().into_arrow_field_ref() else {
            return new_empty_array(&arrow_schema::DataType::Null);
        };
        MapArray::try_new(
            entry,
            self.offsets.clone(),
            records.clone(),
            self.nulls.clone(),
            false,
        )
        .map_or_else(
            |_| new_empty_array(&arrow_schema::DataType::Null),
            |array| Arc::new(array) as ArrayRef,
        )
    }

    fn into_serie(self) -> Serie {
        Serie::Map(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Map(column) => Some(column),
            _ => None,
        }
    }
}

impl fmt::Debug for MapSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "MapSerie", formatter)
    }
}

serie_leaf!(MapSerie);
