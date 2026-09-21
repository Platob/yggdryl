# Run-end

A run-ends column beside the values it repeats: two children, one encoding, and no trace of it in the value a reader gets back.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::RunEndEncoded(RunEndEncodedType)`, the `RunEndType` payload and the `RunEndEncodedField` marker |
| Validates | At construction: the run-ends field is non-null and its datatype is `int16`, `int32` or `int64`; the values field carries whatever it carries |
| Lazy | Nothing - both children are checked once, where the encoding is built |
| Cached | The encoding behind one `Arc<RunEndEncodedType>`, so a clone shares both children; the Arrow projection on the [`Field`](../field.md) |
| Refuses | Nullable run ends, a run-ends width that is not one of the three signed ones, and a child count that is not two |
| Kinds | `DataTypeKind::Nested`, id `run_end_encoded` (`0x9b`); it is a wrapper, so `is_nested()` follows the value it encodes |
| Bindings | `RunEndEncodedType` is Rust only: Python and JavaScript build the encoding from two fields and read its children by name |

## DataType

`DataType::run_end_encoded(run_ends, values)` takes two whole fields, because
both of them are columns: the run ends say where each run finishes, and the
values say what each run holds. Two rules are checked, independently, and the
refusal names the one that fired so a caller fixes the right half.

| child | position | rule |
| --- | ---: | --- |
| `run_ends` | 0 | non-null, and `int16`, `int32` or `int64` - the width bounds how long a column may be |
| `values` | 1 | any datatype, carrying its own name, nullability and metadata |

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind, Field};

    let runs = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::utf8(), true),
    )?;

    assert_eq!(runs.id(), DataTypeId::RunEndEncoded);
    assert_eq!(runs.kind(), DataTypeKind::Nested);
    assert!(DataTypeId::RunEndEncoded.is_wrapper());
    assert!(!runs.is_nested());

    // Two children, positional and named.
    assert_eq!(runs.field_len(), 2);
    assert_eq!(runs.get_field_at(0).map(Field::name), Some("run_ends"));
    assert_eq!(runs.get_field_at(1).map(Field::name), Some("values"));
    let DataType::RunEndEncoded(encoding) = &runs else { panic!("run-end") };
    assert_eq!(encoding.run_ends().dtype(), &DataType::Int32);
    assert_eq!(encoding.values().dtype(), &DataType::utf8());

    // Two independent rules, each reported on its own.
    let unsigned = DataType::run_end_encoded(
        Field::new("run_ends", DataType::UInt32, false),
        Field::new("values", DataType::utf8(), true),
    ).unwrap_err().to_string();
    assert!(unsigned.contains("int16, int32, or int64"), "{unsigned}");

    let nullable = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, true),
        Field::new("values", DataType::utf8(), true),
    ).unwrap_err().to_string();
    assert!(nullable.contains("non-null run_ends field"), "{nullable}");

    assert_eq!(DataType::from_str("ree<int32,utf8>")?, runs);
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    from yggdryl import DataType

    runs = yggdryl.run_end_encoded(
        "runs",
        yggdryl.int32("run_ends", nullable=False),
        yggdryl.utf8("values"),
    ).dtype

    assert runs.id == "run_end_encoded"
    assert runs.kind == "nested"
    assert runs.is_wrapper
    assert not runs.is_nested

    # Two children, positional and named.
    assert len(runs) == 2
    assert [field.name for field in runs] == ["run_ends", "values"]
    assert str(runs["values"].dtype) == "utf8"

    # Two independent rules, each reported on its own.
    with pytest.raises(ValueError, match="int16, int32, or int64"):
        yggdryl.run_end_encoded("bad", yggdryl.uint32("run_ends", nullable=False), yggdryl.utf8("values"))
    with pytest.raises(ValueError, match="non-null run_ends field"):
        yggdryl.run_end_encoded("bad", yggdryl.int16("run_ends"), yggdryl.utf8("values"))

    assert DataType("ree<int32,utf8>") == runs
    assert DataType("runend<int32,utf8>").id == "run_end_encoded"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const runs = fields.runEndEncoded(
      'runs',
      fields.int16('run_ends', { nullable: false }),
      fields.utf8('values'),
    ).dtype

    assert.equal(runs.id, 'run_end_encoded')
    assert.equal(runs.kind, 'nested')
    assert.equal(runs.nested, false)

    // Two children, positional and named.
    assert.equal(runs.length, 2)
    assert.deepEqual(runs.keys(), ['run_ends', 'values'])
    assert.equal(runs.getField('values').dtype.toString(), 'utf8')

    // Two independent rules, each reported on its own.
    assert.throws(
      () => fields.runEndEncoded('bad', fields.uint32('run_ends', { nullable: false }), fields.utf8('values')),
      /int16, int32, or int64/,
    )
    assert.throws(
      () => fields.runEndEncoded('bad', fields.int16('run_ends'), fields.utf8('values')),
      /non-null run_ends field/,
    )
    assert.ok(DataType.from('ree<int16,utf8>').equals(runs))
    ```

