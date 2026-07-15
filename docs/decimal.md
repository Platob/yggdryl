# Decimals — `d32` / `d64` / `d128` / `d256`

A **scaled decimal** is an integer *coefficient* times a power of ten:
`value = coefficient × 10⁻ˢᶜᵃˡᵉ`. yggdryl ships the four Arrow decimal widths, each backed by a
two's-complement coefficient integer:

| width | value type | coefficient | max precision | Arrow |
| --- | --- | --- | --- | --- |
| `d32`  | `D32`  | `i32`  | 9  | `Decimal32`  |
| `d64`  | `D64`  | `i64`  | 18 | `Decimal64`  |
| `d128` | `D128` | `i128` | 38 | `Decimal128` |
| `d256` | `D256` | `i256` | 76 | `Decimal256` |

They report [`DataTypeCategory::Decimal`](types.md) — signed, numeric, fixed-width — so
`is_decimal()` drills down without a `match`.

The family has **two faces**, tied together by the shared `DecimalBacking` / `DecimalCoeff` traits
(one impl per width):

- The self-describing **value type** `D32`/`D64`/`D128`/`D256` — each value carries its own scale,
  with full checked arithmetic, true numeric ordering, conversions, and a byte codec. This is the
  "native decimal", mirrored in **Python and Node**.
- The **columnar** descriptors `DecimalType` / `DecimalField` / `DecimalScalar` / `DecimalSerie` —
  one `(precision, scale)` fixed per column (Arrow's model), converting **zero-copy** to/from
  Arrow's decimal arrays. Rust core (like the rest of [`io::fixed`](fixed.md)'s value/column layer).

## Constructing and printing

A decimal is a coefficient plus a scale (`12345` at scale `2` is `123.45`), or a parsed literal.

=== "Python"

    ```python
    from yggdryl.decimal import D128

    price = D128(12345, 2)          # 123.45
    assert str(price) == "123.45"
    assert price.coefficient == 12345 and price.scale == 2
    assert price.precision == 5 and price.bits == 128

    assert D128.from_string("-0.005") == D128(-5, 3)
    assert D128.from_float(1.5, 1) == D128(15, 1)
    ```

=== "Node"

    ```js
    const { D128 } = require('yggdryl').decimal

    const price = new D128(12345n, 2)      // 123.45 — coefficient is a bigint
    assert(price.toString() === '123.45')
    assert(price.coefficient === 12345n && price.scale === 2)
    assert(price.precision === 5 && price.bits === 128)

    assert(D128.fromString('-0.005').equals(new D128(-5n, 3)))
    assert(D128.fromFloat(1.5, 1).equals(new D128(15n, 1)))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::D128;
    use std::str::FromStr;

    let price = D128::new(12345, 2).unwrap();   // 123.45
    assert_eq!(price.to_string(), "123.45");
    assert_eq!((price.coefficient(), price.scale()), (Some(12345), 2));
    assert_eq!(D128::from_str("-0.005").unwrap(), D128::new(-5, 3).unwrap());
    ```

## Arithmetic — checked, scale-aligning

Addition and subtraction align scales; multiplication sums them; division takes an explicit result
scale (decimal division rarely terminates). Every operation is **checked**: overflow raises a
guided error (in Rust, the `+`/`-`/`*` operators panic with that message; `checked_*` return it).

=== "Python"

    ```python
    from yggdryl.decimal import D128, D64

    a, b = D128(12345, 2), D128(617, 2)    # 123.45, 6.17
    assert str(a + b) == "129.62"
    assert str(a - b) == "117.28"
    assert str(D64(25, 1) + D64(25, 2)) == "2.75"    # mixed scales align
    assert str(D64(25, 1) * D64(20, 1)) == "5.00"    # scales add
    assert str(-a) == "-123.45" and str(abs(D128(-5, 1))) == "0.5"
    assert str(D128(1, 0).div(D128(3, 0), 4)) == "0.3333"   # explicit result scale

    import pytest
    with pytest.raises(ValueError, match="overflow"):
        _ = D128(2**126, 0) + D128(2**126, 0)
    ```

