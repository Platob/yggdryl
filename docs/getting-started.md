# Getting started

Build one runtime, describe a schema, attach metadata, then pick a layer tab.

## Build

=== "Rust"

    ```bash
    cargo build
    cargo test --workspace --features "parquet iceberg"
    ```

=== "Python"

    ```bash
    cd python
    python -m venv .venv
    .venv/bin/python -m pip install maturin pytest pyarrow
    .venv/bin/python -m maturin develop
    .venv/bin/python -m pytest
    ```

=== "JavaScript"

    ```bash
    cd node
    npm ci
    npm run build:debug
    npm test
    ```

On Windows the interpreter is `.venv\Scripts\python`. `maturin develop --release` builds the wheel that benchmarks time.

## Describe a schema

Metadata belongs to the field and behaves like each language's mapping type; a non-null struct field is the schema, and its children are the columns.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let mut symbol = DataType::utf8().nullable_field("symbol");
    symbol.insert_metadata("source", "book")?;
    symbol.set_parquet_field_id(7);
    assert_eq!(symbol.get_metadata("source"), Some("book"));
    assert_eq!(symbol.parquet_field_id()?, Some(7));

    let schema = Field::new(
        "trade",
        DataType::from(StructType::from_fields([
            DataType::Int64.required_field("id"),
            symbol,
            DataType::decimal(18, 4)?.required_field("price"),
        ])?),
        false,
    );

    assert_eq!(schema.field_len(), 3);
    assert_eq!(schema.fields()[2].dtype().to_string(), "decimal64(18,4)");
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    symbol = Field("symbol", "utf8", metadata={"source": "book"})
    # Metadata is a mapping on `field.metadata`; subscripting the field itself
    # reaches a nested child.
    symbol.metadata["venue"] = "XPAR"
    symbol.set_parquet_field_id(7)
    assert symbol.metadata["source"] == "book"
    assert "venue" in symbol.metadata
    assert len(symbol.metadata) == 3
    assert symbol.parquet_field_id == 7

    schema = Field(
        "trade",
        DataType.from_fields(
            [
                Field("id", "int64", nullable=False),
                symbol,
                Field("price", DataType.decimal(18, 4), nullable=False),
            ]
        ),
        nullable=False,
    )

    assert len(schema.dtype) == 3
    assert str(schema.dtype[2].dtype) == "decimal64(18,4)"
    ```

=== "JavaScript"

    ```javascript
    const { DataType, Field } = require('yggdryl')
    const assert = require('node:assert/strict')

    const symbol = new Field('symbol', 'utf8', true, { source: 'book' })
    symbol.set('venue', 'XPAR')
    assert.equal(symbol.get('source'), 'book')
    assert.ok(symbol.has('venue'))
    assert.equal(symbol.size, 2)

    const schema = new Field(
      'trade',
      DataType.fromFields([
        new Field('id', 'int64', false),
        symbol,
        new Field('price', 'decimal(18,4)', false),
      ]),
      false,
    )

    assert.equal(schema.dtype.length, 3)
    assert.equal(String(schema.dtype.getFieldAt(2).dtype), 'decimal64(18,4)')
    ```

## Where next

| You need | Page |
| --- | --- |
| Logical types, parsing, families | [DataType](types/datatype.md) |
| Names, nullability, metadata, casting | [Field](types/field.md), [Cast](types/cast.md) |
| Bytes and records on any storage | [Holder](holder/index.md) |
| gzip, zlib, zstd | [Coding](media/index.md#compression) |
| Character encodings | [Charset](media/index.md#charsets) |
| IPC, Parquet, Avro, Iceberg | [Media](media/index.md) |
| JSON, YAML, TOML, XML | [Structured documents](media/index.md#json) |
| Naming a resource | [URI](uri/index.md) |
| Scalars, schemas, and batch readers at the Arrow boundary | [Arrow](arrow/index.md) |
| Predicates and pushdown | [Expression](expression/index.md) |
| Digests and time-keyed digests | [Hashing](hashing.md) |
| FIX messages, registries, and captures | [FIX](fix/index.md) |

## Repository checks

The full pass, per entry and per language, is on [Testing](testing.md); what a change must satisfy before handoff is on [Contributing](contributing.md).
