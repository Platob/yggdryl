# Field

`yggdryl::field` is the one schema value: a name, a datatype, a nullability flag, and metadata.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let field = Field::new("price", DataType::from_str("decimal(18, 6)")?, false);

    assert_eq!(field.name(), "price");
    assert_eq!(field.data_type(), &DataType::decimal(18, 6)?);
    assert!(!field.is_nullable());
    assert!(field.is_metadata_empty());

    // The canonical text round-trips, and shorthand parses into the same value.
    assert_eq!(Field::from_str(&field.to_string())?, field);
    assert_eq!(Field::from_str("price decimal(18, 6) NOT NULL")?, field);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    field = Field("price", "decimal(18, 6)", nullable=False)

    assert field.name == "price"
    assert field.data_type == DataType("decimal(18, 6)")
    assert field.nullable is False
    assert len(field) == 0

    assert Field.from_str(str(field)) == field
    assert Field.from_str("price decimal(18, 6) NOT NULL") == field
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const field = new Field('price', 'decimal(18, 6)', false)

    assert.equal(field.name, 'price')
    assert.ok(field.dataType.equals(DataType.from('decimal(18, 6)')))
    assert.equal(field.nullable, false)
    assert.equal(field.size, 0)

    assert.ok(Field.from(field.toString()).equals(field))
    assert.ok(Field.from('price decimal(18, 6) NOT NULL').equals(field))
    ```

A field owns exactly those four things. Everything else on the type is a view over them: a typed
accessor for a reserved metadata key, an [Arrow](arrow.md) projection, a comparison, a cast. There
is no separate schema type anywhere in the library, so learning this page is learning the schema
model.

`Field::new` takes a [`DataType`](datatype.md) and validates nothing; `Field::from_parts` takes a
metadata snapshot as well and validates the whole value. The bindings have a single constructor that
takes the metadata inline and always validates, and it infers the datatype from whatever is in that
position - a string in any accepted syntax, a native `DataType`, and in Python a PyArrow type.

## A non-null struct field is the schema

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("trade");

    schema.validate_struct_root()?;
    assert_eq!(schema.field_len(), 2);
    assert_eq!(schema.index_of("symbol"), Some(1));
    assert_eq!(schema.get_field_by_name("id").map(Field::name), Some("id"));

    // A nullable root is not a schema: a whole row cannot be logically absent.
    assert!(schema.with_nullable(true).validate_struct_root().is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, fields

    schema = Field(
        "trade",
        DataType.from_fields([
            fields.int64("id", nullable=False),
            fields.utf8("symbol"),
        ]),
        nullable=False,
    )

    children = schema.data_type
    assert len(children) == 2
    assert "symbol" in children
    assert children["id"].nullable is False
    assert children[1].name == "symbol"
    assert [child.name for child in children] == ["id", "symbol"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fields } = require('yggdryl')

    const schema = new Field(
      'trade',
      DataType.fromFields([
        fields.int64('id', { nullable: false }),
        fields.utf8('symbol', { nullable: true }),
      ]),
      false,
    )

    const children = schema.dataType
    assert.equal(children.length, 2)
    assert.equal(children.at(0).nullable, false)
    assert.equal(children.getByName('symbol').name, 'symbol')
    assert.deepEqual(children.keys(), ['id', 'symbol'])
    ```

The columns of a table are the children of a struct field, so a schema is a field whose datatype is
a struct and whose `nullable` is false. `validate_struct_root` is the check every reader and writer
makes: `require_struct` accepts a nullable struct, because a nullable struct is a perfectly good
*column*, while a nullable *root* would make an entire row absent and no row-oriented reader can
represent that. This root is what [`ipc`](ipc.md), [`parquet`](parquet.md), and
[`iceberg`](iceberg.md) take and return.

Rust reaches the children through the field - `fields`, `field_len`, `get_field`,
`get_field_by_name`, `index_of` - because a struct root is the common case. Python and JavaScript
reach them through `data_type`, where the same children are a sequence: `len`, indexing by position
or name, `in`, and iteration in Python; `length`, `at`, `getByName`, and `keys` in JavaScript.

### Item access reaches a child, never metadata