=== "Node"

    ```js
    const { D128, D64 } = require('yggdryl').decimal

    const a = new D128(12345n, 2), b = new D128(617n, 2)
    assert(a.add(b).toString() === '129.62')
    assert(a.sub(b).toString() === '117.28')
    assert(new D64(25n, 1).add(new D64(25n, 2)).toString() === '2.75')  // scales align
    assert(new D64(25n, 1).mul(new D64(20n, 1)).toString() === '5.00')  // scales add
    assert(a.neg().toString() === '-123.45')
    assert(new D128(1n, 0).div(new D128(3n, 0), 4).toString() === '0.3333')

    assert.throws(() => new D128(2n ** 126n, 0).add(new D128(2n ** 126n, 0)), /overflow/)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::{D128, D64, DecimalError};

    let (a, b) = (D128::new(12345, 2).unwrap(), D128::new(617, 2).unwrap());
    assert_eq!((a + b).to_string(), "129.62");                 // operator panics on overflow
    assert_eq!((D64::new(25, 1).unwrap() * D64::new(20, 1).unwrap()).to_string(), "5.00");
    assert_eq!(a.checked_div(&b, 4).unwrap().to_string(), "20.00"); // checked path returns Result

    let max = D128::new(i128::MAX, 0).unwrap();
    assert!(matches!(
        max.checked_add(&D128::new(1, 0).unwrap()),
        Err(DecimalError::Overflow { .. })
    ));
    ```

## Identity — equal by value, ordered by number

A decimal's identity is its **value**, over a normalized form: `2.5` and `2.50` are equal, hash
equal, and serialize to the same bytes — so a decimal is a first-class dict/set key. Ordering is
true numeric order.

=== "Python"

    ```python
    from yggdryl.decimal import D128, D64

    assert D128(25, 1) == D128(250, 2)                 # 2.5 == 2.50
    assert hash(D128(25, 1)) == hash(D128(250, 2))
    assert len({D128(25, 1), D128(250, 2)}) == 1       # one distinct value
    assert D64(25, 1) < D64(275, 2)                    # 2.5 < 2.75
    ```

=== "Node"

    ```js
    const { D128, D64 } = require('yggdryl').decimal

    assert(new D128(25n, 1).equals(new D128(250n, 2)))          // 2.5 === 2.50
    assert(new D128(25n, 1).hashCode() === new D128(250n, 2).hashCode())
    assert(new D64(25n, 1).compareTo(new D64(275n, 2)) === -1)  // 2.5 < 2.75
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::D128;

    assert_eq!(D128::new(25, 1).unwrap(), D128::new(250, 2).unwrap());   // 2.5 == 2.50
    assert!(D128::new(25, 1).unwrap() < D128::new(275, 2).unwrap());
    // Eq/Hash/serialize_bytes all ride the normalized form, so they never disagree.
    assert_eq!(
        D128::new(25, 1).unwrap().serialize_bytes(),
        D128::new(250, 2).unwrap().serialize_bytes()
    );
    ```

## Conversions, rescaling, and the byte codec

Rescale up losslessly, or down with `round_to_scale` / `trunc_to_scale`; convert to float or exact
integer; cast between widths; and round-trip through the canonical `[scale][coefficient]` bytes.

=== "Python"

    ```python
    from yggdryl.decimal import D64, D32, D128

    assert D128(12300, 2).to_int() == 123           # 123.00 is integral
    assert str(D64(12345, 2).rescale(4)) == "123.4500"
    assert str(D64(12345, 2).round_to_scale(1)) == "123.5"
    assert str(D64(12345, 2).trunc()) == "123"

    wide = D32(12345, 2).to_d128()                  # widen d32 -> d128
    assert str(wide) == "123.45"

    d = D128(-123456789, 4)
    assert D128.deserialize_bytes(d.serialize_bytes()) == d
    ```

=== "Node"

    ```js
    const { D64, D32, D128 } = require('yggdryl').decimal

    assert(new D128(12300n, 2).toInt() === 123n)    // 123.00 is integral
    assert(new D64(12345n, 2).rescale(4).toString() === '123.4500')
    assert(new D64(12345n, 2).roundToScale(1).toString() === '123.5')
    assert(new D64(12345n, 2).trunc().toString() === '123')

    const wide = new D32(12345n, 2).toD128()        // widen d32 -> d128
    assert(wide.toString() === '123.45')

    const d = new D128(-123456789n, 4)
    assert(D128.deserializeBytes(d.serializeBytes()).equals(d))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::{D32, D64, D128, Dec128};

    assert_eq!(D128::new(12300, 2).unwrap().to_i128().unwrap(), 123);
    assert_eq!(D64::new(12345, 2).unwrap().rescale(4).unwrap().to_string(), "123.4500");
    assert_eq!(D32::new(12345, 2).unwrap().cast::<Dec128>().unwrap().to_string(), "123.45");

    let d = D128::new(-123456789, 4).unwrap();
    assert_eq!(D128::deserialize_bytes(&d.serialize_bytes()).unwrap(), d);
    ```

