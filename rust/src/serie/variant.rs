//! The column a self-describing value is stored in.
//!
//! A variant row is the Apache Parquet Variant pair - a metadata dictionary
//! and a value payload - so a variant column holds two byte runs per row and
//! lays out as Arrow's `Struct("metadata": Binary, "value": Binary)`, the
//! shape Parquet, Avro, Arrow and Iceberg all state for the type.
//!
//! The pair is lent where it lies: [`VariantSerie::metadata`] and
//! [`VariantSerie::value`] borrow one row's two runs without reading either,
//! and [`SerieValue::scalar`] answers `Scalar::Variant` - the pair, still
//! undecoded - so a caller forwarding a row moves bytes and a caller reading
//! one calls [`crate::Variant::scalar`] itself.

use std::fmt;
use std::sync::Arc;

use arrow_array::builder::BinaryBuilder;
use arrow_array::{Array, ArrayRef, BinaryArray, StructArray};
use arrow_buffer::NullBuffer;
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields};

use super::Serie;
use crate::value::SerieValue;
use crate::{Field, Result, Scalar, Variant};

/// One column of self-describing values, each row one encoded run of bytes.
///
/// There is one width and no `Large` twin, because `DataType::Variant`
/// projects to that struct of two `Binary` children and nothing else - the
/// standard names the storage, so a second offset width would be a layout no
/// field can declare. [`GenericSequenceSerie`](crate::GenericSequenceSerie)
/// has both widths because `list` and `large_list` are both datatypes.
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

    /// The two Arrow children a variant column lays out as.
    pub(crate) fn arrow_fields() -> Fields {
        Fields::from(vec![
            ArrowField::new("metadata", ArrowDataType::Binary, false),
            ArrowField::new("value", ArrowDataType::Binary, false),
        ])
    }

    /// Borrow row `index`'s metadata dictionary where it lies.
    pub fn metadata(&self, index: usize) -> Option<&[u8]> {
        (!self.is_absent(index)).then(|| self.metadata.value(index))
    }

    /// Borrow row `index`'s value payload where it lies.
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

    /// Append one already-encoded pair, without reading either run.
    pub fn push_pair(&mut self, pair: Option<(&[u8], &[u8])>) {
        self.write(None, pair);
    }

    /// Whether row `index` is past the end or absent.
    fn is_absent(&self, index: usize) -> bool {
        index >= self.metadata.len() || self.nulls.as_ref().is_some_and(|nulls| nulls.is_null(index))
    }

    /// Rewrite row `replace`, or append one more, keeping every other row.
    ///
    /// Arrow's children are as long as the record, so an absent row still
    /// occupies one slot in each: it holds empty bytes, and the validity
    /// bitmap beside them is what says the row is not there.
    fn write(&mut self, replace: Option<usize>, pair: Option<(&[u8], &[u8])>) {
        let rows = self.metadata.len();
        let mut metadata = BinaryBuilder::new();
        let mut value = BinaryBuilder::new();
        let mut present = Vec::with_capacity(rows + 1);
        for row in 0..rows {
            if replace == Some(row) {
                let (left, right) = pair.unwrap_or((&[], &[]));
                metadata.append_value(left);
                value.append_value(right);
                present.push(pair.is_some());
            } else {
                metadata.append_value(self.metadata.value(row));
                value.append_value(self.value.value(row));
                present.push(!self.is_absent(row));
            }
        }
        if replace.is_none() {
            let (left, right) = pair.unwrap_or((&[], &[]));
            metadata.append_value(left);
            value.append_value(right);
            present.push(pair.is_some());
        }
        self.metadata = metadata.finish();
        self.value = value.finish();
        self.nulls = present
            .iter()
            .any(|held| !held)
            .then(|| NullBuffer::from(present));
    }
}

impl SerieValue for VariantSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn len(&self) -> usize {
        self.metadata.len()
    }

    fn null_count(&self) -> usize {
        self.nulls.as_ref().map_or(0, NullBuffer::null_count)
    }

    fn is_null(&self, index: usize) -> bool {
        self.is_absent(index)
    }

    fn scalar(&self, index: usize) -> Result<Scalar> {
        let (Some(metadata), Some(value)) = (self.metadata(index), self.value(index)) else {
            return Ok(Scalar::Null);
        };
        // The pair is answered undecoded: reading it is the caller's ask, and
        // `Variant::scalar` is the one door that reads it.
        Ok(Scalar::Variant(Variant::new(metadata, value)?))
    }

    fn set(&mut self, index: usize, value: Scalar) -> Result<()> {
        super::require_row(self.field.name(), index, self.metadata.len())?;
        let value = self.field.scalar(value)?;
        if value.is_null() {
            self.write(Some(index), None);
            return Ok(());
        }
        let held = value.into_variant()?;
        self.write(Some(index), Some((held.metadata(), held.value())));
        Ok(())
    }

    fn push(&mut self, value: Scalar) -> Result<()> {
        let value = self.field.scalar(value)?;
        if value.is_null() {
            self.push_pair(None);
            return Ok(());
        }
        let held = value.into_variant()?;
        self.push_pair(Some((held.metadata(), held.value())));
        Ok(())
    }

    fn into_arrow_array(&self) -> ArrayRef {
        let children: Vec<ArrayRef> = vec![
            Arc::new(self.metadata.clone()),
            Arc::new(self.value.clone()),
        ];
        StructArray::try_new(Self::arrow_fields(), children, self.nulls.clone()).map_or_else(
            |_| arrow_array::new_empty_array(&ArrowDataType::Null),
            |array| Arc::new(array) as ArrayRef,
        )
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
