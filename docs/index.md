# Yggdryl

Arrow-native schemas, byte storage, and structured values, implemented once in Rust and exposed to Python and JavaScript as views of the same values.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    // A non-null struct field is the schema. There is no separate schema type.
    let schema = Field::new(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])?,
        false,
    );

    assert_eq!(schema.field_len(), 2);
    assert_eq!(schema.index_of("symbol"), Some(1));
    assert!(!schema.fields()[0].is_nullable());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    # A datatype argument accepts its own expression, so "int64" needs no wrapper.
    schema = Field(
        "row",
        DataType.from_fields(
            [Field("id", "int64", nullable=False), Field("symbol", "utf8")]
        ),
        nullable=False,
    )

    assert len(schema.dtype) == 2
    assert schema.dtype[1].name == "symbol"
    assert not schema.dtype[0].nullable
    ```

=== "JavaScript"

    ```javascript
    const { DataType, Field } = require('yggdryl')
    const assert = require('node:assert/strict')

    const schema = new Field(
      'row',
      DataType.fromFields([
        new Field('id', 'int64', false),
        new Field('symbol', 'utf8'),
      ]),
      false,
    )

    assert.equal(schema.dtype.length, 2)
    assert.equal(schema.dtype.getFieldAt(1).name, 'symbol')
    assert.equal(schema.dtype.getFieldAt(0).nullable, false)
    ```

## Layers

One tab per layer in the top bar; one page per family in that layer's sidebar.

| Layer | Owns | Start at |
| --- | --- | --- |
| Types | `DataType`, `Field`, `Scalar`, casting, and the datatype families | [types](types/index.md) |
| Holder | `IOBase` handles: bytes, values, records, and the storage backends | [holder](holder/index.md) |
| Coding | gzip, zlib/deflate, and Zstandard over any handle | [coding](coding/index.md) |
| Media | Arrow IPC, Parquet, Avro, plain-text records, and Iceberg tables | [media](media/index.md) |
| Text | JSON, YAML, and TOML over the shared `Scalar` | [text](text/index.md) |
| URI | `Uri`, `Url`, `Urn`, paths, globs, and partitions | [uri](uri/index.md) |
| Arrow | Scalars, schema projection, and batch readers at the Arrow boundary | [arrow](arrow/index.md) |
| Expression | Predicates: parse, bind, evaluate, and push down | [expression](expression/index.md) |
| xxHash | Digests over bytes, values, handles, and Arrow rows | [xxhash](xxhash/index.md) |
| FIX | Protocol vocabulary, registries, and messages over `Field` | [fix](fix/index.md) |
| Extensions | What crosses the Python and JavaScript boundaries | [Python](extensions/python.md), [JavaScript](extensions/javascript.md) |

## Install

=== "Rust"

    ```toml
    [dependencies]
    yggdryl = "0.1"

    # Parquet and Iceberg are opt-in; everything else is on by default.
    # yggdryl = { version = "0.1", features = ["parquet", "iceberg"] }
    ```

=== "Python"

    ```console
    pip install yggdryl
    ```

=== "JavaScript"

    ```console
    npm install yggdryl
    ```

Then [Getting started](getting-started.md), or the [architecture](architecture.md) for the shape of the whole tree.
