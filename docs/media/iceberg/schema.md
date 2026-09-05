# Iceberg schema

Field ids, the `SchemaUpdate` builder, schema documents, the type mappings, and how a [table](index.md) schema reaches a data file.

## Contract

| | |
| --- | --- |
| Owns | `assign_field_ids`, `last_column_id`, `SchemaUpdate`, `can_promote`, `schema_from_json`, `schema_into_json`, `PrimitiveType` |
| Schema | a non-null struct [`Field`](../../types/field.md) whose children carry `PARQUET:field_id`; there is no schema type |
| Identity | the field id: a rename keeps it, a drop never frees it, new ids continue above `last-column-id` |
| Numbering | `Table::create` and `evolve_schema` number unnumbered fields; `assign_field_ids` serves callers who need ids first |
| Promotions | only those Iceberg allows; `can_promote` refuses the rest naming both sides |
| Commit | Rust `evolve_schema` or `commit_metadata_changes`; Python `with table.update_schema()`; JavaScript `updateSchema()...commit()` |
| Old schemas | kept in `metadata().schemas()`; a file written before a column reads it as null |
| Documents | `required` inverts into nullability; `schema-id`, `doc`, and v3 defaults survive as `iceberg:*` metadata |
| Type mapping | `into_dtype` total; `from_dtype` refuses what Iceberg cannot spell; `Scheme::ICEBERG` widens losslessly |
| Rust only | `PrimitiveType`, the nested mapping, the Parquet writer's settings |
| Benchmarks | none; `rust/benchmarks/media/iceberg.rs` evolves a schema only as fixture setup |

## Use

Add a column, then read the earlier file back with the new column null.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, SchemaUpdate, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::{arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch};
    use std::sync::Arc;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");

    let path = Folder::temporary()?.path()?.join("yggdryl-docs-iceberg-evolution");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(
        Folder::new(&path)?,
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )?;

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    // Add a column. Numbering continues above `last-column-id`, so the new column
    // can never be confused with a dropped one.
    let mut update = SchemaUpdate::from_metadata(table.metadata())?;
    update.add_column("", DataType::Int64.nullable_field("quantity"));
    let evolved = update.into_field()?;
    assert_eq!(table.evolve_schema(evolved)?, 1, "the new schema's id");

    // The old schema is retained, so the snapshot written under it still reads.
    assert_eq!(table.metadata().schemas().len(), 2);
    assert_eq!(table.metadata().schema_by_id(0).unwrap().field_len(), 1);

    // And the file written before the column existed reads it as null.
    for batch in table.scan(None)? {
        let batch = batch?;
        assert_eq!(batch.num_columns(), 2);
        assert_eq!(batch.column_by_name("quantity").unwrap().null_count(), batch.num_rows());
    }
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    schema = columns

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = Table.create(root, schema)
    table.append(pa.record_batch({"id": [1]}, schema=columns))

    # Add a column. Numbering continues above `last-column-id`, so the new column
    # can never be confused with a dropped one.
    with table.update_schema() as update:
        update.add_column("", "quantity: int64")

    # The old schema is retained, so the snapshot written under it still reads.
    assert len(table.schemas) == 2
    assert len(table.schemas[0].dtype) == 1

    # And the file written before the column existed reads it as null.
    rows = table.scan().read_all()
    assert rows.column_names == ["id", "quantity"]
    assert rows.column("quantity").null_count == rows.num_rows
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    const table = iceberg.Table.create(root, schema)
    table.append(new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) }))

    // Add a column. Numbering continues above `last-column-id`, so the new column
    // can never be confused with a dropped one.
    const schemaId = table.updateSchema().addColumn('', 'quantity: int64').commit()
    assert.equal(schemaId, 1, "the new schema's id")

    // The old schema is retained, so the snapshot written under it still reads.
    assert.equal(table.schemas.length, 2)
    assert.equal(table.schemas[0].dtype.length, 1)

    // And the file written before the column existed reads it as null.
    const rows = table.scan().intoTable()
    assert.deepEqual(rows.schema.fields.map((child) => child.name), ['id', 'quantity'])
    assert.equal(rows.getChild('quantity').nullCount, rows.numRows)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Field ids

