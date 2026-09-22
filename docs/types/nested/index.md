# Nested

The datatypes that hold other datatypes: four layouts that carry child fields, two encodings that wrap a value, and one `Nested` value over what they store.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Struct(StructType)`, `Sequence(SequenceType)`, `Mapping(MappingType)`, `Union(UnionFields, UnionMode)`, `Enum(EnumType)` and `RunEndEncoded(RunEndEncodedType)`; the values `Struct`, `Sequence` and `Map`, and `Nested`, the one family value over them |
| Validates | At construction, once: unique child names, non-negative and unique union type ids, a non-negative fixed list length, map entries that are a non-null struct of a key and a value, an integer dictionary key, and non-null `int16`/`int32`/`int64` run ends |
| Lazy | Nothing - a layout is checked when it is built and never re-derived; the `validate` a hand-built payload meets is the same check |
| Cached | The children of one layout live in one shared allocation, so a datatype clone shares them rather than walking them; the Arrow projection is cached on the [`Field`](../field.md) |
| Refuses | A duplicate child name, a duplicate or negative union type id, more than 128 union members, a child count that is not the layout's arity, and a value whose shape is not the one the layout declares |
| Kinds | `DataTypeKind::Nested` for every one of the twelve ids (`0x91`-`0x9c`); `is_nested()` resolves the two wrappers through the value they encode, so a `dictionary(int16,utf8)` answers `false` |
| Bindings | `Nested`, `Sequence`, `Map` and `Struct` are Rust only: Python and JavaScript read a stored value as a [`Scalar`](../scalar.md) whose `family` is `nested` |

## Pages

| page | owns |
| --- | --- |
| [Struct](struct.md) | `StructType`: named children in declaration order, and the non-null struct field that is the row schema |
| [List](list.md) | `SequenceType`: the five list layouts over one item field |
| [Map](map.md) | `MappingType`: the `entries` struct of a key and a value, and the leaf that promises sorted keys |
| [Union](union.md) | `UnionFields` and `UnionMode`: one of several member fields per row, and the `variant(...)` sugar |
| [Dictionary](dictionary.md) | `EnumType`: an integer key column over a value column, with Arrow's dictionary options on the field |
| [Run-end](runend.md) | `RunEndEncodedType`: a run-ends column beside the values it repeats |

Bare [`variant`](../variant.md) is a nested kind too, and a leaf of the `Nested`
value, but it is not a layout over children: it is the Parquet Variant binary
encoding of a value that describes itself, so it has a page of its own.

## What a layout carries

Children are positional and borrowed: a layout answers how many it has and
which one sits at a position, and a caller replaces one rather than reaching a
`&mut` into it. A dictionary is the one nested datatype with no children at
all - it carries two datatypes, not two fields.

| layout | children | named |
| --- | ---: | --- |
| `list`, `list_view`, `fixed_size_list(n)`, `large_list`, `large_list_view` | 1 | the item field |
| `struct` | as declared | each child's own name; `as_fields` borrows the slice |
| `map`, `sorted_map` | 1 | `entries` |
| `union` | one per member | each member's own name, beside its type id |
| `run_end_encoded` | 2 | `run_ends`, then `values` |
| `dictionary` | 0 | `key` and `value` are datatypes, not fields |

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, StructType};

    let quote = DataType::from(StructType::from_fields([
        Field::new("symbol", DataType::utf8(), false),
        Field::new("levels", DataType::list(DataType::Float64.nullable_field("item")), true),
    ])?);

    assert_eq!(quote.field_len(), 2);
    assert_eq!(quote.get_field(0).map(Field::name), Some("symbol"));
    assert_eq!(quote.get_field_by_path("levels").unwrap().dtype().field_len(), 1);
    assert!(quote.get_field_by_path("missing").is_none());

    // Every child-bearing type answers the same two questions.
    let lookup = DataType::map_of(DataType::utf8(), DataType::Int64, true)?;
    assert_eq!(lookup.field_len(), 1);
    assert_eq!(lookup.get_field(0).map(Field::name), Some("entries"));
    assert!(lookup.as_fields().is_none() && quote.as_fields().is_some());

    // A dictionary carries datatypes rather than fields, so it has no child.
    assert_eq!(DataType::dictionary(DataType::Int16, DataType::utf8())?.field_len(), 0);
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Field

    quote = DataType.from_fields([
        Field("symbol", "utf8", nullable=False),
        yggdryl.list("levels", yggdryl.float64("item")),
    ])

    assert len(quote) == 2
    assert [field.name for field in quote] == ["symbol", "levels"]
    assert quote[0].name == "symbol"
    assert quote[-1].name == "levels"
    assert len(quote["levels"].dtype) == 1
    assert "levels" in quote and "missing" not in quote

    lookup = yggdryl.map_of("lookup", "utf8", "int64", keys_sorted=True).dtype
    assert len(lookup) == 1
    assert lookup[0].name == "entries"

    # A dictionary carries datatypes rather than fields, so it has no child.
    assert len(yggdryl.dictionary("codes", "int16", "utf8").dtype) == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const quote = DataType.fromFields([
      fields.utf8('symbol'),
      fields.list('levels', fields.float64('item'), { nullable: true }),
    ])

    assert.equal(quote.length, 2)
    assert.deepEqual(quote.keys(), ['symbol', 'levels'])
    assert.equal(quote.getFieldAt(-1).name, 'levels')
    assert.equal(quote.getField('levels').dtype.length, 1)
    assert.equal(quote.contains('missing'), false)
    assert.deepEqual([...quote].map((field) => field.name), ['symbol', 'levels'])

    const lookup = fields.mapOf('lookup', 'utf8', 'int64', true).dtype
    assert.equal(lookup.length, 1)
    assert.equal(lookup.getFieldAt(0).name, 'entries')

    // A dictionary carries datatypes rather than fields, so it has no child.
    assert.equal(fields.dictionary('codes', 'int16', 'utf8').dtype.length, 0)
    ```

