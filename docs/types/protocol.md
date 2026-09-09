# Protocol

Field metadata the library reads: reserved keys, `scheme:name` properties behind live views, and the `field:partition` marker.

## Contract

| Key | Datatype and rule |
| --- | --- |
| `PARQUET:field_id` | i32, canonicalized on write |
| `field:init` | boolean, absent by default; `false` = declared but refused by constructors |
| `field:partition` | boolean; `true` on partition columns, absent elsewhere |
| `location` | [`Url`](../uri/url-urn.md), a straight key |
| `alias`, `comment`, `display` | validated text; views fall back to straight `comment` and `display` |
| `scheme:name` | protocol property, prefix canonicalized to a known [`Scheme`](scalar.md) |
| `iceberg:table_name` | catalog coordinates are protocol properties, never straight keys |
| `digest:role` | `holder`; anything else refused |
| `digest:time`, `digest:unit` | a holder's coupled instant: one field path, and `s` / `ms` / `us` / `ns` canonicalized; the storage must be a coupled `fixed_size_binary` ([TxHash](../txhash/arrow.md#coupled-holders)) |
| `python:module`, `python:qualname` | the declaring Python class, as dotted names Python itself could have written; `<locals>` is the one non-identifier segment a qualified name may carry |
| `python:kind` | `field`, `dataclass`, `typed_dict`, `named_tuple`, `enum`, `newtype`, `type_alias`, or `class`; anything else refused |
| view | borrow of the one metadata map, cache-aware writes |
| Rust `set` | replaces only this protocol's keys; bindings expose `update`, not `set` |

## Use

The view remembers the scheme; the caller writes the bare name.

