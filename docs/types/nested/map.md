# Map

Keys to values: one `entries` field holding a key and a value, and a leaf that says whether the keys are ordered within a row.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Mapping(MappingType)` with its two leaves over one `MapType`, the `MappingField` marker, and the `Map` value under `Mapping` |
| Validates | At construction: the entries field is non-null, holds a struct of exactly two children, and its key child is non-null |
| Lazy | Nothing - the entries are checked once, where the leaf is built |
| Cached | The parameters behind one `Arc<MapType>`, so a clone shares the whole entries schema; the Arrow projection on the [`Field`](../field.md) |
| Refuses | Nullable entries, entries that are not a key-and-value struct, a nullable key, a duplicate key in a value, and a value that is neither a mapping nor a record |
| Kinds | `DataTypeKind::Nested`, ids `map` (`0x97`) and `sorted_map` (`0x98`); `is_nested()` is `true` |
| Bindings | `MappingType` and `Map` are Rust only: Python and JavaScript build a map through `map` / `map_of` and read a stored value back as a [`Scalar`](../scalar.md) |

## DataType

Key order is not a flag beside the type: it *is* the leaf. A sorted map is a
`SortedMap`, so nothing downstream carries a boolean it has to remember to
honour, and a reader that needs ordered keys asks the type. Both leaves hold
the same `MapType` - the non-null entries struct Arrow spells a map with -
which is why `keys_sorted` reads the leaf and can never disagree with it.

| leaf | id | keys | Arrow storage |
| --- | --- | --- | --- |
| `map` | `0x97` | in no particular order | `Map(entries, false)` |
| `sorted_map` | `0x98` | ordered within each row | `Map(entries, true)` |

`DataType::map` takes the whole entries field; `DataType::map_of` builds it from
a key and a value datatype under Arrow's own names, `key` non-null and `value`
nullable. The canonical display is always the entries form.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, Field, MappingType};

    let lookup = DataType::map_of(DataType::utf8(), DataType::Int64, false)?;
    assert_eq!(lookup.id(), DataTypeId::Map);
    assert_eq!(lookup.field_len(), 1);
    assert_eq!(lookup.get_field_at(0).map(Field::name), Some("entries"));

    // The promise is the leaf, and `keys_sorted` reads it.
    let sorted = DataType::map_of(DataType::utf8(), DataType::Int64, true)?;
    assert_eq!(sorted.id(), DataTypeId::SortedMap);
    let leaf = sorted.as_mapping().expect("a mapping");
    assert!(leaf.keys_sorted());
    assert!(matches!(leaf, MappingType::SortedMap(_)));
    assert_eq!(leaf.entries().name(), "entries");
    assert_eq!(leaf.parameters().entries().dtype().field_len(), 2);
    assert!(sorted.to_string().ends_with("keys_sorted=true)"));
    assert_eq!(DataType::from_str("map<string,int64,keys_sorted=true>")?, sorted);

    // The entries are the key-and-value struct, checked once, here.
    assert!(DataType::map(DataType::Int64.required_field("entries"), false).is_err());
    assert_eq!(DataType::Int64.as_mapping(), None);
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    from yggdryl import DataType

    lookup = yggdryl.map_of("lookup", "utf8", "int64").dtype
    assert lookup.id == "map"
    assert lookup.kind == "nested"
    assert lookup.keys_sorted is False
    assert len(lookup) == 1
    assert lookup[0].name == "entries"

    # The entries are the key-and-value struct Arrow spells a map with.
    entries = lookup[0].dtype
    assert [child.name for child in entries] == ["key", "value"]
    assert entries["key"].nullable is False
    assert entries["value"].nullable is True

    # The promise is the leaf, and `keys_sorted` reads it.
    sorted_map = yggdryl.map_of("lookup", "utf8", "int64", keys_sorted=True).dtype
    assert sorted_map.id == "sorted_map"
    assert sorted_map.keys_sorted is True
    assert DataType("map<string,int64,keys_sorted=true>") == sorted_map
    assert DataType("int64").keys_sorted is None

    with pytest.raises(ValueError, match="key and a value"):
        yggdryl.map("bad", yggdryl.int64("entries", nullable=False))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const lookup = fields.mapOf('lookup', 'utf8', 'int64').dtype
    assert.equal(lookup.id, 'map')
    assert.equal(lookup.kind, 'nested')
    assert.equal(lookup.length, 1)
    assert.equal(lookup.getFieldAt(0).name, 'entries')

    // The entries are the key-and-value struct Arrow spells a map with.
    const entries = lookup.getFieldAt(0).dtype
    assert.deepEqual(entries.keys(), ['key', 'value'])
    assert.equal(entries.getField('key').nullable, false)

    // The promise is the leaf: the sorted map is its own identifier.
    const sorted = fields.mapOf('lookup', 'utf8', 'int64', true).dtype
    assert.equal(sorted.id, 'sorted_map')
    assert.ok(DataType.from('map<string,int64,keys_sorted=true>').equals(sorted))
    assert.throws(
      () => fields.map('bad', fields.int64('entries', { nullable: false })),
      /key and a value/,
    )
    ```

## Field

