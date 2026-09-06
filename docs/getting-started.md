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

A non-null struct field is the schema; its children are the columns.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let schema = Field::new(
        "trade",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
            DataType::decimal(18, 4)?.required_field("price"),
        ])?,
        false,
    );

    assert_eq!(schema.field_len(), 3);
    assert_eq!(schema.fields()[2].dtype().to_string(), "decimal64(18,4)");
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    schema = Field(
        "trade",
        DataType.from_fields(
            [
                Field("id", "int64", nullable=False),
                Field("symbol", "utf8"),
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

    const schema = new Field(
      'trade',
      DataType.fromFields([
        new Field('id', 'int64', false),
        new Field('symbol', 'utf8'),
        new Field('price', 'decimal(18,4)', false),
      ]),
      false,
    )

    assert.equal(schema.dtype.length, 3)
    assert.equal(String(schema.dtype.getFieldAt(2).dtype), 'decimal64(18,4)')
    ```

## Attach metadata

Metadata belongs to the field and behaves like each language's mapping type.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let mut field = Field::new("symbol", DataType::Utf8, true);
    field.insert_metadata("source", "book")?;
    field.set_parquet_field_id(7);

    assert_eq!(field.get_metadata("source"), Some("book"));
    assert_eq!(field.parquet_field_id()?, Some(7));
    ```

=== "Python"

    ```python
    from yggdryl import Field

    field = Field("symbol", "utf8", metadata={"source": "book"})
    # Metadata is a mapping on `field.metadata`; subscripting the field itself
    # reaches a nested child.
    field.metadata["venue"] = "XPAR"
    field.set_parquet_field_id(7)

    assert field.metadata["source"] == "book"
    assert "venue" in field.metadata
    assert len(field.metadata) == 3
    assert field.parquet_field_id == 7
    ```

=== "JavaScript"

    ```javascript
    const { Field } = require('yggdryl')
    const assert = require('node:assert/strict')

    const field = new Field('symbol', 'utf8', true, { source: 'book' })
    field.set('venue', 'XPAR')

    assert.equal(field.get('source'), 'book')
    assert.ok(field.has('venue'))
    assert.equal(field.size, 2)
    ```

## Where next

| You need | Page |
| --- | --- |
| Logical types, parsing, families | [DataType](types/datatype.md) |
| Names, nullability, metadata, casting | [Field](types/field.md), [Cast](types/cast.md) |
| Bytes and records on any storage | [Holder](holder/index.md) |
| gzip, zlib, zstd | [Coding](coding/index.md) |
| IPC, Parquet, Avro, Iceberg | [Media](media/index.md) |
| JSON, YAML, TOML | [Text](text/index.md) |
| Naming a resource | [URI](uri/index.md) |
| Predicates and pushdown | [Expression](expression/index.md) |
| Digests | [xxHash](xxhash/index.md) |
| Language boundaries | [Python](extensions/python.md), [JavaScript](extensions/javascript.md) |

## Repository checks

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features "parquet iceberg" -- -D warnings
cargo test --workspace --all-targets --features "parquet iceberg"
python scripts/check_docs_examples.py
python -m mkdocs build --strict
```
