# S5 residue

- `rust/market, rust/fix`: S4 has not landed on this tree: `release_packages.py crates` lists the core alone, the `[leaves]` lines it names are still commented, and the docs runner and FIX paths are S3's
- `python/benchmarks/types/scalars.py`: a core test or benchmark reaches the market package (1 site(s)): re-fixture or move it (D14); it passes while CI installs both
- `python/tests/enums/test_init.py`: a core test or benchmark reaches the market package (2 site(s)): re-fixture or move it (D14); it passes while CI installs both
- `python/tests/test_datatype.py`: a core test or benchmark reaches the market package (5 site(s)): re-fixture or move it (D14); it passes while CI installs both
- `python/tests/test_logging.py`: a core test or benchmark reaches the market package (1 site(s)): re-fixture or move it (D14); it passes while CI installs both
- `python/tests/typing_bindings.py`: a core test or benchmark reaches the market package (60 site(s)): re-fixture or move it (D14); it passes while CI installs both
- `python/tests/typing_fields.py`: a core test or benchmark reaches the market package (8 site(s)): re-fixture or move it (D14); it passes while CI installs both

## Done

- moves: 25 Python and 23 Node path(s) moved by git
- python: relative imports of 13 moved module(s) re-owned
- python core: the market leaves `yggdryl/__init__.py` (8 statement(s)), its stub, `enums` and `State`; `__path__` extended
- python core: relative imports of moved modules in 1 core file(s) re-spelled to `yggdryl.market`
- python native: `scalar.rs` imports the four market enum classes from `yggdryl.market`, and an enum class is classified without them where the market package is not installed (uncompiled here: `cargo check -p yggdryl-python` is its first proof)
- python: 7 State FIX case(s) moved to the FIX suite
- python market: pyproject (flit_core), README, LICENSE, the root, enums.py, py.typed, the suite's root and pins
- node core: the market doors leave binding.js and fields.js; the natives handed over through implementer.js
- node core: package.json exports `./implementer` and no book; the build regenerates the market natives
- node market: package.json (the exact peer), binding.js, tsconfig, README, LICENSE, the package audit
- node market: package-lock.json derived from the core's lock (9 registry entries, the core linked)
- node declarations: `FieldOptionsInput` is now exported by the core, which the market's factories type with
- node declarations: `NamedField` is now exported by the core, which the market's factories type with
- node declarations: 17 declaration(s), 8 field and 5 listing member(s), 4 augmentation(s) moved; 70 market name(s) exported
- node: implementer.js's list, node/market/index.js and index.d.ts generated
- node re-fixture (D14): the core's factory table and type pins drop the four market kinds the market suite pins, its vocabulary listing the five the market's `enums` holds, the two member lookups move to the market's graph bench, the two FIX-driven logging tests move to the market's fix suite (`node/tests/lib.test.js` removed), and the mixed child script is handed both packages
- cli: `market.rs` embeds the display from `node/market/book/`
- tooling: mypy's namespace, the docs runner, build_docs_fix.js, the inventory checker, the wheel smoke's --market
- paths: 14 moved file(s) re-anchored their own relative paths
- paths: moved paths re-spelled in 47 text file(s)
- python references: 67 file(s) re-spelled to `yggdryl.market`
- javascript references: 58 file(s) re-spelled to `yggdryl-market`
- .api-bindings.txt: 44 Python and 34 JavaScript entries re-homed under the market sections
- ci: rows `python-market` and `node-market`, their two jobs, the market package in the suites that read it, the planner's pins
- release: every crate dry-run and published in order, every name read by preflight and the report, the market packages built once and smoke-tested per platform
- AGENTS.md: sections 2, 3, 4 and 6 (13 edits)