`MappingField` is the typed marker: one field carrying `MappingType`, so the
leaf and its entries are read off the payload. The bindings have two factories
- `map` taking the whole entries field, `map_of` / `mapOf` taking a key and a
value - and `keys_sorted` picks the leaf in both.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, DataTypeId, Field, MappingField};

    let lookup = Field::new("lookup", DataType::map_of(DataType::utf8(), DataType::Int64, true)?, true);
    let leaf = MappingField::from_field(&lookup).expect("the field is the Mapping leaf");

    assert_eq!(leaf.name(), "lookup");
    assert_eq!(leaf.id(), DataTypeId::SortedMap);
    assert!(leaf.typed_dtype_ref().keys_sorted());
    assert_eq!(leaf.typed_dtype_ref().entries().name(), "entries");
    assert_eq!(leaf.to_field(), lookup);

    // A datatype from another family is refused by name.
    let refused = MappingField::try_new("lookup", DataType::utf8(), true).unwrap_err().to_string();
    assert!(refused.contains("mapping"), "{refused}");

    // The entries field can be given by hand, name, nullability and all.
    let entries = DataType::map_of(DataType::utf8(), DataType::Int64, false)?
        .get_field_at(0)
        .cloned()
        .expect("a map has entries");
    assert_eq!(DataType::map(entries, true)?, DataType::map_of(DataType::utf8(), DataType::Int64, true)?);
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    lookup = yggdryl.map_of("lookup", "utf8", "int64", keys_sorted=True, nullable=False)
    assert isinstance(lookup, Field)
    assert lookup.name == "lookup"
    assert lookup.nullable is False
    assert lookup.dtype.id == "sorted_map"

    # `map` takes the entries field itself, which `map_of` builds for you.
    entries = yggdryl.struct(
        "entries",
        [Field("key", "utf8", nullable=False), Field("value", "int64")],
        nullable=False,
    )
    assert yggdryl.map("lookup", entries, keys_sorted=True).dtype == lookup.dtype

    # A key or a value may be named by a Python or a pyarrow type.
    assert yggdryl.map_of("labels", str, int).dtype[0].dtype["value"].dtype.id == "int64"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const lookup = fields.mapOf('lookup', 'utf8', 'int64', true, { nullable: false })
    assert.ok(lookup instanceof Field)
    assert.equal(lookup.name, 'lookup')
    assert.equal(lookup.nullable, false)
    assert.equal(lookup.dtype.id, 'sorted_map')

    // `map` takes the entries field itself, which `mapOf` builds for you.
    const entries = fields.struct(
      'entries',
      [fields.utf8('key', { nullable: false }), fields.int64('value')],
      { nullable: false },
    )
    assert.ok(fields.map('lookup', entries, true).dtype.equals(lookup.dtype))
    ```

## Scalar

`Scalar::Mapping(Mapping::Map(Map))` is the value: the entries in order, in one
shared slice, with arbitrary scalar keys. A key occurs once - a duplicate is
refused where the value is built - and a record is accepted as a map value too,
its names becoming the keys, which is how a JSON or YAML object reaches a map
column.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Map, Mapping, NestedValue, Scalar};

    let lookup = DataType::map_of(DataType::utf8(), DataType::Int64, false)?;
    let value = lookup.scalar(Scalar::from_mapping([
        (Scalar::from("a"), Scalar::from(1_i64)),
        (Scalar::from("b"), Scalar::from(2_i64)),
    ])?)?;
    assert_eq!(value.len(), 2);

    // `Map` is the holder every mapping API answers with; `children` walks keys.
    let held = Map::new(vec![(Scalar::from("a"), Scalar::from(1_i64))]);
    assert_eq!(held.as_slice()[0].0, Scalar::from("a"));
    assert_eq!(held.children().count(), 1);
    assert_eq!(Mapping::from(held.clone()).as_map(), Some(&held));

    // A key occurs once.
    assert!(Scalar::from_mapping([
        (Scalar::from("a"), Scalar::from(1_i64)),
        (Scalar::from("a"), Scalar::from(2_i64)),
    ]).is_err());

    // A record reaches a map column: its names become the keys.
    let from_record = lookup.scalar(Scalar::from_struct([("a", Scalar::from(1_i64))])?)?;
    assert_eq!(from_record.len(), 1);
    ```

=== "Python"

    ```python
    import yggdryl

    lookup = yggdryl.map_of("lookup", "utf8", "int64")
    value = lookup.scalar({"a": 1, "b": 2})

    assert value.as_py() == {"a": 1, "b": 2}
    assert value.family == "nested"
    assert len(value) == 2

    # Walking a mapping walks its keys, which is what `children` answers.
    assert [child.as_py() for child in value] == ["a", "b"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const lookup = fields.mapOf('lookup', 'utf8', 'int64')
    const value = lookup.scalar(new Map([['a', 1n], ['b', 2n]]))

    assert.deepEqual(value.asJs(), new Map([['a', 1], ['b', 2]]))
    assert.equal(value.family, 'nested')
    assert.equal(value.length, 2)

    // Walking a mapping walks its keys, which is what `children` answers.
    assert.deepEqual([...value].map((child) => child.asJs()), ['a', 'b'])
    ```

