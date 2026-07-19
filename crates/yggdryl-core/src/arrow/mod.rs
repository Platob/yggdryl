//! `arrow` — the Apache Arrow **leaf** interop bridge (feature `arrow`).
//!
//! Behind the opt-in `arrow` feature, every **leaf** element type in the crate converts **to and
//! from** its closest Arrow equivalent: a [`DataTypeId`](crate::datatype_id::DataTypeId) (+ its field
//! params) ↔ an Arrow [`DataType`](arrow_schema::DataType), a
//! [`HeaderField`](crate::typed::HeaderField) ↔ an Arrow [`Field`](arrow_schema::Field), and a leaf
//! [`Column`](crate::typed::Column) ↔ an Arrow [`Array`](arrow_array::Array). The nested
//! `Struct` / `List` / `Map` arms are reserved for a later nested phase — they return a guided error
//! here (and the type map emits a documented structural shell).
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
//! | `Struct` / `List` / `Map` | `Struct(empty)` / `List(null item)` / `Map(null entries)` | **structural shell** — children can't come from the id alone; the nested phase fills them in. |
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

pub use array::{column_from_arrow, column_to_arrow};
pub use data_type::{from_arrow_data_type, to_arrow_data_type};
pub use field::{from_arrow_field, to_arrow_field};