Subscripting a `Field` or a `DataType` means one thing: reach a nested child. A `str` is a child
name, an `int` is a position, and `len`, iteration, and membership all speak children. Both classes
answer identically, so a caller walking one object graph never gets a child from one node and a
metadata string from the next. Metadata is reached through [its own view](#metadata-is-a-mapping) -
`field.metadata[...]` in Python, `get_metadata` and friends in Rust - because a view whose keys *are*
keys is where item syntax legitimately means "a key".

Chained subscripts are the nesting story - `order["line"]["price"]` descends two levels, because each
subscript answers a node that subscripts again. There is no dotted-string or tuple path form.

Assignment is dict-like *by name* and list-like *by position*: a known name is replaced in place
keeping its position, an **unknown name appends** a new child, and a position only ever replaces -
past the end is an error, never a silent grow. `del` removes and closes the gap by either form. In
Python this routes through the core's cache-aware child mutation, which is also why a `DataType` -
immutable and hashable - refuses assignment and points at the `Field` that carries it.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let mut order = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([DataType::Float64.required_field("price")])?
            .required_field("line"),
    ])?
    .required_field("order");
    order.insert_metadata("owner", "trading")?;

    // A child by name, by position, and two levels down.
    assert_eq!(order["id"].data_type(), &DataType::Int64);
    assert_eq!(order[1].name(), "line");
    assert_eq!(order["line"]["price"].data_type(), &DataType::Float64);

    // An unknown name appends; a position replaces.
    order.set_field_by_name("venue", DataType::Utf8.nullable_field("venue"))?;
    assert_eq!(order.field_len(), 3);
    order.set_field(0, DataType::Utf8.required_field("id"))?;
    assert_eq!(order["id"].data_type(), &DataType::Utf8);
    assert_eq!(order.remove_field_by_name("venue")?.name(), "venue");

    // Metadata keeps its own named surface.
    assert_eq!(order.get_metadata("owner"), Some("trading"));
    assert!(order.get_field_by_name("owner").is_none());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    order = Field(
        "order",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field(
                "line",
                DataType.from_fields([Field("price", "float64", nullable=False)]),
                nullable=False,
            ),
        ]),
        nullable=False,
        metadata={"owner": "trading"},
    )

    # A child by name, by position, negatively, and two levels down.
    assert order["id"].data_type == DataType("int64")
    assert order[-1].name == "line"
    assert order["line"]["price"].data_type == DataType("float64")

    # The DataType answers the same way, and children drive len/iter/in.
    assert order.data_type["id"].name == "id"
    assert len(order) == 2
    assert [child.name for child in order] == ["id", "line"]
    assert "line" in order

    # An unknown name appends; a position replaces only.
    order["venue"] = Field("venue", "utf8")
    assert len(order) == 3
    order[0] = Field("id", "utf8", nullable=False)
    assert order["id"].data_type == DataType("utf8")
    del order["venue"]
    assert len(order) == 2

    # Metadata is reached through its view, never by subscripting the node.
    assert order.metadata["owner"] == "trading"
    try:
        order["owner"]
    except KeyError:
        pass
    ```

=== "JavaScript"

    !!! note "Rust first"
        The JavaScript binding reaches children through `dataType` with `at`, `getByName`, and
        `keys`; the shared subscript vocabulary lands with the rest of the lifecycle surface.


## Metadata is a mapping

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let mut field = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;
    field.insert_metadata("currency", "EUR")?;
    field.update_metadata([("source", "exchange")])?;

    assert_eq!(field.metadata_len(), 3);
    assert_eq!(field.get_metadata("venue"), Some("XPAR"));
    assert!(field.has_metadata("currency"));
    assert_eq!(
        field.metadata_iter().collect::<Vec<_>>(),
        [("currency", "EUR"), ("source", "exchange"), ("venue", "XPAR")]
    );
    assert_eq!(field.remove_metadata("venue").as_deref(), Some("XPAR"));
    ```

