### D39 - a leaving medium keeps its rank, and what every S6 move does the same way

**The rank.** `RecordOptions` hashes and orders by the codec's rank, so the
`s2_pins` hashes and the order pin are the rank's. `media::codec` gains
`RESERVED_RANKS: [(&str, u8); 4] = [("parquet", 1), ("avro", 2), ("xmla", 4),
("excel", 6)]` - the ranks the core's leaving media hold, a wire contract that
never moves, the shape of `MarketDescriptor::RESERVED_*` - and `claim` admits a
codec whose `(name, rank)` is one of those pairs, refuses a reserved rank under
another name or a reserved name at another rank, and holds every other medium
at or above `EXTERNAL_RANK`; the `CORE` refusal stays. So `yggdryl-parquet`'s
`install()` claims rank 1 as the core did, and every pinned hash, the order pin
(`the_media_order_by_their_rank`, `media_register.rs`'s expected order) and
the rank-1 assertion on the media page are byte-identical through the moves.

**The door.** `implementer.rs` grows once for every media move - the items the
four maps list (`moves_map/{avro,parquet,excel,xmla,iceberg}.md`, section 2:
about 18 for Avro, 15 for Parquet, 24 for Excel, Iceberg's 46 path items and
the methods reached by call) - by S3's routes: raise-and-re-export inside a
crate-private module, a forwarder, a free function over a `pub(crate)`
inherent method, a move, an exported macro. The eight shared medium helpers
(`iobase::transfer::{overwrite_arrow_reader_default_with_field,
append_arrow_reader_default, merge_arrow_reader_default, leaf_writer}`,
`iomedia::{own_options, container_field, container_row_size,
dimension_options}`) are one addition serving every move. What Iceberg
reaches of Avro (the container header, `Cursor`, `DatumCodec`, `parse_header*`,
`MAGIC`, `Blocks::metadata_bytes`) and of Parquet (`ParquetOptions`,
`read_batch_reader_with`, `load_metadata`, `schema_from_metadata`,
`overwrite_buffered`, `READ_AHEAD_BATCHES`, `WHOLE_READ_BYTES`,
`WRITE_BUFFER_BYTES`, `FileStatistics`) is `pub` in those crates under a
`#[doc(hidden)] pub mod implementer` of their own, the same door one level
down.

**The Iceberg view.** `Field::as_iceberg`/`as_iceberg_mut` and the inherent
`impl IcebergField<'_>` leave the core as the FIX view did (D9):
`protocol_field_types!` builds `IcebergField`/`IcebergFieldMut` in
`yggdryl-iceberg`, `IcebergField::new(&field)` the one spelling, the sites
(about a hundred, in tests, benchmarks, pages and skills) swept by one script.
`Transform` and the Iceberg type-string spelling (`iceberg/types.rs`,
`PrimitiveType`) stay the core's (D17).

**The tests.** A core test that builds a leaving crate's object (an
`IcebergTable` in `isin_registry/store.rs`, `warehouse/*`, `graph/serve.rs`,
`fix/schema.rs`, `s3/mod_.rs`; an Avro or Parquet handle in about fifteen
files) cannot dev-depend on that crate (a cycle), so it moves to the crate's
own `rust/<name>/tests/`, its `//!` line naming the core file it pins and the
crate it needs; `tests/allocations.rs`' Iceberg block moves with its own
counting allocator; `FakeS3` and `support/excel_package.rs` stay under
`rust/tests/support/` and are `#[path]`-included where used.

**`install()` at init.** The Python `_native` module init, the Node addon init
and the CLI's `main` call `yggdryl_<crate>::install()` once per linked crate,
in dependency order; a pure-Rust caller installs what it links, and a handle
whose medium no crate claimed is refused naming the crate, as today. The
core's seed claims (`media/codec.rs`, `media/format.rs`,
`warehouse/catalog.rs`, `holder/locator.rs`) go with each move. Avro's
`snap` and the `snappy` gates leave the core's `parquet` feature with the
crate (D16); the dead `cfg(iceberg|s3tables)` arms in the core are deleted.

**The order.** Inside lane M46 after S4: avro, parquet, excel, xmla - each a
commit, one chain and one push for the batch - then, after S6d has landed
`yggdryl-s3`, iceberg with `s3tables` as its feature depending on
`yggdryl-s3` (D15, D36). `iceberg` is `after = ["avro", "parquet", "s3"]` in
the CI leaf table.

## Order amended (2026-10-09 14:20 UTC)
The landing order above is replaced: excel and xmla each a commit after S4; avro, parquet, s3 and iceberg one dependency-closed commit after them (DESIGN.md "S6's shape"), because the core's Iceberg reads Avro and Parquet and its S3 Tables reads S3 - see regroup.md "S6 order amended".
