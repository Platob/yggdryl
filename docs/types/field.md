# Field

`Field` is the one schema value: a name, a datatype, a nullability flag, and metadata.

## Contract

| | |
| --- | --- |
| Owns | name, datatype, `nullable`, metadata; no separate schema type |
| Schema root | a struct `Field` with `nullable` false; `validate_struct_root` checks it |
| Validates | `Field::new` nothing; `Field::from_parts` and the bindings' constructor everything |
| Datatype argument | bindings take text, a native `DataType`, or a PyArrow type (Python) |
| `nullable` default | Python `True`, JavaScript `true` |
| Metadata | string keys and values, lexical order; clones share one map until a write |
| Identity | equality, ordering, hashing include metadata and dictionary state |
| Serialization | one `Field` ⇄ `Scalar` mapping under JSON, YAML, TOML |
| Rendering | `Display` compact, round-trips; `{:#}` / `pretty()` readable |
| JavaScript | children through `field`, `getField`, `setField`, `removeField` rather than subscripts; JSON only, no YAML, TOML or `pretty` |

## Use

Build one, read its four parts, and round-trip the compact text.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let field = Field::new("price", DataType::from_str("decimal(18, 6)")?, false);

    assert_eq!(field.name(), "price");
    assert_eq!(field.dtype(), &DataType::decimal(18, 6)?);
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
    assert field.dtype == DataType("decimal(18, 6)")
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
    assert.ok(field.dtype.equals(DataType.from('decimal(18, 6)')))
    assert.equal(field.nullable, false)
    assert.equal(field.size, 0)

    assert.ok(Field.from(field.toString()).equals(field))
    assert.ok(Field.from('price decimal(18, 6) NOT NULL').equals(field))
    ```

## A non-null struct field is the schema

A table's columns are the children of a struct field with `nullable` false, the root every [media](../media/index.md) reader takes. `require_struct` still accepts a nullable struct column.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("trade");

    schema.validate_struct_root()?;
    assert_eq!(schema.field_len(), 2);
    assert_eq!(schema.index_of("symbol"), Some(1));
    assert_eq!(schema.get_field_by_path("id").map(Field::name), Some("id"));

    // A nullable root is not a schema: a whole row cannot be logically absent.
    assert!(schema.with_nullable(true).validate_struct_root().is_err());
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    from yggdryl import DataType, Field

    schema = Field(
        "trade",
        DataType.from_fields([
            yggdryl.int64("id", nullable=False),
            yggdryl.utf8("symbol"),
        ]),
        nullable=False,
    )

    schema.validate_struct_root()
    assert schema.index_of("symbol") == 1
    children = schema.dtype
    assert len(children) == 2 and "symbol" in children
    assert children["id"].nullable is False
    assert children[1].name == "symbol"
    assert [child.name for child in children] == ["id", "symbol"]

    # A nullable root is not a schema, but it is still a struct column.
    nullable = Field("trade", children)
    nullable.require_struct()
    with pytest.raises(ValueError):
        nullable.validate_struct_root()
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

    const children = schema.dtype
    assert.equal(children.length, 2)
    assert.equal(children.getFieldAt(0).nullable, false)
    assert.equal(children.getFieldByPath('symbol').name, 'symbol')
    assert.deepEqual(children.keys(), ['id', 'symbol'])
    ```

Each lookup exists by position, by path, or either:

| | position | path | either |
|---|---|---|---|
| raising | `field_at` | `field_by_path` | `field` |
| optional | `get_field_at` | `get_field_by_path` | `get_field` |
| replacing | `set_field_at` | `set_field_by_path` | `set_field` |
| removing | `remove_field_at` | `remove_field_by_path` | `remove_field` |

`DataType` answers the same calls, plus `fields`, `field_len`, `index_of`, and `named_field`. The [`FIELD:`](protocol.md) view is `as_field_properties`, `field_properties`, or `fieldProperties`.

## Flattening and expanding

`unnest_fields` flattens struct nesting to dotted leaf paths; `explode_fields` swaps each collection child for what it holds.

