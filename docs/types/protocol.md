# Protocol

Field metadata the library reads: reserved keys, `scheme:name` protocol properties behind live views, and the `field:partition` marker.

## Contract

| Key | Datatype and rule |
| --- | --- |
| `PARQUET:field_id` | signed 32-bit integer, canonicalized on write |
| `field:init` | boolean; absent by default, `false` marks a declared column constructors refuse |
| `field:partition` | boolean; `true` on partition columns, absent elsewhere |
| `location` | [`Url`](../uri/url-urn.md); a straight key, distinct from `http:location` |
| `alias`, `comment`, `display` | validated text; a view falls back to straight `comment` and `display` |
| `scheme:name` | protocol property, prefix canonicalized to a known [`Scheme`](scalar.md) |
| `iceberg:table_name`, `glue:table_name` | catalog coordinates are protocol properties, never straight keys |
| view | a borrow of the one metadata map; writes go through the cache-aware mutation |
| Rust `set` | replaces this protocol's properties only; bindings expose `update`, not `set` |

## Use

A view remembers the scheme, so the caller writes the bare name.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scheme};

    let mut field = Field::new("price", DataType::Int64, false);

    field.as_iceberg_mut().insert("doc", "closing price")?;
    field.as_iceberg_mut().update([("schema-id", "3"), ("field-id", "7")])?;
    field.as_postgres_mut().insert("type", "numeric")?;

    assert_eq!(field.as_iceberg().get("doc"), Some("closing price"));
    assert_eq!(field.as_iceberg().key("doc"), "iceberg:doc");
    assert_eq!(field.as_iceberg().len(), 3);
    assert!(field.as_mysql().is_empty());

    // It is a view of the one metadata map, not a copy of part of it.
    assert_eq!(field.get_metadata("iceberg:doc"), Some("closing price"));
    assert_eq!(field.metadata_len(), 4);

    // A protocol-scoped replacement leaves every other protocol alone.
    field.as_iceberg_mut().set([("doc", "close")])?;
    assert_eq!(field.as_iceberg().iter().collect::<Vec<_>>(), [("doc", "close")]);
    assert_eq!(field.as_postgres().get("type"), Some("numeric"));

    // The view is the field: it dereferences to one, and `as_field` hands back a
    // borrow that outlives the view rather than one that dies with it.
    assert_eq!(field.as_iceberg().dtype(), &DataType::Int64);
    let name = field.as_iceberg().as_field().name();
    assert_eq!(name, "price");

    // The protocol can also come from a value rather than from the code.
    assert_eq!(field.protocol(&Scheme::POSTGRES).get("type"), Some("numeric"));
    ```

=== "Python"

    ```python
    from yggdryl import Field

    field = Field("price", "int64", nullable=False)

    field.iceberg["doc"] = "closing price"
    field.iceberg.update({"schema-id": "3", "field-id": "7"})
    field.postgres["type"] = "numeric"

    assert field.iceberg["doc"] == "closing price"
    assert field.iceberg.key("doc") == "iceberg:doc"
    assert len(field.iceberg) == 3
    assert not field.mysql

    # It is a view of the one metadata mapping, not a copy of part of it.
    assert field.metadata["iceberg:doc"] == "closing price"
    assert len(field.metadata) == 4
    assert dict(field.iceberg.items())["field-id"] == "7"

    del field.iceberg["field-id"]
    assert "field-id" not in field.iceberg
    assert field.protocol("postgres")["type"] == "numeric"
    ```

    !!! note "Rust-only"
        The per-protocol view *types* - `HttpField`, `IcebergField`, `FixField`, and the fifteen others, each
        carrying its protocol's typed vocabulary - are Rust-only for now. `field.iceberg` answers the
        generic property mapping shown here, and the validated HTTP values stay attributes on the
        field itself.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = new Field('price', 'int64', false)

    field.iceberg.set('doc', 'closing price')
    field.iceberg.update({ 'schema-id': '3', 'field-id': '7' })
    field.postgres.set('type', 'numeric')

    assert.equal(field.iceberg.get('doc'), 'closing price')
    assert.equal(field.iceberg.key('doc'), 'iceberg:doc')
    assert.equal(field.iceberg.size, 3)
    assert.equal(field.mysql.size, 0)

    // It is a view of the one metadata map, not a copy of part of it.
    assert.equal(field.get('iceberg:doc'), 'closing price')
    assert.equal(field.size, 4)
    assert.deepEqual([...field.iceberg].sort(), [['doc', 'closing price'], ['field-id', '7'], ['schema-id', '3']])

    assert.equal(field.iceberg.delete('field-id'), true)
    assert.equal(field.iceberg.has('field-id'), false)
    assert.equal(field.protocol('postgres').get('type'), 'numeric')
    ```

    !!! note "Rust-only"
        The per-protocol view *types* - `HttpField`, `IcebergField`, `FixField`, and the fifteen others, each
        carrying its protocol's typed vocabulary - are Rust-only for now. `field.iceberg` answers the
        generic property `Map` shown here, and the validated HTTP values stay accessors on the field
        itself.

