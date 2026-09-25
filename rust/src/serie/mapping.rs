//! The column a mapping field is stored in: the offsets cut, and the entries
//! record column under it.
//!
//! Arrow lays a mapping out as a list of non-null key-value entry records,
//! so the entries are a [`StructSerie`](crate::StructSerie) of the entries
//! field and [`MapSerie::keys`] and [`MapSerie::values`] are its two
//! children. A row reads as a [`Scalar::Map`] or a
//! [`Scalar::SortedMap`], as the field declares, pairing the two back;
//! a write turns each pair into a two-cell run for the entries record's own
//! write, which is no second proof: [`Field::scalar`] on a mapping field
//! already canonicalized every key and value under the entries record's two
//! fields. Whether the keys are sorted is read off the field.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, MapArray, StructArray};
use arrow_buffer::{ArrowNativeType, NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::DataType as ArrowDataType;

use super::{Serie, layout, require_range, require_row, require_window};
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar};

/// The invariant every nested column keeps: its children are aligned, so
/// the Arrow array assembles.
const ALIGNED: &str = "a mapping column's cut ends at its entries: no public path misaligns them";

/// The entries canonical `rows` hold, each pair a two-cell run for the
/// entries record column, in row order, and how many each row holds: an
/// absent row holds none.
fn entries_of(rows: &[Scalar]) -> (Vec<usize>, Vec<Scalar>) {
    let mut lengths = Vec::with_capacity(rows.len());
    let mut entries = Vec::new();
    for row in rows {
        match row.as_mapping() {
            Some(pairs) => {
                lengths.push(pairs.len());
                entries.extend(
                    pairs
                        .iter()
                        .map(|(key, value)| Scalar::from_sequence([key.clone(), value.clone()])),
                );
            }
            None => lengths.push(0),
        }
    }
    (lengths, entries)
}

/// One column of mappings: the offsets that cut it, and the entries record
/// column under them.
///
/// The cut is rebased: its first offset is 0 and its last is the entry
/// count, so a column that grows knows where its entries end.
#[derive(Clone)]
pub struct MapSerie {
    field: Arc<Field>,
    offsets: OffsetBuffer<i32>,
    entries: Serie,
    nulls: Option<NullBuffer>,
}

impl MapSerie {
    /// Pair a mapping field with its cut and the entries it cuts.
    pub(crate) const fn new(
        field: Arc<Field>,
        offsets: OffsetBuffer<i32>,
        entries: Serie,
        nulls: Option<NullBuffer>,
    ) -> Self {
        Self {
            field,
            offsets,
            entries,
            nulls,
        }
    }

    /// Borrow the entries column: a record column of the entries field.
    pub const fn entries(&self) -> &Serie {
        &self.entries
    }

    /// Borrow the keys column, the entries' first child.
    pub fn keys(&self) -> &Serie {
        self.entries.child_at(0).expect(ALIGNED)
    }

    /// Borrow the values column, the entries' second child.
    pub fn values(&self) -> &Serie {
        self.entries.child_at(1).expect(ALIGNED)
    }

    /// Borrow the offsets buffer that cuts the entries, without copying it.
    pub const fn offsets(&self) -> &OffsetBuffer<i32> {
        &self.offsets
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Whether every row's keys are sorted, as the field declares.
    pub fn keys_sorted(&self) -> bool {
        matches!(self.field.dtype(), DataType::SortedMap(_))
    }

    /// Return the entry range row `index` occupies: `None` when the row is
    /// absent or past the end.
    pub fn range(&self, index: usize) -> Option<Range<usize>> {
        let start = usize::try_from(*self.offsets.get(index)?).ok()?;
        let end = usize::try_from(*self.offsets.get(index + 1)?).ok()?;
        if self
            .nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index))
        {
            return None;
        }
        Some(start..end)
    }

    /// Return row `index` as the entries column sliced to it, zero copy.
    pub fn row(&self, index: usize) -> Option<Serie> {
        let range = self.range(index)?;
        self.entries.slice(range.start, range.len()).ok()
    }

    /// The entry range rows `range` occupy, an absent row occupying none;
    /// `range` is one `require_range` admitted.
    fn cut(&self, range: &Range<usize>) -> Range<usize> {
        self.offsets[range.start].as_usize()..self.offsets[range.end].as_usize()
    }

    /// Refuse what a write could not do: an entry total past `i32`, and
    /// whatever the entries record refuses over the cut it will receive.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let (_, entries) = entries_of(rows);
        let replaced = self.cut(range);
        layout::require_offset::<i32>(
            self.field.name(),
            self.entries.len() - replaced.len() + entries.len(),
        )?;
        self.entries.check(&replaced, &entries)
    }

    /// Write canonical `rows` over a checked `range`: the cut re-cut from
    /// `range.start` on, the entries spliced over the entry range the
    /// replaced rows occupied, the validity spliced.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let len = self.offsets.len() - 1;
        let present: Vec<bool> = rows.iter().map(|row| !row.is_null()).collect();
        let (lengths, entries) = entries_of(&rows);
        let offsets = std::mem::replace(&mut self.offsets, OffsetBuffer::new_empty());
        let (offsets, replaced) = layout::splice_offsets(offsets, range.clone(), &lengths);
        self.offsets = offsets;
        self.entries.write(replaced, entries);
        self.nulls = layout::splice_nulls(self.nulls.take(), len, range, &present);
    }

    /// Append `other`'s cut and entries, whose field agrees with this one's,
    /// answering whether `i32` reaches the total and the entries appended;
    /// `false` leaves this column as it was, and the root then reads the
    /// rows so the splice refuses by name.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let len = self.offsets.len() - 1;
        let total = self.entries.len() + other.entries.len();
        if layout::require_offset::<i32>(self.field.name(), total).is_err()
            || !self.entries.append(&other.entries)
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
        let present: Vec<bool> = (0..other.offsets.len() - 1)
            .map(|row| other.nulls.as_ref().is_none_or(|nulls| nulls.is_valid(row)))
            .collect();
        self.nulls = layout::splice_nulls(self.nulls.take(), len, len..len, &present);
        true
    }
}