=== "Python"

    ```python
    from yggdryl import Field

    field = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})
    # Metadata lives on `field.metadata`, a live mapping view. Subscripting the
    # field itself reaches a nested *child*, not a metadata key.
    field.metadata["currency"] = "EUR"
    field.metadata.update(source="exchange")

    assert len(field.metadata) == 3
    assert "venue" in field.metadata
    assert field.metadata["venue"] == "XPAR"
    assert field.metadata.get("missing") is None
    assert list(field.metadata.items()) == [
        ("currency", "EUR"),
        ("source", "exchange"),
        ("venue", "XPAR"),
    ]

    del field.metadata["venue"]
    assert list(field.metadata.keys()) == ["currency", "source"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = new Field('price', 'float64', false, { venue: 'XPAR' })
    field.set('currency', 'EUR')
    field.update(new Map([['source', 'exchange']]))

    assert.equal(field.size, 3)
    assert.equal(field.has('venue'), true)
    assert.equal(field.get('venue'), 'XPAR')
    assert.equal(field.get('missing'), null)
    assert.deepEqual([...field], [
      ['currency', 'EUR'],
      ['source', 'exchange'],
      ['venue', 'XPAR'],
    ])

    assert.equal(field.delete('venue'), true)
    assert.deepEqual(field.keys(), ['currency', 'source'])
    ```

Keys are strings, values are strings, and iteration is in lexical key order, so two independently
built fields with the same entries compare and hash identically. Clones share one metadata map until
a write forces a copy.

Every write validates before it changes anything. `set_metadata` and `update_metadata` in Rust,
`update` in the bindings, build and check the whole batch first, so a bad entry in the middle of a
thousand leaves the field exactly as it was. In Python a field's `hash` is its layout hash, so a
field stays findable in a `dict` across metadata edits.

