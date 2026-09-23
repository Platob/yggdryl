//! The column a self-describing value is stored in.
//!
//! A variant row is the Apache Parquet Variant pair - a metadata dictionary
//! and a value payload - so a variant column holds two byte runs per row and
//! lays out as Arrow's `Struct("metadata": Binary, "value": Binary)`, the
//! shape Parquet, Avro, Arrow and Iceberg all state for the type.
//!
//! The pair is lent where it lies: [`VariantSerie::metadata`] and
//! [`VariantSerie::value`] borrow one row's two runs without reading either,
//! and [`crate::SerieValue::scalar`] answers `Scalar::Variant` - the pair,
//! still undecoded - so a caller forwarding a row moves bytes and a caller
//! reading one calls [`crate::Variant::scalar`] itself. The pair is
//! validated by `Variant::new`, so `push(Scalar::Variant(..))` is the writer
//! and there is no typed one.
//!
//! A write lays its rows out once as the pair array and splices each run
//! the way a byte column splices its one: into the builder Arrow hands back
//! where nothing else holds the run, rebuilt once from prefix, replacement
//! and suffix where it does. An absent row occupies one empty slot in each
//! run, and the validity bitmap beside them is what says it is not there.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, StructArray};
use arrow_buffer::NullBuffer;
use arrow_schema::DataType as ArrowDataType;

use super::bytes::{require_run_fits, run_bytes, splice_run};
use super::{Serie, layout, require_range, require_row, require_window};
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar, Variant};

/// The invariant every nested column keeps: its children are aligned, so
/// the Arrow array assembles.
const ALIGNED: &str = "a variant column's two runs hold its rows: no public path misaligns them";

/// The invariant a canonical row carries into a write: it lays out as the
/// pair array, because the field's contract already encoded it and `check`
/// already measured it.
const LAID_OUT: &str = "a canonical variant row lays out as the pair array: the contract encoded it and `check` measured it";

/// One column of self-describing values, each row one encoded pair.
#[derive(Clone)]
pub struct VariantSerie {
    field: Arc<Field>,
    metadata: BinaryArray,
    value: BinaryArray,
    nulls: Option<NullBuffer>,
}

impl VariantSerie {
    /// Pair a variant field with the two runs that hold its rows.
    pub(crate) const fn new(
        field: Arc<Field>,
        metadata: BinaryArray,
        value: BinaryArray,
        nulls: Option<NullBuffer>,
    ) -> Self {
        Self {
            field,
            metadata,
            value,
            nulls,
        }
    }

    /// Borrow row `index`'s metadata dictionary where it lies: `None` when
    /// the row is absent or past the end.
    pub fn metadata(&self, index: usize) -> Option<&[u8]> {
        (!self.is_absent(index)).then(|| self.metadata.value(index))
    }

    /// Borrow row `index`'s value payload where it lies: `None` when the
    /// row is absent or past the end.
    pub fn value(&self, index: usize) -> Option<&[u8]> {
        (!self.is_absent(index)).then(|| self.value.value(index))
    }

    /// Borrow the Arrow array the metadata dictionaries are.
    pub const fn metadata_array(&self) -> &BinaryArray {
        &self.metadata
    }

    /// Borrow the Arrow array the value payloads are.
    pub const fn value_array(&self) -> &BinaryArray {
        &self.value
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// Whether row `index` is past the end or absent.
    fn is_absent(&self, index: usize) -> bool {
        index >= self.metadata.len()
            || self
                .nulls
                .as_ref()
                .is_some_and(|nulls| nulls.is_null(index))
    }

    /// Refuse what a write could not do: an offsets total past `i32` in
    /// either run.
    ///
    /// A canonical row is the pair or a bare null, so the rows are measured
    /// off the pairs they hold and nothing is laid out here.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let (mut metadata_bytes, mut value_bytes) = (0_usize, 0_usize);
        for row in rows {
            if let Scalar::Variant(held) = row {
                metadata_bytes = metadata_bytes.saturating_add(held.metadata().len());
                value_bytes = value_bytes.saturating_add(held.value().len());
            }
        }
        require_run_fits(self.field.name(), &self.metadata, range, metadata_bytes)?;
        require_run_fits(self.field.name(), &self.value, range, value_bytes)
    }

