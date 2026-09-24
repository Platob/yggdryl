# Struct

Named children in declaration order: the one collection of fields the crate has, and - under a non-null field - the row schema every reader and writer takes.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Struct(StructType)`, the `StructField` marker, and the `Struct` value: a record's children by name, sorted |
| Validates | At construction: two children of one name are refused, and every child validates |
| Lazy | Nothing - the children are collected and checked once, and an empty struct holds no allocation at all |
| Cached | The children in one shared `Arc<[Field]>`, so a clone shares them; the Arrow projection on the [`Field`](../field.md) |
| Refuses | A duplicate child name, a `with_fields` whose count is not the declared arity, and a row value that is neither a record nor a sequence of the declared length |
| Kinds | `DataTypeKind::Nested`, id `struct` (`0x96`); `is_nested()` is `true` |
| Bindings | `StructType` and `Struct` are Rust only: Python builds the datatype with `DataType.from_fields`, JavaScript with `DataType.fromFields`, and both read a row back as a [`Scalar`](../scalar.md) |

## DataType

`StructType` is the collection; `DataType::from` is the datatype over it. The
collection dereferences to `&[Field]`, so everything a caller does with a slice
reads the same through it. `struct` and `row` are the two keywords, and the
SQL, Hive and Spark spellings parse into the same canonical `struct(...)`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind, Field, StructType};

    let children = StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Int64.required_field("quantity"),
    ])?;
    assert_eq!(children.len(), 2);
    assert_eq!(children[1].name(), "quantity");
    assert_eq!(children.get_by_name("symbol").map(Field::name), Some("symbol"));

    let row = DataType::from(children.clone());
    assert_eq!(row.id(), DataTypeId::Struct);
    assert_eq!(row.kind(), DataTypeKind::Nested);
    assert_eq!(row.field_len(), 2);
    assert_eq!(row.as_fields().map(<[Field]>::len), Some(2));

    // `as_fields` is the struct's alone; no other layout has a schema.
    assert!(DataType::serie(DataType::Int64.nullable_field("item")).as_fields().is_none());

    // Two children of one name are refused where the collection is built.
    assert!(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("id"),
    ]).is_err());

    // One canonical spelling, whichever dialect wrote it.
    let parsed = DataType::from_str("struct<id:bigint,tags:array<string>>")?;
    assert_eq!(DataType::from_str("row(id bigint, tags array<string>)")?, parsed);
    assert_eq!(DataType::from_str(&parsed.to_string())?, parsed);
    assert!(parsed.to_string().starts_with("struct(field(\"id\",int64"));
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    from yggdryl import DataType, Field

    row = DataType.from_fields([
        yggdryl.utf8("symbol", nullable=False),
        yggdryl.int64("quantity", nullable=False),
    ])

    assert row.id == "struct"
    assert row.kind == "nested"
    assert row.is_nested
    assert len(row) == 2
    assert [field.name for field in row] == ["symbol", "quantity"]
    assert row["symbol"].nullable is False

    # Two children of one name are refused where the collection is built.
    with pytest.raises(ValueError, match="duplicate field name"):
        DataType.from_fields([Field("id", "int64"), Field("id", "utf8")])

    # One canonical spelling, whichever dialect wrote it.
    parsed = DataType("struct<id:bigint,tags:array<string>>")
    assert DataType("row(id bigint, tags array<string>)") == parsed
    assert DataType(str(parsed)) == parsed
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const row = DataType.fromFields([
      fields.utf8('symbol'),
      fields.int64('quantity'),
    ])

    assert.equal(row.id, 'struct')
    assert.equal(row.kind, 'nested')
    assert.equal(row.nested, true)
    assert.equal(row.length, 2)
    assert.deepEqual(row.keys(), ['symbol', 'quantity'])

    // Two children of one name are refused where the collection is built.
    assert.throws(
      () => DataType.fromFields([fields.int64('id'), fields.utf8('id')]),
      /duplicate field name/,
    )

    // One canonical spelling, whichever dialect wrote it.
    const parsed = DataType.from('struct<id:bigint,tags:array<string>>')
    assert.ok(DataType.from('row(id bigint, tags array<string>)').equals(parsed))
    assert.ok(DataType.from(parsed.toString()).equals(parsed))
    ```

