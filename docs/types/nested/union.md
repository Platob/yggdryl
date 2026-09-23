# Union

One of several member fields per row: a non-negative type id beside each member, a layout mode over all of them, and the `variant(...)` sugar for the common dense case.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Union(UnionFields, UnionMode)`, the `UnionType` payload and the `UnionField` marker; `UnionMode` is the two-variant layout vocabulary |
| Validates | At construction: every type id is non-negative and occurs once, every member name occurs once, and every member validates |
| Lazy | Nothing - the members are collected and checked once, and a union with no members holds no allocation |
| Cached | The members in one shared `Arc<[(i8, Field)]>`, so a clone shares them; the Arrow projection on the [`Field`](../field.md) |
| Refuses | A negative or duplicate type id, a duplicate member name, more than 128 members through the dense sugar, a sparse `variant(...)`, and a value whose type id names no member |
| Kinds | `DataTypeKind::Nested`, one id `union` (`0x99`) for both modes - the mode is a parameter, not a second identifier |
| Bindings | `UnionFields` and `UnionMode` are Rust only: Python and JavaScript pass `(type_id, field)` pairs and a mode word; Python reads them back as `union_mode` and `union_type_ids`, and JavaScript, which has no accessor for either, reads them out of the canonical display |

## DataType

A union is its members paired with their Arrow type ids, plus the mode those
ids are laid out under. `DataType::union` takes both; `DataType::dense_union`
is the common case written short - members in input order, ids `0..`, mode
`Dense` - and it is not a second logical type: it builds the same
`DataType::Union` the explicit call does, which is why display, serialization,
the Arrow projection and row materialization all reuse one contract.

| mode | `as_str` | layout |
| --- | --- | --- |
| `UnionMode::Sparse` | `sparse` | every child has the union's full logical length |
| `UnionMode::Dense` | `dense` | values are packed in each child and addressed by an offset buffer |

The canonical display is `union(<mode>,<id>=<field>,...)`, and the parser reads
`union(...)`, `dense_union(...)`, `sparse_union(...)` and `variant(...)` into
it. Bare `variant` is not a union at all - it is
[the semi-structured datatype](../variant.md) - so the parenthesis is what
disambiguates the two spellings.

=== "Rust"

    ```rust
    use std::str::FromStr as _;

    use yggdryl::{DataType, DataTypeId, Field, UnionFields, UnionMode};

    let members = [
        Field::new("number", DataType::Int64, false),
        Field::new("text", DataType::utf8(), true),
    ];
    let dense = DataType::dense_union(members.clone())?;

    // The sugar is the dense union with ids 0.., not a second logical type.
    assert_eq!(dense, DataType::union([(0, members[0].clone()), (1, members[1].clone())], UnionMode::Dense)?);
    assert_eq!(dense.id(), DataTypeId::Union);
    assert_eq!(dense.name(), "union");
    assert_eq!(dense.field_len(), 2);
    assert!(dense.to_string().starts_with("union(dense,"));
    assert_eq!(DataType::from_str("variant(number:int64,text:string)")?.name(), "union");
    assert_eq!(DataType::from_str("variant")?, DataType::Variant);

    // An explicit union picks its own mode and its own non-negative ids.
    let sparse = DataType::union([(7, Field::new("only", DataType::Int32, false))], UnionMode::Sparse)?;
    let DataType::Union(fields, mode) = &sparse else { panic!("union") };
    assert_eq!(*mode, UnionMode::Sparse);
    assert_eq!(mode.as_str(), "sparse");
    assert_eq!(UnionMode::ALL, [UnionMode::Sparse, UnionMode::Dense]);
    assert_eq!(UnionMode::from_str("Dense")?, UnionMode::Dense);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields.get(0).map(|(id, field)| (id, field.name())), Some((7, "only")));
    assert_eq!(fields.get_by_name("only").map(|(id, _)| id), Some(7));
    assert!(UnionFields::from_fields([(0, members[0].clone()), (0, members[1].clone())]).is_err());
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    from yggdryl import DataType, Field

    variant = DataType.variant([
        Field("number", "int64", nullable=False),
        Field("text", "utf8"),
    ])

    # The sugar is the dense union with ids 0.., not a second logical type.
    assert variant.id == "union"
    assert variant.union_mode == "dense"
    assert variant.union_type_ids == (0, 1)
    assert str(variant).startswith("union(dense,")
    assert [field.name for field in variant] == ["number", "text"]
    assert DataType("variant(number:int64,text:string)").id == "union"
    assert DataType("variant") != variant

    # An explicit union picks its own mode and its own non-negative ids.
    sparse = yggdryl.union("choice", [(0, yggdryl.int64("number", nullable=False)), (7, yggdryl.utf8("text"))], "sparse")
    assert sparse.dtype.union_mode == "sparse"
    assert sparse.dtype.union_type_ids == (0, 7)
    assert DataType("int64").union_mode is None

    with pytest.raises(ValueError, match="duplicate type id"):
        yggdryl.union("bad", [(0, yggdryl.int64("a")), (0, yggdryl.utf8("b"))], "dense")
    with pytest.raises(ValueError, match="non-negative"):
        yggdryl.union("bad", [(-1, yggdryl.int64("a"))], "dense")
    with pytest.raises(ValueError, match="duplicate field name"):
        DataType.variant([Field("same", "int64"), Field("same", "utf8")])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const variant = DataType.variant([
      fields.int64('number', { nullable: false }),
      fields.utf8('text', { nullable: true }),
    ])

    // The sugar is the dense union with ids 0.., not a second logical type.
    assert.equal(variant.id, 'union')
    assert.ok(variant.toString().startsWith('union(dense,'))
    assert.deepEqual(variant.keys(), ['number', 'text'])
    assert.equal(DataType.from('variant(number:int64,text:string)').id, 'union')
    assert.ok(fields.denseUnion('payload', [
      fields.int64('number', { nullable: false }),
      fields.utf8('text', { nullable: true }),
    ]).dtype.equals(variant))

    // An explicit union picks its own mode and its own non-negative ids.
    const sparse = fields.union('choice', [[0, fields.int64('number')], [7, fields.utf8('text')]], 'sparse')
    assert.ok(sparse.dtype.toString().startsWith('union(sparse,'))
    assert.deepEqual(sparse.dtype.keys(), ['number', 'text'])

    assert.throws(
      () => fields.union('bad', [[0, fields.int64('a')], [0, fields.utf8('b')]], 'dense'),
      /duplicate type id/,
    )
    assert.throws(() => DataType.variant([fields.int64('same'), fields.utf8('same')]), /duplicate field name/)
    ```

## Field

`UnionField` is the typed marker: one field carrying `UnionType`, the members
and their mode together. The bindings have two factories - `union` taking
`(type_id, field)` pairs and a mode word, `dense_union` / `denseUnion` taking
bare members - and `DataType.variant` builds the datatype without a field
around it.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, DataTypeId, Field, UnionField, UnionMode};

    let payload = Field::new(
        "payload",
        DataType::union([(3, Field::new("number", DataType::Int64, false))], UnionMode::Dense)?,
        true,
    );
    let leaf = UnionField::from_field(&payload).expect("the field is the Union leaf");

    assert_eq!(leaf.name(), "payload");
    assert_eq!(leaf.id(), DataTypeId::Union);
    assert_eq!(*leaf.typed_dtype_ref().mode(), UnionMode::Dense);
    assert_eq!(leaf.typed_dtype_ref().fields().get(0).map(|(id, _)| id), Some(3));
    assert_eq!(leaf.to_field(), payload);

    // A datatype from another family is refused by name.
    let refused = UnionField::try_new("payload", DataType::utf8(), true).unwrap_err().to_string();
    assert!(refused.contains("union"), "{refused}");
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    payload = yggdryl.dense_union(
        "payload",
        [yggdryl.int64("number", nullable=False), yggdryl.utf8("text")],
        nullable=False,
        metadata={"logical": "variant"},
    )

    assert isinstance(payload, Field)
    assert payload.nullable is False
    assert payload.dtype.union_mode == "dense"
    assert payload.metadata["logical"] == "variant"
    assert yggdryl.DenseUnionField is Field

    # A member keeps its own name, nullability and metadata.
    assert payload.dtype["number"].nullable is False
    assert yggdryl.union("choice", [(3, yggdryl.int32("member"))], "dense").dtype.union_type_ids == (3,)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const payload = fields.denseUnion(
      'payload',
      [fields.int64('number', { nullable: false }), fields.utf8('text')],
      { nullable: false, metadata: { logical: 'variant' } },
    )

    assert.ok(payload instanceof Field)
    assert.equal(payload.nullable, false)
    assert.equal(payload.get('logical'), 'variant')

    // A member keeps its own name, nullability and metadata.
    assert.equal(payload.dtype.getField('number').nullable, false)
    assert.ok(fields.union('choice', [[3, fields.int32('member')]], 'dense').dtype.toString().includes('3='))
    ```

