//! Every column whose rows hold other rows.
//!
//! A nested layout in Arrow is its own buffers plus the columns under them,
//! so that is what these hold: [`StructSerie`] keeps one [`Serie`] per child
//! field, [`SequenceSerie`] keeps the offsets and the item column they cut, and
//! [`MappingSerie`] keeps the offsets and the entry column. Reaching a child is
//! therefore a borrow rather than a projection, and adding or dropping one
//! is one column rather than one row.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use arrow_array::OffsetSizeTrait;
use arrow_array::{
    Array, ArrayRef, LargeListArray, ListArray, MapArray, StructArray, new_empty_array,
};
use arrow_buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::Fields;

use super::Serie;
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar};

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
        let declared = child.require_field()?;
        let name = declared.name().to_owned();
        let mut field = self.field.as_ref().clone();
        field.set_field(name.as_str(), declared.clone())?;
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
                    // A child column fills its slot with the value its own
                    // field declares; a run declares none, so it takes the
                    // bare null a row without that cell holds.
                    let filler = match child.field() {
                        Some(field) => field.default_value()?,
                        None => Scalar::Null,
                    };
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
        let Some(columns) = self
            .children
            .iter()
            .map(Serie::into_arrow_array)
            .collect::<Option<Vec<ArrayRef>>>()
        else {
            return new_empty_array(&arrow_schema::DataType::Null);
        };
        if columns.is_empty() {
            return Arc::new(StructArray::new_empty_fields(self.rows, self.nulls.clone()));
        }
        StructArray::try_new(fields, columns, self.nulls.clone()).map_or_else(
            |_| new_empty_array(&arrow_schema::DataType::Null),
            |array| Arc::new(array) as ArrayRef,
        )
    }

    fn into_serie(self) -> Serie {
        Serie::Struct(Arc::new(self))
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Struct(column) => Some(column.as_ref()),
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

/// One column of sequences: the offsets that cut it, and the item serie
/// under them.
///
/// The items are one serie of the item field, so a caller reading every item
/// of every row reads one buffer rather than walking rows.
///
/// `O` is the offset width Arrow cut the rows at, so
/// [`SequenceSerie`] and [`LargeSequenceSerie`] are two names for one
/// implementation - the same way [`Utf8StringSerie`](crate::Utf8StringSerie)
/// and [`LargeUtf8StringSerie`](crate::LargeUtf8StringSerie) are. The width
/// is the type, so nothing branches on it per row.
#[derive(Clone)]
pub struct GenericSequenceSerie<O: OffsetSizeTrait, K: SequenceKind<O>> {
    field: Arc<Field>,
    offsets: OffsetBuffer<O>,
    items: Serie,
    nulls: Option<NullBuffer>,
    kind: PhantomData<K>,
}

/// A column of sequences, 32-bit offsets.
pub type SequenceSerie = GenericSequenceSerie<i32, Items>;

/// A column of sequences, 64-bit offsets.
pub type LargeSequenceSerie = GenericSequenceSerie<i64, Items>;

/// A column of mappings: the same cut, read as key-value entries.
///
/// Arrow has no large map, so there is no 64-bit twin - the pairing is
/// unrepresentable rather than deleted.
pub type MappingSerie = GenericSequenceSerie<i32, Entries>;

impl<O: OffsetSizeTrait, K: SequenceKind<O>> GenericSequenceSerie<O, K> {
    /// Pair a sequence field with its cut and the serie it cuts.
    pub(crate) fn new(
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
            kind: PhantomData,
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
        let start = (*self.offsets.get(index)?).as_usize();
        let end = (*self.offsets.get(index + 1)?).as_usize();
        Some((start, end))
    }

    /// Borrow the offsets buffer that cuts the items, without copying it.
    pub const fn offsets(&self) -> &OffsetBuffer<O> {
        &self.offsets
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// The same cut with one more row of `items` items on the end.
    fn appended(&self, items: usize) -> OffsetBuffer<O> {
        let mut held: Vec<O> = self.offsets.iter().copied().collect();
        let last = held.last().copied().unwrap_or_default();
        held.push(last + O::usize_as(items));
        OffsetBuffer::new(ScalarBuffer::from(held))
    }
}

impl<O: OffsetSizeTrait, K: SequenceKind<O>> SerieValue for GenericSequenceSerie<O, K> {
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
        index >= SerieValue::len(self)
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
        let items = (start..end)
            .map(|item| self.items.scalar(item))
            .collect::<Result<Vec<Scalar>>>()?;
        K::row(&self.field, items)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        let rows = SerieValue::len(self);
        super::require_row(self.field.name(), index, rows)?;
        let row = self.field.scalar(value)?;
        if row.is_null() {
            self.nulls = with_validity(self.nulls.as_ref(), rows, index, false);
            return Ok(());
        }
        let Some((start, end)) = self.range(index) else {
            return Ok(());
        };
        let items = K::items(&self.field, &row)?;
        if items.len() != end - start {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "a row of {} {} does not fit the {} this cut names",
                    items.len(),
                    K::ITEM,
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
        let rows = SerieValue::len(self);
        let row = self.field.scalar(value)?;
        // An absent row is a cut of no items and a cleared validity bit; a
        // present empty row is the same cut, which is why the bitmap is the
        // only thing that tells the two apart.
        // An absent row is a cut of no items and a cleared validity bit, so
        // there is nothing to read out of it.
        let present = !row.is_null();
        let items = if present {
            K::items(&self.field, &row)?
        } else {
            Vec::new()
        };
        let count = items.len();
        for item in items {
            self.items.push(item)?;
        }
        self.offsets = self.appended(count);
        self.nulls = pushed_validity(self.nulls.as_ref(), rows, present);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        // The items must be a column: Arrow names a list's item layout, and
        // a schema-free run names none.
        let (Some(items), Some(declared)) = (self.items.into_arrow_array(), self.items.field())
        else {
            return new_empty_array(&arrow_schema::DataType::Null);
        };
        let Ok(item) = declared.clone().into_arrow_field_ref() else {
            return new_empty_array(&arrow_schema::DataType::Null);
        };
        K::array(
            &self.field,
            item,
            self.offsets.clone(),
            items,
            self.nulls.clone(),
        )
        .unwrap_or_else(|_| new_empty_array(&arrow_schema::DataType::Null))
    }

    fn into_serie(self) -> Serie {
        K::into_serie(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        K::from_serie(value)
    }
}

/// What a run of cut rows *is*, where two shapes share one layout.
///
/// Arrow lays a mapping out as a list of non-null key-value entry records, so
/// a mapping column and a sequence column hold exactly the same four things:
/// a field, offsets, one serie of what the offsets cut, and a validity
/// bitmap. What differs is how a row reads, what Arrow type it lays out
/// under, and which leaf of the root it is - so that is what this marker
/// carries, and [`GenericSequenceSerie`] is one implementation over all of
/// it.
///
/// It carries more than [`ByteKind`](crate::ByteKind) does, and the
/// difference is worth naming rather than glossing. A byte marker decides
/// only which leaf its layout widens to; the meaning of a row comes from the
/// field, through the crate's one schema-directed decode. This marker
/// decides the meaning too, because the field cannot supply it here: a
/// mapping's rows are stored as records of a key and a value, and
/// [`Field::scalar`] on a mapping field takes a mapping rather than a
/// sequence of those records, so the pairing has to happen somewhere the
/// field is not. That is the one place this leaf departs from "the leaf
/// names the layout, the field names the meaning".
///
/// The bound is on the pair, so a shape that Arrow has no type for is
/// unrepresentable rather than dead: Arrow has no large map, and there is no
/// `SequenceKind<i64>` for [`Entries`].
pub trait SequenceKind<O: OffsetSizeTrait>: Copy + Send + Sync + 'static {
    /// Lay these buffers out as the Arrow array this shape names.
    ///
    /// `field` is the column's own, because a mapping reads `keys_sorted`
    /// off it - Arrow carries that in the datatype, so a column that lost it
    /// would not read back.
    fn array(
        field: &Field,
        item: arrow_schema::FieldRef,
        offsets: OffsetBuffer<O>,
        items: ArrayRef,
        nulls: Option<NullBuffer>,
    ) -> Result<ArrayRef>;

    /// Read one row's cut of items as the value this shape is.
    ///
    /// `field` is the column's own, so a refusal names it rather than the
    /// root.
    fn row(field: &Field, items: Vec<Scalar>) -> Result<Scalar>;

    /// The items one row contributes, in the order they are stored.
    ///
    /// The row is read for what it *means*, never borrowed for what it
    /// happens to store: a sequence row may itself be a column, which lends
    /// no slice, and borrowing one would silently contribute nothing.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming `field` where the row is not the shape this
    /// marker reads, and the row's own where a column it holds cannot be
    /// read.
    fn items(field: &Field, row: &Scalar) -> Result<Vec<Scalar>>;

    /// What this shape calls a row, for the refusals that name one.
    const ITEM: &'static str;

    /// What this shape is called, for the debug rendering that names it.
    const NAME: &'static str;

    /// Widen a column of this shape to the serie root.
    fn into_serie(column: GenericSequenceSerie<O, Self>) -> Serie;

    /// Narrow a serie root to a column of this shape.
    fn from_serie(value: &Serie) -> Option<&GenericSequenceSerie<O, Self>>;
}

/// The marker for rows that are a sequence of items.
#[derive(Clone, Copy, Debug)]
pub struct Items;

/// The marker for rows that are a mapping of key-value entries.
#[derive(Clone, Copy, Debug)]
pub struct Entries;

impl SequenceKind<i32> for Items {
    const ITEM: &'static str = "items";
    const NAME: &'static str = "SequenceSerie";

    fn array(
        _field: &Field,
        item: arrow_schema::FieldRef,
        offsets: OffsetBuffer<i32>,
        items: ArrayRef,
        nulls: Option<NullBuffer>,
    ) -> Result<ArrayRef> {
        Ok(Arc::new(ListArray::try_new(item, offsets, items, nulls)?))
    }

    fn row(_field: &Field, items: Vec<Scalar>) -> Result<Scalar> {
        Ok(Scalar::from_sequence(items))
    }

    fn items(field: &Field, row: &Scalar) -> Result<Vec<Scalar>> {
        let Some(rows) = row.sequence_rows() else {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(field.name()),
                reason: smol_str::format_smolstr!(
                    "expected a sequence of items, got {}",
                    row.kind()
                ),
            });
        };
        Ok(rows?.into_owned())
    }

    fn into_serie(column: GenericSequenceSerie<i32, Self>) -> Serie {
        Serie::Sequence(Arc::new(column))
    }

    fn from_serie(value: &Serie) -> Option<&GenericSequenceSerie<i32, Self>> {
        match value {
            Serie::Sequence(column) => Some(column.as_ref()),
            _ => None,
        }
    }
}