## Field

`StructField` is the typed marker: one field carrying a `StructType`, so the
children are read off the payload rather than matched out of a root datatype.
A **non-null** struct field is the row schema - the root every
[media](../../media/index.md) reader and writer takes - and
[Field](../field.md#a-non-null-struct-field-is-the-schema) owns that rule,
its `validate_struct_root` and `require_struct` doors, and the whole table of
positional and path accessors.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, DataTypeId, Field, StructField, StructType};

    let children = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?;
    let schema = StructField::new("trade", children, false);

    assert_eq!(schema.name(), "trade");
    assert_eq!(schema.id(), DataTypeId::Struct);
    assert_eq!(schema.typed_dtype_ref().len(), 2);
    assert!(!schema.is_nullable());

    // Widened, it is the same field the root constructor builds.
    let root: Field = schema.clone().into_field();
    assert_eq!(root, Field::new("trade", schema.dtype().clone(), false));
    root.validate_struct_root()?;
    assert!(StructField::from_field(&root).is_some());

    // A nullable root is not a schema: a whole row cannot be logically absent.
    assert!(root.with_nullable(true).validate_struct_root().is_err());
    // A datatype from another family is refused by name.
    let refused = StructField::try_new("trade", DataType::utf8(), false).unwrap_err().to_string();
    assert!(refused.contains("struct"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    from yggdryl import Field

    schema = yggdryl.struct(
        "trade",
        [yggdryl.int64("id", nullable=False), yggdryl.utf8("symbol")],
        nullable=False,
    )

    assert isinstance(schema, Field)
    assert schema.dtype.id == "struct"
    assert schema.nullable is False
    assert schema.index_of("symbol") == 1
    schema.validate_struct_root()

    # A nullable root is still a struct column, but it is not a schema.
    nullable = Field("trade", schema.dtype)
    nullable.require_struct()
    with pytest.raises(ValueError):
        nullable.validate_struct_root()

    # Metadata rides beside the datatype, never inside it.
    annotated = yggdryl.struct("trade", [yggdryl.int64("id")], metadata={"owner": "events"})
    assert annotated.metadata["owner"] == "events"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const schema = fields.struct(
      'trade',
      [fields.int64('id', { nullable: false }), fields.utf8('symbol')],
      { nullable: false },
    )

    assert.ok(schema instanceof Field)
    assert.equal(schema.dtype.id, 'struct')
    assert.equal(schema.nullable, false)
    assert.equal(schema.indexOf('symbol'), 1)
    assert.equal(schema.fieldLen, 2)

    // Metadata rides beside the datatype, never inside it.
    const annotated = fields.struct('trade', [fields.int64('id')], {
      metadata: { owner: 'events' },
    })
    assert.equal(annotated.get('owner'), 'events')
    ```

## Scalar

Two spellings of one row. `Scalar::Struct` is named input - the children by
name, sorted, so two statements of one row in two orders are one value - and
the ordered `Scalar::Serie` is what a row **is** once a struct field has
canonicalized it. The field decides: `Field::scalar` takes either spelling and
answers the sequence in the schema's declared order, filling a child the input
did not name with that child's default.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, StructType};

    let schema = Field::new(
        "trade",
        DataType::from(StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])?),
        false,
    );

    // Named input canonicalizes to the ordered row the schema declares.
    let named = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("id", Scalar::from(7_i64)),
    ])?;
    let row = schema.scalar(named)?;
    assert_eq!(row, Scalar::from_sequence([Scalar::from(7_i64), Scalar::from("AAPL")]));

    // An ordered sequence is already the row, so it passes through as itself.
    assert_eq!(schema.scalar(vec![Scalar::from(7_i64), Scalar::from("AAPL")])?, row);
    assert_eq!(row.len(), 2);

    // A duplicate name never becomes a record.
    assert!(Scalar::from_struct([("id", Scalar::from(1_i64)), ("id", Scalar::from(2_i64))]).is_err());
    ```

=== "Python"

    ```python
    import yggdryl

    schema = yggdryl.struct(
        "trade",
        [yggdryl.int64("id", nullable=False), yggdryl.utf8("symbol")],
        nullable=False,
    )

    # A row is the ordered sequence the schema declares.
    row = schema.scalar([7, "AAPL"])
    assert row.as_py() == [7, "AAPL"]
    assert row.family == "nested"
    assert len(row) == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const schema = fields.struct(
      'trade',
      [fields.int64('id', { nullable: false }), fields.utf8('symbol')],
      { nullable: false },
    )

    // A row is the ordered sequence the schema declares.
    const row = schema.scalar([7n, 'AAPL'])
    assert.deepEqual(row.asJs(), [7, 'AAPL'])
    assert.equal(row.family, 'nested')
    assert.equal(row.length, 2)
    ```

## Arrow storage

`ArrowDataType::Struct(fields)`, child for child, with no extension document:
Arrow's struct says everything this one does. A non-null struct root is the
Arrow *schema* rather than a column, which is the one asymmetry -
[Arrow](../../arrow/schema.md) owns that boundary.

| datatype | Arrow storage | extension name |
| --- | --- | --- |
| `struct(...)` | `Struct(fields)` | none |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
    use yggdryl::{DataType, Field, StructType};

    let row = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?);
    let arrow = ArrowDataType::Struct(
        vec![
            Arc::new(ArrowField::new("id", ArrowDataType::Int64, false)),
            Arc::new(ArrowField::new("symbol", ArrowDataType::Utf8, true)),
        ]
        .into(),
    );

    assert_eq!(row.clone().into_arrow_datatype()?, arrow);
    assert_eq!(DataType::from_arrow_datatype(&arrow)?, row);

    // Arrow's own duplicate names have no struct to import into.
    let duplicate = ArrowDataType::Struct(
        vec![
            Arc::new(ArrowField::new("same", ArrowDataType::Int32, false)),
            Arc::new(ArrowField::new("same", ArrowDataType::Utf8, true)),
        ]
        .into(),
    );
    assert!(DataType::from_arrow_datatype(&duplicate).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import Field

    schema = yggdryl.struct(
        "trade",
        [yggdryl.int64("id", nullable=False), yggdryl.utf8("symbol")],
        nullable=False,
    )
    arrow = schema.into_arrow()

    assert arrow.type == pa.struct([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string(), nullable=True),
    ])
    assert arrow.metadata is None
    assert Field.from_arrow(arrow) == schema
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const schema = fields.struct(
      'trade',
      [fields.int64('id', { nullable: false }), fields.utf8('symbol')],
      { nullable: false },
    )

    // A table read under the struct root answers the Arrow schema a row is written as.
    const batch = Serie.fromArrowBatch(
      new arrow.Table({
        id: arrow.vectorFromArray([7n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL'], new arrow.Utf8()),
      }),
      schema,
    ).intoArrowBatch()
    assert.deepEqual(batch.schema.fields.map((field) => field.name), ['id', 'symbol'])
    assert.equal(batch.schema.fields[0].nullable, false)
    ```

## Replacing and removing children

A child is borrowed, never handed out mutably: a caller replaces one by
position or by path, and a name no child carries appends. This is the one
layout that grows and shrinks - every other nested layout has a fixed arity -
and `with_fields` is the whole-collection form, which keeps the count.

=== "Rust"

    ```rust
    use yggdryl::{DataType, StructType};

    let mut row = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?);

    // An unknown name appends, which is how a schema is built up.
    row.set_field("venue", DataType::utf8().nullable_field("venue"))?;
    assert_eq!(row.field_len(), 2);
    assert_eq!(row[1].name(), "venue");

    // A known name replaces in place, keeping its position.
    row.set_field_by_path("id", DataType::utf8().required_field("id"))?;
    assert_eq!(row["id"].dtype(), &DataType::utf8());
    assert_eq!(row[0].name(), "id");

    // Removal returns the prior child and closes the gap.
    assert_eq!(row.remove_field("id")?.name(), "id");
    assert_eq!(row.field_len(), 1);
    assert_eq!(row[0].name(), "venue");

    // The whole-collection form keeps the arity it replaces.
    let widened = DataType::from(StructType::from_fields([DataType::Int32.required_field("id")])?)
        .with_fields([DataType::Int64.required_field("id")])?;
    assert_eq!(widened["id"].dtype(), &DataType::Int64);
    assert!(widened.with_fields([]).is_err());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    row = DataType.from_fields([yggdryl.int64("id", nullable=False)])

    # An unknown name appends; a known one replaces in place.
    row["venue"] = yggdryl.utf8("venue")
    assert len(row) == 2 and row[1].name == "venue"
    row["id"] = yggdryl.utf8("id", nullable=False)
    assert str(row["id"].dtype) == "utf8"

    # Removal closes the gap.
    del row["id"]
    assert len(row) == 1 and row[0].name == "venue"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const row = DataType.fromFields([fields.int64('id', { nullable: false })])

    // An unknown name appends; a known one replaces in place.
    row.setFieldByPath('venue', fields.utf8('venue'))
    assert.equal(row.length, 2)
    assert.equal(row.getFieldAt(1).name, 'venue')
    row.setField('id', fields.utf8('id', { nullable: false }))
    assert.equal(row.getField('id').dtype.toString(), 'utf8')

    // Removal returns the prior child and closes the gap.
    assert.equal(row.removeField('id').name, 'id')
    assert.deepEqual(row.keys(), ['venue'])
    ```

## What a struct is read through

A struct is the node every recursive walk crosses, and each walk is documented
where it is owned rather than restated here:

| walk | owned by |
| --- | --- |
| positional, path and subscript access, in all four raising/optional/replacing/removing forms | [Field](../field.md#item-access-reaches-a-child-never-metadata) |
| `unnest_fields` - every leaf named by its dotted path - and `explode_fields` | [Field](../field.md#flattening-and-expanding) |
| `merge_with` - the union of two schemas, child by child | [Field](../field.md#merging-two-schemas) |
| a Struct inferred from a regex's named captures | [String](../text/string.md#regex-captures) |
| the row a reader yields and a writer takes | [Arrow](../../arrow/schema.md) |

## Edges

- Two children of one name -> `duplicate field name` error, at `StructType::from_fields` and at every import that would build one, Arrow's own schema included.
- An empty struct is legal and holds no allocation: `StructType::new()` is `struct()`, and it is the default a builder starts from.
- `as_fields` answers a struct alone; a serie, a map, a union and the two wrappers answer `None`.
- A child is borrowed, never `&mut`: `set_field_*` replaces and `remove_field_*` removes, and a refusal leaves the datatype exactly as it was.
- `with_fields` keeps the arity: a count that is not the declared one is refused rather than resized.
- A row value is a record or a sequence of the declared length; a record's extra name, or a sequence of the wrong length, is refused with the path that failed.
- A `Scalar::Struct` sorts by name and a row does not: the sequence a field canonicalizes to is in the schema's declared order, and that is the one a reader and a writer speak.
- Equality, order and the stable hash read the children, so a struct datatype is a map key and a cache key as it stands.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test expression -- path::nested
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test metadata -- validation::generic
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- datatype_kind::nested field::generic field::nested mapping::nested merge::nested metadata::generic parser::generic parser::nested protocol::generic protocol::nested serde::generic structure::nested union::variants
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(struct_from_fields_1024|nested_datatype_clone|nested_validate)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py python/tests/test_field.py -k "struct or from_fields or nested"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="struct|fromFields|nested" node/tests/datatype.test.js node/tests/fields.test.js
    npm run --prefix node bench:types
    ```
