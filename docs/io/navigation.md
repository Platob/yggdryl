# Navigation

A [nested column](nested.md) is a tree — a `struct` of columns, a `list` of rows, a `map` of
entries — and yggdryl lets you address any node or cell inside it two ways: by **path** (a
string like `"parent.child"` or `"a[1]"`) or by **coordinate** (a tuple / array of indices).
The addressing is defined once on the erased `AnySerie` in the core and surfaced idiomatically
in each binding — Python's `[]` operator, Node's named methods.

## Paths — the `NodePath` grammar

A path is parsed by a single centralized parser (`io::NodePath`) over a small set of **breaking
characters**. The separators `.` and `-` and the bracketed accessors `[...]`, `(...)`, `{...}`
split a path into segments; a segment is either a **name** (a child by name) or an **index** (a
positional child, or — as the *final* segment — a cell):

| Path | Meaning |
|---|---|
| `a.b` | child column `b` of child `a` (a **column**) |
| `a[1]` | cell `1` of leaf column `a` (a **value**) |
| `a[0].b[2]` | descend list `a` row 0 → struct field `b` → cell 2 |
| `` `odd.name` `` | a backtick-quoted name containing a breaking char (double a backtick to escape) |

A path whose final segment is a **name** addresses a sub-**column**; a final **index**
addresses a single **cell**. Writing targets a cell (a final index).

## Reading

=== "Python"

    ```python
    from yggdryl.types import StructSerie, I64Serie, ListSerie
    from yggdryl.types import Utf8Serie

    ids   = I64Serie([1, 2, 3])
    names = Utf8Serie(["ann", None, "cara"])
    table = StructSerie([("id", ids), ("name", names)])

    row   = table[0]            # a row -> {"id": 1, "name": "ann"}
    cell  = table[2, 1]         # deep cell by coordinate -> "cara"
    cell2 = table["name[2]"]    # deep cell by path       -> "cara"
    col   = table["name"]       # a sub-column (Utf8Serie)
    n     = table.num_children  # 2
    ```

=== "Node"

    ```js
    const { StructSerie, I64Serie, Utf8Serie } = require('yggdryl').types

    const ids   = I64Serie.fromValues([1n, 2n, 3n])
    const names = Utf8Serie.fromOptions(['ann', null, 'cara'])
    const table = StructSerie.fromColumns(/* schema */ table.toField('t'),
                                          [ids.serializeBytes(), names.serializeBytes()])

    const cell = table.getAt([2, 1])   // deep cell by coordinate -> 'cara'
    const cell2 = table.getPath('name[2]')
    const colFrame = table.getColumn('name') // a Buffer frame -> Utf8Serie.deserializeBytes(colFrame)
    const n = table.numChildren()
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::var::Utf8Serie;
    use yggdryl_core::io::nested::StructSerie;
    use yggdryl_core::io::{boxed, AnySerie};

    let table = boxed(StructSerie::from_named(vec![
        ("id", boxed(Serie::from_values(&[1i64, 2, 3]))),
        ("name", boxed(Utf8Serie::from_strs(&[Some("ann"), None, Some("cara")]))),
    ])
    .unwrap());

    let cell = table.get_at(&[1, 2]).unwrap();          // deep cell by coordinate
    let cell2 = table.get_scalar_by_path("name[2]").unwrap(); // deep cell by path
    let col = table.get_by_path("name").unwrap();       // a &dyn AnySerie sub-column
    assert_eq!(table.num_children(), 2);
    ```

- `get_by_path(path)` → the sub-**column** (`&dyn AnySerie`); a path of all names.
- `get_scalar_by_path(path)` / `get_at(&coords)` → a single **cell**; the read-twins of the
  setters below, so `col.get_at(c)` reads exactly what `col.set_at(c, v)` writes.
- On a top-level column, `value(i)` reads row `i`; the Python `s[i]` and Node `getAt([i])` map
  to it (a struct row is a dict/object, a list row its element sub-column, a map row its
  entries).

## Writing

Writes are **length-preserving** — every deep set overwrites an existing cell, so it can never
desync a nested column's offsets or a struct's equal-length invariant. To *grow* a column, use
`append` / `concat` instead.

=== "Python"

    ```python
    table[0, 1] = "ANNE"       # set deep cell by coordinate
    table["name[1]"] = "bo"    # set deep cell by path (fills the previously-null cell)
    assert table["name[1]"] == "bo"
    ```

=== "Node"

    ```js
    table.setAt([0, 1], 'ANNE')
    table.setPath('name[1]', 'bo')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Field;
    use yggdryl_core::io::{AnyScalar, DataTypeId};

    let mut table = table; // from above
    let v = AnyScalar::leaf(Field::of("", DataTypeId::Utf8, 0, false), b"bo".to_vec());
    table.set_by_path("name[1]", &v).unwrap();
    assert_eq!(table.get_scalar_by_path("name[1]").unwrap(), v);
    ```

