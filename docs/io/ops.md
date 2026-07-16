# Numerics & operations

Once data has crossed the typing boundary it lives in a closed, known type space, so the whole
analytics surface — reductions, element-wise arithmetic, reshaping — runs over exact types with
no per-op re-inspection. All of it is defined once in the core and mirrored in both bindings.

## Reductions

Every numeric column (a `Serie<T>` whose element is a `NumericCast` type — the 12 integers and
floats) carries the foundational reductions. A non-numeric column simply doesn't have them.
`mean` / `min` / `max` are `None`/`null` over an empty or all-null column; `sum` is `0`;
`count` is the number of **present** (non-null) elements. `NaN` propagates through `sum`/`mean`
and is skipped by `min`/`max`.

=== "Python"

    ```python
    from yggdryl.types import I64Serie

    col = I64Serie([1, None, 2, 6])
    assert col.count() == 3        # non-null count
    assert col.sum() == 9.0
    assert col.mean() == 3.0
    assert col.min() == 1.0 and col.max() == 6.0
    ```

=== "Node"

    ```js
    const { I64Serie } = require('yggdryl').types

    const col = I64Serie.fromOptions([1n, null, 2n, 6n])
    console.assert(col.count() === 3)
    console.assert(col.sum() === 9)
    console.assert(col.mean() === 3)
    console.assert(col.min() === 1 && col.max() === 6)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::NumericSerie;

    let col = Serie::from_options(&[Some(1i64), None, Some(2), Some(6)]);
    assert_eq!(col.valid_count(), 3);
    assert_eq!(col.sum_f64(), 9.0);
    assert_eq!(col.mean_f64(), Some(3.0));
    assert_eq!(col.min_f64(), Some(1.0));
    assert_eq!(col.max_f64(), Some(6.0));
    ```

For iteration, the core exposes `values()` (a zero-copy raw slice), `iter()` (null-aware
`Option<T>`), and `iter_valid()` (present only) — all allocation-free.

## Vectorized arithmetic

`add` / `sub` / `mul` / `div` / `rem` operate element-wise between two columns, or broadcast a
scalar over one column. Two rules make the result predictable:

- **The result type follows the left operand.** The right operand (a column or a scalar) is
  cast into the left column's element type, range-checked — an out-of-range right is a guided
  error (`i32.add(i64)` → `i32`, `f64.add(i32)` → `f64`).
- **Null propagation & safe division.** A result cell is null if either input is null; integer
  division/remainder by zero yields a **null** cell (never a panic); integer arithmetic **wraps**
  (like Arrow / NumPy); floats are IEEE.

=== "Python"

    ```python
    from yggdryl.types import I64Serie

    a = I64Serie([10, 20, 30])
    b = I64Serie([1, 2, 3])

    assert (a + b).to_options()   == [11, 22, 33]   # operator
    assert a.sub(b).to_options()  == [9, 18, 27]    # named method
    assert (a + 1).to_options()   == [11, 21, 31]   # scalar broadcast
    assert (1 + a).to_options()   == [11, 21, 31]   # commutative reverse
    ```

=== "Node"

    ```js
    const { I64Serie } = require('yggdryl').types

    const a = I64Serie.fromValues([10n, 20n, 30n])
    const b = I64Serie.fromValues([1n, 2n, 3n])

    a.add(b)        // element-wise
    a.sub(b)
    a.add(1)        // scalar broadcast (adds 1 to every element)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::{boxed, AnySerie};

    let a = boxed(Serie::from_values(&[10i64, 20, 30]));
    let b = boxed(Serie::from_values(&[1i64, 2, 3]));

    let sum = a.add(b.as_ref()).unwrap();          // erased, checked -> Box<dyn AnySerie>
    assert_eq!(sum.len(), 3);

    // The fast, infallible path when the inputs are already the same type + length:
    let x = Serie::from_values(&[10i64, 20, 30]);
    let y = Serie::from_values(&[1i64, 2, 3]);
    let z = x.add_unchecked(&y);                   // Serie<i64>
    assert_eq!(z.to_options(), [Some(11), Some(22), Some(33)]);
    ```

The two tiers are a project-wide convention: a **base** op (`add`) that validates + casts and
returns a `Result`, over a fast **`add_unchecked`** that assumes normalized inputs. Nested
columns recurse to their leaves (a struct is combined field-wise, a list element-wise on
matching offsets); temporal columns route through their backing integer, so `date + date` and
`timestamp - timestamp` fall out of the same path.

## Reshape

- **`filter(mask)`** — keep the rows where `mask[i]` is true (bitmap-optimized; the mask length
  must equal the column length). On a nested column, whole rows are kept or dropped.
- **`fill_null(value)`** — replace every null with `value` in one pass, dropping the validity
  mask.
- **`to_struct(name)`** — wrap a column as a one-field struct; **`to_list()`** — wrap each
  element as a singleton list; **`to_map()`** — turn a two-column struct into a map (otherwise
  returns itself).

=== "Python"

    ```python
    from yggdryl.types import I64Serie

    col = I64Serie([10, 20, 30, 40])
    assert col.filter([True, False, True, False]).to_options() == [10, 30]

    holes = I64Serie([1, None, 3])
    assert holes.fill_null(0).to_options() == [1, 0, 3]

    wrapped = col.to_struct("v")   # StructSerie with one field "v"
    ```

=== "Node"

    ```js
    const { I64Serie } = require('yggdryl').types

    const col = I64Serie.fromValues([10n, 20n, 30n, 40n])
    col.filter([true, false, true, false])        // -> [10, 30]
    I64Serie.fromOptions([1n, null, 3n]).fillNull(0)
    col.toStruct('v')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::{boxed, AnySerie};

    let col = boxed(Serie::from_values(&[10i64, 20, 30, 40]));
    let kept = col.filter(&[true, false, true, false]).unwrap();
    assert_eq!(kept.len(), 2);
    ```

## Type inference

When you hold data untyped, `column` infers the tightest column type — the smallest signed
integer that fits, or `f64` for any fractional value, `utf8` for strings, `binary` for bytes;
`None`/`null` makes the column nullable. Pass an explicit `dtype` to override; an ambiguous mix
is a guided error.

=== "Python"

    ```python
    from yggdryl.types import column

    column([1, 2, 3])          # smallest signed int column
    column([1.0, 2, 3])        # f64 column
    column(["a", "b"])         # utf8 column
    column([1, None, 3])       # nullable
    column([1, 2], dtype="i32")
    ```

=== "Node"

    ```js
    const { column } = require('yggdryl').types

    column([1, 2, 3])          // inferred integer column frame
    column(['a', 'b'])         // utf8
    column([1, 2], 'i32')      // explicit dtype
    ```

=== "Rust"

    ```rust
    // In the core you pick the type directly:
    use yggdryl_core::io::fixed::Serie;
    let col = Serie::from_values(&[1i32, 2, 3]);
    let nullable = Serie::from_options(&[Some(1i32), None, Some(3)]);
    ```
