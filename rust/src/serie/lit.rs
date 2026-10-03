//! The column a constant is stored in: one field, one value, a length - and
//! the Arrow array it lays out as, built on first export and shared after.
//!
//! A partition column restored from a path or a manifest, a column every
//! row of which is the field's default, a literal broadcast over a batch, a
//! window's static cell: each is one value repeated, and laying it out row
//! by row costs the rows. This leaf keeps the value once, proven under its
//! field, beside the row it lays out as, and counts; reading a cell clones
//! the value, slicing moves the count, and the full Arrow array is built
//! only when something exports it, then kept so a second export and every
//! slice of it share the buffers. A write of another value lays the column
//! out as its field's own leaf first ([`Serie::splice`]); a write of the
//! same value, or a removal, moves the count.

use std::fmt;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use arrow_array::{ArrayRef, UInt32Array};

use super::{Serie, require_range, require_row, require_window};
use crate::spill::Backing;
use crate::value::SerieValue;
use crate::{Field, Result, Scalar};

/// One constant column: a field, the value every row holds, a length, and
/// the laid-out array once something asked for it.
#[derive(Clone)]
pub struct LitSerie {
    field: Arc<Field>,
    /// The value, canonical under the field.
    value: Scalar,
    rows: usize,
    /// The value laid out once as a one-row column, proven where it was
    /// built: what the full array is taken from, and what reads needing a
    /// layout - a digest, a materialization - reach the value through.
    row: Serie,
    /// The column laid out whole as its field's own leaf, built the first
    /// time something needs the layout - an export, a typed narrowing - and
    /// kept.
    built: OnceLock<Serie>,
}

impl LitSerie {
    /// Pair `field` with `value` repeated `rows` times.
    ///
    /// # Errors
    ///
    /// Returns the field's refusal of the value, and the layout refusal of a
    /// field with no Arrow projection.
    pub(crate) fn new(field: Arc<Field>, value: Scalar, rows: usize) -> Result<Self> {
        let value = field.scalar(value)?;
        let row = super::arrow::from_canonical_rows(Arc::clone(&field), &[&value])?;
        Ok(Self {
            field,
            value,
            rows,
            row,
            built: OnceLock::new(),
        })
    }

    /// The value every row holds.
    #[must_use]
    pub const fn value(&self) -> &Scalar {
        &self.value
    }

    /// The value as the one-row column it lays out as.
    #[must_use]
    pub const fn row(&self) -> &Serie {
        &self.row
    }

    /// Whether the column has been laid out whole and is held.
    #[must_use]
    pub fn is_built(&self) -> bool {
        self.built.get().is_some()
    }

    /// This column laid out as its field's own leaf, every row a cell: the
    /// one row taken `rows` times, built on the first call and shared by
    /// every later one. What a typed narrowing - `as_int64`, `as_utf8` -
    /// reads a constant column through, so a reader of buffers never sees
    /// the constant.
    pub fn laid_out(&self) -> &Serie {
        self.built.get_or_init(|| self.build())
    }

    /// The Arrow array this column is: [`Self::laid_out`]'s buffers.
    pub fn array(&self) -> ArrayRef {
        self.laid_out()
            .into_arrow_array()
            .expect("a laid-out column has buffers")
    }

    fn build(&self) -> Serie {
        let row = self
            .row
            .into_arrow_array()
            .expect("a lit column's row is a laid-out column");
        let taken = if self.rows == 0 {
            Some(row.slice(0, 0))
        } else {
            let indices = UInt32Array::from_value(0, self.rows);
            // Arrow's take answers a zero-width fixed-size list by its child,
            // which has no rows to count, so such a row is laid out instead.
            arrow_select::take::take(row.as_ref(), &indices, None)
                .ok()
                .filter(|taken| taken.len() == self.rows)
        };
        let laid: Result<Serie> = match taken {
            // The rows are the row this column already proved, repeated.
            Some(taken) => {
                super::arrow::land(Arc::clone(&self.field), taken, &super::arrow::Proof::Proven)
                    .map_err(crate::Error::from)
            }
            None => super::arrow::from_canonical_rows(
                Arc::clone(&self.field),
                &vec![&self.value; self.rows],
            ),
        };
        laid.expect("a lit column's value lays out under its field")
    }

    /// This column laid out as its field's own leaf, every row a cell: what
    /// a write of another value lands in.
    ///
    /// # Errors
    ///
    /// The landing's refusal of a field with no Arrow projection.
    pub(crate) fn materialized(&self) -> Result<Serie> {
        Ok(self.laid_out().clone())
    }

