//! [`StructSerie`] ↔ Arrow [`RecordBatch`](arrow_array::RecordBatch) — the top-level table bridge.
//!
//! A struct column *is* a table, so it maps onto a `RecordBatch`: the schema comes from the struct's
//! [`field`](StructSerie::field) (through [`struct_field_to_arrow_schema`]) and each child column
//! through [`column_to_arrow`]. The inverse rebuilds the columns from the batch's arrays + schema.
//!
//! # The row-validity caveat
//!
//! A [`RecordBatch`](arrow_array::RecordBatch) has **no row-level validity** — Arrow cannot mark a
//! whole record null (only individual columns carry nulls). A [`StructSerie`], however, *can* have
//! null rows. This bridge takes the **honest** path: it **refuses** (a guided [`IoError`]) a struct
//! that actually holds null rows, rather than silently dropping the row-null information and
//! round-tripping those rows back as valid. A struct that is merely *nullable* but holds **no** null
//! rows converts cleanly (nothing is lost). To carry row-level nulls into Arrow, convert the struct
//! to a [`StructArray`](arrow_array::StructArray) with
//! [`column_to_arrow`](super::column_to_arrow) instead — a `StructArray` *does* carry a null buffer.

use std::sync::Arc;

use arrow_array::RecordBatch;

use crate::io::memory::IoError;
use crate::typed::nested::StructSerie;

use super::array::{build_error, column_from_arrow, column_to_arrow};
use super::field::column_field_from_arrow;
use super::schema::struct_field_to_arrow_schema;

/// A [`StructSerie`] as an Arrow [`RecordBatch`]: the schema from the struct's
/// [`field`](StructSerie::field), each column through [`column_to_arrow`]. **Errors** (guided) when
/// the struct holds null rows — a `RecordBatch` has no row-level validity (see the
/// [module docs](self) for the caveat and the `StructArray` alternative).
///
/// ```
/// use yggdryl_core::arrow::struct_serie_to_record_batch;
/// use yggdryl_core::typed::fixedbyte::Int64;
/// use yggdryl_core::typed::varbyte::Utf8;
/// use yggdryl_core::typed::{Column, FixedSerie, StructSerie, VarSerie};
///
/// let id = FixedSerie::<Int64>::from_values(&[1, 2, 3]).with_name("id");
/// let name = VarSerie::<Utf8>::from_values(&["ada".into(), "bo".into(), "cy".into()])
///     .with_name("name");
/// let table = StructSerie::from_columns(vec![Column::from(id), Column::from(name)]).unwrap();
///
/// let batch = struct_serie_to_record_batch(&table).unwrap();
/// assert_eq!(batch.num_columns(), 2);
/// assert_eq!(batch.num_rows(), 3);
/// assert_eq!(batch.schema().field(0).name(), "id");
/// ```
pub fn struct_serie_to_record_batch(serie: &StructSerie) -> Result<RecordBatch, IoError> {
    let null_rows = serie.null_count();
    if null_rows > 0 {
        return Err(IoError::TypedCast {
            detail: format!(
                "a RecordBatch has no row-level validity, but this struct has {null_rows} null \
                 row(s): Arrow record batches cannot mark a whole row null — convert the struct to a \
                 StructArray with column_to_arrow (which carries row validity), or fill / drop the \
                 null rows first"
            ),
        });
    }
    let schema = struct_field_to_arrow_schema(&serie.field());
    let mut columns = Vec::with_capacity(serie.num_columns());
    for column in serie.columns() {
        columns.push(column_to_arrow(column)?);
    }
    RecordBatch::try_new(Arc::new(schema), columns).map_err(build_error)
}

/// The inverse of [`struct_serie_to_record_batch`]: an Arrow [`RecordBatch`] → a [`StructSerie`] —
/// each column rebuilt through [`column_from_arrow`] with the [`ColumnField`](crate::typed::ColumnField)
/// derived from the batch's schema field, plus the schema metadata carried back. The struct is
/// non-nullable (a `RecordBatch` has no row-level validity to restore) and unnamed (a schema has no
/// name).
///
/// ```
/// use yggdryl_core::arrow::{struct_serie_from_record_batch, struct_serie_to_record_batch};
/// use yggdryl_core::typed::fixedbyte::Int64;
/// use yggdryl_core::typed::{Column, FixedSerie, StructSerie, Value};
///
/// let id = FixedSerie::<Int64>::from_values(&[7, 8]).with_name("id");
/// let table = StructSerie::from_columns(vec![Column::from(id)]).unwrap();
/// let batch = struct_serie_to_record_batch(&table).unwrap();
///
/// let back = struct_serie_from_record_batch(&batch).unwrap();
/// assert_eq!(back.num_columns(), 1);
/// assert_eq!(back.len(), 2);
/// let row = back.row(1).unwrap();
/// assert_eq!(row.get_by_name("id"), Some(&Value::Int64(8)));
/// ```
pub fn struct_serie_from_record_batch(batch: &RecordBatch) -> Result<StructSerie, IoError> {
    let schema = batch.schema();
    let mut columns = Vec::with_capacity(batch.num_columns());
    for (field, array) in schema.fields().iter().zip(batch.columns()) {
        let column_field = column_field_from_arrow(field);
        columns.push(column_from_arrow(array, &column_field)?);
    }
    let mut serie = StructSerie::from_columns(columns)?;
    for (key, value) in schema.metadata() {
        serie.metadata_mut().insert(key, value);
    }
    Ok(serie)
}
