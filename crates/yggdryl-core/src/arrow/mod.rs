//! `arrow` — the Apache Arrow interop bridge (feature `arrow`).
//!
//! Behind the opt-in `arrow` feature, every element type in the crate converts **to and from** its
//! closest Arrow equivalent: a [`DataTypeId`](crate::datatype_id::DataTypeId) (+ its field params) ↔
//! an Arrow [`DataType`](arrow_schema::DataType), a [`HeaderField`](crate::typed::HeaderField) ↔ an
//! Arrow [`Field`](arrow_schema::Field), and a [`Column`](crate::typed::Column) ↔ an Arrow
//! [`Array`](arrow_array::Array). The **nested** carriers recurse: a struct becomes a
//! [`StructArray`](arrow_array::StructArray), a list a [`ListArray`](arrow_array::ListArray), a map a
//! [`MapArray`](arrow_array::MapArray) — through arbitrary depth. At the top level a
//! [`StructSerie`](crate::typed::StructSerie) ↔ a [`RecordBatch`](arrow_array::RecordBatch) and a
//! [`StructField`](crate::typed::StructField) ↔ a [`Schema`](arrow_schema::Schema).
//!
//! # Closest-match map
//!
//! Conversions are **total** and pick the closest Arrow type; where Arrow has no exact match the
//! bridge maps to the nearest one and documents the lossy edge.
//!
//! | [`DataTypeId`](crate::datatype_id::DataTypeId) | Arrow [`DataType`](arrow_schema::DataType) | notes |
//! |---|---|---|
//! | `Unknown` | `Null` | |
//! | `Bool` | `Boolean` | |
//! | `I8`/`U8`/`I16`/`U16`/`I32`/`U32`/`I64`/`U64` | `Int*` / `UInt*` | exact |
//! | `I128` | `Decimal128(38, 0)` | **lossy:** Arrow has no 128-bit integer; a scale-0 decimal is the closest (an `i128` **is** a scale-0 `Decimal128`). The reverse maps `Decimal128` → `Decimal128`. |
//! | `U128` | `Decimal128(38, 0)` | **lossy:** as `I128`, and a `u128` ≥ 2¹²⁷ shows as **negative** on the Arrow side — the 16 raw bytes still round-trip losslessly. |
//! | `F32`/`F64` | `Float32` / `Float64` | exact |
//! | `Decimal32` | `Decimal128(precision, scale)` | **lossy (widened):** per spec, `Decimal32` maps to `Decimal128` (the reverse maps `Decimal128` → `Decimal128`). Note arrow-rs v56 *does* have a native `Decimal32`; this crate follows the spec's Decimal128 target. |
//! | `Decimal64` | `Decimal128(precision, scale)` | **lossy (widened):** as `Decimal32`. |
//! | `Decimal128` | `Decimal128(precision, scale)` | exact |
//! | `Decimal256` | `Decimal256(precision, scale)` | exact |
//! | `Binary` / `LargeBinary` | `Binary` / `LargeBinary` | exact |
//! | `Utf8` / `LargeUtf8` | `Utf8` / `LargeUtf8` | exact |
//! | `FixedBinary` | `FixedSizeBinary(byte_width)` | exact (width from the field) |
//! | `FixedUtf8` | `FixedSizeBinary(byte_width)` | **lossy:** Arrow has no fixed-size UTF-8; the reverse maps `FixedSizeBinary` → `FixedBinary` (only our own field metadata restores `FixedUtf8`). |
//! | `Struct` / `List` / `Map` | `Struct(empty)` / `List(null item)` / `Map(null entries)` | from the **id alone** this is a structural shell (children can't come from the id); a real nested [`Column`](crate::typed::Column) / [`ColumnField`](crate::typed::ColumnField) carries its children and maps to the full nested type (see below). |
//!
//! # Nested + RecordBatch mapping
//!
//! A nested [`Column`](crate::typed::Column) maps to the matching nested Arrow array — a
//! [`StructSerie`](crate::typed::StructSerie) to a [`StructArray`](arrow_array::StructArray) (child
//! arrays from each child column, fields from each child descriptor, the row-level validity as the
//! array's [`NullBuffer`](arrow_buffer::NullBuffer)); a [`ListSerie`](crate::typed::ListSerie) to a
//! [`ListArray`](arrow_array::ListArray) (`i32` offsets, the flattened child as the values array); a
//! [`MapSerie`](crate::typed::MapSerie) to a [`MapArray`](arrow_array::MapArray) (a
//! `List<Struct<key, value>>`, key field forced non-nullable, `keys_sorted` preserved). The mapping
//! is **recursive** — a struct-of-list-of-map round-trips at any depth — and respects a **sliced**
//! Arrow array on the reverse path (the referenced value / entry range is sliced out and the offsets
//! rebased from 0).
//!
//! At the top level, a [`StructSerie`](crate::typed::StructSerie) ↔ a
//! [`RecordBatch`](arrow_array::RecordBatch) (schema + column arrays) and a
//! [`StructField`](crate::typed::StructField) ↔ a [`Schema`](arrow_schema::Schema) (child fields +
//! struct-level metadata). **Row-validity caveat:** a `RecordBatch` has **no** row-level validity, so
//! [`struct_serie_to_record_batch`] **refuses** (a guided error) a struct that actually holds null
//! rows rather than silently dropping them — a struct that is nullable but holds no null rows converts
//! cleanly, and row-level nulls travel losslessly through a [`StructArray`](arrow_array::StructArray)
//! ([`column_to_arrow`]) instead.
//!
//! # Copy profile
//!
//! On the **`column_to_arrow`** side a value / offsets buffer is handed to an owning
//! [`Buffer`](arrow_buffer::Buffer) with **one copy** from the borrowed slice — unavoidable because
//! the entry point borrows the `&Column` and cannot move its [`Heap`](crate::io::memory::Heap)
//! (with an owned `Heap` this would be a zero-copy `Buffer::from_vec`). `Decimal32`/`Decimal64`
//! (widened `i32`/`i64` → `i128`) and `Decimal256` (via `i256::from_le_bytes`) copy through an owned
//! `Vec` instead. On the **`column_from_arrow`** side each column is rebuilt through the typed
//! encoders (one vectorized copy per column; byte / fixed-size columns rebuild element-by-element to
//! **rebase offsets** and respect a **sliced** input), so a sliced or offset Arrow array round-trips
//! correctly.
//!
//! The `column_to_arrow` value-buffer handoff reinterprets our little-endian wire bytes as the
//! Arrow native element type — correct on a **little-endian host** (the crate's canonical wire
//! format and arrow-rs's only supported host).

mod array;
mod data_type;
mod field;
mod record_batch;
mod schema;

pub use array::{column_from_arrow, column_to_arrow};
pub use data_type::{from_arrow_data_type, to_arrow_data_type};
pub use field::{from_arrow_field, to_arrow_field};
pub use record_batch::{struct_serie_from_record_batch, struct_serie_to_record_batch};
pub use schema::{struct_field_from_arrow_schema, struct_field_to_arrow_schema};
