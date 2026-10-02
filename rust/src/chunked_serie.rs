//! ChunkedSerie: many columns under one field, held apart.
//!
//! A [`Serie`] column is one contiguous set of Arrow buffers, and a
//! [`SerieReader`] is a stream of record columns read once. Between the two
//! sits what a runtime holds when it holds a table: several columns under
//! one field, in order, each its own buffers - what `pyarrow` calls a
//! `ChunkedArray`, and, when the field is a non-null record, a `Table` of
//! one batch per chunk. A [`ChunkedSerie`] is that. Its chunks are [`Serie`]
//! columns of exactly one [`Field`], each proven at its own door, and the
//! collection reads across them as one column would: a row is found by its
//! chunk, the identity is the rows, and a cast is one plan applied to every
//! chunk.
//!
//! It holds and copies nothing a chunk does not. A clone is a pointer bump
//! per chunk, a slice keeps the chunks it reaches, a child of a record is
//! the child of every chunk, and Arrow crosses in and out one chunk at a
//! time - [`ChunkedSerie::from_arrow_arrays`] for the arrays of a chunked
//! array, [`ChunkedSerie::from_arrow_reader`] for the batches of a table -
//! kept apart rather than joined. [`ChunkedSerie::into_serie`] is the one
//! join, and it is spelled as one.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{ArrayRef, Int64Array};
//! use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Field::new("price", DataType::Int64, false);
//! let first: ArrayRef = Arc::new(Int64Array::from(vec![125, 126]));
//! let second: ArrayRef = Arc::new(Int64Array::from(vec![127]));
//!
//! // Two arrays cross as two chunks, their buffers shared, under one plan.
//! let prices = ChunkedSerie::from_arrow_arrays(
//!     Some(&field),
//!     [first, second],
//!     ArrowCastOptions::new(),
//! )?;
//! assert_eq!((prices.len(), prices.num_chunks()), (3, 2));
//!
//! // A row is found by its chunk, and a window keeps the chunks it reaches.
//! assert_eq!(prices.scalar(2)?, Scalar::from(127_i64));
//! assert_eq!(prices.slice(1, 2)?.num_chunks(), 2);
//!
//! // Joining is one verb, and the rows are the identity either way.
//! let joined = prices.into_serie()?;
//! assert_eq!(joined.len(), 3);
//! assert!(prices == joined);
//! # Ok(())
//! # }
//! ```

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, ArrayRef};
use arrow_data::ArrayData;
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

use crate::arrow::{BatchReader, batch_reader};
use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred};
use crate::diff::one_datatype;
use crate::expression::IntoSelector;
use crate::serie::arrow::{
    batch_schema, batch_under, item_field, land_planned, lands_exactly, storage_holds,
};
use crate::serie::{
    Proof, Resolved, Rows, compare_rows, compare_values, hash_rows, land, proven_row,
    require_window,
};
use crate::value::Children;
use crate::window_serie::window_end;
use crate::{DataType, Field, FieldPath, Scalar, Serie, SerieReader, SortOptions};

/// The invariant every chunk carries: it is a column, and its field is the
/// collection's, so its buffers and its field are always there to lend.
const CHUNK: &str = "a chunk is a column under the chunked serie's field";

/// Many columns under one field, held apart: a chunked array, or a table.
///
/// Every chunk is a [`Serie`] column whose field is exactly this one, so a
/// row is read through the chunk that holds it and never proven again. The
/// chunk ends are kept beside the chunks, so finding a row's chunk is a
/// binary search and the length is a read.
///
/// A clone is a pointer bump per chunk. Equality, order and hash are the
/// rows alone, as they are for a [`Serie`], so a chunked serie is one value
/// with the column of its rows however the rows are cut.
#[derive(Clone)]
pub struct ChunkedSerie {
    field: Arc<Field>,
    chunks: Vec<Serie>,
    /// The row each chunk ends before: `ends[i]` is the first row of chunk
    /// `i + 1`, and the last is the length.
    ends: Vec<usize>,
}

impl ChunkedSerie {
    /// Pair `field` with chunks the caller already landed under it: the
    /// doors here, and a plan whose target it is.
    pub(crate) fn from_landed(field: Arc<Field>, chunks: Vec<Serie>) -> Self {
        let mut ends = Vec::with_capacity(chunks.len());
        let mut end = 0;
        for chunk in &chunks {
            end += chunk.len();
            ends.push(end);
        }
        Self {
            field,
            chunks,
            ends,
        }
    }

    /// The field the first chunk carries, shared, or `fallback` when there
    /// is none.
    fn field_of(chunks: &[Serie], fallback: impl FnOnce() -> Arc<Field>) -> Arc<Field> {
        chunks
            .first()
            .and_then(Serie::field_ref)
            .map_or_else(fallback, Arc::clone)
    }

    /// The chunked serie of no chunks under `field`.
    ///
    /// # Errors
    ///
    /// [`Serie::empty`] carries the rule: a field with no Arrow projection,
    /// or a layout this crate keeps no column for, is refused.
    pub fn empty(field: impl Into<Arc<Field>>) -> crate::Result<Self> {
        let field = field.into();
        Serie::empty(Arc::clone(&field))?;
        Ok(Self::from_landed(field, Vec::new()))
    }

    /// One held column as the one chunk it is, sharing its buffers.
    ///
    /// # Errors
    ///
    /// Returns an error for a run, which names no field.
    pub fn from_serie(serie: Serie) -> crate::Result<Self> {
        serie.require_field()?;
        let field = Arc::clone(serie.field_ref().expect(CHUNK));
        Ok(Self::from_landed(field, vec![serie]))
    }

