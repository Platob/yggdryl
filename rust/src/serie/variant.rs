//! The column a self-describing value is stored in.
//!
//! A variant row is one run of bytes - the crate's own encoding, version,
//! identifier, payload, children inside - so a variant column is a byte
//! column and nothing more: the offsets cut one encoded value per row, and
//! [`VariantSerie::bytes`] lends that run where it lies.
//!
//! The encoding itself is [`crate::variant`]'s, reached through
//! [`Scalar::into_variant_bytes`] and [`Scalar::decode_variant_bytes`] -
//! the same two the crate's own variant column is built and read by - so a
//! column grows no second encoder and a row written here reads back byte
//! for byte wherever else the encoding is read.

use std::fmt;
use std::sync::Arc;

use arrow_array::builder::BinaryBuilder;
use arrow_array::{Array, ArrayRef, BinaryArray};
use arrow_buffer::{Buffer, NullBuffer, OffsetBuffer};

use super::Serie;
use crate::value::SerieValue;
use crate::{Field, Result, Scalar};

/// One column of self-describing values, each row one encoded run of bytes.
///
/// There is one width and no `Large` twin, because `DataType::Variant`
/// projects to Arrow `Binary` and nothing else - the encoding names the
/// storage, so a second offset width would be a layout no field can declare.
/// [`GenericSequenceSerie`](crate::GenericSequenceSerie) has both widths
/// because `list` and `large_list` are both datatypes.
#[derive(Clone)]
pub struct VariantSerie {
    field: Arc<Field>,
    runs: BinaryArray,
}

impl VariantSerie {
    /// Pair a variant field with the runs that hold its rows.
    pub(crate) const fn new(field: Arc<Field>, runs: BinaryArray) -> Self {
        Self { field, runs }
    }

    /// Borrow row `index`'s encoded run where it lies.
    ///
    /// The bytes are the crate's variant encoding, so a caller that wants to
    /// forward a row rather than read it moves them without decoding.
    pub fn bytes(&self, index: usize) -> Option<&[u8]> {
        (index < self.runs.len() && !self.runs.is_null(index)).then(|| self.runs.value(index))
    }

    /// Borrow the payload buffer every run lies in, without copying it.
    pub fn payload(&self) -> &Buffer {
        self.runs.values()
    }

    /// Borrow the offsets buffer that cuts the runs, without copying it.
    pub fn offsets(&self) -> &OffsetBuffer<i32> {
        self.runs.offsets()
    }

    /// Borrow the Arrow array these runs are.
    pub const fn array(&self) -> &BinaryArray {
        &self.runs
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.runs.nulls()
    }

    /// Append one already-encoded run, without decoding it.
    pub fn push_bytes(&mut self, value: Option<&[u8]>) {
        self.runs = self.rebuilt(None, value);
    }

    /// The same runs, with row `replace` rewritten or one more on the end.
    fn rebuilt(&self, replace: Option<usize>, value: Option<&[u8]>) -> BinaryArray {
        let mut builder = BinaryBuilder::new();
        for row in 0..self.runs.len() {
            if replace == Some(row) {
                builder.append_option(value);
            } else if self.runs.is_null(row) {
                builder.append_null();
            } else {
                builder.append_value(self.runs.value(row));
            }
        }
        if replace.is_none() {
            builder.append_option(value);
        }
        builder.finish()
    }
}

impl SerieValue for VariantSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.runs.len()
    }

    fn null_count(&self) -> usize {
        self.runs.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.runs.len() || self.runs.is_null(index)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        let Some(bytes) = self.bytes(index) else {
            return Ok(Scalar::Null);
        };
        Scalar::decode_variant_bytes(bytes)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.runs.len())?;
        let value = self.field.scalar(value)?;
        if value.is_null() {
            self.runs = self.rebuilt(Some(index), None);
            return Ok(());
        }
        self.runs = self.rebuilt(Some(index), Some(&value.into_variant_bytes()));
        Ok(())
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let value = self.field.scalar(value)?;
        if value.is_null() {
            self.push_bytes(None);
            return Ok(());
        }
        self.push_bytes(Some(&value.into_variant_bytes()));
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        Arc::new(self.runs.clone())
    }

    fn into_serie(self) -> Serie {
        Serie::Variant(Arc::new(self))
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Variant(column) => Some(column.as_ref()),
            _ => None,
        }
    }
}

impl fmt::Debug for VariantSerie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::debug_leaf(self, "VariantSerie", formatter)
    }
}

serie_leaf!(VariantSerie);
