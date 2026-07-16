# Nested types (struct, list, map)

`io::nested` is the third typed family, the sibling of [fixed](fixed.md) and
[variable](var.md) — for **composite** types whose values are built from *other* columns. It ships
all three Arrow nested shapes:

| type | shape | one row is |
| --- | --- | --- |
| **struct** | an ordered, named set of heterogeneous child columns | one value per field |
| **list** | `i32` offsets over one flattened child column | the child sub-range `child[offsets[i] .. offsets[i + 1]]` |
| **map** | the optimized alias of `List<Struct<{key, value}>>` | the `key -> value` entries in the row's offset range |

A nested column holds child columns of arbitrary type — including other nested columns — so the crate
adds three recursive, type-erased carriers at the **`io` root** (not inside `nested`), because they
describe *any* column, leaf or nested:

| type | role |
| --- | --- |
| `AnySerie` | the erased **data** trait — every concrete series (`Serie<T>`, `DecimalSerie`, `ByteSerie`, `FixedSizeSerie`, `NullSerie`, and `StructSerie` / `ListSerie` / `MapSerie` themselves) implements it, so a nested column holds its children as `Box<dyn AnySerie>` |
| `AnyField` | the erased, recursive **field** descriptor — a `Leaf` (the flat `Field`, reused as `var` does), or a `Struct` / `List` / `Map` carrying its child fields inline |
| `AnyScalar` | the erased **cell value** an `AnySerie::value` yields — `Null`, a `Leaf`, or a `Struct` / `List` / `Map` of child values |

`AnySerie` reimplements nothing: every operation — length, the byte codec, equality, and Arrow
conversion — delegates to the wrapped series' own method, and each nested serie (itself an
`AnySerie`) recurses into its children. So the core still builds **without** the `arrow` feature (the
carriers are plain traits/enums over the existing series), and Arrow **recomposition is zero-copy**
wherever the underlying series is: the fixed and decimal columns hand back their shared `Arc` buffer
through their own `to_arrow_array`.

## `boxed` — erase a series, `as_serie::<T>` — recover it

Any concrete series erases into a `Box<dyn AnySerie>` with the free function `boxed`, and comes
back out with a field-keyed safe downcast — no `Column` enum, no per-type wrapper:

```rust
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::boxed;

let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
let names = boxed(Utf8Serie::from_strs(&[Some("ann"), None, Some("cara")]));

// Recover the concrete series by asserting the element type.
let ids: &Serie<i64> = ids.as_serie::<i64>().unwrap();
assert_eq!(ids.get(0), Some(1));
```

`as_serie::<T>` (and its siblings `as_decimal::<B>` / `as_bytes_serie::<E>`) is the `as_ref<T>`
assumption keyed on the linked field: it downcasts to the concrete series if the element type
matches, else `None` — so a caller that knows a column's `AnyField` can safely assume its type.

## `AnyField` — the recursive schema leaf

`AnyField` is the erased field that a leaf and every nested shape share. A leaf reuses the flat
`Field` (as `var` does); a `Struct` / `List` / `Map` carries its ordered child `AnyField`s inline, so
schemas nest to any depth without a root→nested dependency:

```rust
use yggdryl_core::io::FieldType;
use yggdryl_core::io::fixed::{Field, PrimitiveType};
use yggdryl_core::io::AnyField;

let id = AnyField::leaf(Field::new("id", &PrimitiveType::<i64>::new(), false));
let person = AnyField::struct_("person", vec![id.clone()], true);
assert!(person.is_struct() && person.nullable());
assert_eq!(person.children()[0].name(), "id");
```

## Self-describing series (`.named` / `from_series`)

A nested column builds from *child columns*, and each nested factory needs to know each child's
**name + type**. Rather than pass a separate schema, a series **names itself**: `.named(name)` wraps a
column into a lightweight `NamedSerie` carrier that pairs it with its inferred `AnyField`. The
self-describing factories then read the schema straight off the columns — `StructSerie::from_series`
takes the named columns, and `ListSerie::from_values` / `MapSerie::from_entries` take named children
(the item, or the key/value).

The three languages differ only in how a heterogeneous child crosses the boundary: Python takes the
live `Serie` objects (with a name), Node (napi cannot accept an arbitrary one-of-many class instance)
takes a `StructField` schema plus each child's `serializeBytes()` frame, and Rust names each column
inline with `.named`.

