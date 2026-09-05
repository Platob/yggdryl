# Structured text

`yggdryl::text` reads and writes JSON, JSON Lines, YAML, and TOML over the shared native `Scalar`; the bindings only translate native and Arrow values.

## Contract

| | |
| --- | --- |
| Owns | one [`Scalar`](../types/scalar.md) tree for every format, Field, Arrow, and binding; records keep deterministic name order; `text/plain` lines are [Media](../media/text.md) |
| Core | parsing, inference, casting, and encoding in Rust; bindings translate native records and Arrow holders |
| Returns | native objects; `cls=Scalar` / `{ scalar: true }` answer the lossless core `Scalar`, whose `get`, `path`, `set`, `remove`, and iteration stay exact wrappers |
| Order | parse, then [placeholder](placeholders.md) substitution, then Field interpretation |
| Content | `&str`, `String`, byte slices, and Python `str` are content, never a path; a path is `pathlib.Path`; destination strings are paths |
| Inference | `inferred_scalar_field` / `inferred_array_field` / `inferred_struct_field`, names `value`, `item`, `row`; one path for every runtime |
| `Format` | `Json`, `JsonLines`, `Yaml`, `Toml`; extension, path, MIME, and content sniff share one vocabulary; sniff tries JSON before YAML; anonymous output is JSON |
| `Limits` | input bytes, nesting, decoded nodes, document count, enforced while streaming; four nullable spellings in both bindings; omitted uses the safe core default |
| Errors | name the format and byte offset, cumulative across documents; readers fuse after the first error |
| Coding | `text::from_io` / `into_io` infer format and coding from the handle `MediaType`, so `quotes.json.gz` is JSON through gzip; `from_io_with_field` types strictly |

## Use

=== "Rust"

    ```rust
    use yggdryl::{Scalar};
    use yggdryl::text::json;

    let quote = json::from_utf8(r#"{"symbol":"AAPL","price":12.5}"#)?;

    assert_eq!(
        quote.get_key_str("symbol").and_then(Scalar::as_utf8),
        Some("AAPL")
    );
    assert_eq!(json::into_utf8(&quote)?, r#"{"price":12.5,"symbol":"AAPL"}"#);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import json

    quote = json.loads('{"symbol":"AAPL","price":12.5}', cls=Scalar)

    assert quote["symbol"].as_utf8() == "AAPL"
    assert quote.path("price").kind == "f64"
    assert quote.set("venue", "XNAS").get("venue").as_utf8() == "XNAS"
    assert quote.as_py() == {"price": 12.5, "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, json } = require('yggdryl')

    const quote = json.loads('{"symbol":"AAPL","price":12.5}', { scalar: true })

    assert.ok(quote instanceof Scalar)
    assert.equal(quote.get('symbol').asUtf8(), 'AAPL')
    assert.equal(quote.path('price').kind, 'f64')
    assert.equal(quote.set('venue', 'XNAS').get('venue').asUtf8(), 'XNAS')
    assert.deepEqual(quote.asJs(), { price: 12.5, symbol: 'AAPL' })
    ```

## Pages

| Page | Purpose |
| --- | --- |
| [JSON](json.md) | JSON and JSON Lines; placeholders refused |
| [YAML](yaml.md) | Syntax-proven natural types, document streams, block or flow style |
| [TOML](toml.md) | One string-key record, syntax-proven temporals, inline tables |
| [Placeholders](placeholders.md) | The Jinja-style `{{ }}` contract for YAML and TOML |

## Typed `Scalar` families

Every `Scalar` is hashable and totally ordered; equal numeric or temporal values share one hash across storage widths.

| family | native variants |
| --- | --- |
| absence and logic | `Null`, `Bool` |
| integers | `I8`, `I16`, `I32`, `I64`, `I128` and unsigned peers |
| floats | `F16`, `F32`, `F64` |
| decimals | `D128(coefficient, scale)`, `D256(coefficient, scale)` |
| text and binary | `String`, `Bytes`, `Geospatial` |
| date and time | `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64` |
| elapsed time | `Duration32`, `Duration64` |
| containers | `Sequence`, `Mapping`, `Record` |