The value is type-checked against the target leaf (a wrong type or an out-of-range value is a
guided error, identical across the three languages); a null value writes a null under lenient
nullability.

## Replace a child column

A nested column's immediate children are themselves columns, and you can swap a **whole child
column** in place two ways: **positionally** with `set_child_at(index, serie)`, or **by name** with
`set_child_by(name, serie)` (dict-like add-or-replace on a struct). The replacement is
length-preserving — its `len()` must equal the container's row count — and the derived schema
updates automatically.

=== "Python"

    ```python
    from yggdryl.types import StructSerie, ListSerie, MapSerie
    from yggdryl.types import I32Serie, I64Serie, U8Serie, Utf8Serie

    # struct: positional replace (col[index]) + dict-like add/replace (by name)
    st = StructSerie([("id", I64Serie([1, 2, 3])), ("name", Utf8Serie(["a", "b", "c"]))])
    st.set_child_at(0, I32Serie([10, 20, 30]))     # col 0 keeps its name "id", takes new type + data
    assert st.column(0).to_options() == [10, 20, 30]
    assert st.field(0).name == "id" and st.field(0).type_name == "i32"

    st.set_child_by("score", U8Serie([7, 8, 9]))   # a new name -> add a column
    assert st.column_named("score").to_options() == [7, 8, 9]

    st[0] = I32Serie([100, 200, 300])              # __setitem__ by int  -> set_child_at
    st["score"] = U8Serie([1, 1, 1])               # __setitem__ by name -> set_child_by (replace)

    # list: index 0 / "item" is the flattened item child
    lst = ListSerie(I32Serie([10, 20, 30, 40]), [0, 3, 3, 4])
    lst.set_child_at(0, I32Serie([1, 2, 3, 4]))
    lst.set_child_by("item", I32Serie([5, 6, 7, 8]))

    # map: index 0 / "key" is the keys column, index 1 / "value" is the values column
    mp = MapSerie(Utf8Serie(["a", "b", "c"]), I32Serie([1, 2, 3]), [0, 2, 3])
    mp.set_child_at(0, Utf8Serie(["x", "y", "z"]))   # keys (must stay non-null)
    mp.set_child_by("value", I32Serie([10, 20, 30]))
    ```

=== "Node"

    ```js
    const { StructField, StructSerie, ListSerie, MapSerie, I32Serie, I64Serie } = require('yggdryl').types

    // struct { x: i32, y: i32 }
    const x = new I32Serie([1, 2, 3])
    const y = new I32Serie([10, 20, 30])
    const st = StructSerie.fromColumns(
      new StructField('s', [x.toField('x'), y.toField('y')], false),
      [x.serializeBytes(), y.serializeBytes()],
    )
    st.setChildAt(0, new I64Serie(['100', '200', '300']))  // col 0 keeps its name "x", type -> i64
    st.setChildBy('score', new I32Serie([7, 8, 9]))        // a new name -> add a column

    // list<i32> = [[1,2],[3]] — item child at index 0 / "item"
    const items = new I32Serie([1, 2, 3])
    const list = ListSerie.fromParts(items.toField('item'), items.serializeBytes(), [0, 2, 3])
    list.setChildAt(0, new I64Serie(['9', '8', '7']))
    list.setChildBy('item', new I64Serie(['1', '2', '3']))

    // map<i32,i32> — keys at 0/"key", values at 1/"value"
    const keys = new I32Serie([1, 2, 3])
    const vals = new I32Serie([10, 20, 30])
    const map = MapSerie.fromParts(
      keys.toField('key'), keys.serializeBytes(),
      vals.toField('value'), vals.serializeBytes(),
      [0, 2, 3],
    )
    map.setChildAt(0, new I64Serie(['5', '6', '7']))       // keys (must stay non-null)
    map.setChildBy('value', new I64Serie(['50', '60', '70']))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::var::Utf8Serie;
    use yggdryl_core::io::nested::StructSerie;
    use yggdryl_core::io::{boxed, AnySerie, DataTypeId};

    let mut table: Box<dyn AnySerie> = boxed(StructSerie::from_named(vec![
        ("id", boxed(Serie::from_values(&[1i64, 2, 3]))),
        ("name", boxed(Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]))),
    ])
    .unwrap());

    // Positional replace: col 0 keeps its name "id", takes the new type + data.
    table
        .set_child_at(0, boxed(Serie::from_values(&[10i32, 20, 30])).as_ref())
        .unwrap();
    assert_eq!(table.field("t").child_field_at(0).unwrap().type_id(), DataTypeId::I32);

    // Name-keyed add-or-replace (dict-like on a struct): a new name adds a column.
    table
        .set_child_by("score", boxed(Serie::from_values(&[9i32, 8, 7])).as_ref())
        .unwrap();
    assert_eq!(table.num_children(), 3);
    ```