=== "Python"

    ```python
    from yggdryl.types import StructSerie, I64Serie, Utf8Serie

    ids = I64Serie([1, 2, 3])
    names = Utf8Serie(["ann", None, "cara"])

    # The self-describing builder: (name, column) pairs — the schema is inferred from each column.
    table = StructSerie.from_series([("id", ids), ("name", names)])
    assert table.num_columns == 2 and table.field(1).name == "name"

    # Byte-for-byte identical to the StructSerie([...]) constructor.
    assert table.serialize_bytes() == StructSerie([("id", ids), ("name", names)]).serialize_bytes()
    ```

=== "Node"

    ```js
    const { StructField, StructSerie, I32Serie, Utf8Serie } = require('yggdryl').types

    const ids = new I32Serie([1, 2, 3])
    const names = new Utf8Serie(['ann', null, 'cara'])

    // The schema carries the names; each child crosses as its serializeBytes() frame.
    const schema = new StructField('person', [ids.toField('id'), names.toField('name')], false)
    const table = StructSerie.fromColumns(schema, [ids.serializeBytes(), names.serializeBytes()])
    assert(table.numColumns === 2 && table.field(1).name === 'name')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::var::Utf8Serie;
    use yggdryl_core::io::AnySerie;                 // brings `.named` into scope
    use yggdryl_core::io::nested::StructSerie;

    // A serie carries its own field: `.named("id")` pairs the column with its inferred `AnyField`.
    let ids = Serie::from_values(&[1i64, 2, 3]).named("id");
    let names = Utf8Serie::from_strs(&[Some("ann"), None, Some("cara")]).named("name");
    let table = StructSerie::from_series(vec![ids, names]).unwrap();
    assert_eq!(table.num_columns(), 2);
    assert_eq!(table.field(1).unwrap().name(), "name");
    ```

## StructField — one schema, two Arrow faces

`StructField` is the **single source of truth** for a struct's shape — a validated
struct-shaped `AnyField` (its children hold the ordered child fields). It maps to **both** an
Arrow `Field` (of `Struct` type) *and* an Arrow `Schema` (its children as a top-level schema) —
the natural bridge for a struct column ↔ a `RecordBatch`.

```rust
use yggdryl_core::io::FieldType;
use yggdryl_core::io::fixed::{Field, PrimitiveType};
use yggdryl_core::io::AnyField;
use yggdryl_core::io::nested::StructField;

let schema = StructField::new(
    "person",
    vec![
        AnyField::leaf(Field::new("id", &PrimitiveType::<i64>::new(), false)),
        AnyField::leaf(Field::new("name", &PrimitiveType::<i32>::new(), true)),
    ],
    true,
);
assert_eq!(schema.name(), "person");
assert_eq!(schema.type_name(), "struct");
assert!(schema.is_struct() && schema.nullable());
assert_eq!(schema.num_fields(), 2);
assert_eq!(schema.index_of("name"), Some(1));
```

It is a value type — equal and hashable by content, so a schema works as a map key — with
`with_field` / `with_metadata` / `with_nullable` builders for one-line immutable updates. A
struct field can itself be a child of another struct (an `AnyField::Struct` child), so schemas
nest arbitrarily. `ListField` (its element `item` field) and `MapField` (its `key` / `value` fields
+ `keys_sorted`) are the list and map equivalents, and all three interoperate as `AnyField` children.

## StructSerie — a struct column

`StructSerie` is a nullable struct column: one child `Box<dyn AnySerie>` per field (all the same
length), an ordered schema of `AnyField`s, and an optional top-level validity mask (a null
struct row). It is itself an `AnySerie`, so struct-of-struct nests to any depth.

```rust
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnyScalar};
use yggdryl_core::io::nested::StructSerie;

let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
let names = boxed(Utf8Serie::from_strs(&[Some("ann"), Some("bo"), None]));
let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();

assert_eq!(table.len(), 3);
assert_eq!(table.num_columns(), 2);
assert_eq!(table.field(1).unwrap().name(), "name");

// Downcast a child back to its concrete series, keyed on the field's type.
let ids: &Serie<i64> = table.column(0).unwrap().as_serie::<i64>().unwrap();
assert_eq!(ids.get(1), Some(2));

// A row is an `AnyScalar::Struct` of per-field erased values; a null row is `AnyScalar::Null`.
let AnyScalar::Struct(row) = table.row(0) else { unreachable!() };
assert_eq!(row.len(), 2);

// It round-trips byte-exactly through its own codec (schema + data are self-contained).
assert_eq!(
    StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap(),
    table,
);
```

Every child column may itself be a `StructSerie`, `ListSerie`, or `MapSerie`, so nesting recurses to
any depth and round-trips whole.

## List — offsets over one flattened child