=== "Rust"

    ```rust
    use yggdryl::{DataType, StructType};

    let row = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from(StructType::from_fields([DataType::Float64.required_field("px")])?)
            .nullable_field("line"),
        DataType::list(DataType::Float64.nullable_field("item")).nullable_field("levels"),
    ])?);

    // Structs flatten to leaves; the list stays one column.
    let leaves = row.unnest_fields();
    let names: Vec<&str> = leaves.iter().map(|field| field.name()).collect();
    assert_eq!(names, ["id", "line.px", "levels"]);

    // The nullable parent makes its leaf nullable, and the name resolves.
    assert!(leaves[1].is_nullable());
    assert!(row.get_field_by_path("line.px").is_some());

    // Exploding reaches inside the collection, keeping the column's name.
    let exploded = row.explode_fields();
    assert_eq!(exploded[2].name(), "levels");
    assert_eq!(exploded[2].dtype(), &DataType::Float64);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    row = DataType("struct<id:int64 not null,line:struct<px:float64 not null>,levels:list<float64>>")

    leaves = row.unnest_fields()
    assert [leaf.name for leaf in leaves] == ["id", "line.px", "levels"]
    assert leaves[1].nullable
    assert row.get_field_by_path("line.px") is not None

    exploded = row.explode_fields()
    assert exploded[2].name == "levels"
    assert exploded[2].dtype == DataType("float64")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const row = DataType.from('struct<id:int64 not null,line:struct<px:float64 not null>,levels:list<float64>>')

    const leaves = row.unnestFields()
    assert.deepEqual(leaves.map((leaf) => leaf.name), ['id', 'line.px', 'levels'])
    assert.equal(leaves[1].nullable, true)
    assert.notEqual(row.getFieldByPath('line.px'), null)

    const exploded = row.explodeFields()
    assert.equal(exploded[2].name, 'levels')
    assert.ok(exploded[2].dtype.equals(DataType.from('float64')))
    ```

## Merging two schemas

`merge_with` is the crate's only promotion table, shared by expression typing and value inference. Rules, in order:

1. equal types are that type;
2. `null` yields to the defined side;
3. same-family nesting recurses; a struct takes the union of its fields;
4. bytes win; two byte types meet parameter by parameter - the wider offsets, the variable layout over a fixed one, no bound over a bound when widening, and the mirror when narrowing. A type storing a fixed width beside fixed bytes of that same width - a fixed string, `uuid` - keeps the storage both have: the plain bytes when widening, the side constraining them when narrowing. A numeric width never shares fixed bytes, and neither does a code, whose width bounds variable text: `int32` beside `fixed_size_binary(4)`, and `currency` beside `fixed_size_binary(3)`, are variable bytes;
5. text wins next; two strings meet leaf by leaf - the wider offsets, the variable shape over a fixed one, UTF-8 over two different charsets, no maximum over a maximum when widening, and the narrower shape, repertoire and bound when narrowing. A registered code is the `sized_ascii(n)` it stores when widening and the code itself when narrowing; text absorbing a non-text side is at least `utf8`;
6. numbers meet by width, temporals by unit; widening keeps the widest decimal backing either side declared.

