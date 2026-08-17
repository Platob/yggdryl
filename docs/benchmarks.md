# Benchmarks

Criterion targets for the Rust core and boundary benchmarks for each binding.

=== "Rust"

    ```console
    cargo bench --bench datatype
    cargo bench --bench field
    cargo bench --bench enums
    cargo bench --bench uri
    cargo bench --bench text
    cargo bench --bench json
    cargo bench --bench toml
    cargo bench --bench yaml
    ```

=== "Python"

    ```console
    cd python
    .venv/Scripts/python benchmarks/fields.py --iterations 10000
    .venv/Scripts/python benchmarks/records.py --iterations 100000
    .venv/Scripts/python benchmarks/records_arrow.py --min-time 0.2 --repeat 7
    .venv/Scripts/python benchmarks/codecs.py --iterations 10000
    ```

=== "JavaScript"

    ```console
    npm run --prefix node bench:schema
    npm run --prefix node bench:codec
    npm run --prefix node bench:defaults
    ```

Targets live under each member's `benchmarks/` and mirror the core domains. Run them on a quiet
machine against release artifacts, and compare like-for-like toolchains.

## What each target isolates

| Target | Isolates |
| --- | --- |
| `datatype` | Parsing, decimal and time selection, Arrow projection, defaults, compatibility rewriting |
| `field` | Construction, metadata mutation and cache invalidation, comparison, typed views |
| `enums` | MIME and media parsing, compound filename inference, content-coding recovery |
| `uri` | Parsing, component access, path segments, path interop |
| `text` | Value construction, format inference, display and elision helpers |
| `json`, `toml`, `yaml` | Whole-value and streaming encode and decode |

## Rules

**Keep fixtures outside the loop.** A schema, a parsed URL, or an encoded payload rebuilt on every
iteration measures the fixture, not the operation.

**Keep group identifiers stable.** A Criterion group ID is the only thing making two runs
comparable. When a target is split, keep the existing IDs and add new ones beside them; a renamed ID
silently restarts the history it was meant to extend.

**Say what a number means.** A result is evidence for exactly one claim. An encode timing says
nothing about parse allocations, and a cached projection says nothing about a cold one. Cases that
differ in setup belong in different groups.

**Add a benchmark in its own domain.** A field operation goes in the field target, a codec operation
in its format target. Do not create a target for one case, and do not measure two modules in one
group.

For binding benchmarks, measure only the boundary: recursive conversion and validation happen in
Rust and are already measured there.
