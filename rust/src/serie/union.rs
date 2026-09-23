//! The column a union field is stored in: the type ids, the offsets of a
//! dense layout, and one child column per member.
//!
//! A row reads through its type id into the member's column - at the
//! row's own index for a sparse union, at its offset for a dense one - and
//! is the `[type id, payload]` pair the field's contract spells; a union
//! spells absence inside the member, so a null row is a pair whose payload
//! is absent, and the column's null count is the members' where they are
//! reached.
//!
//! A sparse union's members are row-aligned with it, so every write is in
//! place: the type ids spliced, and each member spliced over the same rows
//! with its payload where the row is its own and its placeholder elsewhere.
//! A dense union appends in place - each payload pushed onto its member and
//! the offset it lands at recorded - while any other write rebuilds: the
//! replacement laid out at the crate's one scalar-array boundary, joined to
//! the rows around it with Arrow's concatenation, which copies only the
//! member slots the rows still reach, and re-imported through the door with
//! its rows already proven.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, UnionArray, new_empty_array};
use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder, NullBuffer, ScalarBuffer};
use arrow_schema::{ArrowError, DataType as ArrowDataType};

use super::layout::splice_scalars;
use super::{Serie, require_range, require_row, require_window};
use crate::budget::MaterializationBudget;
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar, UnionFields, UnionMode};

/// The invariant every union column keeps: its children hold the rows its
/// type ids and offsets reach, so the Arrow array assembles.
const ALIGNED: &str =
    "a union column's children hold the rows its type ids reach: no public path misaligns them";

/// The invariant a write carries in from `check`: the rows fit the layout
/// and every child takes its cells, so the rebuild cannot refuse them.
const CHECKED: &str =
    "check ran on these rows: the layout holds them and every child takes its cells";

/// The invariant a canonical row carries into a write: it is the
/// `[type id, payload]` pair the field's contract rewrote it into, and it
/// lays out as the field's own array.
const CANONICAL: &str = "a canonical union row is [type id, payload]: the contract rewrote it";

/// The invariant the door keeps: a union field declares its members.
const MEMBERS: &str = "a union field declares its members: the datatype validated them";

/// One column of union rows: the type ids, the dense offsets, and one child
/// column per member in the field's declared order.
#[derive(Clone)]
pub struct UnionSerie {
    field: Arc<Field>,
    type_ids: ScalarBuffer<i8>,
    offsets: Option<ScalarBuffer<i32>>,
    children: Vec<Serie>,
}

/// Read a canonical union row as the member it names and its payload.
fn branch(row: &Scalar) -> (i8, &Scalar) {
    let [type_id, payload] = row.as_sequence().expect(CANONICAL) else {
        unreachable!("{CANONICAL}")
    };
    let type_id = type_id
        .as_i128()
        .and_then(|type_id| i8::try_from(type_id).ok())
        .expect(CANONICAL);
    (type_id, payload)
}

impl UnionSerie {
    /// Pair a union field with its type ids, its dense offsets and one
    /// column per member.
    pub(crate) const fn new(
        field: Arc<Field>,
        type_ids: ScalarBuffer<i8>,
        offsets: Option<ScalarBuffer<i32>>,
        children: Vec<Serie>,
    ) -> Self {
        Self {
            field,
            type_ids,
            offsets,
            children,
        }
    }

    /// Borrow the type id of every row, without copying them.
    pub fn type_ids(&self) -> &[i8] {
        &self.type_ids
    }

    /// Borrow the offsets of a dense union, or `None` for a sparse one.
    pub fn offsets(&self) -> Option<&[i32]> {
        self.offsets.as_deref()
    }

    /// Borrow every member's column, in the field's declared order.
    pub fn children(&self) -> &[Serie] {
        &self.children
    }

    /// Borrow the column of the member `type_id` names.
    pub fn child_of(&self, type_id: i8) -> Option<&Serie> {
        let position = self.members().iter().position(|(id, _)| id == type_id)?;
        self.children.get(position)
    }