    /// Hold `chunks` under one field: the first chunk's own, or `field`.
    ///
    /// With no field, the chunks are one datatype in pieces, as a chunked
    /// array is: the field is the first chunk's, nullable where any chunk's
    /// is; a chunk of that datatype under another name or nullability, or
    /// naming its serie items or mapping entries otherwise, is the same
    /// buffers relabelled; and a chunk of another datatype is refused naming
    /// it, because nothing picks which of two datatypes the rows are. With a
    /// field, every chunk already under it is held as it stands and any
    /// other is cast into it under `options`. One plan is compiled per run
    /// of chunks under one source field, so a thousand chunks of one layout
    /// compile once. No chunk under a field is [`Self::empty`].
    ///
    /// # Errors
    ///
    /// Returns an error for a run, which names no layout, for no chunk and
    /// no field, for a chunk of another datatype than the first where no
    /// field was given, [`Self::empty`]'s refusal, and the plan's refusals
    /// for a chunk the field cannot hold.
    pub fn from_series(
        field: Option<&Field>,
        chunks: impl IntoIterator<Item = Serie>,
        options: ArrowCastOptions,
    ) -> crate::arrow::Result<Self> {
        let chunks: Vec<Serie> = chunks.into_iter().collect();
        for chunk in &chunks {
            chunk.require_field()?;
        }
        let declared = field.is_some();
        let field = match field {
            Some(field) if chunks.is_empty() => return Ok(Self::empty(field.clone())?),
            Some(field) => field.clone(),
            None => {
                let Some(first) = chunks.first() else {
                    return Err(no_field("no chunk"));
                };
                let nullable = chunks
                    .iter()
                    .any(|chunk| chunk.field().is_some_and(Field::is_nullable));
                first.require_field()?.clone().with_nullable(nullable)
            }
        };
        let field = Arc::new(field);
        let mut plan: Option<(Arc<Field>, ArrowCastPlan)> = None;
        let mut landed = Vec::with_capacity(chunks.len());
        for (index, chunk) in chunks.into_iter().enumerate() {
            let own = Arc::clone(chunk.field_ref().expect(CHUNK));
            if own == field {
                landed.push(chunk);
                continue;
            }
            if !declared && !one_datatype(own.dtype(), field.dtype()) {
                return Err(disagreeing(index, &field, own.dtype()));
            }
            let current = match plan.take() {
                Some((source, current)) if source == own => (source, current),
                _ => {
                    let compiled = ArrowCastPlan::compile(&own, &field, options)?;
                    (own, compiled)
                }
            };
            landed.push(current.1.apply(&chunk)?);
            plan = Some(current);
        }
        Ok(Self::from_landed(field, landed))
    }

    /// Hold Arrow arrays as chunks: of their own field, or cast into
    /// `field`.
    ///
    /// This is the door a `pyarrow.ChunkedArray` takes, one array per chunk.
    /// With no field the arrays are one datatype in pieces, read as
    /// [`Self::from_series`] reads chunks with no field: the column of the
    /// first array's layout, named `item` and nullable where any array holds
    /// an absent row - read logically, so a null behind a dictionary key, a
    /// run or a union member is one - and an array of another datatype is
    /// refused naming the chunk. With a field, every array is cast into it.
    /// One [`ArrowCastPlan`] is compiled per run of arrays of one layout -
    /// the identity where the layout is the field's, sharing the buffers.
    /// No array under a field is [`Self::empty`].
    ///
    /// # Errors
    ///
    /// Returns an error for no array and no field, for an array of another
    /// datatype than the first where no field was given, naming the chunk,
    /// [`Self::empty`]'s refusal, and [`Serie::from_arrow_array`]'s refusals
    /// for a value the field cannot hold.
    pub fn from_arrow_arrays(
        field: Option<&Field>,
        arrays: impl IntoIterator<Item = ArrayRef>,
        options: ArrowCastOptions,
    ) -> crate::arrow::Result<Self> {
        let arrays: Vec<ArrayRef> = arrays.into_iter().collect();
        let declared = field.is_some();
        let field = match field {
            Some(field) if arrays.is_empty() => return Ok(Self::empty(field.clone())?),
            Some(field) => field.clone(),
            None => {
                let Some(first) = arrays.first() else {
                    return Err(no_field("no array"));
                };
                let nullable = arrays.iter().any(|array| array.logical_null_count() != 0);
                item_field(first.data_type(), nullable)?
            }
        };
        let field = Arc::new(field);
        let mut plan: Option<(Arc<ArrowField>, ArrowCastPlan)> = None;
        // An exact chunk lands as it stands under boxes resolved once for
        // every chunk, and no plan compiles for it; one the landing refuses
        // takes the plan, which repairs or refuses it under `options`.
        let mut resolved: Option<Resolved> = None;
        let mut chunks = Vec::with_capacity(arrays.len());
        for (index, array) in arrays.into_iter().enumerate() {
            if lands_exactly(&field, array.data_type())? {
                let resolved = match &resolved {
                    Some(resolved) => resolved,
                    None => {
                        field.validate_bounded()?;
                        resolved.insert(Resolved::of(Arc::clone(&field)))
                    }
                };
                if let Ok(chunk) = land_planned(resolved, Arc::clone(&array), &Proof::Unproven) {
                    chunks.push(chunk);
                    continue;
                }
            }
            let current = match plan.take() {
                Some((source, current)) if source.data_type() == array.data_type() => {
                    (source, current)
                }
                _ => {
                    if !declared && index > 0 {
                        let own = DataType::from_arrow_datatype(array.data_type())?;
                        if !one_datatype(&own, field.dtype()) {
                            return Err(disagreeing(index, &field, &own));
                        }
                    }
                    let source = Arc::new(ArrowField::new(
                        field.name(),
                        array.data_type().clone(),
                        true,
                    ));
                    let compiled = ArrowCastPlan::compile_arrow(
                        &source,
                        &field,
                        options,
                        Deferred::default(),
                    )?;
                    (source, compiled)
                }
            };
            chunks.push(current.1.cast_array(array)?);
            plan = Some(current);
        }
        let field = Self::field_of(&chunks, || field);
        Ok(Self::from_landed(field, chunks))
    }

    /// Drain a [`SerieReader`] into its chunks: one record column per batch,
    /// under the reader's root, none joined.
    ///
    /// # Errors
    ///
    /// Returns the first failure a batch raises, after which the reader is
    /// fused.
    pub fn from_serie_reader(reader: SerieReader) -> crate::arrow::Result<Self> {
        let root = reader.field().clone();
        let chunks = reader.collect::<crate::arrow::Result<Vec<Serie>>>()?;
        let field = Self::field_of(&chunks, || Arc::new(root));
        Ok(Self::from_landed(field, chunks))
    }

    /// Drain an Arrow batch stream into its chunks: of its own schema, or
    /// cast into `root` by one plan.
    ///
    /// This is the door a `pyarrow.Table` takes: [`SerieReader::from_arrow_reader`]
    /// collected, one chunk per batch and none joined, where
    /// [`Serie::from_arrow_reader`] joins them into one column.
    ///
    /// # Errors
    ///
    /// [`SerieReader::from_arrow_reader`] carries the rule, and the reader
    /// carries its own.
    pub fn from_arrow_reader(
        root: Option<&Field>,
        reader: BatchReader,
        options: ArrowCastOptions,
    ) -> crate::arrow::Result<Self> {
        Self::from_serie_reader(SerieReader::from_arrow_reader(root, reader, options)?)
    }

    /// The field every chunk is typed by.
    pub fn field(&self) -> &Field {
        &self.field
    }

    /// The shared field.
    pub fn field_ref(&self) -> &Arc<Field> {
        &self.field
    }

    /// `serie(<the field named item>)`: what [`Serie::dtype`] answers for a
    /// column of this field.
    pub fn dtype(&self) -> DataType {
        DataType::serie(self.field.as_ref().clone().with_name("item"))
    }

    /// Every chunk, in order.
    pub fn chunks(&self) -> &[Serie] {
        &self.chunks
    }

    /// Chunk `index`, or `None` past the last.
    pub fn chunk(&self, index: usize) -> Option<&Serie> {
        self.chunks.get(index)
    }

