//! [`StructSerie`] **as a DataFrame** — a table of named, typed columns. A struct
//! column's children *are* the frame's columns, so this module adds the table surface on
//! top of the [`StructSerie`](crate::StructSerie) / [`Serie`] / [`NestedSerie`] base:
//! [shape](StructSerie::shape) and [column names](StructSerie::column_names), projection
//! ([`select_columns`](StructSerie::select_columns)), column add / drop / rename
//! ([`with_column`](StructSerie::with_column) / [`drop_columns`](StructSerie::drop_columns)
//! / [`rename`](StructSerie::rename)), row selection ([`head`](StructSerie::head) /
//! [`tail`](StructSerie::tail) / [`slice_rows`](StructSerie::slice_rows) /
//! [`filter`](StructSerie::filter)), row stacking ([`vstack`](StructSerie::vstack)) and a
//! lossless Arrow [`RecordBatch`] round-trip
//! ([`to_record_batch`](StructSerie::to_record_batch) /
//! [`from_record_batch`](StructSerie::from_record_batch)). A frame renders as an aligned
//! table through the one [`display`](Serie::display) method (there is no separate `show`).
//!
//! Columns are accessed with the [`NestedSerie`] vocabulary the frame inherits —
//! [`children`](StructSerie::children), [`child`](NestedSerie::child) (by index) and
//! [`child_by_name`](NestedSerie::child_by_name) (by name) — so the frame adds the
//! *table* operations without restating column access.
//!
//! The richer surface adds schema-cast projection ([`select_fields`](StructSerie::select_fields)
//! — cast + reorder + fill + drop), [sorting](StructSerie::sort_by), a
//! [row index](StructSerie::with_row_index), per-row record access
//! ([`row`](StructSerie::row) → a [`StructScalar`](yggdryl_scalar::StructScalar)) and
//! chunked Arrow table / [reader](StructSerie::to_record_batch_reader) conversions.
//!
//! Every transform is **functional**: it returns a new lazy `StructSerie` that shares the
//! untouched columns' Arrow buffers (no copy), assembling the backing `StructArray` only
//! on demand.

use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray, RecordBatch, RecordBatchIterator, RecordBatchReader};
use arrow_schema::{Field as AField, Schema};
use yggdryl_scalar::{ScalarRef, ScalarValue, StructScalar};
use yggdryl_schema::{DataType, Field};

use crate::error::{SerieError, SerieResult};
use crate::nested::{NestedSerie, StructSerie};
use crate::serie::{dispatch, Serie, SerieRef};
use crate::UInt64RangeSerie;

/// Maps an Arrow error to a [`SerieError`].
fn arrow_err(e: arrow_schema::ArrowError) -> SerieError {
    SerieError::Arrow(e.to_string())
}

impl StructSerie {
    /// The frame shape as `(rows, columns)`.
    ///
    /// ```
    /// use yggdryl_serie::{Int32Serie, Serie, SerieRef, StructSerie};
    /// use std::sync::Arc;
    /// let a: SerieRef = Arc::new(Int32Serie::from_values("a", vec![Some(1), Some(2)]));
    /// let b: SerieRef = Arc::new(Int32Serie::from_values("b", vec![Some(3), Some(4)]));
    /// let frame = StructSerie::from_children("df", vec![a, b]).unwrap();
    /// assert_eq!(frame.shape(), (2, 2));
    /// ```
    pub fn shape(&self) -> (usize, usize) {
        (self.len(), self.children().len())
    }

    /// The number of columns.
    pub fn num_columns(&self) -> usize {
        self.children().len()
    }

    /// The column names, in order.
    pub fn column_names(&self) -> Vec<&str> {
        self.children().iter().map(|c| c.name()).collect()
    }

    /// Projects the frame to the named columns, in the requested order. Errors if a name
    /// is missing. The result is a new lazy frame sharing the selected columns.
    pub fn select_columns(&self, names: &[&str]) -> SerieResult<StructSerie> {
        let mut cols = Vec::with_capacity(names.len());
        for name in names {
            let col = self
                .child_by_name(name)
                .ok_or_else(|| SerieError::Arrow(format!("no column named '{name}' to select")))?;
            cols.push(col);
        }
        StructSerie::from_children(self.name(), cols)
    }

