# Nested

Child-bearing datatypes: read-only children, dictionary and run-end wrappers, and unions with the `variant(...)` sugar.

## Contract

| Aspect | Rule |
| --- | --- |
| Children | Positional, read-only: list 1, map 1 (`entries`), run-end 2, struct/union as declared |
| `as_fields` | Struct only, else `None` |
| Mutation | On [`Field`](field.md) only |
| Wrappers | `kind` `nested`; `is_nested` follows the encoded value |
| Dictionary | Integer key; ordering and id on the `Field` |
| Run-end | `run_ends` non-null `int16`/`int32`/`int64`; `values` carries type and nullability |
| Union | Unique non-negative `i8` ids; `Dense` or `Sparse` |
| `variant(...)` | Dense union, ids `0..`, at most 128 members |
| Parser | `variant(...)`, `dense_union(...)`, `sparse_union(...)` -> `union(<mode>, ...)` |

## Use

Every child-bearing type answers length and item access alike.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let quote = DataType::from_fields([
        Field::new("symbol", DataType::Utf8, false),
        Field::new("levels", DataType::list(DataType::Float64.nullable_field("item")), true),
    ])?;

    assert_eq!(quote.field_len(), 2);
    assert_eq!(quote.get_field(0).map(Field::name), Some("symbol"));
    assert_eq!(quote.get_field_by_path("levels").unwrap().dtype().field_len(), 1);
    assert!(quote.get_field_by_path("missing").is_none());

    // Every child-bearing type answers the same two questions.
    let lookup = DataType::map_of(DataType::Utf8, DataType::Int64, true)?;
    assert_eq!(lookup.field_len(), 1);
    assert_eq!(lookup.get_field(0).map(Field::name), Some("entries"));
    assert!(lookup.as_fields().is_none() && quote.as_fields().is_some());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, types

    quote = DataType.from_fields([
        Field("symbol", "utf8", nullable=False),
        types.list("levels", types.float64("item")),
    ])

    assert len(quote) == 2
    assert [field.name for field in quote] == ["symbol", "levels"]
    assert quote[0].name == "symbol"
    assert quote[-1].name == "levels"
    assert len(quote["levels"].dtype) == 1
    assert "levels" in quote and "missing" not in quote

    lookup = types.map_of("lookup", "utf8", "int64", keys_sorted=True).dtype
    assert len(lookup) == 1
    assert lookup[0].name == "entries"
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
    ```

## Encodings that wrap a value

A wrapper is a storage decision, so `is_nested` resolves through it.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind, Field};

    let codes = DataType::dictionary(DataType::Int16, DataType::Utf8)?;
    let runs = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::Utf8, true),
    )?;

    assert_eq!(codes.kind(), DataTypeKind::Nested);
    assert_eq!(runs.kind(), DataTypeKind::Nested);
    // A wrapper reports the shape of what it encodes, not of its own storage.
    assert!(!codes.is_nested() && !runs.is_nested());

    let DataType::Dictionary(dictionary) = &codes else { panic!("dictionary") };
    assert_eq!(dictionary.key(), &DataType::Int16);
    assert_eq!(dictionary.value(), &DataType::Utf8);

    // The key must be an integer; run ends must be a non-null int16, int32, or int64.
    assert!(DataType::dictionary(DataType::Utf8, DataType::Utf8).is_err());
    assert!(DataType::run_end_encoded(
        Field::new("run_ends", DataType::UInt32, false),
        Field::new("values", DataType::Utf8, true),
    ).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import types

    codes = types.dictionary("codes", "int16", "utf8").dtype
    runs = types.run_end_encoded(
        "runs",
        types.int16("run_ends", nullable=False),
        types.utf8("values"),
    ).dtype

    assert codes.kind == "nested"
    assert runs.kind == "nested"
    assert not codes.is_nested and not runs.is_nested
    assert str(codes) == "dictionary(int16,utf8)"

    with pytest.raises(ValueError, match="integer key datatype"):
        types.dictionary("bad", "utf8", "utf8")
    with pytest.raises(ValueError, match="int16, int32, or int64"):
        types.run_end_encoded(
            "bad", types.uint32("run_ends", nullable=False), types.utf8("values")
        )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const codes = fields.dictionary('codes', 'int16', 'utf8').dtype
    const runs = fields
      .runEndEncoded('runs', fields.int16('run_ends', { nullable: false }), fields.utf8('values'))
      .dtype

    assert.equal(codes.kind, 'nested')
    assert.equal(runs.kind, 'nested')
    assert.equal(codes.nested, false)
    assert.equal(runs.nested, false)
    assert.equal(codes.toString(), 'dictionary(int16,utf8)')

    assert.throws(() => fields.dictionary('bad', 'utf8', 'utf8'), /integer key datatype/)
    assert.throws(
      () => fields.runEndEncoded('bad', fields.uint32('run_ends', { nullable: false }), fields.utf8('values')),
      /int16, int32, or int64/,
    )
    ```

