# Dictionary

A value stored as a code that stands for it: an integer key column over a value column, and the two Arrow options that describe that encoding on the field rather than on the type.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Enum(EnumType)` with its one `Dictionary` leaf over `DictionaryType`, the `EnumField` marker, and the `DictionaryOptions` sidecar a dictionary field carries |
| Validates | At construction: the key is one of the eight integer datatypes; the value is any datatype |
| Lazy | Nothing - the two datatypes are checked once, where the leaf is built |
| Cached | The pair behind one `Arc<DictionaryType>`, so a clone shares both datatypes; the Arrow projection on the [`Field`](../field.md) |
| Refuses | A non-integer key, and dictionary options on any field that is not dictionary-typed |
| Kinds | `DataTypeKind::Nested`, id `dictionary` (`0x9a`); it is a wrapper, so it carries no child field and `is_nested()` follows the value it encodes |
| Bindings | `EnumType` and `DictionaryType` are Rust only: Python reads `dictionary_key` / `dictionary_value` off the datatype, and both bindings read the options off the field |

The name is about what the column means, not how it is laid out: the values are
drawn from a closed set and the rows carry codes into it. That is a different
fact from the [vocabulary](../text/string.md#the-vocabulary-a-string-column-declares)
a *name* is drawn from, whose datatype is `string`.

## DataType

`DataType::dictionary(key, value)` is the whole constructor: a key column of
codes and a value column they stand for. The key is checked once - it is one of
`int8`, `int16`, `int32`, `int64`, `uint8`, `uint16`, `uint32`, `uint64` - and
the value is whatever the column decodes to, a nested datatype included.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind, EnumType};

    let codes = DataType::dictionary(DataType::Int16, DataType::utf8())?;
    assert_eq!(codes.to_string(), "dictionary(int16,utf8)");
    assert_eq!(codes.id(), DataTypeId::Dictionary);
    assert_eq!(codes.kind(), DataTypeKind::Nested);

    // A wrapper reports the shape of what it encodes, not of its own storage.
    assert!(DataTypeId::Dictionary.is_wrapper());
    assert!(!codes.is_nested());
    assert_eq!(codes.field_len(), 0);

    // The pair reads back as two datatypes, never as two fields.
    let DataType::Enum(EnumType::Dictionary(dictionary)) = &codes else { panic!("dictionary") };
    assert_eq!(dictionary.key(), &DataType::Int16);
    assert_eq!(dictionary.value(), &DataType::utf8());

    // The key must be an integer; the value may be anything, nested included.
    assert!(DataType::dictionary(DataType::utf8(), DataType::utf8()).is_err());
    assert!(DataType::dictionary(DataType::Float64, DataType::utf8()).is_err());
    assert_eq!(DataType::from_str("dict<int16,string>")?, codes);
    assert!(DataType::dictionary(
        DataType::Int32,
        DataType::list(DataType::utf8().nullable_field("item")),
    )?.is_nested());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, types

    codes = types.dictionary("codes", "int16", "utf8").dtype
    assert str(codes) == "dictionary(int16,utf8)"
    assert codes.id == "dictionary"
    assert codes.kind == "nested"

    # A wrapper reports the shape of what it encodes, not of its own storage.
    assert codes.is_wrapper
    assert not codes.is_nested
    assert len(codes) == 0

    # The pair reads back as two datatypes, never as two fields.
    assert str(codes.dictionary_key) == "int16"
    assert str(codes.dictionary_value) == "utf8"
    assert DataType("int64").dictionary_key is None

    # The key must be an integer; a Python or pyarrow type names one too.
    assert str(types.dictionary("status", int, str).dtype) == "dictionary(int64,utf8)"
    with pytest.raises(ValueError, match="integer key datatype"):
        types.dictionary("bad", "utf8", "utf8")
    assert DataType("dict<int16,string>") == codes
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const codes = fields.dictionary('codes', 'int16', 'utf8').dtype
    assert.equal(codes.toString(), 'dictionary(int16,utf8)')
    assert.equal(codes.id, 'dictionary')
    assert.equal(codes.kind, 'nested')

    // A wrapper reports the shape of what it encodes, not of its own storage.
    assert.equal(codes.nested, false)
    assert.equal(codes.length, 0)

    // The key must be an integer.
    assert.throws(() => fields.dictionary('bad', 'utf8', 'utf8'), /integer key datatype/)
    assert.ok(DataType.from('dict<int16,string>').equals(codes))
    ```