## Field

`RunEndEncodedField` is the typed marker: one field carrying `RunEndType`, the
shared encoding and nothing else. Unlike a [dictionary](dictionary.md#field),
a run-end field carries no sidecar - the encoding is entirely in the datatype -
so the field is a name, a nullability and metadata, exactly like every other.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, DataTypeId, Field, RunEndEncodedField};

    let runs = Field::new(
        "runs",
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::utf8(), true),
        )?,
        true,
    );
    let leaf = RunEndEncodedField::from_field(&runs).expect("the field is the run-end leaf");

    assert_eq!(leaf.name(), "runs");
    assert_eq!(leaf.id(), DataTypeId::RunEndEncoded);
    assert_eq!(leaf.typed_dtype_ref().encoding().values().name(), "values");
    assert_eq!(leaf.to_field(), runs);

    // No sidecar: the encoding is the datatype, and nothing rides beside it.
    assert_eq!(runs.dictionary_id(), None);

    // A datatype from another family is refused by name.
    let refused = RunEndEncodedField::try_new("runs", DataType::utf8(), true).unwrap_err().to_string();
    assert!(refused.contains("run_end_encoded"), "{refused}");
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    runs = yggdryl.run_end_encoded(
        "runs",
        yggdryl.int32("run_ends", nullable=False),
        yggdryl.utf8("values"),
        nullable=False,
        metadata={"owner": "events"},
    )

    assert isinstance(runs, Field)
    assert runs.name == "runs"
    assert runs.nullable is False
    assert runs.metadata["owner"] == "events"

    # No sidecar: the encoding is the datatype, and nothing rides beside it.
    assert runs.dictionary_id is None

    # The values child keeps its own name, nullability and metadata.
    encoded = yggdryl.run_end_encoded(
        "runs",
        yggdryl.int16("run_ends", nullable=False),
        yggdryl.utf8("values", nullable=False, metadata={"logical": "status"}),
    )
    assert encoded.dtype["values"].nullable is False
    assert encoded.dtype["values"].metadata["logical"] == "status"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const runs = fields.runEndEncoded(
      'runs',
      fields.int32('run_ends', { nullable: false }),
      fields.utf8('values'),
      { nullable: false, metadata: { owner: 'events' } },
    )

    assert.ok(runs instanceof Field)
    assert.equal(runs.name, 'runs')
    assert.equal(runs.nullable, false)
    assert.equal(runs.get('owner'), 'events')

    // No sidecar: the encoding is the datatype, and nothing rides beside it.
    assert.equal(runs.dictionaryId, null)
    assert.equal(runs.dtype.getField('values').nullable, true)
    ```

## Scalar

The encoding is a storage decision, so it is not in the value: a cell of a
run-end column is the value it decodes to, under the values datatype. There is
no run-end `Scalar` variant, no run index in a value, and nothing downstream
has to know a column was run-end encoded to read it.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};

    let runs = Field::new(
        "runs",
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::utf8(), true),
        )?,
        false,
    );

    let value = runs.scalar("X")?;
    assert_eq!(value, Scalar::from("X"));
    assert_eq!(value.dtype()?, DataType::utf8());

    // The default is the values datatype's, because the value is all there is.
    assert_eq!(runs.default_value()?, Scalar::from(""));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    runs = yggdryl.run_end_encoded(
        "runs",
        yggdryl.int32("run_ends", nullable=False),
        yggdryl.utf8("values"),
        nullable=False,
    )

    value = runs.scalar("X")
    assert value.as_py() == "X"
    assert value.dtype == DataType("utf8")
    assert value.family == "text"

    # The default is the values datatype's, because the value is all there is.
    assert runs.default_scalar().as_py() == ""
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const runs = fields.runEndEncoded(
      'runs',
      fields.int16('run_ends', { nullable: false }),
      fields.utf8('values'),
      { nullable: false },
    )

    const value = runs.scalar('X')
    assert.equal(value.asJs(), 'X')
    assert.equal(value.dtype.toString(), 'utf8')

    // The default is the values datatype's, because the value is all there is.
    assert.equal(runs.defaultJSValue(), '')
    ```

## Arrow storage

`ArrowDataType::RunEndEncoded(run_ends, values)`: both children cross as whole
fields, names and nullability included, with no extension document. On the C
Data Interface the node is `+r` with the two children in the same order, so the
encoding survives the boundary exactly as declared.

