# Field

`Field` is the one schema value: a name, a datatype, a nullability flag, and metadata.

## Contract

| | |
| --- | --- |
| Owns | name, datatype, `nullable`, metadata; no separate schema type |
| Schema root | a struct `Field` with `nullable` false; `validate_struct_root` checks it |
| Validates | `Field::new` nothing; `Field::from_parts` and the bindings' constructor everything |
| Datatype argument | bindings take text, a native `DataType`, or a PyArrow type (Python) |
| `nullable` default | Python `True`, JavaScript `false` |
| Metadata | string keys and values, lexical order; clones share one map until a write |
| Identity | equality, ordering, hashing include metadata and dictionary state |
| Serialization | one `Field` ⇄ `Scalar` mapping under JSON, YAML, TOML |
| Rendering | `Display` compact, round-trips; `{:#}` / `pretty()` readable |
| JavaScript | subscripts, YAML/TOML, `pretty` are Rust first; `toJSON` exists |

## Use

Build one, read its four parts, and round-trip the compact text.
=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let field = Field::new("price", DataType::from_str("decimal(18, 6)")?, false);

    assert_eq!(field.name(), "price");
    assert_eq!(field.dtype(), &DataType::decimal(18, 6)?);
    assert!(!field.is_nullable());
    assert!(field.is_metadata_empty());

    // The canonical text round-trips, and shorthand parses into the same value.
    assert_eq!(Field::from_str(&field.to_string())?, field);
    assert_eq!(Field::from_str("price decimal(18, 6) NOT NULL")?, field);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    field = Field("price", "decimal(18, 6)", nullable=False)

    assert field.name == "price"
    assert field.dtype == DataType("decimal(18, 6)")
    assert field.nullable is False
    assert len(field) == 0

    assert Field.from_str(str(field)) == field
    assert Field.from_str("price decimal(18, 6) NOT NULL") == field
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const field = new Field('price', 'decimal(18, 6)', false)

    assert.equal(field.name, 'price')
    assert.ok(field.dtype.equals(DataType.from('decimal(18, 6)')))
    assert.equal(field.nullable, false)
    assert.equal(field.size, 0)

    assert.ok(Field.from(field.toString()).equals(field))
    assert.ok(Field.from('price decimal(18, 6) NOT NULL').equals(field))
    ```
## A non-null struct field is the schema

A table's columns are the children of a struct field with `nullable` false, the root every [media](../media/index.md) reader takes. `require_struct` still accepts a nullable struct column.
=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("trade");

    schema.validate_struct_root()?;
    assert_eq!(schema.field_len(), 2);
    assert_eq!(schema.index_of("symbol"), Some(1));
    assert_eq!(schema.get_field_by_path("id").map(Field::name), Some("id"));

    // A nullable root is not a schema: a whole row cannot be logically absent.
    assert!(schema.with_nullable(true).validate_struct_root().is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, types

    schema = Field(
        "trade",
        DataType.from_fields([
            types.int64("id", nullable=False),
            types.utf8("symbol"),
        ]),
        nullable=False,
    )

    children = schema.dtype
    assert len(children) == 2
    assert "symbol" in children
    assert children["id"].nullable is False
    assert children[1].name == "symbol"
    assert [child.name for child in children] == ["id", "symbol"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, fields } = require('yggdryl')

    const schema = new Field(
      'trade',
      DataType.fromFields([
        fields.int64('id', { nullable: false }),
        fields.utf8('symbol', { nullable: true }),
      ]),
      false,
    )

    const children = schema.dtype
    assert.equal(children.length, 2)
    assert.equal(children.getFieldAt(0).nullable, false)
    assert.equal(children.getFieldByPath('symbol').name, 'symbol')
    assert.deepEqual(children.keys(), ['id', 'symbol'])
    ```
Each lookup exists by position, by path, or either:

| | position | path | either |
|---|---|---|---|
| raising | `field_at` | `field_by_path` | `field` |
| optional | `get_field_at` | `get_field_by_path` | `get_field` |
| replacing | `set_field_at` | `set_field_by_path` | `set_field` |
| removing | `remove_field_at` | `remove_field_by_path` | `remove_field` |
`DataType` answers the same calls, plus `fields`, `field_len`, `index_of`, and `named_field`. The [`field:`](protocol.md) view is `as_field_properties`, `field_properties`, or `fieldProperties`.

## Flattening and expanding

`unnest_fields` flattens struct nesting to dotted leaf paths and keeps a list or map as one leaf column. `explode_fields` swaps each collection child for what it holds, keeping name and order.

Rust only.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    let row = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([DataType::Float64.required_field("px")])?
            .nullable_field("line"),
        DataType::list(DataType::Float64.nullable_field("item")).nullable_field("levels"),
    ])?;

    // Structs flatten to leaves; the list stays one column.
    let leaves = row.unnest_fields();
    let names: Vec<&str> = leaves.iter().map(|field| field.name()).collect();
    assert_eq!(names, ["id", "line.px", "levels"]);

    // The nullable parent makes its leaf nullable, and the name resolves.
    assert!(leaves[1].is_nullable());
    assert!(row.get_field_by_path("line.px").is_some());

    // Exploding reaches inside the collection, keeping the column's name.
    let exploded = row.explode_fields();
    assert_eq!(exploded[2].name(), "levels");
    assert_eq!(exploded[2].dtype(), &DataType::Float64);
    ```
## Merging two schemas

`merge_with` is the crate's only promotion table, shared by expression typing and value inference. Rules, in order:

1. equal types are that type;
2. `null` yields to the defined side;
3. same-family nesting recurses; a struct takes the union of its fields;
4. bytes win;
5. text wins next; ASCII widths meet at the wider width, or the narrower when narrowing. A width beside variable text meets at the variable text, or the width when narrowing;
6. numbers meet by width, temporals by unit.

Anything left is refused. `Field.merge_with` keeps the receiver's name, is nullable when either side is, keeps dictionary options where both encode, and unions metadata, receiver winning.

Rust only.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let left = DataType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::Utf8.required_field("venue"),
    ])?;
    let right = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])?;

    let merged = left.merge_with(&right, true)?;

    // The shared column widens; the two unshared ones arrive nullable.
    assert_eq!(merged["id"].dtype(), &DataType::Int64);
    assert!(merged["venue"].is_nullable());
    assert!(merged["price"].is_nullable());

    // Narrowing meets at the tightest type naming both.
    assert_eq!(
        DataType::Int32.merge_with(&DataType::Int64, false)?,
        DataType::Int32,
    );

    // Bytes win over text, and text over numbers.
    assert_eq!(DataType::Utf8.merge_with(&DataType::Binary, true)?, DataType::Binary);
    assert_eq!(DataType::Int64.merge_with(&DataType::Utf8, true)?, DataType::Utf8);

    // A field merge carries nullability and metadata across.
    let a = Field::new("price", DataType::Int32, false);
    let b = Field::new("price", DataType::Int64, true);
    let field = a.merge_with(&b, true)?;
    assert_eq!(field.dtype(), &DataType::Int64);
    assert!(field.is_nullable());
    ```
## Item access reaches a child, never metadata