    /// How many chunks the rows are cut into.
    pub fn num_chunks(&self) -> usize {
        self.chunks.len()
    }

    /// The number of rows across every chunk: a read, kept beside the
    /// chunks.
    pub fn len(&self) -> usize {
        self.ends.last().copied().unwrap_or(0)
    }

    /// Whether no chunk holds a row.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many rows hold no value: one read per chunk.
    pub fn null_count(&self) -> usize {
        self.chunks.iter().map(Serie::null_count).sum()
    }

    /// The chunk row `index` is in and its offset there, or `None` past the
    /// end: a binary search over the chunk ends.
    fn locate(&self, index: usize) -> Option<(usize, usize)> {
        if index >= self.len() {
            return None;
        }
        let chunk = self.ends.partition_point(|&end| end <= index);
        let start = if chunk == 0 { 0 } else { self.ends[chunk - 1] };
        Some((chunk, index - start))
    }

    /// Refuse a row past the end, naming the serie and both counts.
    fn located(&self, index: usize) -> crate::Result<(usize, usize)> {
        self.locate(index)
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "row {index} is past the end of {} rows",
                    self.len()
                ),
            })
    }

    /// Whether row `index` holds no value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when `index` is
    /// past the end.
    pub fn is_null(&self, index: usize) -> crate::Result<bool> {
        let (chunk, offset) = self.located(index)?;
        self.chunks[chunk].is_null(offset)
    }

    /// Row `index`, built as one value out of the chunk that holds it.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when `index` is
    /// past the end; a stored row never refuses.
    pub fn scalar(&self, index: usize) -> crate::Result<Scalar> {
        let (chunk, offset) = self.located(index)?;
        self.chunks[chunk].scalar(offset)
    }

    /// Row `index`, or `None` past the end.
    pub fn get(&self, index: usize) -> Option<Scalar> {
        let (chunk, offset) = self.locate(index)?;
        Some(proven_row(&self.chunks[chunk], offset))
    }

    /// Every row, built once, chunk after chunk.
    pub fn rows(&self) -> Vec<Scalar> {
        self.iter().collect()
    }

    /// The rows, each built as the walk reaches it.
    pub fn iter(&self) -> ChunkedRows<'_> {
        ChunkedRows {
            chunks: self.chunks.iter(),
            current: None,
        }
    }

    /// The window `offset..offset + length`, zero copy: the chunks it
    /// reaches, found by two binary searches, the two at its edges sliced
    /// and every other one a pointer bump; a zero-length window keeps none.
    ///
    /// # Errors
    ///
    /// Returns an error naming the serie and both counts when the window
    /// reaches past the end.
    pub fn slice(&self, offset: usize, length: usize) -> crate::Result<Self> {
        require_window(self.field.name(), offset, length, self.len())?;
        let chunks = match self.reach(offset, length) {
            Some(reach) => {
                let mut chunks = Vec::with_capacity(reach.1 - reach.0 + 1);
                self.push_pieces(offset, length, reach, &mut chunks)?;
                chunks
            }
            None => Vec::new(),
        };
        Ok(Self::from_landed(Arc::clone(&self.field), chunks))
    }

    /// The first and the last chunk the window `offset..offset + length`
    /// reaches, by two binary searches; `None` for a zero-length window.
    /// The caller proved the window.
    fn reach(&self, offset: usize, length: usize) -> Option<(usize, usize)> {
        let last = length.checked_sub(1)?;
        Some((self.locate(offset)?.0, self.locate(offset + last)?.0))
    }

    /// Push the pieces of the chunks `first..=last` the window
    /// `offset..offset + length` reaches onto `pieces`, in order, its room
    /// reserved once and amortized - none where the caller sized the vector:
    /// the two at its edges sliced, every other one a pointer bump. The
    /// caller found the reach with [`Self::reach`] or [`Self::located`].
    fn push_pieces(
        &self,
        offset: usize,
        length: usize,
        (first, last): (usize, usize),
        pieces: &mut Vec<Serie>,
    ) -> crate::Result<()> {
        let end = offset + length;
        pieces.reserve(last - first + 1);
        let mut start = if first == 0 { 0 } else { self.ends[first - 1] };
        for chunk in &self.chunks[first..=last] {
            let stop = start + chunk.len();
            let low = offset.max(start);
            let high = end.min(stop);
            pieces.push(if low == start && high == stop {
                chunk.clone()
            } else {
                chunk.slice(low - start, high - low)?
            });
            start = stop;
        }
        Ok(())
    }

    /// The chunked serie `select` reaches in every chunk, or `None` where it
    /// reaches nothing: a pointer bump per chunk into one vector sized up
    /// front, and with no chunk the field `select` reaches in the empty
    /// column of this field.
    fn select(&self, select: impl Fn(&Serie) -> Option<&Serie>) -> Option<Self> {
        let mut chunks = Vec::with_capacity(self.chunks.len());
        for chunk in &self.chunks {
            chunks.push(select(chunk)?.clone());
        }
        let field = match chunks.first() {
            Some(first) => Arc::clone(first.field_ref()?),
            None => {
                let probe = Serie::empty(Arc::clone(&self.field)).ok()?;
                Arc::clone(select(&probe)?.field_ref()?)
            }
        };
        Some(Self::from_landed(field, chunks))
    }

    /// A record's child named `name`, chunk by chunk; `None` elsewhere.
    ///
    /// This is what a table's column is: the child of every batch, under
    /// the child field, nothing copied.
    pub fn child(&self, name: &str) -> Option<Self> {
        self.select(|chunk| chunk.child(name))
    }

    /// A record's child, or a union's member, at `index`.
    pub fn child_at(&self, index: usize) -> Option<Self> {
        self.select(|chunk| chunk.child_at(index))
    }

    /// Every child of a record, or member of a union; empty elsewhere.
    pub fn children(&self) -> Vec<Self> {
        let Some(first) = self.chunks.first() else {
            let Ok(probe) = Serie::empty(Arc::clone(&self.field)) else {
                return Vec::new();
            };
            return probe
                .children()
                .iter()
                .filter_map(|child| {
                    Some(Self::from_landed(
                        Arc::clone(child.field_ref()?),
                        Vec::new(),
                    ))
                })
                .collect();
        };
        (0..first.children().len())
            .filter_map(|index| self.child_at(index))
            .collect()
    }

    /// A sequence's items, a mapping's entries, an encoding's values.
    pub fn items(&self) -> Option<Self> {
        self.select(Serie::items)
    }

    /// The chunked column `path` reaches, exactly as
    /// [`Serie::get_child_by_path`] reaches it in each chunk.
    pub fn get_child_by_path(&self, path: &FieldPath) -> Option<Self> {
        self.select(|chunk| chunk.get_child_by_path(path))
    }

    /// Append one chunk: a column under the field as it stands, any other
    /// column cast into it under `options` by one plan compiled here.
    ///
    /// # Errors
    ///
    /// Returns an error for a run, and the plan's refusals for a column the
    /// field cannot hold; a refusal leaves this serie as it was.
    pub fn push_chunk(
        &mut self,
        chunk: Serie,
        options: ArrowCastOptions,
    ) -> crate::arrow::Result<()> {
        let own = chunk.require_field()?;
        let landed = if own == self.field.as_ref() {
            chunk
        } else {
            chunk.cast(&self.field, options)?
        };
        let end = self.len() + landed.len();
        self.chunks.push(landed);
        self.ends.push(end);
        Ok(())
    }

    /// Every row as one column: the one join.
    ///
    /// No chunk is the empty column of the field, one chunk is itself, and
    /// several are concatenated once and landed as rows the chunks already
    /// proved, so no row is read.
    ///
    /// # Errors
    ///
    /// Returns an error when the joined buffers outgrow what one column can
    /// index - offsets past their width, or a dictionary whose gathered
    /// vocabulary no key of its width reaches, naming its path - or
    /// [`Serie::empty`]'s refusal.
    pub fn into_serie(&self) -> crate::arrow::Result<Serie> {
        match self.chunks.as_slice() {
            [] => Ok(Serie::empty(Arc::clone(&self.field))?),
            [one] => Ok(one.clone()),
            many => {
                let arrays: Vec<ArrayRef> = many
                    .iter()
                    .map(|chunk| chunk.into_arrow_array().expect(CHUNK))
                    .collect();
                require_joinable(&self.field, &arrays)?;
                let borrowed: Vec<&dyn Array> = arrays.iter().map(AsRef::as_ref).collect();
                let joined = arrow_select::concat::concat(&borrowed)?;
                land(Arc::clone(&self.field), joined, &Proof::Proven)
            }
        }
    }

    /// Every chunk under `target`: one plan compiled, applied to each.
    ///
    /// A chunked serie already under `target` is itself, chunks shared.
    ///
    /// # Errors
    ///
    /// The plan's refusals, raised before a chunk is touched where the two
    /// fields alone decide them.
    pub fn cast(&self, target: &Field, options: ArrowCastOptions) -> crate::arrow::Result<Self> {
        if self.field.as_ref() == target {
            return Ok(self.clone());
        }
        let plan = ArrowCastPlan::compile(&self.field, target, options)?;
        let mut chunks = Vec::with_capacity(self.chunks.len());
        for chunk in &self.chunks {
            chunks.push(plan.apply(chunk)?);
        }
        let field = Self::field_of(&chunks, || Arc::new(target.clone()));
        Ok(Self::from_landed(field, chunks))
    }

    // --------------------------------------------------------------------
    // Ordering, uniqueness and grouping: what a `Serie` answers, across
    // the chunks - per chunk where a chunk alone can answer, and through
    // the one join where the rows must be seen together.
    // --------------------------------------------------------------------

    /// The row positions in sorted order under `options`, over every
    /// chunk, as a `uint32` column named `index`: the one join, then
    /// [`Serie::sort_indices`].
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, SortOptions};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![3, 1]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![2]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let order = prices.sort_indices(SortOptions::default())?;
    /// assert_eq!(order.rows().to_vec(), vec![Scalar::from(1_u32), Scalar::from(2_u32), Scalar::from(0_u32)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_serie`]'s refusal and [`Serie::sort_indices`]'s.
    pub fn sort_indices(&self, options: SortOptions) -> crate::Result<Serie> {
        self.into_serie()?.sort_indices(options)
    }

    /// Whether the rows are in sorted order under `options` across the
    /// chunks: every chunk sorted, and at every chunk edge the last row of
    /// one no greater than the first of the next, compared on the rung the
    /// chunks sort on - Arrow's comparator over both chunks' buffers where
    /// they order as their values, the values' own order otherwise - so the
    /// answer is the joined column's. One comparator, or one row per side,
    /// per edge, and no join.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, SortOptions};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![2, 3]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// assert!(prices.is_sorted(SortOptions::default()));
    /// assert!(!prices.is_sorted(SortOptions::descending()));
    /// # Ok(())
    /// # }
    /// ```
    pub fn is_sorted(&self, options: SortOptions) -> bool {
        if !self.chunks.iter().all(|chunk| chunk.is_sorted(options)) {
            return false;
        }
        let mut edges = self.chunks.iter().filter(|chunk| !chunk.is_empty());
        let Some(mut before) = edges.next() else {
            return true;
        };
        for chunk in edges {
            if before.compare_across(before.len() - 1, chunk, 0, options) == Ordering::Greater {
                return false;
            }
            before = chunk;
        }
        true
    }

    /// Whether no two rows across the chunks hold one value: the one
    /// join, then [`Serie::is_unique`]; a join the chunks refuse - a
    /// dictionary whose gathered vocabulary outgrows its key - walks the
    /// rows into one set instead, one row built per row.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![2]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// assert!(!prices.is_unique());
    /// assert!(prices.slice(0, 2)?.is_unique());
    /// # Ok(())
    /// # }
    /// ```
    pub fn is_unique(&self) -> bool {
        match self.into_serie() {
            Ok(joined) => joined.is_unique(),
            Err(_) => {
                // `Scalar`'s hash reads canonical content only, never the
                // interior-mutable caches a datatype holds.
                #[allow(clippy::mutable_key_type)]
                let mut seen: std::collections::HashSet<Scalar> =
                    std::collections::HashSet::with_capacity(self.len());
                self.iter().all(|row| seen.insert(row))
            }
        }
    }

    /// How many distinct values the rows hold across the chunks: the one
    /// join, then [`Serie::unique_count`], or the rows walked into one set
    /// where the join is refused, as [`Self::is_unique`] walks them.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, true);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![None, Some(1)]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// assert_eq!(prices.unique_count(), 2);
    /// # Ok(())
    /// # }
    /// ```
    pub fn unique_count(&self) -> usize {
        match self.into_serie() {
            Ok(joined) => joined.unique_count(),
            Err(_) => {
                #[allow(clippy::mutable_key_type)]
                let seen: std::collections::HashSet<Scalar> = self.iter().collect();
                seen.len()
            }
        }
    }

    /// The rows in sorted order under `options`, as a chunked serie of one
    /// chunk: the one join, then the sort.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, SortOptions};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![3, 1]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![2]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let sorted = prices.into_sorted(SortOptions::descending())?;
    /// assert_eq!(sorted.num_chunks(), 1);
    /// assert_eq!(sorted.rows(), vec![Scalar::from(3_i64), Scalar::from(2_i64), Scalar::from(1_i64)]);
    /// assert_eq!(prices.num_chunks(), 2);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_serie`]'s refusal and [`Serie::into_sorted`]'s.
    pub fn into_sorted(&self, options: SortOptions) -> crate::Result<Self> {
        let sorted = self.into_serie()?.into_sorted(options)?;
        Ok(Self::from_landed(Arc::clone(&self.field), vec![sorted]))
    }

    /// The first occurrence of every value across the chunks, as a chunked
    /// serie of one chunk: the one join, then [`Serie::into_unique`].
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, StringArray};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("venue", DataType::utf8(), false);
    /// let first: ArrayRef = Arc::new(StringArray::from(vec!["XNYS", "XNAS"]));
    /// let second: ArrayRef = Arc::new(StringArray::from(vec!["XNYS"]));
    /// let venues = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// assert_eq!(venues.into_unique()?.rows(), vec![Scalar::from("XNYS"), Scalar::from("XNAS")]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_serie`]'s refusal and [`Serie::into_unique`]'s.
    pub fn into_unique(&self) -> crate::Result<Self> {
        let unique = self.into_serie()?.into_unique()?;
        Ok(Self::from_landed(Arc::clone(&self.field), vec![unique]))
    }

    /// The rows in reverse order: the chunks reversed, each reversed, kept
    /// apart.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![3]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let reversed = prices.into_reversed();
    /// assert_eq!(reversed.num_chunks(), 2);
    /// assert_eq!(reversed.rows(), vec![Scalar::from(3_i64), Scalar::from(2_i64), Scalar::from(1_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn into_reversed(&self) -> Self {
        let chunks = self.chunks.iter().rev().map(Serie::into_reversed).collect();
        Self::from_landed(Arc::clone(&self.field), chunks)
    }

    /// The rows `indices` names, in that order, as a chunked serie of one
    /// chunk: the one join, then [`Serie::into_taken`]. `indices` is an
    /// integer column or run naming rows across the chunks.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![3]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let picked = prices.into_taken(&Serie::new(vec![Scalar::from(2_u32), Scalar::from(0_u32)]))?;
    /// assert_eq!(picked.rows(), vec![Scalar::from(3_i64), Scalar::from(1_i64)]);
    /// assert!(prices.into_taken(&Serie::new(vec![Scalar::from(3_u32)])).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_serie`]'s refusal and [`Serie::into_taken`]'s.
    pub fn into_taken(&self, indices: &Serie) -> crate::Result<Self> {
        let taken = self.into_serie()?.into_taken(indices)?;
        Ok(Self::from_landed(Arc::clone(&self.field), vec![taken]))
    }

    /// The rows `mask` keeps, chunk by chunk and kept apart: `mask` is a
    /// boolean column or run as long as the whole, cut to each chunk's
    /// window - zero copy for a column mask - and applied to that chunk
    /// alone.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![3]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let mask = Serie::new(vec![Scalar::from(true), Scalar::Null, Scalar::from(true)]);
    /// let kept = prices.into_filtered(&mask)?;
    /// assert_eq!(kept.num_chunks(), 2);
    /// assert_eq!(kept.rows(), vec![Scalar::from(1_i64), Scalar::from(3_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when `mask` is another length,
    /// and [`Serie::into_filtered`]'s refusal.
    pub fn into_filtered(&self, mask: &Serie) -> crate::Result<Self> {
        if mask.len() != self.len() {
            return Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new(self.field.name()),
                reason: smol_str::format_smolstr!(
                    "a mask of {} rows cannot filter the {} rows {} holds",
                    mask.len(),
                    self.len(),
                    self.field.name()
                ),
            });
        }
        let mut chunks = Vec::with_capacity(self.chunks.len());
        let mut start = 0;
        for chunk in &self.chunks {
            let window = mask.slice(start, chunk.len())?;
            chunks.push(chunk.into_filtered(&window)?);
            start += chunk.len();
        }
        Ok(Self::from_landed(Arc::clone(&self.field), chunks))
    }

    /// The rows grouped by `keys`, a serie as long as the whole: one
    /// `(key, rows)` per distinct key in order of first occurrence, each
    /// group's rows a chunked serie whose chunks are what each chunk
    /// contributed, kept apart - so sorted keys cut every group as zero-copy
    /// slices of the chunks. Keys held as a chunked serie are
    /// [`Self::partition_by_chunked`]'s.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![3]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let venues = Serie::new(vec![Scalar::from("XNAS"), Scalar::from("XNYS"), Scalar::from("XNAS")]);
    /// let groups = prices.partition_by(&venues)?;
    /// assert_eq!(groups.len(), 2);
    /// assert_eq!(groups[0].0, Scalar::from("XNAS"));
    /// assert_eq!(groups[0].1.num_chunks(), 2);
    /// assert_eq!(groups[0].1.rows(), vec![Scalar::from(1_i64), Scalar::from(3_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when `keys` is another length,
    /// and [`Serie::partition_by`]'s refusal.
    pub fn partition_by(&self, keys: &Serie) -> crate::Result<Vec<(Scalar, Self)>> {
        self.require_keys(keys.len())?;
        let mut start = 0;
        self.partition_chunks(|chunk| {
            let window = keys.slice(start, chunk.len())?;
            start += chunk.len();
            Ok(Cow::Owned(window))
        })
    }

    /// The rows grouped by `keys`, a chunked serie as long as the whole,
    /// exactly as [`Self::partition_by`] groups them: where the two are cut
    /// at the same rows each chunk of rows is grouped by the chunk of keys
    /// beside it, with no join; otherwise the keys are joined once and cut
    /// to each chunk of rows.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array, StringArray};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let price = Field::new("price", DataType::Int64, false);
    /// let venue = Field::new("venue", DataType::utf8(), false);
    /// let prices: [ArrayRef; 2] = [Arc::new(Int64Array::from(vec![1, 2])), Arc::new(Int64Array::from(vec![3]))];
    /// let venues: [ArrayRef; 2] = [
    ///     Arc::new(StringArray::from(vec!["XNAS", "XNYS"])),
    ///     Arc::new(StringArray::from(vec!["XNAS"])),
    /// ];
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&price), prices, ArrowCastOptions::new())?;
    /// let venues = ChunkedSerie::from_arrow_arrays(Some(&venue), venues, ArrowCastOptions::new())?;
    /// let groups = prices.partition_by_chunked(&venues)?;
    /// assert_eq!(groups.len(), 2);
    /// assert_eq!(groups[0].0, Scalar::from("XNAS"));
    /// assert_eq!(groups[0].1.rows(), vec![Scalar::from(1_i64), Scalar::from(3_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when `keys` is another length,
    /// [`Self::into_serie`]'s refusal where the keys are joined, and
    /// [`Serie::partition_by`]'s refusal.
    pub fn partition_by_chunked(&self, keys: &Self) -> crate::Result<Vec<(Scalar, Self)>> {
        self.require_keys(keys.len())?;
        if keys.ends == self.ends {
            let mut chunks = keys.chunks.iter();
            return self.partition_chunks(|_| {
                Ok(Cow::Borrowed(
                    chunks
                        .next()
                        .expect("keys cut at the same rows hold as many chunks"),
                ))
            });
        }
        self.partition_by(&keys.into_serie()?)
    }

    /// The rows cut into windows of equal adjacent keys across the chunks,
    /// exactly as [`Serie::window_by`] cuts the joined column, `sorted`
    /// meaning what it means there: one `(key, rows)` per maximal run of
    /// adjacent rows whose keys are equal, in row order, every window a
    /// zero-copy [`Self::slice`] keeping the chunks it reaches - so a run
    /// that crosses a chunk edge is one window over both. With `sorted`,
    /// each key once and in key order - ascending, absent keys last: keys
    /// already in order answer the same windows, and keys out of order
    /// regroup the runs stably by key, each window the zero-copy pieces of
    /// its runs in the order they arrived. No row is copied and the chunks
    /// are never joined, so a key that changes on every row answers a piece
    /// per row: [`Self::into_serie`] first, then [`Serie::window_by`], is
    /// the cheaper door there.
    ///
    /// No window states a record of static values, as a held window
    /// [`Serie::window_by`] lends does
    /// ([`WindowSerie::static_values`](crate::WindowSerie::static_values)):
    /// a window's key is the first half of its item, and its place among
    /// the windows - what a record calls `windownum` - is its place in the
    /// `Vec`. So a key cell named `windownum` or `rownum`, which
    /// [`Serie::window_by`] refuses because its record names both, is taken
    /// here; alias it before joining the chunks to window the joined column.
    ///
    /// The key is bound once, against the field, and computed chunk by
    /// chunk; at each chunk edge the pending window's key, already built, is
    /// compared in place against the next chunk's first key row, as the
    /// ordering verbs order them, so a run continuing across the edge builds
    /// no key. An empty chunk adds no row and no edge. The cost is one bind,
    /// one key column, comparator and bitmap per chunk holding a row - each
    /// on the terms [`Serie::window_by`] states for its key - and per window
    /// its key and its slice, with no join; a regrouping adds one stable
    /// sort of the runs, never of the rows.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, StringArray};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("venue", DataType::utf8(), false);
    /// let first: ArrayRef = Arc::new(StringArray::from(vec!["XNAS", "XNAS"]));
    /// let second: ArrayRef = Arc::new(StringArray::from(vec!["XNAS", "XNYS"]));
    /// let venues = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let windows = venues.window_by("venue", false)?;
    /// assert_eq!(windows.len(), 2);
    /// // The XNAS run crosses the chunk edge, and stays one window over both.
    /// assert_eq!(windows[0].0, Scalar::from_sequence([Scalar::from("XNAS")]));
    /// assert_eq!((windows[0].1.len(), windows[0].1.num_chunks()), (3, 2));
    /// assert_eq!(windows[1].1.rows(), vec![Scalar::from("XNYS")]);
    ///
    /// // Sorted over keys out of order: each key once, its runs kept apart.
    /// let first: ArrayRef = Arc::new(StringArray::from(vec!["XNYS", "XNAS"]));
    /// let second: ArrayRef = Arc::new(StringArray::from(vec!["XNAS", "XNYS"]));
    /// let venues = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let windows = venues.window_by("venue", true)?;
    /// assert_eq!(windows.len(), 2);
    /// assert_eq!(windows[0].0, Scalar::from_sequence([Scalar::from("XNAS")]));
    /// assert_eq!((windows[0].1.len(), windows[0].1.num_chunks()), (2, 2));
    /// assert_eq!(windows[1].1.rows(), vec![Scalar::from("XNYS"); 2]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::window_by`]'s refusals but the one for a key cell named
    /// `windownum` or `rownum`, naming the field, before any row is read -
    /// with no chunk at all as with many.
    pub fn window_by(
        &self,
        by: impl IntoSelector,
        sorted: bool,
    ) -> crate::Result<Vec<(Scalar, Self)>> {
        let key = by.into_selector()?.bind_key(
            &SerieReader::root_of(&self.field)?,
            self.field.name(),
            "window by",
        )?;
        // Each run as its key, the row it starts at and its length, in
        // arrival order.
        let mut runs: Vec<(Scalar, usize, usize)> = Vec::new();
        let mut descent = false;
        let mut base = 0;
        for chunk in self.chunks.iter().filter(|chunk| !chunk.is_empty()) {
            let keys = key.apply_serie(chunk)?;
            let cut = keys.window_starts(false)?;
            descent |= cut.descent.is_some();
            let mut start = 0;
            while start < keys.len() {
                let end = window_end(&cut.starts, start);
                // Only a chunk's first window meets an edge, against the
                // pending window's key; a continuing run builds no key.
                let edge = match runs.last() {
                    Some(last) if start == 0 => {
                        keys.compare_to_row(&last.0, 0, SortOptions::default())
                    }
                    _ => Ordering::Less,
                };
                if edge == Ordering::Equal
                    && let Some(last) = runs.last_mut()
                {
                    last.2 += end;
                } else {
                    descent |= edge == Ordering::Greater;
                    runs.push((proven_row(&keys, start), base + start, end - start));
                }
                start = end;
            }
            base += chunk.len();
        }
        if !(sorted && descent) {
            return runs
                .into_iter()
                .map(|(value, offset, length)| Ok((value, self.slice(offset, length)?)))
                .collect();
        }
        // Stable, so the runs of one key keep their arrival order.
        runs.sort_by(|left, right| compare_values(&left.0, &right.0, SortOptions::default()));
        // Each window as its key - its first run's - and its runs' pieces,
        // in one vector a window grown geometrically, never once a run.
        let mut windows: Vec<(Scalar, Vec<Serie>)> = Vec::new();
        for (value, offset, length) in runs {
            // A run is never empty, so its first and its last row locate.
            let reach = (
                self.located(offset)?.0,
                self.located(offset + length - 1)?.0,
            );
            match windows.last_mut() {
                Some((last, pieces))
                    if compare_values(last, &value, SortOptions::default()) == Ordering::Equal =>
                {
                    self.push_pieces(offset, length, reach, pieces)?;
                }
                _ => {
                    let mut pieces = Vec::new();
                    self.push_pieces(offset, length, reach, &mut pieces)?;
                    windows.push((value, pieces));
                }
            }
        }
        Ok(windows
            .into_iter()
            .map(|(value, pieces)| (value, Self::from_landed(Arc::clone(&self.field), pieces)))
            .collect())
    }

    /// Refuse a key count that is not this serie's length, naming the field.
    fn require_keys(&self, keys: usize) -> crate::Result<()> {
        if keys == self.len() {
            return Ok(());
        }
        Err(crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new(self.field.name()),
            reason: smol_str::format_smolstr!(
                "{keys} keys cannot partition the {} rows {} holds",
                self.len(),
                self.field.name()
            ),
        })
    }

    /// Group every chunk by the keys `keys_of` answers for it, as long as
    /// the chunk, and merge each chunk's groups by key in order of first
    /// occurrence, every group keeping what each chunk contributed apart.
    fn partition_chunks<'k>(
        &self,
        mut keys_of: impl FnMut(&Serie) -> crate::Result<Cow<'k, Serie>>,
    ) -> crate::Result<Vec<(Scalar, Self)>> {
        let mut groups: Vec<(Scalar, Vec<Serie>)> = Vec::new();
        // `Scalar`'s hash reads canonical content only, never the
        // interior-mutable caches a datatype holds, so the key is stable.
        #[allow(clippy::mutable_key_type)]
        let mut group_of: std::collections::HashMap<Scalar, usize> =
            std::collections::HashMap::new();
        for chunk in &self.chunks {
            let keys = keys_of(chunk)?;
            for (key, rows) in chunk.partition_by(&keys)? {
                let next = groups.len();
                let group = *group_of.entry(key.clone()).or_insert(next);
                if group == next {
                    groups.push((key, Vec::new()));
                }
                groups[group].1.push(rows);
            }
        }
        Ok(groups
            .into_iter()
            .map(|(key, chunks)| (key, Self::from_landed(Arc::clone(&self.field), chunks)))
            .collect())
    }

    /// The bytes the rows occupy: every chunk's, as its own slice counts
    /// them.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![3]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// assert_eq!(
    ///     prices.memory_size(),
    ///     prices.chunks().iter().map(|chunk| chunk.memory_size()).sum::<usize>()
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn memory_size(&self) -> usize {
        self.chunks.iter().map(Serie::memory_size).sum()
    }

    /// Sort the rows in place under `options`, answering this serie so
    /// calls chain: [`Self::into_sorted`], one chunk, replacing the chunks.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, SortOptions};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![2, 1]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![2]));
    /// let mut prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// prices.as_sorted(SortOptions::default())?.as_unique()?;
    /// assert_eq!(prices.rows(), vec![Scalar::from(1_i64), Scalar::from(2_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_sorted`]'s refusal, which leaves this serie as it was.
    pub fn as_sorted(&mut self, options: SortOptions) -> crate::Result<&mut Self> {
        *self = self.into_sorted(options)?;
        Ok(self)
    }

    /// Keep the first occurrence of every value, in place, answering this
    /// serie: [`Self::into_unique`], one chunk, replacing the chunks.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![2, 1]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![2]));
    /// let mut prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// prices.as_unique()?;
    /// assert_eq!((prices.num_chunks(), prices.rows()), (1, vec![Scalar::from(2_i64), Scalar::from(1_i64)]));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_unique`]'s refusal, which leaves this serie as it was.
    pub fn as_unique(&mut self) -> crate::Result<&mut Self> {
        *self = self.into_unique()?;
        Ok(self)
    }

    /// Reverse the rows in place, answering this serie: the chunks
    /// reversed, each reversed where it stands.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![3]));
    /// let mut prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// prices.as_reversed()?;
    /// assert_eq!(prices.num_chunks(), 2);
    /// assert_eq!(prices.rows(), vec![Scalar::from(3_i64), Scalar::from(2_i64), Scalar::from(1_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Never, in practice: the signature matches the other `as_*` writes.
    pub fn as_reversed(&mut self) -> crate::Result<&mut Self> {
        self.chunks.reverse();
        for chunk in &mut self.chunks {
            chunk.as_reversed()?;
        }
        *self = Self::from_landed(Arc::clone(&self.field), std::mem::take(&mut self.chunks));
        Ok(self)
    }

    /// Keep the rows `indices` names, in place, answering this serie:
    /// [`Self::into_taken`], one chunk, replacing the chunks.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![3]));
    /// let mut prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// prices.as_taken(&Serie::new(vec![Scalar::from(2_u32), Scalar::from(1_u32)]))?;
    /// assert_eq!(prices.rows(), vec![Scalar::from(3_i64), Scalar::from(2_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_taken`]'s refusal, which leaves this serie as it was.
    pub fn as_taken(&mut self, indices: &Serie) -> crate::Result<&mut Self> {
        *self = self.into_taken(indices)?;
        Ok(self)
    }

    /// Keep the rows `mask` keeps, in place and chunk by chunk, answering
    /// this serie.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![3]));
    /// let mut prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// prices.as_filtered(&Serie::new(vec![Scalar::from(false), Scalar::from(true), Scalar::from(true)]))?;
    /// assert_eq!((prices.num_chunks(), prices.rows()), (2, vec![Scalar::from(2_i64), Scalar::from(3_i64)]));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::into_filtered`]'s refusal, which leaves this serie as it was.
    pub fn as_filtered(&mut self, mask: &Serie) -> crate::Result<&mut Self> {
        *self = self.into_filtered(mask)?;
        Ok(self)
    }

    /// Every chunk's buffers as one Arrow array each, shared.
    pub fn into_arrow_arrays(&self) -> Vec<ArrayRef> {
        self.chunks
            .iter()
            .map(|chunk| chunk.into_arrow_array().expect(CHUNK))
            .collect()
    }

    /// Every chunk as one Arrow batch, in order: a table.
    ///
    /// A record's chunks are its batches, their children the columns; any
    /// other field's chunks are each the one column of a `row` root, named
    /// as it is, exactly as [`Serie::into_arrow_batch`] crosses one. The
    /// schema is built once and shared by every batch.
    ///
    /// # Errors
    ///
    /// Returns an error for a record chunk holding an absent row, which a
    /// table cannot state.
    pub fn into_arrow_reader(&self) -> crate::arrow::Result<BatchReader> {
        let schema = batch_schema(&self.field)?;
        let mut batches = Vec::with_capacity(self.chunks.len());
        for chunk in &self.chunks {
            batches.push(batch_under(&schema, chunk)?);
        }
        Ok(batch_reader(schema, batches))
    }
}

