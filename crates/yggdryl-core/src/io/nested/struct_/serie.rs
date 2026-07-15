//! [`StructSerie`] — a nullable **struct column**: a set of equal-length, heterogeneous child
//! [`Column`]s addressed by an ordered schema, plus an optional top-level validity mask. It is the
//! nested peer of the leaf `Serie`s, and the bridge between yggdryl and Arrow's `StructArray` /
//! `RecordBatch` (feature `arrow`): a struct column *is* a batch of named columns.

use super::{StructField, StructType};
use crate::io::bitmap::Bitmap;
use crate::io::nested::struct_::scalar::StructScalar;
use crate::io::nested::{Column, ColumnField, Value};
use crate::io::{Bytes, DataTypeId, IOCursor, IoError, SerieType};

/// A **nullable struct column** — one child [`Column`] per field (all of the same length), an
/// ordered schema of [`ColumnField`]s, and an optional top-level validity mask (a null struct row).
///
/// ```
/// use yggdryl_core::io::fixed::Serie;
/// use yggdryl_core::io::var::Utf8Serie;
/// use yggdryl_core::io::nested::{Column, StructSerie};
///
/// let ids = Column::from(Serie::from_values(&[1i64, 2, 3]));
/// let names = Column::from(Utf8Serie::from_strs(&[Some("a"), None, Some("c")]));
/// let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();
/// assert_eq!(table.len(), 3);
/// assert_eq!(table.num_columns(), 2);
/// assert_eq!(table.field(1).unwrap().name(), "name");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructSerie {
    fields: Vec<ColumnField>,
    columns: Vec<Column>,
    validity: Option<Bitmap>,
    len: usize,
}

impl StructSerie {
    /// A struct column from named child columns — the schema is inferred from each column's type
    /// (nullability inferred from whether it holds nulls). Errors
    /// [`Unsupported`](IoError::Unsupported) if the columns are not all the same length.
    pub fn from_named(columns: Vec<(&str, Column)>) -> Result<Self, IoError> {
        let len = columns.first().map_or(0, |(_, column)| column.len());
        for (name, column) in &columns {
            if column.len() != len {
                return Err(IoError::Unsupported {
                    what: format!(
                        "struct child column {name:?} has length {} but the struct length is \
                         {len}; every child column must be the same length",
                        column.len()
                    ),
                });
            }
        }
        let fields = columns
            .iter()
            .map(|(name, column)| column.field(name))
            .collect();
        let columns = columns.into_iter().map(|(_, column)| column).collect();
        Ok(Self {
            fields,
            columns,
            validity: None,
            len,
        })
    }

    /// A struct column from an explicit schema + one child column per field, with an optional
    /// per-row **present** mask (`present[i] == false` marks row `i` a null struct). Errors if the
    /// counts or lengths disagree.
    pub fn from_columns(
        fields: Vec<ColumnField>,
        columns: Vec<Column>,
        present: Option<&[bool]>,
    ) -> Result<Self, IoError> {
        if fields.len() != columns.len() {
            return Err(IoError::Unsupported {
                what: format!(
                    "struct has {} fields but {} child columns; they must match",
                    fields.len(),
                    columns.len()
                ),
            });
        }
        let len = columns.first().map_or(0, Column::len);
        for (field, column) in fields.iter().zip(&columns) {
            if column.len() != len {
                return Err(IoError::Unsupported {
                    what: format!(
                        "struct child column {:?} has length {} but the struct length is {len}",
                        field.name(),
                        column.len()
                    ),
                });
            }
        }
        let validity = present.and_then(|flags| {
            let mut bitmap = Bitmap::all_present(len);
            for (index, &is_present) in flags.iter().take(len).enumerate() {
                if !is_present {
                    bitmap.set(index, false);
                }
            }
            (bitmap.null_count() > 0).then_some(bitmap)
        });
        Ok(Self {
            fields,
            columns,
            validity,
            len,
        })
    }

    /// An empty (zero-row) struct column of the given schema.
    pub fn empty(schema: &StructField) -> Self {
        Self {
            fields: schema.fields().to_vec(),
            columns: schema.fields().iter().map(Column::empty_of).collect(),
            validity: None,
            len: 0,
        }
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column has no rows.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of null struct rows.
    pub fn null_count(&self) -> usize {
        self.validity.as_ref().map_or(0, Bitmap::null_count)
    }

    /// Whether any struct row is null.
    pub fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }

