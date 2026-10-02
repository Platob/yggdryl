# Spec: spilled series (A) and struct conversions (B)

Base: HEAD `d7c4d93`. Task #14 (window statics) is in flight in `rust/src/serie/arrow.rs`, `serie/order.rs`, `window_serie.rs`, `rust/tests/allocations.rs`, `tests/root/window_serie.rs` and `tests/serie/arrow.rs`. Anything touching those files waits for #14 to be committed (§11). Line anchors below come from the working tree and will drift by a few lines. The phase that edits a file re-reads its anchors first.

## 0. Ground verified in the code and the locked dependencies

| Fact | Where |
| --- | --- |
| A Buffer built from a `bytes::Bytes` keeps that Bytes' owner alive (`Deallocation::Custom(Arc<bytes::Bytes>)`). | arrow-buffer 59.2 `buffer/immutable.rs:530`, `bytes.rs:229-239` |
| `Buffer::into_mutable` refuses any allocation that is not `Standard`. So every leaf writer's existing "own it or copy once" fallback copies a mapped leaf to the heap. | `immutable.rs:383`, `mutable.rs:187-191`; `serie/layout.rs:4-9,46,88,118,160`, `primitive.rs:184-199`, `bytes.rs:249-267,810` |
| `Buffer::data_ptr()` is public and answers the base of the underlying allocation, which every slice of it shares. `Bytes::deallocation` is pub(crate), so nothing can ask a Buffer who owns it. | `immutable.rs:115`, `:240` |
| `bytes::Bytes::from_owner(T: AsRef<[u8]> + Send + 'static)` exists. It was added in bytes 1.9.0, and 1.10.1 fixed a leak in it. The manifest says `bytes = "1"` and the lock has 1.12.1. | bytes-1.12.1 `src/bytes.rs:254`, CHANGELOG; `rust/Cargo.toml:73` |
| `MmapMut::make_read_only(self) -> Result<Mmap>` is a safe call (mprotect / VirtualProtect). The crate's single `unsafe` is `map_file` (`MmapMut::map_mut`). | memmap2-0.9.11 `lib.rs:1388`; `local/file.rs:5-14,246-253` |
| `StreamWriter::write` passes uncompressed body buffers to the sink as `EncodedBuffer::Raw` and skips the intermediate body `Vec<u8>`. The stream writer uses `DictionaryTracker::new(false)` (replacement allowed); `FileWriter` uses `new(true)` (replacement refused). The default `IpcWriteOptions` alignment is 64. | arrow-ipc-59.2.0 `writer.rs:81,906,945,2085,1661,2065,527` |
| `StreamDecoder::decode(&mut Buffer)` slices the message and the body out of the caller's Buffer. `with_require_alignment(true)` turns a realigning copy into an error. | `reader/stream.rs:106,159,~190,~209` |
| `child_of` re-masks a non-contract child under a parent holding absent rows onto a fresh heap null buffer. This happens whatever the `Proof`, and the value buffers stay shared. | `serie/arrow.rs:721-745`, `parent_nulls :219` |
| The IPC medium writes only the stream dialect and stages it whole in a `Vec`. | `ipc/mod.rs:591-614` |
| merge spills through a named `TemporaryFile`, removed in `Drop`, read back on the same handle through `FileWriter::into_inner`, then `FileReader::set_index`. | `media/merge.rs:23-24,52,144-145,224-226,286,319-390` |
| The crate root declares `pub mod bytes`. Spell the external crate `::bytes::Bytes` so the path cannot be read as the module. | `lib.rs` (`pub mod bytes;`) |
| `Serie::as_struct -> Option<&StructSerie>` is a leaf narrowing. `Scalar::as_struct -> Option<&BTreeMap>` is the named input map. Neither DataType nor Field has an `as_<leaf>` family. `StructField::from_field` and `DataType::as_fields` are the borrowed narrowings. | `serie.rs:3489`, `scalar.rs:2041`, `value/mod.rs:788`, `structure.rs:659` |
| The bindings already have inference doors that use the requested name: Python `Scalar.into_struct_field` and `into_array_field`, Node `Scalar` `intoStructField` and `intoArrayField`. The JS record-class static getter `intoStructField` is a different thing. | `python/src/scalar.rs:1101-1122`, `node/src/text/codec.rs:494-521`, `.api-bindings.txt:66,400,532` |

## 1. Semantics

### A. Spilled is a state, not a type

- **No `SpilledSerie` struct and no `Serie` variant.** A spilled column is an ordinary `Serie` or `ChunkedSerie` whose leaves hold Arrow arrays sliced out of one read-only memory mapping of a private IPC stream file. Every `as_<leaf>` narrowing, `value(i)`, window, cast, export, comparison and serde works unchanged and zero copy.
  - A variant would make every narrowing answer `None` for spilled data.
  - A wrapper type would be a second collection API.
  - "SpilledSerie" is used only as a docs heading.