## Scalar

A union value is the two things a row of one needs: the type id that names the
branch, and the payload under that branch's field. It is stored as the ordered
pair - a two-item `Scalar::Serie` - and the id canonicalizes to `Int64`, so
a narrower spelling of the same number reads back the same value.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, UnionMode};

    let payload = Field::new(
        "payload",
        DataType::union(
            [
                (0, Field::new("number", DataType::Int64, false)),
                (1, Field::new("text", DataType::utf8(), true)),
            ],
            UnionMode::Dense,
        )?,
        true,
    );

    // The type id names the branch; the payload is read under that member.
    let number = payload.scalar(Scalar::from_sequence([Scalar::from(0_i64), Scalar::from(7_i64)]))?;
    assert_eq!(number, Scalar::from_sequence([Scalar::from(0_i64), Scalar::from(7_i64)]));
    let text = payload.scalar(Scalar::from_sequence([Scalar::from(1_i32), Scalar::from("hi")]))?;
    assert_eq!(text, Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("hi")]));

    // A type id no member carries names no branch.
    let refused = payload
        .scalar(Scalar::from_sequence([Scalar::from(9_i64), Scalar::from(1_i64)]))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("unknown union type id 9"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    payload = yggdryl.dense_union("p", [yggdryl.int64("number", nullable=False), yggdryl.utf8("text")])

    # The type id names the branch; the payload is read under that member.
    assert payload.scalar([0, 7]).as_py() == [0, 7]
    assert payload.scalar([1, "hi"]).as_py() == [1, "hi"]

    # A type id no member carries names no branch.
    with pytest.raises(ValueError, match="unknown union type id 9"):
        payload.scalar([9, 1])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const payload = fields.denseUnion('p', [
      fields.int64('number', { nullable: false }),
      fields.utf8('text'),
    ])

    // The type id names the branch; the payload is read under that member.
    assert.deepEqual(payload.scalar([0n, 7n]).asJs(), [0, 7])
    assert.deepEqual(payload.scalar([1n, 'hi']).asJs(), [1, 'hi'])

    // A type id no member carries names no branch.
    assert.throws(() => payload.scalar([9n, 1n]), /unknown union type id 9/)
    ```

## Arrow storage

`ArrowDataType::Union(fields, mode)`: the type ids and the members cross
exactly as declared, and the mode is Arrow's own. On the C Data Interface the
format string spells the mode and then every type id in order - `+ud:0,1` for a
dense union of ids `0` and `1` - so the tags survive the boundary rather than
being renumbered.

| datatype | Arrow storage | extension name |
| --- | --- | --- |
| `union(dense,...)` | `Union(fields, Dense)` | none |
| `union(sparse,...)` | `Union(fields, Sparse)` | none |

=== "Rust"

    ```rust
    use arrow_schema::{DataType as ArrowDataType, UnionMode as ArrowUnionMode};
    use yggdryl::{DataType, Field, UnionMode};

    let dense = DataType::union(
        [
            (0, Field::new("number", DataType::Int64, false)),
            (7, Field::new("text", DataType::utf8(), true)),
        ],
        UnionMode::Dense,
    )?;
    let arrow = dense.clone().into_arrow_datatype()?;

    // The ids are declared, not positional, so they cross as they are.
    let ArrowDataType::Union(fields, mode) = &arrow else { panic!("union") };
    assert_eq!(*mode, ArrowUnionMode::Dense);
    assert_eq!(fields.iter().map(|(id, _)| id).collect::<Vec<_>>(), [0, 7]);
    assert_eq!(DataType::from_arrow_datatype(&arrow)?, dense);

    // The C Data Interface node spells the mode and every id.
    let ffi = dense.clone().into_arrow_datatype_ffi()?;
    assert_eq!(ArrowDataType::try_from(&ffi)?, arrow);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import DataType

    payload = yggdryl.dense_union("payload", [yggdryl.int64("number", nullable=False), yggdryl.utf8("text")])
    arrow = payload.into_arrow()

    assert arrow.type.mode == "dense"
    assert tuple(arrow.type.type_codes) == (0, 1)
    assert arrow.metadata is None
    assert DataType.from_arrow(arrow.type) == payload.dtype
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    // The mode and the ids are the datatype, so the canonical text carries both.
    const dense = fields.union(
      'payload',
      [[0, fields.int64('number', { nullable: false })], [7, fields.utf8('text')]],
      'dense',
    ).dtype

    assert.ok(dense.toString().startsWith('union(dense,0='))
    assert.ok(dense.toString().includes(',7='))
    assert.ok(DataType.from(dense.toString()).equals(dense))
    ```

## The dense-union sugar

`variant(...)` is the input spelling for "these members, in this order, dense,
ids from zero". It stays sugar: its canonical display is `union(dense,...)`, so
a schema written that way reads back as a union. The name moved because bare
`variant` is now a datatype of its own - the self-describing semi-structured
value Iceberg v3, Parquet and Doris share - and one word cannot name both a
union of declared members and a value that declares itself.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    // Ids from zero, in input order, and Arrow's id capacity is the limit.
    let accepted = DataType::dense_union(
        (0..128).map(|index| Field::new(format!("member_{index}"), DataType::Int64, true)),
    )?;
    assert_eq!(accepted.field_len(), 128);
    assert_eq!(accepted.get_field(127).map(Field::name), Some("member_127"));

    let error = DataType::dense_union(
        (0..129).map(|index| Field::new(format!("member_{index}"), DataType::Int64, true)),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("variant cannot contain more than 128 members"), "{error}");

    // The sugar is not a second type, so it round-trips as a union.
    let sugar = DataType::from_str("variant(number:int64,text:string)")?;
    assert_eq!(DataType::from_str(&sugar.to_string())?, sugar);
    assert!(DataType::from_str("variant(sparse,number:int64)").is_err());
    assert_eq!(DataType::dense_union([])?, DataType::union([], yggdryl::UnionMode::Dense)?);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    # The sugar is not a second type, so it round-trips as a union.
    sugar = DataType("variant(number:int64,text:string)")
    assert sugar.id == "union"
    assert sugar.union_mode == "dense"
    assert DataType(str(sugar)) == sugar

    # Bare `variant` is the semi-structured datatype, not a union.
    assert DataType("variant").id == "variant"
    with pytest.raises(ValueError, match="variant layout must be dense"):
        DataType("variant(sparse,number:int32)")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // The sugar is not a second type, so it round-trips as a union.
    const sugar = DataType.from('variant(number:int64,text:string)')
    assert.equal(sugar.id, 'union')
    assert.ok(DataType.from(sugar.toString()).equals(sugar))

    // Bare `variant` is the semi-structured datatype, not a union.
    assert.equal(DataType.from('variant').id, 'variant')
    assert.throws(() => DataType.fromString('variant(sparse,number:int32)'), /dense/i)
    ```

## Edges

- A negative type id -> `type id must be non-negative`; a repeated one -> `duplicate type id`; a repeated member name -> `duplicate field name`. All three fire where the members are collected.
- A type id must fit an `i8`, so a union holds at most 128 members and the dense sugar says so by name.
- `variant(...)` requires dense and sequential ids from zero; a sparse one, or an out-of-order id, is refused at the position that broke the rule.
- Bare `variant` is [the semi-structured datatype](../variant.md), and `variant(...)` with members is this sugar. The parenthesis is the whole of the difference.
- The mode is a parameter, not an identifier: `union(dense,...)` and `union(sparse,...)` share `DataTypeId::Union`, and are still different datatypes.
- A union value is the pair `[type_id, payload]`; the id canonicalizes to `Int64`, and an id no member carries -> `unknown union type id <n>`.
- A union has one child per member, so `with_fields` takes exactly that many and keeps the declared ids and the mode.
- A union member is a whole [`Field`](../field.md): it carries its own name, nullability and metadata, and those survive the Arrow round trip.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test expression -- path::nested
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- datatype_kind::nested field::generic field::nested mapping::nested merge::nested parser::grammar parser::nested protocol::nested structure::nested union::variants
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/variant_from_fields_128'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "union or variant"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="variant|Union|union" node/tests/datatype.test.js node/tests/fields.test.js
    npm run --prefix node bench:types
    ```