## Reserved keys and protocol properties

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MimeType, Scheme};

    let mut field = Field::new("payload", DataType::Binary, false);

    field.set_parquet_field_id(17);
    field.set_init(false);
    field.set_content_type("application/json; charset=utf-8")?;
    field.set_property(&Scheme::POSTGRES, "type", "jsonb")?;

    assert_eq!(field.parquet_field_id()?, Some(17));
    assert_eq!(field.get_metadata("PARQUET:field_id"), Some("17"));
    assert!(!field.is_init()?);
    assert_eq!(field.get_metadata("field:init"), Some("false"));

    // An http: property answers to either scheme and to a raw key lookup.
    assert_eq!(field.mime_type()?, MimeType::JSON);
    assert_eq!(
        field.get_property(&Scheme::HTTPS, "Content-Type"),
        field.content_type()
    );
    assert_eq!(field.get_metadata("http:content-type"), field.content_type());
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
    field.set_content_type("application/json; charset=utf-8")
    field.set_property("postgres", "type", "jsonb")

    assert field.parquet_field_id == 17
    assert field.metadata["PARQUET:field_id"] == "17"
    assert field.metadata["field:init"] == "false"

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
    field.setContentType('application/json; charset=utf-8')
    field.setProperty('postgres', 'type', 'jsonb')

    assert.equal(field.parquetFieldId, 17)
    assert.equal(field.get('PARQUET:field_id'), '17')
    assert.equal(field.get('field:init'), 'false')

    assert.ok(field.mimeType.equals(MimeType.JSON))
    assert.equal(field.getProperty('https', 'Content-Type'), field.contentType)
    assert.equal(field.get('http:content-type'), field.contentType)
    assert.deepEqual(field.propertyIter('postgres'), [{ key: 'type', value: 'jsonb' }])
    ```

A handful of keys mean something to the library, and each one has a typed accessor that parses and
canonicalizes on the way in and out. `PARQUET:field_id` is a signed 32-bit integer and is what
`parquet_field_id` reads; writing `"+00017"` through the mapping stores `"17"`, and writing
`"2147483648"` fails.
`field:init` is a reserved boolean: it is absent for an ordinary field, and setting it to `false`
marks a field a schema still declares but a constructor must not accept. `location` parses as a
[`Url`](uri.md), and `alias`, `catalog_name`, `schema_name`, and `table_name` carry validated text.

Anything shaped `scheme:name` is a protocol property, keyed by a known [`Scheme`](enums.md). The
prefix is canonicalized, so `HTTPS:Content-Type`, `HTTP:content-type`, and `http:content-type` are
one entry, and `get_property` matches HTTP names case-insensitively. The `http:` family is the one
with parsing accessors on top - `content_type`, `content_length`, `mime_type`, `media_type`,
`http_location` - because a field is also how a remote resource describes itself.

Setting `field:init` has named methods in Rust only (`set_init`, `is_init`, `with_init`); Python and
JavaScript write the reserved key through the mapping, which validates it exactly the same way.

## One protocol at a time

Spelling `scheme:name` at a call site means spelling it right in every branch it appears in. A
protocol view remembers the protocol instead, so the caller writes the bare name:

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scheme};

    let mut field = Field::new("price", DataType::Int64, false);

    field.iceberg_mut().insert("doc", "closing price")?;
    field.iceberg_mut().update([("schema-id", "3"), ("field-id", "7")])?;
    field.postgres_mut().insert("type", "numeric")?;

    assert_eq!(field.iceberg().get("doc"), Some("closing price"));
    assert_eq!(field.iceberg().key("doc"), "iceberg:doc");
    assert_eq!(field.iceberg().len(), 3);
    assert!(field.mysql().is_empty());

    // It is a view of the one metadata map, not a copy of part of it.
    assert_eq!(field.get_metadata("iceberg:doc"), Some("closing price"));
    assert_eq!(field.metadata_len(), 4);

    // A protocol-scoped replacement leaves every other protocol alone.
    field.iceberg_mut().set([("doc", "close")])?;
    assert_eq!(field.iceberg().iter().collect::<Vec<_>>(), [("doc", "close")]);
    assert_eq!(field.postgres().get("type"), Some("numeric"));

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

The view is a borrow, not a snapshot: it reads out of the field's own metadata and writes through the
field's own cache-aware mutation, so two views of one field see each other's writes and a protocol
write invalidates a populated Arrow projection exactly as a direct metadata write does. Every
well-known protocol has a named accessor - `iceberg`, `postgres`, `http`, `arrow`, `spark`, `s3`, and
the rest of the [`Scheme`](enums.md) vocabulary - and `protocol` takes one that is only known at
runtime. There is no `https` accessor, because HTTPS shares the canonical `http:` namespace; the view
for either scheme reports `http` as its prefix.

Rust's `set` is the one operation that is not a plain map write: it replaces exactly this protocol's
properties and leaves every other key untouched, which is what a protocol-scoped assignment has to
mean when one map holds them all. The bindings expose the mapping and `update` but not that
replacement, for the same reason they expose no whole-metadata `set`: in Python `set` on a mapping
means one key, and in JavaScript `Map.set` does too.

## A field can be a partition column

Nothing in a batch says which of its columns belong in a directory name, so a schema that means to be
stored partitioned says so on the columns themselves:

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
    assert!(schema.get_field_by_name("year").expect("the column").is_partition());

    // The two halves of the layout: what a path spells, and what a leaf stores.
    assert_eq!(schema.without_partition_fields()?.field_len(), 1);
    assert_eq!(schema.only_partition_fields()?.field_len(), 2);

    // The mark is reserved metadata, so it round-trips like any other.
    assert_eq!(
        schema.get_field_by_name("year").expect("the column").get_metadata("field:partition"),
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
    assert schema.data_type["year"].is_partition
    assert not schema.data_type["price"].is_partition

    assert len(schema.without_partition_fields().data_type) == 1
    assert len(schema.only_partition_fields().data_type) == 2
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
    assert.equal(schema.dataType.getByName('year').isPartition, true)
    assert.equal(schema.dataType.getByName('price').isPartition, false)

    assert.equal(schema.withoutPartitionFields().dataType.length, 1)
    assert.equal(schema.onlyPartitionFields().dataType.length, 2)
    ```