## Reserved keys

Typed accessors parse and canonicalize both ways; `http:` parsing lives on Rust's HTTP view and on the binding field itself.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MimeType, Scheme};

    let mut field = Field::new("payload", DataType::Binary, false);

    field.set_parquet_field_id(17);
    field.set_init(false);
    field.set_display("Raw payload")?;
    field.as_http_mut().set_content_type("application/json; charset=utf-8")?;
    field.set_property(&Scheme::POSTGRES, "type", "jsonb")?;

    assert_eq!(field.parquet_field_id()?, Some(17));
    assert_eq!(field.get_metadata("PARQUET:field_id"), Some("17"));
    assert!(!field.is_init()?);
    assert_eq!(field.get_metadata("field:init"), Some("false"));

    // A straight key belongs to no protocol, so every protocol falls back to it.
    assert_eq!(field.display(), Some("Raw payload"));
    assert_eq!(field.as_postgres().display(), Some("Raw payload"));

    // An http: property answers to either scheme and to a raw key lookup, and its
    // parsing accessors live on the field's http: view.
    assert_eq!(field.as_http().mime_type()?, MimeType::JSON);
    assert_eq!(
        field.get_property(&Scheme::HTTPS, "Content-Type"),
        field.as_http().content_type()
    );
    assert_eq!(
        field.get_metadata("http:content-type"),
        field.as_http().content_type()
    );
    assert_eq!(
        field.property_iter(&Scheme::POSTGRES).collect::<Vec<_>>(),
        [("type", "jsonb")]
    );
    ```

=== "Python"

    ```python
    from yggdryl import Field, MimeType

    field = Field("payload", "binary", nullable=False)

    field.set_parquet_field_id(17)
    field.metadata["field:init"] = "false"
    field.set_display("Raw payload")
    field.set_content_type("application/json; charset=utf-8")
    field.set_property("postgres", "type", "jsonb")

    assert field.parquet_field_id == 17
    assert field.metadata["PARQUET:field_id"] == "17"
    assert field.metadata["field:init"] == "false"

    # A straight key belongs to no protocol, so every protocol falls back to it.
    assert field.display == "Raw payload"
    assert field.postgres.display == "Raw payload"

    assert field.mime_type == MimeType.JSON
    assert field.get_property("https", "Content-Type") == field.content_type
    assert field.metadata["http:content-type"] == field.content_type
    assert list(field.property_iter("postgres")) == [("type", "jsonb")]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, MimeType } = require('yggdryl')

    const field = new Field('payload', 'binary', false)

    field.setParquetFieldId(17)
    field.set('field:init', 'false')
    field.setDisplay('Raw payload')
    field.setContentType('application/json; charset=utf-8')
    field.setProperty('postgres', 'type', 'jsonb')

    assert.equal(field.parquetFieldId, 17)
    assert.equal(field.get('PARQUET:field_id'), '17')
    assert.equal(field.get('field:init'), 'false')

    // A straight key belongs to no protocol, so every protocol falls back to it.
    assert.equal(field.display, 'Raw payload')
    assert.equal(field.postgres.display, 'Raw payload')

    assert.ok(field.mimeType.equals(MimeType.JSON))
    assert.equal(field.getProperty('https', 'Content-Type'), field.contentType)
    assert.equal(field.get('http:content-type'), field.contentType)
    assert.deepEqual(field.propertyIter('postgres'), [{ key: 'type', value: 'jsonb' }])
    ```

## Views

Rust only: a protocol's typed vocabulary lives on its view, so deleting a namespace never touches `Field`.

| View | Vocabulary |
| --- | --- |
| `HttpField`, `HttpFieldMut` | `content_type`, `content_length`, `mime_type`, `media_type`, `location` |
| [`IcebergField`, `IcebergFieldMut`](../media/iceberg/schema.md) | `doc`, `schema_id`, `spec_id`, `transform` |
| [`FixField`, `FixFieldMut`](../fix/index.md) | `branch`, `id`, `tag`, `tags`, `aliases`, `description` |

## Partition columns

The reserved `field:partition` key marks partition columns on the fields themselves.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let schema = DataType::from_fields([
        DataType::Int32.required_field("year"),
        DataType::Utf8.required_field("venue"),
        DataType::Int64.required_field("price"),
    ])?
    .required_field("row")
    .with_partition_fields(&["year", "venue"])?;

    assert!(schema.has_partition_fields());
    assert_eq!(schema.partition_field_names().collect::<Vec<_>>(), ["year", "venue"]);
    assert!(schema.get_field_by_path("year").expect("the column").is_partition());

    // The two halves of the layout: what a path spells, and what a leaf stores.
    assert_eq!(schema.without_partition_fields()?.field_len(), 1);
    assert_eq!(schema.only_partition_fields()?.field_len(), 2);

    // The mark is reserved metadata, so it round-trips like any other.
    assert_eq!(
        schema.get_field_by_path("year").expect("the column").get_metadata("field:partition"),
        Some("true")
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    schema = Field(
        "row",
        DataType.from_fields([
            Field("year", "int32", nullable=False),
            Field("venue", "string", nullable=False),
            Field("price", "int64", nullable=False),
        ]),
        nullable=False,
    ).with_partition_fields(["year", "venue"])

    assert schema.has_partition_fields
    assert schema.partition_field_names == ["year", "venue"]
    assert schema.dtype["year"].is_partition
    assert not schema.dtype["price"].is_partition

    assert len(schema.without_partition_fields().dtype) == 1
    assert len(schema.only_partition_fields().dtype) == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const schema = new Field(
      'row',
      DataType.fromFields([
        new Field('year', 'int32', false),
        new Field('venue', 'string', false),
        new Field('price', 'int64', false),
      ]),
      false,
    ).withPartitionFields(['year', 'venue'])

    assert.equal(schema.hasPartitionFields, true)
    assert.deepEqual(schema.partitionFieldNames(), ['year', 'venue'])
    assert.equal(schema.dtype.getFieldByPath('year').isPartition, true)
    assert.equal(schema.dtype.getFieldByPath('price').isPartition, false)

    assert.equal(schema.withoutPartitionFields().dtype.length, 1)
    assert.equal(schema.onlyPartitionFields().dtype.length, 2)
    ```

