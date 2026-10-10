# S6b residue

Tree: `/home/user/yggdryl-s6-excel`

## Done

- rust/tests/allocations.rs: 3 item(s) moved to rust/excel/tests/allocations.rs
- rust/tests/holder/mod_.rs: 3 item(s) moved to rust/excel/tests/holder/mod_.rs
- rust/tests/iobase_calls.rs: 2 item(s) moved to rust/excel/tests/iobase_calls.rs
- rust/tests/media/magic.rs: 2 item(s) moved to rust/excel/tests/media/magic.rs
- rust/tests/media/mod_.rs: 6 item(s) moved to rust/excel/tests/media/mod_.rs
- rust/tests/media/options.rs: 15 item(s) moved to rust/excel/tests/media/options.rs
- rust/tests/media_register.rs: 2 item(s) moved to rust/excel/tests/media_register.rs
- tests: 9 file(s) split off the core's; the media bench and the interop target lose Excel
- moves: 27 files moved by git
- crate: lib.rs and its install(), the bench root; core: the seed, RESERVED_RANKS, the implementer, the exclude
- paths: 57 use statements and 143 paths re-owned, 39 files
- file paths re-pointed in 17 files
- calls: 7 method call(s) on core types re-spelled through their forwarders
- install: 7 harnesses, 367 tests, the bench main, 1 crate example(s), 4 page blocks, the bindings and the CLI
- manifests: rust/excel, the workspace, the bindings and the CLI
- generate_internals.py: rust/src/lib.rs: internals re-exports 140 module(s)
rust/excel/src/lib.rs: internals re-exports 0 module(s)
- docs: the Excel page, the media overview, the benchmark index, contributing, architecture, the skills, the README, AGENTS.md
- .api-inventory.txt: the Excel sections re-homed under yggdryl_excel, its install, the core's additions

## Residue

- `rust/excel/tests/media/magic.rs`: trait import(s) `IORecordOptions`, `IOBase`, `IOMedia` kept only for a method call: the compiler loop drops each it finds unused
- `rust/tests/media/mod_.rs`: trait import(s) `IORecordOptions`, `IOBase`, `IOMedia` kept only for a method call: the compiler loop drops each it finds unused
- `rust/excel/tests/media/mod_.rs`: trait import(s) `IORecordOptions`, `IOBase`, `IOMedia` kept only for a method call: the compiler loop drops each it finds unused
- `rust/excel/tests/media_register.rs`: trait import(s) `LocatedTable`, `TableFormat`, `ObjectValue` kept only for a method call: the compiler loop drops each it finds unused
- `scripts/check_docs_examples.py`: the Rust page blocks still compile as a core test target, which cannot link `yggdryl-excel` (a cycle): the blocks that install it compile once S4 moves the runner under `cli/` (D13)