    /// Return whether the union is sparse or dense, as the field declares.
    pub fn mode(&self) -> UnionMode {
        match self.field.dtype() {
            DataType::Union(_, mode) => *mode,
            _ => UnionMode::Sparse,
        }
    }

    /// The members the field declares, in the order the children hold.
    fn members(&self) -> &UnionFields {
        match self.field.dtype() {
            DataType::Union(members, _) => members,
            _ => unreachable!("{MEMBERS}"),
        }
    }

    /// The child row `index` reads: the member's position and the row in
    /// its column; the caller keeps `index` below the length.
    fn slot(&self, index: usize) -> (usize, usize) {
        let type_id = self.type_ids[index];
        let position = self
            .members()
            .iter()
            .position(|(id, _)| id == type_id)
            .expect(ALIGNED);
        let row = match &self.offsets {
            Some(offsets) => usize::try_from(offsets[index]).expect(ALIGNED),
            None => index,
        };
        (position, row)
    }

    /// The cells member `position` receives for canonical `rows`: its own
    /// payloads and, in a sparse union, its placeholder for every row of
    /// another member - the crate's one answer to what occupies a slot an
    /// inactive type id hides.
    fn cells_for(&self, position: usize, rows: &[Scalar]) -> crate::arrow::Result<Vec<Scalar>> {
        let (type_id, member) = self.members().get(position).expect(ALIGNED);
        let sparse = matches!(self.mode(), UnionMode::Sparse);
        let placeholder = (sparse && rows.iter().any(|row| branch(row).0 != type_id))
            .then(|| crate::arrow::value::physical_placeholder_for_field(member))
            .transpose()?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                let (id, payload) = branch(row);
                if id == type_id {
                    Some(payload.clone())
                } else {
                    placeholder.clone()
                }
            })
            .collect())
    }

    /// Whether `total` rows fit the layout: a dense union's offsets are
    /// `i32`; a sparse one has no bound of its own.
    fn fits(&self, total: usize) -> bool {
        matches!(self.mode(), UnionMode::Sparse) || i32::try_from(total).is_ok()
    }

    /// Whether a write over `range` appends to a dense union, which is the
    /// one dense write made in place.
    fn appends_dense(&self, range: &Range<usize>) -> bool {
        self.offsets.is_some() && range.is_empty() && range.start == self.type_ids.len()
    }

    /// The position in the declared members of the member `type_id` names.
    fn position_of(&self, type_id: i8) -> usize {
        self.members()
            .iter()
            .position(|(id, _)| id == type_id)
            .expect(CANONICAL)
    }

    /// Refuse a length the layout cannot reach, naming the column.
    fn require_fit(&self, total: usize) -> Result<()> {
        if self.fits(total) {
            return Ok(());
        }
        Err(crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new(self.field.name()),
            reason: smol_str::format_smolstr!(
                "{total} rows are past the {} a dense union's offsets reach in {}",
                i32::MAX,
                self.field.name()
            ),
        })
    }

    /// Lay canonical `rows` out as this column's array, once, at the
    /// crate's one scalar-array boundary.
    fn laid_out(&self, rows: &[Scalar]) -> ArrayRef {
        let borrowed: Vec<&Scalar> = rows.iter().collect();
        crate::arrow::value::array_from_values(&self.field, &borrowed).expect(CHECKED)
    }

    /// Take `joined` - this column's storage - as this column, through the
    /// door with its rows already proven.
    fn rebuilt(&self, joined: ArrayRef) -> Self {
        let serie =
            super::arrow::column_of(Arc::clone(&self.field), joined, None, true).expect(CHECKED);
        let Serie::Union(held) = serie else {
            unreachable!("{CHECKED}")
        };
        Arc::unwrap_or_clone(held)
    }

    /// Refuse what a write could not do: a length past a dense union's
    /// offsets, a sparse replacement past the crate's materialization
    /// budget - every member at the replacement's length - and, recursively,
    /// whatever each child refuses for the cells it will receive.
    ///
    /// A sparse member is checked over the rows it is written over; a dense
    /// member receives its payloads at its end, and in a dense append the
    /// last offset it lands at must still be an `i32`.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        self.require_fit(self.type_ids.len() - range.len() + rows.len())?;
        let sparse = matches!(self.mode(), UnionMode::Sparse);
        if sparse {
            let mut budget = MaterializationBudget::default();
            for (_, member) in self.members() {
                budget.add_array(member.dtype(), rows.len())?;
            }
        }
        for (position, child) in self.children.iter().enumerate() {
            let cells = self.cells_for(position, rows)?;
            let held = child.len();
            if self.appends_dense(range) {
                self.require_fit(held + cells.len())?;
            }
            let over = if sparse { range.clone() } else { held..held };
            child.check(&over, &cells)?;
        }
        Ok(())
    }

    /// Write canonical `rows` over a checked `range`: in place for a sparse
    /// union and a dense append, a rebuild for any other dense write.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let ids: Vec<i8> = rows.iter().map(|row| branch(row).0).collect();
        if self.offsets.is_none() {
            for position in 0..self.children.len() {
                let cells = self.cells_for(position, &rows).expect(CHECKED);
                self.children[position].write(range.clone(), cells);
            }
            self.type_ids = splice_scalars(std::mem::take(&mut self.type_ids), range, &ids);
            return;
        }
        if self.appends_dense(&range) {
            self.append_dense(&rows, &ids);
            return;
        }
        self.rebuild(range, &rows);
    }

    /// Append canonical `rows` of type ids `ids` to a dense union: each
    /// payload pushed onto its member, and the offset it lands at recorded.
    fn append_dense(&mut self, rows: &[Scalar], ids: &[i8]) {
        let mut payloads: Vec<Vec<Scalar>> = vec![Vec::new(); self.children.len()];
        let mut next: Vec<usize> = self.children.iter().map(Serie::len).collect();
        let offsets: Vec<i32> = rows
            .iter()
            .zip(ids)
            .map(|(row, type_id)| {
                let position = self.position_of(*type_id);
                payloads[position].push(branch(row).1.clone());
                let at = next[position];
                next[position] += 1;
                i32::try_from(at).expect(CHECKED)
            })
            .collect();
        for (child, cells) in self.children.iter_mut().zip(payloads) {
            if !cells.is_empty() {
                let held = child.len();
                child.write(held..held, cells);
            }
        }
        let len = self.type_ids.len();
        self.type_ids = splice_scalars(std::mem::take(&mut self.type_ids), len..len, ids);
        let held = self.offsets.take().expect(CHECKED);
        self.offsets = Some(splice_scalars(held, len..len, &offsets));
    }

    /// Replace rows `range` of a dense union by canonical `rows`, rebuilt.
    fn rebuild(&mut self, range: Range<usize>, rows: &[Scalar]) {
        let array = self.into_arrow_array();
        let len = self.type_ids.len();
        let joined = joined(
            &self.field,
            vec![
                array.slice(0, range.start),
                self.laid_out(rows),
                array.slice(range.end, len - range.end),
            ],
        )
        .expect(CHECKED);
        *self = self.rebuilt(joined);
    }

    /// Append `other`'s rows, whose field agrees with this one's, answering
    /// whether the layout reaches the total; when it does not, nothing
    /// moved.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        if !self.fits(self.type_ids.len() + other.type_ids.len()) {
            return false;
        }
        let Ok(joined) = joined(
            &self.field,
            vec![self.into_arrow_array(), other.into_arrow_array()],
        ) else {
            return false;
        };
        *self = self.rebuilt(joined);
        true
    }
}