    /// The number of child columns (fields).
    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }

    /// The child field descriptors, in order.
    pub fn fields(&self) -> &[ColumnField] {
        &self.fields
    }

    /// The child field at `index`, or `None` if out of range.
    pub fn field(&self, index: usize) -> Option<&ColumnField> {
        self.fields.get(index)
    }

    /// The child column at `index`, or `None` if out of range.
    pub fn column(&self, index: usize) -> Option<&Column> {
        self.columns.get(index)
    }

    /// The child column named `name` (first match), or `None`.
    pub fn column_named(&self, name: &str) -> Option<&Column> {
        let index = self.fields.iter().position(|f| f.name() == name)?;
        self.columns.get(index)
    }

    /// The typed [`StructType`] descriptor (its child fields).
    pub fn data_type(&self) -> StructType {
        StructType::new(self.fields.clone())
    }

    /// A [`StructField`] naming this struct column, its nullability inferred from whether it holds
    /// any null rows.
    pub fn to_field(&self, name: &str) -> StructField {
        StructField::new(name, self.fields.clone(), self.has_nulls())
    }

    /// The row at `index` as a [`Value::Struct`] — [`Value::Null`] if the row is null or out of
    /// range (consistent with a leaf column's null cell). Use [`row_scalar`](StructSerie::row_scalar)
    /// to inspect the (logically-absent) child values under a null row.
    pub fn get_row(&self, index: usize) -> Value {
        if index >= self.len || self.validity.as_ref().is_some_and(|v| !v.get(index)) {
            return Value::Null;
        }
        Value::Struct(Box::new(self.row_scalar(index)))
    }

    /// The row at `index` as a [`StructScalar`] — its `is_null` flag reflects the top-level
    /// validity, but its per-field values are always populated (Arrow keeps child values under a
    /// null parent). Out of range yields a null scalar with empty values.
    pub fn row_scalar(&self, index: usize) -> StructScalar {
        if index >= self.len {
            return StructScalar::null(self.fields.clone(), Vec::new());
        }
        let values: Vec<Value> = self
            .columns
            .iter()
            .map(|column| column.get(index))
            .collect();
        if self.validity.as_ref().is_some_and(|v| !v.get(index)) {
            StructScalar::null(self.fields.clone(), values)
        } else {
            StructScalar::new(self.fields.clone(), values)
        }
    }

    // ---- serialization: the schema, then each child via its own `Serie` codec ----------

    /// Reads a struct body — `[validity?][each child column]` — guided by its `schema` and row
    /// count `len`. Each child delegates to its `Serie`'s own `read_from`.
    pub(crate) fn read_body<R: IOCursor>(
        schema: &StructField,
        len: usize,
        source: &mut R,
    ) -> Result<Self, IoError> {
        let validity = read_struct_validity(source, len)?;
        let mut columns = Vec::with_capacity(schema.num_fields());
        for field in schema.fields() {
            let column = Column::read_from(field, source)?;
            // A well-formed frame writes every child at the struct length; reject a corrupt frame
            // whose child declares a different length (else a later StructArray build would panic).
            if column.len() != len {
                return Err(IoError::Unsupported {
                    what: format!(
                        "struct child {:?} decoded with length {} but the struct length is {len}; \
                         the serialized frame is corrupt",
                        field.name(),
                        column.len()
                    ),
                });
            }
            columns.push(column);
        }
        Ok(Self {
            fields: schema.fields().to_vec(),
            columns,
            validity: normalize_validity(validity),
            len,
        })
    }

    /// This struct column's canonical bytes — a self-contained `[schema][len][body]` frame — as an
    /// owned `Vec`. The exact inverse of [`deserialize_bytes`](StructSerie::deserialize_bytes).
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Writes a self-contained frame: `[schema_len][schema][len][validity?][each child column]`.
    /// Each child delegates to its own `Serie`'s `write_to` (which is itself batched), so there is
    /// no parallel column codec.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        let schema_bytes = schema_to_bytes(&self.to_field(""));
        sink.write_all(&(schema_bytes.len() as u64).to_le_bytes())?;
        sink.write_all(&schema_bytes)?;
        sink.write_all(&(self.len as u64).to_le_bytes())?;
        write_struct_validity(sink, self.validity.as_ref())?;
        for column in &self.columns {
            column.write_to(sink)?;
        }
        Ok(())
    }

    /// Reconstructs a struct column from the bytes produced by
    /// [`serialize_bytes`](StructSerie::serialize_bytes).
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(&mut Bytes::from_slice(bytes))
    }

    /// Reads a struct column written by [`write_to`](StructSerie::write_to).
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let schema_len = read_u64(source)? as usize;
        let schema_bytes = source.read_exact_vec(schema_len)?;
        let schema = schema_from_bytes(&schema_bytes)?;
        let len = read_u64(source)? as usize;
        Self::read_body(&schema, len, source)
    }
}

