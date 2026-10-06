# Key series

`KeySerie` holds one key record, its payload rows and their context. `KeySeries`
holds these items; `StreamKeySerie` yields them as the source advances.

## Context and payload

| Accessor | Meaning |
| --- | --- |
| `key_field` / `keyField` | Record field of the key cells |
| `key_paths` / `keyPaths` | Source path per key cell; absent for computed or external keys |
| `serie_field` / `serieField` | Field of the payload rows |
| `field` | Key cells followed by payload children |
| `key` | A `Scalar` record run in key-field order |
| `rows` | The generic `Serie` payload |
| `rownum` | Absolute source start for an adjacent window; absent for partitions or gathered rows |

A bare top-level key moves that child out of the payload. Nested, computed and
external keys add cells while retaining the payload. A key name colliding with a
remaining child needs an alias. Names such as `rownum` and `windownum` are ordinary
user fields. A `KeySerie` has no row length or row iteration; ask its `rows`.

## Windows and partitions

`window_by` / `windowBy` cuts adjacent equal keys. Held inputs answer `KeySeries`
and share payload buffers; sorted held inputs gather stably when necessary.
Streamed inputs answer `StreamKeySerie`, keep a bounded lookahead and lend one
payload at a time. Reading a payload after the walk passed it refuses once.
`sorted=true` on a stream verifies incoming order and refuses the first descent.

`partition_by` / `partitionBy` groups distinct keys. Held inputs answer
`KeySeries`; streamed inputs close partitions according to `PartitionOptions`
(`max_open`, `threads`, `clustered`). Every partition has no source start.
Further clustering retains the outer key context and paths.

Keys take selector text, a `Selector`, `FieldPath` values or typed external native
columns. Construct external value keys explicitly with `Serie::from_scalars`,
Python `Serie.from_` or JavaScript `Serie.from`; an untyped value list is refused.
External streaming keys are refused before their producer is pulled.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};
    let root = Field::new("row", DataType::from(StructType::from_fields([
        Field::new("venue", DataType::utf8(), false),
    Field::new("qty", DataType::Int64, false),
    ])?), false);
    let rows = Serie::from_scalars(root, [
        Scalar::from_sequence([Scalar::from("a"), Scalar::from(1_i64)]),
        Scalar::from_sequence([Scalar::from("a"), Scalar::from(2_i64)]),
        Scalar::from_sequence([Scalar::from("b"), Scalar::from(3_i64)]),
    ])?;
    let groups = rows.window_by("venue as desk", false)?;
    assert_eq!(groups.len(), 2);
    let first = groups.get(0).unwrap();
    assert_eq!(first.rownum(), Some(0));
    assert_eq!(first.rows().len(), 2);
    assert_eq!(first.serie_field().fields()[0].name(), "qty");
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import KeySeries, Serie
    rows = Serie.from_(pa.table({"venue": ["a", "a", "b"], "qty": [1, 2, 3]}))
    groups = rows.window_by("venue as desk")
    assert isinstance(groups, KeySeries)
    first = groups[0]
    assert first.key.as_py() == ["a"]
    assert first.rownum == 0
    assert first.rows.child("qty").as_py() == [1, 2]
    assert len(rows.partition_by("venue")) == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { KeySeries, Serie } = require('yggdryl')
    const rows = Serie.fromArrowBatch(arrow.tableFromArrays({ venue: ['a', 'a', 'b'], qty: [1, 2, 3] }))
    const groups = rows.windowBy('venue as desk')
    assert.ok(groups instanceof KeySeries)
    const first = groups.get(0)
    assert.deepEqual(first.key.asJs(), ['a'])
    assert.equal(first.rownum, 0)
    assert.deepEqual(first.rows.child('qty').asJs(), [1, 2])
    assert.equal(rows.partitionBy('venue').length, 2)
    ```

## Conversion

Every key kind answers `into_stream` / `intoStream` and
`into_chunked_stream` / `intoChunkedStream`. These restore the global record,
with key cells repeated over their payload, and never join across key items.
Every native record-write door accepts key kinds directly.