## Arrow storage

`ArrowDataType::Map(entries, keys_sorted)`: the entries field crosses as it is,
and the promise about key order becomes Arrow's flag. What is a leaf here is a
flag there, so the two spellings can never disagree - and on the C Data
Interface the flag is `MAP_KEYS_SORTED`, written by this crate rather than by
Arrow's own field conversion, which drops it.

| datatype | Arrow storage | extension name |
| --- | --- | --- |
| `map(entries,keys_sorted=false)` | `Map(entries, false)` | none |
| `map(entries,keys_sorted=true)` | `Map(entries, true)` | none |

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::DataType;

    let sorted = DataType::map_of(DataType::utf8(), DataType::Int64, true)?;
    let arrow = sorted.clone().into_arrow_datatype()?;
    assert!(matches!(arrow, ArrowDataType::Map(_, true)));
    assert_eq!(DataType::from_arrow_datatype(&arrow)?, sorted);

    // The unsorted leaf is the same storage under the other flag.
    let plain = DataType::map_of(DataType::utf8(), DataType::Int64, false)?;
    assert!(matches!(plain.clone().into_arrow_datatype()?, ArrowDataType::Map(_, false)));
    assert_ne!(plain, sorted);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import DataType, Field

    sorted_map = yggdryl.map_of("lookup", "utf8", "int32", keys_sorted=True, nullable=False)
    arrow = sorted_map.into_arrow()

    assert arrow.type.equals(pa.map_(pa.string(), pa.int32(), keys_sorted=True))
    assert arrow.type.keys_sorted is True
    assert arrow.nullable is False
    assert Field.from_arrow(arrow).dtype.id == "sorted_map"
    assert DataType.from_arrow(pa.map_(pa.string(), pa.int32())).id == "map"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    // The flag is the leaf, so the identifier is the whole of the promise.
    assert.equal(fields.mapOf('lookup', 'utf8', 'int32', true).dtype.id, 'sorted_map')
    assert.equal(fields.mapOf('lookup', 'utf8', 'int32').dtype.id, 'map')
    assert.ok(
      DataType.from('map<utf8,int32,keys_sorted=true>').equals(
        fields.mapOf('lookup', 'utf8', 'int32', true).dtype,
      ),
    )
    ```

## A map is not transparent to a path

A [list](list.md#a-list-is-transparent-to-a-path) hides its item from a dotted
path; a map does not. Its one child is the `entries` field, addressed by name,
so a key is never a borrowed schema child and `mapping['entries']` names
nothing: a key is a value, and a schema walk only ever crosses fields.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let row = DataType::from(StructType::from_fields([
        DataType::map_of(DataType::utf8(), DataType::Int64, false)?.required_field("mapping"),
    ])?);

    // The entries are a step the path spells out.
    assert_eq!(row.get_field_by_path("mapping.entries.value").map(Field::name), Some("value"));
    assert!(row.get_field_by_path("mapping.value").is_none());

    // A key is a value, so a text-key selector names no schema child.
    assert!(row.get_field_by_path("mapping['entries']").is_none());
    assert!(row.field_by_path("mapping['entries']").is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    row = DataType("struct<mapping:map<utf8,int64>>")

    # The entries are a step the path spells out.
    assert row.get_field_by_path("mapping.entries.value").name == "value"
    assert row.get_field_by_path("mapping.value") is None

    # A key is a value, so a text-key selector names no schema child.
    assert row.get_field_by_path("mapping['entries']") is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const row = DataType.from('struct<mapping:map<utf8,int64>>')

    // The entries are a step the path spells out.
    assert.equal(row.getFieldByPath('mapping.entries.value').name, 'value')
    assert.equal(row.getFieldByPath('mapping.value'), null)

    // A key is a value, so a text-key selector names no schema child.
    assert.equal(row.getFieldByPath("mapping['entries']"), null)
    ```

## Edges

- Nullable entries -> `entries field must be non-null`; entries that are not a struct of exactly two children -> `entries field must hold a key and a value`; a nullable key child -> `key field must be non-null`. All three fire where the leaf is built and again at `validate`.
- `map_of` names the children `key` and `value`, the key non-null and the value nullable; `map` takes whatever entries field it is given, subject to those three rules.
- A duplicate key in a value -> `mapping contains a duplicate key`, with the position of the second one.
- A record reaches a map column - its names become the keys - but a mapping does not reach a [struct](struct.md) column: a row is an ordered sequence, never a map.
- Key order is the leaf: `map` and `sorted_map` are different datatypes, they are not equal, and a merge of one with the other does not meet.
- Arrow's own field conversion drops `MAP_KEYS_SORTED` when it adds a field's flags, so this crate writes the C Data Interface node itself.
- A map has exactly one child, so `with_fields` takes exactly one entries field and keeps the leaf's key ordering.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::nested datatype::arrow field::nested
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(nested_datatype_clone|nested_validate)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py python/tests/types/test_factories.py -k "map or nested"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="map|nested" node/tests/types/datatype.test.js node/tests/types/fields.test.js
    npm run --prefix node bench:types
    ```
