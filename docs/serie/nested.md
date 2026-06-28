# Nested series (struct / list / map)

`StructSerie`, `ListSerie<O>` and `MapSerie` are **columns of columns**. Each builds its
child [`Serie`]s **recursively** through the same factory, so arbitrarily deep nesting (a
list of structs of maps, …) resolves uniformly. The `NestedSerie` trait exposes
`child_count` / `child(index)` / `children` / `child_by_name` and the `a.b.c` path
navigation.

See also: [Serie (the typed column)](serie.md) · [Lazy & range](lazy.md) · [Frame](frame.md).

## Build a nested column

The bindings build each container from native values in one line: `Serie.struct(name,
children)` from child columns, `Serie.list(name, rows)` from a list of sub-lists, and
`Serie.map(name, rows)` from a list of dicts. The element / key / value types are inferred
just like the [`Serie` constructor](serie.md#type-inference), or pass an explicit
`dtype`. In Rust the same constructors are `StructSerie::from_children`,
`ListSerie::<O>::from_values` (a flattened element column + per-row lengths) and
`MapSerie::from_values` (flattened keys/values + per-row lengths).

!!! tip "The plain constructor infers these too"
    In the bindings you rarely need the explicit factories: `Serie(name, values)` /
    `new Serie(name, values)` **auto-detects** a list value (→ list column) or a dict /
    object value (→ map column), recursively. The `Serie.list` / `Serie.map` factories are
    the explicit form (and let you spell out an element `dtype`). See
    [type inference](serie.md#type-inference).

=== "Python"

    ```python
    import yggdryl

    # struct: from child columns
    rec = yggdryl.Serie.struct("rec", [
        yggdryl.Serie("id", [1, 2]),
        yggdryl.Serie("name", ["a", "b"]),
    ])
    assert rec.children()[0].name == "id"

    # list: from a list of sub-lists (None is a null row)
    nums = yggdryl.Serie.list("nums", [[1, 2], [], None, [3]])
    assert nums.num_rows == 4 and nums.null_count == 1
    assert nums.value_at(0) == "[1, 2]"
    floats = yggdryl.Serie.list("f", [[1], [2, 3]], dtype="float64")  # cast the elements

    # map: from a list of dicts (None is a null row)
    m = yggdryl.Serie.map("m", [{"a": 1, "b": 2}, {"c": 3}])
    assert m.value_at(0) == "{a=1, b=2}"
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    // struct: from child columns
    const rec = Serie.struct('rec', [
      new Serie('id', [1, 2]),
      new Serie('name', ['a', 'b']),
    ])
    if (rec.children()[0].name !== 'id') throw new Error('struct')

    // list: from an array of sub-arrays (null is a null row)
    const nums = Serie.list('nums', [[1, 2], [], null, [3]])
    if (nums.numRows !== 4 || nums.nullCount !== 1) throw new Error('list')
    if (nums.valueAt(0) !== '[1, 2]') throw new Error('render')
    const floats = Serie.list('f', [[1], [2, 3]], 'float64')  // cast the elements

    // map: from an array of objects (null is a null row)
    const m = Serie.map('m', [{ a: 1, b: 2 }, { c: 3 }])
    if (m.valueAt(0) !== '{a=1, b=2}') throw new Error('map')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{
        Int32Serie, VarcharSerie, StructSerie, ListSerie, MapSerie,
        NestedSerie, Serie, SerieRef, Scalar,
    };
    use std::sync::Arc;

    // struct: from child columns
    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let name: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("name", vec![Some("a"), Some("b")]));
    let rec = StructSerie::from_children("rec", vec![id, name])?;
    assert_eq!(rec.child_count(), 2);

    // list: a flattened element column + per-row lengths ([[1, 2], [], None, [3]])
    let flat: SerieRef = Arc::new(Int32Serie::from_values("item", vec![Some(1), Some(2), Some(3)]));
    let nums = ListSerie::<i32>::from_values("nums", flat, &[Some(2), Some(0), None, Some(1)])?;
    assert_eq!(nums.null_count(), 1);
    assert_eq!(nums.value_at(0), Scalar::Other("[1, 2]".into()));

    // map: flattened keys/values + per-row lengths ([{a=1, b=2}, {c=3}])
    let keys: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("key", vec![Some("a"), Some("b"), Some("c")]));
    let vals: SerieRef = Arc::new(Int32Serie::from_values("value", vec![Some(1), Some(2), Some(3)]));
    let m = MapSerie::from_values("m", keys, vals, &[Some(2), Some(1)])?;
    assert_eq!(m.value_at(0), Scalar::Other("{a=1, b=2}".into()));
    ```

## Lists & maps in depth

A `ListSerie<O>` keeps the elements of every row in a single flattened child column; each
row is a zero-copy sub-slice of it (`value_slice`). A `MapSerie` keeps two flattened child
columns (keys and values), each row a `[offset, next_offset)` run of pairs. Both build
their children recursively, so a `list<struct<…>>` or `map<str, list<int>>` resolves the
whole way down.

=== "Python"

    ```python
    import yggdryl

    nums = yggdryl.Serie.list("nums", [[1, 2], [3], None])
    assert nums.child(0).name == "item"                  # the flattened element column
    assert nums.value_at(1) == "[3]"
    assert nums.value_at(2) is None                      # null row

    m = yggdryl.Serie.map("m", [{"a": 1, "b": 2}, {"c": 3}])
    assert m.child("value").value_at(0) == 1             # the flattened value column
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const nums = Serie.list('nums', [[1, 2], [3], null])
    if (nums.child(0).name !== 'item') throw new Error('child')
    if (nums.valueAt(1) !== '[3]') throw new Error('render')
    if (nums.valueAt(2) !== null) throw new Error('null')

    const m = Serie.map('m', [{ a: 1, b: 2 }, { c: 3 }])
    if (m.child('value').valueAt(0) !== 1) throw new Error('value')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, NestedSerie, ListSerie, Scalar, Serie};
    use yggdryl_serie::arrow_array::{ArrayRef, ListArray};
    use yggdryl_serie::arrow_array::types::Int32Type;
    use std::sync::Arc;

    // list<int32>: [[1, 2], [3], null] — also reachable from any Arrow array via `from_array`
    let la = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![
        Some(vec![Some(1), Some(2)]), Some(vec![Some(3)]), None,
    ]);
    let serie = from_array("l", Arc::new(la) as ArrayRef)?;
    let list = serie.as_any().downcast_ref::<ListSerie<i32>>().unwrap();

    assert_eq!(list.value_slice(0).unwrap().len(), 2);   // the sub-list is a zero-copy Serie
    assert!(list.value_slice(2).is_none());              // null row
    assert_eq!(list.value_at(1), Scalar::Other("[3]".into()));
    ```

## Child access — by index, name or path

Any column exposes `select("a.b.c")` to navigate into nested children, and `as_nested()`
(Rust) / `child` / `children` for the child API (by index, or by name with a case-sensitive
→ case-insensitive fallback). A path segment may be **wrapped** (`[name]`, `"name"`,
`'name'`, `` `name` ``) to match the literal name exactly (and to contain dots); a bare
numeric segment is a child index. The path is **parsed first**, so `select` returns
`Result<Option<…>>`: a malformed path (unclosed wrapper, empty segment) is an error, while
a well-formed path that does not resolve — a missing child, or a leaf column — is `Ok(None)`
(`None` in the bindings).

=== "Python"

    ```python
    import yggdryl

    rec = yggdryl.Serie.struct("rec", [
        yggdryl.Serie.struct("inner", [yggdryl.Serie("a", [1, 2])]),
        yggdryl.Serie("Label", ["x", "y"]),
    ])
    assert rec.select("inner.a")[1] == 2          # node path
    assert rec.select("label").name == "Label"    # case-insensitive
    assert rec.child(0).name == "inner"           # by index
    assert rec.select("inner.zzz") is None        # unresolved
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const rec = Serie.struct('rec', [
      Serie.struct('inner', [new Serie('a', [1, 2])]),
      new Serie('Label', ['x', 'y']),
    ])
    if (rec.select('inner.a').get(1) !== 2) throw new Error('path')
    if (rec.select('label').name !== 'Label') throw new Error('ci')
    if (rec.child(0).name !== 'inner') throw new Error('index')
    if (rec.select('inner.zzz') !== null) throw new Error('unresolved')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{StructSerie, Int32Serie, VarcharSerie, NestedSerie, Serie, SerieRef, Scalar};
    use std::sync::Arc;

    let inner: SerieRef = Arc::new(StructSerie::from_children("inner", vec![
        Arc::new(Int32Serie::from_values("a", vec![Some(1), Some(2)])) as SerieRef,
    ]).unwrap());
    let label: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("Label", vec![Some("x"), Some("y")]));
    let rec: SerieRef = Arc::new(StructSerie::from_children("rec", vec![inner, label]).unwrap());

    // select returns Result<Option<…>>: Ok(Some) found, Ok(None) unresolved, Err malformed
    assert_eq!(rec.select("inner.a")?.unwrap().value_at(1), Scalar::Int(2)); // path
    assert_eq!(rec.select("label")?.unwrap().name(), "Label");               // case-insensitive
    assert_eq!(rec.select("[inner].a")?.unwrap().value_at(0), Scalar::Int(1)); // wrapped exact
    assert!(rec.select("inner.zzz")?.is_none());                             // unresolved
    assert!(rec.select("inner.").is_err());                                  // malformed path

    let nested = rec.as_nested().unwrap();
    assert_eq!(nested.child(0).unwrap().name(), "inner"); // by index
    assert_eq!(nested.children().len(), 2);
    ```

A struct-typed column is also a **DataFrame** — its children *are* the table's columns.
See [Frame (DataFrame)](frame.md) for the table surface (select / filter / sort / records).

## Next

- [Frame (DataFrame)](frame.md) — the table surface over a struct column
- [Serie (the typed column)](serie.md) — the base column
- [Lazy & range](lazy.md) — computed columns and the row index
