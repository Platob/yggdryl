# Yggdryl

Arrow-native schemas, byte storage, and structured values, implemented once in Rust and exposed to Python and JavaScript as views of the same values.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    // A non-null struct field is the schema. There is no separate schema type.
    let schema = Field::new(
        "row",
        DataType::from(StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])?),
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
| Warehouse | Catalogs, namespaces and tables over folders and registered objects, the dotted path a plan reads, and the process's `SystemWarehouse` | [warehouse](warehouse/index.md) |
| Coding | gzip, zlib/deflate, and Zstandard over any handle | [compression](media/compression.md) |
| Charset | UTF-8, UTF-16, US-ASCII, and the ISO 8859, Windows, DOS and Mac code pages over any handle | [charsets](media/charsets.md) |
| Media | Arrow IPC, Parquet, Avro, CSV, plain-text records, XML for Analysis rowsets and their provider, and Iceberg tables | [media](media/index.md) |
| Text | JSON, YAML, TOML, and XML over the shared `Scalar` | [JSON](media/json.md), [YAML](media/yaml.md), [TOML](media/toml.md), [XML](media/xml.md) |
| URI | `Uri`, `Url`, `Urn`, `Arn`, paths, globs, and partitions | [uri](uri/index.md) |
| Arrow | Scalars, schema projection, and batch readers at the Arrow boundary | [arrow](arrow/index.md) |
| Expression | Predicates: parse, bind, evaluate, and push down | [expression](expression/index.md) |
| Hashing | xxHash digests over bytes, values, handles, and Arrow rows, and TxHash: an instant coupled with a digest, its sortable keys, coupled columns, and the `DIGEST:time` holder | [hashing](hashing.md) |
| Logging | Python's `logging` owned by the core - loggers, levels, handlers and formatters behind the `log` facade, hosted by `logging` in Python and reached as `logging` in JavaScript - with log files on any storage handle | [logging](logging.md) |
| Graph | Market elements and events, the book walk, its candles and views, and the book display `yggdryl market serve` hosts over a table | [graph](graph/index.md) |
| FIX | Protocol vocabulary, registries, and messages over `Field`, with a live [explorer](fix/explorer.md), [decoder](fix/decode.md) and [composer](fix/encode.md) | [fix](fix/index.md) |

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