impl SerieValue for MapSerie {
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
        let (keys, values) = (self.keys(), self.values());
        // The entries are written straight into the mapping's own storage,
        // the span's length being what the cell holds.
        let mapping = Scalar::try_mapping(range.len(), |offset| {
            let entry = range.start + offset;
            Ok((keys.scalar(entry)?, values.scalar(entry)?))
        })?;
        Ok(self.field.dtype().declared_layout(mapping))
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.len())?;
        let first = self.offsets[offset];
        let last = self.offsets[offset + length];
        let rebased: Vec<i32> = self.offsets[offset..=offset + length]
            .iter()
            .map(|held| *held - first)
            .collect();
        Ok(Self::new(
            Arc::clone(&self.field),
            OffsetBuffer::new(ScalarBuffer::from(rebased)),
            self.entries
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
        let ArrowDataType::Map(entries, _) = self
            .field
            .as_arrow_field_ref()
            .expect(ALIGNED)
            .data_type()
            .clone()
        else {
            unreachable!("a mapping field projects to a map")
        };
        let records = self.entries.into_arrow_array().expect(ALIGNED);
        let records = records
            .as_any()
            .downcast_ref::<StructArray>()
            .expect(ALIGNED)
            .clone();
        Arc::new(
            MapArray::try_new(
                entries,
                self.offsets.clone(),
                records,
                self.nulls.clone(),
                self.keys_sorted(),
            )
            .expect(ALIGNED),
        )
    }

    fn into_serie(self) -> Serie {
        super::Leaf::root(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        super::Leaf::narrow(value)
    }
}

impl fmt::Debug for MapSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, "MapSerie", formatter)
    }
}

serie_leaf!(MapSerie);

/// Build the column `field` types out of a map array, or answer `None` for
/// a layout that is not one.
///
/// The cut is rebased onto exactly the entries it reaches; the entries take
/// the door under the entries reached by present rows. A null map or ancestor
/// masks its physical entries. Narrow entry subtrees compact hidden spans;
/// layout-contract entries retain their shared physical buffers.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proof: &super::arrow::Proof,
    budget: &mut crate::budget::MaterializationBudget,
    resolved: Option<&super::arrow::Resolved>,
) -> crate::arrow::Result<Option<Serie>> {
    use super::arrow::{held, rebased};

    let Some(map) = field.dtype().as_mapping() else {
        return Ok(None);
    };
    let entries_field = map.entries();
    if !matches!(array.data_type(), ArrowDataType::Map(..)) {
        return Ok(None);
    }
    let maps = held::<MapArray>(&array)?;
    if matches!(proof, super::arrow::Proof::Unproven) {
        crate::cast::columns::validate_map_invariants(
            &map,
            &maps,
            parent.map(NullBuffer::inner),
            budget,
        )
        .map_err(|refusal| {
            crate::arrow::Error::IncompatibleSchema(format!("column {:?}: {refusal}", field.name()))
        })?;
    }
    let entries: ArrayRef = Arc::new(maps.entries().clone());
    let (offsets, values) = rebased(maps.offsets(), &entries);
    let hidden = super::arrow::parent_nulls(parent, maps.nulls(), budget)?;
    let (offsets, values) = super::arrow::compact_offsets(
        offsets,
        values,
        entries_field.dtype(),
        hidden.as_ref(),
        budget,
    )?;
    let hidden = super::arrow::offset_parent(&offsets, values.len(), hidden.as_ref(), budget)?;
    let (entries_field, below) = super::arrow::resolved_child(resolved, 0, entries_field);
    let entries = super::arrow::child_of(
        entries_field,
        values,
        hidden.as_ref(),
        proof.child(0),
        budget,
        below,
    )?;
    Ok(Some(
        MapSerie::new(field, offsets, entries, maps.nulls().cloned()).into_serie(),
    ))
}