impl SequenceKind<i64> for Items {
    const ITEM: &'static str = "items";
    const NAME: &'static str = "LargeSequenceSerie";

    fn array(
        _field: &Field,
        item: arrow_schema::FieldRef,
        offsets: OffsetBuffer<i64>,
        items: ArrayRef,
        nulls: Option<NullBuffer>,
    ) -> Result<ArrayRef> {
        Ok(Arc::new(LargeListArray::try_new(
            item, offsets, items, nulls,
        )?))
    }

    fn row(_field: &Field, items: Vec<Scalar>) -> Result<Scalar> {
        Ok(Scalar::from_sequence(items))
    }

    fn items(field: &Field, row: &Scalar) -> Result<Vec<Scalar>> {
        let Some(rows) = row.sequence_rows() else {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(field.name()),
                reason: smol_str::format_smolstr!(
                    "expected a sequence of items, got {}",
                    row.kind()
                ),
            });
        };
        Ok(rows?.into_owned())
    }

    fn into_serie(column: GenericSequenceSerie<i64, Self>) -> Serie {
        Serie::LargeSequence(Arc::new(column))
    }

    fn from_serie(value: &Serie) -> Option<&GenericSequenceSerie<i64, Self>> {
        match value {
            Serie::LargeSequence(column) => Some(column.as_ref()),
            _ => None,
        }
    }
}

