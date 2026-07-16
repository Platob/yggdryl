# Arrow interop — nested (struct, list, map)

Behind the `arrow` feature, the three nested columns (see [Types → Nested](../types/nested.md)) each
bridge to their Arrow array, **recursively** — `StructSerie` ↔ `StructArray` (and `RecordBatch`),
`ListSerie` ↔ `ListArray`, `MapSerie` ↔ `MapArray` — each leaf converting through one generic
`ArrayData` (buffers + `DataType`) path, so numbers, decimals, temporal, fixed-size bytes, utf8, and
binary all map with no per-type code.

Recomposition is **zero-copy** wherever the underlying leaf is: the fixed and decimal columns hand
back their shared `Arc` buffer through their own `to_arrow_array`. The offsets (list, map) and the
top-level validity map straight onto Arrow's `OffsetBuffer` / `NullBuffer`.

Across the languages: **Rust** owns the `to_arrow_array` / `from_arrow_array` conversions; **Python**
exports and imports every nested column zero-copy through `pyarrow` (the Arrow C Data Interface, the
PyCapsule protocol); **Node** has no Arrow-array bridge (apache-arrow JS ships no C Data Interface
consumer), so its cross-language interop is the shared `serializeBytes` wire form — the exact same
bytes the Rust core reads to build these arrays.

## Struct ↔ StructArray / RecordBatch

A struct column *is* a batch of named columns: it maps to a nullable `StructArray`, or — when it has
no null rows — to a `RecordBatch` (each field a top-level batch column).

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.types import StructSerie, I32Serie, Utf8Serie

    table = StructSerie([("id", I32Serie([1, 2, 3])), ("name", Utf8Serie(["ann", None, "cara"]))])

    # Zero-copy export into pyarrow via the Arrow C Data Interface (PyCapsule protocol).
    arr = pa.array(table)                                  # a StructArray, no payload copy
    assert arr.field(0).to_pylist() == [1, 2, 3]
    assert StructSerie.from_arrow(arr) == table            # ...and back, zero-copy
    assert StructSerie.from_arrow(pa.RecordBatch.from_struct_array(arr)) == table
    ```

=== "Node"

    ```js
    const { StructField, StructSerie, I32Serie, Utf8Serie } = require('yggdryl').types

    // Node has no Arrow-array bridge; interop is the shared byte codec — byte-identical to Rust's.
    const ids = new I32Serie([1, 2, 3])
    const names = new Utf8Serie(['ann', null, 'cara'])
    const schema = new StructField('person', [ids.toField('id'), names.toField('name')], false)
    const table = StructSerie.fromColumns(schema, [ids.serializeBytes(), names.serializeBytes()])
    assert(StructSerie.deserializeBytes(table.serializeBytes()).equals(table))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::var::Utf8Serie;
    use yggdryl_core::io::boxed;
    use yggdryl_core::io::nested::StructSerie;

    let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
    let names = boxed(Utf8Serie::from_strs(&[Some("ann"), Some("bo"), None]));
    let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();

    // A RecordBatch (each field becomes a batch column) and back — byte-exact.
    let batch = table.to_record_batch().unwrap();
    assert_eq!(batch.num_rows(), 3);
    assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), table);

    // Or a (nullable) StructArray, for a struct that has null rows.
    let array = table.to_arrow_array().unwrap();
    ```

## List ↔ ListArray

A `ListSerie` maps to Arrow's `ListArray`: the flattened child mapped by its own `to_arrow_array`, the
offsets as an `OffsetBuffer`, and the top-level validity as a `NullBuffer`. The item field is
non-nullable when the child holds no nulls. Import reads the array's **logical** window, so a *sliced*
`ListArray` converts correctly (the child is windowed and the offsets rebased to `0`).

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.types import ListSerie, I32Serie

    # Rows [[10, 20, 30], [], [40]] over the flat child [10, 20, 30, 40].
    col = ListSerie(I32Serie([10, 20, 30, 40]), [0, 3, 3, 4])

    arr = pa.array(col)                                    # a ListArray, imported zero-copy
    assert pa.types.is_list(arr.type) and arr.type.value_type == pa.int32()
    assert arr.to_pylist() == [[10, 20, 30], [], [40]]

    assert ListSerie.from_arrow(arr) == col                # ...and back, zero-copy
    ```

