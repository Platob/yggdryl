# Serie layouts

Many of one thing, in all five layouts Arrow gives it: one item field, and a leaf that says how the rows are laid out.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `DataType::Serie`, `SerieView`, `LargeSerie`, `LargeSerieView`, `FixedSizeSerie` - its five leaves, each a `DataType` variant of its own - `SerieType` the typed field's payload over them, the `SerieField` marker, and the `Run` value: the schema-free run a row canonicalizes to, one leaf of [`Serie`](../serie.md), held by whichever of `Scalar::Serie`, `SerieView`, `LargeSerie`, `LargeSerieView`, `FixedSizeSerie` matches the leaf |
| Validates | At construction: a fixed length is non-negative, and the item field validates |
| Lazy | Nothing - a leaf holds one shared item field and, on the fixed leaf, one `i32` |
| Cached | The item field behind one `Arc<Field>`, so a leaf clone shares it; the Arrow projection on the [`Field`](../field.md) |
| Refuses | A negative fixed length, a second child, and a value that is not a sequence of the item's datatype |
| Kinds | `DataTypeKind::Nested`, ids `0x91`-`0x95`; `is_nested()` is `true` |
| Bindings | `SerieType`, `Run` and `Serie` are Rust only: Python and JavaScript build a leaf through its own factory and read a stored run back as a [`Scalar`](../scalar.md) |

## DataType

The five leaves differ in how the rows are laid out, never in what a row holds:
one item field, the same for all of them. That is why the item is the whole of
what most readers need, and why a dotted path treats it as a step it need not
spell.

| leaf | offsets | length | Arrow storage | also parsed as |
| --- | --- | --- | --- | --- |
| `serie` (`0x91`) | 32-bit | variable | `List` | `array<T>`, `list<T>` |
| `large_serie` (`0x92`) | 64-bit | variable | `LargeList` | `largearray<T>`, `large_list<T>` |
| `serie_view` (`0x93`) | 32-bit, viewed | variable | `ListView` | `arrayview<T>`, `list_view<T>` |
| `large_serie_view` (`0x94`) | 64-bit, viewed | variable | `LargeListView` | `largearrayview<T>`, `large_list_view<T>` |
| `fixed_size_serie(n)` (`0x95`) | none | exactly `n` | `FixedSizeList(n)` | `fixedarray<T,n>`, `fixed_size_list<T,n>` |