    /// A new frame with `column` appended, or replacing an existing column of the same
    /// name. The column's length must match the frame's (unless the frame is empty).
    pub fn with_column(&self, column: SerieRef) -> SerieResult<StructSerie> {
        if !self.children().is_empty() && column.len() != self.len() {
            return Err(SerieError::Arrow(format!(
                "column '{}' has length {} but the frame has {} rows",
                column.name(),
                column.len(),
                self.len()
            )));
        }
        let mut cols: Vec<SerieRef> = self.children().to_vec();
        match cols.iter().position(|c| c.name() == column.name()) {
            Some(index) => cols[index] = column,
            None => cols.push(column),
        }
        StructSerie::from_children(self.name(), cols)
    }

    /// A new frame without the named columns (names that are absent are ignored).
    pub fn drop_columns(&self, names: &[&str]) -> SerieResult<StructSerie> {
        let cols: Vec<SerieRef> = self
            .children()
            .iter()
            .filter(|c| !names.contains(&c.name()))
            .cloned()
            .collect();
        StructSerie::from_children(self.name(), cols)
    }

    /// A new frame with column `old` renamed to `new` (a no-op if `old` is absent).
    /// Renaming re-fields the column, which realises it if it was lazy.
    pub fn rename(&self, old: &str, new: &str) -> SerieResult<StructSerie> {
        let cols: Vec<SerieRef> = self
            .children()
            .iter()
            .map(|c| {
                if c.name() == old {
                    let field = c.field().copy(Some(new.to_string()), None, None, None);
                    // `dispatch` (not `from_arrow`) so an Arrow-normalised column — e.g. a
                    // map with `keys`/`values` entry names — is not wrongly rejected.
                    dispatch(field, c.array()).unwrap_or_else(|_| c.clone())
                } else {
                    c.clone()
                }
            })
            .collect();
        StructSerie::from_children(self.name(), cols)
    }

    /// A zero-copy row slice of `length` rows starting at `offset`, as a new frame.
    pub fn slice_rows(&self, offset: usize, length: usize) -> SerieResult<StructSerie> {
        let cols: Vec<SerieRef> = self
            .children()
            .iter()
            .map(|c| c.slice(offset, length))
            .collect();
        StructSerie::from_children(self.name(), cols)
    }

    /// The first `n` rows, as a new frame.
    pub fn head(&self, n: usize) -> SerieResult<StructSerie> {
        self.slice_rows(0, n.min(self.len()))
    }

    /// The last `n` rows, as a new frame.
    pub fn tail(&self, n: usize) -> SerieResult<StructSerie> {
        let len = self.len();
        let n = n.min(len);
        self.slice_rows(len - n, n)
    }

    /// Keeps the rows where `mask` is `true` (the mask length must equal the row count),
    /// as a new frame — the row-filter every column store needs.
    pub fn filter(&self, mask: &[bool]) -> SerieResult<StructSerie> {
        if mask.len() != self.len() {
            return Err(SerieError::Arrow(format!(
                "filter mask length {} does not match the frame's {} rows",
                mask.len(),
                self.len()
            )));
        }
        let predicate = BooleanArray::from(mask.to_vec());
        let cols = self
            .children()
            .iter()
            .map(|c| {
                let kept = arrow_select::filter::filter(c.array().as_ref(), &predicate)
                    .map_err(arrow_err)?;
                dispatch(c.field().clone(), kept)
            })
            .collect::<SerieResult<Vec<SerieRef>>>()?;
        StructSerie::from_children(self.name(), cols)
    }

    /// Stacks `other`'s rows below this frame's, as a new frame. The two frames must have
    /// the same column names and types (each column pair is concatenated).
    pub fn vstack(&self, other: &StructSerie) -> SerieResult<StructSerie> {
        if self.column_names() != other.column_names() {
            return Err(SerieError::Arrow(
                "vstack requires both frames to have the same column names in the same order"
                    .into(),
            ));
        }
        let cols = self
            .children()
            .iter()
            .zip(other.children())
            .map(|(a, b)| {
                let combined =
                    arrow_select::concat::concat(&[a.array().as_ref(), b.array().as_ref()])
                        .map_err(arrow_err)?;
                dispatch(a.field().clone(), combined)
            })
            .collect::<SerieResult<Vec<SerieRef>>>()?;
        StructSerie::from_children(self.name(), cols)
    }