!!! note "Wide coefficients"

    `d256`'s coefficient can exceed 128 bits. It marshals to a native big integer in both bindings
    — a Python `int` and a JS `bigint` — carried through its decimal digits, so `D256(10**60, 5)`
    round-trips its full coefficient. The constructor range-checks in the core, so an out-of-range
    coefficient raises the same guided message in all three languages.

## Native language interop

A decimal coerces to its platform's native numeric types. In Python it round-trips through the
standard library's `decimal.Decimal` and answers `int()` / `float()`; in Node the coefficient and
the truncated integer are `bigint`, and `toFloat()` gives a `number`.

=== "Python"

    ```python
    import decimal
    from yggdryl.decimal import D128

    d = D128(12345, 2)                          # 123.45
    assert int(D128(19, 1)) == 1                # int() truncates toward zero (1.9 -> 1)
    assert float(d) == 123.45

    native = d.to_decimal()                     # -> decimal.Decimal
    assert isinstance(native, decimal.Decimal) and native == decimal.Decimal("123.45")
    assert D128.from_decimal(decimal.Decimal("1.5E+3")) == D128(1500, 0)   # scientific too
    ```

=== "Node"

    ```js
    const { D128 } = require('yggdryl').decimal

    assert(new D128(19n, 1).toBigInt() === 1n)      // truncate toward zero (1.9 -> 1)
    assert(new D128(12345n, 2).toFloat() === 123.45)
    assert(new D128(12345n, 2).coefficient === 12345n)  // native bigint
    assert(D128.fromString('1.5E+3').toString() === '1500')  // scientific notation
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::D128;
    use std::str::FromStr;

    // No native "big decimal" in std — the string and i128 are the interop points.
    assert_eq!(D128::from_str("1.5e3").unwrap().to_string(), "1500"); // scientific notation
    assert_eq!(D128::new(12300, 2).unwrap().to_i128().unwrap(), 123);
    ```

## Columns and Arrow interop (Rust core)

A **column** fixes one `(precision, scale)` for every element (Arrow's model) and stores raw
coefficients, so `DecimalSerie` converts **zero-copy** to/from Arrow's decimal arrays — the
coefficient allocation is shared (an `Arc` bump), and the array carries the column's precision and
scale.

```rust
use yggdryl_core::io::fixed::{D128, D128Serie};
use yggdryl_core::io::SerieType;

let col = D128Serie::from_options(
    20, 2,
    &[Some(D128::new(12345, 2).unwrap()), None, Some(D128::new(600, 2).unwrap())],
).unwrap();
assert_eq!(col.len(), 3);
assert_eq!(col.null_count(), 1);
assert_eq!(col.get(0).unwrap().to_string(), "123.45");

# #[cfg(feature = "arrow")]
# {
let array = col.to_arrow_array();               // zero-copy Decimal128Array
assert_eq!((array.precision(), array.scale()), (20, 2));
assert_eq!(D128Serie::from_arrow_array(&array), col);
# }
```

A `DecimalScalar` / `DecimalSerie` element *is* a value-type `Decimal`, so the two faces compose:
read a value out of a column (`get` / `get_scalar`), do arithmetic on it, and write it back with
`push` (append) or `set` (overwrite index `i`) — plus the bulk `set_range` / `set_scalars` /
`set_values` (see [Typed data — fixed-width](fixed.md#in-place-set-single-and-bulk)). Every write
re-expresses the value at the column's scale/precision with a guided error.

## Design notes

- **Shared decimal traits.** Every width is one impl of `DecimalCoeff` (the coefficient integer +
  its checked arithmetic and little-endian codec) and `DecimalBacking` (the marker tying a width to
  its `DataTypeId`, name, max precision, and Arrow `Decimal*Type`). The value type `Decimal<B>` and
  all four columnar descriptors are generic over `B: DecimalBacking`.
- **Checked, with guided errors.** Overflow, divide-by-zero, and lossy rescales return a
  `DecimalError` whose message names the offending value and the remedy — identical text across
  Rust, Python, and Node. The `+`/`-`/`*` Rust operators panic with that message (like integer
  operators in debug); reach for `checked_*` to handle it as a value.
- **Bit-canonical identity over the normalized form.** `Eq`, `Hash`, and `serialize_bytes` all ride
  the trailing-zero-stripped normalized coefficient, so they can never disagree, while `Ord` is the
  true numeric order. Value arithmetic and identity are stack-only (no per-op allocation, even at
  256 bits — see the [benchmark report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/decimal.md)).
- **Closest-Arrow, losslessly.** `d128`/`d256` map to `Decimal128`/`Decimal256`; on Arrow 56,
  `d32`/`d64` map to the native `Decimal32`/`Decimal64`. An erased `Field` round-trips its
  `(precision, scale)` through reserved metadata keys, since the byte width alone cannot express
  them.