The mark is the reserved `field:partition` key, so it travels wherever field metadata travels - into
Arrow, into a Parquet footer, through a JSON round trip - and a field that is not a partition column
carries no marker at all, which keeps two schemas that partition the same way exactly equal. A folder
write reads the marks to lay an empty tree out, a folder read puts them back on the columns it
restored from the path, and Iceberg builds an identity spec from them; that whole story is in
[storage](io.md#partition-columns-in-the-data) and [Iceberg](iceberg.md#partition-specs-and-the-hive-layout).

## Typed field aliases

=== "Rust"

    ```rust
    use yggdryl::field::{Int64Field, TimestampField, Utf8Field, integer};
    use yggdryl::{DataType, Field, TimeUnit};

    let id = Int64Field::new("id", false);
    let symbol = Utf8Field::from_parts("symbol", true, [("source", "feed")])?;
    let at = TimestampField::try_new("at", DataType::Timestamp(TimeUnit::Microsecond, None), false)?;

    // A typed field derefs to the field it wraps.
    assert_eq!(id.name(), "id");
    assert_eq!(symbol.get_metadata("source"), Some("feed"));
    assert_eq!(at.data_type().to_string(), "timestamp(us)");

    // The marker is checked, never assumed.
    assert!(
        Field::new("id", DataType::Utf8, false)
            .try_into_typed::<integer::Int64>()
            .is_err()
    );
    assert_eq!(id.into_field().data_type(), &DataType::Int64);
    ```

=== "Python"

    ```python
    from yggdryl import Field, fields

    id_field = fields.int64("id", nullable=False)
    symbol = fields.utf8("symbol", metadata={"source": "feed"})
    at = fields.timestamp("at", "us", nullable=False)

    assert isinstance(id_field, Field)
    assert str(id_field.data_type) == "int64"
    assert symbol.metadata["source"] == "feed"
    assert str(at.data_type) == "timestamp(us)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const id = fields.int64('id')
    const symbol = fields.utf8('symbol', { nullable: true, metadata: { source: 'feed' } })
    const at = fields.timestamp('at', 'us')

    assert.ok(id instanceof Field)
    assert.equal(id.dataType.toString(), 'int64')
    assert.equal(symbol.get('source'), 'feed')
    assert.equal(at.dataType.toString(), 'timestamp(us)')
    ```

`Int64Field` and the forty aliases beside it are `TypedField<K>`, one `Field` plus a zero-sized sealed
marker, `repr(transparent)` and exactly the size of the field it holds. The marker constrains the
variant only: a decimal's precision, a timestamp's unit, a list's child all stay in the wrapped
field, so the typed view never duplicates schema state. `try_as_typed` borrows a `TypedFieldRef`
without allocating, `try_into_typed` consumes, and there is no `DerefMut` - replacing the datatype
through a generic reference could violate `K`, so `set_data_type` on a typed field re-checks the
marker and leaves the value untouched when it fails.

Aliases with a statically known datatype get a `new(name, nullable)` that cannot fail, plus
`from_parts(name, nullable, metadata)`; parameterized ones take the datatype through `try_new`. In
Python and JavaScript the aliases are static views over the same native class: `yggdryl.fields.int64`
returns an ordinary `Field`, typed as `Int64Field` for a checker only. Watch the default -
`nullable` defaults to `True` in Python and to `false` in JavaScript.

The three newest datatypes -
[variant, geometry, and geography](datatype.md#variant-geometry-and-geography) - follow the same
pattern: `yggdryl::field::VariantField` is parameterless and gets the static `new(name, nullable)`,
while `GeometryField` and `GeographyField` are parameterized by CRS and edge algorithm, so they
take their datatype through `try_new`. The binding-side `VariantField`, `GeometryField`, and
`GeographyField` aliases beside `fields.variant`, `fields.geometry`, and `fields.geography` are
checker-level views over the one native class exactly like every alias above.

## Row values are validated against the root

Validating and canonicalizing a [`Value`](generic.md) against a struct root is Rust-only. Python and
JavaScript reconcile Arrow data instead.

```rust
use yggdryl::{DataType, Field, Value};

let schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Float32.nullable_field("price"),
])?
.required_field("trade");

// A row is one ordered sequence with one value per struct child.
let row = Value::from_sequence([Value::from(7u64), Value::from(0.1f64)]);
schema.validate_value(&row)?;

// Canonicalizing narrows every value into the representation the root declares.
let canonical = schema.canonicalize_value(row)?;
assert_eq!(canonical.get(0), Some(&Value::I64(7)));
assert_eq!(
    canonical.get(1).and_then(Value::as_f64),
    Some(f64::from(0.1f32))
);

// A value that does not fit names the path walked to reach it.
let wrong = Value::from_sequence([Value::from("seven"), Value::Null]);
let message = schema.validate_value(&wrong).unwrap_err().to_string();
assert!(message.contains("$.trade.id"), "{message}");
```

The two calls answer different questions. `validate_value` asks whether the row is *representable*:
right arity, no null in a required column, every scalar inside its declared range. It accepts a
`U64` where an `Int64` is declared, because the value fits. `canonicalize_value` then rewrites the
row into the exact representation - that `U64` becomes an `I64`, an `f64` bound for a `Float32`
column is rounded through `f32` - and returns the input untouched when nothing needed changing, so
a correctly built row costs nothing. Both walk the schema, not the value, and both report the
dot/bracket path of the first thing that does not fit.

## Serializing a schema

`Field` reads and writes the three structured-text formats through **one** structural model. There
is exactly one `Field` ⇄ `Value` mapping - `to_value`/`from_value` in Rust, `to_dict`/`from_dict` in
Python - and JSON, YAML, and TOML are three writers over it, so the three agree by construction
rather than by three sets of tests. That is also what makes a schema *embeddable*: a configuration
document can carry a declared schema inline beside the rest of its settings, with no
JSON-string-inside-YAML awkwardness.

The shape is what the JSON emit has always been: `name`, `data_type`, `nullable`, then
`dictionary_id` only when it is non-zero and `dictionary_is_ordered` only when it is set, then
`metadata`. An unset optional attribute is **omitted**, never emitted as null - which is also why
TOML, which has no null, loses nothing on the way out.

Each format takes the shared [`Formatting`](text.md#laying-out-a-dump) option; Python spells it as an
`indent` keyword.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};
    use yggdryl::generic::Value;

    let field = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;

    // One structural model, three formats over it.
    assert_eq!(Field::from_value(field.to_value())?, field);
    assert_eq!(Field::from_json(&field.to_json()?)?, field);
    assert_eq!(Field::from_yaml(&field.to_yaml()?)?, field);
    assert_eq!(Field::from_toml(&field.to_toml()?)?, field);

    // The mapping is the shared `Value`, so it drops into any document.
    let shape = field.to_value();
    assert_eq!(shape.get_key_str("name").and_then(Value::as_str), Some("price"));
    // Unset optional attributes are absent rather than null.
    assert!(shape.get_key_str("dictionary_id").is_none());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    field = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})

    assert Field.from_dict(field.to_dict()) == field
    assert Field.from_json(field.to_json()) == field
    assert Field.from_yaml(field.to_yaml()) == field
    assert Field.from_toml(field.to_toml()) == field

    shape = field.to_dict()
    assert shape["name"] == "price"
    assert "dictionary_id" not in shape
    ```

=== "JavaScript"

    !!! note "Rust first"
        The YAML and TOML pair lands in the JavaScript binding once the core surface settles;
        `toJSON` is already there.

## A readable rendering

`Display` - and Python's `str`/`repr` - is the compact constructor form, and it stays exactly as it
is: it round-trips through `from_str`, and the error messages, the documentation, and Python's
`repr` all depend on that. It is also unreadable the moment a struct nests three levels deep.

The readable form is the **alternate**: `{:#}` in Rust, or the named `pretty()` adapter that backs
it, and `pretty()` in Python. One fact per line, one indent per nesting level, and only the
attributes that are actually set - a `dictionary_id` of `0` or empty metadata is noise the compact
form already omits. Metadata renders as indented `@key = value` lines rather than one braced blob.
The output is stable across runs; nothing in it iterates a hash map.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let order = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([DataType::Float64.required_field("price")])?
            .nullable_field("line"),
    ])?
    .required_field("order");

    // Compact still round-trips.
    assert_eq!(Field::from_str(&order.to_string())?, order);

    // Readable is the alternate, or the named adapter - one implementation.
    assert_eq!(format!("{order:#}"), order.pretty().to_string());
    assert_eq!(
        format!("{order:#}"),
        concat!(
            "order: struct[2], required\n",
            "  id: int64, required\n",
            "  line: struct[1], nullable\n",
            "    price: float64, required",
        ),
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    order = Field(
        "order",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field(
                "line",
                DataType.from_fields([Field("price", "float64", nullable=False)]),
            ),
        ]),
        nullable=False,
    )

    # `repr` is unchanged - the eval-round-trip form Python expects.
    assert repr(order).startswith("Field.from_str(")
    assert Field.from_str(str(order)) == order

    assert order.pretty() == (
        "order: struct[2], required\n"
        "  id: int64, required\n"
        "  line: struct[1], nullable\n"
        "    price: float64, required"
    )
    ```

=== "JavaScript"

    !!! note "Rust first"
        `pretty` lands in the JavaScript binding once the core surface settles.


## Comparing two fields

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let left = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;
    let right = Field::from_parts("price", DataType::Float64, true, [("venue", "XNAS")])?;

    assert!(!left.equals(&right, true));
    assert_eq!(
        left.show_diffs(&right, true, false).collect::<Vec<_>>(),
        [
            "≠ $.nullable: false → true",
            "≠ $.metadata[\"venue\"]: \"XPAR\" → \"XNAS\"",
        ]
    );
    assert_eq!(left.show_diff(&left, true, true), "✓ equal");
    assert_eq!(left.show_diff(&left, true, false), "");
    ```