    /// Write canonical `rows` over a checked `range`.
    ///
    /// The rows are laid out once at the crate's one scalar-array boundary,
    /// so a column built by pushes is buffer for buffer the column
    /// `from_scalars` builds from the same rows.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let borrowed: Vec<&Scalar> = rows.iter().collect();
        let pair = crate::arrow::value::array_from_values(&self.field, &borrowed).expect(LAID_OUT);
        let (metadata, value, nulls) = runs_of(pair.as_ref()).expect(LAID_OUT);
        let present: Vec<bool> = (0..pair.len())
            .map(|row| nulls.as_ref().is_none_or(|nulls| nulls.is_valid(row)))
            .collect();
        self.write_runs(range, metadata, value, &present);
    }

    /// Append `other`'s runs, whose field agrees with this one's, answering
    /// whether both runs' offsets reach the total; `false` leaves this
    /// column as it was, and the root then reads the rows and refuses by
    /// name.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        let len = self.metadata.len();
        let name = self.field.name();
        let fits = require_run_fits(
            name,
            &self.metadata,
            &(len..len),
            run_bytes(&other.metadata),
        )
        .and_then(|()| require_run_fits(name, &self.value, &(len..len), run_bytes(&other.value)))
        .is_ok();
        if fits {
            let present: Vec<bool> = (0..other.metadata.len())
                .map(|row| !other.is_absent(row))
                .collect();
            self.write_runs(len..len, &other.metadata, &other.value, &present);
        }
        fits
    }

    /// Replace rows `range` by the pairs `metadata` and `value` hold, of
    /// which `present` says which are there.
    fn write_runs(
        &mut self,
        range: Range<usize>,
        metadata: &BinaryArray,
        value: &BinaryArray,
        present: &[bool],
    ) {
        let len = self.metadata.len();
        let taken = std::mem::replace(&mut self.metadata, BinaryArray::new_null(0));
        self.metadata = splice_run(taken, range.clone(), metadata);
        let taken = std::mem::replace(&mut self.value, BinaryArray::new_null(0));
        self.value = splice_run(taken, range.clone(), value);
        self.nulls = layout::splice_nulls(self.nulls.take(), len, range, present);
    }
}

/// The two runs and the validity a pair array holds, or `None` for an array
/// that is not the Parquet Variant pair.
fn runs_of(array: &dyn Array) -> Option<(&BinaryArray, &BinaryArray, Option<&NullBuffer>)> {
    let pair = array.as_any().downcast_ref::<StructArray>()?;
    let metadata = pair.column(0).as_any().downcast_ref::<BinaryArray>()?;
    let value = pair.column(1).as_any().downcast_ref::<BinaryArray>()?;
    Some((metadata, value, pair.nulls()))
}

impl SerieValue for VariantSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    fn len(&self) -> usize {
        self.metadata.len()
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.metadata.len())?;
        Ok(self.is_absent(index))
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.metadata.len())?;
        let (Some(metadata), Some(value)) = (self.metadata(index), self.value(index)) else {
            return Ok(Scalar::Null);
        };
        // The pair is answered undecoded: reading it is the caller's ask, and
        // `Variant::scalar` is the one door that reads it.
        Ok(Scalar::Variant(Variant::new(metadata, value)?))
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.metadata.len())?;
        Ok(Self::new(
            Arc::clone(&self.field),
            self.metadata.slice(offset, length),
            self.value.slice(offset, length),
            self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
        ))
    }

    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.metadata.len())?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        self.check(&range, &canonical)?;
        self.write(range, canonical);
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        let children: Vec<ArrayRef> = vec![
            Arc::new(self.metadata.clone()),
            Arc::new(self.value.clone()),
        ];
        Arc::new(
            StructArray::try_new(crate::variant_fields(), children, self.nulls.clone())
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

impl fmt::Debug for VariantSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, "VariantSerie", formatter)
    }
}

serie_leaf!(VariantSerie);

/// Build the column a variant field types out of its pair array, or answer
/// `None` for a field that is not a variant.
///
/// `DataType::Variant` projects to the Parquet Variant pair and nothing
/// else, so this is tried before the record module and takes the one
/// storage the door has already proven.
pub(crate) fn column_of(
    field: Arc<Field>,
    array: ArrayRef,
    parent: Option<&NullBuffer>,
    proof: &super::arrow::Proof,
) -> crate::arrow::Result<Option<Serie>> {
    let _ = (parent, proof);
    if !matches!(field.dtype(), DataType::Variant)
        || !matches!(array.data_type(), ArrowDataType::Struct(_))
    {
        return Ok(None);
    }
    let Some((metadata, value, nulls)) = runs_of(array.as_ref()) else {
        return Err(crate::arrow::Error::Internal {
            site: "serie::variant::pair",
        });
    };
    Ok(Some(
        VariantSerie::new(field, metadata.clone(), value.clone(), nulls.cloned()).into_serie(),
    ))
}
