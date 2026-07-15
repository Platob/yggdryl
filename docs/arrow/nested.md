# Arrow interop — nested (struct)

Behind the `arrow` feature, a `StructSerie` (see [Types → Nested](../types/nested.md)) bridges to
Arrow's `StructArray` **and** `RecordBatch`, **recursively** — each leaf converting through one
generic `ArrayData` (buffers + `DataType`) path, so numbers, decimals, temporal, fixed-size bytes,
utf8, and binary all map with no per-type code. A struct column *is* a batch of named columns.

Recomposition is **zero-copy** wherever the underlying leaf is: the fixed and decimal columns hand
back their shared `Arc` buffer through their own `to_arrow_array`.

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

    // Node has no Arrow-array bridge (apache-arrow JS ships no C Data Interface consumer);
    // interop is the shared byte codec — byte-identical to the Rust core's.
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

## The null-row rule

A struct with **null rows** has no `RecordBatch` form (a batch has no top-level validity) — convert
it to a nullable `StructArray` instead; `to_record_batch` returns a **guided error** in that case.
An Arrow type this crate does not model surfaces the same guided error on import.

## The Python PyCapsule bridge

The Python extension implements the **Arrow PyCapsule protocol** (`__arrow_c_array__` /
`__arrow_c_schema__`), so a `StructSerie` hands its columns to `pyarrow` with **no payload copy**
(the child buffers are shared through the C Data Interface), and `StructSerie.from_arrow(...)` pulls
any C-Data-exposing pyarrow object (a `StructArray`, a `RecordBatch`, …) back the same way.

The same `arrow` feature completes the leaf families' array interop too — `Utf8Serie` /
`BinarySerie` ↔ `StringArray` / `BinaryArray`, and the fixed-size byte columns ↔
`FixedSizeBinaryArray` — so every column type converts to and from an Arrow array, at both the typed
and the erased-`AnySerie` level. As elsewhere, Arrow types never appear in a public signature; the
mapping is centralized on `DataTypeId`.
