//! The column a self-describing value is stored in.
//!
//! A variant row is the Apache Parquet Variant pair - a metadata dictionary
//! and a value payload - so a variant column holds two byte runs per row and
//! lays out as Arrow's `Struct("metadata": Binary, "value": Binary)`, the
//! shape Parquet, Avro, Arrow and Iceberg all state for the type.
//!
//! The leaf holds Arrow's buffers - each run's offsets and payload, and the
//! column's validity - never an Arrow array, and beside them where they live
//! (`Backing`): the heap, or a spill file's read-only mapping. A run states
//! no validity of its own: the pair's two children are non-nullable, so
//! the column's bitmap is the one that says a row is absent.
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
//! and suffix where it does - a mapped run among them. An absent row
//! occupies one empty slot in each run, and the validity bitmap beside them
//! is what says it is not there.
//!
//! # Unsafe
//!
//! One use: `GenericByteArray::new_unchecked`, through the byte leaves'
//! `runs_unchecked`, wherever a run becomes a `BinaryArray` again - its
//! `array` and its splice. A run's parts are private and enter only by
//! taking apart a child of a pair array the landing door proved or
//! `array_of_rows` laid out, so the rebuild has nothing to check, and a
//! checked one would walk every offset.

#![allow(unsafe_code)]

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, StructArray};
use arrow_buffer::{ArrowNativeType, Buffer, NullBuffer, OffsetBuffer};
use arrow_schema::DataType as ArrowDataType;

use super::bytes::{require_run_fits, run_bytes, runs_unchecked, splice_run};
use super::{Serie, layout, require_range, require_row, require_window};
use crate::spill::Backing;
use crate::value::SerieValue;
use crate::{DataType, Field, Result, Scalar, Variant};

/// The invariant every nested column keeps: its children are aligned, so
/// the Arrow array assembles.
const ALIGNED: &str = "a variant column's two runs hold its rows: no public path misaligns them";

/// The invariant a canonical row carries into a write: it lays out as the
/// pair array, because the field's contract already encoded it and `check`
/// already measured it.
const LAID_OUT: &str = "a canonical variant row lays out as the pair array: the contract encoded it and `check` measured it";

/// One of the pair's two byte runs: row `i`'s bytes lie in `data` between
/// `offsets[i]` and `offsets[i + 1]`.
#[derive(Clone)]
struct ByteRun {
    offsets: OffsetBuffer<i32>,
    data: Buffer,
}

impl ByteRun {
    /// Take one child of a pair array apart; its own validity, which the
    /// column's masks, is not kept.
    fn of(run: BinaryArray) -> Self {
        let (offsets, data, _) = run.into_parts();
        Self { offsets, data }
    }

    /// The rows the offsets delimit.
    fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    /// Borrow the bytes of row `index`, which is in range, where they lie.
    fn get(&self, index: usize) -> &[u8] {
        &self.data[self.offsets[index].as_usize()..self.offsets[index + 1].as_usize()]
    }

    /// The Arrow array this run is, sharing its buffers: pointer bumps, no
    /// copy and no check.
    fn array(&self) -> BinaryArray {
        // SAFETY: the parts entered through `of` from a child of a pair array
        // the landing door proved or `array_of_rows` laid out, and were since
        // only sliced (`slice`) or replaced whole by a builder's (`splice`);
        // the fields are private, so no other path reaches them.
        unsafe { runs_unchecked(self.offsets.clone(), self.data.clone(), None) }
    }

    /// The run of rows `offset..offset + length`, sharing the payload.
    fn slice(&self, offset: usize, length: usize) -> Self {
        Self {
            offsets: self.offsets.slice(offset, length),
            data: self.data.clone(),
        }
    }

    /// Replace rows `range` by `replacement`'s runs, whose offsets total
    /// was checked.
    ///
    /// The parts are taken out first, so the run the splice takes apart is
    /// their one holder and Arrow hands them back as a builder where nothing
    /// else holds them.
    fn splice(&mut self, range: Range<usize>, replacement: &BinaryArray) {
        let offsets = std::mem::replace(&mut self.offsets, OffsetBuffer::new_empty());
        let data = std::mem::take(&mut self.data);
        // SAFETY: this run's own parts, moved out whole, under the argument
        // `array` makes for them.
        let taken = unsafe { runs_unchecked(offsets, data, None) };
        let (offsets, data, _) = splice_run(taken, range, replacement).into_parts();
        self.offsets = offsets;
        self.data = data;
    }
}

/// One column of self-describing values, each row one encoded pair.
#[derive(Clone)]
pub struct VariantSerie {
    field: Arc<Field>,
    metadata: ByteRun,
    value: ByteRun,
    nulls: Option<NullBuffer>,
    /// Where the buffers live: carried by a slice and a clone, the heap's
    /// again after a write.
    backing: Backing,
}