## Field

`EnumField` is the typed marker, and a dictionary field is the one field in the
crate that carries a sidecar: Arrow's IPC dictionary identifier and its
ordering flag. Those two facts describe *this column's* encoding, not its
datatype - two dictionary fields with different identifiers have the same
datatype - so they ride on the field rather than costing every other field
sixteen bytes to say it has none. Every field that is not dictionary-typed
answers `None` and refuses to be given them.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let mut codes = Field::new("codes", DataType::dictionary(DataType::Int16, DataType::utf8())?, true);
    assert_eq!(codes.dictionary_id(), Some(0));
    assert_eq!(codes.dictionary_is_ordered(), Some(false));

    codes.set_dictionary_options(42, true)?;
    assert_eq!(codes.dictionary_id(), Some(42));
    assert_eq!(codes.dictionary_is_ordered(), Some(true));
    assert!(codes.to_string().contains("dictionary_id=42"));
    assert!(codes.to_string().contains("dictionary_is_ordered=true"));

    // The persistent form is the same rule, returning the field.
    let wide = Field::new("wide", DataType::dictionary(DataType::Int16, DataType::utf8())?, true)
        .try_with_dictionary_options(9_007_199_254_740_993, false)?;
    assert_eq!(wide.dictionary_id(), Some(9_007_199_254_740_993));

    // Every other field carries none, and refuses to be given one.
    let mut plain = Field::new("id", DataType::Int64, false);
    assert_eq!(plain.dictionary_id(), None);
    let refused = plain.set_dictionary_options(1, true).unwrap_err().to_string();
    assert!(refused.contains("dictionary datatype"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import types

    codes = types.dictionary("codes", "int16", "utf8", metadata={"logical": "status"})
    assert codes.dictionary_id == 0
    assert codes.dictionary_is_ordered is False

    codes.set_dictionary_options(42, True)
    assert codes.dictionary_id == 42
    assert codes.dictionary_is_ordered is True
    assert codes.metadata["logical"] == "status"

    # Every other field carries none, and refuses to be given one.
    plain = types.int64("id")
    assert plain.dictionary_id is None
    with pytest.raises(ValueError, match="dictionary datatype"):
        plain.set_dictionary_options(1, True)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    // The identifier is an i64, so it crosses as a bigint at every width.
    const codes = fields.dictionary('codes', 'int16', 'utf8', {
      metadata: { logical: 'status' },
    })
    assert.equal(codes.dictionaryId, 0n)
    assert.equal(codes.dictionaryIsOrdered, false)

    codes.setDictionaryOptions(42n, true)
    assert.equal(codes.dictionaryId, 42n)
    assert.equal(codes.dictionaryIsOrdered, true)
    assert.ok(codes.toString().includes('dictionary_id=42'))
    assert.equal(codes.get('logical'), 'status')

    // Every other field carries none, and refuses to be given one.
    const plain = fields.int64('id')
    assert.equal(plain.dictionaryId, null)
    assert.throws(() => plain.setDictionaryOptions(1n, true), /dictionary datatype/)
    ```

## Scalar

The encoding is a storage decision, so it is not in the value: a cell of a
dictionary column is the value it decodes to, under the value datatype. There
is no `Scalar` variant for a code, and nothing downstream has to know a column
was dictionary-encoded to read it.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};

    let codes = Field::new("codes", DataType::dictionary(DataType::Int16, DataType::utf8())?, true);
    let value = codes.scalar("AAPL")?;

    assert_eq!(value, Scalar::from("AAPL"));
    assert_eq!(value.dtype()?, DataType::utf8());

    // The default is the value datatype's, because the value is all there is.
    let required = Field::new("codes", DataType::dictionary(DataType::Int16, DataType::utf8())?, false);
    assert_eq!(required.default_value()?, Scalar::from(""));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, types

    codes = types.dictionary("codes", "int16", "utf8")
    value = codes.scalar("AAPL")

    assert value.as_py() == "AAPL"
    assert value.dtype == DataType("utf8")
    assert value.family == "text"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const codes = fields.dictionary('codes', 'int16', 'utf8')
    const value = codes.scalar('AAPL')

    assert.equal(value.asJs(), 'AAPL')
    assert.equal(value.dtype.toString(), 'utf8')
    assert.equal(value.family, 'text')
    assert.equal(
      fields.dictionary('codes', 'int16', 'utf8', { nullable: false }).defaultJSValue(),
      '',
    )
    ```

## Arrow storage

`ArrowDataType::Dictionary(key, value)`, with the ordering flag on the Arrow
field beside it. The ordering crosses both ways, everywhere. The IPC dictionary
identifier is transport rather than schema - the IPC writer assigns each
dictionary column one of its own - so it survives only where the boundary has a
place for it: `arrow_schema::Field` keeps it, and the C Data Interface and
pyarrow, which carry the ordering flag alone, do not.

| datatype | Arrow storage | carried on the field |
| --- | --- | --- |
| `dictionary(int16,utf8)` | `Dictionary(Int16, Utf8)` | `ordered`, from `dictionary_is_ordered` |
| any dictionary | - | `dictionary_id` is transport: `arrow_schema::Field` carries one, the C Data Interface and pyarrow do not, and the IPC writer assigns its own |

On the C Data Interface an encoded extension declares its identity once, on the
node the field is: Arrow's dictionary values are a bare datatype, so the outer
node writes the extension entries and the values node does not.

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let codes = DataType::dictionary(DataType::Int16, DataType::utf8())?;
    let arrow = codes.clone().into_arrow_datatype()?;
    assert_eq!(
        arrow,
        ArrowDataType::Dictionary(Box::new(ArrowDataType::Int16), Box::new(ArrowDataType::Utf8))
    );
    assert_eq!(DataType::from_arrow_datatype(&arrow)?, codes);

    // The ordering flag crosses on the field; the transport id does not.
    let ordered = Field::new("codes", codes, true).try_with_dictionary_options(42, true)?;
    let projected = ordered.clone().into_arrow_field()?;
    assert_eq!(projected.dict_is_ordered(), Some(true));
    let imported = Field::from_arrow_field(&projected)?;
    assert_eq!(imported.dictionary_is_ordered(), Some(true));
    // `arrow_schema::Field` still carries an identifier, so this one survives;
    // the C Data Interface has only the ordering flag, and drops it.
    assert_eq!(imported.dictionary_id(), Some(42));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field, types

    codes = types.dictionary("codes", "int16", "utf8")
    codes.set_dictionary_options(42, True)
    arrow = codes.into_arrow()

    assert arrow.type == pa.dictionary(pa.int16(), pa.string(), ordered=True)
    assert arrow.type.ordered is True

    # The ordering crosses back; a pyarrow field has no place for the
    # transport identifier, so that one does not.
    imported = Field.from_arrow(arrow)
    assert imported.dictionary_is_ordered is True
    assert imported.dictionary_id == 0
    assert DataType.from_arrow(pa.dictionary(pa.int8(), pa.string())).id == "dictionary"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    // A cast through a struct root answers the Arrow field a column is written as.
    const row = fields.struct('row', [fields.dictionary('codes', 'int16', 'utf8')], {
      nullable: false,
    })
    const table = row.castArrow(
      new arrow.Table({ codes: arrow.vectorFromArray(['AAPL'], new arrow.Utf8()) }),
    )

    const projected = table.schema.fields[0]
    assert.equal(projected.name, 'codes')
    assert.ok(arrow.DataType.isDictionary(projected.type))
    assert.equal(projected.type.indices.toString(), 'Int16')
    ```

## Exploding a dictionary column

A dictionary is a collection for the purpose of
[`explode_fields`](../field.md#flattening-and-expanding): the column stands for
its value, so expanding one replaces the encoding with what it encodes, keeping
the column's name and its metadata.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let row = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::dictionary(DataType::Int16, DataType::utf8())?.required_field("codes"),
    ])?);

    let exploded = row.explode_fields();
    assert_eq!(exploded.iter().map(Field::name).collect::<Vec<_>>(), ["id", "codes"]);
    assert_eq!(exploded[0].dtype(), &DataType::Int64, "not a collection");
    assert_eq!(exploded[1].dtype(), &DataType::utf8(), "a dictionary answers its value");

    // `unnest_fields` treats it as one leaf, because it is one column.
    assert_eq!(row.unnest_fields().iter().map(Field::name).collect::<Vec<_>>(), ["id", "codes"]);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, types

    row = DataType.from_fields([
        types.int64("id", nullable=False),
        types.dictionary("codes", "int16", "utf8", nullable=False),
    ])

    exploded = row.explode_fields()
    assert [field.name for field in exploded] == ["id", "codes"]
    assert str(exploded[1].dtype) == "utf8"

    # `unnest_fields` treats it as one leaf, because it is one column.
    assert [field.name for field in row.unnest_fields()] == ["id", "codes"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const row = DataType.fromFields([
      fields.int64('id', { nullable: false }),
      fields.dictionary('codes', 'int16', 'utf8', { nullable: false }),
    ])

    const exploded = row.explodeFields()
    assert.deepEqual(exploded.map((field) => field.name), ['id', 'codes'])
    assert.equal(exploded[1].dtype.toString(), 'utf8')

    // `unnestFields` treats it as one leaf, because it is one column.
    assert.deepEqual(row.unnestFields().map((field) => field.name), ['id', 'codes'])
    ```

## Edges

- A non-integer key -> `expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got <type>`, at construction and again at `validate`.
- A dictionary carries two datatypes, not two fields: `field_len()` is `0`, `as_fields()` is `None`, and there is nothing for a path to descend into.
- `kind()` is `nested` and `is_nested()` is not: the encoding is nested storage, and the shape is the value's. `dictionary(int16,list(...))` answers `true`.
- Dictionary options on any other field -> `dictionary options require a dictionary datatype`; every other field answers `None` rather than a default.
- The ordering flag survives every Arrow round trip; the IPC dictionary identifier survives only `arrow_schema::Field`, and a field imported through the C Data Interface or pyarrow reads `0` - it is the writer's transport id, not part of the schema two readers agree on.
- Two dictionary fields with different identifiers have the same datatype, which is exactly why the identifier is on the field.
- A value is the decoded value: there is no `Scalar` variant for a code, and a cell's datatype is the value datatype.
- A merge keeps the options only when both sides have them, and the merged column is ordered only when both are - see [Field](../field.md#merging-two-schemas).
- Bare Arrow has no `sized` or extension notion here: a dictionary of an extension-typed value declares that identity on the outer node, so an importer reads it once.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::nested datatype::arrow field::nested field::generic
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(nested_datatype_clone|nested_validate)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py python/tests/types/test_field.py python/tests/types/test_annotation_options.py -k "dictionary"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="dictionary|nested" node/tests/types/field.test.js node/tests/types/fields.test.js node/tests/types/defaults.test.js
    npm run --prefix node bench:types
    ```