impl SerieType for StructSerie {
    type Elem = Value;

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        self.null_count()
    }

    fn get(&self, index: usize) -> Option<Value> {
        match self.get_row(index) {
            Value::Null => None,
            value => Some(value),
        }
    }
}

/// Writes the struct's top-level validity `[has_validity: u8][validity bytes?]`.
fn write_struct_validity<W: IOCursor>(
    sink: &mut W,
    validity: Option<&Bitmap>,
) -> Result<(), IoError> {
    let present = validity.is_some_and(|bitmap| bitmap.null_count() > 0);
    sink.write_all(&[u8::from(present)])?;
    if present {
        sink.write_all(validity.unwrap().as_bytes())?;
    }
    Ok(())
}

/// Reads the struct's top-level validity for `len` rows (the mask read is length-bounded).
fn read_struct_validity<R: IOCursor>(
    source: &mut R,
    len: usize,
) -> Result<Option<Bitmap>, IoError> {
    let mut flag = [0u8; 1];
    source.read_exact(&mut flag)?;
    if flag[0] == 0 {
        return Ok(None);
    }
    let bits = source.read_exact_vec(len.div_ceil(8))?;
    Ok(Some(Bitmap::from_bytes(&bits, len)))
}

/// Drops an all-present mask to `None` so equality/serialization stay canonical.
fn normalize_validity(validity: Option<Bitmap>) -> Option<Bitmap> {
    validity.filter(|bitmap| bitmap.null_count() > 0)
}

/// Serializes a struct schema ([`StructField`]) to bytes (via its Arrow-independent field tree).
fn schema_to_bytes(schema: &StructField) -> Vec<u8> {
    let mut out = Vec::new();
    encode_field(&ColumnField::Struct(schema.clone()), &mut out);
    out
}

/// Deserializes a struct schema from [`schema_to_bytes`] bytes.
fn schema_from_bytes(bytes: &[u8]) -> Result<StructField, IoError> {
    let mut cursor = 0usize;
    match decode_field(bytes, &mut cursor)? {
        ColumnField::Struct(schema) => Ok(schema),
        _ => Err(IoError::Unsupported {
            what: "serialized struct schema did not decode to a struct".to_string(),
        }),
    }
}

/// Encodes a `ColumnField` into `out` (a compact, Arrow-independent field-tree codec).
fn encode_field(field: &ColumnField, out: &mut Vec<u8>) {
    match field {
        ColumnField::Leaf(leaf) => {
            out.push(0); // leaf tag
            encode_str(leaf.name(), out);
            out.extend_from_slice(&crate::io::FieldType::type_id(leaf).as_u16().to_le_bytes());
            out.extend_from_slice(&(leaf.byte_width() as u64).to_le_bytes());
            out.push(u8::from(leaf.nullable()));
            encode_headers(leaf.metadata(), out);
        }
        ColumnField::Struct(schema) => {
            out.push(1); // struct tag
            encode_str(schema.name(), out);
            out.push(u8::from(schema.nullable()));
            encode_headers(schema.metadata(), out);
            out.extend_from_slice(&(schema.num_fields() as u64).to_le_bytes());
            for child in schema.fields() {
                encode_field(child, out);
            }
        }
    }
}

