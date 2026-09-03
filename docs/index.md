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

## What is here

**A schema is a field.** [`DataType`](datatype.md) is the logical type tree and
[`Field`](field.md) adds a name, nullability, and metadata. A non-null struct field describes
rows, so there is no second schema type to keep in sync, and [casting](field.md) reconciles
incoming Arrow data to it.

**Storage is one trait.** [`IOBase`](io.md) addresses bytes positionally and lazily: building
a handle touches nothing, reading something absent yields nothing, writing creates. An in-memory
buffer, a [local file or directory](local.md), and a
[compressed view](gzip.md) of either are all the same trait, and
[one enum](generic.md) names every implementation.

**Records ride on storage.** Any handle reads and writes Arrow batches, choosing
[Arrow IPC](ipc.md) or [Parquet](parquet.md) from its own media type, and
[Iceberg](iceberg.md) reads its schemas as ordinary fields.

**Scalars are one tree.** [JSON](json.md), [YAML](yaml.md), and [TOML](toml.md) share
the [structured value](text.md), and [URIs](uri.md) name where any of it lives.

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

Start with [Getting started](getting-started.md), or read the [architecture](architecture.md) for
the shape of the whole thing first.