`assign_field_ids` numbers depth first from `start`, keeps any id already present, and returns the first id it did not use.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{assign_field_ids, last_column_id};
    use yggdryl::DataType;

    let leg = DataType::from_fields([DataType::decimal(18, 4)?.required_field("price")])?;
    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        leg.nullable_field("leg"),
    ])?
    .required_field("row");

    // Depth first from `start`; the return value is the first id it did not use.
    assert_eq!(assign_field_ids(&mut schema, 1)?, 4);
    assert_eq!(schema.fields()[0].parquet_field_id()?, Some(1));
    assert_eq!(schema.fields()[1].parquet_field_id()?, Some(2));
    assert_eq!(schema.fields()[1].fields()[0].parquet_field_id()?, Some(3));
    assert_eq!(last_column_id(&schema)?, 3, "what a table records as last-column-id");

    // The root is not a column, so it is not numbered.
    assert_eq!(schema.parquet_field_id()?, None);

    // A field that already carries an id keeps it, so a second pass changes nothing.
    assert_eq!(assign_field_ids(&mut schema, 100)?, 100);
    assert_eq!(schema.fields()[0].parquet_field_id()?, Some(1));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl.media.iceberg import assign_field_ids

    columns = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field(
            "leg",
            pa.struct([pa.field("price", pa.decimal128(18, 4), nullable=False)]),
        ),
    ])

    # Depth first from `start`; the numbered schema is what comes back, so the
    # schema handed in is left as it was.
    schema = assign_field_ids(columns, 1)
    assert [child.parquet_field_id for child in schema.dtype] == [1, 2]
    assert schema.dtype[1].dtype[0].parquet_field_id == 3

    # The root is not a column, so it is not numbered.
    assert schema.parquet_field_id is None

    # A field that already carries an id keeps it, so a second pass changes nothing.
    assert [child.parquet_field_id for child in assign_field_ids(schema, 100).dtype] == [1, 2]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields, iceberg } = require('yggdryl')

    const leg = fields.struct('leg', [Field.from('price: decimal(18, 4)')])
    const plain = fields.struct('row', [Field.from('id: int64'), leg], { nullable: false })

    // Depth first from `start`; the numbered schema is what comes back, so the
    // schema handed in is left as it was.
    const schema = iceberg.assignFieldIds(plain)
    assert.equal(plain.dtype.getFieldAt(0).parquetFieldId, null)
    assert.equal(schema.dtype.getFieldAt(0).parquetFieldId, 1)
    assert.equal(schema.dtype.getFieldAt(1).parquetFieldId, 2)
    assert.equal(schema.dtype.getFieldAt(1).dtype.getFieldAt(0).parquetFieldId, 3)

    // The root is not a column, so it is not numbered.
    assert.equal(schema.parquetFieldId, null)

    // A field that already carries an id keeps it, so a second pass changes nothing.
    assert.equal(iceberg.assignFieldIds(schema, 100).dtype.getFieldAt(0).parquetFieldId, 1)
    ```

A schema document from an unnumbered tree is refused, while creating a table numbers first.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::schema_into_json;
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");

    let message = schema_into_json(&schema).unwrap_err().to_string();
    assert!(message.contains("assign_field_ids"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table

    # A plain PyArrow schema carries no ids; creating the table numbers it.
    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    table = Table.create(IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades"), columns)

    assert [child.parquet_field_id for child in table.schema.dtype] == [1]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, fields, iceberg } = require('yggdryl')

    // A plain schema carries no ids; creating the table numbers it.
    const unnumbered = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    const table = iceberg.Table.create(root, unnumbered)
    assert.equal(table.schema.dtype.getFieldAt(0).parquetFieldId, 1)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

## Evolving a schema

`SchemaUpdate` records operations against the current schema; `into_field` yields the next one, and one commit publishes it.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{can_promote, FormatVersion, PartitionSpec, SchemaUpdate, Table};
    use yggdryl::holder::local::Folder;
    use yggdryl::DataType;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-evolution");
    let _ = std::fs::remove_dir_all(&root);
    let schema = DataType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");
    let mut table = Table::create(
        Folder::new(&root)?,
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )?;

    // Legal promotions pass; anything else is refused naming both sides.
    assert!(can_promote(&DataType::Int32, &DataType::Int64).is_ok());
    assert!(can_promote(&DataType::decimal(10, 2)?, &DataType::decimal(18, 2)?).is_ok());
    let message = can_promote(&DataType::Int64, &DataType::Int32).unwrap_err().to_string();
    assert!(message.contains("int64") && message.contains("int32"));

    // Widen id, rename symbol, add venue - one evolved schema, one commit.
    let mut update = SchemaUpdate::from_metadata(table.metadata())?;
    update.update_type("id", DataType::Int64);
    update.rename_column("symbol", "ticker");
    update.add_column("", DataType::Utf8.nullable_field("venue"));
    let evolved = update.into_field()?;

    table.commit_metadata_changes(|metadata| {
        let schema_id = metadata.add_schema(evolved.clone())?;
        metadata.set_current_schema(schema_id)
    })?;

    let current = table.schema()?;
    assert_eq!(current.get_field_by_path("id").expect("the column").dtype(), &DataType::Int64);
    // A renamed column keeps its identifier: the name is a label, the id is the column.
    assert_eq!(current.get_field_by_path("ticker").expect("the column").parquet_field_id()?, Some(2));
    assert_eq!(current.get_field_by_path("venue").expect("the column").parquet_field_id()?, Some(3));

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase
    from yggdryl.media.iceberg import Table, can_promote

    # Legal promotions pass; anything else is refused naming both sides.
    assert can_promote("int32", "int64") is None
    assert can_promote("decimal128(10, 2)", "decimal128(18, 2)") is None
    with pytest.raises(ValueError, match="int64 to int32"):
        can_promote("int64", "int32")

    columns = pa.schema([
        pa.field("id", pa.int32(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "trades"
    table = Table.create(IOBase(root), columns)

    # Widen id, rename symbol, add venue - one evolved schema, one commit.
    with table.update_schema() as update:
        update.update_type("id", "int64").rename_column("symbol", "ticker")
        update.add_column("", "venue: string")

    children = list(table.schema.dtype)
    assert [child.name for child in children] == ["id", "ticker", "venue"]
    assert str(children[0].dtype) == "int64"
    # A renamed column keeps its identifier: the name is a label, the id is the column.
    assert [child.parquet_field_id for child in children] == [1, 2, 3]

    shutil.rmtree(root.parent)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { Field, fields, iceberg } = require('yggdryl')

    // Legal promotions pass; anything else is refused naming both sides.
    iceberg.canPromote('int32', 'int64')
    iceberg.canPromote('decimal128(10, 2)', 'decimal128(18, 2)')
    assert.throws(() => iceberg.canPromote('int64', 'int32'), /int64 to int32/)

    const declared = fields.struct('row', [Field.from('id: int32'), Field.from('symbol: utf8')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-')), 'trades')
    const table = iceberg.Table.create(root, declared)

    // Widen id, rename symbol, add venue - one evolved schema, one commit.
    const schemaId = table
      .updateSchema()
      .updateType('id', 'int64')
      .renameColumn('symbol', 'ticker')
      .addColumn('', 'venue: utf8')
      .commit()
    assert.equal(schemaId, 1)

    const evolved = table.schema
    assert.deepEqual(Array.from(evolved.dtype, (child) => child.name), ['id', 'ticker', 'venue'])
    assert.equal(String(evolved.dtype.getFieldAt(0).dtype), 'int64')
    // A renamed column keeps its identifier: the name is a label, the id is the column.
    assert.deepEqual(Array.from(evolved.dtype, (child) => child.parquetFieldId), [1, 2, 3])

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

`TableMetadata` carries the rest of the update vocabulary, each through the official builder before `commit_metadata_changes`.

| change | `TableMetadata` methods |
| --- | --- |
| properties | `set_property`, `remove_property` |
| location, uuid, format | `set_location`, `assign_uuid`, `upgrade_format_version` |
| refs and snapshots | `set_snapshot_ref`, `remove_snapshot_ref`, `remove_snapshots` |
| specs | `add_spec`, `set_default_spec` |
| sort orders | `add_sort_order`, `set_default_sort_order` |

## Schemas as documents

`schema_from_json` and `schema_into_json` convert between the document and a `Field`; both bindings take the mapping their own JSON decoder produces.

=== "Rust"

    ```rust
    use yggdryl::media::iceberg::{schema_from_json, schema_into_json};
    use yggdryl::{DataType};
    use yggdryl::text::json;

    let document = json::from_utf8(
        r#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"},
            {"id":2,"name":"symbol","required":false,"type":"string"}
        ]}"#,
    )?;

    // An Iceberg schema is a non-null struct field; its columns are the children.
    let schema = schema_from_json("row", &document)?;
    assert!(schema.is_struct());
    assert!(!schema.is_nullable());
    assert_eq!(schema.field_len(), 2);
    assert_eq!(schema.fields()[0].dtype(), &DataType::Int64);

    // `required` inverts into nullability, and `id` becomes PARQUET:field_id.
    assert!(!schema.fields()[0].is_nullable());
    assert!(schema.fields()[1].is_nullable());
    assert_eq!(schema.fields()[0].parquet_field_id()?, Some(1));
    assert_eq!(schema.fields()[0].get_metadata("PARQUET:field_id"), Some("1"));

    // The same document comes back out.
    assert_eq!(schema_into_json(&schema)?, document);
    ```

=== "Python"

    ```python
    import json

    from yggdryl.media.iceberg import schema_from_json, schema_into_json

    document = json.loads("""{"type":"struct","schema-id":0,"fields":[
        {"id":1,"name":"id","required":true,"type":"long"},
        {"id":2,"name":"symbol","required":false,"type":"string"}
    ]}""")

    # An Iceberg schema is a non-null struct field; its columns are the children.
    schema = schema_from_json("row", document)
    assert schema.dtype.kind == "nested"
    assert not schema.nullable
    assert len(schema.dtype) == 2
    assert str(schema.dtype[0].dtype) == "int64"

    # `required` inverts into nullability, and `id` becomes PARQUET:field_id.
    assert not schema.dtype[0].nullable
    assert schema.dtype[1].nullable
    assert schema.dtype[0].parquet_field_id == 1
    assert schema.dtype[0].metadata["PARQUET:field_id"] == "1"

    # The same document comes back out.
    assert schema_into_json(schema) == document
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { iceberg, json } = require('yggdryl')

    const document = json.loads(
      Buffer.from(`{"type":"struct","schema-id":0,"fields":[
        {"id":1,"name":"id","required":true,"type":"long"},
        {"id":2,"name":"symbol","required":false,"type":"string"}
      ]}`),
    )

    // An Iceberg schema is a non-null struct field; its columns are the children.
    const schema = iceberg.schemaFromJson('row', document)
    assert.equal(schema.dtype.kind, 'nested')
    assert.equal(schema.nullable, false)
    assert.equal(schema.dtype.length, 2)
    assert.equal(String(schema.dtype.getFieldAt(0).dtype), 'int64')

    // `required` inverts into nullability, and `id` becomes PARQUET:field_id.
    assert.equal(schema.dtype.getFieldAt(0).nullable, false)
    assert.equal(schema.dtype.getFieldAt(1).nullable, true)
    assert.equal(schema.dtype.getFieldAt(0).parquetFieldId, 1)
    assert.equal(schema.dtype.getFieldAt(0).get('PARQUET:field_id'), '1')

    // The same document comes back out.
    assert.deepEqual(iceberg.schemaIntoJson(schema).asJs(), document)
    ```

Documents pass through the core [JSON](../../text/json.md) codec as [`Scalar`](../../types/scalar.md) values, validated by the official Iceberg model before projection.

| document | `Field` |
| --- | --- |
| root `name` | the name you pass; Iceberg names columns, not the schema |
| `"required": true` | `is_nullable() == false` |
| `id` | `PARQUET:field_id` metadata |
| `schema-id` | `iceberg:schema-id` on the root |
| `doc` | `iceberg:doc` |
| v3 `initial-default`, `write-default` | `iceberg:initial-default`, `iceberg:write-default` |

## Primitive types

Rust only.

```rust
use yggdryl::media::iceberg::PrimitiveType;
use yggdryl::{DataType, TimeUnit, Timezone};