/// The refusal a chunked serie of nothing reads when no field names it.
/// Refuse chunk `index`, of another datatype than the first, where no
/// field was declared to cast it into.
fn disagreeing(index: usize, field: &Field, dtype: &DataType) -> crate::arrow::Error {
    crate::arrow::Error::IncompatibleSchema(format!(
        "chunk {index} of {:?} is {dtype}, and the first chunk {}; pass a field to cast into",
        field.name(),
        field.dtype()
    ))
}

/// Refuse the join Arrow's concatenation panics on rather than refuses: a
/// dictionary whose vocabularies it gathers whole - beneath a layout it
/// joins through `MutableArrayData`, or over values it never merges - past
/// the largest key of its width.
fn require_joinable(field: &Field, arrays: &[ArrayRef]) -> crate::arrow::Result<()> {
    let is_dictionary = |node: &ArrowDataType| matches!(node, ArrowDataType::Dictionary(..));
    if !arrays
        .first()
        .is_some_and(|first| storage_holds(first.data_type(), &is_dictionary))
    {
        return Ok(());
    }
    let datas: Vec<ArrayData> = arrays.iter().map(|array| array.to_data()).collect();
    let borrowed: Vec<&ArrayData> = datas.iter().collect();
    match outgrown_vocabulary(&borrowed, false) {
        None => Ok(()),
        Some((path, gathered, key)) => Err(crate::arrow::Error::Unsupported {
            kind: "serie",
            reason: format!(
                "joining {} chunks of {:?} gathers {gathered} dictionary values at ${path}, \
                 past the largest {} key ({})",
                arrays.len(),
                field.name(),
                DataType::from_arrow_datatype(&key)
                    .map_or_else(|_| key.to_string(), |key| key.to_string()),
                key_reach(&key)
            ),
        }),
    }
}