## The `Nested` value

`Nested` is the one value over the family: a `Sequence`, a `Mapping`, a
`Struct` or a [`Variant`](../variant.md). It answers the kind every leaf
shares, the datatype the held leaf's children name, and the `Scalar` it widens
back to. Each leaf answers `NestedValue`: how many direct children it has and
an iterator over them - a sequence's values, a mapping's **keys**, and a
record's values in sorted name order.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind, FamilyValue, Nested, NestedValue, Scalar, StructType};

    let sequence = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]);
    let mapping = Scalar::from_mapping([(Scalar::from("k"), Scalar::from(1_i64))])?;
    let record = Scalar::from_struct([("id", Scalar::from(1_i64))])?;

    assert_eq!(Nested::KIND, DataTypeKind::Nested);
    let held = Nested::from_scalar(&sequence).expect("a sequence");
    let Nested::Sequence(leaf) = &held else { panic!("a sequence") };
    assert_eq!(leaf.len(), 2);
    assert_eq!(leaf.children().count(), 2);
    assert_eq!(held.dtype()?, DataType::list(DataType::Int64.required_field("item")));
    assert_eq!(held.into_scalar(), sequence);

    // Every leaf narrows the same way, and no other family does.
    assert!(matches!(mapping.as_nested(), Some(Nested::Mapping(_))));
    assert!(matches!(record.as_nested(), Some(Nested::Struct(_))));
    assert_eq!(Scalar::from(1_i64).as_nested(), None);
    assert_eq!(
        Nested::from_scalar(&record).expect("a record").dtype()?,
        DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    );
    ```

=== "Python"

    ```python
    import yggdryl

    levels = yggdryl.list("levels", yggdryl.float64("item")).scalar([1.5, 2.5])
    lookup = yggdryl.map_of("lookup", "utf8", "int64").scalar({"a": 1})

    # The family is one word, and the length counts direct children.
    assert levels.family == "nested"
    assert lookup.family == "nested"
    assert len(levels) == 2
    assert [child.as_py() for child in levels] == [1.5, 2.5]

    # A mapping walks its keys, which is what `children` answers natively.
    assert len(lookup) == 1
    assert [child.as_py() for child in lookup] == ["a"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const levels = fields.list('levels', fields.float64('item')).scalar([1.5, 2.5])
    const lookup = fields.mapOf('lookup', 'utf8', 'int64').scalar(new Map([['a', 1n]]))

    // The family is one word, and the length counts direct children.
    assert.equal(levels.family, 'nested')
    assert.equal(lookup.family, 'nested')
    assert.equal(levels.length, 2)
    assert.deepEqual([...levels].map((child) => child.asJs()), [1.5, 2.5])

    // A mapping walks its keys, which is what `children` answers natively.
    assert.equal(lookup.length, 1)
    assert.deepEqual([...lookup].map((child) => child.asJs()), ['a'])
    ```

## Edges

- Duplicate child names -> `duplicate field name` error; a struct and a union both refuse them, and `from_fields` fails rather than keeping the first.
- An unknown child name -> `None`; `in` / `contains` answers false. A path that names no child reports the names that do exist.
- A layout keeps its arity: `with_fields` refuses a child count that is not the layout's, and a list refuses a second child rather than becoming a struct.
- Python `yggdryl.*` and JavaScript `fields.*` factories answer a `Field`, not a bare datatype; `.dtype` reaches the type.
- `as_fields` is a struct's alone: every other layout answers `None`, because its children are not a schema.
- A wrapper is a storage decision: `kind()` is `nested`, and `is_nested()` follows the value it encodes.
- Bare `variant` is [the semi-structured datatype](../variant.md), not a union; `variant(...)` with members is the dense-union sugar, and the parenthesis is what disambiguates them.
- A nested datatype is hashable and totally ordered, children included, so it is a map key and a cache key as it stands.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test expression -- path::nested
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- datatype_kind::nested field::generic field::nested mapping::nested merge::nested parser::nested protocol::nested structure::nested union::variants
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(nested_datatype_clone|datatype_stable_hash|nested_validate|struct_from_fields_1024|variant_from_fields_128)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "nested or variant or union or map or dictionary or from_fields or read_only"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="variant|recursive|fromFields|nested|Union" node/tests/datatype.test.js node/tests/fields.test.js node/tests/defaults.test.js
    npm run --prefix node bench:types
    ```