// Every Iceberg primitive name has exactly one physical datatype.
assert_eq!(PrimitiveType::from_str("long")?.into_dtype()?, DataType::Int64);
assert_eq!(PrimitiveType::from_str("string")?.into_dtype()?, DataType::Utf8);
assert_eq!(
    PrimitiveType::from_str("decimal(18, 4)")?.into_dtype()?,
    DataType::decimal(18, 4)?
);

// Iceberg fixed every temporal resolution at microseconds until v3 added the
// nanosecond pair.
assert_eq!(
    PrimitiveType::from_str("timestamp")?.into_dtype()?,
    DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE }
);
assert_eq!(
    PrimitiveType::from_str("timestamp_ns")?.into_dtype()?,
    DataType::DateTime64 { unit: TimeUnit::Nanosecond, timezone: Timezone::NAIVE }
);
assert_eq!(
    PrimitiveType::from_str("time")?.into_dtype()?,
    DataType::time(TimeUnit::Microsecond)?
);

// A v3 `unknown` column always reads as null, which Arrow spells exactly.
assert_eq!(PrimitiveType::from_str("unknown")?.into_dtype()?, DataType::Null);

// A name round trips through `Display`.
assert_eq!(PrimitiveType::from_str("fixed[16]")?.to_string(), "fixed[16]");
```

`PrimitiveType` is the whole Iceberg type vocabulary, parsed from the spelling in table metadata JSON.

| Iceberg | `DataType` | Version |
| --- | --- | --- |
| `boolean` | `Boolean` | v1 |
| `int` | `Int32` | v1 |
| `long` | `Int64` | v1 |
| `float` | `Float32` | v1 |
| `double` | `Float64` | v1 |
| `decimal(p, s)` | `Decimal128 { precision: p, scale: s }` | v1 |
| `date` | `Date32` | v1 |
| `time` | `Time64(Microsecond)` | v1 |
| `timestamp` | `DateTime64 { unit: Microsecond, timezone: NAIVE }` | v1 |
| `timestamptz` | `DateTime64 { unit: Microsecond, timezone: UTC }` | v1 |
| `timestamp_ns` | `DateTime64 { unit: Nanosecond, timezone: NAIVE }` | v3 |
| `timestamptz_ns` | `DateTime64 { unit: Nanosecond, timezone: UTC }` | v3 |
| `string` | `Utf8` | v1 |
| `uuid` | `FixedSizeBinary(16)` | v1 |
| `fixed[n]` | `FixedSizeBinary(n)` | v1 |
| `binary` | `Binary` | v1 |
| `unknown` | `Null` | v3 |

`into_dtype` is total; `from_dtype` names the datatype it refuses instead of widening it.

```rust
use yggdryl::media::iceberg::PrimitiveType;
use yggdryl::DataType;