/// The first dictionary at or below `datas` - one array's data per chunk,
/// all of one layout - whose vocabularies Arrow's concatenation gathers
/// whole past the largest key of its width: its path, how many values it
/// gathers, and its key. `whole` says the node is reached through
/// `MutableArrayData`, which gathers every dictionary beneath it whole;
/// the concatenation itself merges the vocabularies of primitive and plain
/// byte values, and recurses into lists, mappings, records and runs.
fn outgrown_vocabulary(
    datas: &[&ArrayData],
    whole: bool,
) -> Option<(String, usize, ArrowDataType)> {
    use ArrowDataType as A;
    let (children, whole): (Vec<(&str, usize)>, bool) = match datas.first()?.data_type() {
        A::Dictionary(key, values) => {
            let merged = values.is_primitive()
                || matches!(
                    values.as_ref(),
                    A::Utf8 | A::LargeUtf8 | A::Binary | A::LargeBinary
                );
            let vocabularies = datas
                .iter()
                .map(|data| data.child_data().first())
                .collect::<Option<Vec<&ArrayData>>>()?;
            let shared = vocabularies.windows(2).all(|pair| pair[0].ptr_eq(pair[1]));
            let gathered: usize = vocabularies.iter().map(|values| values.len()).sum();
            return ((whole || !merged) && !shared && gathered > key_reach(key))
                .then(|| (String::new(), gathered, key.as_ref().clone()));
        }
        A::List(item)
        | A::LargeList(item)
        | A::ListView(item)
        | A::LargeListView(item)
        | A::Map(item, _) => (vec![(item.name().as_str(), 0)], whole),
        A::Struct(fields) => (
            fields
                .iter()
                .enumerate()
                .map(|(index, field)| (field.name().as_str(), index))
                .collect(),
            whole,
        ),
        A::RunEndEncoded(_, values) => (vec![(values.name().as_str(), 1)], whole),
        A::FixedSizeList(item, _) => (vec![(item.name().as_str(), 0)], true),
        A::Union(members, _) => (
            members
                .iter()
                .enumerate()
                .map(|(index, (_, member))| (member.name().as_str(), index))
                .collect(),
            true,
        ),
        _ => return None,
    };
    children.into_iter().find_map(|(name, index)| {
        let below = datas
            .iter()
            .map(|data| data.child_data().get(index))
            .collect::<Option<Vec<&ArrayData>>>()?;
        outgrown_vocabulary(&below, whole)
            .map(|(path, gathered, key)| (format!(".{name}{path}"), gathered, key))
    })
}

