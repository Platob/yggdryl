# S6a residue

Tree: `/tmp/s/s6_excel/avrosim`

## Done

- rust/tests/csv/options.rs: 1 item(s) moved to rust/avro/tests/csv/options.rs
- rust/tests/iobase_calls.rs: 2 item(s) moved to rust/avro/tests/iobase_calls.rs
- rust/tests/ipc/mod_.rs: 1 item(s) moved to rust/avro/tests/ipc/mod_.rs
- rust/tests/media/options.rs: 2 item(s) moved to rust/avro/tests/media/options.rs
- rust/tests/media_register.rs: 1 item(s) moved to rust/avro/tests/media_register.rs
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
- paths: 76 use statements and 243 paths re-owned, 45 files
- file paths re-pointed in 16 files
- sibling leaf tests reaching Avro install it: rust/excel/tests/iobase_calls.rs, rust/excel/tests/media/mod_.rs, rust/excel/tests/media/options.rs, rust/excel/tests/media_register.rs, rust/market/tests/graph/serve.rs
- install: 10 harnesses, 166 tests, the bench main, 5 page blocks, the bindings and the CLI
- manifests: rust/avro, the workspace, the core, the bindings and the CLI
- generate_internals.py: rust/src/lib.rs: internals re-exports 114 module(s)
rust/avro/src/lib.rs: internals re-exports 3 module(s)
rust/excel/src/lib.rs: internals re-exports 0 module(s)
rust/fix/src/lib.rs: internals re-exports 16 module(s)
rust/market/src/lib.rs: internals re-exports 7 module(s)
- docs: the Avro page, the media overview, the skills, contributing, the READMEs, AGENTS.md
- .api-inventory.txt: the Avro sections re-homed under yggdryl_avro, the crate root, its implementer, the core's additions

## Residue

- `rust/src/iobase/transfer.rs`: `pub(crate) fn append_arrow_reader_default` matched 0 times, not raised
- `rust/src/iobase/transfer.rs`: `pub(crate) fn merge_arrow_reader_default` matched 0 times, not raised
- `rust/src/iobase/transfer.rs`: `pub(crate) fn overwrite_arrow_reader_default_with_field` matched 0 times, not raised
- `rust/src/iobase.rs`: the statement does not name all of ['append_arrow_reader_default', 'merge_arrow_reader_default', 'overwrite_arrow_reader_default_with_field']
- `rust/src/iomedia.rs`: `pub(crate) fn container_origin` matched 0 times, not raised
- `rust/src/iomedia.rs`: `pub(crate) fn own_options` matched 0 times, not raised
- `rust/src/iomedia.rs`: `pub(crate) fn dimension_options` matched 0 times, not raised
- `rust/src/iomedia.rs`: `pub(crate) fn container_row_size` matched 0 times, not raised
- `scripts/tests/test_ci_plan.py`: anchor matched 0 times, not edited: 'self.assertTrue({"zip-interop", "avro-interop", "excel-interop", "object-interop",\n       '
- `.github/workflows/ci.yml`: anchor matched 0 times, not edited: '# every default-feature test target links and the `interop` target the\n    # ZIP, Avro and'
- `docs/media/index.md`: anchor matched 0 times, not edited: 'the core claims its own - Arrow IPC, Parquet, Avro, plain text, XML for Analysis, CSV and '
- `docs/media/index.md`: anchor matched 0 times, not edited: 'The core claims its own seven media before the register answers anything, until they move '
- `docs/contributing.md`: anchor matched 0 times, not edited: 'and one root folder per medium: `rust/src/ipc/`, `parquet/`, `avro/`, `csv/`, `iceberg/`, '
- `README.md`: anchor matched 0 times, not edited: 'src/{ipc,parquet,avro,csv,iceberg,xmla,excel}/'
- `.api-inventory.txt`: anchor matched 0 times, not edited: 'pub const EXTERNAL_RANK: u8 = 32  (the rank a medium outside the core takes at least; the '
- `.api-inventory.txt`: anchor matched 0 times, not edited: 'the core claims its own seven before the register answers anything, until they move to the'
- `rust/src/iceberg/manifest.rs`: the core's `iceberg` feature names `yggdryl_avro`, which the core cannot depend on (a cycle): it compiles once Iceberg is `yggdryl-iceberg`; until then every `--all-features` build, the bindings (which enable `yggdryl/iceberg`) and the core's Iceberg tests fail - see the report
- `rust/tests/iceberg/evolve.rs`: a core test or bench reads Avro at 1 line(s) (67): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/manifest.rs`: a core test or bench reads Avro at 15 line(s) (172, 267, 494, 505, 516, 529, 548, 587, 918, 976, 1145, 1181, 1273, 1289, 1299): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/metadata.rs`: a core test or bench reads Avro at 1 line(s) (83): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/mod_.rs`: a core test or bench reads Avro at 27 line(s) (63, 103, 203, 218, 2130, 2225, 2226, 2282, 2318, 6120, 6516, 6667, 6728, 6811, 7083, 9294, 9332, 9334, 9356, 9360, 9439, 9463, 9484, 9503, 9524, 9538, 11581): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/official.rs`: a core test or bench reads Avro at 2 line(s) (36, 74): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/scan.rs`: a core test or bench reads Avro at 6 line(s) (36, 173, 525, 771, 1091, 1277): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/snapshot.rs`: a core test or bench reads Avro at 3 line(s) (47, 830, 837): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/iceberg/table.rs`: a core test or bench reads Avro at 4 line(s) (200, 204, 208, 213): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/tests/interop/iceberg.rs`: a core test or bench reads Avro at 3 line(s) (382, 386, 402): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
- `rust/benchmarks/media/iceberg.rs`: a core test or bench reads Avro at 5 line(s) (258, 382, 405, 554, 579): Iceberg's own (moves with `yggdryl-iceberg`, which installs Avro) or to move by hand (D39)