Subscripting a `Field` or a `DataType` reaches a child: a `str` is a name, an `int` a position, and `len`, iteration, and membership speak children. Metadata is reached through [its own view](#metadata-is-a-mapping).

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let mut order = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([DataType::Float64.required_field("price")])?
            .required_field("line"),
    ])?
    .required_field("order");
    order.insert_metadata("owner", "trading")?;

    // A child by name, by position, and two levels down.
    assert_eq!(order["id"].dtype(), &DataType::Int64);
    assert_eq!(order[1].name(), "line");
    assert_eq!(order["line"]["price"].dtype(), &DataType::Float64);

    // An unknown name appends; a position replaces.
    order.set_field_by_path("venue", DataType::Utf8.nullable_field("venue"))?;
    assert_eq!(order.field_len(), 3);
    order.set_field(0, DataType::Utf8.required_field("id"))?;
    assert_eq!(order["id"].dtype(), &DataType::Utf8);
    assert_eq!(order.remove_field_by_path("venue")?.name(), "venue");

    // Metadata keeps its own named surface.
    assert_eq!(order.get_metadata("owner"), Some("trading"));
    assert!(order.get_field_by_path("owner").is_none());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    order = Field(
        "order",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field(
                "line",
                DataType.from_fields([Field("price", "float64", nullable=False)]),
                nullable=False,
            ),
        ]),
        nullable=False,
        metadata={"owner": "trading"},
    )

    # A child by name, by position, negatively, and two levels down.
    assert order["id"].dtype == DataType("int64")
    assert order[-1].name == "line"
    assert order["line"]["price"].dtype == DataType("float64")

    # The DataType answers the same way, and children drive len/iter/in.
    assert order.dtype["id"].name == "id"
    assert len(order) == 2
    assert [child.name for child in order] == ["id", "line"]
    assert "line" in order

    # An unknown name appends; a position replaces only.
    order["venue"] = Field("venue", "utf8")
    assert len(order) == 3
    order[0] = Field("id", "utf8", nullable=False)
    assert order["id"].dtype == DataType("utf8")
    del order["venue"]
    assert len(order) == 2

    # Metadata is reached through its view, never by subscripting the node.
    assert order.metadata["owner"] == "trading"
    try:
        order["owner"]
    except KeyError:
        pass
    ```

=== "JavaScript"

    !!! note "Rust first"
        The JavaScript binding reaches children through `dtype` with `at`, `getByName`, and
        `keys`; the shared subscript vocabulary lands with the rest of the lifecycle surface.
| | path (`str`) | position (`int`) |
| --- | --- | --- |
| read | whole name first, then split at each `.` from the left | negative counts from the end |
| assign | replaces in place; appends when unresolved | replaces only; past the end errors |
| `del` | removes, closing the gap | removes, closing the gap |

## Metadata is a mapping

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let mut field = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;
    field.insert_metadata("currency", "EUR")?;
    field.update_metadata([("source", "exchange")])?;

    assert_eq!(field.metadata_len(), 3);
    assert_eq!(field.get_metadata("venue"), Some("XPAR"));
    assert!(field.has_metadata("currency"));
    assert_eq!(
        field.metadata_iter().collect::<Vec<_>>(),
        [("currency", "EUR"), ("source", "exchange"), ("venue", "XPAR")]
    );
    assert_eq!(field.remove_metadata("venue").as_deref(), Some("XPAR"));
    ```

=== "Python"

    ```python
    from yggdryl import Field

    field = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})
    # Metadata lives on `field.metadata`, a live mapping view. Subscripting the
    # field itself reaches a nested *child*, not a metadata key.
    field.metadata["currency"] = "EUR"
    field.metadata.update(source="exchange")

    assert len(field.metadata) == 3
    assert "venue" in field.metadata
    assert field.metadata["venue"] == "XPAR"
    assert field.metadata.get("missing") is None
    assert list(field.metadata.items()) == [
        ("currency", "EUR"),
        ("source", "exchange"),
        ("venue", "XPAR"),
    ]

    del field.metadata["venue"]
    assert list(field.metadata.keys()) == ["currency", "source"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = new Field('price', 'float64', false, { venue: 'XPAR' })
    field.set('currency', 'EUR')
    field.update(new Map([['source', 'exchange']]))

    assert.equal(field.size, 3)
    assert.equal(field.has('venue'), true)
    assert.equal(field.get('venue'), 'XPAR')
    assert.equal(field.get('missing'), null)
    assert.deepEqual([...field], [
      ['currency', 'EUR'],
      ['source', 'exchange'],
      ['venue', 'XPAR'],
    ])

    assert.equal(field.delete('venue'), true)
    assert.deepEqual(field.keys(), ['currency', 'source'])
    ```