- **`is_spilled(&self) -> bool`** (the AGENTS spelling of the user's `spilled -> bool`). True when **some** non-empty buffer the column reaches lies in a live spill mapping. That covers values, offsets, views, validity, children, dictionary values, run ends and union ids. "Some" is the garbage-collection fact: dropping the value may release a spill.
  - It is not "every byte is on disk". Landing re-masks some child bitmaps onto the heap, and a write copies the leaf it touches.
  - It is false for a Run, for a column that reaches no buffer byte (zero rows, the `null` datatype), and when no spill lives.
- **Heap-free** (crate-private): every non-empty buffer is mapped. Vacuously true for a column with no buffer bytes. This is the test `into_spilled` uses to answer a clone instead of writing again.
- **`into_spilled(&self, folder)`** answers a new value; `self` is untouched.
  - The column is written once, uncompressed, into an anonymous file in `folder`.
  - It is mapped read-only once and decoded zero copy.
  - It lands again under the source's own `Arc<Field>` with `Proof::Proven`.
  - The answer is equal to the source, since identity is the rows alone.
- **`as_spilled(&mut self, folder)`** is `*self = self.into_spilled(folder)?`. It is atomic.
- **`into_resident` / `as_resident`** are the reverse pair. They copy each mapped buffer once into an aligned heap buffer and keep heap buffers by pointer. This is the deterministic way to release a spill without editing the column.
- **Copy on write.** A write to a spilled column copies the leaf it touches through the existing fallback. That leaf moves to the heap; the rest stays mapped.
- **Garbage collection is ownership.** The mapping lives inside the Arrow buffers' own `Arc`. The file's blocks are freed when the last value referencing any slice of it drops: a clone, slice, window, chunk, `Scalar::from(serie)`, an `into_arrow_array` result, or a pyarrow export. No leaf field, no guard on `Serie`, no sweeper.
- **Window statics** (WINDOW_STATICS_SPEC) are never written. A `SerieReader` window's record is dropped by `SerieReader::into_spilled`; a held window forwards `is_spilled` only.

### B. Struct state pair

- **`into_struct_<root>(&self)`** answers the value itself when it is already a struct (clones only bump pointers), otherwise a one-child `struct<self>`. There is one per root: `into_struct_type`, `into_struct_field`, `into_struct_scalar`, `into_struct_serie`, plus `ChunkedSerie::into_struct_serie`.
- **No `as_struct_*` form (the user's decision):** `as_<noun>` stays reserved for dedicated borrowed views, so the conversion has only its `into_struct_<root>` spelling. The borrowed "is it a struct" half is the existing narrowings below.
- **The borrowed "already a struct?" half stays the existing narrowings**: `DataType::as_fields`, `StructField::from_field`, `Serie::as_struct`, `Scalar::as_struct`. The only new predicate is `DataType::is_struct()` (`Field::is_struct` exists).
- "Already a struct" means the Struct shape at any nullability, with any absent rows. It is answered as itself, losslessly.
- A wrapper is always required, because a wrap never adds an absent row.
- A wrap is refused past `DataType::PARSE_RECURSION_LIMIT` (`validate_bounded`), naming the field.
- A Run is refused by `Serie::require_field`'s refusal.

## 2. Rust API (files per Layout)

**`rust/src/serie/spill.rs`** is new and holds the verbs, like `serie/order.rs`. It is declared with `mod spill;` in `serie.rs`.

```rust
impl Serie {
    pub fn is_spilled(&self) -> bool;
    pub fn into_spilled(&self, folder: &crate::local::LocalFolder) -> crate::arrow::Result<Self>;
    pub fn as_spilled(&mut self, folder: &crate::local::LocalFolder) -> crate::arrow::Result<&mut Self>;
    pub fn into_resident(&self) -> crate::arrow::Result<Self>;
    pub fn as_resident(&mut self) -> crate::arrow::Result<&mut Self>;
}
impl SerieReader {
    /// Drains batch by batch into one spill file; peak heap is one batch; window statics dropped.
    pub fn into_spilled(self, folder: &crate::local::LocalFolder) -> crate::arrow::Result<crate::ChunkedSerie>;
}
```

- **Run refused:** `into_spilled` refuses a Run (`require_field`).
- **Clone, no file:** `into_spilled` answers a clone and writes no file when the column is heap-free. That covers zero rows and the `null` datatype.
- **No-op resident:** `into_resident` answers a clone when nothing is mapped.

**`rust/src/chunked_serie.rs`** (AGENTS: ChunkedSerie verbs live here):

```rust
impl ChunkedSerie {
    pub fn is_spilled(&self) -> bool;                                    // any chunk
    pub fn into_spilled(&self, folder: &LocalFolder) -> crate::arrow::Result<Self>;
    pub fn as_spilled(&mut self, folder: &LocalFolder) -> crate::arrow::Result<&mut Self>;
    pub fn into_resident(&self) -> crate::arrow::Result<Self>;
    pub fn as_resident(&mut self) -> crate::arrow::Result<&mut Self>;
    pub fn into_struct_serie(&self) -> crate::Result<Self>;              // one shared root Arc for every chunk
}
```

`into_spilled` keeps heap-free chunks (empty ones included) by pointer and writes the rest, one batch per chunk, into **one** new file with one mapping. It answers a clone when there is nothing to write, and rebuilds through `from_landed(Arc::clone(&self.field), chunks)`.

**`rust/src/window_serie.rs`**:
- `WindowSerie::is_spilled(&self) -> bool` and `WindowSerieMut::is_spilled(&self) -> bool`.
- Each answers the viewed serie's value. There are no other spill verbs: a window never changes what it views.

**`rust/src/spill.rs`** is new, crate-private (`mod spill;` in `lib.rs`), and is a root implementation of its own name:

```rust
pub(crate) fn spill_columns(
    field: &Arc<Field>,
    columns: impl IntoIterator<Item = crate::arrow::Result<ArrayRef>>,
    folder: &LocalFolder,
) -> crate::arrow::Result<Vec<Serie>>;            // creates no file when `columns` is empty
pub(crate) fn holds_spill(data: &ArrayData) -> bool;   // any; no lock and no allocation while LIVE_COUNT == 0
pub(crate) fn is_heap_free(data: &ArrayData) -> bool;  // every non-empty buffer mapped
pub(crate) fn resident(data: &ArrayData) -> crate::arrow::Result<ArrayData>; // mapped buffers copied once, heap kept by pointer
struct Mapping { map: memmap2::Mmap }             // AsRef<[u8]>; new() registers (base, len); Drop unregisters, then the Mmap field unmaps
static LIVE: Mutex<BTreeMap<usize, usize>>;       // base -> len of each live mapping; poison read through PoisonError::into_inner
static LIVE_COUNT: AtomicUsize;
#[cfg(feature = "internals")] #[doc(hidden)]
pub mod internals { pub fn live() -> usize; pub fn mapped_bytes() -> usize; pub fn is_mapped(address: usize) -> bool; }
```

**`rust/src/local/folder.rs`**:

```rust
impl LocalFolder {
    /// `yggdryl-{purpose}-{pid}-{seq}`, create_new (100 attempts on AlreadyExists), read+write;
    /// Unix: mode 0o600 and unlinked at once (the fd keeps the inode);
    /// Windows: custom_flags(FILE_FLAG_DELETE_ON_CLOSE = 0x0400_0000),
    ///          share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE = 0x7).
    /// A missing folder: one create_dir_all, then the create is retried once (EAFP).
    pub(crate) fn anonymous_file(&self, purpose: &str) -> crate::Result<std::fs::File>;
}
```

**`rust/src/local/file.rs`**:
- Add `pub(crate) fn map_read_only(file: &File) -> crate::Result<memmap2::Mmap>`, defined as `map_file(file)?.make_read_only()`. **No new `unsafe`**: the module still "uses it once".
- The module doc gains one sentence: spill files are mapped read-only through the same constructor and share the SIGBUS and aliasing hazard.

**`rust/src/ipc/mod.rs`**:

```rust
pub(crate) fn encode_stream<W: std::io::Write>(
    target: W, codec: Codec, level: Level, schema: &Schema,
    batches: impl IntoIterator<Item = Result<RecordBatch>>,
) -> Result<()>;   // codec writer + StreamWriter(default IpcWriteOptions: alignment 64, V5, uncompressed) + finish
pub(crate) fn decode_stream(buffer: arrow_buffer::Buffer)
    -> impl Iterator<Item = Result<RecordBatch>>;   // StreamDecoder::new().with_require_alignment(true); finish() checked at the end
```

`overwrite_arrow_reader` becomes `encode_stream(&mut encoded, handle.codec(), options.level(), ..)` followed by `write_all_bytes`. It still stages the encoded bytes whole, so its wire bytes and its call-count pins do not move.

**`rust/src/media/merge.rs`**: `TemporaryFile` (`:319-390`) is deleted. `MergeState.spill: Option<FileWriter<std::fs::File>>` is created from `LocalFolder::temporary()?.anonymous_file("merge")?`, and `SpilledMergeReader.spilled: FileReader<std::fs::File>`. The lazy `set_index` read is kept.

**`rust/Cargo.toml`**: `bytes = "1.10.1"`. `from_owner` arrived in 1.9.0, and 1.10.1 fixes a leak in it. The lock already resolves 1.12.1.

**`rust/src/structure.rs`** (DataType, Field and the Struct value):

```rust
impl DataType {
    pub fn is_struct(&self) -> bool;                                // Field::is_struct now reads it
    pub fn into_struct_type(&self) -> crate::Result<DataType>;      // self, or struct<value: self> with `value` nullable; bounded
}
impl Field {
    pub fn into_struct_field(&self) -> crate::Result<Field>;        // self (any nullability), or required `row` whose one child is self unchanged; bounded
}
impl Scalar {
    pub fn into_struct_scalar(&self) -> Scalar;                     // Scalar::Struct as is, else Struct({"value": self})
}
```

**`rust/src/serie/structure.rs`**:

```rust
impl StructSerie { pub(crate) fn wrap(root: Arc<Field>, child: Serie) -> Self; } // nulls None, rows = child.len()
impl Serie {
    pub fn into_struct_serie(&self) -> crate::Result<Serie>;        // Struct leaf as is; Run refused; else wrap(Arc::new(field.into_struct_field()?), self.clone())
}
```

**`rust/src/media/mod.rs`**: `pub const DEFAULT_VALUE_NAME: &str = "value";` beside `DEFAULT_ROOT_NAME`.

**`rust/src/serie/arrow.rs`** (changed bodies):
- `record_root` is deleted.
- `SerieReader::root_of` keeps its signature. Its body becomes `Ok(field.into_struct_field()?.with_nullable(false))`, which produces identical output.
- `held_root` becomes `Ok(Arc::new(SerieReader::root_of(field)?))`; the bound check moved into the owner.
- `held_record`'s non-record arm becomes `StructSerie::wrap(Arc::clone(root), serie)`. Its record arm and absent-row refusal stay.
- `batch_under` reads `field.is_struct()`.
- The "absent rows, which a table cannot state" text is written once, in one crate-private fn both sites call.
- A crate-private `pub(super) fn root_arc(&self) -> &Arc<Field>` on `SerieReader` lets `into_spilled` land chunks under the reader's own root.

**Deleted:** `serie::arrow::record_root`, `excel::sheet::record_columns`, `media::merge::TemporaryFile`.

## 3. Storage, zero copy and GC

**Format.** The Arrow IPC **stream** dialect, uncompressed, at alignment 64. Reasons:
- It is the crate's one IPC dialect.
- Chunks with different dictionaries are legal replacements in it, whereas `FileWriter` refuses replacement.
- Eager decode is metadata-only, so the file format's footer and random access buy nothing.
- Compression would force a decode to the heap.

Each column is written as **one batch of one column** under `Schema::new([field.as_arrow_field_ref()?.clone()])`. A struct column's own row validity rides inside it. `batch_under`'s record flattening, which refuses absent rows, and any extra nesting level are never used, so A does not depend on B and the nesting ceiling never refuses a spill.

**Write.**
1. `folder.anonymous_file("spill")`.
2. `BufWriter<File>`.
3. `encode_stream(&mut writer, Codec::Identity, Level::DEFAULT, &schema, columns.map(|c| RecordBatch::try_new(..)))`.
4. `into_inner()`, which flushes. There is no fsync and no msync, and `LocalFile` is not used because it remaps and msyncs on flush (`local/file.rs:197-246`).

Heap cost:
- An unsliced column costs O(nodes): flatbuffers, the Identity encoder box, the `BufWriter` capacity, `ArrayData` trees.
- A sliced column adds O(slice) for re-based offsets and re-aligned bitmaps.
- No IOBase handle is involved.

**Map and decode.**
1. `local::file::map_read_only(&file)`, then drop the `File`. memmap2 duplicates the handle on Windows; Unix needs no fd.
2. `Mapping::new(mmap)` registers `(base, len)`.
3. `Buffer::from(::bytes::Bytes::from_owner(mapping))`.
4. `ipc::decode_stream(buffer)`. Each message and body is a `slice_with_length` of the mapping. The stream starts at offset 0 of a page-aligned mapping and every message is padded to 64, so alignment holds, and a violation is refused (`Error::Internal`, site `spill::decode`) rather than copied.
5. Arrow's decode-time `validate_data` runs: offsets, UTF-8 and dictionary keys, O(bytes) of CPU over pages still hot from the write, with no heap. `with_skip_validation` is rejected because it would be a second `unsafe`.

**Land.**
- `require_projection(field, first column)` once, then `Resolved::of(Arc::clone(field))` once, then `land_planned(&resolved, column, &Proof::Proven)` per batch.
- The stored `Arc<Field>` is the authority for extension names, dictionary identity and charset documents. The IPC schema is transport only and is never imported, so no dictionary-id sidecar is needed.
- **New reviewed `Proof::Proven` site.** The rows are a byte-identical reproduction of a column that already landed. They were written by this process into an anonymous 0600 file no other process can name, and re-checked structurally by Arrow's decode. A spill only ever receives columns a caller holds, never an `OnRead` column, which by contract never leaves its reader.
- `resident`'s re-landing is a second listed site: a copy of a landed column.

**Lifetime.**
- **Unix:** the file is unlinked at creation. A crash or SIGKILL leaves nothing behind, nobody can reopen or truncate the file by name, and the kernel frees its blocks at the last unmap.
- **Windows:** `FILE_FLAG_DELETE_ON_CLOSE` deletes the file when the last handle closes, which is memmap2's duplicate, closed after `UnmapViewOfFile`.
- **Drop order** in `Mapping`: unregister (the `Drop` body), then unmap (the `Mmap` field), then the kernel deletes. Removing the entry before munmap is load-bearing: it rules out address reuse in the registry.

**Registry.** `holds_spill` and `is_heap_free` walk `ArrayData`: `buffers()`, `nulls().map(NullBuffer::buffer)`, and `child_data()` recursively. Each non-empty `Buffer::data_ptr()` is matched exactly against the keys of `LIVE`. The match is exact through slices, clones and copy-on-write, because a live entry's range cannot also hold a heap allocation. The `LIVE_COUNT == 0` fast path means no lock and no allocation.

**Resident copy.** `resident` rebuilds the `ArrayData` recursively:
- each mapped buffer is copied with `Buffer::from_slice_ref(buffer.as_slice())` (64-aligned, keeping the `ArrayData` offset);
- each mapped `NullBuffer` is copied as `BooleanBuffer::new(copied, offset, len)`;
- heap buffers are kept by pointer;
- `ArrayDataBuilder::build()` validates once, and the result lands with `Proof::Proven`.

There is no IPC round trip and no dependence on allocator alignment.

**Location.**
- Rust takes `&LocalFolder` explicitly, so `LocalFolder::temporary()?` is the caller's visible choice.
- The bindings default to `LocalFolder.temporary()`.
- No environment variable: AGENTS reserves environment reads for the `LocalFolder` roots.
- `std::env::temp_dir()` is often tmpfs, which relieves no RAM; the docs say so.
- Each `into_spilled` call is one file and one mapping. A ChunkedSerie or reader spill is one mapping in all, which keeps `vm.max_map_count` (65530 by default) in mind.

**Accounting.** `memory_size` is unchanged and counts mapped bytes where the slice reaches them. A resident-only size is an open decision (§12).

## 4. Verbs on a spilled serie

| Verb | On a spilled column |
| --- | --- |
| `scalar`, `get`, `rows`, `iter`, leaf `values` / `value(i)` / `offsets` / `nulls`, equality, hash, `Display`, serde, digests | read the mapping; zero heap per row, at most a page fault |
| `as_<leaf>`, `get_<leaf>_mut` (borrow) | unchanged |
| `clone`, `window`, `window_mut`, `Scalar::from(serie)`, `as_serie` | pointer moves; stays spilled and keeps the file |
| `slice` | primitive, bytes, struct: Arrow slice, stays spilled; serie, list and map leaves restate offsets on the heap (`sequence.rs:317,609`, `mapping.rs:231`) while values stay mapped (`is_spilled` stays true) |
| `into_arrow_array` / `_batch` / `_reader` / `_scalar`, `SerieReader::from_serie` / `from_chunked`, `IOMedia::write_arrow` | mapped buffers handed out zero copy; a writer reads the mapped pages |
| `__arrow_c_array__` / PyCapsule (Python) | zero copy; the release callback owns the mapping |
| Node crossing | copied IPC, as always |
| `cast` to the same field / identity plan | the same buffers; stays spilled |
| `cast` otherwise, `into_sorted` / `unique` / `reversed` / `taken` / `filtered`, `partition_by` | new heap buffers, except where a kernel answers its input (identity); the source stays spilled |
| `set`, `push`, `insert`, `remove`, `pop`, `truncate`, `clear`, `extend`, `extend_from_serie`, `resize`, `set_child`, `set_cell`, `splice`, `as_sorted` / `unique` / `reversed` / `taken` / `filtered` | copy the touched leaf to the heap once (copy on write); untouched leaves stay mapped |
| `into_struct_serie` | zero copy wrap; stays spilled |
| `ChunkedSerie` joins: `into_serie`, `sort_indices`, `into_sorted`, `into_unique`, `into_taken`, `is_unique` | concat on the heap, as today; documented |
| `ChunkedSerie` per-chunk verbs: `window_by`, `into_filtered`, `into_reversed`, `partition_by`, `is_sorted` | keep mapped pieces where a chunk alone answers |
| `memory_size` | counts mapped bytes (unchanged definition) |

## 5. Struct conversions

**Spelling (decided by the user).** Only `into_struct_type`, `into_struct_field`, `into_struct_scalar` and `into_struct_serie` (plus `ChunkedSerie::into_struct_serie`): each answers self when already a struct, else the one-child wrap. There is no `as_struct_*` conversion: `as_<noun>` stays reserved for dedicated borrowed views, and the borrowed "is it a struct" half is the existing narrowings (`DataType::as_fields`, `StructField::from_field`, `Serie::as_struct`, `Scalar::as_struct`) plus `DataType::is_struct`.
- The suffix names the root being converted, which keeps them apart from the unsuffixed leaf narrowing `Serie::as_struct`. The docs put both in one table.
- AGENTS' Public vocabulary gains a row: `into_struct_<root>`, self when already a struct, else the one-child wrap.

| Root | Already a struct | Otherwise | Borrowed "is it" (unchanged) |
| --- | --- | --- | --- |
| `DataType` | itself | `struct<value: self>`, `value` nullable (as `DataType::shared_field` does, `typed.rs:351`), bounded | `as_fields()`, `is_struct()` (new) |
| `Field` | itself, nullability, metadata and dictionary id kept | `Field` named `row` (`DEFAULT_ROOT_NAME`), required, no metadata, one child = self unchanged; bounded. This is today's `record_root` exactly | `StructField::from_field`, `is_struct` |
| `Scalar` | `Scalar::Struct` itself | `Scalar::Struct({"value": self})`, the named input shape that `Field::scalar` canonicalizes against the wrapped field; a null becomes `{value: null}` (hence the nullable child); a positional run is a value, not a row, so it is wrapped. Infallible | `Scalar::as_struct` |
| `Serie` | the Struct leaf, absent rows kept | `StructSerie::wrap(Arc::new(field.into_struct_field()?), self.clone())`: one child, no row validity, nothing copied or read; Run refused | `Serie::as_struct` |
| `ChunkedSerie` | itself | every chunk wrapped under one shared root `Arc` | per chunk `as_struct` |

**Table rule stays separate.** `SerieReader::root_of` keeps forcing a required root (`with_nullable(false)`). `held_record` keeps refusing a record column with absent rows. Two readings exist there (children as columns, or one struct column), so the refusal stands and names the remedy: `into_struct_serie` wrapped once more.

**Rerouted in this change** (each exactly equivalent, and the replaced code is deleted):

1. `serie/arrow.rs` `record_root` (`:1105`) is deleted; it becomes the wrap branch of `Field::into_struct_field`.
2. `SerieReader::root_of` (`:1694`) becomes `Ok(field.into_struct_field()?.with_nullable(false))`. Every caller inherits it unchanged:
   - `order.rs` `window_key`, `chunked_serie.rs` `window_by` and `into_arrow_reader`, `batch_schema`, `SerieReader::cast`;
   - `python/src/serie.rs:601-603` (`batch_root`), `python/src/chunked_serie.rs:659`, `node/src/serie.rs:673`, `node/src/chunked_serie.rs:381`.

   Any path that previously failed later at plan compile or schema export for a wrap past the ceiling now fails here, through `validate_bounded`. If a pinned text moves, it is re-pinned with that reason.
3. `held_root` (`:1496`) becomes `Arc::new(root_of(field)?)`. It keeps one shared root per reader, so the chunked and reader allocation pins hold.
4. `held_record` (`:1504`): the non-record arm becomes `StructSerie::wrap(Arc::clone(root), serie)`.
5. `batch_under` (`:1129`) reads `is_struct()`, and the two absent-row refusals share one constructor.
6. `excel/sheet.rs:909-923` `record_columns` is deleted. Callers use `SerieReader::root_of(field)?` and `serie.into_struct_serie()?`, whose `as_struct()` children are taken; `None` is `Error::Internal` and is unreachable by construction.
7. `avro/arrow.rs:39-45` becomes `Field::new(DEFAULT_VALUE_NAME, dtype, nullable).into_struct_field()?.with_name(root_name)`. A non-record Avro node never maps to a struct, so the output is unchanged.
8. The `"value"` literals become `DEFAULT_VALUE_NAME`, nullability unchanged: `typed.rs:317` `SHARED_NAME` (deleted), `media/inference.rs:70`, `serie/arrow.rs:1098` (`default_dtype_array` stays required), `avro/arrow.rs:40`.
9. The `"row"` literals become `DEFAULT_ROOT_NAME`:
   - Rust: `datatype.rs:1669` (`DataType::into_arrow_schema` keeps its refusal of a non-struct), `parquet/mod.rs:610`, `arrow/mod.rs:587-588`, `media/inference.rs:133`, `iceberg/metadata.rs:1122,1127`.
   - Python: `python/src/serie.rs:598` (the `DEFAULT_ROOT` const is deleted), `python/src/field.rs:104`, `python/src/iomedia.rs:1188,1304,1334`, `python/src/scalar.rs:1165`.
   - `text/batch.rs:183` is an alias list and stays as it is.

**Deliberately not rerouted** (left to the dedup review or a separate decision):
- The expression roots (`expression/plan.rs:1047`, `selector.rs:781`, `filter.rs:191`, `records.rs:148`) stay record-only.
- `arrow_schema_from_field` (`arrow/mod.rs:312`) and `ArrowCastPlan::compile_schema` (`cast.rs:587`) keep refusing.
- `DataType::into_arrow_schema` / `Field::into_arrow_schema` stay non-widened.
- Multi-child roots: `serie/order.rs:1474` `record_of` (`key`), `xxhash/arrow.rs:507`, `excel/sheet.rs:680`, `xmla/definitions.rs:140`, `xmla/rowset.rs:758`, `python/src/iceberg.rs:234`, `node/src/iceberg.rs:89`.
- `field.dtype().as_fields().is_some()` in `cast.rs:500,598`, `arrow/mod.rs:320` and `iceberg/statistics.rs:231` becomes `Field::is_struct`.

## 6. Bindings

Both bindings only coerce and redirect. The `folder` argument meets one boundary and is resolved once into the core `LocalFolder`.

### Python (§3)

| Class | Spill | Struct |
| --- | --- | --- |
| `Serie`, `ChunkedSerie` | `is_spilled()`; `into_spilled(folder=...)`; `as_spilled(folder=...) -> Self`; `into_resident()`; `as_resident() -> Self` | `into_struct_serie()` |
| `WindowSerie` (and the mutable window class where one is bound) | `is_spilled()` | |
| `SerieReader` | `into_spilled(folder=...) -> ChunkedSerie` | |
| `DataType` | | `is_struct()`, `into_struct_type()` |
| `Field` (already mutable, e.g. `set_name`) | | `into_struct_field()` |
| `Scalar` | | `into_struct_scalar()` |

- **`folder` intake** accepts `LocalFolder`, a `LocalPath` taken as a directory, `str`, `os.PathLike`, or a `file:` `Url`. It goes through the binding's existing holder coercion (`python/src/holder/handles.rs:175`). Omitted (`...`) or `None` means `LocalFolder.temporary()`. Any other holder raises `ValueError` naming its scheme.
- **Off the GIL:** spill and resident run off the GIL, as the other I/O doors do.
- **Zero-copy export:** a C Data / PyCapsule export of a spilled column is zero copy. A pyarrow array that outlives the `Serie` keeps the mapping alive.
- **No rename (the user's decision): `Scalar.into_struct_field` keeps its name and handles both readings transparently, with no ambiguity between them:**
  - Rows of named records (today's domain, `Scalar::inferred_struct_field`) answer the inferred struct root, exactly as today.
  - Any other value answers `inferred_scalar_field()?.into_struct_field()`: a struct value answers its own struct field, and anything else is wrapped as `struct<value: ...>` under the generic rule.
  - Both arms are one core reading: a crate-level `Scalar::inferred_record_field` (or the body of `inferred_struct_field` widened the same way). The binding only redirects.
  - `Scalar.into_array_field` and `Scalar.into_field` stay unchanged.
  - Sweep `_native.pyi`, `typing_bindings.py`, `test_scalar.py`, `docs/types/scalar.md` and `.api-bindings.txt:66` for the widened contract, adding tests for a non-record value that is now wrapped.

### Node (§4)

| Class | Spill | Struct |
| --- | --- | --- |
| `Serie`, `ChunkedSerie` | `isSpilled()`, `intoSpilled(folder?)`, `asSpilled(folder?)`, `intoResident()`, `asResident()` | `intoStructSerie()` |
| `WindowSerie` | `isSpilled()` | |
| `SerieReader` | `intoSpilled(folder?) -> ChunkedSerie` | |
| `DataType` | | `isStruct()`, `intoStructType()` |
| `Field` (already mutable, e.g. `setField`) | | `intoStructField()` |
| `Scalar` | | `intoStructScalar()` |

- **In-place wrappers** return `this`. They follow `asSorted`'s `_asSortedNative` pattern in `node/binding.js` / `binding.d.ts`.
- **`folder` intake** is a path string or a `file:` URL string, resolved through `Holder::from_url` (`node/src/iobase.rs:112`) and required to be local. `undefined` is skipped; `null` clears. Both mean temporary.
- **No zero-copy claim** for Node: the crossing is copied IPC.
- **No rename (the user's decision):** the Scalar instance method `intoStructField` keeps its name and handles both readings, exactly as Python's does (named record rows answer the inferred root; any other value its own field through the generic rule). `intoArrayField` is unchanged.
  - The record-class static getter `intoStructField` (`.api-bindings.txt:400`, `node/README.md:174`) keeps its name too, since its meaning is the new verb's.
  - Tests in `text/codec.test.js` and `*.types.ts` cover the widened contract.
- **Regenerate** `node/index.js` / `index.d.ts` with `npm run --prefix node build:debug`.

### Both
- Parity tests mirror the sources (§7).
- Boundary benchmarks (§9).
- `.api-bindings.txt` gains the new names and the renames.

## 7. Tests (mirror files; refusals first)

### `rust/tests/root/spill.rs` (new; mirrors `rust/src/spill.rs`; harness `rust/tests/root.rs`)
- Refusals:
  - a folder path under a regular file → `Error::Io` naming the folder, and `live()` unchanged;
  - an empty column iterator creates no file.
- `#[cfg(feature = "internals")] mod internal`:
  - `live()` rises by one per spill call;
  - `live()` stays at one while any of these lives after the source drops: a clone, a slice, a window, a chunk, `Scalar::from(serie)`, an `into_arrow_array` result;
  - `live()` returns to zero after the last one drops;
  - `is_mapped(addr)` holds for every buffer address a spilled column reaches;
  - `mapped_bytes()` of a sliced column's spill is smaller than the whole column's.
- `#[cfg(target_os = "linux")]`:
  - `/proc/self/maps` shows exactly one `(deleted)` mapping under the test folder per live spill, and zero after the last drop;
  - the folder lists no entry right after `into_spilled` (Unix).

### `rust/tests/local/folder.rs` and `local/file.rs` (internals forwards)
- `anonymous_file` creates a missing folder once, leaves no directory entry on Unix, and refuses a folder that is a file, naming it.
- `map_read_only` maps exactly what was written.

### `rust/tests/ipc/mod_.rs`
- `overwrite_arrow_reader` bytes are unchanged against a direct `StreamWriter` into a `Vec`.
- internal: `decode_stream` over a heap `Buffer` is zero copy (value pointers lie inside it);
- internal: a buffer offset by one byte is refused, not copied.

### `rust/tests/media/merge.rs`
- A merge large enough to spill answers the same rows.
- No `yggdryl-merge-*` entry remains in the temporary folder during or after the merge.

### `rust/tests/serie/spill.rs` (new; mirrors `serie/spill.rs`; harness `rust/tests/serie.rs`)

Refusals first:
- A Run is refused by name, and `self` is unchanged.
- `as_spilled` into an unwritable folder leaves `self` equal and `is_spilled() == false`.
- A zero-row column and a `null`-type column answer a clone, write no file, and keep `is_spilled() == false`.

Round-trip matrix: `into_spilled(..)? == source`, `is_spilled()`, `Arc::ptr_eq` on `field_ref`, and the field unchanged (metadata, extension name, dictionary id). Cases:

| Group | Layouts |
| --- | --- |
| Numbers and booleans | every primitive width; float16; boolean; null |
| Decimals | decimal32/64/128/256; the fixed `Decimal` and `BigDecimal` |
| Temporal | every temporal, with and without a zone; every interval layout |
| Text and bytes | the 18 string leaves (sized and fixed ascii and cp1252 included); the 6 byte leaves |
| Identities and enums | uuid; a code (`ccy`); `state`; `version`; `timezone`; a dictionary enum, and one under a serie |
| Nested | the five serie layouts, including a zero-width fixed-size serie; a sorted map; a nullable struct with absent rows holding a decimal child (the re-mask case, `is_spilled` still true) |
| Encodings | dense and sparse unions with a null row; run-end; variant; geometry |
| Shape | a sliced column; zero rows |

A layout the IPC stream cannot carry back is refused before mapping, by name, and never read back lossily. The matrix is red before it is green.

Further behaviour:
- **Idempotence:** `into_spilled` of a heap-free spilled column is a clone, and `live()` is unchanged.
- **Copy on write:**
  - `push` / `set` on a spilled int64 → value correct, `is_spilled() == false`, an earlier clone still spilled;
  - `set_cell` on one child of a spilled struct → `is_spilled()` stays true.
- **Resident:** `into_resident` → equal, `is_spilled() == false`, and `live()` back to zero after the spilled value drops.
- **Reader:** `SerieReader::into_spilled` over 1000 batches → 1000 chunks equal to the source, one mapping, and a window sub-reader's statics are absent.

### `rust/tests/root/chunked_serie.rs`
- One mapping per `into_spilled`, whatever the chunk count.
- Heap-free chunks are kept by pointer while the rest go to a new mapping.
- Chunks with different dictionaries round-trip.
- `IOMedia::write_arrow(SerieReader::from_chunked(spilled))` to IPC and Parquet buffers is byte-equal to the heap version.
- `into_struct_serie` shares one root (`Arc::ptr_eq`) across chunks.
- `into_resident`.

### `rust/tests/root/window_serie.rs`
- `WindowSerie::is_spilled` and `WindowSerieMut::is_spilled` forward the serie's answer, including on a `window_by` window that carries an origin.

### `rust/tests/root/structure.rs`
- **`into_struct_type`:** int64 → `struct<value: int64>` with a nullable child; a struct answers itself (`shares_storage_with`).
- **`into_struct_field`:**
  - the child keeps its name, nullability and metadata;
  - the wrapper is a required `row`;
  - a nullable struct answers itself, still nullable;
  - at `PARSE_RECURSION_LIMIT` it is refused, naming the field and the limit.
- **`into_struct_scalar`:** `Int64(5)` → `{value: 5}`; `Null` → `{value: null}`; `Scalar::Struct` answers itself; the canonicalization `Field::scalar(wrapped)` holds under `into_struct_type()`'s field.
- **Predicate:** `DataType::is_struct`.

### `rust/tests/serie/structure.rs`
- `into_struct_serie` is zero copy: the child's `into_arrow_array` buffers are `Arc::ptr_eq` with the source.
- A struct with absent rows answers itself, validity kept.
- A Run is refused.

### Unchanged suites prove the reroute
- `rust/tests/serie/arrow.rs`: the `root_of`, `from_serie`, `from_chunked`, `into_arrow_batch` and held-record shape and refusal tests.
- The excel and avro suites.

### Bindings
- **Python** (`test_serie.py`, `test_chunked_serie.py`, `test_window_serie.py`, `test_datatype.py`, `test_field.py`, `test_scalar.py`, `typing_bindings.py`):
  - every verb;
  - the `folder` spellings and the scheme refusal;
  - a pyarrow array reads correctly after `del serie; gc.collect()`;
  - the renamed `inferred_*` doors;
  - `mypy --strict`.
- **Node** (`serie.test.js`, `chunked_serie.test.js`, `window_serie.test.js`, `datatype.test.js`, `field.test.js`, `text/codec.test.js`, their `.types.ts`): the same cases, plus `tsc --noEmit`.

## 8. Pins

**IOBase calls.**
- None added: a spill takes no IOBase handle. The writer is a `std::fs::File` the crate owns, and reads cost zero `pread` calls.
- Unchanged and re-run by their filters: `rust/tests/iobase_calls.rs` `ipc_costs` (`:748`) and `windowing_a_read_adds_no_call` (`:766`), after the `encode_stream` extraction.
- A count that moves is a defect, never a re-pin.

**Allocations** (`rust/tests/allocations.rs`; each at 4,096 and 262,144 rows, through `counted` / `peaked`):

| Row | Claim |
| --- | --- |
| `into_spilled_allocates_no_row` | count and peak equal at both sizes for unsliced int64, utf8 and `struct<serie<int32>>` with no absent parents |
| `a_spilled_read_costs_what_a_heap_read_costs` | `scalar(i)` and leaf `value(i)` counts equal the heap column's |
| `is_spilled_allocates_nothing_while_no_spill_lives` | 0; with a live spill, flat across sizes (O(nodes)) |
| `a_reader_spills_holding_one_batch` | `peaked` equal for 10 and 1000 equal batches |
| `a_spill_under_absent_parents_costs_one_bitmap_per_remasked_child` | the non-contract re-mask stated exactly; contract leaves add 0 |
| `into_resident_copies_each_mapped_buffer_once` | allocated bytes = mapped buffer bytes + O(nodes) |
| `into_struct_on_a_struct_costs_a_clone` | `into_struct_type`, `_field` and `_serie` on a struct cost exactly what `clone` costs (a Serie: 0) |
| `a_struct_wrap_costs_one_root_and_never_a_row` | flat across sizes for DataType, Field, Scalar and Serie |
| `a_chunked_struct_wrap_shares_one_root` | the root is allocated once for 1 and for 100 chunks |

The existing `SerieReader::from_serie` / `from_chunked` / `ChunkedSerie::from_series` pins (around `allocations.rs:4493-4534`, `:4620`, `:4766`) and the serie and window verb rows (`:2468`, `:2684`) do not move.

The counting allocator cannot see a mapping. Zero copy is proved by `internals::is_mapped`, `Arc::ptr_eq`, and heap that stays flat across sizes.

## 9. Benchmarks

- **`rust/benchmarks/arrow.rs`**, group `serie_spill`, at the `bench_profile` corpus sizes:
  - `into_spilled` throughput for int64, utf8 and a struct;
  - time to first row and a full scan, spilled against heap;
  - `into_resident`;
  - `into_struct_serie` in ns.
  - The baseline is a heap clone and an arrow-ipc `StreamWriter`-to-`Vec` plus `StreamReader` round trip on the same payload.
  - Smoke with `cargo bench -p yggdryl --bench arrow -- serie_spill --quick`. Page numbers come only from the release run.
- **`python/benchmarks/arrow.py`**: spill round trip and full drain, against `pa.ipc.new_stream` + `pa.memory_map` + `pa.ipc.open_stream(..).read_all()` over the same stream.
- **Node:** a spill row in `npm run --prefix node bench:types`.

## 10. Docs and skills

- **`docs/types/serie.md`:**
  - a "Spilled series" section, in this order: contract (a state, not a type; `is_spilled` means "some buffer"; copy on write; garbage collection at the last view) → a Rust/Python/JavaScript example asserting equality and `is_spilled` → edges (tmpfs; `vm.max_map_count`; the SIGBUS and aliasing hazard; Node copies; `memory_size` counts mapped bytes; one view pins the whole file) → the §4 verb table → Performance from the release bench;
  - an "As a struct" section with the §5 table and the relation to `Serie::as_struct` and `SerieReader::root_of`.
- **`docs/types/chunked-serie.md`:**
  - the larger-than-RAM recipe: `read_arrow` → `SerieReader::into_spilled` → `window_by` / `into_filtered` / `partition_by` → `write_arrow(SerieReader::from_chunked(..))`;
  - which verbs join on the heap;
  - one mapping per call;
  - `into_struct_serie`.
- **`docs/types/window-serie.md`:** `is_spilled`. **`docs/arrow/readers.md`:** `SerieReader::into_spilled`, batch by batch, statics dropped.
- **`docs/types/datatype.md`, `field.md`, `scalar.md`:**
  - the struct pair;
  - `DataType::is_struct`;
  - the child names (`value` / the field's own) and the required `row` wrapper;
  - the bound refusal;
  - the Rust-only in-place forms;
  - the renamed Scalar inference doors, set beside the new verbs.
- **`docs/holder/index.md`, Local section:** anonymous spill files (unlinked on Unix, delete-on-close on Windows), the read-only mapping through the one unsafe constructor, and tmpfs.
- **`docs/media/index.md`, IPC section:** one sentence: a spill rides the stream dialect, uncompressed, at alignment 64.
- **`docs/benchmarks.md`:** index the new table.
- **`skills/yggdryl-arrow`:**
  - `SKILL.md` door-table rows: spill, resident, is-spilled, wrap a column as a record;
  - pitfalls: writes un-spill; joins land on the heap; Node copies;
  - recipes in `references/rust.md`, `python.md`, `javascript.md`.
- **`skills/yggdryl-types`:** the struct pair, and the `inferred_*` rename (`SKILL.md`, `references/javascript.md`). Also `skills/yggdryl` if its entry table lists Serie doors.
- **`AGENTS.md`:**
  - Layout rows: root `spill.rs`, `serie/spill.rs` in the serie row, `anonymous_file` under local;
  - the `local/file.rs` note that the spill reuses the one unsafe constructor read-only;
  - the Zero copy "Holds" bullet: a spilled column's buffers are slices of one read-only mapping, freed with the last view;
  - "Serie is the collection" rows: "a column held on disk", "any column as a record";
  - the Proof::Proven list: a spill's read-back, and a resident copy;
  - the Public vocabulary row for `into_struct_<root>` and the spill and resident pairs;
  - the Windows CI leg in §2's job table.
- **Inventories:**
  - `.api-inventory.txt` sections: serie (`:5602`), chunked_serie, window_serie (`:4236`), structure (`:5194` / `:5277`), media (`:3346`, `DEFAULT_VALUE_NAME`), the `root_of` text;
  - `.api-bindings.txt`;
  - `python scripts/check_api_inventory.py`.

## 11. Phases (disjoint file sets; one commit at the end)

| Phase | Starts | Files (owned by exactly one worker) | Settle |
| --- | --- | --- | --- |
| P0 gate | — | Task #14 committed; re-read every anchor | — |
| A1 local | any time (may use a worktree off `d7c4d93`, merged after #14) | `local/file.rs`, `local/folder.rs`, `media/merge.rs`; tests `local/file.rs`, `local/folder.rs`, `media/merge.rs` | `cargo check -p yggdryl --all-targets`; `cargo test -p yggdryl --test local --features internals`; `--test media merge` |
| A2 ipc | any time | `ipc/mod.rs`; test `ipc/mod_.rs` | `--test ipc`; `--test iobase_calls ipc` (unchanged) |
| B1 types + literal sweep | any time | `structure.rs`, `datatype.rs`, `media/mod.rs`, `typed.rs`, `media/inference.rs`, `avro/arrow.rs`, `parquet/mod.rs`, `arrow/mod.rs`, `iceberg/metadata.rs`; test `root/structure.rs` | `--test root structure`; `--test avro`; `--test media inference`; `--test parquet`; `--test iceberg metadata` |
| A3 spill owner | after A1 and A2 | `spill.rs`, `lib.rs` (`mod spill;` + `python scripts/generate_internals.py`), `rust/Cargo.toml`; tests `root/spill.rs`, `root.rs` | `--test root spill --features internals` |
| B2 serie struct | after #14 and B1 | `serie/structure.rs`, `serie/arrow.rs`, `excel/sheet.rs`; tests `serie/structure.rs` (+ `serie/arrow.rs` unchanged) | `--test serie structure`; `--test serie arrow`; `--test excel` |
| A4 serie spill | after A3 and B2 | `serie.rs` (`mod spill;`), `serie/spill.rs`, `chunked_serie.rs` (spill + struct verbs), `window_serie.rs`; tests `serie/spill.rs`, `serie.rs`, `root/chunked_serie.rs`, `root/window_serie.rs`, `allocations.rs`; `benchmarks/arrow.rs` | refusal smoke first, then the matrix: `--test serie spill`; `--test root chunked_serie window_serie`; `--test allocations spill struct`; `cargo bench --bench arrow -- serie_spill --quick` |
| W whole run | after A4, in one background chain | — | inventories by hand + `check_api_inventory.py`; `cargo test --all-targets --all-features --no-fail-fast`; clippy `-D warnings` (crate and workspace); `cargo test --doc`; `generate_internals.py --check`. Re-pin only a ceiling-refusal text that moved, with its reason; never a cost pin |
| P Python | after W | `python/src/{serie,chunked_serie,window_serie,datatype,field,scalar,iomedia,iceberg}.rs`, `_native.pyi`, tests, `python/benchmarks/arrow.py` | `maturin develop`, then pytest per file; `mypy --strict` |
| N Node | after P | `node/src/{serie,chunked_serie,window_serie,datatype,field}.rs`, `text/codec.rs`, `binding.js` / `.d.ts`, regenerated `index.*`, tests, bench | `build:debug`; `node --test` per file; `test:package:debug`; `npm test`; docs manifests `--check` |
| D docs | after N | §10 pages, skills, `AGENTS.md`, `.api-*.txt`, `.github/workflows/ci.yml` (Windows leg) | `mkdocs build --strict`; `check_docs_examples.py --lang rust` / `python` / `javascript` |
| Ship | | one commit, push, read CI per job | report the skipped local-only checks: release benches; Windows behaviour if no leg is added |

Worker rules: no worker runs `cargo fmt --all` while others edit, and all edits are scripted exact-string edits, each anchor asserted once.

## 12. Open decisions (defaults stated)

1. **Predicate spelling.** Default `is_spilled` (AGENTS `is_*`), not the user's bare `spilled`.
2. **SpilledSerie type.** Default: none; spilled is a state of `Serie` / `ChunkedSerie`. The alternative, a public handle that mirrors every verb, is rejected.
3. **Struct conversion names. DECIDED by the user ("Remove the as_ staying with as_<noun> for dedicated borrowed"):** `into_struct_*` only; no `as_struct_*` conversion anywhere, in Rust or the bindings.
4. **Borrowed "is it a struct".** Default: no new `as_struct` narrowings on DataType or Field; the existing narrowings plus `DataType::is_struct` answer it.
5. **Already-struct nullability.** Default: answered as itself (nullable kept, absent rows kept). `root_of` alone forces required.
6. **Wrap child.** Default: `value`, nullable, for DataType and Scalar; the field's own name for Field and Serie; a required `row` root for Field and Serie.
7. **Scalar wrap shape.** Default: `Scalar::Struct({value: self})`, the named input shape, not an ordered run (a run is a struct only beside its field).
8. **Binding inference doors. DECIDED by the user ("Why inferred? Keep it and adapt transparently handling both"):** no rename. `Scalar.into_struct_field` / `intoStructField` keep their names and read both: named record rows answer the inferred struct root as today; any other value answers its inferred field through the generic `into_struct_field` (itself when a struct, else wrapped). `into_array_field` / `intoArrayField` and `into_field` are unchanged.
9. **Folder.** Default: an explicit `&LocalFolder` in Rust, temporary in the bindings, no environment variable.
10. **Format.** Default: IPC stream, one file per call. Rejected: the file format with a footer (refuses dictionary replacement) and one file per chunk (more mappings).
11. **Crash safety.** Default: anonymous file (unlinked at create on Unix, delete-on-close on Windows). Rejected: a named file removed in `Drop`.
12. **Writes to a spilled column.** Default: silent copy on write, documented. Rejected: a named refusal.
13. **Reverse verbs.** Default: include `into_resident` / `as_resident`.
14. **`SerieReader::into_spilled` return type.** Default: `ChunkedSerie` (re-readable, windowable, no edit to the reader's `Source`). A lazy spilled reader is a follow-up.
15. **Size answer.** Default: `memory_size` unchanged; a resident-only size answer is a follow-up.
16. **Decode validation.** Default: kept. `with_skip_validation` is rejected because it would be a second `unsafe`.
17. **Unsafe budget.** Default: zero new `unsafe` (`map_file` + `make_read_only`).
18. **Windows proof. DECIDED by the user ("Add windows too"):** add a narrow `windows-latest` job to `.github/workflows/ci.yml` running `cargo test --locked -p yggdryl --test root spill`, `--test serie spill` and `--test media merge` (default features), so the anonymous file's delete-on-close and the read-only mapping are proven on Windows; AGENTS.md §2's job table gains its row.
19. **Re-mask under absent parents.** Default: unchanged in this change. A follow-up in `child_of`: skip the re-mask when the child's validity already covers the parent's absence (one word-wise AND, no allocation). This would make every spilled column heap-free.
20. **Automatic spilling** by byte threshold or budget, and an IOBase "lend a Buffer" hook so the IPC medium reads `LocalFile` zero copy: default not in this change.
21. **Expression roots and `into_arrow_schema` widening.** Default: unchanged, still refusing.
22. **`bytes` floor.** Default `1.10.1`.
23. **Found defect.** `.arrow`, `.feather` and `.ipc` (ARROW_FILE) read and write the stream dialect (`media/mod.rs:116`, `media/options.rs:1807,1862`). Default: a separate task with pyarrow interop in both directions.

## 13. Overlap with the dedup review (C)

**Lands here, so the review prompt excludes it:**
- the single temporary-file owner (`anonymous_file`, merge's `TemporaryFile` deleted);
- the single struct-wrap owner (`record_root`, `held_record`'s wrap, excel `record_columns`, avro's root);
- `DEFAULT_VALUE_NAME` and the `"row"` / `"value"` literals in the files listed in §5;
- the absent-row refusal text written once;
- the IPC encode door shared by the medium and the spill.

**Left to the review:**
- `iceberg/staging.rs` folded onto the temporary-file owner;
- multi-child record constructors (`record_of`, `level_root`, the xmla, excel and iceberg roots);
- `as_fields().is_some()` → `is_struct()` in `cast.rs`, `arrow/mod.rs` and `iceberg/statistics.rs`;
- the expression-root decision;
- the `child_of` re-mask skip (decision 19).