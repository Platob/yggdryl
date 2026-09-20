# Types

Datatypes, fields, scalar values, and their shared vocabulary live in one type layer.

## Contract

| key | value |
| --- | --- |
| Owns | `DataType`, `Field`, `Scalar`, and the shared enums re-exported at the crate root |
| Arrow | projection is always compiled; the crate is Arrow-native |
| Bindings | Rust, Python, JavaScript |
| Rust bench target | one, `types`; each page scopes it with a Criterion filter |
| Rust test target | one integration target, `types`, requiring `arrow` |

## Pages

| group | page | owns |
| --- | --- | --- |
| Core | [DataType](datatype.md) | The owned logical type: parsing, canonical display, Arrow projection, defaults |
| Core | [Field](field.md) | Name, datatype, nullability, metadata: the struct root, merge, and diffs |
| Core | [Scalar](scalar.md) | The value every layer speaks, the shared enums, and `FieldScalar` |
| Core | [Cast](cast.md) | The field as cast target, over Scalar rows, Arrow arrays, and record batches |
| Families | [Numeric](numeric.md) | Boolean, integer, floating, decimal, and the `Decimal18` value |
| Families | [Temporal](temporal.md) | The five families - date, time, datetime, duration, interval - their leaves and units, the ISO spellings, the unit and zone vocabulary |
| Families | [Strings & bytes](text.md) | The string family (eighteen leaves: six shapes in each of UTF-8, US-ASCII and windows-1252), the byte family (six leaves), `Str` and `Bytes`, the version value, the `uri` family's `url` and `urn` leaves, regex-capture schema inference |
| Families | [Codes](codes.md) | The ten registered codes over `utf8` storage, `ascii_packed`, `StringEnum` and the ISO listings, the three securities identifiers and their check digits, the `state` lifecycle |
| Families | [UUID](uuid.md) | The 128-bit identifier over `fixed_size_binary(16)` storage |
| Families | [Nested](nested.md) | Children, dictionary and run-end encodings, unions |
| Families | [Geospatial](geospatial.md) | Variant, geometry, geography, and the WKB reader |
| Encoding | [Variant encoding](variant.md) | any value as one byte stream and back: the version, the family-laid datatype identifier, the payload; what pickle carries and a variant column stores |
| Families | [Protocol](protocol.md) | Reserved metadata keys and scheme-prefixed protocol properties |
| Reference | [Playground](playground.md) | Every US-ASCII string width, code, and refusal, as the package answered them |

## Edges

- `cargo bench --bench types -- value` -> three groups carry that name; scope with a function prefix.
- `Field::validate` and Scalar row validation against a struct root -> Rust and Python (`validate`, `validate_value`, `canonicalize_value`); JavaScript validates at every entry point.
- `FieldScalar` and the `wkb` reader -> Rust only; a geospatial value crosses a binding as plain WKB bytes.
- A Python benchmark `--iterations` must be positive; Node benches read `YGGDRYL_BENCH_ITERATIONS`, default 100000.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- arithmetic::tests decimal::tests diff::tests merge::tests metadata::tests path::tests protocol::tests scalar::tests string::tests timezone::tests version::tests
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types
    cargo bench --manifest-path rust/Cargo.toml --bench types
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types python/tests/test_enums.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/types/*.test.js" node/tests/enums.test.js
    npm run --prefix node bench:types
    npm run --prefix node bench:types:defaults
    ```