    /// The frame's columns as an Arrow [`RecordBatch`] (one Arrow field + array per
    /// column). A struct-level null mask, if any, is not represented (a `RecordBatch` has
    /// no row validity); use [`array`](Serie::array) for the nullable `StructArray`.
    pub fn to_record_batch(&self) -> SerieResult<RecordBatch> {
        let fields = self
            .children()
            .iter()
            .map(|c| c.field().to_arrow().map(Arc::new))
            .collect::<Result<Vec<Arc<AField>>, _>>()?;
        let columns: Vec<ArrayRef> = self.children().iter().map(|c| c.array()).collect();
        let schema = Arc::new(Schema::new(fields));
        RecordBatch::try_new(schema, columns).map_err(|e| SerieError::Arrow(e.to_string()))
    }

    /// Builds a frame named `name` from an Arrow [`RecordBatch`] — each batch column
    /// becomes a frame column (built recursively, so nested columns resolve too).
    pub fn from_record_batch(
        name: impl Into<String>,
        batch: &RecordBatch,
    ) -> SerieResult<StructSerie> {
        let children = batch
            .schema()
            .fields()
            .iter()
            .zip(batch.columns())
            .map(|(field, column)| dispatch(Field::from_arrow(field), column.clone()))
            .collect::<SerieResult<Vec<SerieRef>>>()?;
        StructSerie::from_children(name, children)
    }
}

/// Schema-cast projection, sorting, row records, a row index, and chunked Arrow
/// `RecordBatch` / reader conversions — the richer DataFrame surface.
impl StructSerie {
    /// Projects and **casts** the frame to an explicit list of target [`Field`]s: each
    /// target field takes the source column of the same name **cast to its type** (or, if
    /// absent, an optimized **fill** — nulls when nullable, else the type default), in the
    /// target order, dropping unlisted columns. The schema-cast companion to
    /// [`select_columns`](StructSerie::select_columns) (which only reorders/projects),
    /// powered by the same `Serie::cast` struct kernel.
    pub fn select_fields(&self, fields: Vec<Field>) -> SerieResult<StructSerie> {
        let casted = self.cast(&DataType::struct_(fields))?;
        casted
            .as_any()
            .downcast_ref::<StructSerie>()
            .cloned()
            .ok_or_else(|| SerieError::Arrow("schema cast did not yield a struct frame".into()))
    }

    /// A new frame with the rows sorted by column `column` (ascending unless
    /// `descending`), reordering every column by the sort permutation (Arrow's
    /// `sort_to_indices` + `take`). Nulls sort first ascending, last descending.
    pub fn sort_by(&self, column: &str, descending: bool) -> SerieResult<StructSerie> {
        let key = self
            .child_by_name(column)
            .ok_or_else(|| SerieError::Arrow(format!("no column named '{column}' to sort by")))?;
        let options = arrow_ord::sort::SortOptions {
            descending,
            nulls_first: !descending,
        };
        let indices = arrow_ord::sort::sort_to_indices(key.array().as_ref(), Some(options), None)
            .map_err(arrow_err)?;
        let cols = self
            .children()
            .iter()
            .map(|c| {
                let taken = arrow_select::take::take(c.array().as_ref(), &indices, None)
                    .map_err(arrow_err)?;
                dispatch(c.field().clone(), taken)
            })
            .collect::<SerieResult<Vec<SerieRef>>>()?;
        StructSerie::from_children(self.name(), cols)
    }

    /// A new frame with a `0..rows` integer index column named `name` prepended (a lazy
    /// `uint64` [`RangeSerie`](crate::RangeSerie), so it costs nothing until materialised).
    pub fn with_row_index(&self, name: &str) -> SerieResult<StructSerie> {
        let index: SerieRef = Arc::new(UInt64RangeSerie::uint64(name, 0, 1, self.len()));
        let mut cols = Vec::with_capacity(self.children().len() + 1);
        cols.push(index);
        cols.extend(self.children().iter().cloned());
        StructSerie::from_children(self.name(), cols)
    }

