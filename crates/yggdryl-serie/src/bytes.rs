//! Arrow-IPC byte serialization for a column: [`Serie::to_bytes`](crate::Serie::to_bytes)
//! / [`from_bytes`] round-trip a [`Serie`](crate::Serie) **losslessly** (type, name,
//! nulls and values, including nested) through the Arrow IPC **stream** format — the
//! canonical bytes form a binding's pickle / `toJSON` uses. **All column-bytes logic
//! lives here.**

use std::io::Cursor;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch};
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::Schema;
use yggdryl_schema::Field;

use crate::error::{SerieError, SerieResult};
use crate::serie::{from_arrow, SerieRef};

/// Maps an Arrow IPC error to a [`SerieError`].
fn ipc_err(e: arrow_schema::ArrowError) -> SerieError {
    SerieError::Arrow(e.to_string())
}

/// Encodes a single column (`field` + `array`) as an Arrow IPC stream — a one-field
/// record batch. The inverse of [`from_bytes`].
pub(crate) fn to_ipc_bytes(field: &Field, array: ArrayRef) -> SerieResult<Vec<u8>> {
    let schema = Arc::new(Schema::new(vec![field.to_arrow()?]));
    let batch = RecordBatch::try_new(schema.clone(), vec![array]).map_err(ipc_err)?;
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut buf, &schema).map_err(ipc_err)?;
        writer.write(&batch).map_err(ipc_err)?;
        writer.finish().map_err(ipc_err)?;
    }
    Ok(buf)
}

/// Decodes a single-column Arrow IPC stream (as written by
/// [`Serie::to_bytes`](crate::Serie::to_bytes)) back into a [`Serie`](crate::Serie),
/// preserving the field's name / type / nullability and the column's values.
pub fn from_bytes(bytes: &[u8]) -> SerieResult<SerieRef> {
    let mut reader = StreamReader::try_new(Cursor::new(bytes), None).map_err(ipc_err)?;
    let schema = reader.schema();
    let afield = schema.fields().first().ok_or_else(|| {
        SerieError::Arrow("Arrow IPC stream has no column to read as a serie".into())
    })?;
    let field = Field::from_arrow(afield.as_ref());
    // A serie is a single column / single batch — take the first.
    let batch = reader
        .next()
        .ok_or_else(|| SerieError::Arrow("Arrow IPC stream has no record batch".into()))?
        .map_err(ipc_err)?;
    from_arrow(field, batch.column(0).clone())
}
