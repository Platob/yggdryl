# Field

A `Field` is a named [`DataType`](datatype.md) with optional byte-keyed metadata. It
is the recursive node of the schema: a list item, struct member or union alternative is
itself a `Field`, so a struct-typed field **is a schema**. A field is three things —
a `name`, a `dtype` and an optional `metadata` map — and a handful of well-known
metadata keys (`comment`, `index_name`, `index_level`) have typed accessors.

## Construct & mutate

A field is built from a `name` and a `dtype`; both are mutable in place.

=== "Python"

    ```python
    import yggdryl

    f = yggdryl.Field("id", yggdryl.DataType.int64())
    assert f.name == "id"
    assert f.dtype == yggdryl.DataType.int64()
    f.name = "ident"
    f.dtype = yggdryl.DataType.int32()
    ```

=== "Node"

    ```javascript
    const { Field, DataType } = require("yggdryl");

    const f = new Field("id", DataType.int64());
    f.name;                       // "id"
    f.dtype.equals(DataType.int64()); // true
    f.name = "ident";
    f.dtype = DataType.int32();
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};

    let mut f = Field::new("id", DataType::int64());
    assert_eq!(f.name, "id");
    assert_eq!(f.dtype, DataType::int64());
    f.name = "ident".into();
    f.dtype = DataType::int32();
    ```

## Metadata

The optional `metadata` is a `map<bytes, bytes>`. It is `None` until a key is set and
clears back to `None` when emptied.

=== "Python"

    ```python
    import yggdryl

    f = yggdryl.Field("id", yggdryl.DataType.int64())
    assert f.metadata is None
    f.metadata = {b"unit": b"count"}          # replace the whole map
    assert f.metadata[b"unit"] == b"count"
    f.metadata = {}                            # an empty map clears to None
    assert f.metadata is None
    ```

=== "Node"

    ```javascript
    const { Field, DataType } = require("yggdryl");

    const f = new Field("id", DataType.int64());
    f.setMetadata(Buffer.from("unit"), Buffer.from("count"));
    f.getMetadata(Buffer.from("unit")).toString();      // "count"
    f.removeMetadata(Buffer.from("unit")).toString();   // "count"
    f.getMetadata(Buffer.from("unit"));                 // null
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};

    let mut f = Field::new("id", DataType::int64());
    assert!(f.metadata.is_none());
    f.set_metadata(b"unit".to_vec(), b"count".to_vec());
    assert_eq!(f.get_metadata(b"unit"), Some(b"count".as_slice()));
    assert_eq!(f.remove_metadata(b"unit"), Some(b"count".to_vec()));
    assert!(f.metadata.is_none());              // emptied -> None
    ```

## Reserved accessors

Three well-known keys have typed getters/setters that mutate the metadata map in
place: `comment` and `index_name` (UTF-8 strings) and `index_level` (a `u16`, stored as
its decimal text). Setting `None` removes the key.

=== "Python"

    ```python
    import yggdryl

    f = yggdryl.Field("x", yggdryl.DataType.int32())
    f.comment = "a note"
    f.index_name = "idx"
    f.index_level = 7
    assert f.comment == "a note"
    assert f.index_level == 7
    assert f.metadata[b"comment"] == b"a note"   # stored under reserved byte keys
    f.comment = None                              # clears the key
    assert f.comment is None
    ```

=== "Node"

    ```javascript
    const { Field, DataType } = require("yggdryl");

    const f = new Field("x", DataType.int32());
    f.comment = "a note";
    f.indexName = "idx";
    f.indexLevel = 7;
    f.comment;                                    // "a note"
    f.indexLevel;                                 // 7
    f.getMetadata(Buffer.from("comment")).toString(); // "a note"
    f.comment = null;                             // clears the key
    f.comment;                                    // null
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};

    let mut f = Field::new("x", DataType::int32());
    f.set_comment(Some("a note"));
    f.set_index_name(Some("idx"));
    f.set_index_level(Some(7));
    assert_eq!(f.comment().as_deref(), Some("a note"));
    assert_eq!(f.index_level(), Some(7));
    assert_eq!(f.get_metadata(b"comment"), Some(b"a note".as_slice()));
    f.set_comment(None);                          // clears the key
    assert_eq!(f.comment(), None);
    ```

## Equality & hashing

A field is `==`-comparable and hashable over its `name`, `dtype` and `metadata`.

=== "Python"

    ```python
    import yggdryl

    a = yggdryl.Field("id", yggdryl.DataType.int64())
    b = yggdryl.Field("id", yggdryl.DataType.int64())
    assert a == b
    assert hash(a) == hash(b)
    b.comment = "x"
    assert a != b
    ```

=== "Node"

    ```javascript
    const { Field, DataType } = require("yggdryl");

    const a = new Field("id", DataType.int64());
    const b = new Field("id", DataType.int64());
    a.equals(b);                       // true
    a.hashCode() === b.hashCode();     // true
    b.comment = "x";
    a.equals(b);                       // false
    ```

=== "Rust"

    ```rust
    use yggdryl_schema::{DataType, Field};

    let a = Field::new("id", DataType::int64());
    let mut b = Field::new("id", DataType::int64());
    assert_eq!(a, b);
    b.set_comment(Some("x"));
    assert_ne!(a, b);
    ```

## Next

- [DataType](datatype.md) — the type a field names
- Back to [Getting started](../getting-started.md)