=== "Python"

    ```python
    from yggdryl import Field

    left = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})
    right = Field("price", "float64", metadata={"venue": "XNAS"})

    assert not left.equals(right)
    assert list(left.show_diffs(right)) == [
        "≠ $.nullable: false → true",
        '≠ $.metadata["venue"]: "XPAR" → "XNAS"',
    ]
    assert left.show_diff(left) == "✓ equal"
    assert left.show_diff(left, return_equal=False) == ""
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const left = new Field('price', 'float64', false, { venue: 'XPAR' })
    const right = new Field('price', 'float64', true, { venue: 'XNAS' })

    assert.equal(left.equals(right), false)
    assert.deepEqual([...left.showDiffs(right)], [
      '≠ $.nullable: false → true',
      '≠ $.metadata["venue"]: "XPAR" → "XNAS"',
    ])
    assert.equal(left.showDiff(left), '✓ equal')
    assert.equal(left.showDiff(left, true, false), '')
    ```

`equals` answers yes or no and takes `with_metadata`, which drops metadata from the comparison at
every depth rather than only at the root. `show_diffs` answers *why*, one line at a time: it is a
lazy iterator (`Differences` in Rust, borrowing both sides; `OwnedDifferences` when the lines must
outlive them), so a thousand-key metadata difference streams instead of building a report.
`show_diff` joins the lines into one string.

