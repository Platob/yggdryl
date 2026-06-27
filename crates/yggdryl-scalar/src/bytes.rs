//! Arrow-IPC byte serialization for a [`Scalar`]: [`Scalar::to_bytes`] writes the value
//! as a one-field, one-row IPC **stream**, and [`from_bytes`] reads it back. This is the
//! canonical interchange form the bindings' pickle / `toJSON` use; the round-trip is
//! lossless at the **Arrow** level (logical refinements like [`Json`](crate::Scalar::Json)
//! normalise to their physical type — see the [`arrow`](crate::arrow) module docs; use
//! [`to_str`](crate::Scalar::to_str) / [`to_json`](crate::Scalar::to_json) to carry the
//! exact logical type).

use std::io::Cursor;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{Field as AField, Schema};

use crate::error::{ScalarError, ScalarResult};
use crate::scalar::Scalar;

/// Maps an Arrow IPC error to a [`ScalarError`].
fn ipc_err(e: arrow_schema::ArrowError) -> ScalarError {
    ScalarError::Arrow(e.to_string())
}

impl Scalar {
    /// Serialises the value to **Arrow IPC stream bytes** — a one-field, one-row batch,
    /// read back by [`from_bytes`]. The canonical bytes form a binding's pickle /
    /// `toJSON` uses.
    ///
    /// ```
    /// use yggdryl_scalar::{Scalar, from_bytes};
    /// let value = Scalar::utf8("hello");
    /// assert_eq!(from_bytes(&value.to_bytes().unwrap()).unwrap(), value);
    /// ```
    pub fn to_bytes(&self) -> ScalarResult<Vec<u8>> {
        let array = self.to_array()?;
        let field = AField::new("scalar", array.data_type().clone(), true);
        let schema = Arc::new(Schema::new(vec![field]));
        let batch = RecordBatch::try_new(schema.clone(), vec![array]).map_err(ipc_err)?;
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer = StreamWriter::try_new(&mut buf, &schema).map_err(ipc_err)?;
            writer.write(&batch).map_err(ipc_err)?;
            writer.finish().map_err(ipc_err)?;
        }
        Ok(buf)
    }
}

/// Decodes the Arrow IPC stream written by [`Scalar::to_bytes`] back into a [`Scalar`]
/// (the value of the single one-row column).
pub fn from_bytes(bytes: &[u8]) -> ScalarResult<Scalar> {
    let mut reader = StreamReader::try_new(Cursor::new(bytes), None).map_err(ipc_err)?;
    let batch = reader
        .next()
        .ok_or_else(|| ScalarError::Arrow("Arrow IPC stream has no record batch".into()))?
        .map_err(ipc_err)?;
    Scalar::from_array(batch.column(0).as_ref(), 0)
}