impl SequenceKind<i32> for Entries {
    const ITEM: &'static str = "entries";
    const NAME: &'static str = "MappingSerie";

    fn array(
        field: &Field,
        item: arrow_schema::FieldRef,
        offsets: OffsetBuffer<i32>,
        items: ArrayRef,
        nulls: Option<NullBuffer>,
    ) -> Result<ArrayRef> {
        let Some(records) = items.as_any().downcast_ref::<StructArray>() else {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(field.name()),
                reason: smol_str::SmolStr::new_static(
                    "a mapping's entries are a record of a key and a value",
                ),
            });
        };
        // Arrow carries `keys_sorted` in the datatype, and this crate carries
        // it as the leaf a mapping datatype is. Reading it off the field is
        // what makes a sorted map read back as one.
        let sorted = match field.dtype() {
            DataType::Mapping(mapping) => mapping.keys_sorted(),
            _ => false,
        };
        Ok(Arc::new(MapArray::try_new(
            item,
            offsets,
            records.clone(),
            nulls,
            sorted,
        )?))
    }

    fn row(field: &Field, items: Vec<Scalar>) -> Result<Scalar> {
        let mut entries = Vec::with_capacity(items.len());
        for pair in items {
            let cells = pair.as_sequence().unwrap_or_default();
            let [key, value] = cells else {
                return Err(crate::Error::InvalidRecord {
                    path: smol_str::SmolStr::new(field.name()),
                    reason: smol_str::SmolStr::new_static(
                        "a mapping entry holds a key and a value",
                    ),
                });
            };
            entries.push((key.clone(), value.clone()));
        }
        Scalar::from_mapping(entries)
    }

    fn items(field: &Field, row: &Scalar) -> Result<Vec<Scalar>> {
        let Some(pairs) = row.as_mapping() else {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(field.name()),
                reason: smol_str::format_smolstr!(
                    "expected a mapping of entries, got {}",
                    row.kind()
                ),
            });
        };
        Ok(pairs
            .iter()
            .map(|(key, value)| Scalar::from_sequence([key.clone(), value.clone()]))
            .collect())
    }

    fn into_serie(column: GenericSequenceSerie<i32, Self>) -> Serie {
        Serie::Mapping(Arc::new(column))
    }

    fn from_serie(value: &Serie) -> Option<&GenericSequenceSerie<i32, Self>> {
        match value {
            Serie::Mapping(column) => Some(column.as_ref()),
            _ => None,
        }
    }
}

impl<O: OffsetSizeTrait, K: SequenceKind<O>> fmt::Debug for GenericSequenceSerie<O, K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, K::NAME, formatter)
    }
}

serie_leaf!(
    GenericSequenceSerie,
    O: arrow_array::OffsetSizeTrait,
    K: SequenceKind<O>
);