## Unions and the dense-union sugar

The sugar spells `DataType::dense_union`, `DataType.variant(fields)`, or `fields.dense_union` / `fields.denseUnion` (alias `DenseUnionField`); explicit ids and modes use `DataType::union` or `fields.union`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, UnionMode};

    let members = [
        Field::new("number", DataType::Int64, false),
        Field::new("text", DataType::Utf8, true),
    ];
    let members_union = DataType::dense_union(members.clone())?;

    // The member sugar is not a second logical type: it is the dense union
    // with IDs 0.. - and bare `variant` is a datatype of its own, so the
    // parenthesis is what disambiguates the two spellings.
    assert_eq!(
        members_union,
        DataType::union(
            [(0, members[0].clone()), (1, members[1].clone())],
            UnionMode::Dense,
        )?
    );
    assert_eq!(members_union.name(), "union");
    assert!(members_union.to_string().starts_with("union(dense,"));
    assert_eq!(DataType::from_str("variant(number:int64,text:string)")?.name(), "union");
    assert_eq!(DataType::from_str("variant")?, DataType::Variant);

    // An explicit union picks its own mode and its own non-negative type IDs.
    let sparse = DataType::union(
        [(7, Field::new("only", DataType::Int32, false))],
        UnionMode::Sparse,
    )?;
    assert_eq!(sparse.get_field(0).map(Field::name), Some("only"));
    assert!(DataType::union(
        [(0, members[0].clone()), (0, members[1].clone())],
        UnionMode::Dense,
    ).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Field

    variant = DataType.variant([
        Field("number", "int64", nullable=False),
        Field("text", "utf8"),
    ])

    assert variant.id == "union"
    assert str(variant).startswith("union(dense,")
    assert [field.name for field in variant] == ["number", "text"]
    assert DataType("variant(number:int64,text:string)").id == "union"

    with pytest.raises(ValueError, match="duplicate field name"):
        DataType.variant([Field("same", "int64"), Field("same", "utf8")])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const variant = DataType.variant([
      fields.int64('number'),
      fields.utf8('text', { nullable: true }),
    ])

    assert.equal(variant.id, 'union')
    assert.ok(variant.toString().startsWith('union(dense,'))
    assert.deepEqual(variant.keys(), ['number', 'text'])
    assert.equal(DataType.from('variant(number:int64,text:string)').id, 'union')

    assert.throws(
      () => DataType.variant([fields.int64('same'), fields.utf8('same')]),
      /duplicate field name/,
    )
    ```

## Edges

- Duplicate member names -> `duplicate field name` error.
- Unknown child name -> `None`; `in` / `contains` false.
- Non-integer dictionary key -> `integer key datatype` error.
- Nullable or unsigned `run_ends` -> `int16, int32, or int64` error.
- Duplicate union type id -> `Err`.
- Bare `variant` -> the [Variant datatype](geospatial.md), not a union.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::nested datatype::parser::variant datatype::parser::union field::nested
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(nested_datatype_clone|datatype_stable_hash|nested_validate|struct_from_fields_1024|variant_from_fields_128)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py python/tests/types/test_factories.py -k "nested or variant or union or map or dictionary or from_fields or read_only"
    python/.venv/bin/python python/benchmarks/types.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="variant|recursive|fromFields|nested|Union" node/tests/types/datatype.test.js node/tests/types/fields.test.js node/tests/types/defaults.test.js
    npm run --prefix node bench:types
    ```
