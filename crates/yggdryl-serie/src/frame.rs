//! [`StructSerie`] **as a DataFrame** — a table of named, typed columns. A struct
//! column's children *are* the frame's columns, so this module adds the table surface on
//! top of the [`StructSerie`](crate::StructSerie) / [`Serie`] / [`NestedSerie`] base:
//! [shape](StructSerie::shape) and [column names](StructSerie::column_names), projection
//! ([`select_columns`](StructSerie::select_columns)), column add / drop / rename
//! ([`with_column`](StructSerie::with_column) / [`drop_columns`](StructSerie::drop_columns)
//! / [`rename`](StructSerie::rename)), row selection ([`head`](StructSerie::head) /
//! [`tail`](StructSerie::tail) / [`slice_rows`](StructSerie::slice_rows) /
//! [`filter`](StructSerie::filter)), row stacking ([`vstack`](StructSerie::vstack)), a table
//! [render](StructSerie::show) and a lossless Arrow [`RecordBatch`] round-trip
//! ([`to_record_batch`](StructSerie::to_record_batch) /
//! [`from_record_batch`](StructSerie::from_record_batch)).
//!
//! Columns are accessed with the [`NestedSerie`] vocabulary the frame inherits —
//! [`children`](StructSerie::children), [`child`](NestedSerie::child) (by index) and
//! [`child_by_name`](NestedSerie::child_by_name) (by name) — so the frame adds the
//! *table* operations without restating column access.
//!
//! Every transform is **functional**: it returns a new lazy `StructSerie` that shares the
//! untouched columns' Arrow buffers (no copy), assembling the backing `StructArray` only
//! on demand.

use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray, RecordBatch};
use arrow_schema::{Field as AField, Schema};
use yggdryl_schema::Field;

use crate::error::{SerieError, SerieResult};
use crate::nested::{NestedSerie, StructSerie};
use crate::serie::{dispatch, Serie, SerieRef};

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

    /// Renders the frame as a readable text table (`name: type` headers, aligned cells),
    /// showing at most `max_rows` rows (`None` = all). The building block for a frame's
    /// `__str__` / display.
    pub fn show(&self, max_rows: Option<usize>) -> String {
        let rows = self.len();
        let cols = self.children();
        if cols.is_empty() {
            return format!("empty frame ({rows} rows, 0 columns)");
        }
        let shown = max_rows.map_or(rows, |m| m.min(rows));

        let headers: Vec<String> = cols
            .iter()
            .map(|c| format!("{}: {}", c.name(), c.data_type().to_str()))
            .collect();
        let cells: Vec<Vec<String>> = cols
            .iter()
            .map(|c| {
                (0..shown)
                    .map(|r| {
                        let value = c.value_at(r);
                        if value.is_null() {
                            "null".to_string()
                        } else {
                            value.to_string()
                        }
                    })
                    .collect()
            })
            .collect();
        let widths: Vec<usize> = (0..cols.len())
            .map(|ci| {
                let header = headers[ci].chars().count();
                let widest = cells[ci]
                    .iter()
                    .map(|s| s.chars().count())
                    .max()
                    .unwrap_or(0);
                header.max(widest)
            })
            .collect();

        let mut out = String::new();
        let join_row = |fields: &[String]| -> String {
            fields
                .iter()
                .enumerate()
                .map(|(i, f)| format!("{f:<width$}", width = widths[i]))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        out.push_str(&join_row(&headers));
        out.push('\n');
        out.push_str(
            &widths
                .iter()
                .map(|w| "-".repeat(*w))
                .collect::<Vec<_>>()
                .join("-+-"),
        );
        out.push('\n');
        for r in 0..shown {
            // `cells` is column-major (one Vec per column); a row picks index `r` from each.
            let row: Vec<String> = cells.iter().map(|col| col[r].clone()).collect();
            out.push_str(&join_row(&row));
            out.push('\n');
        }
        if rows > shown {
            out.push_str(&format!("… ({} more rows)\n", rows - shown));
        }
        out
    }
}