A `ListSerie` is a **nullable list column**: `i32` **offsets** over one flattened child column, plus
an optional per-row validity mask. Row `i` is the child sub-range `child[offsets[i] .. offsets[i + 1]]`
— so `[[1, 2], [], [3]]` stores the flat child `[1, 2, 3]` and offsets `[0, 2, 2, 3]`. The offsets
have `len + 1` entries: they start at `0`, are non-decreasing, and end at the child length. Storing
one flat child (rather than a `Vec` of sub-columns) is the optimization — a whole list column is one
child column plus a small offsets array.

- **build** — from a self-describing child (the item) + offsets + an optional `present` mask
  (`present[i] == false` marks row `i` a null list);
- **navigate** — `values()` (the flat child), `offsets()`, `row(i)` (the row's element sub-column, or
  a null row), `item_field()`, `len` / `null_count` / `has_nulls`, and `slice(offset, len)` (a fresh
  windowed column, offsets rebased to `0`);
- **describe** — `to_field(name)` yields a `ListField` (nullability inferred from the null rows).

=== "Python"

    ```python
    from yggdryl.types import ListSerie, I32Serie

    # Rows [[1, 2], [], [3]] over the flat child [1, 2, 3] partitioned by offsets [0, 2, 2, 3].
    col = ListSerie(I32Serie([1, 2, 3]), [0, 2, 2, 3])
    assert len(col) == 3 and col.null_count == 0
    assert col.offsets == [0, 2, 2, 3]

    # The flat child, rewrapped to its concrete Serie.
    assert isinstance(col.values, I32Serie) and col.values.get(2) == 3

    # Each row is its element sub-Serie (None for a null row).
    row0 = col.row(0)
    assert [row0.get(i) for i in range(len(row0))] == [1, 2]
    assert len(col.row(1)) == 0            # the empty row

    # A null list row, via the present mask.
    with_null = ListSerie(I32Serie([1, 2, 3]), [0, 2, 2, 3], present=[True, False, True])
    assert with_null.has_nulls and with_null.row(1) is None

    # Byte round-trip — the same canonical bytes in every language.
    assert ListSerie.deserialize_bytes(col.serialize_bytes()) == col
    ```

=== "Node"

    ```js
    const { ListSerie, ListField, I32Serie } = require('yggdryl').types

    // A child crosses as its field + serializeBytes() frame (napi cannot take a Serie instance).
    const child = new I32Serie([1, 2, 3])
    const col = ListSerie.fromParts(child.toField('item'), child.serializeBytes(), [0, 2, 2, 3])
    assert(col.length === 3 && col.nullCount === 0)
    assert.deepEqual(col.offsets, [0, 2, 2, 3])

    // The flat child crosses back as bytes; rebuild it with the matching Serie class.
    assert(I32Serie.deserializeBytes(col.itemBytes()).get(2) === 3)

    // A null list row, via the present mask.
    const withNull = ListSerie.fromParts(
      child.toField('item'), child.serializeBytes(), [0, 2, 2, 3], [true, false, true])
    assert(withNull.hasNulls)

    const field = col.toField('scores')   // -> a ListField
    assert(field instanceof ListField)

    // Byte round-trip — byte-identical to the Rust core and the Python extension.
    assert(ListSerie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::AnySerie;                 // brings `.named` into scope
    use yggdryl_core::io::nested::ListSerie;

    // Rows [[1, 2], [], [3]] over the flat child [1, 2, 3]. The item names itself with `.named`.
    let items = Serie::from_values(&[1i32, 2, 3]).named("item");
    let list = ListSerie::from_values(items, &[0, 2, 2, 3], None).unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list.offsets(), &[0, 2, 2, 3]);

    // The flat child, downcast back to its concrete Serie.
    let child: &Serie<i32> = list.values().as_serie::<i32>().unwrap();
    assert_eq!(child.get(2), Some(3));

    // Per-row access: `row_scalar(i)` is a `ListScalar` (its `is_null` + element sub-column).
    assert_eq!(list.row_scalar(0).len(), 2);
    assert_eq!(list.row_scalar(1).len(), 0);        // the empty row

    // A fresh windowed column (offsets rebased to 0), and the byte round-trip.
    let tail = list.slice(1, 2);
    assert_eq!(tail.len(), 2);
    assert_eq!(ListSerie::deserialize_bytes(&list.serialize_bytes()).unwrap(), list);
    ```

## Map — an optimized `List<Struct<key, value>>`