=== "Node"

    ```js
    const { ListSerie, I32Serie } = require('yggdryl').types

    // Node has no Arrow-array bridge — cross-language interop is the shared serializeBytes wire form.
    const child = new I32Serie([10, 20, 30, 40])
    const col = ListSerie.fromParts(child.toField('item'), child.serializeBytes(), [0, 3, 3, 4])
    assert(ListSerie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::AnySerie;                 // brings `.named` into scope
    use yggdryl_core::io::nested::ListSerie;

    let items = Serie::from_values(&[10i32, 20, 30, 40]).named("item");
    let col = ListSerie::from_values(items, &[0, 3, 3, 4], None).unwrap();

    let array = col.to_arrow_array().unwrap();             // an Arrow ListArray
    let field = col.to_field("scores").to_arrow_field();   // a Field of List type
    let back = ListSerie::from_arrow_array(&array, &field).unwrap();
    assert_eq!(back, col);
    ```

## Map ↔ MapArray

A `MapSerie` maps to Arrow's `MapArray`: the two-column entries struct mapped by
`StructSerie::to_arrow_array`, the offsets as an `OffsetBuffer`, the validity as a `NullBuffer`, and
the `keys_sorted` flag carried through. The non-nullable `entries` field is built from the struct
array's own data type so it matches exactly (Arrow requires `field.data_type() == entries.data_type()`),
and the key field is forced non-null on import (Arrow's "a map key is never null" invariant).

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.types import MapSerie, Utf8Serie, I64Serie

    # Rows {"a": 1, "b": 2}, {"c": 3} over 3 entries.
    col = MapSerie(Utf8Serie(["a", "b", "c"]), I64Serie([1, 2, 3]), [0, 2, 3])

    arr = pa.array(col)                                    # a MapArray, imported zero-copy
    assert isinstance(arr, pa.MapArray)
    assert arr.to_pylist() == [[("a", 1), ("b", 2)], [("c", 3)]]

    assert MapSerie.from_arrow(arr) == col                 # ...and back, zero-copy
    ```

=== "Node"

    ```js
    const { MapSerie, Utf8Serie, I64Serie } = require('yggdryl').types

    // Node has no Arrow-array bridge — cross-language interop is the shared serializeBytes wire form.
    const keys = new Utf8Serie(['a', 'b', 'c'])
    const values = new I64Serie(['1', '2', '3'])           // i64 values cross as strings
    const col = MapSerie.fromParts(
      keys.toField('key'), keys.serializeBytes(),
      values.toField('value'), values.serializeBytes(),
      [0, 2, 3], undefined, false)
    assert(MapSerie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::var::Utf8Serie;
    use yggdryl_core::io::AnySerie;                 // brings `.named` into scope
    use yggdryl_core::io::nested::MapSerie;

    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
    let values = Serie::from_values(&[1i64, 2, 3]).named("value");
    let col = MapSerie::from_entries(keys, values, &[0, 2, 3], None, false).unwrap();

    let array = col.to_arrow_array().unwrap();             // an Arrow MapArray
    let field = col.to_field("counts").to_arrow_field();   // a Field of Map type
    let back = MapSerie::from_arrow_array(&array, &field).unwrap();
    assert_eq!(back, col);
    ```

## The null-row rule

A struct with **null rows** has no `RecordBatch` form (a batch has no top-level validity) — convert
it to a nullable `StructArray` instead; `to_record_batch` returns a **guided error** in that case. A
list or map row can also be null (its top-level `NullBuffer`); that maps to Arrow directly with no
restriction. An Arrow type this crate does not model surfaces the same guided error on import, and a
nested child at a temporal resolution Arrow cannot express (`Minute`…`Year`) makes the whole
`to_arrow_array` a guided error naming the unit (the column still round-trips through the byte codec).

## The Python PyCapsule bridge

The Python extension implements the **Arrow PyCapsule protocol** (`__arrow_c_array__` /
`__arrow_c_schema__`) on every nested column, so a `StructSerie` / `ListSerie` / `MapSerie` hands its
buffers to `pyarrow` with **no payload copy** (shared through the C Data Interface), and
`from_arrow(...)` pulls any C-Data-exposing pyarrow object (a `StructArray`, a `RecordBatch`, a
`ListArray`, a `MapArray`, …) back the same way.

The same `arrow` feature completes the leaf families' array interop too — `Utf8Serie` /
`BinarySerie` ↔ `StringArray` / `BinaryArray`, and the fixed-size byte columns ↔
`FixedSizeBinaryArray` — so every column type converts to and from an Arrow array, at both the typed
and the erased-`AnySerie` level. As elsewhere, Arrow types never appear in a public signature; the
mapping is centralized on `DataTypeId`.