/// Decodes a `ColumnField` from `bytes` at `*cursor`, advancing it.
fn decode_field(bytes: &[u8], cursor: &mut usize) -> Result<ColumnField, IoError> {
    use crate::io::fixed::Field as LeafField;
    let tag = read_byte(bytes, cursor)?;
    match tag {
        0 => {
            let name = decode_str(bytes, cursor)?;
            let type_id_raw = read_u16(bytes, cursor)?;
            let type_id =
                DataTypeId::from_u16(type_id_raw).ok_or_else(|| IoError::Unsupported {
                    what: format!("unknown data-type id 0x{type_id_raw:04x} in serialized schema"),
                })?;
            let byte_width = read_u64_at(bytes, cursor)? as usize;
            let nullable = read_byte(bytes, cursor)? != 0;
            let metadata = decode_headers(bytes, cursor)?;
            Ok(ColumnField::Leaf(
                LeafField::of(&name, type_id, byte_width, nullable).with_metadata(metadata),
            ))
        }
        1 => {
            let name = decode_str(bytes, cursor)?;
            let nullable = read_byte(bytes, cursor)? != 0;
            let metadata = decode_headers(bytes, cursor)?;
            let count = read_u64_at(bytes, cursor)? as usize;
            // Each child is at least one byte, so a `count` beyond the remaining bytes is corrupt;
            // cap the pre-allocation so it cannot overflow (the loop then errors on the short read).
            let mut children = Vec::with_capacity(count.min(bytes.len()));
            for _ in 0..count {
                children.push(decode_field(bytes, cursor)?);
            }
            Ok(ColumnField::Struct(
                StructField::new(&name, children, nullable).with_metadata(metadata),
            ))
        }
        other => Err(IoError::Unsupported {
            what: format!("unknown field tag {other} in serialized schema"),
        }),
    }
}

// ---- small length-prefixed codec primitives for the schema tree -------------------------

