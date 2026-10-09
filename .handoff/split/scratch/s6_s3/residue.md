# S6 object stores residue

Tree: `/home/user/yggdryl-s6-s3`

## Done

- rust/tests/aws/environment.rs: 1 item(s) moved to rust/s3/tests/aws/environment.rs
- rust/tests/holder/backend.rs: 2 item(s) moved to rust/s3/tests/holder/backend.rs
- rust/tests/holder/mod_.rs: 7 item(s) moved to rust/s3/tests/holder/mod_.rs
- rust/tests/iobase_calls.rs: 3 item(s) moved to rust/s3/tests/iobase_calls.rs
- rust/tests/warehouse/handle.rs: 1 item(s) moved to rust/s3/tests/warehouse/handle.rs
- rust/tests/warehouse/media.rs: 1 item(s) moved to rust/s3/tests/warehouse/media.rs
- rust/tests/s3/mod_.rs: 5 Iceberg test(s) kept in the core for the Iceberg move
- Iceberg's suites and bench over the store wait under `s3tables`: rust/benchmarks/media/iceberg.rs, rust/tests/iceberg.rs, rust/tests/iceberg/catalog/mod_.rs, rust/tests/iceberg/scan.rs, rust/tests/iceberg/staging.rs, rust/tests/iceberg/table.rs
- moves: 54 files moved by git
- crate: lib.rs and install(), the client's flattened imports, the session's doors through the implementer
- core: 23 `s3` gates of aws/, auth/, http/ and xml/ re-keyed or deleted
- the core's s3tables/ re-spelled onto `yggdryl_s3`: rust/src/s3tables/catalog.rs
- paths: 124 use statements and 193 paths re-owned, 69 files
- file paths re-pointed in 38 files
- install: 6 harnesses, 274 tests, the bench main, 13 rustdoc examples, 1 page blocks, the bindings and the CLI
- manifests: rust/s3, the workspace, the bindings and the CLI
- generate_internals.py: rust/src/lib.rs: internals re-exports 129 module(s)
rust/s3/src/lib.rs: internals re-exports 11 module(s)

## For the Iceberg move

- `rust/benchmarks/media/iceberg.rs:1878`
- `rust/src/s3tables/catalog.rs:507,512,671,674`
- `rust/tests/iceberg/catalog/mod_.rs:1373,1390,1395`
- `rust/tests/iceberg/scan.rs:1827,1922`
- `rust/tests/iceberg/staging.rs:254,329`
- `rust/tests/s3/mod_.rs:30,63,73,83,116`
- `rust/tests/s3tables/live.rs:26,246,270`
- `rust/tests/s3.rs (the remnant's harness)`

## Residue

- `scripts/check_docs_examples.py`: the Rust pages compile in the core's `rust/tests/docs_examples.rs` until S4 moves the runner to `cli/tests/` (D13): a block naming `yggdryl_s3` compiles only there
