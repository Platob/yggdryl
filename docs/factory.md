# Factory

The `yggdryl.factory` module is a convenience **type-inference** factory for the
bindings: it inspects a native value, infers the matching `yggdryl` data type, and
builds the corresponding scalar, data type or field — so a value crosses without
naming its type.

It is a **binding convenience** (Python and Node), layered over the per-type
factories the model already carries (a data type builds its own scalar and field —
see [`ScalarFactory`](scalar.md) / [`FieldFactory`](field.md)). In Rust you name the
type directly (`Int64Type.scalar(42)`); the bindings add the inferring shortcut.

## From easy to most optimized

The three factory tiers trade convenience for control; pick the lowest tier that
fits and move down as a path gets hot:

1. **Inference (easiest)** — the bindings' `factory.scalar/dtype/field(value)`:
   nothing named, the type inferred from the value (native values, `dict` / plain
   objects, or any yggdryl object). One FFI call; ideal for scripting ergonomics.
2. **Per-type factories (typed)** — the model's own `ScalarFactory` /
   `FieldFactory`: the data type builds its scalar and field
   (`Int64Type.scalar(42)`, `data_type.field("id", False)`) with no inference cost
   and the concrete class statically known.
3. **Direct constructors + zero-copy Arrow (most optimized)** — the concrete
   constructors (`Int64Serie` from a buffer, `RecordScalar` from column series) and
   the `from_arrow` decomposition, which take existing buffers apart and share
   them: reference-count bumps, never element copies.

## Inference

The inference mirrors the model's available types, and — since Python and JS are
the inferring surfaces — accepts **any object of this project** as well as native
values:

| Value | Inferred type |
| --- | --- |
| `int` (Python) / integer `number` or `bigint` (Node) | `int64` |
| `bytes` / `bytearray` (Python) / `Buffer` (Node) | `binary` |
| `None` (Python) / `null` / `undefined` (Node) | `null` |
| a list/array of integers | `int64` serie (empty defaults to it) |
| a `dict` (Python) / plain object (Node) | `struct` — a `RecordScalar` row, each field inferred |
| a yggdryl **scalar** object | its own data type (`scalar()` re-wraps the value) |
| a yggdryl **dtype** object | itself (`scalar()` builds its default scalar) |
| a yggdryl **field** object | its data type |

A value the model has no type for — a `float`, `str`/`string`, `bool`/`boolean`,
an integer outside the `int64` range, or a list of anything but integers — raises
an actionable error. Build those through the explicit per-type factories.
External Arrow objects (pyarrow, Arrow JS) await the Arrow C Data Interface and
are documented future work.

## `scalar` / `dtype` / `field`

Three functions infer from a value: `scalar(value)` builds the matching scalar,
`dtype(value)` the matching data type, and `field(name, value, nullable=True)` the
matching field (keeping the name, nullable by default).

=== "Python"

    ```python
    from yggdryl import factory

    # scalar(value): infer the type and build the scalar.
    assert factory.scalar(42).data_type().name() == "int64"
    assert factory.scalar(b"\x01\x02").data_type().name() == "binary"
    assert factory.scalar(None).is_null()
    assert factory.scalar([1, 2, 3]).to_pylist() == [1, 2, 3]  # int64 serie

    # dtype(value): infer just the data type.
    assert factory.dtype(42).name() == "int64"

    # field(name, value): infer the field, keeping the name (nullable by default).
    scores = factory.field("scores", [1, 2, 3])
    assert (scores.name(), scores.data_type().name()) == ("scores", "list")

    # A value with no model type raises.
    try:
        factory.scalar(1.5)  # float
    except ValueError:
        pass
    ```

=== "Node"

    ```js
    const { factory } = require('yggdryl')

    // scalar(value): infer the type and build the scalar.
    assert.equal(factory.scalar(42).dataType().name(), 'int64')
    assert.equal(factory.scalar(Buffer.from([1, 2])).dataType().name(), 'binary')
    assert.ok(factory.scalar(null).isNull())
    assert.deepEqual(factory.scalar([1, 2, 3]).toArray(), [1n, 2n, 3n]) // int64 serie

    // dtype(value): infer just the data type.
    assert.equal(factory.dtype(42).name(), 'int64')

    // field(name, value): infer the field, keeping the name (nullable by default).
    const scores = factory.field('scores', [1, 2, 3])
    assert.equal(scores.dataType().name(), 'list')

    // A value with no model type throws.
    assert.throws(() => factory.scalar(1.5)) // non-integer number
    ```

=== "Rust"

    ```rust
    // Rust has no inference factory: name the type and use its own factory
    // (`ScalarFactory` / `FieldFactory`), which the bindings' `factory` wraps.
    use yggdryl_scalar::yggdryl_dtype::Int64Type;
    use yggdryl_scalar::{Int64Scalar, ScalarFactory};

    fn main() {
        assert_eq!(Int64Type.scalar(42), Int64Scalar::new(42));
    }
    ```