    /// The record at `index` as a [`StructScalar`] — one typed [`ScalarRef`] per column,
    /// the per-row accessor for a frame. Reads each column's cell as a rich scalar; an
    /// out-of-bounds index yields a struct of typed nulls.
    pub fn row(&self, index: usize) -> SerieResult<StructScalar> {
        let fields: Vec<Field> = self.children().iter().map(|c| c.field().clone()).collect();
        let values = self
            .children()
            .iter()
            .map(|c| {
                ScalarValue::from_array(c.array().as_ref(), index)
                    .map(ScalarValue::into_scalar)
                    .map_err(|e| SerieError::Arrow(e.to_string()))
            })
            .collect::<SerieResult<Vec<ScalarRef>>>()?;
        Ok(StructScalar::from_children(fields, values))
    }

    /// The frame split into Arrow [`RecordBatch`]es of at most `max_rows` rows each (a
    /// "table" of chunks). An empty frame yields a single empty batch (preserving the
    /// schema).
    pub fn to_record_batches(&self, max_rows: usize) -> SerieResult<Vec<RecordBatch>> {
        if max_rows == 0 {
            return Err(SerieError::Arrow("max_rows must be greater than 0".into()));
        }
        let rows = self.len();
        if rows == 0 {
            return Ok(vec![self.to_record_batch()?]);
        }
        let mut batches = Vec::with_capacity(rows.div_ceil(max_rows));
        let mut offset = 0;
        while offset < rows {
            let len = max_rows.min(rows - offset);
            batches.push(self.slice_rows(offset, len)?.to_record_batch()?);
            offset += len;
        }
        Ok(batches)
    }

    /// Builds a frame named `name` by concatenating a sequence of Arrow
    /// [`RecordBatch`]es (they must share a schema) — the inverse of
    /// [`to_record_batches`](StructSerie::to_record_batches).
    pub fn from_record_batches(
        name: impl Into<String>,
        batches: &[RecordBatch],
    ) -> SerieResult<StructSerie> {
        let first = batches
            .first()
            .ok_or_else(|| SerieError::Arrow("no record batches to read".into()))?;
        let combined =
            arrow_select::concat::concat_batches(&first.schema(), batches).map_err(arrow_err)?;
        StructSerie::from_record_batch(name, &combined)
    }

    /// The frame as an Arrow [`RecordBatchReader`] — a streaming scanner over chunks of at
    /// most `max_rows` rows, the shape Arrow/Parquet readers and downstream scanners
    /// consume.
    pub fn to_record_batch_reader(
        &self,
        max_rows: usize,
    ) -> SerieResult<Box<dyn RecordBatchReader + Send>> {
        let batches = self.to_record_batches(max_rows)?;
        let schema = batches
            .first()
            .map(|b| b.schema())
            .unwrap_or_else(|| Arc::new(Schema::empty()));
        let reader = RecordBatchIterator::new(batches.into_iter().map(Ok), schema);
        Ok(Box::new(reader))
    }

    /// Builds a frame named `name` by draining an Arrow [`RecordBatchReader`] (a scanner /
    /// Parquet reader) — the inverse of
    /// [`to_record_batch_reader`](StructSerie::to_record_batch_reader).
    pub fn from_record_batch_reader(
        name: impl Into<String>,
        reader: impl RecordBatchReader,
    ) -> SerieResult<StructSerie> {
        let batches = reader.collect::<Result<Vec<_>, _>>().map_err(arrow_err)?;
        StructSerie::from_record_batches(name, &batches)
    }

    /// The frame as an **Arrow IPC stream** of its [`RecordBatch`] (columns as top-level
    /// fields) — bytes any Arrow library reads back as a multi-column table (e.g.
    /// `pyarrow.ipc.open_stream(bytes).read_all()`). The cross-language table interchange.
    pub fn to_ipc_bytes(&self) -> SerieResult<Vec<u8>> {
        let batch = self.to_record_batch()?;
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer = arrow_ipc::writer::StreamWriter::try_new(&mut buf, &batch.schema())
                .map_err(arrow_err)?;
            writer.write(&batch).map_err(arrow_err)?;
            writer.finish().map_err(arrow_err)?;
        }
        Ok(buf)
    }

    /// Builds a frame named `name` from an **Arrow IPC stream** (as written by
    /// [`to_ipc_bytes`](StructSerie::to_ipc_bytes) or any Arrow library) — every batch is
    /// concatenated.
    pub fn from_ipc_bytes(name: impl Into<String>, bytes: &[u8]) -> SerieResult<StructSerie> {
        let reader = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes), None)
            .map_err(arrow_err)?;
        StructSerie::from_record_batch_reader(name, reader)
    }
}