    /// Forget the laid-out array, keeping the value: what a spill of this
    /// column is, since the value is its whole content.
    pub(crate) fn forget_built(&mut self) {
        self.built = OnceLock::new();
    }

    /// Whether every one of `rows` - canonical under the field - is this
    /// column's value, so a write of them moves the count alone.
    pub(crate) fn holds(&self, rows: &[Scalar]) -> bool {
        rows.iter().all(|row| *row == self.value)
    }

    /// Refuse what a write could not do: nothing, for a constant - a row of
    /// another value lays the column out as its field's leaf first
    /// ([`Serie::write`]), and a row of this value moves the count.
    pub(crate) fn check(&self, range: &Range<usize>, rows: &[Scalar]) -> Result<()> {
        let _ = (range, rows);
        Ok(())
    }

    /// Write canonical `rows` over a checked `range`, every one this value:
    /// only the count moves.
    pub(crate) fn write(&mut self, range: Range<usize>, rows: Vec<Scalar>) {
        debug_assert!(
            self.holds(&rows),
            "a lit column writes its own value by count"
        );
        self.rows = self.rows - range.len() + rows.len();
        self.forget_built();
    }

    /// Where the unit lives: a constant has no buffer to carry anywhere.
    pub(crate) fn set_backing(&mut self, _backing: Backing) {}

    /// The bytes this column's own buffers span: the one row, and the built
    /// array where it is held.
    pub(crate) fn own_size(&self) -> usize {
        self.row.memory_size() + self.built.get().map_or(0, Serie::resident_size)
    }

    /// Append `other`'s rows where it holds this value; `false` leaves this
    /// column as it was.
    pub(crate) fn append(&mut self, other: &Self) -> bool {
        if other.value != self.value {
            return false;
        }
        self.rows += other.rows;
        self.forget_built();
        true
    }
}

impl SerieValue for LitSerie {
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
        if self.value.is_null() { self.rows } else { 0 }
    }

    fn is_null(&self, index: usize) -> Result<bool> {
        require_row(self.field.name(), index, self.rows)?;
        Ok(self.value.is_null())
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        require_row(self.field.name(), index, self.rows)?;
        Ok(self.value.clone())
    }

    fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        require_window(self.field.name(), offset, length, self.rows)?;
        let built = OnceLock::new();
        if let Some(laid) = self.built.get()
            && let Ok(window) = laid.slice(offset, length)
        {
            // A slice of a laid-out column shares its buffers.
            let _ = built.set(window);
        }
        Ok(Self {
            field: Arc::clone(&self.field),
            value: self.value.clone(),
            rows: length,
            row: self.row.clone(),
            built,
        })
    }

    /// Replace rows `range` by `rows`, each of which holds this value: the
    /// count moves. A write of another value is [`Serie::splice`]'s, which
    /// lays the column out as its field's leaf first.
    fn splice(&mut self, range: Range<usize>, rows: Vec<Scalar>) -> Result<()> {
        require_range(self.field.name(), &range, self.rows)?;
        let canonical = rows
            .into_iter()
            .map(|row| self.field.scalar(row))
            .collect::<Result<Vec<Scalar>>>()?;
        if !self.holds(&canonical) {
            // The leaf alone cannot change what it is; `Serie::splice` lays
            // the column out before a write of another value reaches it.
            return Err(crate::Error::conflict(
                "the value a lit column holds",
                "another value",
                self.field.name(),
            ));
        }
        self.write(range, canonical);
        Ok(())
    }

    /// What the rows occupy once laid out: the one row's buffers, counted
    /// per row - near enough to size by, stable whether the array is built
    /// or not.
    fn memory_size(&self) -> usize {
        self.row.memory_size().saturating_mul(self.rows)
    }

    /// The one row, and the built array while it is held.
    fn resident_size(&self) -> usize {
        self.own_size()
    }

    /// Never: a constant is never on disk, a spill forgets its built array.
    fn is_spilled(&self) -> bool {
        false
    }

    fn spill(&mut self, options: &crate::SpillOptions) -> Result<()> {
        if !options.is_never() && bytes(self.own_size()) > options.byte_size() {
            self.forget_built();
        }
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        self.array()
    }

    fn into_serie(self) -> Serie {
        super::Leaf::root(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        super::Leaf::narrow(value)
    }
}

fn bytes(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

impl fmt::Debug for LitSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_column(self, "LitSerie", formatter)
    }
}

serie_leaf!(LitSerie);