Folder writes, folder reads, and Iceberg identity specs read the mark: see [Partitions](../holder/iobase/partitions.md) and [Iceberg](../media/iceberg/index.md).

## Edges

- `"+00017"` to `PARQUET:field_id` -> stored as `"17"`; `"2147483648"` -> refused.
- `HTTPS:Content-Type`, `HTTP:content-type`, `http:content-type` -> one entry; `get_property` matches HTTP names case-insensitively.
- `https` -> no accessor; the view for either scheme reports `http`.
- Rust `field.location()` -> straight `location`; `as_http().location()` -> `http:location`, spelled `http_location` / `httpLocation` in the bindings.
- `set_init`, `is_init`, `with_init` -> Rust only; the bindings' mapping write validates identically.
- `display` -> `set_display`, `display`, `remove_display` on the field and every view; Rust adds `try_with_display`.
- Protocol write -> invalidates a populated Arrow projection, like a direct metadata write.
- Non-partition column -> no `field:partition` key, so schemas partitioned alike compare equal.
- `field:partition` -> travels into Arrow, a Parquet footer, and a JSON round trip.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib types::protocol
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- http protocol partition
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(protocol_|http_|partition_|without_partition|typed_location|typed_field_id)'
    cargo bench --manifest-path rust/Cargo.toml --features iceberg --bench types -- '^value/iceberg_'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_field.py -k "http or protocol or partition or typed_names"
    python/.venv/bin/python python/benchmarks/types.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="HTTP|protocol|partition|typed names" node/tests/types/field.test.js
    npm run --prefix node bench:types
    ```