/// Join `pieces` into one array of `field`'s storage, skipping the empty
/// ones: the kernel refuses an empty input.
fn joined(field: &Field, pieces: Vec<ArrayRef>) -> std::result::Result<ArrayRef, ArrowError> {
    let pieces: Vec<&dyn Array> = pieces
        .iter()
        .filter(|piece| !piece.is_empty())
        .map(|piece| piece.as_ref())
        .collect();
    if pieces.is_empty() {
        let storage = field.as_arrow_field_ref().expect(ALIGNED).data_type();
        return Ok(new_empty_array(storage));
    }
    arrow_select::concat::concat(&pieces)
}

impl SerieValue for UnionSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.type_ids.len()
    }

    fn null_count(&self) -> usize {
        (0..self.type_ids.len())
            .filter(|index| {
                let (position, row) = self.slot(*index);
                self.children[position].is_null(row).expect(ALIGNED)
            })
            .count()
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.type_ids.len())?;
        let (position, row) = self.slot(index);
        self.children[position].is_null(row)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.type_ids.len())?;
        let (position, row) = self.slot(index);
        let payload = self.children[position].scalar(row)?;
        Ok(Scalar::from_sequence([
            Scalar::from(i64::from(self.type_ids[index])),
            payload,
        ]))
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.type_ids.len())?;
        let (offsets, children) = match &self.offsets {
            Some(offsets) => (Some(offsets.slice(offset, length)), self.children.clone()),
            None => (
                None,
                self.children
                    .iter()
                    .map(|child| child.slice(offset, length))
                    .collect::<Result<Vec<Serie>>>()?,
            ),
        };
        Ok(Self::new(
            Arc::clone(&self.field),
            self.type_ids.slice(offset, length),
            offsets,
            children,
        ))
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.type_ids.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        let ArrowDataType::Union(fields, _) = self
            .field
            .as_arrow_field_ref()
            .expect(ALIGNED)
            .data_type()
            .clone()
        else {
            unreachable!("a union field projects to a union")
        };
        let children = self
            .children
            .iter()
            .map(|child| child.into_arrow_array().expect(ALIGNED))
            .collect();
        Arc::new(
            UnionArray::try_new(
                fields,
                self.type_ids.clone(),
                self.offsets.clone(),
                children,
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

impl fmt::Debug for UnionSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, "UnionSerie", formatter)
    }
}

