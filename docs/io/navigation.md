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

## Child access & slicing

- `child_serie_at(i)` / `child_serie_by(name)` (Rust), `child_at` / `child_named` (Python),
  `childAt` / `childNamed` (Node) — the immediate child columns of a nested column.
- Python `s[a:b]` and Node `slice(start, length)` return a fresh sub-range column (delegating to
  the core `slice(offset, len)`).