Keys and values are strings in lexical key order, so equal entries compare and hash identically. Every write validates the whole batch first; a bad entry leaves the field as it was.

## Typed field aliases

=== "Rust"

    ```rust
    use yggdryl::types::{Int64Field, DateTime64Field, Utf8Field, integer};
    use yggdryl::{DataType, Field, TimeUnit, Timezone};

    let id = Int64Field::new("id", false);
    let symbol = Utf8Field::from_parts("symbol", true, [("source", "feed")])?;
    let at = DateTime64Field::try_new("at", DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE }, false)?;

    // A typed field derefs to the field it wraps.
    assert_eq!(id.name(), "id");
    assert_eq!(symbol.get_metadata("source"), Some("feed"));
    assert_eq!(at.dtype().to_string(), "datetime64(us)");

    // The marker is checked, never assumed.
    assert!(
        Field::new("id", DataType::Utf8, false)
            .try_into_typed::<integer::Int64Type>()
            .is_err()
    );
    assert_eq!(id.into_field().dtype(), &DataType::Int64);
    ```

=== "Python"

    ```python
    from yggdryl import Field, types

    id_field = types.int64("id", nullable=False)
    symbol = types.utf8("symbol", metadata={"source": "feed"})
    at = types.datetime64("at", "us", nullable=False)

    assert isinstance(id_field, Field)
    assert str(id_field.dtype) == "int64"
    assert symbol.metadata["source"] == "feed"
    assert str(at.dtype) == "datetime64(us)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const id = fields.int64('id')
    const symbol = fields.utf8('symbol', { nullable: true, metadata: { source: 'feed' } })
    const at = fields.datetime64('at', 'us')

    assert.ok(id instanceof Field)
    assert.equal(id.dtype.toString(), 'int64')
    assert.equal(symbol.get('source'), 'feed')
    assert.equal(at.dtype.toString(), 'datetime64(us)')
    ```
`Int64Field` and its fifty-five siblings are `TypedField<K>`: one `Field` plus a zero-sized sealed marker, `repr(transparent)`. The marker constrains the variant only; every parameter stays in the wrapped field.

| alias | constructors |
| --- | --- |
| static datatype (`Int64Field`, `Utf8Field`, `VariantField`, `UuidField`, `Ascii16Field` to `Ascii128Field`, `CountryField`, `CurrencyField`, `MicField`, `CfiField`) | `new(name, nullable)`, infallible; `from_parts(name, nullable, metadata)` |
| parameterized (`DateTime64Field`, `GeometryField`, `GeographyField`) | `try_new(name, dtype, nullable)` |
| from a `Field` | `try_as_typed` borrows; `try_into_typed` consumes |
| bindings | `types.int64` / `fields.int64` return the native `Field`, typed for a checker only; `fields.ascii(name, width)` |

[Geospatial](geospatial.md), [ASCII](ascii.md), and [UUID](uuid.md) aliases follow this pattern; a registered code builds its own datatype, not an ASCII width.

## Converting to one native field

| | typed value | struct root |
| --- | --- | --- |
| Rust | `TypedField<K>::into_field(self)` | `StructField::into_struct_field(self)` |
| Python | `field(value, name=None)` | cached `Class.field() -> StructField`, installed by `@scalar` |
| JavaScript | `intoField(value, name = null)` | static getter `Class.intoStructField`, memoized by `intoField` |

No name, `None`/`null`, or the existing name returns the cached native value; another name returns a renamed clone. The root must be a non-null struct field.

## Serializing a schema