A `MapSerie` is a **nullable map column** — the optimized alias of `List<Struct<{key, value}>>`. It
holds a two-column entries store (a `StructSerie` of `key` and `value`), `i32` offsets over those
entries, an optional per-row validity mask, and a `keys_sorted` flag. Row `i` is the entries
`key[j] -> value[j]` for `j` in `[offsets[i], offsets[i + 1])`. A **map key is never null** (Arrow's
`Map` invariant), so the key column must not carry nulls — otherwise a guided error.

- **build** — `from_entries(keys, values, offsets, present, keys_sorted)`: two self-describing child
  columns + offsets (+ an optional null-row mask);
- **look up** — `get_value(row, key)` scans the row's entries and returns the mapped value (an
  allocation-free bit-canonical key compare; the first positional match wins for duplicate keys);
- **navigate** — `keys()` / `values()` (the flat children), `offsets()`, `row(i)` (the row's
  `[key, value]` entries), `key_field()` / `value_field()`, `keys_sorted`, and `slice(offset, len)`;
- **describe** — `to_field(name)` yields a `MapField`.

=== "Python"

    ```python
    from yggdryl.types import MapSerie, StructSerie, Utf8Serie, I64Serie

    # Rows {"a": 1, "b": 2}, {"c": 3} over 3 entries partitioned by offsets [0, 2, 3].
    keys = Utf8Serie(["a", "b", "c"])
    values = I64Serie([1, 2, 3])
    col = MapSerie(keys, values, [0, 2, 3])
    assert len(col) == 2 and col.offsets == [0, 2, 3] and not col.keys_sorted

    # The flat key/value children, rewrapped to their concrete Serie.
    assert isinstance(col.keys, Utf8Serie) and col.keys.get(0) == "a"

    # get_value: the probe is a single-element Serie of the key type; the hit is a one-element
    # value Serie (None if the key is absent from that row). i64 cells cross as decimal strings.
    assert col.get_value(0, Utf8Serie(["b"])).get(0) == "2"
    assert col.get_value(0, Utf8Serie(["c"])) is None      # "c" is only in row 1
    assert col.get_value(1, Utf8Serie(["c"])).get(0) == "3"

    # A row is its [key, value] entries StructSerie.
    assert isinstance(col.row(0), StructSerie) and len(col.row(0)) == 2

    # Byte round-trip — the same canonical bytes in every language.
    assert MapSerie.deserialize_bytes(col.serialize_bytes()) == col
    ```

=== "Node"

    ```js
    const { MapSerie, MapField, Utf8Serie, I64Serie } = require('yggdryl').types

    // Each child crosses as its field + serializeBytes() frame. i64 values cross as strings.
    const keys = new Utf8Serie(['a', 'b', 'c'])
    const values = new I64Serie(['1', '2', '3'])
    const col = MapSerie.fromParts(
      keys.toField('key'), keys.serializeBytes(),
      values.toField('value'), values.serializeBytes(),
      [0, 2, 3], undefined, false)
    assert(col.length === 2 && col.keysSorted === false)
    assert.deepEqual(col.offsets, [0, 2, 3])

    // getValueBytes: the probe is a leaf key's canonical bytes; the hit is the value's LE bytes.
    const i64le = (n) => { const b = Buffer.alloc(8); b.writeBigInt64LE(BigInt(n)); return b }
    assert(col.getValueBytes(0, Buffer.from('b')).equals(i64le(2)))  // row0 {a->1, b->2}
    assert(col.getValueBytes(0, Buffer.from('z')) === null)          // absent key

    const field = col.toField('counts')   // -> a MapField
    assert(field instanceof MapField)

    // Byte round-trip — byte-identical to the Rust core and the Python extension.
    assert(MapSerie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::var::Utf8Serie;
    use yggdryl_core::io::AnySerie;                 // brings `.named` into scope
    use yggdryl_core::io::nested::MapSerie;

    // Rows {"a": 1, "b": 2}, {"c": 3} over 3 entries partitioned by offsets [0, 2, 3].
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
    let values = Serie::from_values(&[1i64, 2, 3]).named("value");
    let map = MapSerie::from_entries(keys, values, &[0, 2, 3], None, false).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.offsets(), &[0, 2, 3]);

    // get_value: probe with a key scalar; the value scalar (or None) comes back.
    let first_key = map.keys().value(0);                       // the "a" cell of row 0
    assert_eq!(map.get_value(0, &first_key), Some(map.values().value(0)));

    // The flat key/value children, and the byte round-trip.
    let keys: &Utf8Serie = map.keys().as_bytes_serie().unwrap();
    assert_eq!(keys.get(2), Some("c"));
    assert_eq!(MapSerie::deserialize_bytes(&map.serialize_bytes()).unwrap(), map);
    ```

## The optimized byte codec — one self-contained frame, recursive