// The variants that differ only in physical layout collapse onto one name.
assert_eq!(PrimitiveType::from_dtype(&DataType::Utf8)?, PrimitiveType::String);
assert_eq!(PrimitiveType::from_dtype(&DataType::LargeUtf8)?, PrimitiveType::String);
assert_eq!(PrimitiveType::from_dtype(&DataType::BinaryView)?, PrimitiveType::Binary);
assert_eq!(
    PrimitiveType::from_dtype(&DataType::decimal64(9, 2)?)?,
    PrimitiveType::Decimal { precision: 9, scale: 2 }
);

// A datatype Iceberg cannot express is reported, never approximated.
let message = PrimitiveType::from_dtype(&DataType::Int8).unwrap_err().to_string();
assert!(message.contains("int8"));
assert!(PrimitiveType::from_dtype(&DataType::Int16).is_err());

// A UUID is the core's own `uuid`, so the spelling survives the round trip
// in the datatype rather than in a marker beside the column.
assert_eq!(PrimitiveType::Uuid.into_dtype()?, DataType::Uuid);
assert_eq!(
    PrimitiveType::from_dtype(&PrimitiveType::Uuid.into_dtype()?)?.to_string(),
    "uuid"
);
```

`iceberg` is a [compatibility target](../../types/datatype.md) like `spark` and `polars`, so lossless widenings live in one walker.

```rust
use yggdryl::media::iceberg::PrimitiveType;
use yggdryl::{DataType, Scheme};