impl VariantSerie {
    /// Pair a variant field with the two runs that hold its rows, taking
    /// them apart.
    pub(crate) fn new(
        field: Arc<Field>,
        metadata: BinaryArray,
        value: BinaryArray,
        nulls: Option<NullBuffer>,
    ) -> Self {
        Self {
            field,
            metadata: ByteRun::of(metadata),
            value: ByteRun::of(value),
            nulls,
            backing: Backing::Heap,
        }
    }

    /// Borrow row `index`'s metadata dictionary where it lies: `None` when
    /// the row is absent or past the end.
    pub fn metadata(&self, index: usize) -> Option<&[u8]> {
        (!self.is_absent(index)).then(|| self.metadata.get(index))
    }

    /// Borrow row `index`'s value payload where it lies: `None` when the
    /// row is absent or past the end.
    pub fn value(&self, index: usize) -> Option<&[u8]> {
        (!self.is_absent(index)).then(|| self.value.get(index))
    }

    /// Return the Arrow array the metadata dictionaries are, sharing their
    /// buffers.
    pub fn metadata_array(&self) -> BinaryArray {
        self.metadata.array()
    }

    /// Return the Arrow array the value payloads are, sharing their
    /// buffers.
    pub fn value_array(&self) -> BinaryArray {
        self.value.array()
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub const fn nulls(&self) -> Option<&NullBuffer> {
        self.nulls.as_ref()
    }

    /// State where the buffers live: what a spill sets over the buffers it
    /// mapped.
    pub(crate) fn set_backing(&mut self, backing: Backing) {
        self.backing = backing;
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
        require_run_fits(
            self.field.name(),
            &self.metadata.offsets,
            range,
            metadata_bytes,
        )?;
        require_run_fits(self.field.name(), &self.value.offsets, range, value_bytes)
    }

    /// Write canonical `rows` over a checked `range`.
    ///
    /// The rows are laid out once at the crate's one scalar-array boundary,
    /// so a column built by pushes is buffer for buffer the column
    /// `from_scalars` builds from the same rows.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        let borrowed: Vec<&Scalar> = rows.iter().collect();
        let pair = crate::serie::value::array_of_rows(&self.field, &borrowed).expect(LAID_OUT);
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
            &self.metadata.offsets,
            &(len..len),
            run_bytes(&other.metadata.offsets),
        )
        .and_then(|()| {
            require_run_fits(
                name,
                &self.value.offsets,
                &(len..len),
                run_bytes(&other.value.offsets),
            )
        })
        .is_ok();
        if fits {
            let present: Vec<bool> = (0..other.metadata.len())
                .map(|row| !other.is_absent(row))
                .collect();
            self.write_runs(
                len..len,
                &other.metadata.array(),
                &other.value.array(),
                &present,
            );
        }
        fits
    }

    /// Replace rows `range` by the pairs `metadata` and `value` hold, of
    /// which `present` says which are there; what comes back is the heap's.
    fn write_runs(
        &mut self,
        range: Range<usize>,
        metadata: &BinaryArray,
        value: &BinaryArray,
        present: &[bool],
    ) {
        let len = self.metadata.len();
        self.metadata.splice(range.clone(), metadata);
        self.value.splice(range.clone(), value);
        self.nulls = layout::splice_nulls(self.nulls.take(), len, range, present);
        self.backing = Backing::Heap;
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
        Ok(Self {
            field: Arc::clone(&self.field),
            metadata: self.metadata.slice(offset, length),
            value: self.value.slice(offset, length),
            nulls: self.nulls.as_ref().map(|nulls| nulls.slice(offset, length)),
            backing: self.backing,
        })
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
            Arc::new(self.metadata.array()),
            Arc::new(self.value.array()),
        ];
        Arc::new(
            StructArray::try_new(crate::variant_fields(), children, self.nulls.clone())
                .expect(ALIGNED),
        )
    }

    fn memory_size(&self) -> usize {
        // The pair array is never boxed to count it: the validity beside the
        // two runs, each read as its own array of pointer bumps.
        self.nulls
            .as_ref()
            .map_or(0, |nulls| nulls.len().div_ceil(8))
            + crate::arrow::sliced_size(&self.metadata_array())
            + crate::arrow::sliced_size(&self.value_array())
    }

    fn resident_size(&self) -> usize {
        if self.backing.is_mapped() {
            0
        } else {
            self.memory_size()
        }
    }

    fn into_serie(self) -> Serie {
        super::Leaf::root(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        super::Leaf::narrow_laid(value)
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
    _budget: &mut crate::budget::MaterializationBudget,
    _resolved: Option<&super::arrow::Resolved>,
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
