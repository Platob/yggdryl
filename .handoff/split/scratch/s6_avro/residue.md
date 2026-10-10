# S6a residue

Tree: `/home/user/yggdryl-s6-avro`

## Done

- rust/tests/csv/options.rs: 1 item(s) moved to rust/avro/tests/csv/options.rs
- rust/tests/graph/serve.rs: 1 item(s) moved to rust/avro/tests/graph/serve.rs
- rust/tests/iobase_calls.rs: 3 item(s) moved to rust/avro/tests/iobase_calls.rs
- rust/tests/ipc/mod_.rs: 1 item(s) moved to rust/avro/tests/ipc/mod_.rs
- rust/tests/media/mod_.rs: 3 item(s) moved to rust/avro/tests/media/mod_.rs
- rust/tests/media/options.rs: 13 item(s) moved to rust/avro/tests/media/options.rs
- rust/tests/media_register.rs: 2 item(s) moved to rust/avro/tests/media_register.rs
- rust/tests/root/ascii.rs: 1 item(s) moved to rust/avro/tests/root/ascii.rs
- rust/tests/root/iomedia.rs: 7 item(s) moved to rust/avro/tests/root/iomedia.rs
- rust/tests/root/media_serie.rs: 3 item(s) moved to rust/avro/tests/root/media_serie.rs
- rust/tests/warehouse/folder.rs: 2 item(s) moved to rust/avro/tests/warehouse/folder.rs
- rust/tests/xmla/options.rs: 1 item(s) moved to rust/avro/tests/xmla/options.rs
- benches: rust/avro/benchmarks/avro/io.rs, rust/avro/benchmarks/avro/calls.rs carved out of the core's shared record benches
- moves: 24 files moved by git
- PlanCache moved into implementer.rs, 7 core file(s) re-pointed
- rust/src/iceberg/manifest.rs: the manifest's container reads re-spelled onto `yggdryl_avro`
- crate: lib.rs, implementer.rs, the raises for Iceberg, the gates; core: the seed, RESERVED_RANKS, the implementer
- paths: 75 use statements and 243 paths re-owned, 43 files
- file paths re-pointed in 16 files
- install: 11 harnesses, 182 tests, the bench main, 4 page blocks, the bindings and the CLI
- manifests: rust/avro, the workspace, the core, the bindings and the CLI
- generate_internals.py: rust/src/lib.rs: internals re-exports 138 module(s)
rust/avro/src/lib.rs: internals re-exports 3 module(s)
- docs: the Avro page, the media overview, the skills, contributing, the READMEs, AGENTS.md
- .api-inventory.txt: the Avro sections re-homed under yggdryl_avro, the crate root, its implementer, the core's additions

## Residue

- `rust/src/iceberg/manifest.rs`: the core's `iceberg` feature names `yggdryl_avro`, which the core cannot depend on (a cycle): it compiles once Iceberg is `yggdryl-iceberg`; until then every `--all-features` build, the bindings (which enable `yggdryl/iceberg`) and the core's Iceberg tests fail - see the report
- `rust/tests/iceberg/evolve.rs`: a core test or bench reads Avro at 1 line(s) (67): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/manifest.rs`: a core test or bench reads Avro at 15 line(s) (172, 267, 494, 505, 516, 529, 548, 587, 918, 976, 1145, 1181, 1273, 1289, 1299): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/metadata.rs`: a core test or bench reads Avro at 1 line(s) (83): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/mod_.rs`: a core test or bench reads Avro at 27 line(s) (63, 103, 203, 218, 2200, 2295, 2296, 2352, 2388, 6201, 6597, 6748, 6809, 6892, 7164, 9376, 9414, 9416, 9438, 9442, 9521, 9545, 9566, 9585, 9606, 9620, 11666): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/official.rs`: a core test or bench reads Avro at 2 line(s) (36, 74): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/scan.rs`: a core test or bench reads Avro at 6 line(s) (36, 173, 525, 771, 1091, 1277): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/snapshot.rs`: a core test or bench reads Avro at 3 line(s) (47, 830, 837): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/table.rs`: a core test or bench reads Avro at 4 line(s) (200, 204, 208, 213): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/interop/iceberg.rs`: a core test or bench reads Avro at 3 line(s) (382, 386, 402): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/benchmarks/media/iceberg.rs`: a core test or bench reads Avro at 5 line(s) (258, 382, 405, 554, 579): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