One `Field` ⇄ `Scalar` mapping (`into_value`/`from_value`, `into_dict`/`from_dict`) backs JSON, YAML, and TOML, so a schema embeds inline in any document. Each writer takes the shared [`Formatting`](../text/index.md) option, `indent` in Python.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};
    use yggdryl::types::Scalar;

    let field = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;

    // One structural model, three formats over it.
    assert_eq!(Field::from_value(field.clone().into_value())?, field);
    assert_eq!(Field::from_json(&field.clone().into_json()?)?, field);
    assert_eq!(Field::from_yaml(&field.clone().into_yaml()?)?, field);
    assert_eq!(Field::from_toml(&field.clone().into_toml()?)?, field);

    // The mapping is the shared `Scalar`, so it drops into any document.
    let shape = field.into_value();
    assert_eq!(shape.get_key_str("name").and_then(Scalar::as_utf8), Some("price"));
    // Unset optional attributes are absent rather than null.
    assert!(shape.get_key_str("dictionary_id").is_none());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    field = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})

    assert Field.from_dict(field.into_dict()) == field
    assert Field.from_json(field.into_json()) == field
    assert Field.from_yaml(field.into_yaml()) == field
    assert Field.from_toml(field.into_toml()) == field

    shape = field.into_dict()
    assert shape["name"] == "price"
    assert "dictionary_id" not in shape
    ```

=== "JavaScript"

    !!! note "Rust first"
        The YAML and TOML pair lands in the JavaScript binding once the core surface settles;
        `toJSON` is already there.
## A readable rendering

`Display`, and Python's `str`/`repr`, is the compact form that round-trips through `from_str`. The readable form is the alternate: `{:#}`, the `pretty()` adapter behind it, and Python's `pretty()`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let order = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([DataType::Float64.required_field("price")])?
            .nullable_field("line"),
    ])?
    .required_field("order");

    // Compact still round-trips.
    assert_eq!(Field::from_str(&order.to_string())?, order);

    // Readable is the alternate, or the named adapter - one implementation.
    assert_eq!(format!("{order:#}"), order.pretty().to_string());
    assert_eq!(
        format!("{order:#}"),
        concat!(
            "order: struct[2], required\n",
            "  id: int64, required\n",
            "  line: struct[1], nullable\n",
            "    price: float64, required",
        ),
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    order = Field(
        "order",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field(
                "line",
                DataType.from_fields([Field("price", "float64", nullable=False)]),
            ),
        ]),
        nullable=False,
    )

    # `repr` is unchanged - the eval-round-trip form Python expects.
    assert repr(order).startswith("Field.from_str(")
    assert Field.from_str(str(order)) == order

    assert order.pretty() == (
        "order: struct[2], required\n"
        "  id: int64, required\n"
        "  line: struct[1], nullable\n"
        "    price: float64, required"
    )
    ```

=== "JavaScript"

    !!! note "Rust first"
        `pretty` lands in the JavaScript binding once the core surface settles.
## Comparing two fields

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field};

    let left = Field::from_parts("price", DataType::Float64, false, [("venue", "XPAR")])?;
    let right = Field::from_parts("price", DataType::Float64, true, [("venue", "XNAS")])?;

    assert!(!left.equals(&right, true));
    assert_eq!(
        left.show_diffs(&right, true, false).collect::<Vec<_>>(),
        [
            "≠ $.nullable: false → true",
            "≠ $.metadata[\"venue\"]: \"XPAR\" → \"XNAS\"",
        ]
    );
    assert_eq!(left.show_diff(&left, true, true), "✓ equal");
    assert_eq!(left.show_diff(&left, true, false), "");
    ```

=== "Python"

    ```python
    from yggdryl import Field

    left = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})
    right = Field("price", "float64", metadata={"venue": "XNAS"})

    assert not left.equals(right)
    assert list(left.show_diffs(right)) == [
        "≠ $.nullable: false → true",
        '≠ $.metadata["venue"]: "XPAR" → "XNAS"',
    ]
    assert left.show_diff(left) == "✓ equal"
    assert left.show_diff(left, return_equal=False) == ""
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const left = new Field('price', 'float64', false, { venue: 'XPAR' })
    const right = new Field('price', 'float64', true, { venue: 'XNAS' })

    assert.equal(left.equals(right), false)
    assert.deepEqual([...left.showDiffs(right)], [
      '≠ $.nullable: false → true',
      '≠ $.metadata["venue"]: "XPAR" → "XNAS"',
    ])
    assert.equal(left.showDiff(left), '✓ equal')
    assert.equal(left.showDiff(left, true, false), '')
    ```