serie_leaf!(UnionSerie);

/// The rows of a child that member `type_id`'s type ids reach, as the
/// validity a parent lends: a sparse child's slot is reached where the row
/// is the member's, a dense child's where an offset points at it.
///
/// What an inactive slot holds is a placeholder Arrow leaves unspecified,
/// so a required member is judged, and a narrow one read, only where it is
/// reached.
fn reached(
    type_id: i8,
    type_ids: &ScalarBuffer<i8>,
    offsets: Option<&ScalarBuffer<i32>>,
    len: usize,
) -> NullBuffer {
    let Some(offsets) = offsets else {
        return NullBuffer::new(BooleanBuffer::from_iter(
            type_ids.iter().map(|id| *id == type_id),
        ));
    };
    let mut bits = BooleanBufferBuilder::new(len);
    bits.append_n(len, false);
    for (id, offset) in type_ids.iter().zip(offsets.iter()) {
        if *id != type_id {
            continue;
        }
        if let Some(at) = usize::try_from(*offset).ok().filter(|at| *at < len) {
            bits.set_bit(at, true);
        }
    }
    NullBuffer::new(bits.finish())
}

/// Build the column `field` types out of a union array, or answer `None`
/// for a layout that is not one.
///
/// Each member takes the door with its own field and the rows the type ids
/// reach as its parent, so a required member is judged only where a row is
/// its, and an inactive slot's placeholder is never read.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proven: bool,
) -> crate::arrow::Result<Option<Serie>> {
    let _ = parent;
    if !matches!(array.data_type(), ArrowDataType::Union(..)) {
        return Ok(None);
    }
    let DataType::Union(members, _) = field.dtype() else {
        return Err(crate::arrow::Error::Internal {
            site: "serie::union::column_of",
        });
    };
    let (_, type_ids, offsets, arrays) = super::arrow::held::<UnionArray>(&array)?.into_parts();
    let children = members
        .iter()
        .zip(arrays)
        .map(|((type_id, member), child)| {
            let reached = reached(type_id, &type_ids, offsets.as_ref(), child.len());
            super::arrow::column_of(Arc::new(member.clone()), child, Some(&reached), proven)
        })
        .collect::<crate::arrow::Result<Vec<Serie>>>()?;
    Ok(Some(
        UnionSerie::new(field, type_ids, offsets, children).into_serie(),
    ))
}
