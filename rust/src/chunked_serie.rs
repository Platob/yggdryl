//! ChunkedSerie: many columns under one field, held apart.
//!
//! A [`Serie`] column is one contiguous set of Arrow buffers, and a
//! [`StreamChunkedSerie`] is a stream of record columns read once. Between the two
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
use std::sync::{Arc, OnceLock};

use arrow_array::{Array, ArrayRef};
use arrow_data::ArrayData;
use arrow_row::{RowConverter, SortField};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

use crate::arrow::{BatchReader, batch_reader};
use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred};
use crate::diff::one_datatype;
use crate::expression::{self, BoundSelector, IntoOrderings, Projection, Selector};
use crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE;
use crate::serie::arrow::{
    batch_schema, batch_under, item_field, land_planned, lands_exactly, storage_holds,
};
use crate::serie::{
    Proof, Resolved, Rows, compare_rows, compare_values, hash_rows, land, proven_row,
    require_window, stored_order_is_value_order,
};
use crate::value::Children;
use crate::{
    DataType, Field, FieldPath, Scalar, Serie, SortOptions, SpillOptions, StreamChunkedSerie,
};

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
    /// [`Self::resident_size`] kept beside the chunks, so a push settles
    /// against the bound without walking every chunk: summed where the
    /// chunks are paired with the field, added to per push, read again
    /// after a spill. Every write replaces the chunks through
    /// [`Self::from_landed`], which is what keeps it exact.
    resident: usize,
    /// The one join, kept once a [`Serie`] holding this chunked serie is
    /// asked what only one column answers - a typed narrowing, a cast, a
    /// sort - and the failure it met, where it met one. A push or a spill
    /// lets it go; every other write is a new value.
    held: OnceLock<(Serie, Option<smol_str::SmolStr>)>,
}

impl ChunkedSerie {
    /// Pair `field` with chunks the caller already landed under it: the
    /// doors here, and a plan whose target it is.
    pub(crate) fn from_landed(field: Arc<Field>, chunks: Vec<Serie>) -> Self {
        let mut ends = Vec::with_capacity(chunks.len());
        let mut end = 0;
        let mut resident = 0;
        for chunk in &chunks {
            end += chunk.len();
            ends.push(end);
            resident += chunk.resident_size();
        }
        Self {
            field,
            chunks,
            ends,
            resident,
            held: OnceLock::new(),
        }
    }

    /// No chunk yet under `field`, with room for `capacity` chunks and
    /// their ends: what a verb that pushes its chunks one by one starts
    /// from, so neither vector grows on the way.
    fn with_chunk_capacity(field: Arc<Field>, capacity: usize) -> Self {
        Self {
            field,
            chunks: Vec::with_capacity(capacity),
            ends: Vec::with_capacity(capacity),
            resident: 0,
            held: OnceLock::new(),
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
        Ok(Self::from_landed(field, landed).verified_edges()?)
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
                    chunks.push(chunk.verified_order()?);
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
            chunks.push(current.1.cast_array(array)?.verified_order()?);
            plan = Some(current);
        }
        let field = Self::field_of(&chunks, || field);
        Ok(Self::from_landed(field, chunks).verified_edges()?)
    }

    /// Drain a [`StreamChunkedSerie`] into its chunks: one record column per batch,
    /// under the reader's root, none joined.
    ///
    /// # Errors
    ///
    /// Returns the first failure a batch raises, after which the reader is
    /// fused.
    pub fn from_chunked_stream(reader: StreamChunkedSerie) -> crate::arrow::Result<Self> {
        let mut root = Some(reader.field().clone());
        let mut chunked = Self::with_chunk_capacity(Arc::new(reader.field().clone()), 0);
        for chunk in reader.into_chunks() {
            let chunk = chunk?;
            // The first chunk's field is the one every chunk shares.
            if let Some(root) = root.take() {
                chunked.field = chunk.field_ref().map_or_else(|| Arc::new(root), Arc::clone);
            }
            chunked.push_landed(chunk)?;
        }
        Ok(chunked)
    }

    /// Drain an Arrow batch stream into its chunks: of its own schema, or
    /// cast into `root` by one plan.
    ///
    /// This is the door a `pyarrow.Table` takes: [`StreamChunkedSerie::from_arrow_reader`]
    /// collected, one chunk per batch and none joined, where
    /// [`Serie::from_arrow_reader`] joins them into one column.
    ///
    /// # Errors
    ///
    /// [`StreamChunkedSerie::from_arrow_reader`] carries the rule, and the reader
    /// carries its own.
    pub fn from_arrow_reader(
        root: Option<&Field>,
        reader: BatchReader,
        options: ArrowCastOptions,
    ) -> crate::arrow::Result<Self> {
        Self::from_chunked_stream(StreamChunkedSerie::from_arrow_reader(
            root, reader, options,
        )?)
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
        if let Some(by) = self.declared_order()?
            && let Some(last) = self.chunks.last()
            && !last.edge_in_order(&landed, &by)?
        {
            return Err(self.edge_refusal(self.chunks.len(), &by).into());
        }
        self.push_landed(landed)?;
        Ok(())
    }

    /// The `order by` keys this chunked record's field declares its rows
    /// keep across every chunk: [`Serie::declared_order`] over the field.
    ///
    /// # Errors
    ///
    /// [`Serie::declared_order`]'s.
    pub fn declared_order(&self) -> crate::Result<Option<Vec<expression::Ordering>>> {
        if self.field.dtype().as_fields().is_none() || !self.field.as_sort().declares_order() {
            return Ok(None);
        }
        self.field.as_sort().by()
    }

