# Types

Datatypes, fields, scalar values, and their shared vocabulary live in one type layer.

## Contract

| key | value |
| --- | --- |
| Owns | `DataType`, `Field`, `Scalar`, and the shared enums re-exported at the crate root |
| Layout | the Core pages below, then one subsection per family, then one page per type inside it |
| Arrow | projection is always compiled; the crate is Arrow-native |
| Bindings | Rust, Python, JavaScript |
| Rust bench target | one, `types`; each page scopes it with a Criterion filter |
| Rust test target | one integration target, `types`, requiring `arrow` |

## How a type page reads

A type's core file holds its datatype, then its field, then its scalar, so its
page is written in that order: **Contract**, **DataType**, **Field**,
**Scalar**, **Arrow storage**, then one section per behaviour that type has -
casts, grammar, parsing, arithmetic, bounds, vocabulary - then **Edges** and
**Commands**. Every example appears in Rust, Python and JavaScript tabs, in that
order; a call only one binding has is named as such rather than invented for the
other two.

A family subsection is a folder: `index.md` for what its leaves share - the
range of identifiers the family is, the leaf contract its values answer, the
casts and the Arrow rules they have in common - and one page per type in it. A
type whose family is itself has a page at the root of this tab instead of a
folder of one page.

## Core

| page | owns |
| --- | --- |
| [DataType](datatype.md) | The owned logical type: parsing, canonical display, Arrow projection, defaults |
| [Field](field.md) | Name, datatype, nullability, metadata: the struct root, merge, and diffs |
| [Scalar](scalar.md) | The value every layer speaks, the shared enums, and `FieldScalar` |
| [Serie](serie.md) | Many values: a schema-free `Run`, or the Arrow buffers of one field nested as `Serie` children, read and written as a collection and crossing to Arrow by sharing buffers |
| [Chunked serie](chunked-serie.md) | Many `Serie` columns under one field, held apart: a chunked array, or a table of one batch per chunk, read across its chunks as one column and joined only by `into_serie` |
| [Cast](cast.md) | The field as cast target, over Scalar rows, Arrow arrays, and record batches |
| [Paths](paths.md) | `FieldPath`: the one path into a nested schema or value, resolved once and applied many times |
| [Protocol](protocol.md) | Reserved metadata keys and scheme-prefixed protocol properties |

## Families

| subsection | type pages | what the family shares |
| --- | --- | --- |
| [Numeric](numeric/index.md) | [Integer](numeric/integer.md), [Floating](numeric/floating.md), [Decimal](numeric/decimal.md), [Boolean](numeric/boolean.md) | The four families whose value is a number: one kind each, the widths they are spelled at, the selectors, the typed markers, and the widening and casts they share |
| [Temporal](temporal/index.md) | [Date](temporal/date.md), [Time](temporal/time.md), [Datetime](temporal/datetime.md), [Duration](temporal/duration.md), [Interval](temporal/interval.md), [Time zone](temporal/timezone.md) | Eight leaves over one `TimeUnit` vocabulary and one `Timezone` value, `DataTypeId::temporal_family` naming the five, the ISO spellings, and the merge that refuses to cross families |
| [Strings & bytes](text/index.md) | [String](text/string.md), [Bytes](text/bytes.md) | Two families whose value is a run of bytes: eighteen string leaves in three charsets, six byte leaves, `Str` and `Bytes`, the bound rule and the serialized shape |
| [Codes](codes/index.md) | [Ccy](codes/ccy.md), [Country](codes/country.md), [MIC](codes/mic.md), [CFI](codes/cfi.md), [ISIN](codes/isin.md), [CUSIP](codes/cusip.md), [SEDOL](codes/sedol.md), [Bloomberg](codes/bloomberg.md), [FIGI](codes/figi.md), [Side](codes/side.md), [State](codes/state.md), [TimeInForce](codes/timeinforce.md) | Twelve identities over a published registry: `DataType::CODES`, the `CodeValue` contract, US-ASCII storage with its own Arrow extension name, `StringEnum` vocabularies and the check digits |
| [Nested](nested/index.md) | [Struct](nested/struct.md), [Serie layouts](nested/sequence.md), [Map](nested/map.md), [Union](nested/union.md), [Dictionary](nested/dictionary.md), [Run-end](nested/runend.md) | The datatypes that hold other datatypes: four layouts over child fields, two encodings over a value, one shared child allocation, and the `NestedValue` contract |
| [Geospatial](geospatial/index.md) | [Geometry](geospatial/geometry.md), [Geography](geospatial/geography.md) | One Well-Known Binary payload under two readings, the shared `GeospatialParameters`, the CRS and edge vocabulary, and the dependency-free `wkb` reader |

## Single types

| page | owns |
| --- | --- |
| [UUID](uuid.md) | The 128-bit identifier, its spellings, and its `arrow.uuid` storage |
| [Version](version.md) | Three numeric components in four bytes, numerically ordered, not lexicographic |
| [Media types](mediatype.md) | `mimetype` and `mediatype`: what a record's bytes are, and what they were declared under |
| [Variant](variant.md) | The Apache Parquet Variant encoding: one metadata dictionary and one value payload, the pair Parquet, Avro, Arrow and Iceberg all state for a `variant` column |
| [Value stream](value-stream.md) | Any value as one byte stream and back: the version, the family-laid datatype identifier, the payload; what pickle carries |

The [playground](playground.md) renders every US-ASCII string width, every
registered code, every refusal and a declared vocabulary as the package itself
answered them.

## Edges

- `cargo bench --bench types -- value` -> three groups carry that name; scope with a function prefix.
- `Field::validate` and Scalar row validation against a struct root -> Rust and Python (`validate`, `validate_value`, `canonicalize_value`); JavaScript validates at every entry point.
- `FieldScalar` and the `wkb` reader -> Rust only; a geospatial value crosses a binding as plain WKB bytes.
- A family is a range of identifiers, never a type: a family with one leaf is that leaf, and its page sits at this tab's root rather than in a folder.
- A Python benchmark `--iterations` must be positive; Node benches read `YGGDRYL_BENCH_ITERATIONS`, default 100000.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test parquet -- metadata
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- arithmetic decimal::internal::reading diff::internal merge::internal metadata::internal path protocol::internal::tests scalar::internal string::codes timezone::internal version::internal
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test expression --test media_type --test metadata --test mime_type --test root --test text --test uri --test value
    cargo bench --manifest-path rust/Cargo.toml --bench types
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test__classes.py python/tests/test__defaults.py python/tests/test__hints.py python/tests/test_cast.py python/tests/test_datatype.py python/tests/test_field.py python/tests/test_protocol.py python/tests/test_scalar.py python/tests/test_timezone.py python/tests/test_version.py python/tests/enums/test_init.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/datatype.test.js node/tests/defaults.test.js node/tests/field.test.js node/tests/fields.test.js node/tests/timezone.test.js node/tests/value.test.js node/tests/version.test.js node/tests/enums/vocabulary.test.js
    npm run --prefix node bench:types
    npm run --prefix node bench:types:defaults
    ```
