# Field

A `Field` names a [`DataType`](datatype.md), marks it nullable, attaches metadata,
and can sit in a graph (an optional parent + child accessors). A field whose type is
a struct **is a schema**. Fields are what make the type system recursive: a list
item, struct member or map entry is itself a `Field`.

## Construct & inspect

=== "Python"

    ```python
    import yggdryl

    f = yggdryl.Field("id", yggdryl.DataType.int(64), nullable=False)
    assert f.name == "id"
    assert not f.nullable
    assert f.data_type == yggdryl.DataType.int(64)
    assert str(f) == "id: int64 not null"
    assert yggdryl.Field.from_str("id: int64 not null").name == "id"
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const f = new yggdryl.Field("id", yggdryl.DataType.int(64), false);
    f.name;        // "id"
    f.nullable;    // false
    f.toString();  // "id: int64 not null"
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};

    let f = Field::new("id", DataType::int(64, true), false);
    assert_eq!(f.name(), "id");
    assert_eq!(f.to_str(), "id: int64 not null");
    ```

!!! tip "Field string forms"
    `from_str` accepts a `:` or a space between the name and type, an optional
    trailing `not null`, and a name wrapped in `"…"`, `'…'`, `` `…` `` or `[…]` —
    so SQL/Hive DDL works: `qty: int64 not null`, `col struct<a: str>`,
    `"my col" varchar(255)`.

## Metadata

Arbitrary string metadata, with `comment` as a named convenience getter/setter.

=== "Python"

    ```python
    import yggdryl

    f = yggdryl.Field("id", yggdryl.DataType.int(64)).with_comment("primary key")
    assert f.comment == "primary key"
    f.set_metadata("unit", "count")
    assert f.get_metadata("unit") == "count"
    assert f.metadata()["unit"] == "count"
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const f = new yggdryl.Field("id", yggdryl.DataType.int(64)).withComment("primary key");
    f.comment;                 // "primary key"
    f.setMetadata("unit", "count");
    f.getMetadata("unit");     // "count"
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};

    let f = Field::new("id", DataType::int(64, true), true).with_comment("primary key");
    assert_eq!(f.comment(), Some("primary key"));
    ```

## Schema graph

A struct-typed field exposes its members through case-insensitive / index child
accessors, and `with_linked_children` wires `parent` pointers for upward traversal.
The `parent` is navigational only — it is excluded from equality, hashing and
serialization, so it never breaks `Hash` / pickle and a linked schema compares equal
to the unlinked one.

=== "Python"

    ```python
    import yggdryl

    schema = yggdryl.Field("rec", yggdryl.DataType.struct_([
        yggdryl.Field("Id", yggdryl.DataType.int(64), nullable=False),
        yggdryl.Field("addr", yggdryl.DataType.struct_([
            yggdryl.Field("City", yggdryl.DataType.varchar()),
        ])),
    ]), nullable=False)

    assert schema.child("id").name == "Id"        # case-insensitive
    assert schema.child_index("addr") == 1
    linked = schema.with_linked_children()
    assert linked.child("addr").child("city").root().name == "rec"
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const schema = new yggdryl.Field("rec", yggdryl.DataType.struct([
      new yggdryl.Field("Id", yggdryl.DataType.int(64), false),
      new yggdryl.Field("addr", yggdryl.DataType.struct([
        new yggdryl.Field("City", yggdryl.DataType.varchar()),
      ])),
    ]), false);

    schema.child("id").name;                 // "Id" (case-insensitive)
    const linked = schema.withLinkedChildren();
    linked.child("addr").child("city").root().name; // "rec"
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};

    let schema = Field::new("rec", DataType::struct_(vec![
        Field::new("id", DataType::int(64, true), false),
    ]), false);
    assert_eq!(schema.child("ID").unwrap().name(), "id"); // case-insensitive
    ```

## Merge

`merge` unifies two same-named fields under a strategy: the types merge
([`DataType::merge`](datatype.md#coercion-merge)), the result is nullable if either
side is, and metadata is unioned.

=== "Python"

    ```python
    import yggdryl

    a = yggdryl.Field("x", yggdryl.DataType.int(8), nullable=False)
    b = yggdryl.Field("x", yggdryl.DataType.int(32))
    merged = a.merge(b, "promote")
    assert merged.data_type == yggdryl.DataType.int(32)
    assert merged.nullable
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const a = new yggdryl.Field("x", yggdryl.DataType.int(8), false);
    const b = new yggdryl.Field("x", yggdryl.DataType.int(32));
    const merged = a.merge(b, "promote");
    merged.dataType.equals(yggdryl.DataType.int(32)); // true
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field, MergeStrategy};

    let a = Field::new("x", DataType::int(8, true), false);
    let b = Field::new("x", DataType::int(32, true), true);
    let merged = a.merge(&b, MergeStrategy::Promote)?;
    assert_eq!(merged.data_type(), &DataType::int(32, true));
    ```

## Schema ↔ Arrow

In Rust (with the `arrow` feature), a struct-typed field converts to an
`arrow_schema::Schema` and back.

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};

    let schema = Field::new("rec", DataType::struct_(vec![
        Field::new("id", DataType::int(64, true), false),
    ]), false);
    let arrow = schema.to_arrow_schema()?;           // arrow_schema::Schema
    let back = Field::from_arrow_schema("rec", &arrow, false);
    ```

## Next

- [DataType](datatype.md) — the type a field names
- Back to [Getting started](../getting-started.md)