=== "Rust"

    ```rust
    use yggdryl::expression::Function;
    use yggdryl::{DataType, Field, Scheme};

    let mut field = Field::new("price", DataType::Int64, false);

    field.as_iceberg_mut().insert("doc", "closing price")?;
    field.as_iceberg_mut().update([("schema-id", "3"), ("field-id", "7")])?;
    field.as_postgres_mut().insert("type", "numeric")?;
    field.as_digest_mut().set_holder()?;
    field.as_identity_mut().update([("role", "primary"), ("nulls", "distinct")])?;
    field.as_partition_mut().set_transform(Function::Year)?;
    field.as_partition_mut().set_sources(["event"])?;

    assert_eq!(field.as_iceberg().get("doc"), Some("closing price"));
    assert_eq!(field.as_iceberg().key("doc"), "iceberg:doc");
    assert_eq!(field.as_iceberg().len(), 3);
    assert!(field.as_mysql().is_empty());
    assert!(field.as_digest().is_holder());
    assert_eq!(field.as_identity().get("role"), Some("primary"));
    assert_eq!(field.as_partition().transform()?, Some(Function::Year));

    // It is a view of the one metadata map, not a copy of part of it.
    assert_eq!(field.get_metadata("iceberg:doc"), Some("closing price"));
    assert_eq!(field.get_metadata("digest:role"), Some("holder"));
    assert_eq!(field.metadata_len(), 9);

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
    field.digest["role"] = "holder"
    field.identity.update({"role": "primary", "nulls": "distinct"})
    field.partition.transform = "year"
    field.partition.sources = ["event"]

    assert field.iceberg["doc"] == "closing price"
    assert field.iceberg.key("doc") == "iceberg:doc"
    assert len(field.iceberg) == 3
    assert not field.mysql
    assert field.digest["role"] == "holder"
    assert field.identity["role"] == "primary"
    assert field.partition.transform == "year"

    # It is a view of the one metadata mapping, not a copy of part of it.
    assert field.metadata["iceberg:doc"] == "closing price"
    assert field.metadata["digest:role"] == "holder"
    assert len(field.metadata) == 9
    assert dict(field.iceberg.items())["field-id"] == "7"

    del field.iceberg["field-id"]
    assert "field-id" not in field.iceberg
    assert field.protocol("postgres")["type"] == "numeric"
    ```

    !!! note "Rust-only"
        The per-protocol view types (`HttpField`, `IcebergField`, `FixField`, `DigestField`,
        `IdentityField`, and seventeen others) are Rust-only. Python reads the generic property
        mapping through `field.iceberg`, and the validated HTTP values stay attributes on the
        field. `field.partition` and [`field.python`](../extensions/python.md#the-declaring-class)
        are the exceptions: `sources`, `transform` and
        [`apply_arrow_batch`](../holder/iobase/partitions.md#derived-partition-columns) on one,
        `class_metadata` and its three parts on the other, each answered only by its own view.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = new Field('price', 'int64', false)

    field.iceberg.set('doc', 'closing price')
    field.iceberg.update({ 'schema-id': '3', 'field-id': '7' })
    field.postgres.set('type', 'numeric')
    field.digest.set('role', 'holder')
    field.identity.update({ role: 'primary', nulls: 'distinct' })
    field.partition.update({ transform: 'year', sources: '["event"]' })

    assert.equal(field.iceberg.get('doc'), 'closing price')
    assert.equal(field.iceberg.key('doc'), 'iceberg:doc')
    assert.equal(field.iceberg.size, 3)
    assert.equal(field.mysql.size, 0)
    assert.equal(field.digest.get('role'), 'holder')
    assert.equal(field.identity.get('role'), 'primary')
    assert.equal(field.partition.get('transform'), 'year')

    // It is a view of the one metadata map, not a copy of part of it.
    assert.equal(field.get('iceberg:doc'), 'closing price')
    assert.equal(field.get('digest:role'), 'holder')
    assert.equal(field.size, 9)
    assert.deepEqual([...field.iceberg].sort(), [['doc', 'closing price'], ['field-id', '7'], ['schema-id', '3']])

    assert.equal(field.iceberg.delete('field-id'), true)
    assert.equal(field.iceberg.has('field-id'), false)
    assert.equal(field.protocol('postgres').get('type'), 'numeric')
    ```

    !!! note "Rust-only"
        The per-protocol view types (`HttpField`, `IcebergField`, `FixField`, `DigestField`,
        `IdentityField`, `PartitionField`, `PythonField`, and fifteen others) are Rust-only, and
        so are the partition and Python-class vocabularies Python binds. JavaScript reads the
        generic property `Map` through `field.iceberg` and `field.python`, and the validated HTTP
        values stay accessors on the field.

## Reserved keys

Typed accessors parse and canonicalize both ways.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MimeType, PythonKind, PythonMetadata, Scheme};

    let mut field = Field::new("payload", DataType::Binary, false);

    field.set_parquet_field_id(17);
    field.set_init(false);
    field.set_display("Raw payload")?;
    field.as_http_mut().set_content_type("application/json; charset=utf-8")?;
    field.set_property(&Scheme::POSTGRES, "type", "jsonb")?;
    field
        .as_python_mut()
        .set_class(&PythonMetadata::new("app.wire", "Payload", PythonKind::Dataclass)?)?;

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

    // The declaring class is one value, and the bare name is derived from the
    // qualified one rather than stored beside it.
    assert_eq!(field.as_python().class_name(), Some("Payload"));
    assert_eq!(field.get_metadata("python:kind"), Some("dataclass"));
    assert_eq!(field.as_python().import_path().as_deref(), Some("app.wire.Payload"));
    ```

=== "Python"

    ```python
    from yggdryl import Field, MimeType, PythonMetadata

    field = Field("payload", "binary", nullable=False)

    field.set_parquet_field_id(17)
    field.metadata["field:init"] = "false"
    field.set_display("Raw payload")
    field.set_content_type("application/json; charset=utf-8")
    field.set_property("postgres", "type", "jsonb")
    field.python.class_metadata = PythonMetadata("app.wire", "Payload", "dataclass")

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

    # The declaring class is one value, and the bare name is derived from the
    # qualified one rather than stored beside it.
    assert field.python.class_name == "Payload"
    assert field.metadata["python:kind"] == "dataclass"
    assert field.python.import_path == "app.wire.Payload"
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
    field.python.update({ kind: 'dataclass', module: 'app.wire', qualname: 'Payload' })

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

    // The declaring class a Python schema carries reads back by name here; the
    // typed vocabulary over it is Rust and Python only.
    assert.equal(field.python.get('qualname'), 'Payload')
    assert.equal(field.get('python:kind'), 'dataclass')
    ```

## Views

Typed vocabulary lives on the view, never on `Field`. The one exception is
[`Field::apply_arrow_batch`](field.md#applying-a-schemas-declarations), which owns no
vocabulary of its own: it asks each declaring protocol in turn, in the order their answers
depend on.

| View | Vocabulary |
| --- | --- |
| `HttpField`, `HttpFieldMut` | `content_type`, `content_length`, `mime_type`, `media_type`, `location` |
| [`IcebergField`, `IcebergFieldMut`](../media/iceberg/schema.md) | `doc`, `schema_id`, `spec_id`, `transform` |
| [`FixField`, `FixFieldMut`](../fix/index.md) | `branch`, `id`, `tag`, `tags`, `aliases`, `description` |
| [`DigestField`, `DigestFieldMut`](../xxhash/values.md#filling-digest-holders) | `is_holder`, `algorithm`, `sources`, `apply_arrow_batch`, and their setters; [`time`, `unit`, `is_coupled`](../txhash/arrow.md#coupled-holders) and their setters |
| `IdentityField` | no typed vocabulary: arbitrary inert text under `identity:` |
| [`PartitionField`, `PartitionFieldMut`](../holder/iobase/partitions.md#derived-partition-columns) | `sources`, `transform`, `expression`, `apply_arrow_batch`, and the two setters |
| [`PythonField`, `PythonFieldMut`](../extensions/python.md#the-declaring-class) | `class`, `module`, `qualname`, `class_name`, `kind`, `import_path`, and their setters |

## Digest holders and their sources

`digest:role` has one value, `holder`: it says a field *stores* a row digest. What that digest
reads is named on the holder as `digest:sources`, so the fields it reads carry no metadata at all.
`["*"]` - which is also what naming nothing means - selects every field of the Struct except a
holder, in declaration order.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let id = Field::new("id", DataType::Int64, false);
    let price = Field::new("price", DataType::Float64, false);
    let mut stored = Field::new("row_digest", DataType::UInt64, false);
    stored.as_digest_mut().set_holder()?;

    let fallback = DataType::from_fields([id.clone(), price.clone(), stored.clone()])?
        .required_field("row");
    assert_eq!(fallback.digest_field_names().collect::<Vec<_>>(), ["id", "price"]);

    // Narrowing the input is the holder's business; `id` stays an ordinary column.
    let mut narrowed = stored;
    narrowed.as_digest_mut().set_sources(["id"])?;
    assert_eq!(narrowed.as_digest().sources()?, Some(vec!["id".to_owned()]));
    assert!(id.as_digest().is_empty());

    let explicit = DataType::from_fields([id, price, narrowed])?.required_field("row");
    assert_eq!(explicit.digest_field_names().collect::<Vec<_>>(), ["id", "price"]);
    assert_eq!(explicit.only_digest_fields()?.field_len(), 2);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    identifier = Field("id", "int64", nullable=False)
    price = Field("price", "float64", nullable=False)
    stored = Field("row_digest", "uint64", nullable=False)
    stored.digest["role"] = "holder"

    fallback = Field(
        "row", DataType.from_fields([identifier, price, stored]), nullable=False
    )
    assert fallback.digest_field_names == ["id", "price"]

    # Narrowing the input is the holder's business; `id` stays an ordinary column.
    stored.digest["sources"] = '["id"]'
    assert dict(identifier.digest) == {}

    explicit = Field(
        "row", DataType.from_fields([identifier, price, stored]), nullable=False
    )
    assert explicit.digest_field_names == ["id", "price"]
    assert len(explicit.only_digest_fields().dtype) == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const identifier = new Field('id', 'int64', false)
    const price = new Field('price', 'float64', false)
    const stored = new Field('row_digest', 'uint64', false)
    stored.digest.set('role', 'holder')

    const fallback = new Field(
      'row', DataType.fromFields([identifier, price, stored]), false,
    )
    assert.deepEqual(fallback.digestFieldNames(), ['id', 'price'])

    stored.digest.set('sources', '["id"]')
    assert.deepEqual(identifier.digest.entries(), [])

    const explicit = new Field(
      'row', DataType.fromFields([identifier, price, stored]), false,
    )
    assert.deepEqual(explicit.digestFieldNames(), ['id', 'price'])
    assert.equal(explicit.onlyDigestFields().dtype.length, 2)
    ```

| key | rule |
| --- | --- |
| Namespace | `Scheme::DIGEST`; `digest_fields`, `digest_field_names`, `digest_field_len`, `only_digest_fields` |
| Selection | direct Struct children, in declaration order |
| `DigestField` | `is_holder`, `algorithm`, `sources`, `apply_arrow_batch` |
| `DigestFieldMut` | `set_holder`, `set_algorithm`, `remove_algorithm`, `set_sources`, `remove_sources`, `remove_role` |
| `digest:sources` | holder-local ordered JSON array; `["*"]` and absence both select every non-holder, `[]` selects nothing, `"*"` may not travel beside a path |
| `digest:algorithm` | optional canonical [`DigestAlgorithm`](../xxhash/index.md); its width must match the storage |
| Widths | XXH32: `int32`, `uint32`; XXH64 and XXH3-64: `int64`, `uint64`; XXH3-128: `fixed_size_binary(16)` |
| Signed storage | the same digest bits, never a checked numeric conversion |

Typed setters require `holder`, validate the width, and fail atomically; generic metadata writes
canonicalize the algorithm token and the path JSON. Arrow row hashing reads the same contract in
[`row_digests`](../xxhash/values.md), and path resolution, nested-holder reuse, algorithm fallback,
and batch filling live with [xxHash](../xxhash/index.md).

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

Folder writes and reads and Iceberg identity specs read the mark: [Partitions](../holder/iobase/partitions.md), [Iceberg](../media/iceberg/index.md).

## Edges

- `"+00017"` to `PARQUET:field_id` -> stored as `"17"`; `"2147483648"` -> refused.
- `HTTPS:Content-Type`, `HTTP:content-type`, `http:content-type` -> one entry, matched case-insensitively.
- `https` -> no accessor; either scheme's view reports `http`.
- Rust `field.location()` -> straight `location`; `as_http().location()` -> `http:location` (`http_location` / `httpLocation` in the bindings).
- `with_init` -> Rust only; `set_init` and `is_init` are in Python, and the bindings' mapping write validates identically.
- `display` -> named in all three (`set_display`, `display`, `remove_display`) on the field and every view; `try_with_display` is Rust only.
- Deleting a protocol's namespace -> leaves `Field`'s own reserved state untouched.
- Protocol write -> invalidates a populated Arrow projection, like a direct metadata write.
- Non-partition column -> no `field:partition` key; schemas partitioned alike compare equal.
- `field:partition` -> travels into Arrow, Parquet footers, and JSON round trips.
- `digest:role` other than `holder` -> refused, and the failed write leaves the field unchanged.
- `digest:sources` on a field that is not a holder -> refused when the plan is built; a source names a field, it never marks one.
- A root whose children are all holders -> no digest values, the empty ordered sequence.
- Changing or removing a holder role -> refused until `digest:algorithm` and `digest:sources` are gone.
- `identity:` properties -> inert text; `field:partition` stays the marker `partition_fields` reads.
- `partition:transform` -> parsed on read, never on write: a generic `update` stores any text, and the typed reader is what names the vocabulary it is not in.
- `partition:transform` without a `partition:sources` -> refused by `expression`, naming the key it wants beside it.
- `apply_arrow_batch` -> the one verb both declaring protocols answer; each walks the Structs it declares and leaves a written value alone.

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
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="HTTP|protocol|partition|typed names" node/tests/types/field.test.js
    npm run --prefix node bench:types
    ```