    /// Whether the rows are already in the order `by` states, read with no
    /// copy: the field declares at least `by`, proven where the chunks
    /// landed, or every chunk's rows and every chunk edge are read once in
    /// it - what a writer asks before sorting rows that usually arrive in
    /// order.
    ///
    /// # Errors
    ///
    /// The binder's refusal of a key.
    #[cfg(feature = "iceberg")]
    pub(crate) fn keeps_order(&self, by: &[expression::Ordering]) -> crate::Result<bool> {
        if by.is_empty() {
            return Ok(true);
        }
        if self
            .declared_order()?
            .is_some_and(|declared| declared.starts_with(by))
        {
            return Ok(true);
        }
        for chunk in &self.chunks {
            if chunk.first_disorder(by)?.is_some() {
                return Ok(false);
            }
        }
        for pair in self.chunks.windows(2) {
            if !pair[0].edge_in_order(&pair[1], by)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// This chunked serie checked at every chunk edge against the order its
    /// field declares - the last row of each chunk against the first of the
    /// next - refusing the first edge out of order by its chunk; a field
    /// declaring none as it is. Each chunk's own rows are checked where it
    /// lands.
    pub(crate) fn verified_edges(self) -> crate::Result<Self> {
        let Some(by) = self.declared_order()? else {
            return Ok(self);
        };
        for (index, pair) in self.chunks.windows(2).enumerate() {
            if !pair[0].edge_in_order(&pair[1], &by)? {
                return Err(self.edge_refusal(index + 1, &by));
            }
        }
        Ok(self)
    }

    /// The refusal of chunk `index` opening out of the declared order.
    fn edge_refusal(&self, index: usize, by: &[expression::Ordering]) -> crate::Error {
        crate::Error::InvalidRecord {
            path: smol_str::SmolStr::new(self.field.name()),
            reason: smol_str::format_smolstr!(
                "chunk {index} of {} opens out of the order its field declares, `{}`",
                self.field.name(),
                crate::serie::spelled(by)
            ),
        }
    }

    /// This chunked record under its field declaring `by` as the order its
    /// rows keep, every chunk declaring it too; anything but a record, or an
    /// empty `by`, as it is.
    fn declaring_order(self, by: &[expression::Ordering]) -> crate::Result<Self> {
        if by.is_empty() || self.field.dtype().as_fields().is_none() {
            return Ok(self);
        }
        let mut root = (*self.field).clone();
        if root.as_sort_mut().set_by(by.iter().cloned()).is_err() {
            return Ok(self);
        }
        let chunks = self
            .chunks
            .iter()
            .map(|chunk| chunk.clone().declaring_order(by))
            .collect::<crate::Result<Vec<_>>>()?;
        Ok(Self::from_landed(Arc::new(root), chunks))
    }

    /// The keys a whole-row sort of this record under `options` is: every
    /// child in declaration order, each under `options`; `None` for
    /// anything but a record.
    fn whole_row_order(&self, options: SortOptions) -> Option<Vec<expression::Ordering>> {
        let fields = self.field.dtype().as_fields()?;
        Some(
            fields
                .iter()
                .map(|child| {
                    expression::Ordering::new(expression::Term::column(child.name()), options)
                })
                .collect(),
        )
    }

    /// Append one chunk already landed under the field, then settle: the
    /// heaviest chunks spilled once the chunks pass the process bound. The
    /// bound is read against the running resident total, so a push under it
    /// walks no chunk.
    pub(crate) fn push_landed(&mut self, landed: Serie) -> crate::Result<()> {
        self.held = OnceLock::new();
        let end = self.len() + landed.len();
        self.resident += landed.resident_size();
        self.chunks.push(landed);
        self.ends.push(end);
        let options = SpillOptions::from_env()?;
        if options.is_never()
            || u64::try_from(self.resident).unwrap_or(u64::MAX) <= options.byte_size()
        {
            return Ok(());
        }
        self.spill(options)
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
            // Nothing laid out: the empty column, or the one chunk itself.
            [] | [_] => self.joined(),
            _ => Ok(self.joined()?.settled()?),
        }
    }

    /// The rows as one column, joined once and kept beside the chunks - what
    /// a [`Serie`] holding this chunked serie reads where only one column
    /// answers - and the failure the join met, where it met one, answered
    /// beside the empty column of the field.
    pub(crate) fn held(&self) -> &(Serie, Option<smol_str::SmolStr>) {
        self.held
            .get_or_init(|| crate::shared_stream::held_join(self, None))
    }

    /// A kept join's failure, without requesting that join.
    pub(crate) fn held_failure(&self) -> Option<&smol_str::SmolStr> {
        self.held.get().and_then(|(_, failure)| failure.as_ref())
    }

    /// The chunks, each a column of the field, in row order.
    #[must_use]
    pub fn into_chunks(self) -> Vec<Serie> {
        self.chunks
    }

    /// [`Self::into_serie`] before it settles: the one join, resident
    /// whatever the process default, for a caller that settles the column
    /// under a bound of its own.
    pub(crate) fn joined(&self) -> crate::arrow::Result<Serie> {
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
        Ok(Self::from_landed(field, chunks).verified_edges()?)
    }

    // --------------------------------------------------------------------
    // Ordering, uniqueness and grouping: what a `Serie` answers, across
    // the chunks - per chunk where a chunk alone can answer, by merging the
    // chunks sorted on their own where the rows come out in order, and
    // through the one join where an answer is one column over every row.
    // --------------------------------------------------------------------

    /// The row positions in sorted order under `options`, over every
    /// chunk, as a `uint32` column named `index`: the one join, then
    /// [`Serie::sort_indices`]. The positions are one column addressing
    /// every row, so they stay the join; the rows themselves in order are
    /// [`Self::into_sorted`]'s merge, which joins nothing.
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

    /// The row positions in the order the `order by` keys of `by` state,
    /// over every chunk, as a `uint32` column named `index`: the keys
    /// resolved, then the one join, then [`Serie::sort_indices_by`] - its
    /// rung, its stability. The positions are one column addressing every
    /// row, so they stay the join; the rows themselves in order are
    /// [`Self::into_sort_by`]'s merge, which joins nothing.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![3, 1]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![2]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let order = prices.sort_indices_by("price desc")?;
    /// assert_eq!(order.rows().to_vec(), vec![Scalar::from(0_u32), Scalar::from(2_u32), Scalar::from(1_u32)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// The keys' parse error before the join, then [`Self::into_serie`]'s
    /// refusal and [`Serie::sort_indices_by`]'s.
    pub fn sort_indices_by(&self, by: impl IntoOrderings) -> crate::Result<Serie> {
        let by = by.into_orderings()?;
        self.into_serie()?.sort_indices_by(by)
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

    /// The rows in sorted order under `options`, this serie untouched, with
    /// no join: each chunk sorted on its own by [`Serie::into_sorted`], then
    /// the sorted chunks merged into chunks of at most
    /// [`DEFAULT_RECORD_BATCH_ROW_SIZE`] rows. The rows and their order are
    /// the joined column's [`Serie::into_sorted`] - stable, rows of one
    /// value in chunk order, then in row order - and no `uint32` index
    /// column bounds them, only each chunk.
    ///
    /// The merge keeps one cursor per chunk in a binary heap, the least key
    /// on top and a tie going to the earlier chunk. A cursor reads its
    /// sorted chunk a block at a time - [`DEFAULT_RECORD_BATCH_ROW_SIZE`]
    /// rows shared among the chunks, never fewer than 1,024 a chunk - and
    /// every comparison is on one rung for the whole merge: where the key's
    /// stored order is its value order, no float lies beneath it and Arrow's
    /// row format has a layout for it, each block's keys are encoded once by
    /// one converter under `options` and compared as bytes, nothing built
    /// per row; otherwise - a version, a windows-1252 text, a registered
    /// code, a URL, a union, a float, whose foreign NaN the format would
    /// order by its bits - each cursor builds its current key as it advances
    /// and compares it as the values order. Each output batch is the
    /// positions the merge yielded, gathered by one `interleave` over the
    /// sorted chunks, landed as rows the chunks already proved, and settled.
    /// One chunk holding rows merges nothing: its sorted self is cut into
    /// zero-copy slices.
    ///
    /// At any time the sort holds the sorted chunks, settled as one chunked
    /// serie as each lands - the heaviest spilled first past the process
    /// bound ([`SpillOptions::from_env`]) - and dropped when the merge ends;
    /// one cursor per chunk and its block's key rows; the current output
    /// batch's positions and its gathered arrays; and the output so far,
    /// settled the same way. The rows are copied twice - sorted, then
    /// gathered - and never a third time.
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
    /// [`Serie::into_sorted`]'s refusal for a chunk, and, where two chunks
    /// or more hold rows, [`Self::into_serie`]'s refusal of a dictionary
    /// whose gathered vocabulary outgrows its key.
    pub fn into_sorted(&self, options: SortOptions) -> crate::Result<Self> {
        if let Some(whole) = self.whole_row_order(options)
            && self
                .declared_order()?
                .is_some_and(|declared| declared.starts_with(&whole))
        {
            // The field proves the order across every chunk edge: a clone.
            return Ok(self.clone());
        }
        let sorted = self.sorted_chunks(|chunk| chunk.into_sorted(options))?;
        let merged = sorted.merged(&[options], |chunk, start, len| {
            Ok(vec![chunk.slice(start, len)?])
        })?;
        match self.whole_row_order(options) {
            Some(by) => merged.declaring_order(&by),
            None => Ok(merged),
        }
    }

    /// The rows in the order the `order by` keys of `by` state, this serie
    /// untouched, with no join: the keys bound once against the field, each
    /// chunk sorted on its own by [`Serie::into_sort_by`], then the sorted
    /// chunks merged as [`Self::into_sorted`] merges them - its rung, its
    /// blocks, its batches, what it holds - over the key cells the bound
    /// keys compute for each cursor's block, each under its own key's
    /// options. The rows and their order are the joined column's
    /// [`Serie::into_sort_by`]: stable, rows whose every key is equal in
    /// chunk order, then in row order; a row the record leaves absent is
    /// absent in every key.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use arrow_array::{ArrayRef, Int64Array};
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("price", DataType::Int64, false);
    /// let first: ArrayRef = Arc::new(Int64Array::from(vec![3, 1]));
    /// let second: ArrayRef = Arc::new(Int64Array::from(vec![2]));
    /// let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let sorted = prices.into_sort_by("price")?;
    /// assert_eq!(sorted.num_chunks(), 1);
    /// assert_eq!(sorted.rows(), vec![Scalar::from(1_i64), Scalar::from(2_i64), Scalar::from(3_i64)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::sort_indices_by`]'s refusals of the keys - their parse,
    /// none at all, an `unnest`, a term reaching no column or two, two keys
    /// publishing one name - raised before any chunk is read, with no chunk
    /// as with many; then [`Self::into_sorted`]'s.
    pub fn into_sort_by(&self, by: impl IntoOrderings) -> crate::Result<Self> {
        self.sorted_by_on(by, 1)
    }

    /// [`Self::into_sort_by`] with the chunks sorted on up to `threads`
    /// threads before the one merge - what a write that hands each of its
    /// parts a share of its threads sorts a part by. The result is the one
    /// the sequential sort answers, row for row: each chunk is sorted on its
    /// own whichever thread sorts it, and the sorted chunks land in chunk
    /// order, so the merge's tie to the earlier chunk keeps it stable. At
    /// most `threads` sorted chunks are held beyond what has landed and
    /// settled.
    pub(crate) fn sorted_by_on(
        &self,
        by: impl IntoOrderings,
        threads: usize,
    ) -> crate::Result<Self> {
        let by = by.into_orderings()?;
        let key = self.sort_key(&by)?;
        if self
            .declared_order()?
            .is_some_and(|declared| declared.starts_with(&by))
        {
            // The field proves the order across every chunk edge: a clone.
            return Ok(self.clone());
        }
        let sorted = self.sorted_chunks_on(threads, |chunk| chunk.into_sort_by(by.as_slice()))?;
        let options: Vec<SortOptions> = by.iter().map(expression::Ordering::options).collect();
        sorted
            .merged(&options, |chunk, start, len| {
                let keys = key.apply_serie_window(chunk, start, len)?;
                Ok(keys
                    .as_struct()
                    .expect("a key is a record column")
                    .children()
                    .to_vec())
            })?
            .declaring_order(&by)
    }

    /// The first occurrence of every value across the chunks, in order of
    /// first occurrence - the joined column's [`Serie::into_unique`] - with
    /// no join: each chunk's sorted order computed once, as
    /// [`Serie::sort_indices`] computes it, ascending with absent values
    /// last; the chunks merged in that order as [`Self::into_sorted`] merges
    /// them, its rung over the whole row; and the first row of every run of
    /// equal values marked in its chunk's mask - the first in chunk order,
    /// then in row order, because a tie keeps that order. Each chunk is then
    /// filtered by its own mask and kept apart, and a chunk left with no row
    /// is dropped. An absent row is one value. One chunk holding rows is its
    /// own [`Serie::into_unique`].
    ///
    /// The cost is the sort's merge, its cursors reading each chunk in
    /// sorted order a block at a time, taken out of the chunk rather than
    /// held sorted beside it, plus every chunk's order (four bytes a row)
    /// and mask (one byte a row), held until the filters run - and no
    /// output batch is gathered.
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
    /// let second: ArrayRef = Arc::new(StringArray::from(vec!["XNYS", "XPAR"]));
    /// let venues = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    /// let unique = venues.into_unique()?;
    /// assert_eq!(unique.rows(), vec![Scalar::from("XNYS"), Scalar::from("XNAS"), Scalar::from("XPAR")]);
    /// // Kept apart: each chunk keeps its own first occurrences.
    /// assert_eq!(unique.num_chunks(), 2);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::sort_indices`]'s refusal for a chunk, and the filter
    /// kernel's refusal of a layout, naming the chunk's field.
    pub fn into_unique(&self) -> crate::Result<Self> {
        let mut live: Vec<&Serie> = Vec::with_capacity(self.chunks.len());
        live.extend(self.chunks.iter().filter(|chunk| !chunk.is_empty()));
        if let [one] = live.as_slice() {
            return Ok(Self::from_landed(
                Arc::clone(&self.field),
                vec![one.into_unique()?],
            ));
        }
        let options = SortOptions::default();
        let mut orders: Vec<Vec<u32>> = Vec::with_capacity(live.len());
        for chunk in &live {
            orders.push(chunk.sorted_order(options)?);
        }
        let mut masks: Vec<Vec<bool>> = live.iter().map(|chunk| vec![false; chunk.len()]).collect();
        let lens: Vec<usize> = live.iter().map(|chunk| chunk.len()).collect();
        let mut merge = Merge::new(&lens, vec![options], true, |source, start, len| {
            Ok(vec![
                live[source].taken(&orders[source][start..start + len])?,
            ])
        })?;
        while let Some((source, at, opens)) = merge.next()? {
            if opens {
                masks[source][orders[source][at] as usize] = true;
            }
        }
        drop(merge);
        let mut unique = Self::with_chunk_capacity(Arc::clone(&self.field), live.len());
        for (chunk, mask) in live.into_iter().zip(&masks) {
            let kept = chunk.filtered(mask)?;
            if !kept.is_empty() {
                unique.push_landed(kept)?;
            }
        }
        Ok(unique)
    }

    /// Every chunk holding a row, sorted on its own by `sort`, held as one
    /// chunked serie under this field and settled as each lands: the
    /// heaviest spilled first once the sorted chunks pass the process
    /// bound.
    fn sorted_chunks(&self, sort: impl Fn(&Serie) -> crate::Result<Serie>) -> crate::Result<Self> {
        let mut sorted = Self::with_chunk_capacity(Arc::clone(&self.field), self.chunks.len());
        for chunk in self.chunks.iter().filter(|chunk| !chunk.is_empty()) {
            sorted.push_landed(sort(chunk)?)?;
        }
        Ok(sorted)
    }

    /// [`Self::sorted_chunks`] on up to `threads` threads: the chunks
    /// holding a row sorted `threads` at a time, each window's sorted chunks
    /// landed in chunk order and settled before the next window is sorted,
    /// so what is held beyond the settled chunks is at most one window.
    fn sorted_chunks_on(
        &self,
        threads: usize,
        sort: impl Fn(&Serie) -> crate::Result<Serie> + Sync,
    ) -> crate::Result<Self> {
        let chunks: Vec<&Serie> = self
            .chunks
            .iter()
            .filter(|chunk| !chunk.is_empty())
            .collect();
        let threads = threads.min(chunks.len()).max(1);
        if threads <= 1 {
            return self.sorted_chunks(sort);
        }
        let mut sorted = Self::with_chunk_capacity(Arc::clone(&self.field), chunks.len());
        for window in chunks.chunks(threads) {
            let answers: Vec<crate::Result<Serie>> = std::thread::scope(|scope| {
                let sorting: Vec<_> = window
                    .iter()
                    .map(|chunk| scope.spawn(|| sort(chunk)))
                    .collect();
                sorting
                    .into_iter()
                    .map(|sorting| {
                        sorting.join().unwrap_or_else(|_| {
                            Err(crate::Error::InvalidRecord {
                                path: smol_str::SmolStr::new_static("$"),
                                reason: smol_str::SmolStr::new_static(
                                    "expected every chunk sort to finish, got one that panicked",
                                ),
                            })
                        })
                    })
                    .collect()
            });
            for answer in answers {
                sorted.push_landed(answer?)?;
            }
        }
        Ok(sorted)
    }

    /// The `order by` keys of `by` bound once against this field, refused
    /// exactly as [`Serie::sort_indices_by`] refuses them, before any chunk
    /// is read.
    fn sort_key(&self, by: &[expression::Ordering]) -> crate::Result<BoundSelector> {
        if by.is_empty() {
            // The column's own refusal of no key, word for word.
            Serie::empty(Arc::clone(&self.field))?.sorted_order_by(by)?;
        }
        Selector::new(by.iter().map(|key| Projection::new(key.term().clone()))).bind_key(
            &StreamChunkedSerie::root_of(&self.field)?,
            self.field.name(),
            "sort by",
        )
    }

    /// These chunks, each already sorted under `options`, merged into one
    /// sorted chunked serie of chunks of at most
    /// [`DEFAULT_RECORD_BATCH_ROW_SIZE`] rows: `keys` answers the key cells
    /// of rows `start..start + len` of a chunk, one per option. One chunk
    /// holding rows is cut into zero-copy slices of itself, merging
    /// nothing; several are merged by [`Merge`], each output batch's rows
    /// gathered by one `interleave` over the chunks and landed as rows the
    /// chunks already proved, then settled.
    fn merged(
        &self,
        options: &[SortOptions],
        mut keys: impl FnMut(&Serie, usize, usize) -> crate::Result<Vec<Serie>>,
    ) -> crate::Result<Self> {
        let batch = DEFAULT_RECORD_BATCH_ROW_SIZE;
        let mut merged =
            Self::with_chunk_capacity(Arc::clone(&self.field), self.len().div_ceil(batch));
        if let [one] = self.chunks.as_slice() {
            let len = one.len();
            if len <= batch {
                merged.push_landed(one.clone())?;
            } else {
                for start in (0..len).step_by(batch) {
                    merged.push_landed(one.slice(start, batch.min(len - start))?)?;
                }
            }
            return Ok(merged);
        }
        if self.chunks.is_empty() {
            return Ok(merged);
        }
        let arrays = self.into_arrow_arrays();
        require_joinable(&self.field, &arrays)?;
        let arrays: Vec<&dyn Array> = arrays.iter().map(AsRef::as_ref).collect();
        let lens: Vec<usize> = self.chunks.iter().map(Serie::len).collect();
        let mut merge = Merge::new(&lens, options.to_vec(), false, |source, start, len| {
            keys(&self.chunks[source], start, len)
        })?;
        let mut positions: Vec<(usize, usize)> = Vec::with_capacity(batch.min(self.len()));
        while let Some((source, at, _)) = merge.next()? {
            positions.push((source, at));
            if positions.len() == batch {
                merged.push_landed(self.gathered(&arrays, &positions)?)?;
                positions.clear();
            }
        }
        if !positions.is_empty() {
            merged.push_landed(self.gathered(&arrays, &positions)?)?;
        }
        Ok(merged)
    }

    /// The rows `positions` names - a chunk and a row of it each - gathered
    /// out of `arrays`, the chunks' own, by one `interleave`, and landed
    /// under the field as rows the chunks already proved, then settled.
    fn gathered(
        &self,
        arrays: &[&dyn Array],
        positions: &[(usize, usize)],
    ) -> crate::Result<Serie> {
        let array = arrow_select::interleave::interleave(arrays, positions)?;
        if array.len() != positions.len() {
            // Arrow answers a zero-width fixed-size serie by its child,
            // which has no rows to count: the rows are read, as a take
            // reads them.
            return Serie::from_scalars(
                Arc::clone(&self.field),
                positions
                    .iter()
                    .map(|(chunk, row)| proven_row(&self.chunks[*chunk], *row)),
            );
        }
        land(Arc::clone(&self.field), array, &Proof::Proven)?.settled()
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
        let chunks: Vec<Serie> = self.chunks.iter().rev().map(Serie::into_reversed).collect();
        let field = Self::field_of(&chunks, || self.reversed_field());
        Self::from_landed(field, chunks)
    }

    /// This field with the order it declares turned around, as every
    /// reversed chunk states it: the empty column of the field reversed is
    /// the one door that writes the flipped declaration.
    fn reversed_field(&self) -> Arc<Field> {
        Serie::empty(Arc::clone(&self.field))
            .ok()
            .and_then(|empty| empty.into_reversed().field_ref().cloned())
            .unwrap_or_else(|| Arc::clone(&self.field))
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
        // The take keeps or clears the declared order; the field follows it.
        let field = Self::field_of(std::slice::from_ref(&taken), || Arc::clone(&self.field));
        Ok(Self::from_landed(field, vec![taken]))
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

    /// The bytes the rows occupy in memory: every chunk's
    /// [`Serie::resident_size`], summed - kept beside the chunks, so the
    /// answer costs no walk.
    pub fn resident_size(&self) -> usize {
        self.resident
    }

    /// Whether every chunk's rows lie in a spill file: no byte resident, and
    /// some bytes - read in that order, so a resident chunk answers off its
    /// flags. A chunked serie of no chunk is never spilled.
    pub fn is_spilled(&self) -> bool {
        self.resident_size() == 0 && self.memory_size() > 0
    }

    /// Move chunks to disk until the resident bytes are under `options`'
    /// bound: the heaviest chunks spill whole first, each through
    /// [`Serie::spill`], and the sum is read again after each, so a chunked
    /// serie under the bound is untouched and one over it keeps its lightest
    /// chunks resident.
    ///
    /// ```
    /// use yggdryl::{ChunkedSerie, DataType, Scalar, Serie, SpillOptions};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = DataType::Int64.required_field("price");
    /// let heavy = Serie::from_scalars(field.clone(), (0..1_024_i64).map(Scalar::from))?;
    /// let light = Serie::from_scalars(field.clone(), (0..8_i64).map(Scalar::from))?;
    /// let mut prices = ChunkedSerie::from_series(Some(&field), [heavy, light], Default::default())?;
    /// prices.spill(&SpillOptions::new().with_byte_size(1_024))?;
    /// assert!(prices.chunk(0).expect("the heavy chunk").is_spilled());
    /// assert!(!prices.chunk(1).expect("the light chunk").is_spilled());
    /// assert_eq!(prices.resident_size(), 64);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Serie::spill`]'s refusal of its folder, leaving the chunks spilled
    /// so far mapped and the rest as they were.
    pub fn spill(&mut self, options: &SpillOptions) -> crate::Result<()> {
        if options.is_never() {
            return Ok(());
        }
        self.held = OnceLock::new();
        let bound = options.byte_size();
        let resident = |chunks: &[Serie]| {
            chunks
                .iter()
                .map(|chunk| u64::try_from(chunk.resident_size()).unwrap_or(u64::MAX))
                .fold(0_u64, u64::saturating_add)
        };
        if resident(&self.chunks) <= bound {
            return Ok(());
        }
        let mut order: Vec<(usize, usize)> = self
            .chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| (index, chunk.resident_size()))
            .collect();
        order.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
        let whole = options.clone().with_byte_size(0);
        let outcome = (|| {
            for (index, _) in order {
                if resident(&self.chunks) <= bound {
                    break;
                }
                self.chunks[index].spill(&whole)?;
            }
            Ok(())
        })();
        self.resident = self.chunks.iter().map(Serie::resident_size).sum();
        outcome
    }

    /// [`Self::spill`], answering this serie so calls chain.
    ///
    /// # Errors
    ///
    /// [`Self::spill`]'s.
    pub fn as_spilled(&mut self, options: &SpillOptions) -> crate::Result<&mut Self> {
        self.spill(options)?;
        Ok(self)
    }

    /// A copy of this chunked serie spilled under `options`' bound, this
    /// one untouched: the chunks the bound leaves resident are shared, the
    /// rest written once and mapped.
    ///
    /// # Errors
    ///
    /// [`Self::spill`]'s.
    pub fn into_spilled(&self, options: &SpillOptions) -> crate::Result<Self> {
        let mut spilled = self.clone();
        spilled.spill(options)?;
        Ok(spilled)
    }

    /// Sort the rows in place under `options`, answering this serie so
    /// calls chain: [`Self::into_sorted`]'s merged chunks replacing the
    /// chunks.
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

    /// Sort the rows in place in the order the `order by` keys of `by`
    /// state, answering this serie so calls chain: [`Self::into_sort_by`]'s
    /// merged chunks replacing the chunks.
    ///
    /// # Errors
    ///
    /// [`Self::into_sort_by`]'s refusals, which leave this serie as it was.
    pub fn as_sort_by(&mut self, by: impl IntoOrderings) -> crate::Result<&mut Self> {
        *self = self.into_sort_by(by)?;
        Ok(self)
    }

    /// Keep the first occurrence of every value, in place, answering this
    /// serie: [`Self::into_unique`]'s filtered chunks replacing the chunks.
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
        let field = Self::field_of(&self.chunks, || self.reversed_field());
        *self = Self::from_landed(field, std::mem::take(&mut self.chunks));
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

/// The fewest rows a merge cursor reads of its chunk at once, however many
/// chunks share one output batch's worth: a block amortizes its key's
/// computation and its conversion over this many rows at least.
const MERGE_BLOCK_ROWS: usize = 1_024;

/// How a [`Merge`] orders two cursors' keys: one rung for the whole merge.
enum Rung {
    /// Arrow's row format: each block's key cells encoded once by the one
    /// converter, under each key's options, and compared as bytes.
    Rows(RowConverter),
    /// The values' own order: each cursor's current key cells built as it
    /// advances, compared cell by cell under each key's options.
    Values,
}

impl Rung {
    /// The row format where every key cell's stored order is its value
    /// order, no float lies beneath it - a float may hold a NaN other than
    /// the one its values read, which the format orders by its bits - and
    /// the format has a layout for it; the values' order otherwise.
    fn of(cells: &[Serie], options: &[SortOptions]) -> crate::Result<Self> {
        let float = |node: &ArrowDataType| {
            matches!(
                node,
                ArrowDataType::Float16 | ArrowDataType::Float32 | ArrowDataType::Float64
            )
        };
        let mut fields = Vec::with_capacity(cells.len());
        for (cell, options) in cells.iter().zip(options) {
            let (Some(field), Some(array)) = (cell.field(), cell.into_arrow_array()) else {
                return Ok(Self::Values);
            };
            if !stored_order_is_value_order(field.dtype())
                || storage_holds(array.data_type(), &float)
            {
                return Ok(Self::Values);
            }
            fields.push(SortField::new_with_options(
                array.data_type().clone(),
                options.into_arrow(),
            ));
        }
        if !RowConverter::supports_fields(&fields) {
            return Ok(Self::Values);
        }
        Ok(Self::Rows(RowConverter::new(fields)?))
    }
}

/// One sorted chunk's place in a [`Merge`]: the row it stands on and the
/// block of key cells that row lies in.
struct Cursor {
    /// The chunk, by its place among the merged chunks.
    source: usize,
    /// The chunk's row count.
    len: usize,
    /// The row the cursor stands on.
    at: usize,
    /// The rows `start..end` the loaded block covers.
    start: usize,
    end: usize,
    /// The block's key cells, one column per key.
    cells: Vec<Serie>,
    /// On the row-format rung, the block's key rows, the buffer reused
    /// block after block.
    rows: Option<arrow_row::Rows>,
    /// On the values rung, the current row's key cells, the vector reused
    /// from the first row on.
    key: Vec<Scalar>,
}

impl Cursor {
    /// Read the loaded block on `rung`: its key rows encoded, or the
    /// current row's key built.
    fn read(&mut self, rung: &Rung) -> crate::Result<()> {
        match rung {
            Rung::Rows(converter) => {
                let arrays: Vec<ArrayRef> = self
                    .cells
                    .iter()
                    .map(|cell| cell.into_arrow_array().expect(CHUNK))
                    .collect();
                let capacity = self.end - self.start;
                let rows = self
                    .rows
                    .get_or_insert_with(|| converter.empty_rows(capacity, 0));
                rows.clear();
                converter.append(rows, &arrays)?;
            }
            Rung::Values => self.build_key(),
        }
        Ok(())
    }

    /// The current row's key cells, built into the reused vector.
    fn build_key(&mut self) {
        let row = self.at - self.start;
        self.key.clear();
        self.key
            .extend(self.cells.iter().map(|cell| proven_row(cell, row)));
    }

    /// The current row's key bytes, on the row-format rung.
    fn row(&self) -> arrow_row::Row<'_> {
        self.rows
            .as_ref()
            .expect("a row-format cursor holds its block's rows")
            .row(self.at - self.start)
    }
}

/// A k-way merge of sorted chunks: one [`Cursor`] per chunk holding a row,
/// in a binary heap of their places with the least key on top and a tie
/// going to the earlier chunk, so equal keys come out in chunk order, then
/// in each chunk's own - stable, as one sort of the joined rows is.
///
/// `load` answers the key cells of rows `start..start + len` of a chunk in
/// its sorted order, one column per option; a cursor loads a block of
/// [`DEFAULT_RECORD_BATCH_ROW_SIZE`] rows shared among the chunks, never
/// fewer than [`MERGE_BLOCK_ROWS`], so the key rows held at once are one
/// output batch's worth until the chunks outnumber what that allows. The
/// rung is read off the first block and holds for the merge. The row-format
/// rung allocates per block, never per row; the values rung builds each
/// key once as its cursor reaches it.
struct Merge<L> {
    load: L,
    options: Vec<SortOptions>,
    /// The rows a cursor loads at once.
    span: usize,
    rung: Rung,
    cursors: Vec<Cursor>,
    /// The places of the cursors still standing on a row, as a binary heap.
    heap: Vec<usize>,
    /// Whether [`Self::next`] says where a run of equal keys opens.
    distinct: bool,
    /// The key last yielded, where `distinct`: its bytes on the row-format
    /// rung, its cells on the values rung, each buffer reused.
    last_row: Vec<u8>,
    last_key: Vec<Scalar>,
    /// Whether a row was yielded yet.
    yielded: bool,
}

impl<L: FnMut(usize, usize, usize) -> crate::Result<Vec<Serie>>> Merge<L> {
    /// The merge of chunks of `lens` rows each, ordered under `options`;
    /// `distinct` asks every yielded row whether it opens a run of equal
    /// keys. Each chunk holding a row loads its first block here.
    fn new(
        lens: &[usize],
        options: Vec<SortOptions>,
        distinct: bool,
        mut load: L,
    ) -> crate::Result<Self> {
        let live = lens.iter().filter(|len| **len > 0).count();
        let span = (DEFAULT_RECORD_BATCH_ROW_SIZE / live.max(1)).max(MERGE_BLOCK_ROWS);
        let mut cursors = Vec::with_capacity(live);
        for (source, &len) in lens.iter().enumerate().filter(|(_, len)| **len > 0) {
            let end = span.min(len);
            cursors.push(Cursor {
                source,
                len,
                at: 0,
                start: 0,
                end,
                cells: load(source, 0, end)?,
                rows: None,
                key: Vec::new(),
            });
        }
        let rung = match cursors.first() {
            Some(first) => Rung::of(&first.cells, &options)?,
            None => Rung::Values,
        };
        for cursor in &mut cursors {
            cursor.read(&rung)?;
        }
        let mut merge = Self {
            load,
            options,
            span,
            rung,
            heap: (0..cursors.len()).collect(),
            cursors,
            distinct,
            last_row: Vec::new(),
            last_key: Vec::new(),
            yielded: false,
        };
        for place in (0..merge.heap.len() / 2).rev() {
            merge.sift_down(place);
        }
        Ok(merge)
    }

    /// The next row in merged order - its chunk, its row there, and, where
    /// `distinct`, whether its key differs from the row yielded before it -
    /// or `None` once every chunk is read.
    fn next(&mut self) -> crate::Result<Option<(usize, usize, bool)>> {
        let Some(&top) = self.heap.first() else {
            return Ok(None);
        };
        let (source, at) = (self.cursors[top].source, self.cursors[top].at);
        let opens = self.distinct && self.opens(top);
        if self.advance(top)? {
            self.sift_down(0);
        } else {
            self.heap.swap_remove(0);
            if !self.heap.is_empty() {
                self.sift_down(0);
            }
        }
        Ok(Some((source, at, opens)))
    }

    /// Whether cursor `place`'s key differs from the key last yielded,
    /// keeping it as the last where it does.
    fn opens(&mut self, place: usize) -> bool {
        let cursor = &self.cursors[place];
        let opens = match self.rung {
            Rung::Rows(_) => {
                let row = cursor.row().data();
                let opens = !self.yielded || row != self.last_row.as_slice();
                if opens {
                    self.last_row.clear();
                    self.last_row.extend_from_slice(row);
                }
                opens
            }
            Rung::Values => {
                let opens = !self.yielded
                    || cursor
                        .key
                        .iter()
                        .zip(&self.last_key)
                        .zip(&self.options)
                        .any(|((key, last), options)| {
                            compare_values(key, last, *options) != Ordering::Equal
                        });
                if opens {
                    self.last_key.clone_from(&cursor.key);
                }
                opens
            }
        };
        self.yielded = true;
        opens
    }

    /// Move cursor `place` to its next row, loading the next block where it
    /// leaves its own; `false` once its chunk is read.
    fn advance(&mut self, place: usize) -> crate::Result<bool> {
        let cursor = &mut self.cursors[place];
        cursor.at += 1;
        if cursor.at == cursor.len {
            cursor.cells.clear();
            cursor.rows = None;
            return Ok(false);
        }
        if cursor.at == cursor.end {
            cursor.start = cursor.at;
            cursor.end = cursor.len.min(cursor.at + self.span);
            cursor.cells = (self.load)(cursor.source, cursor.start, cursor.end - cursor.start)?;
            cursor.read(&self.rung)?;
        } else if matches!(self.rung, Rung::Values) {
            cursor.build_key();
        }
        Ok(true)
    }

    /// Cursor `left`'s key against cursor `right`'s, a tie going to the
    /// earlier chunk.
    fn compare(&self, left: usize, right: usize) -> Ordering {
        let (left, right) = (&self.cursors[left], &self.cursors[right]);
        let step = match self.rung {
            Rung::Rows(_) => left.row().cmp(&right.row()),
            Rung::Values => left
                .key
                .iter()
                .zip(&right.key)
                .zip(&self.options)
                .map(|((left, right), options)| compare_values(left, right, *options))
                .find(|step| *step != Ordering::Equal)
                .unwrap_or(Ordering::Equal),
        };
        step.then(left.source.cmp(&right.source))
    }

    /// Restore the heap below `place`, whose cursor may have moved.
    fn sift_down(&mut self, mut place: usize) {
        let len = self.heap.len();
        loop {
            let left = 2 * place + 1;
            if left >= len {
                return;
            }
            let right = left + 1;
            let least = if right < len
                && self.compare(self.heap[right], self.heap[left]) == Ordering::Less
            {
                right
            } else {
                left
            };
            if self.compare(self.heap[least], self.heap[place]) != Ordering::Less {
                return;
            }
            self.heap.swap(place, least);
            place = least;
        }
    }
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