`equals` answers yes or no; `show_diffs` answers why as a lazy iterator (`Differences` borrows, `OwnedDifferences` owns). `show_diff` joins the lines; [`DataType`](datatype.md) has the same two calls.

## Edges

- nullable root -> `validate_struct_root` refuses.
- Python `field(x, idx=..., path=...)` naming more than one -> refused.
- `unnest_fields` -> a leaf under a nullable ancestor is nullable; each name resolves through `field_by_path`.
- `explode_fields` -> one level per call; nullable when the collection or its element is.
- both projections -> a list of fields, not a node.
- `merge_with(other, false)` -> the tightest type naming both; `true` widens losslessly.
- merged struct -> a one-sided child becomes nullable; receiver order, additions appended.
- boolean beside datetime, decimal beside float -> refused.
- `order["a.b"]` -> a child literally named `a.b` wins over `a` then `b`.
- list or run-end node -> exactly one or two children; grow and shrink refuse.
- Python `DataType[...] = ...` -> refused; it points at the owning `Field`.
- Python first `hash(field)` -> locks mutation on that wrapper; `copy.copy` unlocks; `stable_hash()` never locks.
- binding metadata and protocol views -> unhashable; Rust's borrowed protocol view is not `Borrow<Field>`.
- typed field -> no `DerefMut`; a failing `set_dtype` leaves the value untouched.
- `field(value, name)` with a non-string name or a non-field value -> `TypeError`.
- emitted shape -> `dictionary_id` only when non-zero, `dictionary_is_ordered` only when set; unset optionals omitted, never null.
- `pretty()` -> only set attributes; metadata as `@key = value` lines; stable across runs.
- `with_metadata=false` -> metadata dropped at every depth.
- `return_equal` -> false for `show_diffs`, true for `show_diff`; only `show_diff` prints `✓ equal`.
- diff paths -> `$`-rooted places such as `$.nullable` and `$.fields[2]`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- field::generic field::nested field::serde field::comparison field::typed field::arrow field::integer field::floating field::decimal field::temporal field::binary field::scalar
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::field types::typed types::diff types::merge
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^parse/field_'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/(nested_field_clone|field_stable_hash|metadata_)'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^typed/'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^comparison/'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^arrow/(field_|struct_field_)'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_field.py python/tests/types/test_factories.py python/tests/types/test_field_classes.py python/tests/types/test_field_classes_arrow.py python/tests/types/test_field_classes_edges.py python/tests/types/test_field_classes_py314.py python/tests/types/test_protocol_hashability.py
    python/.venv/bin/python python/benchmarks/types.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/types/field.test.js node/tests/types/fields.test.js
    npm run --prefix node bench:types
    ```

## Performance

Rust times both consuming typed accessors, construction outside the timer; the bindings hold the cached value and price a renamed clone separately. One local Windows x86_64 release run: Criterion point estimates, Python median per call, JavaScript whole-loop rate.

| runtime operation | estimate |
| --- | ---: |
| Rust `TypedField::into_field` | 41.5 ns |
| Rust `StructField::into_struct_field` | 34.7 ns |
| Python cached `Class.field()` | 677 ns |
| Python global `field(Class)` | 1.27 us |
| Python renamed `field(Class, name=...)` | 9.26 us |
| JavaScript `intoField(nativeField)` | 40.0 ns (25.0M calls/s) |
| JavaScript cached `intoField(Class)` | 72.0 ns (13.9M calls/s) |
| JavaScript renamed `intoField(Class, name)` | 33.6 us (29.7k calls/s) |
```bash
cargo bench --manifest-path rust/Cargo.toml --bench types -- '^typed/'
python/.venv/bin/python python/benchmarks/types.py --iterations 10000
npm run --prefix node bench:types
```