// The narrow integers widen; the refusals stay refusals.
let widened = DataType::Int8.into_scheme_compat(&Scheme::ICEBERG)?;
assert_eq!(widened, DataType::Int32);
assert_eq!(PrimitiveType::from_dtype(&widened)?.to_string(), "int");
assert!(DataType::Interval(yggdryl::TimeUnit::YearMonth).into_scheme_compat(&Scheme::ICEBERG).is_err());
```

## Nested types

Rust only.

```rust
use yggdryl::media::iceberg::{schema_from_json, schema_into_json};
use yggdryl::{DataType};
use yggdryl::text::json;

let document = json::from_utf8(
    r#"{"type":"struct","schema-id":0,"fields":[
        {"id":1,"name":"legs","required":false,"type":{
            "type":"list","element-id":2,"element":{
                "type":"struct","fields":[
                    {"id":3,"name":"price","required":true,"type":"decimal(18, 4)"}
                ]
            },"element-required":true
        }},
        {"id":4,"name":"tags","required":false,"type":{
            "type":"map","key-id":5,"key":"string","value-id":6,"value":"int",
            "value-required":false
        }}
    ]}"#,
)?;

let schema = schema_from_json("row", &document)?;

// A list becomes a `List` whose item field is named `element` and carries `element-id`.
let legs = &schema.fields()[0];
let DataType::List(element) = legs.dtype() else { panic!("expected a list") };
assert_eq!(element.name(), "element");
assert_eq!(element.parquet_field_id()?, Some(2));
assert!(!element.is_nullable());
assert_eq!(element.fields()[0].name(), "price");