Arithmetic is checked in the Rust value model, both bindings redirect to it, and only unambiguous typed results exist.

| operands | supported operations | result rule |
| --- | --- | --- |
| integers | `+`, `-`, `*`, `/`, `%`, unary `-`, `abs` | keep a shared width; mixed signed/unsigned inputs promote only when lossless |
| floats | `+`, `-`, `*`, `/`, `%`, unary `-`, `abs` | retain the widest float input; mixing an integer uses `F64` |
| exact decimals | `+`, `-`, `*`, `/`, `%`, unary `-`, `abs` | preserve an exact coefficient and scale; an inexact quotient is refused |
| temporal and duration | temporal `+/-` duration, temporal `-` temporal, duration `+/-` duration, duration `*` integer, duration `/` integer | preserve the temporal kind or return an exact duration in the finest required unit |
| null | every binary operation above | propagate `Null` |

Rust has `checked_add`, `checked_sub`, `checked_mul`, `checked_div`, `checked_rem`, `checked_neg`, `checked_abs`, and `Result<Scalar>` operator traits; Python adds operators, JavaScript only the named methods.

=== "Rust"

    ```rust
    use yggdryl::Scalar;

    assert_eq!(
        Scalar::from(-1_i8).checked_add(&Scalar::from(2_u8))?,
        Scalar::from(1_i16),
    );
    assert_eq!(
        Scalar::d128(1, 0).checked_div(&Scalar::d128(2, 0))?,
        Scalar::d128(5, 1),
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar

    assert (Scalar.from_py(40) + 2).as_py() == 42
    assert Scalar.decimal(1, 0).divide(Scalar.decimal(2, 0)) == Scalar.decimal(5, 1)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    assert.equal(Scalar.fromJs(40).add(2).asJs(), 42)
    assert.ok(Scalar.decimal(1n).divide(Scalar.decimal(2n)).equals(Scalar.decimal(5n, 1)))
    ```

| item | rule |
| --- | --- |
| temporal | a `TimeUnit` and a `Timezone`; `Timezone::NAIVE` is the wall-clock marker, never a nullable field; `DateTime64` is the one datetime |
| rows | `Record` is sorted name-to-value input; a Struct `Field` resolves it into one `Sequence` in child-field order; `Mapping` is insertion-ordered with any unique `Scalar` key |
| accessors | `as_bytes`, `as_utf8`, `as_json_bytes` / `as_json_utf8`; native `from_*` / `into_*` [Arrow](../arrow/scalars.md) conversions; read-only `count`, `unit`, `zone`, `unscaled`, `scale` |

## Field-directed parsing