/// The largest key Arrow's concatenation lets a dictionary of `key` reach.
fn key_reach(key: &ArrowDataType) -> usize {
    use ArrowDataType as A;
    let largest: u64 = match key {
        A::Int8 => i8::MAX.unsigned_abs().into(),
        A::Int16 => i16::MAX.unsigned_abs().into(),
        A::Int32 => i32::MAX.unsigned_abs().into(),
        A::Int64 => i64::MAX.unsigned_abs(),
        A::UInt8 => u8::MAX.into(),
        A::UInt16 => u16::MAX.into(),
        A::UInt32 => u32::MAX.into(),
        _ => u64::MAX,
    };
    usize::try_from(largest).unwrap_or(usize::MAX)
}

fn no_field(what: &str) -> crate::arrow::Error {
    crate::arrow::Error::IncompatibleSchema(format!(
        "a chunked serie of {what} names no field; pass one"
    ))
}

impl Rows for ChunkedSerie {
    fn rows_len(&self) -> usize {
        self.len()
    }

    fn row_at(&self, index: usize) -> Cow<'_, Scalar> {
        let (chunk, offset) = self.locate(index).expect(CHUNK);
        Cow::Owned(proven_row(&self.chunks[chunk], offset))
    }
}

impl PartialEq for ChunkedSerie {
    fn eq(&self, other: &Self) -> bool {
        compare_rows(self, other) == Ordering::Equal
    }
}