The `list` spellings in the last column are the names the family had before it
took its own: `list`, `list_view`, `fixed_size_list`, `large_list` and
`large_list_view` are `DataTypeId::LEGACY_NAMES`, still read by every door that
reads a datatype's name - `DataTypeId::from_str`, the type grammar (folded, so
`largelist` and `LARGE-LIST` too), the `DataType` and `Field` serde tags, the
`Scalar` wire tags and both bindings - so a schema, a document or a pickle
written before the rename still reads. Nothing writes them: `as_str`,
`Display`, the pretty form, `Scalar::kind()` and every tag spell the `serie`
names. The Arrow storage column is Arrow's own vocabulary and keeps Arrow's
names.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind, Field, SerieType};

    let levels = DataType::serie(DataType::Float64.nullable_field("item"));
    assert_eq!(levels.to_string(), r#"serie(field("item",float64,nullable=true,metadata={}))"#);
    assert_eq!(levels.id(), DataTypeId::Serie);
    assert_eq!(levels.kind(), DataTypeKind::Nested);
    assert!(levels.is_nested());
    assert_eq!(levels.field_len(), 1);
    assert_eq!(levels.get_field_at(0).map(Field::name), Some("item"));

    // The payload reads back the leaf: its item, its length, its layout.
    let fixed = DataType::fixed_size_serie(DataType::Int32.nullable_field("item"), 3)?;
    let leaf = fixed.as_serie_type().expect("a sequence");
    assert_eq!(leaf.fixed_length(), Some(3));
    assert_eq!(leaf.item().name(), "item");
    assert!(!leaf.is_view() && !leaf.is_large());

    let viewed = DataType::large_serie_view(DataType::Int32.nullable_field("item"));
    let viewed_leaf = viewed.as_serie_type().expect("a sequence");
    assert!(viewed_leaf.is_view() && viewed_leaf.is_large());
    assert_eq!(viewed_leaf.fixed_length(), None);
    assert_eq!(
        DataType::from(viewed_leaf.with_item(DataType::Int64.nullable_field("item"))),
        DataType::large_serie_view(DataType::Int64.nullable_field("item"))
    );

    assert_eq!(DataType::from_str("array<int64>")?, DataType::serie(DataType::Int64.nullable_field("item")));
    assert!(DataType::fixed_size_serie(DataType::Int32.nullable_field("item"), -1).is_err());
    assert_eq!(DataType::Int64.as_serie_type(), None);

    // The spellings the family had before it took its own still read, and
    // nothing writes them back.
    assert_eq!(DataTypeId::from_str("fixed_size_list")?, DataTypeId::FixedSizeSerie);
    assert_eq!(DataTypeId::from_legacy_name("LARGE_LIST"), Some(DataTypeId::LargeSerie));
    let legacy = DataType::from_str("list<int64>")?;
    assert_eq!(legacy, DataType::serie(DataType::Int64.nullable_field("item")));
    assert_eq!(legacy.to_string(), r#"serie(field("item",int64,nullable=true,metadata={}))"#);
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    levels = yggdryl.serie("levels", yggdryl.float64("item")).dtype
    assert levels.id == "serie"
    assert levels.kind == "nested"
    assert levels.is_nested
    assert len(levels) == 1
    assert levels[0].name == "item"

    # One factory per leaf, each taking the item field it repeats.
    item = yggdryl.int32("item")
    assert yggdryl.serie_view("value", item).dtype.id == "serie_view"
    assert yggdryl.large_serie("value", item).dtype.id == "large_serie"
    assert yggdryl.large_serie_view("value", item).dtype.id == "large_serie_view"
    assert yggdryl.fixed_size_serie("value", item, 3).dtype.id == "fixed_size_serie"
    assert str(yggdryl.fixed_size_serie("value", item, 3).dtype).endswith(",3)")

    # Every dialect's spelling parses into the same leaf.
    assert DataType("array<int64>") == DataType("serie<int64>")
    assert DataType("fixedarray<int32,3>") == DataType("fixed_size_serie(int32,3)")
    assert DataType("largearrayview<int64>").id == "large_serie_view"

    # The spellings the family had before it took its own still read, and
    # nothing writes them back.
    assert DataType("list<int64>") == DataType("serie<int64>")
    assert DataType("fixed_size_list(int32,3)").id == "fixed_size_serie"
    assert str(DataType("list<int64>")).startswith("serie(")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const levels = fields.serie('levels', fields.float64('item')).dtype
    assert.equal(levels.id, 'serie')
    assert.equal(levels.kind, 'nested')
    assert.equal(levels.nested, true)
    assert.equal(levels.length, 1)
    assert.equal(levels.getFieldAt(0).name, 'item')

    // One factory per leaf, each taking the item field it repeats.
    const item = fields.int32('item')
    assert.equal(fields.serieView('value', item).dtype.id, 'serie_view')
    assert.equal(fields.largeSerie('value', item).dtype.id, 'large_serie')
    assert.equal(fields.largeSerieView('value', item).dtype.id, 'large_serie_view')
    assert.equal(fields.fixedSizeSerie('value', item, 3).dtype.id, 'fixed_size_serie')
    assert.ok(fields.fixedSizeSerie('value', item, 3).dtype.toString().endsWith(',3)'))

    // Every dialect's spelling parses into the same leaf.
    assert.ok(DataType.from('array<int64>').equals(DataType.from('serie<int64>')))
    assert.equal(DataType.from('largearrayview<int64>').id, 'large_serie_view')

    // The spellings the family had before it took its own still read, and
    // nothing writes them back.
    assert.ok(DataType.from('list<int64>').equals(DataType.from('serie<int64>')))
    assert.equal(DataType.from('fixed_size_list(int32,3)').id, 'fixed_size_serie')
    assert.ok(DataType.from('list<int64>').toString().startsWith('serie('))
    ```

## Field

`SerieField` is the typed marker: one field carrying `SerieType`, so the
leaf and its item are read off the payload rather than matched out of a root
datatype. The bindings have one factory per leaf, each taking the item field it
repeats; the column is nullable unless the call says otherwise, and the item's
own nullability is the item field's.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, DataTypeId, Field, SerieField, SerieType};

    let levels = SerieField::new(
        "levels",
        SerieType::Serie(std::sync::Arc::new(DataType::Float64.nullable_field("item"))),
        true,
    );
    assert_eq!(levels.name(), "levels");
    assert_eq!(levels.id(), DataTypeId::Serie);
    assert_eq!(levels.typed_dtype_ref().item().name(), "item");
    assert!(levels.is_nullable());

    // Widened, it is the same column the root constructor builds.
    let root: Field = levels.into_field();
    assert_eq!(root, Field::new("levels", DataType::serie(DataType::Float64.nullable_field("item")), true));
    assert!(SerieField::from_field(&root).is_some());

    // A datatype from another family is refused by name.
    let refused = SerieField::try_new("levels", DataType::utf8(), true).unwrap_err().to_string();
    assert!(refused.contains("sequence"), "{refused}");

    // The item is a whole field, so it carries its own name and nullability.
    let required = DataType::serie(DataType::Float64.required_field("item"));
    assert!(!required.get_field_at(0).unwrap().is_nullable());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    levels = yggdryl.serie("levels", yggdryl.float64("item"), nullable=False)
    assert isinstance(levels, Field)
    assert levels.name == "levels"
    assert levels.nullable is False
    assert levels.dtype[0].name == "item"
    assert levels.dtype[0].nullable is True

    # The item is a whole field, so it carries its own name and nullability.
    required = yggdryl.serie("levels", yggdryl.float64("item", nullable=False))
    assert required.dtype[0].nullable is False

    # Metadata rides beside the datatype, and a child keeps its own.
    annotated = yggdryl.serie("levels", yggdryl.float64("item"), metadata={"owner": "book"})
    assert annotated.metadata["owner"] == "book"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const levels = fields.serie('levels', fields.float64('item'), { nullable: false })
    assert.ok(levels instanceof Field)
    assert.equal(levels.name, 'levels')
    assert.equal(levels.nullable, false)
    assert.equal(levels.dtype.getFieldAt(0).name, 'item')
    assert.equal(levels.dtype.getFieldAt(0).nullable, true)

    // The item is a whole field, so it carries its own name and nullability.
    const required = fields.serie('levels', fields.float64('item', { nullable: false }))
    assert.equal(required.dtype.getFieldAt(0).nullable, false)

    // Metadata rides beside the datatype, and a child keeps its own.
    const annotated = fields.serie('levels', fields.float64('item'), {
      metadata: { owner: 'book' },
    })
    assert.equal(annotated.get('owner'), 'book')
    ```

## Scalar

`Scalar::Serie(Serie)` - or `SerieView`, `LargeSerie`, `LargeSerieView`,
`FixedSizeSerie`, one per leaf - holds many values, and `Serie::Run` is the
run: the values in order, in one shared slice, declaring no field. The leaf is
the column's decision and never the value's, so a cell read out of any of the
five layouts is the same run, and its datatype is the `serie` of its items. The
column itself - the Arrow buffers under the item field, read and written in
place - is the other leaf of [`Serie`](../serie.md).

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind, NestedValue, Run, Scalar, Serie};

    let levels = DataType::serie(DataType::Float64.nullable_field("item"));
    let value = levels.scalar(vec![Scalar::from(1.5_f64), Scalar::Null])?;
    assert_eq!(value.len(), 2);
    assert_eq!(value.dtype()?, levels);

    // `Run` is the schema-free leaf of `Serie`, and what a row canonicalizes to.
    let held = Run::new(vec![Scalar::from(1_i64), Scalar::from(2_i64)]);
    assert_eq!(held.len(), 2);
    assert_eq!(held.as_slice()[1], Scalar::from(2_i64));
    assert_eq!(held.children().count(), 2);
    assert_eq!(
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]),
        Scalar::Serie(Serie::Run(held.clone()))
    );
    // A serie is in the nested family's range of identifiers.
    let many = Scalar::Serie(Serie::from(held));
    assert_eq!(many.kind(), "serie");
    assert_eq!(many.family(), DataTypeKind::Nested);
    assert!(DataTypeKind::Nested.contains(many.id()));

    // A run of the wrong item datatype is refused, with the path that failed.
    let refused = levels.scalar(vec![Scalar::from("text")]).unwrap_err().to_string();
    assert!(refused.contains("float64"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    import yggdryl

    levels = yggdryl.serie("levels", yggdryl.float64("item"))
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

    const levels = fields.serie('levels', fields.float64('item'))
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
| `serie(item)` | `List(item)` | none |
| `large_serie(item)` | `LargeList(item)` | none |
| `serie_view(item)` | `ListView(item)` | none |
| `large_serie_view(item)` | `LargeListView(item)` | none |
| `fixed_size_serie(item,n)` | `FixedSizeList(item, n)` | none |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
    use yggdryl::{DataType, Field};

    let levels = DataType::serie(DataType::Float64.nullable_field("item"));
    let item = Arc::new(ArrowField::new("item", ArrowDataType::Float64, true));
    assert_eq!(levels.clone().into_arrow_datatype()?, ArrowDataType::List(Arc::clone(&item)));
    assert_eq!(DataType::from_arrow_datatype(&ArrowDataType::List(item))?, levels);

    // The length is part of the storage, so it crosses with it.
    let fixed = DataType::fixed_size_serie(DataType::Int32.nullable_field("item"), 3)?;
    assert!(matches!(fixed.clone().into_arrow_datatype()?, ArrowDataType::FixedSizeList(_, 3)));
    assert_eq!(DataType::from_arrow_datatype(&fixed.clone().into_arrow_datatype()?)?, fixed);

    // Every leaf is its own Arrow storage, so none of them collapses.
    for leaf in [
        DataType::large_serie(DataType::Int32.nullable_field("item")),
        DataType::serie_view(DataType::Int32.nullable_field("item")),
        DataType::large_serie_view(DataType::Int32.nullable_field("item")),
    ] {
        assert_eq!(DataType::from_arrow_datatype(&leaf.clone().into_arrow_datatype()?)?, leaf);
    }
    ```

=== "Python"

    ```python
    import pyarrow as pa

    import yggdryl

    from yggdryl import DataType, Field

    levels = yggdryl.serie("levels", yggdryl.float64("item"))
    arrow = levels.into_arrow()
    assert arrow.type == pa.list_(pa.field("item", pa.float64(), nullable=True))
    assert arrow.metadata is None
    assert Field.from_arrow(arrow) == levels

    # The length is part of the storage, so it crosses with it.
    fixed = yggdryl.fixed_size_serie("items", yggdryl.int32("item"), 3)
    assert fixed.into_arrow().type == pa.list_(pa.field("item", pa.int32()), 3)
    assert DataType.from_arrow(pa.large_list(pa.int32())).id == "large_serie"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    // A column crossing out as a table answers the Arrow field it is written as.
    const levels = Serie.fromArrowArray(
      arrow.vectorFromArray([[1.5, 2.5]], new arrow.List(
        new arrow.Field('item', new arrow.Float64(), true),
      )),
      fields.serie('levels', fields.float64('item')),
    )

    const projected = levels.intoArrowBatch().schema.fields[0]
    assert.equal(projected.name, 'levels')
    assert.equal(projected.type.children[0].name, 'item')
    assert.equal(projected.metadata.get('ARROW:extension:name'), undefined)
    ```

## A serie is transparent to a path

The five layouts hold exactly one child, so a dotted path treats the item as a
step it need not spell: `orders.price` reaches the price of an
`array<struct>` item the way `orders.item.price` does, and the item's own name
still wins outright. A write is not transparent - it addresses the item by its
own name - which is what keeps a serie from ever growing a second child.

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

    // A write addresses the item by name, and a serie refuses a second child.
    let mut serie = DataType::serie(DataType::Int32.nullable_field("item"));
    serie.set_field_at(0, DataType::Int64.nullable_field("item"))?;
    assert_eq!(serie, DataType::serie(DataType::Int64.nullable_field("item")));
    let refused = serie
        .set_field_by_path("extra", DataType::utf8().nullable_field("extra"))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("a struct field"), "{refused}");
    assert!(serie.remove_field_at(0).is_err());
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

- A negative fixed length -> refused at `fixed_size_serie`, again at `validate`, and again at the Arrow projection, with the same message each time.
- A serie holds exactly one child: `set_field_at(0, ..)` replaces it, an unknown name is refused rather than appended, and `remove_field_at(0)` is refused rather than leaving a serie with none.
- `unnest_fields` treats a serie as one leaf, because a serie is one column; `explode_fields` is what reaches inside it and answers the item's datatype, nullable when the column or its item is.
- The item's name is the item's: a leaf built by a parser is named `item`, and a leaf built by hand keeps whatever it was given.
- A value carries no layout: a cell read out of `fixed_size_serie(item,3)`, `large_serie` or a view is a `Run`, and its datatype is the `serie` of its items; the column those cells are cut from is a [`Serie`](../serie.md) leaf of that layout.
- A merge reaches the item: two series of the same leaf meet at the serie of the item that holds both, and two different leaves do not meet - see [Field](../field.md#merging-two-schemas).
- An old spelling is read, never written: `list<int64>` parses to `serie<int64>` and prints as `serie(...)`, so a schema read from an old document and written back carries the new name.
- `serie_view` and `large_serie_view` have no Arrow JS materialization, so `Serie#intoArrowScalar`, `intoArrowArray` and `intoArrowBatch` refuse them in JavaScript while every other leaf answers.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test expression -- path::nested
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- datatype::arrow datatype_kind::nested field::generic field::nested mapping::nested merge::nested parser::nested protocol::nested structure::nested union::variants
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(nested_datatype_clone|nested_validate)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py python/tests/test__defaults.py -k "serie or nested"
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="serie|nested" node/tests/fields.test.js node/tests/defaults.test.js
    npm run --prefix node bench:types
    ```
