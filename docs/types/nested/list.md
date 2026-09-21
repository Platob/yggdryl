# List

Many of one thing, in all five layouts Arrow gives it: one item field, and a leaf that says how the rows are laid out.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Sequence(SequenceType)` with its five leaves, the `SequenceField` marker, and the `Sequence` value |
| Validates | At construction: a fixed length is non-negative, and the item field validates |
| Lazy | Nothing - a leaf holds one shared item field and, on the fixed leaf, one `i32` |
| Cached | The item field behind one `Arc<Field>`, so a leaf clone shares it; the Arrow projection on the [`Field`](../field.md) |
| Refuses | A negative fixed length, a second child, and a value that is not a sequence of the item's datatype |
| Kinds | `DataTypeKind::Nested`, ids `0x91`-`0x95`; `is_nested()` is `true` |
| Bindings | `SequenceType` and `Sequence` are Rust only: Python and JavaScript build a leaf through its own factory and read a stored run back as a [`Scalar`](../scalar.md) |

## DataType

The five leaves differ in how the rows are laid out, never in what a row holds:
one item field, the same for all of them. That is why the item is the whole of
what most readers need, and why a dotted path treats it as a step it need not
spell.

| leaf | offsets | length | Arrow storage | also parsed as |
| --- | --- | --- | --- | --- |
| `list` (`0x91`) | 32-bit | variable | `List` | `array<T>` |
| `large_list` (`0x92`) | 64-bit | variable | `LargeList` | `largearray<T>` |
| `list_view` (`0x93`) | 32-bit, viewed | variable | `ListView` | `arrayview<T>` |
| `large_list_view` (`0x94`) | 64-bit, viewed | variable | `LargeListView` | `largearrayview<T>` |
| `fixed_size_list(n)` (`0x95`) | none | exactly `n` | `FixedSizeList(n)` | `fixedarray<T,n>` |

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind, Field, SequenceType};

    let levels = DataType::list(DataType::Float64.nullable_field("item"));
    assert_eq!(levels.to_string(), r#"list(field("item",float64,nullable=true,metadata={}))"#);
    assert_eq!(levels.id(), DataTypeId::List);
    assert_eq!(levels.kind(), DataTypeKind::Nested);
    assert!(levels.is_nested());
    assert_eq!(levels.field_len(), 1);
    assert_eq!(levels.get_field_at(0).map(Field::name), Some("item"));

    // The payload reads back the leaf: its item, its length, its layout.
    let fixed = DataType::fixed_size_list(DataType::Int32.nullable_field("item"), 3)?;
    let leaf = fixed.as_sequence_type().expect("a sequence");
    assert_eq!(leaf.fixed_length(), Some(3));
    assert_eq!(leaf.item().name(), "item");
    assert!(!leaf.is_view() && !leaf.is_large());

    let viewed = DataType::large_list_view(DataType::Int32.nullable_field("item"));
    let viewed_leaf = viewed.as_sequence_type().expect("a sequence");
    assert!(viewed_leaf.is_view() && viewed_leaf.is_large());
    assert_eq!(viewed_leaf.fixed_length(), None);
    assert_eq!(
        DataType::from(viewed_leaf.with_item(DataType::Int64.nullable_field("item"))),
        DataType::large_list_view(DataType::Int64.nullable_field("item"))
    );

    assert_eq!(DataType::from_str("array<int64>")?, DataType::list(DataType::Int64.nullable_field("item")));
    assert!(DataType::fixed_size_list(DataType::Int32.nullable_field("item"), -1).is_err());
    assert_eq!(DataType::Int64.as_sequence_type(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, types

    levels = types.list("levels", types.float64("item")).dtype
    assert levels.id == "list"
    assert levels.kind == "nested"
    assert levels.is_nested
    assert len(levels) == 1
    assert levels[0].name == "item"

    # One factory per leaf, each taking the item field it repeats.
    item = types.int32("item")
    assert types.list_view("value", item).dtype.id == "list_view"
    assert types.large_list("value", item).dtype.id == "large_list"
    assert types.large_list_view("value", item).dtype.id == "large_list_view"
    assert types.fixed_size_list("value", item, 3).dtype.id == "fixed_size_list"
    assert str(types.fixed_size_list("value", item, 3).dtype).endswith(",3)")

    # Every dialect's spelling parses into the same leaf.
    assert DataType("array<int64>") == DataType("list<int64>")
    assert DataType("fixedarray<int32,3>") == DataType("fixed_size_list(int32,3)")
    assert DataType("largearrayview<int64>").id == "large_list_view"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const levels = fields.list('levels', fields.float64('item')).dtype
    assert.equal(levels.id, 'list')
    assert.equal(levels.kind, 'nested')
    assert.equal(levels.nested, true)
    assert.equal(levels.length, 1)
    assert.equal(levels.getFieldAt(0).name, 'item')

    // One factory per leaf, each taking the item field it repeats.
    const item = fields.int32('item')
    assert.equal(fields.listView('value', item).dtype.id, 'list_view')
    assert.equal(fields.largeList('value', item).dtype.id, 'large_list')
    assert.equal(fields.largeListView('value', item).dtype.id, 'large_list_view')
    assert.equal(fields.fixedSizeList('value', item, 3).dtype.id, 'fixed_size_list')
    assert.ok(fields.fixedSizeList('value', item, 3).dtype.toString().endsWith(',3)'))

    // Every dialect's spelling parses into the same leaf.
    assert.ok(DataType.from('array<int64>').equals(DataType.from('list<int64>')))
    assert.equal(DataType.from('largearrayview<int64>').id, 'large_list_view')
    ```

