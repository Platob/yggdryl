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

use arrow_array::builder::{BinaryBuilder, LargeBinaryBuilder};
use arrow_array::{Array, ArrayRef, BinaryArray, LargeBinaryArray};
use arrow_buffer::{Buffer, NullBuffer, OffsetBuffer};

use super::Serie;
use crate::value::SerieValue;
use crate::{Field, Result, Scalar};

/// The two byte layouts a variant column is written under.
#[derive(Clone, Debug)]
pub(crate) enum EncodedRuns {
    /// 32-bit offsets.
    Small(BinaryArray),
    /// 64-bit offsets.
    Large(LargeBinaryArray),
}

impl EncodedRuns {
    /// The rows these runs hold.
    fn len(&self) -> usize {
        match self {
            Self::Small(runs) => runs.len(),
            Self::Large(runs) => runs.len(),
        }
    }

    /// How many rows hold no value.
    fn null_count(&self) -> usize {
        match self {
            Self::Small(runs) => runs.null_count(),
            Self::Large(runs) => runs.null_count(),
        }
    }

    /// Whether row `index` holds no value.
    fn is_null(&self, index: usize) -> bool {
        match self {
            Self::Small(runs) => index >= runs.len() || runs.is_null(index),
            Self::Large(runs) => index >= runs.len() || runs.is_null(index),
        }
    }

    /// The encoded run row `index` holds.
    fn value(&self, index: usize) -> Option<&[u8]> {
        if self.is_null(index) {
            return None;
        }
        Some(match self {
            Self::Small(runs) => runs.value(index),
            Self::Large(runs) => runs.value(index),
        })
    }

    /// The validity bitmap, or `None` where no row is absent.
    fn nulls(&self) -> Option<&NullBuffer> {
        match self {
            Self::Small(runs) => runs.nulls(),
            Self::Large(runs) => runs.nulls(),
        }
    }

    /// The payload buffer every run lies in.
    fn payload(&self) -> &Buffer {
        match self {
            Self::Small(runs) => runs.values(),
            Self::Large(runs) => runs.values(),
        }
    }

    /// The same runs with one more encoded value on the end.
    fn appended(&self, value: Option<&[u8]>) -> Self {
        match self {
            Self::Small(runs) => {
                let mut builder = BinaryBuilder::new();
                for row in 0..runs.len() {
                    if runs.is_null(row) {
                        builder.append_null();
                    } else {
                        builder.append_value(runs.value(row));
                    }
                }
                builder.append_option(value);
                Self::Small(builder.finish())
            }
            Self::Large(runs) => {
                let mut builder = LargeBinaryBuilder::new();
                for row in 0..runs.len() {
                    if runs.is_null(row) {
                        builder.append_null();
                    } else {
                        builder.append_value(runs.value(row));
                    }
                }
                builder.append_option(value);
                Self::Large(builder.finish())
            }
        }
    }

    /// The same runs with row `index` rewritten.
    fn replaced(&self, index: usize, value: Option<&[u8]>) -> Self {
        match self {
            Self::Small(runs) => {
                let mut builder = BinaryBuilder::new();
                for row in 0..runs.len() {
                    if row == index {
                        builder.append_option(value);
                    } else if runs.is_null(row) {
                        builder.append_null();
                    } else {
                        builder.append_value(runs.value(row));
                    }
                }
                Self::Small(builder.finish())
            }
            Self::Large(runs) => {
                let mut builder = LargeBinaryBuilder::new();
                for row in 0..runs.len() {
                    if row == index {
                        builder.append_option(value);
                    } else if runs.is_null(row) {
                        builder.append_null();
                    } else {
                        builder.append_value(runs.value(row));
                    }
                }
                Self::Large(builder.finish())
            }
        }
    }

    /// The shared Arrow handle these runs already are.
    fn shared(&self) -> ArrayRef {
        match self {
            Self::Small(runs) => Arc::new(runs.clone()),
            Self::Large(runs) => Arc::new(runs.clone()),
        }
    }
}

/// One column of self-describing values, each row one encoded run of bytes.
#[derive(Clone)]
pub struct VariantSerie {
    field: Arc<Field>,
    runs: EncodedRuns,
}

impl VariantSerie {
    /// Pair a variant field with the runs that hold its rows.
    pub(crate) const fn new(field: Arc<Field>, runs: EncodedRuns) -> Self {
        Self { field, runs }
    }

    /// Borrow row `index`'s encoded run where it lies.
    ///
    /// The bytes are the crate's variant encoding, so a caller that wants to
    /// forward a row rather than read it moves them without decoding.
    pub fn bytes(&self, index: usize) -> Option<&[u8]> {
        self.runs.value(index)
    }

    /// Borrow the payload buffer every run lies in, without copying it.
    pub fn payload(&self) -> &Buffer {
        self.runs.payload()
    }

    /// Borrow the offsets buffer, without copying it.
    ///
    /// Answers `None` for a column written under 64-bit offsets;
    /// [`Self::large_offsets`] is that column's.
    pub fn offsets(&self) -> Option<&OffsetBuffer<i32>> {
        match &self.runs {
            EncodedRuns::Small(runs) => Some(runs.offsets()),
            EncodedRuns::Large(_) => None,
        }
    }

    /// Borrow the 64-bit offsets buffer, without copying it.
    pub fn large_offsets(&self) -> Option<&OffsetBuffer<i64>> {
        match &self.runs {
            EncodedRuns::Small(_) => None,
            EncodedRuns::Large(runs) => Some(runs.offsets()),
        }
    }

    /// Borrow the validity bitmap, or `None` where no row is absent.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        self.runs.nulls()
    }

    /// Append one already-encoded run, without decoding it.
    pub fn push_bytes(&mut self, value: Option<&[u8]>) {
        self.runs = self.runs.appended(value);
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
        self.runs.is_null(index)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        let Some(bytes) = self.runs.value(index) else {
            return Ok(Scalar::Null);
        };
        Scalar::decode_variant_bytes(bytes)
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.runs.len())?;
        let value = self.field.scalar(value)?;
        if value.is_null() {
            self.runs = self.runs.replaced(index, None);
            return Ok(());
        }
        self.runs = self.runs.replaced(index, Some(&value.into_variant_bytes()));
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
        self.runs.shared()
    }

    fn into_serie(self) -> Serie {
        Serie::Variant(self)
    }

    fn from_serie(value: &Serie) -> Option<&Self> {
        match value {
            Serie::Variant(column) => Some(column),
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
