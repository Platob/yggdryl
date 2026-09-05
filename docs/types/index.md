# Types

Datatypes, fields, scalar values, and their shared vocabulary live in one type layer.

## Contract

| key | value |
| --- | --- |
| Owns | `DataType`, `Field`, `Scalar`, and the shared enums re-exported at the crate root |
| Arrow | projection sits behind the default `arrow` feature |
| Bindings | Rust, [Python](../extensions/python.md), [JavaScript](../extensions/javascript.md) |
| Rust bench target | one, `types`; each page scopes it with a Criterion filter |
| Rust test target | one integration target, `types`, requiring `arrow` |

## Pages

| group | page | owns |
| --- | --- | --- |
| Core | [DataType](datatype.md) | The owned logical type: parsing, canonical display, Arrow projection, defaults |
| Core | [Field](field.md) | Name, datatype, nullability, metadata: the struct root, merge, and diffs |
| Core | [Scalar](scalar.md) | The value every layer speaks, the shared enums, and `TypedScalar` |
| Core | [Cast](cast.md) | The field as cast target, over Scalar rows, Arrow arrays, and record batches |
| Families | [Numeric & temporal](numeric.md) | Boolean, integer, floating, decimal, and the temporal vocabulary |
| Families | [Text & bytes](text.md) | The utf8 and binary widths, plus regex-capture schema inference |
| Families | [ASCII](ascii.md) | Variable and fixed widths, the four registered codes, `AsciiEnum` |
| Families | [UUID](uuid.md) | The 128-bit identifier over `FixedSizeBinary(16)` |
| Families | [Nested](nested.md) | Children, dictionary and run-end encodings, unions |
| Families | [Geospatial](geospatial.md) | Variant, geometry, geography, and the WKB reader |
| Families | [Protocol](protocol.md) | Reserved metadata keys and scheme-prefixed protocol properties |
| Reference | [Playground](playground.md) | Every ASCII datatype, code, and refusal, as the package answered them |

## Edges

- `cargo bench --bench types -- value` -> three groups carry that name; scope with a function prefix.
- `Field::validate` and Scalar row validation against a struct root -> Rust only; the bindings validate at every entry point.
- `TypedScalar` and the `wkb` reader -> Rust only; a geospatial value crosses a binding as plain WKB bytes.
- A Python benchmark `--iterations` must be positive; Node benches read `YGGDRYL_BENCH_ITERATIONS`, default 100000.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib types::
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types
    cargo bench --manifest-path rust/Cargo.toml --bench types
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types python/tests/test_enums.py
    python/.venv/bin/python python/benchmarks/types.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/types/*.test.js" node/tests/enums.test.js
    npm run --prefix node bench:types
    npm run --prefix node bench:types:defaults
    ```