## Field

`SequenceField` is the typed marker: one field carrying `SequenceType`, so the
leaf and its item are read off the payload rather than matched out of a root
datatype. The bindings have one factory per leaf, each taking the item field it
repeats; the column is nullable unless the call says otherwise, and the item's
own nullability is the item field's.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, DataTypeId, Field, SequenceField, SequenceType};

    let levels = SequenceField::new(
        "levels",
        SequenceType::List(std::sync::Arc::new(DataType::Float64.nullable_field("item"))),
        true,
    );
    assert_eq!(levels.name(), "levels");
    assert_eq!(levels.id(), DataTypeId::List);
    assert_eq!(levels.typed_dtype_ref().item().name(), "item");
    assert!(levels.is_nullable());

    // Widened, it is the same column the root constructor builds.
    let root: Field = levels.into_field();
    assert_eq!(root, Field::new("levels", DataType::list(DataType::Float64.nullable_field("item")), true));
    assert!(SequenceField::from_field(&root).is_some());

    // A datatype from another family is refused by name.
    let refused = SequenceField::try_new("levels", DataType::utf8(), true).unwrap_err().to_string();
    assert!(refused.contains("sequence"), "{refused}");

    // The item is a whole field, so it carries its own name and nullability.
    let required = DataType::list(DataType::Float64.required_field("item"));
    assert!(!required.get_field_at(0).unwrap().is_nullable());
    ```

=== "Python"

    ```python
    from yggdryl import Field, types

    levels = types.list("levels", types.float64("item"), nullable=False)
    assert isinstance(levels, Field)
    assert levels.name == "levels"
    assert levels.nullable is False
    assert levels.dtype[0].name == "item"
    assert levels.dtype[0].nullable is True

    # The item is a whole field, so it carries its own name and nullability.
    required = types.list("levels", types.float64("item", nullable=False))
    assert required.dtype[0].nullable is False

    # Metadata rides beside the datatype, and a child keeps its own.
    annotated = types.list("levels", types.float64("item"), metadata={"owner": "book"})
    assert annotated.metadata["owner"] == "book"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const levels = fields.list('levels', fields.float64('item'), { nullable: false })
    assert.ok(levels instanceof Field)
    assert.equal(levels.name, 'levels')
    assert.equal(levels.nullable, false)
    assert.equal(levels.dtype.getFieldAt(0).name, 'item')
    assert.equal(levels.dtype.getFieldAt(0).nullable, true)

    // The item is a whole field, so it carries its own name and nullability.
    const required = fields.list('levels', fields.float64('item', { nullable: false }))
    assert.equal(required.dtype.getFieldAt(0).nullable, false)

    // Metadata rides beside the datatype, and a child keeps its own.
    const annotated = fields.list('levels', fields.float64('item'), {
      metadata: { owner: 'book' },
    })
    assert.equal(annotated.get('owner'), 'book')
    ```

## Scalar

`Scalar::Sequence(Sequence)` is the run: the values in order, in one shared
slice. The leaf is the column's decision and never the value's, so a cell read
out of any of the five layouts is the same sequence, and its datatype is the
`list` of its items.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FamilyValue, Nested, NestedValue, Scalar, Sequence};

    let levels = DataType::list(DataType::Float64.nullable_field("item"));
    let value = levels.scalar(vec![Scalar::from(1.5_f64), Scalar::Null])?;
    assert_eq!(value.len(), 2);
    assert_eq!(value.dtype()?, levels);

    // `Sequence` is the holder every sequence API answers with.
    let held = Sequence::new(vec![Scalar::from(1_i64), Scalar::from(2_i64)]);
    assert_eq!(held.len(), 2);
    assert_eq!(held.as_slice()[1], Scalar::from(2_i64));
    assert_eq!(held.children().count(), 2);
    assert_eq!(Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]), Scalar::Sequence(held.clone()));
    assert!(matches!(Scalar::Sequence(held).as_nested(), Some(Nested::Sequence(_))));

    // A run of the wrong item datatype is refused, with the path that failed.
    let refused = levels.scalar(vec![Scalar::from("text")]).unwrap_err().to_string();
    assert!(refused.contains("float64"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import types

    levels = types.list("levels", types.float64("item"))
    value = levels.scalar([1.5, 2.5, None])

    assert value.as_py() == [1.5, 2.5, None]
    assert value.family == "nested"
    assert len(value) == 3
    assert [child.as_py() for child in value] == [1.5, 2.5, None]

    # A run of the wrong item datatype is refused, with the path that failed.
    with pytest.raises(ValueError, match="float64"):
        levels.scalar(["text"])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const levels = fields.list('levels', fields.float64('item'))
    const value = levels.scalar([1.5, 2.5, null])

    assert.deepEqual(value.asJs(), [1.5, 2.5, null])
    assert.equal(value.family, 'nested')
    assert.equal(value.length, 3)

    // A run of the wrong item datatype is refused, with the path that failed.
    assert.throws(() => levels.scalar(['text']), /float64/)
    ```