Dumps use ordinary format values; exact values without native syntax become scaled-decimal, base64, or ISO strings. A schemaless read returns only what the grammar proves, so pass a `Field` for exact types.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};
    use yggdryl::text::json;

    let amount = Field::new(
        "amount",
        DataType::decimal128(8, 2)?,
        false,
    );
    let value = json::from_utf8_with_field(r#""12.50""#, &amount)?;

    assert_eq!(value, Scalar::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field, Scalar
    from yggdryl.text import json

    amount = Field("amount", "decimal128(8, 2)", nullable=False)
    value = json.loads('"12.50"', field=amount, cls=Scalar)

    assert value.kind == "d128"
    assert value.unscaled == 1_250
    assert json.loads('"12.50"', field=amount) == Decimal("12.50")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, json } = require('yggdryl')

    const amount = new Field('amount', 'decimal128(8, 2)', false)
    const value = json.loads('"12.50"', { field: amount, scalar: true })

    assert.equal(value.kind, 'd128')
    assert.equal(value.unscaled, 1250n)
    assert.equal(value.scale, 2)
    ```

`field=` requests strict typing; other Python `cls=` targets are dataclass/object materializers with safe wrapper casts. Arrow columns [cast](../types/cast.md) by the same rules in both directions.

## Raw document codecs

| format | documents | natural root |
| --- | --- | --- |
| JSON | one; JSON Lines for many | any JSON value |
| YAML | one or more | any YAML value |
| TOML | exactly one | a string-key record |

`Json`, `JsonLines`, `Yaml`, and `Toml` share one `TextCodec` contract; each format has one inferring entry point that redirects to the explicit form.

| surface | names |
| --- | --- |
| Rust transports | `from_utf8`, `from_bytes`, `from_reader`, `into_utf8`, `into_bytes`, `into_writer`; `_all` for JSON streams, JSON Lines, and YAML documents |
| Rust inferring entry | `from_json_scalar`, `from_json_scalar_with_field`, `into_json_scalar` and the YAML / TOML twins, at the crate root |
| Python | `loads` / `dumps`; `dump(value)` bytes, `dump(value, utf8=True)` text, `dump(value, destination)` writes |
| JavaScript | `loads` / `dumps`; a `Buffer`, or a write to a Node / WHATWG destination |
| generic facade | Python `from_io`, `from_stream`, `into_io`, `into_stream`; JavaScript `from`, `fromStream`, `into`, `intoStream` |

The generic facade infers the format once, then redirects to that implementation. Named sources use their compound suffix; anonymous input is sniffed by the core.

=== "Rust"

    ```rust
    use yggdryl::text::{self, Format};
    use yggdryl::Scalar;

    let (format, value) = text::from_utf8_inferred(r#"{"id":1}"#)?;

    assert_eq!(format, Format::Json);
    assert_eq!(value.get_key_str("id"), Some(&Scalar::from(1_u64)));
    assert_eq!(text::into_utf8(&value, format)?, r#"{"id":1}"#);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import codec

    value = codec.from_io('{"id":1}', cls=Scalar)

    assert isinstance(value, Scalar)
    assert value["id"].kind == "u64"
    assert codec.into_io(value, format="json", utf8=True) == '{"id":1}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, codec } = require('yggdryl')

    const value = codec.from('{"id":1}', { scalar: true })

    assert.ok(value instanceof Scalar)
    assert.equal(value.get('id').kind, 'u64')
    assert.deepEqual(codec.into(value, { format: 'json' }), Buffer.from('{"id":1}'))
    ```

JSON Lines is collection-valued, JSON and TOML return one value, and YAML can stay lazy on the explicit stream path. Field casting and the four decode limits survive redirection.

## Formatting

`Formatting` changes bytes, never meaning; `Indent` is `Default`, `None`, `Spaces(n)`, or `Tabs`.

| format | `Indent` rule |
| --- | --- |
| JSON | compact by default; spaces for pretty output |
| YAML | two-space block style by default; `None` selects flow style |
| TOML | indentation only for nested readability |

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{Scalar};
    use yggdryl::text::json;

    let value = Scalar::from_record([("id", Scalar::from(1_i64))])?;
    let pretty =
        json::into_utf8_with_formatting(&value, Formatting::indented(2))?;

    assert_eq!(pretty, "{\n  \"id\": 1\n}");
    assert_eq!(json::from_utf8(&pretty)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.text import json

    pretty = json.dumps({"id": 1}, indent=2)
    compact = json.dumps({"id": 1}, indent=None)

    assert pretty == b'{\n  "id": 1\n}'
    assert compact == b'{"id":1}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const pretty = json.dumps({ id: 1 }, { indent: 2 })
    const compact = json.dumps({ id: 1 }, { indent: null })

    assert.deepEqual(pretty, Buffer.from('{\n  "id": 1\n}'))
    assert.deepEqual(compact, Buffer.from('{"id":1}'))
    ```

Bindings spell `indent=2` / `{ indent: 2 }`, `None` / `null`, and `"\t"`; the same value carries the [coded handle](../coding/index.md) compression level.

## Edges

- `25:30:00` as a time of day -> `01:30:00`; hours fold modulo the day up to `99`.
- `2026-08-17T24:00:00` as a datetime -> the 18th at midnight.
- `26:03:04`, `P1DT2H3M4S`, `PT93784S` as a duration -> one count; written back as `PT<seconds>S`.
- A time of day or duration with a zone -> refused; both must be naive.
- Empty or positional rows without a `Field` -> ambiguous; an explicit `Field` is required.
- Overflow, division by zero, inexact decimal quotient, undefined operand pair -> four separate core errors.
- `+` on text or containers -> absent; concatenation is not arithmetic.
- `count`, `unit`, `zone`, `unscaled`, `scale` on an unrelated kind -> `None` / `null`.
- Text naming an existing file, given to `from_json_scalar` -> parsed as content, never read.
- `_with_limits` / `_with_formatting` -> explicit form only; the inferring entry point has neither.
- An explicit format that contradicts a suffix -> rejected.
- Duplicate keys, invalid UTF-8, unsupported natural shape, failed Field conversion -> error, never silent coercion.
- `indent` above sixteen spaces -> clamped by the core formatter.
- Whole-value writes -> publish when complete; reader / writer functions stream; [record and listing](../holder/iobase/values.md) APIs stay lazy.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text
    cargo test --features "parquet iceberg" -p yggdryl --test text value::
    cargo test --features "parquet iceberg" -p yggdryl --test text format::
    cargo test --features "parquet iceberg" -p yggdryl --test text structured::
    cargo test --features "parquet iceberg" -p yggdryl --lib text::
    cargo bench -p yggdryl --bench text -- codec/value
    cargo bench -p yggdryl --bench types -- value
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text
    python/.venv/bin/python -m pytest python/tests/text/test_codec_facade.py python/tests/text/test_codec_fields.py python/tests/text/test_codec_native_returns.py python/tests/text/test_codec_options.py
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text
    npm run --prefix node bench:text
    ```

## Performance

### Scalar and Arrow boundary costs

Windows x86_64 release smoke runs, Criterion group `value` in `--bench types` and `python/benchmarks/types/scalars.py`, with conversion setup outside the timed loop.

| Rust core operation | estimate |
| --- | ---: |
| stable hash of a four-field `Record` | 227 ns |
| infer that Record's datatype | 675 ns |
| persistent Record field update | 273 ns |
| restate Date32 days as nanoseconds | 3.10 ns |
| `as_json_bytes` | 2.67 us |
| `as_json_utf8` | 2.67 us |

| CPython release boundary | estimate |
| --- | ---: |
| native Python into / from `Scalar` | 1.66 us / 596 ns |
| stable hash | 232 ns |
| JSON bytes / UTF-8 | 675 ns / 664 ns |
| Arrow scalar into / from `Scalar` | 16.3 us / 6.66 us |
| Arrow array, 4,096 values, into / from `Scalar` | 312 us / 1.76 ms |
| Arrow batch, 4,096 rows, into / from `Scalar` | 1.27 ms / 1.88 ms |
| Arrow table, 4,096 rows, into / from `Scalar` | 1.39 ms / 1.94 ms |

```bash
cargo bench -p yggdryl --bench types -- value
python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
```

### Natural-codec boundary costs

One Windows x86_64 release run, `python/benchmarks/text.py` and `node/benchmarks/text.js`, one fixture per runtime; compare routes within a runtime, never Python against Node.

| CPython operation | JSON | TOML | YAML |
| --- | ---: | ---: | ---: |
| field class encode | 150 us | 140 us | 202 us |
| field class decode | 340 us | 363 us | 387 us |
| bytes decode | 19.5 us | 26.5 us | 47.6 us |
| reader redirect | 26.8 us | 27.9 us | 51.5 us |
| writer redirect | 141 us | 145 us | 200 us |

| Node operation | JSON | TOML | YAML |
| --- | ---: | ---: | ---: |
| natural document decode | 9.37 ms | 14.3 ms | 16.5 ms |
| natural document emit | 18.0 ms | 15.5 ms | 24.3 ms |

```bash
python/.venv/bin/python python/benchmarks/text.py --iterations 10000
npm run --prefix node bench:text
```