Anything left is refused. Every rule answers in Python and JavaScript too; the parameter-by-parameter cases are shown once, in Rust.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let left = DataType::from(StructType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::utf8().required_field("venue"),
    ])?);
    let right = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])?);

    let merged = left.merge_with(&right, true)?;

    // The shared column widens; the two unshared ones arrive nullable.
    assert_eq!(merged["id"].dtype(), &DataType::Int64);
    assert!(merged["venue"].is_nullable());
    assert!(merged["price"].is_nullable());

    // Narrowing meets at the tightest type naming both.
    assert_eq!(
        DataType::Int32.merge_with(&DataType::Int64, false)?,
        DataType::Int32,
    );

    // Bytes win over text, and text over numbers.
    assert_eq!(DataType::utf8().merge_with(&DataType::binary(), true)?, DataType::binary());
    assert_eq!(DataType::Int64.merge_with(&DataType::utf8(), true)?, DataType::utf8());

    // Two strings, and two byte types, meet parameter by parameter.
    assert_eq!(
        DataType::from_str("utf8(8)")?.merge_with(&DataType::from_str("sized_ascii(32)")?, true)?,
        DataType::sized_utf8(32)?,
    );
    assert_eq!(
        DataType::from_str("utf8(8)")?.merge_with(&DataType::large_ascii(), true)?,
        DataType::large_utf8(),
    );
    assert_eq!(
        DataType::fixed_ascii(4)?.merge_with(&DataType::fixed_ascii(8)?, false)?,
        DataType::fixed_ascii(4)?,
    );
    assert_eq!(
        DataType::from_str("binary(16)")?.merge_with(&DataType::large_binary(), true)?,
        DataType::large_binary(),
    );
    // A fixed string and fixed bytes of one width keep that storage; a
    // number's width is its own encoding and never bytes it shares.
    assert_eq!(
        DataType::fixed_ascii(4)?.merge_with(&DataType::fixed_binary(4)?, true)?,
        DataType::fixed_binary(4)?,
    );
    assert_eq!(
        DataType::fixed_ascii(4)?.merge_with(&DataType::fixed_binary(4)?, false)?,
        DataType::fixed_ascii(4)?,
    );
    assert_eq!(
        DataType::Int32.merge_with(&DataType::fixed_binary(4)?, true)?,
        DataType::binary(),
    );

    // Narrowing keeps the tighter type: the code over the width it stores in.
    assert_eq!(DataType::Currency.merge_with(&DataType::utf8(), true)?, DataType::utf8());
    assert_eq!(DataType::Currency.merge_with(&DataType::utf8(), false)?, DataType::Currency);

    // Widening never re-encodes a decimal's storage to fit the precision.
    assert_eq!(
        DataType::decimal128(10, 2)?.merge_with(&DataType::Int16, true)?,
        DataType::decimal128(10, 2)?,
    );

    // A field merge carries nullability and metadata across.
    let a = Field::new("price", DataType::Int32, false);
    let b = Field::new("price", DataType::Int64, true);
    let field = a.merge_with(&b, true)?;
    assert_eq!(field.dtype(), &DataType::Int64);
    assert!(field.is_nullable());
    assert!(DataType::Boolean.merge_with(&DataType::Int64, true).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Field

    left = DataType("struct<id:int32 not null,venue:utf8 not null>")
    right = DataType("struct<id:int64 not null,price:float64 not null>")
    merged = left.merge_with(right)

    assert merged["id"].dtype == DataType("int64")
    assert merged["venue"].nullable and merged["price"].nullable
    assert [child.name for child in merged] == ["id", "venue", "price"]

    assert DataType("int32").merge_with("int64", upscale=False) == DataType("int32")
    assert DataType("utf8").merge_with("binary") == DataType("binary")
    assert DataType("int64").merge_with("utf8") == DataType("utf8")

    field = Field("price", "int32", nullable=False).merge_with(Field("price", "int64"))
    assert field.dtype == DataType("int64")
    assert field.nullable
    with pytest.raises(ValueError):
        DataType("boolean").merge_with("int64")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const left = DataType.from('struct<id:int32 not null,venue:utf8 not null>')
    const right = DataType.from('struct<id:int64 not null,price:float64 not null>')
    const merged = left.mergeWith(right)

    assert.ok(merged.getField('id').dtype.equals(DataType.from('int64')))
    assert.equal(merged.getField('venue').nullable, true)
    assert.equal(merged.getField('price').nullable, true)
    assert.deepEqual(merged.keys(), ['id', 'venue', 'price'])

    assert.ok(DataType.from('int32').mergeWith('int64', false).equals(DataType.from('int32')))
    assert.ok(DataType.from('utf8').mergeWith('binary').equals(DataType.from('binary')))
    assert.ok(DataType.from('int64').mergeWith('utf8').equals(DataType.from('utf8')))

    const field = new Field('price', 'int32', false).mergeWith(new Field('price', 'int64', true))
    assert.ok(field.dtype.equals(DataType.from('int64')))
    assert.equal(field.nullable, true)
    assert.throws(() => DataType.from('boolean').mergeWith('int64'))
    ```

## Item access reaches a child, never metadata

Subscripting a `Field` or a `DataType` reaches a child: a `str` is a name, an `int` a position, and `len`, iteration, and membership speak children. Metadata is reached through [its own view](#metadata-is-a-mapping).

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let mut order = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from(StructType::from_fields([DataType::Float64.required_field("price")])?)
            .required_field("line"),
    ])?)
    .required_field("order");
    order.insert_metadata("owner", "trading")?;

    // A child by name, by position, and two levels down.
    assert_eq!(order["id"].dtype(), &DataType::Int64);
    assert_eq!(order[1].name(), "line");
    assert_eq!(order["line"]["price"].dtype(), &DataType::Float64);

    // An unknown name appends; a position replaces.
    order.set_field_by_path("venue", DataType::utf8().nullable_field("venue"))?;
    assert_eq!(order.field_len(), 3);
    order.set_field(0, DataType::utf8().required_field("id"))?;
    assert_eq!(order["id"].dtype(), &DataType::utf8());
    assert_eq!(order.remove_field_by_path("venue")?.name(), "venue");

    // Metadata keeps its own named surface.
    assert_eq!(order.get_metadata("owner"), Some("trading"));
    assert!(order.get_field_by_path("owner").is_none());
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
    assert order["id"].dtype == DataType("int64")
    assert order[-1].name == "line"
    assert order["line"]["price"].dtype == DataType("float64")

    # The DataType answers the same way, and children drive len/iter/in.
    assert order.dtype["id"].name == "id"
    assert len(order) == 2
    assert [child.name for child in order] == ["id", "line"]
    assert "line" in order

    # An unknown name appends; a position replaces only.
    order["venue"] = Field("venue", "utf8")
    assert len(order) == 3
    order[0] = Field("id", "utf8", nullable=False)
    assert order["id"].dtype == DataType("utf8")
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

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const order = new Field(
      'order',
      DataType.fromFields([
        new Field('id', 'int64', false),
        new Field('line', DataType.fromFields([new Field('price', 'float64', false)]), false),
      ]),
      false,
      { owner: 'trading' },
    )

    // JavaScript has no subscripts: the same lookups are named calls.
    assert.equal(order.field('id').dtype.toString(), 'int64')
    assert.equal(order.getFieldAt(-1).name, 'line')
    assert.equal(order.field('line').field('price').dtype.toString(), 'float64')

    order.setField('venue', new Field('venue', 'utf8'))
    assert.equal(order.fieldLen, 3)
    order.setField(0, new Field('id', 'utf8', false))
    assert.equal(order.field('id').dtype.toString(), 'utf8')
    assert.equal(order.removeField('venue').name, 'venue')

    assert.equal(order.get('owner'), 'trading')
    assert.equal(order.getField('owner'), null)
    ```

| | path (`str`) | position (`int`) |
| --- | --- | --- |
| read | whole name first, then split at each `.` from the left | negative counts from the end |
| assign | replaces in place; appends when unresolved | replaces only; past the end errors |
| `del` | removes, closing the gap | removes, closing the gap |

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

Keys and values are strings in lexical key order, so equal entries compare and hash identically. Every write validates the whole batch first; a bad entry leaves the field as it was.

## The field leaves

=== "Rust"

    ```rust
    use yggdryl::{DateTimeField, DateTimeType, FieldValue as _, Int64Field, StringField, StringType};
    use yggdryl::{DataType, Field, TimeUnit, Timezone};

    let id = Int64Field::unit("id", false);
    let symbol = StringField::new("symbol", StringType::default(), true);
    let at = DateTimeField::new(
        "at",
        DateTimeType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE },
        false,
    );

    // A leaf answers its own datatype, already narrowed.
    assert_eq!(id.name(), "id");
    assert_eq!(symbol.dtype(), &DataType::utf8());
    assert_eq!(at.dtype().to_string(), "datetime64(us)");

    // The root enum is the leaves, so narrowing is a match and never a check
    // that could have been skipped: a field of another datatype is another
    // variant, and there is nothing to assume.
    let root: Field = id.into_field();
    assert_eq!(root.dtype(), &DataType::Int64);
    assert!(Int64Field::from_field(&Field::new("id", DataType::utf8(), false)).is_none());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    id_field = yggdryl.int64("id", nullable=False)
    symbol = yggdryl.utf8("symbol", metadata={"source": "feed"})
    at = yggdryl.datetime64("at", "us", nullable=False)

    assert isinstance(id_field, Field)
    assert str(id_field.dtype) == "int64"
    assert symbol.metadata["source"] == "feed"
    assert str(at.dtype) == "datetime64(us)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const id = fields.int64('id')
    const symbol = fields.utf8('symbol', { nullable: true, metadata: { source: 'feed' } })
    const at = fields.datetime64('at', 'us')

    assert.ok(id instanceof Field)
    assert.equal(id.dtype.toString(), 'int64')
    assert.equal(symbol.get('source'), 'feed')
    assert.equal(at.dtype.toString(), 'datetime64(us)')
    ```

`Int64Field` and its siblings are `FieldOf<D>`: one field carrying its family's own datatype. There is no marker to check, because `Field` is an enum over exactly these leaves - the variant *is* the proof, and the payload holds whatever parameters the family declares. `FieldValue` and `DataTypeValue` are the contracts a leaf and its payload answer - `Field` and `DataType` answer them too - declared beside the value contracts `Value` and `FamilyValue` in `rust/src/value/` and re-exported at the crate root.

| alias | constructors |
| --- | --- |
| a datatype that carries no parameters (`Int64Field`, `VariantField`, `VersionField`, `CountryField`, `CurrencyField`, `MicCodeField`, `CfiCodeField`, `IsinCodeField`, `CusipCodeField`, `SedolCodeField`, `BloombergCodeField`, `FIGICodeField`, `SideField`, `StateField`, `TimeInForceField`) | `unit(name, nullable)`: there is nothing to pass, so naming the datatype again would say it twice |
| a family with leaves or parameters (`StringField`, `BytesField`, `UuidField`, `DecimalField`, `UriField`, `DateField`, `TimeField`, `DateTimeField`, `DurationField`, `IntervalField`, `SequenceField`, `GeometryField`, `GeographyField`) | `new(name, dtype, nullable)`, taking that family's own payload |
| from a `Field` | `FieldValue::from_field` borrows the leaf, `None` for another variant; `into_field` widens back to the root |
| bindings | `yggdryl.int64` / `fields.int64` return the native `Field`, typed for a checker only; `yggdryl.string(name, layout=, charset=, fixed=, max=)` / `fields.string(name, { layout, charset, fixed, max })`, `yggdryl.bytes` / `fields.bytes`, `yggdryl.fixed_ascii(name, width)` / `fields.fixedAscii(name, width)`, `yggdryl.version` / `fields.version`, `yggdryl.figi` / `fields.figi` |

[Geospatial](geospatial/index.md), [Strings & bytes](text/index.md), [Codes](codes/index.md), [UUID](uuid.md), and [Version](version.md) aliases follow this pattern; a registered code builds its own datatype, not a fixed string. Rust keeps one cached parameter-free field for `DataType::FIGICode`; `DataType::FIGICode.shared_field()` answers that shared field.

## Converting to one native field

| | typed value | struct root |
| --- | --- | --- |
| Rust | `TypedField<K>::into_field(self)` | `StructField::into_struct_field(self)` |
| Python | `field(value, name=None)` | cached `Class.into_field() -> StructField`, installed by `@scalar` |
| JavaScript | `intoField(value, name = null)` | static getter `Class.intoStructField`, memoized by `intoField` |

No name, `None`/`null`, or the existing name returns the cached native value; another name returns a renamed clone. The root must be a non-null struct field.

Python spells the class accessor `into_field` because a `@scalar` class converts only as a struct root and has no leaf form to tell it apart from. On a *value* Python keeps the pair the other two rows have: [`Scalar.into_field`](scalar.md) for the leaf and `Scalar.into_struct_field` for the root.

## Applying a schema's declarations

A `Field` states more about a batch than its shape. A
[`PARTITION:`](../holder/iobase/partitions.md#derived-partition-columns) declaration says a
column is *derived* from another; a [`DIGEST:`](../hashing.md) role says a column *holds*
the row's hash. `apply_arrow_batch` is the one entry point that asks every declaring protocol,
in the order their answers depend on: `cast` reconciles the batch to this root, `partition`
computes the derived columns, and `digest` fills the holders last, over the rows as they
finally stand.

Each protocol walks the Structs it declares and leaves a column holding anything but its
canonical default alone, so applying twice writes nothing the first pass already did.

The declarations name every column they add, so the applied shape is a property of two schemas
and never of the data: `apply_arrow_schema` answers it without reading a row, and
`apply_arrow_reader` uses that to report the shape a stream will have before its first batch is
pulled - which is what lets a partitioned read be handed straight to a write. The whole applied
plan is compiled from those two schemas once, so a stream pays for it once.

`options` carries the [cast policy](cast.md#strict-nullability) the first step runs under, and
under `strict` nullability it also holds after the protocols have run. A field an enabled protocol
materializes may arrive absent or holding its canonical default - closing that hole is the
protocol's job, and it has not run yet - but the applied batch is checked again once every
protocol is done, so a required column its protocol did not write is still refused by path.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Date32Array, RecordBatch};
    use yggdryl::expression::Function;
    use yggdryl::{ArrowCastOptions, DataType, StructType};

    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut().set_sources(["event"])?;
    year.as_partition_mut().set_transform(Function::Year)?;
    let mut row_digest = DataType::UInt64.nullable_field("row_digest");
    row_digest.as_digest_mut().set_holder()?;
    let root = DataType::from(StructType::from_fields([
        DataType::date32().required_field("event"),
        year,
        row_digest,
    ])?)
    .required_field("row");

    let batch = RecordBatch::try_from_iter([(
        "event",
        Arc::new(Date32Array::from(vec![19_723])) as ArrayRef,
    )])?;

    let applied = root.apply_arrow_batch(&batch, true, true, true, ArrowCastOptions::new())?;

    assert_eq!(applied.num_columns(), 3);
    // The digest saw the derived column, because the partition step ran first.
    assert_eq!(applied.column(2).null_count(), 0);
    // Applying again writes nothing: every column now holds a written value.
    assert_eq!(
        root.apply_arrow_batch(&applied, true, true, true, ArrowCastOptions::new())?,
        applied,
    );

    // The same shape, with no rows read and no batch pulled.
    let shape =
        root.apply_arrow_schema(batch.schema(), true, true, true, ArrowCastOptions::new())?;
    assert_eq!(shape, applied.schema());

    let mut stream = root.apply_arrow_reader(
        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        true,
        true,
        true,
        ArrowCastOptions::new(),
    )?;
    assert_eq!(arrow_array::RecordBatchReader::schema(&stream), shape);
    assert_eq!(stream.next().expect("one batch")?.num_columns(), 3);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    year = Field("year", "int32", nullable=True)
    year.partition.sources = ["event"]
    year.partition.transform = "year"
    row_digest = Field("row_digest", "uint64", nullable=True)
    row_digest.digest["role"] = "holder"
    root = Field(
        "row",
        DataType.from_fields(
            [Field("event", "date32", nullable=False), year, row_digest]
        ),
        nullable=False,
    )
    batch = pa.record_batch({"event": pa.array([19_723], pa.date32())})

    applied = root.apply_arrow_batch(batch)

    assert applied.column_names == ["event", "year", "row_digest"]
    assert applied.column("year").to_pylist() == [2024]
    assert applied.column("row_digest").null_count == 0
    assert root.apply_arrow_batch(applied).equals(applied)

    # Each step is separately switchable; a cast alone materializes and writes nothing.
    cast_only = root.apply_arrow_batch(batch, digest=False, transform=False)
    assert cast_only.column("year").to_pylist() == [None]

    # A declared non-null column its protocol did not write is refused by path.
    required_year = Field("year", "int32", nullable=False)
    required_year.partition.sources = ["event"]
    required_year.partition.transform = "year"
    strict_root = Field(
        "row",
        DataType.from_fields([Field("event", "date32", nullable=False), required_year]),
        nullable=False,
    )
    # With the partition step on, the column it writes satisfies its own
    # declaration; with it off, nothing is going to write it.
    assert strict_root.apply_arrow_batch(batch, nullability="strict").num_columns == 2
    try:
        strict_root.apply_arrow_batch(batch, transform=False, nullability="strict")
    except ValueError as error:
        assert "$.year" in str(error), error
    else:
        raise AssertionError("a strict apply must refuse the unwritten column")

    # The same shape, with no rows read and no batch pulled.
    assert root.apply_arrow_schema(batch.schema) == applied.schema
    stream = root.apply_arrow_reader(
        pa.RecordBatchReader.from_batches(batch.schema, [batch])
    )
    assert stream.schema == applied.schema
    assert stream.read_all().num_rows == 1
    ```

    !!! note "Rust and Python only"
        JavaScript binds no `applyArrowBatch` on a `Field`; a batch crosses it as copied IPC.

## Serializing a schema

One `Field` ⇄ `Scalar` mapping (`into_value`/`from_value`, `into_dict`/`from_dict`) backs JSON, YAML, and TOML, so a schema embeds inline in any document. Each writer takes the shared [`Formatting`](../media/index.md#json) option, `indent` in Python.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};
    use yggdryl::Scalar;

    let field = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;

    // One structural model, three formats over it.
    assert_eq!(Field::from_value(field.clone().into_value())?, field);
    assert_eq!(Field::from_json(&field.clone().into_json()?)?, field);
    assert_eq!(Field::from_yaml(&field.clone().into_yaml()?)?, field);
    assert_eq!(Field::from_toml(&field.clone().into_toml()?)?, field);

    // The mapping is the shared `Scalar`, so it drops into any document.
    let shape = field.into_value();
    assert_eq!(shape.get_key_str("name").and_then(Scalar::as_str), Some("price"));
    // Unset optional attributes are absent rather than null.
    assert!(shape.get_key_str("dictionary_id").is_none());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    field = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})

    assert Field.from_dict(field.into_dict()) == field
    assert Field.from_json(field.into_json()) == field
    assert Field.from_yaml(field.into_yaml()) == field
    assert Field.from_toml(field.into_toml()) == field

    shape = field.into_dict()
    assert shape["name"] == "price"
    assert "dictionary_id" not in shape
    ```

=== "JavaScript"

    !!! note "Rust and Python only"
        JavaScript has no YAML or TOML writer; it reads and writes the same model as JSON
        through `toJSON`, `toJSONBytes`, `Field.fromJSON`, and `Field.fromJSONBytes`
        ([String](text/string.md#serialized-shape) round-trips one).

## A readable rendering

`Display`, and Python's `str`/`repr`, is the compact form that round-trips through `from_str`. The readable form is the alternate: `{:#}`, the `pretty()` adapter behind it, and Python's `pretty()`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let order = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from(StructType::from_fields([DataType::Float64.required_field("price")])?)
            .nullable_field("line"),
    ])?)
    .required_field("order");

    // Compact still round-trips.
    assert_eq!(Field::from_str(&order.to_string())?, order);

    // Readable is the alternate, or the named adapter - one implementation.
    assert_eq!(format!("{order:#}"), order.into_pretty_str().to_string());
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

    !!! note "Rust and Python only"
        JavaScript has no `pretty`; `toString` is the compact form that round-trips.

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

`equals` answers yes or no; `show_diffs` answers why as a lazy iterator (`Differences` borrows, `OwnedDifferences` owns). `show_diff` joins the lines; [`DataType`](datatype.md) has the same two calls.

## Edges

- nullable root -> `validate_struct_root` refuses.
- Python `field(x, idx=..., path=...)` naming more than one -> refused.
- an optional lookup -> `Option` in Rust, `None` in Python, `null` in JavaScript.
- `unnest_fields` -> a list or map stays one leaf column; a leaf under a nullable ancestor is nullable.
- `unnest_fields` names -> each one resolves through `field_by_path`.
- `explode_fields` -> a list gives its item, a map its entries, a dictionary or run-end its values.
- `explode_fields` -> one level per call; the column keeps its name and place; nullable when the collection or its element is.
- both projections -> a list of fields, not a node; `DataType::from(StructType::from_fields(..)?)` rebuilds one.
- `merge_with(other, upscale)` -> `upscale` widens by default and loses nothing; `false` meets at the tightest type naming both, keeping a code, a `uuid`, or a fixed string over the plainer shape storing it.
- widening a decimal -> the widest backing either side declared, never a re-encoding down to what the merged precision needs.
- `Field::merge_with` -> receiver's name; nullable when either side is; dictionary options only where both encode; metadata unioned, receiver winning.
- merged struct -> a one-sided child becomes nullable; receiver order, additions appended.
- boolean beside datetime, decimal beside float -> refused.
- `order["a.b"]` -> a child literally named `a.b` wins over `a` then `b`.
- list or run-end node -> exactly one or two children; grow and shrink refuse.
- Python `DataType[...] = ...` -> refused; it points at the owning `Field`.
- Python first `hash(field)` -> locks mutation on that wrapper; `copy.copy` unlocks; `stable_hash()` never locks.
- binding metadata and protocol views -> unhashable; Rust's borrowed protocol view is not `Borrow<Field>`.
- typed field -> no `DerefMut`; a failing `set_dtype` leaves the value untouched.
- `field(value, name)` with a non-string name or a non-field value -> `TypeError`.
- emitted shape -> `name`, `dtype`, `nullable`, then `dictionary_id` when non-zero and `dictionary_is_ordered` when set, then `metadata`.
- unset optional attribute -> omitted, never emitted as null.
- `pretty()` -> only set attributes; metadata as `@key = value` lines; stable across runs.
- `with_metadata=false` -> metadata dropped at every depth.
- `return_equal` -> false for `show_diffs`, true for `show_diff`; only `show_diff` prints `✓ equal`.
- diff paths -> `$`-rooted places such as `$.nullable` and `$.fields[2]`.
- `apply_arrow_batch` -> `cast`, then `partition`, then `digest`; a later step reads what an earlier one wrote.
- `apply_arrow_batch(digest=True, cast=False)` -> the digest step reconciles to the root for itself, because a holder is addressed by position.
- a column holding anything but its canonical default -> left alone by every step, so applying twice changes nothing.
- `apply_arrow_schema` -> the empty batch through the same steps; a declaration that cannot be satisfied fails here, not on the first batch.
- `nullability="strict"` -> a field an enabled protocol materializes may arrive absent; every other declared non-null field is refused where it stands, and the applied batch is checked again once the protocols are done.
- `apply_arrow_reader` with all three off -> the reader itself, unwrapped; otherwise the applied schema is derived once and reported before the first pull.
- a batch that fails inside `apply_arrow_reader` -> that batch's `Err`; the reader is not fused after it.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test expression -- path::nested
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test metadata -- validation::generic
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- boolean bytes::fields decimal::fields diff::comparison field::arrow field::generic field::nested floating integer mapping::nested merge::nested metadata::generic parser::generic protocol::generic protocol::nested serde::generic serde::schemas temporal::fields typed
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --doc -- Field::apply_arrow
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- diff::internal merge::internal
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^parse/field_'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(nested_field_clone|field_stable_hash|metadata_)'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^typed/'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^comparison/'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^arrow/(field_|struct_field_)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_field.py python/tests/test_datatype.py python/tests/test__classes.py python/tests/test_protocol.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/field.test.js node/tests/fields.test.js
    npm run --prefix node bench:types
    ```

## Performance

Rust times both consuming typed accessors, construction outside the timer; the bindings hold the cached value and price a renamed clone separately. One local Windows x86_64 release run: Criterion point estimates, Python median per call, JavaScript whole-loop rate.

| runtime operation | estimate |
| --- | ---: |
| Rust `TypedField::into_field` | 41.5 ns |
| Rust `StructField::into_struct_field` | 34.7 ns |
| Python cached `Class.into_field()` | 677 ns |
| Python global `field(Class)` | 1.27 us |
| Python renamed `field(Class, name=...)` | 9.26 us |
| JavaScript `intoField(nativeField)` | 40.0 ns (25.0M calls/s) |
| JavaScript cached `intoField(Class)` | 72.0 ns (13.9M calls/s) |
| JavaScript renamed `intoField(Class, name)` | 33.6 us (29.7k calls/s) |

```bash
cargo bench --manifest-path rust/Cargo.toml --bench types -- '^typed/'
python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
npm run --prefix node bench:types
```