Both diff calls take `return_equal`, which decides what an equal comparison reports: nothing at all,
or exactly one `✓ equal` line. `show_diffs` defaults it to false and `show_diff` to true in the
bindings, which is why an equal `show_diff` prints a marker and an equal `show_diffs` yields
nothing. Paths are `$`-rooted and name the part that changed - `$.nullable`, `$.data_type.length`,
`$.metadata["venue"]`, `$.fields[2]` - so a diff line is a place, not a prose sentence. The same
two calls exist on [`DataType`](datatype.md) with the same output.

## Casting Arrow data through a field

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int64Array, StringArray};
    use yggdryl::field::Int64Field;
    use yggdryl::{ArrowCast, DataType, Field};

    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));

    // Any field answers with an ArrayRef, because any field could be any datatype.
    let field = Field::new("id", DataType::Int64, false);
    let cast = field.cast_arrow_array(Arc::clone(&text), false)?;
    assert_eq!(cast.data_type(), &arrow_schema::DataType::Int64);

    // A typed field already knows its variant, so it answers with the array itself.
    let typed = Int64Field::new("id", false);
    let ids: Int64Array = typed.cast_arrow_array(text, false)?;
    assert_eq!(ids.values(), &[1, 2]);

    // safe nulls a failed conversion; a non-null field then defaults it.
    let broken: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));
    assert!(typed.cast_arrow_array(Arc::clone(&broken), false).is_err());
    let repaired: Int64Array = typed.cast_arrow_array(broken, true)?;
    assert_eq!(repaired.values(), &[1, 0]);
    assert_eq!(repaired.null_count(), 0);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field

    field = Field("id", "int64", nullable=False)

    ids = field.cast_arrow_array(pa.array(["1", "2"]))
    assert ids.equals(pa.array([1, 2], type=pa.int64()))

    # safe nulls a failed conversion; a non-null field then defaults it.
    repaired = field.cast_arrow_array(pa.array(["1", "not a number"]))
    assert repaired.equals(pa.array([1, 0], type=pa.int64()))
    assert repaired.null_count == 0

    try:
        field.cast_arrow_array(pa.array(["1", "not a number"]), safe=False)
    except ValueError:
        pass
    else:
        raise AssertionError("an unsafe cast must fail")
    ```

The field is always the *target*: an incoming array is reconciled to the field's datatype and
nullability, never the other way around. `ArrowCast` is implemented for both `Field` and
[`DataType`](datatype.md) and returns an `ArrayRef`, because a generic field could be any datatype.
A `TypedField` has already committed to a variant, so `Int64Field::cast_arrow_array` returns an
`Int64Array` and the caller reads values without a downcast; `cast_arrow_scalar` does the same for a
one-element array. A few variants keep an `ArrayRef` return - a timestamp's unit and a dictionary's
key type decide the physical array, so there is no single concrete type to name.

`safe` is Arrow's own cast option. When it is true a supported conversion failure becomes null, and
a non-nullable field then replaces that null with its canonical default (`Field::default_value`);
when it is false the failure is an error. A nullable field keeps the null either way.

The same trait reconciles a whole record batch to a struct root.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::{ArrowCast, DataType, Field};

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("trade");

    let source = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            ArrowField::new("symbol", ArrowDataType::Utf8, true),
            ArrowField::new("id", ArrowDataType::Int32, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["ACME"])),
            Arc::new(Int32Array::from(vec![7])),
        ],
    )?;

    let batch = schema.cast_arrow_batch(source, false)?;
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(batch.schema().field(0).name(), "id");
    assert_eq!(batch.column(0).data_type(), &ArrowDataType::Int64);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import DataType, Field, fields

    schema = Field(
        "trade",
        DataType.from_fields([
            fields.int64("id", nullable=False),
            fields.utf8("symbol"),
        ]),
        nullable=False,
    )

    source = pa.record_batch({
        "symbol": pa.array(["ACME"]),
        "id": pa.array([7], type=pa.int32()),
    })

    batch = schema.cast_arrow_batch(source)
    assert batch.schema.names == ["id", "symbol"]
    assert batch.column("id").type == pa.int64()
    ```