## Arrow storage

Arrow already says the layout, so the storage *is* the leaf: all five cross
bare, item field and all, with no extension document. `FixedSizeList` carries
its length in the storage, which is why the length is validated where the
datatype is built rather than where a batch is written.

| datatype | Arrow storage | extension name |
| --- | --- | --- |
| `list(item)` | `List(item)` | none |
| `large_list(item)` | `LargeList(item)` | none |
| `list_view(item)` | `ListView(item)` | none |
| `large_list_view(item)` | `LargeListView(item)` | none |
| `fixed_size_list(item,n)` | `FixedSizeList(item, n)` | none |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
    use yggdryl::{DataType, Field};

    let levels = DataType::list(DataType::Float64.nullable_field("item"));
    let item = Arc::new(ArrowField::new("item", ArrowDataType::Float64, true));
    assert_eq!(levels.clone().into_arrow_datatype()?, ArrowDataType::List(Arc::clone(&item)));
    assert_eq!(DataType::from_arrow_datatype(&ArrowDataType::List(item))?, levels);

    // The length is part of the storage, so it crosses with it.
    let fixed = DataType::fixed_size_list(DataType::Int32.nullable_field("item"), 3)?;
    assert!(matches!(fixed.clone().into_arrow_datatype()?, ArrowDataType::FixedSizeList(_, 3)));
    assert_eq!(DataType::from_arrow_datatype(&fixed.clone().into_arrow_datatype()?)?, fixed);

    // Every leaf is its own Arrow storage, so none of them collapses.
    for leaf in [
        DataType::large_list(DataType::Int32.nullable_field("item")),
        DataType::list_view(DataType::Int32.nullable_field("item")),
        DataType::large_list_view(DataType::Int32.nullable_field("item")),
    ] {
        assert_eq!(DataType::from_arrow_datatype(&leaf.clone().into_arrow_datatype()?)?, leaf);
    }
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field, types

    levels = types.list("levels", types.float64("item"))
    arrow = levels.into_arrow()
    assert arrow.type == pa.list_(pa.field("item", pa.float64(), nullable=True))
    assert arrow.metadata is None
    assert Field.from_arrow(arrow) == levels

    # The length is part of the storage, so it crosses with it.
    fixed = types.fixed_size_list("items", types.int32("item"), 3)
    assert fixed.into_arrow().type == pa.list_(pa.field("item", pa.int32()), 3)
    assert DataType.from_arrow(pa.large_list(pa.int32())).id == "large_list"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    // A cast through a struct root answers the Arrow field a column is written as.
    const row = fields.struct('row', [fields.list('levels', fields.float64('item'))], {
      nullable: false,
    })
    const table = row.castArrow(
      new arrow.Table({
        levels: arrow.vectorFromArray([[1.5, 2.5]], new arrow.List(
          new arrow.Field('item', new arrow.Float64(), true),
        )),
      }),
    )

    const projected = table.schema.fields[0]
    assert.equal(projected.name, 'levels')
    assert.equal(projected.type.children[0].name, 'item')
    assert.equal(projected.metadata.get('ARROW:extension:name'), undefined)
    ```

## A list is transparent to a path

The five layouts hold exactly one child, so a dotted path treats the item as a
step it need not spell: `orders.price` reaches the price of an
`array<struct>` item the way `orders.item.price` does, and the item's own name
still wins outright. A write is not transparent - it addresses the item by its
own name - which is what keeps a list from ever growing a second child.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let orders = DataType::from_str("struct<orders:array<struct<price:double>>>")?;

    // Both spellings reach the same child, and so does an item selector.
    assert_eq!(orders.get_field_by_path("orders.price").map(Field::name), Some("price"));
    assert_eq!(orders.get_field_by_path("orders.item.price").map(Field::name), Some("price"));
    assert_eq!(orders.get_field_by_path("orders[0].price").map(Field::name), Some("price"));
    assert_eq!(orders.get_field_by_path("orders.item").map(Field::name), Some("item"));
    assert!(orders.get_field_by_path("orders.quantity").is_none());

    // A write addresses the item by name, and a list refuses a second child.
    let mut list = DataType::list(DataType::Int32.nullable_field("item"));
    list.set_field_at(0, DataType::Int64.nullable_field("item"))?;
    assert_eq!(list, DataType::list(DataType::Int64.nullable_field("item")));
    let refused = list
        .set_field_by_path("extra", DataType::utf8().nullable_field("extra"))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("a struct field"), "{refused}");
    assert!(list.remove_field_at(0).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    orders = DataType("struct<orders:array<struct<price:double>>>")

    # Both spellings reach the same child, and so does an item selector.
    assert orders.get_field_by_path("orders.price").name == "price"
    assert orders.get_field_by_path("orders.item.price").name == "price"
    assert orders.get_field_by_path("orders[0].price").name == "price"
    assert orders.get_field_by_path("orders.item").name == "item"
    assert orders.get_field_by_path("orders.quantity") is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const orders = DataType.from('struct<orders:array<struct<price:double>>>')

    // Both spellings reach the same child, and so does an item selector.
    assert.equal(orders.getFieldByPath('orders.price').name, 'price')
    assert.equal(orders.getFieldByPath('orders.item.price').name, 'price')
    assert.equal(orders.getFieldByPath('orders[-1].price').name, 'price')
    assert.equal(orders.getFieldByPath('orders.item').name, 'item')
    assert.equal(orders.getFieldByPath('orders.quantity'), null)
    ```

