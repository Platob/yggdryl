//! The column a record field is stored in: one child column per child
//! field, beside the record's own validity.
//!
//! The children are columns, not projections, so [`StructSerie::child`] is
//! a borrow and [`StructSerie::without_child`] keeps every other child's
//! buffers exactly as they were. Every child holds exactly `len` rows, and
//! no public path hands a child out mutably, so the alignment cannot break
//! and assembling the Arrow array cannot fail. A null record row clears the
//! record's validity bit and gives every child one placeholder slot - the
//! crate's one answer to what occupies a slot a parent null hides - so a
//! column built by pushes is buffer-for-buffer the column `from_scalars`
//! builds from the same rows.
//!
//! A write descends: `check` asks every child what it would refuse for the
//! cells it will receive, and only then `write` hands each child its cells,
//! so a refusal three levels down leaves every buffer at every level as it
//! was.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, StructArray};
use arrow_buffer::NullBuffer;
use arrow_schema::DataType as ArrowDataType;

use super::{Serie, layout, require_range, require_row, require_window};
use crate::expression::FieldSegment;
use crate::value::SerieValue;
use crate::{DataType, Field, FieldPath, Result, Scalar, StructType};

/// The invariant every nested column keeps: its children are aligned, so
/// the Arrow array assembles.
const ALIGNED: &str = "a record column's children hold its rows: no public path misaligns them";

/// The invariant a write carries in from `check`: it ran on these rows
/// under these fields, so a placeholder it built builds again.
const CHECKED: &str = "check ran on these rows: a placeholder it built, write builds again";

/// One column of records: the field that types them, and one column per
/// child field.
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

    /// Borrow one child column by exact name.
    pub fn child(&self, name: &str) -> Option<&Serie> {
        self.children.get(self.field.index_of(name)?)
    }

    /// Borrow child `index`.
    pub fn child_at(&self, index: usize) -> Option<&Serie> {
        self.children.get(index)
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Replace the child of `child`'s name, or add it, extending the field.
    ///
    /// Every other child's buffers are shared, not rebuilt, and no row is
    /// read. A name reaches a child exactly as [`Field::index_of`] resolves
    /// it, so a child named `a.b` is one child and never two levels.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when `child` is a run, or does not
    /// hold exactly as many rows as this column does.
    pub fn set_child(&mut self, child: Serie) -> Result<()> {
        let declared = child.require_field()?.clone();
        if child.len() != self.rows {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "a child of {} rows does not fit the {} rows {} holds",
                    child.len(),
                    self.rows,
                    self.field.name()
                ),
            });
        }
        let mut field = self.field.as_ref().clone();
        let position = match self.field.index_of(declared.name()) {
            Some(index) => {
                field.set_field(index, declared)?;
                index
            }
            None => {
                let mut fields = self.field.fields().to_vec();
                fields.push(declared);
                field.set_dtype(DataType::from(StructType::from_fields(fields)?))?;
                self.children.len()
            }
        };
        self.field = Arc::new(field);
        if position < self.children.len() {
            self.children[position] = child;
        } else {
            self.children.push(child);
        }
        Ok(())
    }

    /// Write one cell of row `index`, `path` deep, in place.
    ///
    /// Every level is row-aligned, so `index` is the same row at every level;
    /// the leaf's field proves `value` once and its column writes one slot.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when `index` is past the end, when
    /// a segment is not a record child's name or a level on the way is not a
    /// record column, when row `index` of any record on the way is absent,
    /// or when `value` is not one the leaf's field accepts.
    pub fn set_cell(&mut self, path: &FieldPath, index: usize, value: Scalar) -> Result<()> {
        self.set_cell_by_segments(path.segments(), index, value)
    }

    /// The one walk a cell write takes, one record level per named segment.
    fn set_cell_by_segments(
        &mut self,
        segments: &[FieldSegment],
        index: usize,
        value: Scalar,
    ) -> Result<()> {
        require_row(self.field.name(), index, self.rows)?;
        if self
            .nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(index))
        {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "row {index} of {} is absent; set the whole row",
                    self.field.name()
                ),
            });
        }
        let refusal = |reason: smol_str::SmolStr| crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new(self.field.name()),
            reason,
        };
        let Some((segment, rest)) = segments.split_first() else {
            return Err(refusal(smol_str::format_smolstr!(
                "a cell of {} is a child's name, got an empty path",
                self.field.name()
            )));
        };
        let FieldSegment::Field(name) = segment else {
            return Err(refusal(smol_str::format_smolstr!(
                "a cell of {} is a child's name, got {segment}",
                self.field.name()
            )));
        };
        let Some(position) = self.field.index_of(name) else {
            return Err(refusal(smol_str::format_smolstr!(
                "{} has no child named {name:?}",
                self.field.name()
            )));
        };
        let child = &mut self.children[position];
        if rest.is_empty() {
            return child.set(index, value);
        }
        match child {
            Serie::Struct(held) => Arc::make_mut(held).set_cell_by_segments(rest, index, value),
            other => Err(refusal(smol_str::format_smolstr!(
                "{name:?} of {} is not a record column, so it holds no cell {}",
                self.field.name(),
                other.field().map_or("", Field::name)
            ))),
        }
    }

    /// Return this column with the child of `name` dropped, sharing every
    /// other child.
    ///
    /// # Errors
    ///
    /// Returns an error naming the column when no child is named `name`.
    pub fn without_child(&self, name: &str) -> Result<Self> {
        let Some(index) = self.field.index_of(name) else {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "{} has no child named {name:?}",
                    self.field.name()
                ),
            });
        };
        let mut field = self.field.as_ref().clone();
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

    /// The cells child `position` receives for canonical `rows`: the row's
    /// own cell, or the child's placeholder under an absent record.
    ///
    /// The placeholder is built once, and only when a row is absent; a
    /// canonical record row is a run of exactly one cell per child.
    fn cells_for(&self, position: usize, rows: &[Scalar]) -> crate::arrow::Result<Vec<Scalar>> {
        let placeholder = rows
            .iter()
            .any(Scalar::is_null)
            .then(|| {
                crate::arrow::value::physical_placeholder_for_field(&self.field.fields()[position])
            })
            .transpose()?
            .unwrap_or(Scalar::Null);
        Ok(rows
            .iter()
            .map(|row| {
                row.as_sequence()
                    .map_or_else(|| placeholder.clone(), |cells| cells[position].clone())
            })
            .collect())
    }

    /// Refuse what a write could not do: whatever each child refuses for
    /// the cells it will receive.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        for (position, child) in self.children.iter().enumerate() {
            let cells = self.cells_for(position, rows)?;
            child.check(range, &cells)?;
        }
        Ok(())
    }

    /// Write canonical `rows` over a checked `range`: each child one column
    /// of cells, the validity spliced, the count moved.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let present: Vec<bool> = rows.iter().map(|row| !row.is_null()).collect();
        for position in 0..self.children.len() {
            let cells = self.cells_for(position, &rows).expect(CHECKED);
            self.children[position].write(range.clone(), cells);
        }
        self.nulls = layout::splice_nulls(self.nulls.take(), self.rows, range.clone(), &present);
        self.rows = self.rows - range.len() + rows.len();
    }

    /// Append `other` child by child, whose field agrees with this one's,
    /// answering whether every child appended; when one cannot, none does.
    ///
    /// A child answers `false` only for a total its layout cannot reach -
    /// an offsets cut past its offset type - and leaves itself as it was,
    /// so the children before it are cut back to the rows they held and the
    /// record stays aligned; the root then reads the rows and the splice
    /// refuses by name.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let prior = self.rows;
        let mut done = 0;
        for (mine, theirs) in self.children.iter_mut().zip(&other.children) {
            if !mine.append(theirs) {
                break;
            }
            done += 1;
        }
        if done < self.children.len() {
            for child in &mut self.children[..done] {
                child.write(prior..prior + other.rows, Vec::new());
            }
            return false;
        }
        let present: Vec<bool> = (0..other.rows)
            .map(|row| other.nulls.as_ref().is_none_or(|nulls| nulls.is_valid(row)))
            .collect();
        self.nulls = layout::splice_nulls(self.nulls.take(), prior, prior..prior, &present);
        self.rows += other.rows;
        true
    }
}