fn encode_str(value: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(&(value.len() as u64).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn encode_headers(headers: &crate::io::Headers, out: &mut Vec<u8>) {
    let pairs = headers.serialize_bytes();
    out.extend_from_slice(&(pairs.len() as u64).to_le_bytes());
    out.extend_from_slice(&pairs);
}

fn decode_headers(bytes: &[u8], cursor: &mut usize) -> Result<crate::io::Headers, IoError> {
    let len = read_u64_at(bytes, cursor)? as usize;
    let slice = take(bytes, cursor, len)?;
    crate::io::Headers::deserialize_bytes(slice).map_err(|_| IoError::Unsupported {
        what: "corrupt headers in serialized schema".to_string(),
    })
}

fn decode_str(bytes: &[u8], cursor: &mut usize) -> Result<String, IoError> {
    let len = read_u64_at(bytes, cursor)? as usize;
    let slice = take(bytes, cursor, len)?;
    String::from_utf8(slice.to_vec()).map_err(|_| IoError::Unsupported {
        what: "corrupt UTF-8 field name in serialized schema".to_string(),
    })
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8], IoError> {
    let end = cursor.checked_add(len).filter(|end| *end <= bytes.len());
    match end {
        Some(end) => {
            let slice = &bytes[*cursor..end];
            *cursor = end;
            Ok(slice)
        }
        None => Err(IoError::UnexpectedEof {
            offset: *cursor as u64,
            requested: len,
            available: bytes.len().saturating_sub(*cursor),
        }),
    }
}

fn read_byte(bytes: &[u8], cursor: &mut usize) -> Result<u8, IoError> {
    Ok(take(bytes, cursor, 1)?[0])
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, IoError> {
    let slice = take(bytes, cursor, 2)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u64_at(bytes: &[u8], cursor: &mut usize) -> Result<u64, IoError> {
    let slice = take(bytes, cursor, 8)?;
    Ok(u64::from_le_bytes(slice.try_into().unwrap()))
}

fn read_u64<R: IOCursor>(source: &mut R) -> Result<u64, IoError> {
    let mut bytes = [0u8; 8];
    source.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

// -------------------------------------------------------------------------------------
// Arrow interop (feature `arrow`): struct column <-> StructArray, and <-> RecordBatch.
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
impl StructSerie {
    /// This struct column as an Arrow [`StructArray`](arrow_array::StructArray) — **recursive**,
    /// each child column mapped by [`Column::to_arrow_array`], with the top-level validity as a
    /// `NullBuffer`.
    pub fn to_arrow_array(&self) -> arrow_array::StructArray {
        let arrow_fields: Vec<arrow_schema::Field> =
            self.fields.iter().map(ColumnField::to_arrow).collect();
        let nulls = self.validity.as_ref().map(|bitmap| {
            let buffer = arrow_buffer::Buffer::from(bitmap.as_bytes());
            arrow_buffer::NullBuffer::new(arrow_buffer::BooleanBuffer::new(buffer, 0, self.len))
        });
        if arrow_fields.is_empty() {
            // A field-less struct: Arrow needs the length supplied explicitly.
            return arrow_array::StructArray::new_empty_fields(self.len, nulls);
        }
        let child_arrays: Vec<arrow_array::ArrayRef> =
            self.columns.iter().map(Column::to_arrow_array).collect();
        arrow_array::StructArray::new(
            arrow_schema::Fields::from(arrow_fields),
            child_arrays,
            nulls,
        )
    }

    /// Builds a struct column from an Arrow [`StructArray`](arrow_array::StructArray) and its
    /// [`Field`](arrow_schema::Field) (of `Struct` type), recovering each child recursively.
    pub fn from_arrow_array(
        array: &arrow_array::StructArray,
        field: &arrow_schema::Field,
    ) -> Result<Self, IoError> {
        use crate::io::nested::validity_from_arrow;
        use arrow_array::Array;
        let arrow_schema::DataType::Struct(child_fields) = field.data_type() else {
            return Err(IoError::Unsupported {
                what: format!(
                    "expected an Arrow Struct field, got {:?}",
                    field.data_type()
                ),
            });
        };
        let mut fields = Vec::with_capacity(child_fields.len());
        let mut columns = Vec::with_capacity(child_fields.len());
        for (arrow_field, child) in child_fields.iter().zip(array.columns()) {
            fields.push(ColumnField::from_arrow(arrow_field).ok_or_else(|| {
                IoError::Unsupported {
                    what: format!(
                        "struct child {:?} of type {:?} is not a yggdryl-modeled column type",
                        arrow_field.name(),
                        arrow_field.data_type()
                    ),
                }
            })?);
            columns.push(Column::from_arrow_array(child.as_ref(), arrow_field)?);
        }
        Ok(Self {
            fields,
            columns,
            validity: validity_from_arrow(array),
            len: array.len(),
        })
    }

    /// This struct column as an Arrow [`RecordBatch`](arrow_array::RecordBatch) (feature `arrow`) —
    /// each field becomes a batch column. A `RecordBatch` has no top-level validity, so a struct
    /// column with **null rows** cannot be a batch: errors [`Unsupported`](IoError::Unsupported) in
    /// that case (convert via [`to_arrow_array`](StructSerie::to_arrow_array) instead).
    pub fn to_record_batch(&self) -> Result<arrow_array::RecordBatch, IoError> {
        use std::sync::Arc;
        if self.has_nulls() {
            return Err(IoError::Unsupported {
                what: "a struct column with null rows has no RecordBatch form (a batch has no \
                       top-level validity); use to_arrow_array for a nullable StructArray"
                    .to_string(),
            });
        }
        let arrow_fields: Vec<arrow_schema::Field> =
            self.fields.iter().map(ColumnField::to_arrow).collect();
        let schema = Arc::new(arrow_schema::Schema::new(arrow_fields));
        let columns: Vec<arrow_array::ArrayRef> =
            self.columns.iter().map(Column::to_arrow_array).collect();
        if columns.is_empty() {
            // A field-less batch still needs its row count.
            let options = arrow_array::RecordBatchOptions::new().with_row_count(Some(self.len));
            return arrow_array::RecordBatch::try_new_with_options(schema, columns, &options)
                .map_err(record_batch_err);
        }
        arrow_array::RecordBatch::try_new(schema, columns).map_err(record_batch_err)
    }

    /// Builds a struct column from an Arrow [`RecordBatch`](arrow_array::RecordBatch) (feature
    /// `arrow`) — its columns become the struct's fields (no top-level nulls).
    pub fn from_record_batch(batch: &arrow_array::RecordBatch) -> Result<Self, IoError> {
        let schema = batch.schema();
        let mut fields = Vec::with_capacity(schema.fields().len());
        let mut columns = Vec::with_capacity(schema.fields().len());
        for (arrow_field, array) in schema.fields().iter().zip(batch.columns()) {
            fields.push(ColumnField::from_arrow(arrow_field).ok_or_else(|| {
                IoError::Unsupported {
                    what: format!(
                    "record batch column {:?} of type {:?} is not a yggdryl-modeled column type",
                    arrow_field.name(),
                    arrow_field.data_type()
                ),
                }
            })?);
            columns.push(Column::from_arrow_array(array.as_ref(), arrow_field)?);
        }
        Ok(Self {
            fields,
            columns,
            validity: None,
            len: batch.num_rows(),
        })
    }
}

/// Maps an Arrow `RecordBatch` construction error to a guided [`IoError`].
#[cfg(feature = "arrow")]
fn record_batch_err(error: arrow_schema::ArrowError) -> IoError {
    IoError::Unsupported {
        what: format!("could not build a RecordBatch from the struct column: {error}"),
    }
}