// A map becomes a `Map` over a non-null `entries` struct of `key` and `value`.
let tags = &schema.fields()[1];
let DataType::Map(map) = tags.dtype() else { panic!("expected a map") };
assert_eq!(map.entries().name(), "entries");
assert!(!map.entries().is_nullable());
assert!(!map.entries().fields()[0].is_nullable());
assert!(map.entries().fields()[1].is_nullable());
assert_eq!(map.entries().fields()[0].parquet_field_id()?, Some(5));

assert_eq!(schema_into_json(&schema)?, document);
```

`struct`, `list`, and `map` nest to any depth; `element`, `key`, `value`, and `entries` are synthesized names carrying `element-id`, `key-id`, and `value-id`.

## Into a data file

Rust only.

```rust
use arrow_array::RecordBatch;
use yggdryl::arrow;
use yggdryl::media::iceberg::schema_from_json;
use yggdryl::IOMedia;
use yggdryl::holder::Buffer;
use yggdryl::text::json;
use yggdryl::media::parquet::Parquet;

let document = json::from_utf8(
    r#"{"type":"struct","fields":[
        {"id":7,"name":"id","required":true,"type":"long"},
        {"id":8,"name":"symbol","required":false,"type":"string"}
    ]}"#,
)?;
let schema = schema_from_json("row", &document)?;

let mut media = Parquet::new(Buffer::new());
let options = media.record_options()?;
media.overwrite_arrow_reader(
    arrow::batch_reader(
        schema.into_arrow_schema()?,
        std::iter::empty::<RecordBatch>(),
    ),
    &options,
)?;

// The ids Iceberg assigned are the ids in the file.
let written = media.read_arrow_field(&options)?;
assert_eq!(written.fields()[0].parquet_field_id()?, Some(7));
assert_eq!(written.fields()[1].parquet_field_id()?, Some(8));
assert!(!written.fields()[0].is_nullable());
```

[Parquet](../parquet.md) has no Iceberg code path: it stores `PARQUET:field_id` in the [footer](../parquet-footer.md) and reads it back, so a reader resolves columns by id.

## Edges

- unnumbered tree -> `schema_into_json` fails and the message names `assign_field_ids`; `Table::create` numbers first.
- `assign_field_ids` on numbered fields -> ids kept, the return equals `start`; the root is never numbered.
- Python `assign_field_ids` and JavaScript `assignFieldIds` -> return a numbered copy; the input is left as it was.
- `can_promote(int64, int32)` -> refused, the message reads `int64 to int32`.
- equivalent schema, spec, or sort order -> the builder's canonical id is reused; a conflicting requested id is reassigned.
- `from_dtype` of `int8`, `uint32`, `interval`, `union`, `decimal256`, or a non micro/nano unit -> refused naming the datatype.
- `Scheme::ICEBERG` -> `Int8` widens to `Int32`; `Interval` stays refused.
- `LargeUtf8`, `BinaryView`, `decimal64` -> collapse onto `string`, `binary`, `decimal(p, s)`.
- `unknown` (v3) -> `DataType::Null`, every value reads as null.
- `Uuid` -> `DataType::Uuid`, spelled `uuid` on the way back.
- map key -> always required; absent `element-required` or `value-required` -> required.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::evolve::tests
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::schema_documents
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::types
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::datatype_coverage
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_iceberg.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/iceberg.test.js
    ```