Children are selected in target order by ASCII-case-insensitive name, so column order in the source
does not matter. Extra source columns are dropped, a missing nullable column is null-filled, and a
missing required column is filled with its canonical default. An already exact batch is returned
unchanged - the same arrays, not copies - which is what makes a cast safe to put in front of every
read. A `RecordBatch` is a `StructArray` plus a schema, so there is no second engine here: the batch
goes through the same recursive field cast an array does.

## The generic cast

One name, the kind inferred and kept. In Python, `field.cast_arrow(value)` takes whatever
Arrow-shaped thing you hold and hands back the same kind, cast to the field: a `pyarrow` `Scalar`,
`Array`, `ChunkedArray`, `RecordBatch`, `Table`, `RecordBatchReader`, `Dataset`, or `Scanner`; a
`polars` `DataFrame`, which crosses at the newest compat level so view arrays stay view arrays; a
`polars` `LazyFrame`, which *stays lazy* - its schema is read with `collect_schema`, which computes
no rows, and the cast is mapped over the engine's batches, so nothing is collected until you
collect; and a `pandas` `DataFrame` or `Series`, which crosses through Arrow and comes back as
itself. Streams are cast batch by batch. `field.cast(value)` is the same dispatch with plain Python
values allowed - a bare `5` becomes the typed scalar the field declares. `cast_arrow_scalar` and
`cast_arrow_record_batch` are the spelled-out single-kind names.

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    schema = Field("row", DataType("struct<id: int64, symbol: string>"), False)
    table = pa.table({"id": pa.array([1, 2], pa.int32()), "symbol": ["AAPL", "MSFT"]})

    # A table comes back a table, a reader a reader, a frame a frame.
    cast = schema.cast_arrow(table)
    assert cast.schema.field("id").type == pa.int64()

    # The generic name also takes plain values, as the typed scalar.
    price = Field("price", DataType("int64"), False)
    assert price.cast(5).as_py() == 5
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, fields } = require('yggdryl')

    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      { nullable: false },
    )
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
    })

    // Whatever Arrow JS holds casts batch by batch and comes back a Table.
    const cast = schema.castArrow(table)
    assert.equal(cast.numRows, 2)
    assert.ok(schema.cast(table).numRows === 2)
    ```

See [Python](extensions/python.md) and [JavaScript](extensions/javascript.md) for what each
binding adds on top of this field.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/field.ipynb){ download },
[Python](notebooks/python/field.ipynb){ download },
[JavaScript](notebooks/javascript/field.ipynb){ download }.

<!-- /notebooks -->