impl SerieValue for StructSerie {
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
        if self.is_null(index)? {
            return Ok(Scalar::Null);
        }
        Scalar::try_sequence(self.children.len(), |position| {
            self.children[position].scalar(index)
        })
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.rows)?;
        let children = self
            .children
            .iter()
            .map(|child| child.slice(offset, length))
            .collect::<Result<Vec<Serie>>>()?;
        Ok(Self::new(
            Arc::clone(&self.field),
            children,
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
        if self.children.is_empty() {
            return Arc::new(StructArray::new_empty_fields(self.rows, self.nulls.clone()));
        }
        let ArrowDataType::Struct(fields) = self
            .field
            .as_arrow_field_ref()
            .expect(ALIGNED)
            .data_type()
            .clone()
        else {
            unreachable!("a record field projects to a struct")
        };
        let columns = self
            .children
            .iter()
            .map(|child| child.into_arrow_array().expect(ALIGNED))
            .collect();
        Arc::new(StructArray::try_new(fields, columns, self.nulls.clone()).expect(ALIGNED))
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
        super::debug_column(self, "StructSerie", formatter)
    }
}

serie_leaf!(StructSerie);

/// Build the column `field` types out of a struct array, or answer `None`
/// for a layout that is not one.
///
/// Each child takes the door with its own field and the record's validity
/// as its parent, so a required child is judged only where the record is
/// present.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proven: bool,
) -> crate::arrow::Result<Option<Serie>> {
    let _ = parent;
    if !matches!(array.data_type(), ArrowDataType::Struct(_)) {
        return Ok(None);
    }
    let records = super::arrow::held::<StructArray>(&array)?;
    let children = field
        .fields()
        .iter()
        .zip(records.columns())
        .map(|(child, column)| {
            super::arrow::column_of(
                Arc::new(child.clone()),
                Arc::clone(column),
                records.nulls(),
                proven,
            )
        })
        .collect::<crate::arrow::Result<Vec<Serie>>>()?;
    Ok(Some(
        StructSerie::new(field, children, records.nulls().cloned(), records.len()).into_serie(),
    ))
}