## Edges

- A negative fixed length -> refused at `fixed_size_list`, again at `validate`, and again at the Arrow projection, with the same message each time.
- A list holds exactly one child: `set_field_at(0, ..)` replaces it, an unknown name is refused rather than appended, and `remove_field_at(0)` is refused rather than leaving a list with none.
- `unnest_fields` treats a list as one leaf, because a list is one column; `explode_fields` is what reaches inside it and answers the item's datatype, nullable when the column or its item is.
- The item's name is the item's: a leaf built by a parser is named `item`, and a leaf built by hand keeps whatever it was given.
- A value carries no layout: a cell read out of `fixed_size_list(item,3)`, `large_list` or a view is a `Sequence`, and its datatype is the `list` of its items.
- A merge reaches the item: two lists of the same leaf meet at the list of the item that holds both, and two different leaves do not meet - see [Field](../field.md#merging-two-schemas).
- `list_view` and `large_list_view` have no default Arrow JS materialization, so `defaultArrowScalar` refuses them in JavaScript while every other leaf answers.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::nested datatype::arrow field::nested
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(nested_datatype_clone|nested_validate)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_factories.py python/tests/types/test_defaults.py -k "list or nested"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="list|nested" node/tests/types/fields.test.js node/tests/types/defaults.test.js
    npm run --prefix node bench:types
    ```