- **`set_child_at(index, serie)`** — positional. On a **struct** it replaces `columns[index]` (the
  slot's schema **name** is preserved — only the type + data change); on a **list** index `0` is the
  flattened item child; on a **map** `0` is the keys column and `1` the values column.
- **`set_child_by(name, serie)`** — name-keyed. On a **struct** it is **dict-like**: an existing name
  replaces that column, a new name **adds** a field; on a **map** `"key"` / `"value"` select the two
  children; on a **list** `"item"` selects the item child.
- The new child's `len()` must equal the container's row count (a struct's rows, a list / map's
  flattened entries) — a wrong length is a guided error naming both. A **map key must stay non-null**,
  so replacing the keys column with a nullable one is a guided error.
- A **leaf** column has no children, so the operation is a guided error (overwrite a leaf cell with
  the deep-cell setters above instead). In Python the two setters live only on the nested column
  classes (a leaf column simply does not expose them); in Node and the Rust core every column exposes
  them and a leaf returns the guided error. Python's `struct[int] = serie` maps to `set_child_at` and
  `struct["name"] = serie` to `set_child_by` — a scalar / coordinate key stays the deep-cell set from
  [Writing](#writing) above.

## Bulk range overwrite

To overwrite a **contiguous run of cells** in one shot — rather than one deep-cell set at a time —
`set_slice(offset, other)` copies every cell of `other` into `[offset, offset + other.len())`. It is
**length-preserving** and **leaf-only**.

=== "Python"

    ```python
    from yggdryl.types import I32Serie, Utf8Serie

    s = I32Serie([0, 0, 0, 0, 0])
    s.set_slice(1, I32Serie([7, 8]))        # overwrite rows [1, 3)
    assert s.to_options() == [0, 7, 8, 0, 0]
    assert len(s) == 5                      # length preserved

    s[1:3] = I32Serie([7, 8])               # the slice-assignment twin (step must be 1)
    s[0:2] = I32Serie([9, None])            # a null cell passes through -> [9, None, 8, 0, 0]

    # every leaf family — var / decimal / temporal — via the same op:
    u = Utf8Serie(["a", "b", "c"])
    u[1:3] = Utf8Serie(["B", "C"])
    assert u.to_options() == ["a", "B", "C"]
    ```

=== "Node"

    ```js
    const { I32Serie, Utf8Serie } = require('yggdryl').types

    const col = new I32Serie([0, 0, 0, 0, 0])
    col.setSlice(1, new I32Serie([7, null]))   // overwrite rows [1, 3); a null cell is preserved
    // -> [0, 7, null, 0, 0], length still 5

    // setSlice does NOT cast — the source must already be the target's leaf type.
    const u = new Utf8Serie(['a', 'b', 'c', 'd'])
    u.setSlice(1, new Utf8Serie(['X', 'Y']))   // -> ['a', 'X', 'Y', 'd']
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::{boxed, AnySerie};

    let mut col = boxed(Serie::from_values(&[0i64, 0, 0, 0, 0]));
    let patch = boxed(Serie::from_options(&[Some(7i64), None]));
    col.set_slice(1, patch.as_ref()).unwrap();  // overwrite rows [1, 3)
    assert_eq!(
        col.as_serie::<i64>().unwrap().to_options(),
        [Some(0), Some(7), None, Some(0), Some(0)],
    );
    assert_eq!(col.len(), 5);                    // length preserved
    ```

- **Length-preserving, clamped bounds.** The whole source must land inside self
  (`offset + other.len() <= len`); a source that runs past the end is a guided out-of-bounds error,
  leaving the column unchanged.
- **No cast.** Unlike the arithmetic ops (which cast the right operand), `set_slice` requires the
  source to be the target's **concrete leaf type** — a decimal must share its scale, a temporal its
  unit — else a guided error.
- **Leaf-only.** A nested column is a guided error (a whole-row range overwrite would resize a list /
  map's flattened child and desync its offsets) — replace a child with `set_child_at` /
  `set_child_by`, or grow rows with `append` / `concat`, instead.
- Python's `serie[a:b] = other` is the same op: it needs step `1` and `b - a == other.len()` (a
  length change is a guided error), and a bare `serie[i] = other` is rejected — a leaf column supports
  slice assignment only.

## Child access & slicing

- `child_serie_at(i)` / `child_serie_by(name)` (Rust), `child_at` / `child_named` (Python),
  `childAt` / `childNamed` (Node) — the immediate child columns of a nested column.
- Python `s[a:b]` and Node `slice(start, length)` return a fresh sub-range column (delegating to
  the core `slice(offset, len)`).