| datatype | Arrow storage | extension name |
| --- | --- | --- |
| `run_end_encoded(run_ends,values)` | `RunEndEncoded(run_ends, values)` | none |

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let runs = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::utf8(), true),
    )?;
    let arrow = runs.clone().into_arrow_datatype()?;

    let ArrowDataType::RunEndEncoded(run_ends, values) = &arrow else { panic!("run-end") };
    assert_eq!(run_ends.name(), "run_ends");
    assert!(!run_ends.is_nullable());
    assert_eq!(values.data_type(), &ArrowDataType::Utf8);
    assert_eq!(DataType::from_arrow_datatype(&arrow)?, runs);

    // The C Data Interface node carries the same two children.
    let ffi = runs.clone().into_arrow_datatype_ffi()?;
    assert_eq!(ArrowDataType::try_from(&ffi)?, arrow);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import DataType, Field

    runs = yggdryl.run_end_encoded(
        "runs",
        yggdryl.int32("run_ends", nullable=False),
        yggdryl.utf8("values"),
    )
    arrow = runs.into_arrow()

    assert arrow.type == pa.run_end_encoded(pa.int32(), pa.string())
    assert arrow.metadata is None
    assert Field.from_arrow(arrow).dtype == runs.dtype
    assert DataType.from_arrow(pa.run_end_encoded(pa.int16(), pa.int64())).id == "run_end_encoded"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    // Apache Arrow JS has no run-end layout to materialize, so a default
    // Arrow scalar is refused by name rather than approximated.
    const runs = fields.runEndEncoded(
      'runs',
      fields.int16('run_ends', { nullable: false }),
      fields.utf8('values'),
    )
    assert.throws(() => runs.defaultArrowScalar(), /unsupported/)

    // The datatype itself is complete, and round-trips through its own text.
    assert.equal(runs.dtype.id, 'run_end_encoded')
    assert.equal(runs.dtype.keys().length, 2)
    ```

## Exploding a run-end column

A run-end column is a collection for the purpose of
[`explode_fields`](../field.md#flattening-and-expanding): the column stands for
its values, so expanding one replaces the encoding with what it encodes,
keeping the column's name and its metadata. `unnest_fields` treats it as one
leaf, because it is one column.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let row = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::utf8(), true),
        )?
        .required_field("state"),
    ])?);

    let exploded = row.explode_fields();
    assert_eq!(exploded.iter().map(Field::name).collect::<Vec<_>>(), ["id", "state"]);
    assert_eq!(exploded[1].dtype(), &DataType::utf8(), "a run-end answers its values");
    // The values child is nullable, so the expanded column is too.
    assert!(exploded[1].is_nullable());

    assert_eq!(row.unnest_fields().iter().map(Field::name).collect::<Vec<_>>(), ["id", "state"]);
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    row = DataType.from_fields([
        yggdryl.int64("id", nullable=False),
        yggdryl.run_end_encoded(
            "state",
            yggdryl.int32("run_ends", nullable=False),
            yggdryl.utf8("values"),
            nullable=False,
        ),
    ])

    exploded = row.explode_fields()
    assert [field.name for field in exploded] == ["id", "state"]
    assert str(exploded[1].dtype) == "utf8"
    assert exploded[1].nullable is True

    assert [field.name for field in row.unnest_fields()] == ["id", "state"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const row = DataType.fromFields([
      fields.int64('id', { nullable: false }),
      fields.runEndEncoded(
        'state',
        fields.int32('run_ends', { nullable: false }),
        fields.utf8('values'),
        { nullable: false },
      ),
    ])

    const exploded = row.explodeFields()
    assert.deepEqual(exploded.map((field) => field.name), ['id', 'state'])
    assert.equal(exploded[1].dtype.toString(), 'utf8')

    assert.deepEqual(row.unnestFields().map((field) => field.name), ['id', 'state'])
    ```

## Edges

- Nullable run ends -> `expected a non-null run_ends field, got nullable field "<name>"`; an unsigned or narrower width -> `expected a run_ends datatype of int16, int32, or int64, got <type>`. Two rules, reported one at a time.
- The run-ends width bounds how long a run-end column may be, which is why `int16` is legal and `uint32` is not: the ends are signed offsets into the logical column.
- The children are positional: `run_ends` is `0` and `values` is `1`, and `with_fields` takes exactly two in that order.
- `kind()` is `nested` and `is_nested()` is not: the encoding is nested storage, and the shape is the values'.
- A run-end field carries no sidecar, unlike a [dictionary](dictionary.md) field: `dictionary_id` answers `None`, because nothing about this encoding is per-column transport.
- A value is the decoded value: there is no run-end `Scalar` variant, no run index in a value, and the default is the values datatype's default.
- Apache Arrow JS has no run-end layout, so `defaultArrowScalar` refuses the column by name in JavaScript; every other surface answers.
- The two children round-trip through the C Data Interface as `+r`, and through `arrow_schema` as whole fields, so the names a schema declared are the names that come back.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test expression -- path::nested
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- datatype::arrow datatype_kind::nested field::generic field::nested mapping::nested merge::nested parser::nested protocol::nested structure::nested union::variants
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(nested_datatype_clone|nested_validate)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py python/tests/test__defaults.py -k "run_end or nested"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="runEnd|run_end|nested" node/tests/fields.test.js node/tests/defaults.test.js
    npm run --prefix node bench:types
    ```