Every nested serie has `serialize_bytes()` / `deserialize_bytes()` (Node:
`serializeBytes` / `deserializeBytes`) producing the **same canonical bytes in all three languages**,
so a struct / list / map built in one language deserializes byte-for-byte in another. The frame is
self-contained and self-describing — it packs the schema, the row count, the top-level validity, and
the offsets into **one** pre-sized buffer, then the child column(s) serialize themselves into it:

| serie | frame layout |
| --- | --- |
| `StructSerie` | `[schema][len][validity?][children…]` |
| `ListSerie`   | `[schema][len][validity?][offsets][child]` |
| `MapSerie`    | `[schema][len][validity?][offsets][entries]` (entries = a two-column struct frame) |

Because each child serializes itself through the same recursive dispatch, an arbitrarily deep
nesting — a `list<map<utf8, struct<...>>>`, a `map<utf8, list<i32>>`, a `struct` of any of these —
**round-trips whole** in a single `serialize_bytes` / `deserialize_bytes` pair, and equal columns
serialize to equal bytes (offsets and validity are canonicalized at construction). This is the
cross-language wire form: it is the exact same bytes the Rust core reads to build its Arrow arrays,
and — because Node has no Arrow-array bridge — it is how a Node-built column reaches pyarrow (through
Rust or Python).

## Arrow interop

Behind the `arrow` feature, `StructSerie` / `ListSerie` / `MapSerie` bridge to Arrow's `StructArray`
(and `RecordBatch`), `ListArray`, and `MapArray` **recursively**, and the Python extension exports
each to `pyarrow` zero-copy via the Arrow C Data Interface. **Arrow interop →
[see Arrow interop → Nested](../arrow/nested.md)** for the full three-language reference (the
`StructArray` / `RecordBatch` / `ListArray` / `MapArray` conversions, the null-row rule, and the
PyCapsule bridge). The mapping is centralized on [`DataTypeId`](schema.md), and Arrow types never
appear in a public signature.

## In Python and Node

Both extensions mirror the whole nested surface — `StructField` / `ListField` / `MapField` (the
schemas) and `StructSerie` / `ListSerie` / `MapSerie` (the columns) — over the same core, so a nested
value built in one language **serializes to identical bytes** in the others. The one platform
difference is how a heterogeneous child column is passed in: Python takes the live `Serie` objects
directly, while Node (napi cannot accept an arbitrary one-of-many class instance) takes a schema/field
plus each child's `serializeBytes()` frame. Both round-trip byte-for-byte with the Rust core.

=== "Python"

    ```python
    from yggdryl.types import StructField, StructSerie, I64Serie, Utf8Serie

    ids = I64Serie([1, 2, 3])
    names = Utf8Serie(["ann", None, "cara"])
    table = StructSerie([("id", ids), ("name", names)])          # live Serie children
    assert len(table) == 3 and table.num_columns == 2
    assert table.column_named("name").get(0) == "ann"            # re-wrapped, typed child
    assert StructSerie.deserialize_bytes(table.serialize_bytes()) == table

    schema = table.to_field("person")                            # StructField schema
    assert schema.num_fields == 2 and schema.field(1).name == "name"
    ```

=== "Node"

    ```js
    const { StructField, StructSerie, I32Serie, Utf8Serie } = require('yggdryl').types

    const ids = new I32Serie([1, 2, 3])
    const names = new Utf8Serie(['ann', null, 'cara'])
    // A child crosses as a schema field + its serialized bytes.
    const schema = new StructField('person', [ids.toField('id'), names.toField('name')], false)
    const table = StructSerie.fromColumns(schema, [ids.serializeBytes(), names.serializeBytes()])
    assert(table.length === 3 && table.numColumns === 2)
    assert(Utf8Serie.deserializeBytes(table.columnBytesNamed('name')).get(0) === 'ann')
    assert(StructSerie.deserializeBytes(table.serializeBytes()).equals(table))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::boxed;
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::var::Utf8Serie;
    use yggdryl_core::io::nested::StructSerie;

    let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
    let names = boxed(Utf8Serie::from_strs(&[Some("ann"), None, Some("cara")]));
    let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();
    assert_eq!(table.len(), 3);
    let ids: &Serie<i64> = table.column(0).unwrap().as_serie::<i64>().unwrap();
    assert_eq!(ids.get(0), Some(1));
    assert_eq!(StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap(), table);
    ```

The Python extension additionally exports every nested column to `pyarrow` **zero-copy** through the
Arrow PyCapsule protocol, and the Node extension interops through the shared byte codec — see
[Arrow interop → Nested](../arrow/nested.md).
