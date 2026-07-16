# The `io` layer

`io` is yggdryl's Apache-Arrow-backed physical and typed-data layer — the single source of
truth that both the Python and Node extensions mirror method-for-method. Its pages follow the
same folder tree as the Rust core (`crates/yggdryl-core/src/io/`).

## Addressing & bytes

- **[URIs and URLs](uri.md)** — `Uri` / `Url` / `Authority`: RFC 3986, parsed from scratch,
  doubling as POSIX-normalized filesystem paths.
- **[Byte I/O](bytes.md)** — the `Bytes` buffer and `Whence` seeking (`pread` / `pwrite`,
  cursor `read` / `write`).
- **[Headers](headers.md)** — the one centralized byte-string key/value map, used for both
  HTTP headers and Arrow schema metadata.

## The typed-data model

- **[Schema](schema.md)** — `DataType` / `Field` / `DataTypeId` and the coarse
  `DataTypeCategory`, plus the erased `AnyField` / `AnyScalar` / `AnySerie` layer that lets a
  heterogeneous column tree be carried, navigated, and round-tripped.
- **[Navigation](navigation.md)** — addressing a value *inside* a column tree by path
  (`"parent.child"`, `"a[1]"`) or coordinate (`(i, j)`): read, write, slice, and the Python
  `[]` / Node named-method ergonomics.
- **[Numerics & operations](ops.md)** — the analytics seam: reductions (count / sum / mean /
  min / max), vectorized arithmetic (`add` / `sub` / `mul` / `div` / `rem`, serie×serie and
  scalar broadcast), and the reshape ops (`filter`, `fill_null`, `to_list` / `to_struct` /
  `to_map`).

## The type families

- **[Fixed-width](fixed/index.md)** — the numeric primitives (`u8`…`i256`, floats), fixed-size
  binary/utf8, and the null column; with **[Decimals](fixed/decimal.md)** (`d32`…`d256`) and
  **[Temporal](fixed/temporal.md)** (dates, times, timestamps, durations).
- **[Variable-length](var.md)** — UTF-8 and binary columns (offsets + data).
- **[Nested](nested.md)** — `struct`, `list`, and `map`, recursive to any depth.

## Casting & Arrow

- **[Casting](converter.md)** — `cast::<U>()` across the numeric family (value / scalar /
  serie / buffer), with the utf8/binary bridges.
- **[Arrow interop](arrow/index.md)** — behind the `arrow` feature, every column converts
  to/from an Arrow array (zero-copy where the physical layout matches), with the Python pyarrow
  C-Data bridge; see each type's page for its exact mapping.

Every value type is **equal, hashable, and byte-serializable** identically across the three
languages, so a column built in one language round-trips through another via
`serialize_bytes` / `deserialize_bytes`.