impl Eq for ChunkedSerie {}

impl PartialEq<Serie> for ChunkedSerie {
    fn eq(&self, other: &Serie) -> bool {
        compare_rows(self, other) == Ordering::Equal
    }
}

impl PartialEq<ChunkedSerie> for Serie {
    fn eq(&self, other: &ChunkedSerie) -> bool {
        compare_rows(self, other) == Ordering::Equal
    }
}

impl PartialOrd<Serie> for ChunkedSerie {
    fn partial_cmp(&self, other: &Serie) -> Option<Ordering> {
        Some(compare_rows(self, other))
    }
}

impl PartialOrd<ChunkedSerie> for Serie {
    fn partial_cmp(&self, other: &ChunkedSerie) -> Option<Ordering> {
        Some(compare_rows(self, other))
    }
}

impl Ord for ChunkedSerie {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_rows(self, other)
    }
}

impl PartialOrd for ChunkedSerie {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for ChunkedSerie {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_rows(self, state);
    }
}

impl fmt::Display for ChunkedSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}[", self.field.name())?;
        for (index, row) in self.iter().enumerate() {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{row:?}")?;
        }
        formatter.write_str("]")
    }
}

impl fmt::Debug for ChunkedSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChunkedSerie")
            .field("field", &self.field)
            .field("chunks", &self.chunks.len())
            .field("len", &self.len())
            .field("nulls", &self.null_count())
            .finish()
    }
}

impl<'a> IntoIterator for &'a ChunkedSerie {
    type Item = Scalar;
    type IntoIter = ChunkedRows<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// The rows of a [`ChunkedSerie`], each built as the walk reaches it, chunk
/// after chunk.
pub struct ChunkedRows<'a> {
    chunks: std::slice::Iter<'a, Serie>,
    current: Option<Children<'a>>,
}

impl Iterator for ChunkedRows<'_> {
    type Item = Scalar;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(row) = self.current.as_mut().and_then(Iterator::next) {
                return Some(row.into_owned());
            }
            self.current = Some(self.chunks.next()?.iter());
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let held = self.current.as_ref().map_or(0, |rows| rows.size_hint().0);
        let rest: usize = self.chunks.clone().map(Serie::len).sum();
        (held + rest, Some(held + rest))
    }
}

impl std::iter::FusedIterator for ChunkedRows<'_> {}
