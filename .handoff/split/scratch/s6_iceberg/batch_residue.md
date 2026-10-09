# S6 Iceberg residue

Tree: `/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad/s6_iceberg/batch`

## Done

- rust/tests/allocations.rs: 11 item(s) moved and 0 copied to rust/iceberg/tests/allocations.rs
- rust/tests/fix/enrich.rs: 2 item(s) moved and 0 copied to rust/iceberg/tests/fix/enrich.rs
- rust/tests/fix/schema.rs: 2 item(s) moved and 0 copied to rust/iceberg/tests/fix/schema.rs
- rust/tests/graph/serve.rs: 3 item(s) moved and 0 copied to rust/iceberg/tests/graph/serve.rs
- rust/tests/isin_registry/store.rs: 7 item(s) moved and 0 copied to rust/iceberg/tests/isin_registry/store.rs
- rust/tests/root/ascii.rs: 1 item(s) moved and 0 copied to rust/iceberg/tests/root/ascii.rs
- rust/tests/root/error.rs: 1 item(s) moved and 0 copied to rust/iceberg/tests/root/error.rs
- rust/tests/root/protocol.rs: 1 item(s) moved and 0 copied to rust/iceberg/tests/root/protocol.rs
- rust/tests/root/version.rs: its Iceberg gates resolved for a build without the crate
- rust/tests/uri/handle.rs: 1 item(s) moved and 0 copied to rust/iceberg/tests/uri/handle.rs
- rust/tests/warehouse/catalog.rs: 0 item(s) moved and 1 copied to rust/iceberg/tests/warehouse/catalog.rs
- rust/tests/warehouse/folder.rs: 2 item(s) moved and 0 copied to rust/iceberg/tests/warehouse/folder.rs
- rust/tests/warehouse/media.rs: 0 item(s) moved and 1 copied to rust/iceberg/tests/warehouse/media.rs
- rust/tests/warehouse/table.rs: 1 item(s) moved and 0 copied to rust/iceberg/tests/warehouse/table.rs
- rust/avro/tests/warehouse/folder.rs: 0 item(s) moved and 1 copied to rust/iceberg/tests/warehouse/folder.rs
- test split: 32 item(s) moved, 3 copied with the gate's other side
- benches: rust/iceberg/benchmarks/iceberg/field.rs, rust/iceberg/benchmarks/iceberg/aws.rs, rust/iceberg/benchmarks/iceberg.rs written; the core's `media`, `types` and `holder` lose their Iceberg rows
- moves: 61 files moved by git
- rust/iceberg/tests/iceberg/types.rs: its items naming only the core's vocabulary back in rust/tests/root/iceberg.rs
- rust/iceberg/tests/iceberg/official.rs: the official failure's pin, beside the door it pins
- crate: lib.rs, the view minted in field.rs, the official doors, the method redirects
- core: lib.rs, the type vocabulary, the protocol entry, error.rs, three seeds; 15 file(s) un-gated
- core manifest: the `iceberg` and `s3tables` features, `iceberg-official` and `uuid` gone
- aws::Answer moved into implementer.rs; the identity sources re-pointed
- implementer.rs: the Iceberg section, 55 entries added, 33 raises and the moved `Answer`
- paths: 269 use statements and 651 paths re-owned, 82 files
- file paths re-pointed in 51 files
- imports: 1 crate file(s) lose the names the redirects left unused
- crate gates: `iceberg` resolved and `s3` read as `s3tables` in 5 file(s)
- view: 200 `as_iceberg` calls swept
- interop: the crate's `interop` target, the exchange kept at `target/iceberg-interop`, the driver re-pointed
- install: 14 harnesses, 645 tests, 27 doc examples, 26 page blocks, the bench main, the bindings and the CLI
- manifests: rust/iceberg, the workspace, the bindings, the CLI and the sibling leaves
- generate_internals.py: rust/src/lib.rs: internals re-exports 114 module(s)
rust/avro/src/lib.rs: internals re-exports 3 module(s)
rust/iceberg/src/lib.rs: internals re-exports 10 module(s)
rust/parquet/src/lib.rs: internals re-exports 1 module(s)
rust/s3/src/lib.rs: internals re-exports 11 module(s)
- tooling: the per-crate generators, the internals re-exports regenerated, the logging facade's row
- CI: the `iceberg` leaf line on, `x-iceberg` reading the crate, the MSRV job and the PyIceberg build re-pointed
- docs: the Iceberg page's overview, the feature tables, 11 pages' prose re-pointed at `yggdryl-iceberg`
- AGENTS.md: the features, the layout rows, the gated loop, the MSRV row
- .api-inventory.txt: the Iceberg sections re-homed under yggdryl_iceberg, the core's type section, the view, the implementer additions
- commands: 71 files' cargo lines lose the core's `iceberg`, `s3tables` and `parquet` features

## Residue

- `scripts/check_docs_examples.py`: the Rust pages compile in the core's `rust/tests/docs_examples.rs` until S4 moves the runner to `cli/tests/` (D13): a block naming `yggdryl_iceberg` compiles only there
