# Structured text


`yggdryl::text` owns two related surfaces:

- `text/plain` as physical-line records through the ordinary media methods;
- JSON, JSON Lines, YAML, and TOML over the shared native [`Scalar`](types.md).

Parsing, schema inference, casting, and encoding stay in the Rust core. Python
and JavaScript only translate their native record and Arrow holders.


## Shared scalar codecs

### Raw shared-Scalar access

`Scalar` is the one Rust value tree used by JSON, YAML, TOML, Fields, Arrow, and
both extensions. Records use deterministic name order.

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

Bindings return native objects by default. Their explicit `Scalar` wrappers are
the lossless pivot for widths, D256 decimals, exact temporal units, hashing,
and Arrow conversion. Container access stays exact too: indexing, `get`,
`path`, iteration, `keys` / `values` / `items`, and persistent `set` / `remove`
return new or child `Scalar` wrappers instead of converting the whole tree.

#### Typed `Scalar` families

Every `Scalar` is hashable and totally ordered. Equal numeric or temporal values
share one hash even when their storage widths differ; width remains available
for datatype and Arrow projection.

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

Arithmetic is checked in the Rust value model and both bindings redirect to
those same rules. Only operations with an unambiguous typed result exist:

| operands | supported operations | result rule |
| --- | --- | --- |
| integers | `+`, `-`, `*`, `/`, `%`, unary `-`, `abs` | keep a shared width; mixed signed/unsigned inputs promote only when lossless |
| floats | `+`, `-`, `*`, `/`, `%`, unary `-`, `abs` | retain the widest float input; mixing an integer uses `F64` |
| exact decimals | `+`, `-`, `*`, `/`, `%`, unary `-`, `abs` | preserve an exact coefficient and scale; an inexact quotient is refused |
| temporal and duration | temporal `+/-` duration, temporal `-` temporal, duration `+/-` duration, duration `*` integer, duration `/` integer | preserve the temporal kind or return an exact duration in the finest required unit |
| null | every binary operation above | propagate `Null` |

Overflow, division by zero, an inexact decimal quotient, and an undefined
operand pair are separate core errors. Text and containers do not overload
`+`: concatenation is deliberately not arithmetic. Rust exposes
`checked_add` / `checked_sub` / `checked_mul` / `checked_div` /
`checked_rem`, `checked_neg`, and `checked_abs`; its operator traits return a
`Result<Scalar>`. Python provides the normal operators plus the named methods,
and JavaScript provides the named methods because JavaScript cannot overload
operators.

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

Every temporal carries a `TimeUnit` and `Timezone`. `Timezone::NAIVE` is the
explicit marker for a wall-clock reading; zones are never represented by a
nullable field. A time-of-day or duration must be naive. `DateTime64` is the
single instant/wall-clock datetime value.

`Record` is a sorted name-to-value input shape. A Struct `Field` is still the
only row schema: applying it resolves a Record's names and returns one ordered
`Sequence` in child-field order. `Mapping` remains insertion-ordered and may
use any unique `Scalar` as a key.

When no Field is supplied, `Scalar::inferred_scalar_field`,
`inferred_array_field`, and `inferred_struct_field` are the one inference path
used by Rust and both bindings. Their stable names are `value`, `item`, and
`row`; empty or positional rows remain ambiguous and require an explicit Field.

Use `as_bytes` for binary/geospatial values, `as_utf8` for strings, and
`as_json_bytes` / `as_json_utf8` for natural compact JSON. The bindings expose
the same accessors and native `from_*` / `into_*` Arrow conversions. Python and
JavaScript also expose read-only `count`, `unit`, and `zone` temporal parts and
`unscaled` and `scale` decimal parts; unrelated kinds return `None` / `null`,
so inspecting a D256 coefficient never routes through a narrower host value.

#### Scalar and Arrow boundary costs

These release smoke runs measure the accessors above and keep conversion setup
outside the timed loop. They were recorded on Windows x86_64; regenerate them
on the deployment host before comparing releases.

```console
cargo bench -p yggdryl --bench datatype -- value
cd python
.venv/Scripts/python benchmarks/types.py --iterations 10000
```

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

### Field-directed parsing

JSON, YAML, and TOML dumps use ordinary format values, never private Yggdryl
envelopes. Exact values without native syntax use interoperable strings: scaled
decimals, base64 bytes, and ISO temporal text. A schemaless read returns only
what the grammar proves.

An hour past the end of the day is read, not refused, and what it means is
whose day it is. A time of day folds it modulo the day, with hours to `99`, so
`25:30:00` is `01:30:00`. A datetime carries it into the following date, so
`2026-08-17T24:00:00` is the 18th at midnight. A duration keeps it plain and
also reads the clock spelling beside the ISO one, so `26:03:04`, `P1DT2H3M4S`,
and `PT93784S` are one count; minutes and seconds stay under sixty everywhere,
and a duration writes back as `PT<seconds>S`. An Arrow column casts by the same
rules, in both directions: see [casting through a field](types.md#casting-arrow-data-through-a-field).

Pass a `Field` when exact types are required. Parsing happens first, optional
placeholder substitution happens second, and the Field interprets and
canonicalizes the resulting natural value last.

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

`field=` requests native strict typing. Python `cls=Scalar` and JavaScript
`scalar: true` return the resulting core `Scalar` without a natural-language
round trip; omit them for ordinary Python/JavaScript values. Other Python
`cls=` targets remain dataclass/object materializers with safe wrapper casts.

### Raw document codecs

| format | documents | natural root |
| --- | --- | --- |
| JSON | one; JSON Lines for many | any JSON value |
| YAML | one or more | any YAML value |
| TOML | exactly one | a string-key record |

Rust names the transport in `from_utf8`, `from_bytes`, `from_reader`,
`into_utf8`, `into_bytes`, and `into_writer`. `_all` covers JSON streams, JSON
Lines, and YAML documents. `Json`, `JsonLines`, `Yaml`, and `Toml` implement the
same `TextCodec` contract.

Each format also has exactly one inferring entry point that names the `Scalar`
it answers - `from_json_scalar`, `from_json_scalar_with_field` and
`into_json_scalar`, with the YAML and TOML twins - re-exported at the crate root
beside `Scalar`. It coerces at the boundary and redirects to the explicit form:
`&str`, `String`, `&[u8]`, `Vec<u8>` or any other byte-like value is content,
never a path, so text that names an existing file is parsed rather than read,
and a caller who needs `_with_limits` or `_with_formatting` calls the explicit
form. Python `loads(..., cls=Scalar)` with `dumps` and JavaScript
`loads(..., { scalar: true })` with `dumps` are the bindings' one inferring
entry point already.

Python and JavaScript retain the formats' familiar `loads` / `dumps` names.
Python `dump(value)` returns bytes, `dump(value, utf8=True)` returns text, and
`dump(value, destination)` writes directly. JavaScript returns `Buffer` or
writes to a supplied Node/WHATWG destination.

When the format is not known at the call site, the generic facade performs one
inference and then redirects to that same format implementation. Named sources
and destinations use their compound suffix; anonymous input is sniffed by the
Rust core; anonymous output defaults to JSON. An explicit format that
contradicts a suffix is rejected.

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

Python names its generic operations `from_io`, `from_stream`, `into_io`, and
`into_stream`; JavaScript uses `from`, `fromStream`, `into`, and `intoStream`.
JSON Lines is collection-valued, while JSON and TOML return one value and YAML
can remain lazy on the explicit stream path. Field casting and all four nullable
decode limits remain core operations after redirection.

#### Natural-codec boundary costs

One Windows x86_64 release run used the same eight-leg field-class document
for every Python format and the same larger natural document for every Node
format. These fixtures compare routes within a runtime, not Python against
Node. Regenerate with `python benchmarks/text.py --iterations 10000` and
`npm run bench:codec`.

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

### Formatting

`Formatting` changes bytes, never meaning. Its `Indent` is `Default`, `None`,
`Spaces(n)`, or `Tabs`:

- JSON defaults to compact and uses spaces for pretty output;
- YAML defaults to two-space block style; `None` selects flow style;
- TOML uses indentation only for nested readability.

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

Python and JavaScript use `indent=2` / `{ indent: 2 }`, `None` / `null` for
compact or flow layout, and `"\t"` for tabs. Omission keeps each format's core
default; space counts above sixteen are clamped by that same core formatter.

The same value also carries the compression level used when a generic dump is
redirected through a coded handle.

<a id="jinja-style-placeholders"></a>

### Placeholders

YAML and TOML may resolve placeholders in string values. JSON refuses the
feature because it is an interchange format. Substitution is off unless a
mapping or the environment switch is supplied.

| form | meaning |
| --- | --- |
| `{{ NAME }}` | resolve `NAME`; absence is an error |
| `{{ NAME \| default(LITERAL) }}` | use a JSON-scalar fallback |
| `{{{{` | emit a literal `{{` |

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Scalar;

    let loading = Loading::new().with_placeholders(
        Placeholders::new().with_variable("HOST", Scalar::from("db.internal")),
    );
    let value = yggdryl::text::from_utf8_with(
        "host: \"{{ HOST }}\"\nport: \"{{ PORT | default(8080) }}\"\n",
        Format::Yaml,
        &loading,
    )?;

    assert_eq!(value.get_key_str("host").and_then(Scalar::as_utf8), Some("db.internal"));
    assert_eq!(value.get_key_str("port"), Some(&Scalar::from(8080_i64)));
    ```

=== "Python"

    ```python
    from yggdryl.text import yaml

    document = 'host: "{{ HOST }}"\nport: "{{ PORT | default(8080) }}"\n'
    value = yaml.loads(document, placeholders={"HOST": "db.internal"})

    assert value == {"host": "db.internal", "port": 8080}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const document = 'host: "{{ HOST }}"\nport: "{{ PORT | default(8080) }}"\n'
    const value = yaml.loads(document, {
      placeholders: { HOST: 'db.internal' },
    })

    assert.deepEqual(value, { host: 'db.internal', port: 8080 })
    ```

The document is parsed before substitution, so placeholders never create keys,
containers, or syntax. Quote a placeholder where the grammar would otherwise
interpret its braces structurally. A supplied mapping wins over the process
environment, and the environment is never read unless `environment=True`.
Resolved secrets become ordinary values and can leak if subsequently dumped.

Field interpretation runs after substitution. A placeholder can therefore
supply the natural string that a decimal, binary, or temporal Field consumes.

The raw bytes are scanned once for `{{`. When none is present, no value walk
runs. This measured 256-entry YAML documents with the feature off and on
(containerized x86_64 Linux, Criterion medians with 95% intervals):

```text
codec/placeholder/none/off  272.81 us   [271.30 us 274.52 us]
codec/placeholder/none/on   266.07 us   [265.12 us 267.21 us]
codec/placeholder/few/off   265.58 us   [264.58 us 266.86 us]
codec/placeholder/few/on    327.80 us   [325.00 us 330.56 us]
codec/placeholder/most/off  264.84 us   [262.10 us 268.46 us]
codec/placeholder/most/on   386.80 us   [384.48 us 389.17 us]
```

The no-placeholder guard is within run noise. Substitution work was about
0.5 us per rebuilt scalar in that run. Dumping writes resolved values and never
reintroduces placeholders.

### Limits and errors

`Limits` bounds input bytes, nesting, decoded nodes, and document count.
Readers enforce the bounds while streaming and fuse after the first error.
Both bindings expose nullable spellings of all four limits; omitted values use
the same safe core defaults.

Codec errors name the format and byte offset. Offsets are cumulative for
multi-document readers, so failures remain locatable in the original input.
Duplicate keys, invalid UTF-8, unsupported natural shapes, and Field conversion
fail rather than silently coercing a value.

### `IOBase` and content coding

`Format` has `Json`, `JsonLines`, `Yaml`, and `Toml`; extension, path, and MIME
inference share one vocabulary. Content inference tries JSON before YAML
because most JSON is valid YAML. In Python a `str` is document content, so use
`pathlib.Path` for a path; destination strings are unambiguous paths.

`text::from_io` and `text::into_io` infer both format and content coding from a
handle's `MediaType`. Thus `quotes.json.gz` parses JSON through gzip without a
format or coding argument. `from_io_with_field` applies the same strict typing
as in-memory loaders. Whole-value writes publish when complete; reader/writer
functions stream directly and leave record and listing APIs lazy.

## JSON


JSON uses ordinary interoperable documents backed by the shared Rust codec.

### Text media and Arrow batches

A JSON document is a raw structured value, not the line-record media surface.
For streamed Arrow batches with explicit overwrite and append behavior, use
[Text media](media.md#plain-text-records). Text deliberately refuses
keyed merge because a line has no stable row identity. JSON Lines is the
multi-value document stream described below.

### Raw shared-Scalar access

Rust returns `Scalar`; Python and JavaScript project the same tree into native
objects. Dumps produce JSON accepted by other implementations, and loads return
only types the JSON grammar proves.

=== "Rust"

    ```rust
    use yggdryl::{Scalar};
    use yggdryl::text::json;

    let value = json::from_utf8(r#"{"symbol":"AAPL","quantity":100}"#)?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_utf8),
        Some("AAPL")
    );
    assert_eq!(
        json::into_utf8(&value)?,
        r#"{"quantity":100,"symbol":"AAPL"}"#
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import json

    natural = json.loads('{"symbol":"AAPL","quantity":100}')
    value = json.loads('{"symbol":"AAPL","quantity":100}', cls=Scalar)

    assert value.kind == "record"
    assert value.as_py() == natural == {"quantity": 100, "symbol": "AAPL"}
    assert json.dumps(value) == b'{"quantity":100,"symbol":"AAPL"}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, json } = require('yggdryl')

    const natural = json.loads('{"symbol":"AAPL","quantity":100}')
    const value = json.loads('{"symbol":"AAPL","quantity":100}', { scalar: true })
    const encoded = json.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'record')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(json.loads(encoded), natural)
    ```

Objects become name-sorted `Record` values, arrays become `Sequence`, and
numbers use the narrowest exact natural family available to the parser.
Duplicate object names are rejected. Python `cls=Scalar` and JavaScript
`{ scalar: true }` return that exact native tree directly; omitting the selector
keeps the existing natural Python/JavaScript result.

#### One inferring entry point

`yggdryl::from_json_scalar`, `from_json_scalar_with_field` and
`into_json_scalar` are JSON's [inferring entry points](text.md#raw-document-codecs)
over `from_bytes`, `from_bytes_with_field` and `into_utf8`. Text that names an
existing file is parsed as JSON, so a bare file name fails as invalid syntax
rather than being read.

=== "Rust"

    ```rust
    use yggdryl::{from_json_scalar, into_json_scalar};

    let value = from_json_scalar(r#"{"symbol":"AAPL","quantity":100}"#)?;
    let encoded = into_json_scalar(&value)?;

    assert_eq!(encoded, r#"{"quantity":100,"symbol":"AAPL"}"#);
    assert_eq!(from_json_scalar(encoded.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import json

    value = json.loads('{"symbol":"AAPL","quantity":100}', cls=Scalar)
    encoded = json.dumps(value)

    assert encoded == b'{"quantity":100,"symbol":"AAPL"}'
    assert json.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const value = json.loads('{"symbol":"AAPL","quantity":100}', { scalar: true })
    const encoded = json.dumps(value)

    assert.equal(encoded.toString(), '{"quantity":100,"symbol":"AAPL"}')
    assert.ok(json.loads(encoded, { scalar: true }).equals(value))
    ```

### Natural values and exact Fields

JSON has syntax for null, booleans, finite numbers, strings, arrays, and
string-key objects. Other native values use interoperable scalar spellings:

| native value | natural JSON |
| --- | --- |
| `D128`, `D256` | scale-preserving string |
| `Bytes`, `Geospatial` | base64 string |
| date, time, `DateTime64`, duration | ISO string when representable |
| non-finite float | error |
| Mapping with non-string keys | error |

There is no private marker envelope. Consequently a schemaless reader sees
`"12.50"`, `"AP8="`, and `"2026-08-15T10:30:00Z"` as strings. Pass a native
[`Field`](types.md) when those spellings must recover exact types:

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};
    use yggdryl::text::json;

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    let decoded = json::from_utf8_with_field(r#""12.50""#, &amount)?;

    assert_eq!(decoded, Scalar::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field
    from yggdryl.text import json

    amount = Field("amount", "decimal128(8, 2)", nullable=False)

    assert json.loads('"12.50"', field=amount) == Decimal("12.50")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, json } = require('yggdryl')

    const amount = new Field('amount', 'decimal128(8, 2)', false)
    const decoded = json.loads('"12.50"', { field: amount })

    assert.equal(decoded.kind, 'd128')
    assert.equal(decoded.unscaled, 1250n)
    ```

A Struct Field resolves object names into one ordered row `Sequence` in Rust.
Python and JavaScript restore those names as a dictionary or object; Python
may also pass `cls=SomeDataclass` to materialize the already-decoded row.

### Documents and streams

Rust names each transport in the method:

- `from_utf8`, `from_bytes`, and `from_reader` decode one document;
- `into_utf8`, `into_bytes`, and `into_writer` encode one document;
- `_with_field` applies exact typing;
- `_all` consumes whitespace-separated JSON values;
- `from_lines_*` and the `JsonLines` format require one value per non-empty
  line.

Borrowed reader iterators yield one `Result<Scalar>` at a time and fuse after
the first error. Writers stream directly to `Write`. Python and JavaScript keep
the conventional `loads` / `dumps` names and leave caller-owned streams open.

=== "Rust"

    ```rust
    use yggdryl::text::json;

    let rows = json::from_lines_utf8("{\"id\":1}\n{\"id\":2}\n")?;
    let mut destination = Vec::new();
    json::into_writer_all(&rows, &mut destination)?;

    assert_eq!(rows.len(), 2);
    assert_eq!(destination, b"{\"id\":1}\n{\"id\":2}\n");
    ```

=== "Python"

    ```python
    import io

    from yggdryl.text import json

    rows = list(json.loads_all('{"id":1}\n{"id":2}\n'))
    destination = io.BytesIO()
    json.dump_all(rows, destination)

    assert rows == [{"id": 1}, {"id": 2}]
    assert destination.getvalue() == b'{"id":1}\n{"id":2}\n'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const rows = json.loadsAll('{"id":1}\n{"id":2}\n')
    const encoded = json.dumpAll(rows)

    assert.deepEqual(rows, [{ id: 1 }, { id: 2 }])
    assert.deepEqual(json.loadsAll(encoded), rows)
    ```

`loads_all` decodes held content; `load_all` lazily pulls a path or readable
stream. A malformed JSON Lines row reports its byte offset in the original
input, including preceding lines.

### Formatting

JSON defaults to compact output. Rust `Formatting::indented(n)` adds spaces and
newlines; `Formatting::compact()` explicitly requests no layout. Formatting
never changes the parsed value.

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

    pretty = json.dumps({"child": {"id": 1}}, indent=2)
    compact = json.dumps({"child": {"id": 1}}, indent=None)

    assert b'\n  "child"' in pretty
    assert compact == b'{"child":{"id":1}}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const pretty = json.dumps({ child: { id: 1 } }, { indent: 2 })
    const compact = json.dumps({ child: { id: 1 } }, { indent: null })

    assert.ok(pretty.includes(Buffer.from('\n  "child"')))
    assert.deepEqual(compact, Buffer.from('{"child":{"id":1}}'))
    ```

Keys are emitted in deterministic Record order. A general `Mapping` keeps its
insertion order but is writable only when every key is a string.

### Placeholders

JSON rejects placeholder options. Configuration substitution is available for
YAML and TOML, while JSON remains an interchange format whose parsed value is
determined only by its bytes and optional Field.

### Limits and errors

Decode options expose the same nullable core limits in every language:
`max_depth` / `maxDepth`, input bytes, decoded nodes, and document count.
Omitted values use the safe core defaults. The parser also keeps a hard nesting
ceiling, so an adversarial caller cannot request a stack-exhausting depth.

Errors name JSON and the byte offset: invalid UTF-8, duplicate keys, trailing
data in a one-document call, a malformed JSON Lines row, non-finite output, and
Field conversion all fail at the boundary.

### `IOBase` and content coding

Generic structured-text I/O derives JSON and any outer coding from the
handle's `MediaType`. A `quotes.json.gz` handle therefore decodes gzip and JSON
without a format or compression argument. The same plan writes and publishes
the complete value.

Python accepts `PathLike` sources and destinations. JavaScript accepts path
strings, file descriptors, file URLs, and streams.

## YAML


YAML supports one document or a document stream, using ordinary YAML without
private Yggdryl tags.

### Text media and Arrow batches

A YAML document stream carries raw structured values, not line-record media.
For streamed Arrow batches with explicit overwrite and append behavior, use
[Text media](media.md#plain-text-records). Text deliberately refuses
keyed merge because a line has no stable row identity.

### Raw shared-Scalar access

Rust returns the shared `Scalar`; Python and JavaScript project it into native
objects through the same codec.

=== "Rust"

    ```rust
    use yggdryl::{Scalar};
    use yggdryl::text::yaml;

    let value = yaml::from_utf8("symbol: AAPL\nquantity: 2\n")?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_utf8),
        Some("AAPL")
    );
    assert_eq!(yaml::into_utf8(&value)?, "quantity: 2\nsymbol: AAPL\n");
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import yaml

    natural = yaml.loads("symbol: AAPL\nquantity: 2\n")
    value = yaml.loads("symbol: AAPL\nquantity: 2\n", cls=Scalar)

    assert value.kind == "record"
    assert value.as_py() == natural == {"quantity": 2, "symbol": "AAPL"}
    assert yaml.dumps(value) == b"quantity: 2\nsymbol: AAPL\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, yaml } = require('yggdryl')

    const natural = yaml.loads('symbol: AAPL\nquantity: 2\n')
    const value = yaml.loads('symbol: AAPL\nquantity: 2\n', { scalar: true })
    const encoded = yaml.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'record')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(yaml.loads(encoded), natural)
    ```

Mappings with string names become sorted `Record` values. YAML mappings with
other keys remain `Mapping` and preserve insertion order. Duplicate keys are
rejected. Python `cls=Scalar` and JavaScript `{ scalar: true }` return the exact
native tree; omitting the selector keeps natural language objects.

#### One inferring entry point

`yggdryl::from_yaml_scalar`, `from_yaml_scalar_with_field` and
`into_yaml_scalar` are YAML's [inferring entry points](text.md#raw-document-codecs)
over `from_bytes`, `from_bytes_with_field` and `into_utf8`. Text that names an
existing file is parsed as YAML, so a bare file name is that plain string
scalar rather than the file's content.

=== "Rust"

    ```rust
    use yggdryl::{from_yaml_scalar, into_yaml_scalar};

    let value = from_yaml_scalar("symbol: AAPL\nquantity: 2\n")?;
    let encoded = into_yaml_scalar(&value)?;

    assert_eq!(encoded, "quantity: 2\nsymbol: AAPL\n");
    assert_eq!(from_yaml_scalar(encoded.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import yaml

    value = yaml.loads("symbol: AAPL\nquantity: 2\n", cls=Scalar)
    encoded = yaml.dumps(value)

    assert encoded == b"quantity: 2\nsymbol: AAPL\n"
    assert yaml.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const value = yaml.loads('symbol: AAPL\nquantity: 2\n', { scalar: true })
    const encoded = yaml.dumps(value)

    assert.equal(encoded.toString(), 'quantity: 2\nsymbol: AAPL\n')
    assert.ok(yaml.loads(encoded, { scalar: true }).equals(value))
    ```

### Natural values and exact Fields

Schemaless reads preserve only types proven by YAML syntax: null, boolean,
integer, float, string, sequence, mapping, and standard binary. Unknown custom
tags do not create private runtime classes; the tagged scalar or collection is
read by its natural shape.

Exact native values dump as interoperable YAML:

| native value | natural YAML |
| --- | --- |
| `D128`, `D256` | quoted scale-preserving string |
| `Bytes`, `Geospatial` | standard `!!binary` base64 |
| date, time, `DateTime64`, duration | ISO scalar when representable |
| `F16`, `F32`, `F64` | YAML float, including non-finite values |

`!!binary` is a YAML standard tag, not a private marker envelope. No private
tag is written. A plain quoted `"AP8="` therefore stays a string unless a
[`Field`](types.md) declares it binary.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};
    use yggdryl::text::yaml;

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    let decoded = yaml::from_utf8_with_field("'12.50'\n", &amount)?;

    assert_eq!(decoded, Scalar::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field
    from yggdryl.text import yaml

    amount = Field("amount", "decimal128(8, 2)", nullable=False)

    assert yaml.loads("'12.50'\n", field=amount) == Decimal("12.50")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, yaml } = require('yggdryl')

    const amount = new Field('amount', 'decimal128(8, 2)', false)
    const decoded = yaml.loads("'12.50'\n", { field: amount })

    assert.equal(decoded.kind, 'd128')
    assert.equal(decoded.scale, 2)
    ```

A Struct Field resolves record names into its child order and returns a row
`Sequence` in Rust; Python and JavaScript restore a dictionary or object at
their boundary. Malformed exact values and missing required children fail.

### Documents and streams

Rust `from_utf8`, `from_bytes`, and `from_reader` require exactly one YAML
document. Their `_all` forms return every document, and
`from_reader_iter[_with_field]` yields lazily and fuses after the first error.
`into_writer_all` emits a YAML document stream.

Python and JavaScript use `loads` / `dumps` for one document and
`loads_all` / `dump_all` for a stream. Python `load_all` and JavaScript
`loadAll` keep readable streams lazy.

=== "Rust"

    ```rust
    use yggdryl::text::yaml;

    let documents = yaml::from_utf8_all("id: 1\n---\nid: 2\n")?;
    let mut destination = Vec::new();
    yaml::into_writer_all(&documents, &mut destination)?;

    assert_eq!(documents.len(), 2);
    assert_eq!(yaml::from_bytes_all(&destination)?, documents);
    ```

=== "Python"

    ```python
    import io

    from yggdryl.text import yaml

    documents = list(yaml.load_all(io.BytesIO(b"id: 1\n---\nid: 2\n")))

    assert documents == [{"id": 1}, {"id": 2}]
    assert yaml.dumps_all(documents) == b"id: 1\n---\nid: 2\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const documents = yaml.loadsAll('id: 1\n---\nid: 2\n')
    const encoded = yaml.dumpAll(documents)

    assert.deepEqual(documents, [{ id: 1 }, { id: 2 }])
    assert.deepEqual(yaml.loadsAll(encoded), documents)
    ```

Each error carries both the document start and the failing byte offset in the
whole source. After failure, the iterator is exhausted.

### Formatting

YAML defaults to two-space block style. `Formatting::indented(n)` changes the
block width; `Formatting::compact()` selects flow style. Layout changes bytes,
never meaning.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{Scalar};
    use yggdryl::text::yaml;

    let value = Scalar::from_record([("id", Scalar::from(1_i64))])?;
    let flow =
        yaml::into_utf8_with_formatting(&value, Formatting::compact())?;

    assert_eq!(flow, "{id: 1}\n");
    assert_eq!(yaml::from_utf8(&flow)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.text import yaml

    value = {"child": {"id": 1}}
    laid_out = yaml.dumps(value, indent=4)
    flow = yaml.dumps(value, indent=None)

    assert b"\n    id:" in laid_out
    assert flow.startswith(b"{")
    assert yaml.loads(laid_out) == yaml.loads(flow) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const value = { child: { id: 1 } }
    const laidOut = yaml.dumps(value, { indent: 4 })
    const flow = yaml.dumps(value, { indent: null })

    assert.ok(laidOut.includes(Buffer.from('\n    id:')))
    assert.equal(flow[0], '{'.charCodeAt(0))
    assert.deepEqual(yaml.loads(laidOut), yaml.loads(flow))
    ```

The writer quotes scalars when their plain spelling would change type or
structure. Deterministic Record order makes repeated dumps byte-identical.

### Placeholders

Placeholder substitution is opt-in and runs after YAML parsing. It changes
string values only, never keys or document structure. Quote placeholders:
unquoted braces are YAML flow-mapping syntax.

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Scalar;

    let loading = Loading::new().with_placeholders(
        Placeholders::new().with_variable("PORT", Scalar::from(8080_i64)),
    );
    let value = yggdryl::text::from_utf8_with(
        "port: \"{{ PORT }}\"\n",
        Format::Yaml,
        &loading,
    )?;

    assert_eq!(value.get_key_str("port"), Some(&Scalar::from(8080_i64)));
    ```

=== "Python"

    ```python
    from yggdryl.text import yaml

    options = {"placeholders": {"PORT": 8080}}

    assert yaml.loads('port: "{{ PORT }}"\n', **options) == {"port": 8080}
    assert isinstance(yaml.loads("port: {{ PORT }}\n", **options)["port"], dict)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { yaml } = require('yggdryl')

    const options = { placeholders: { PORT: 8080 } }

    assert.deepEqual(yaml.loads('port: "{{ PORT }}"\n', options), { port: 8080 })
    ```

The supplied mapping wins over the environment. Environment lookup is a
separate switch and remains off by default. Field interpretation runs after
substitution, so a resolved natural string can become an exact typed value.
See [structured text](text.md#jinja-style-placeholders) for syntax, security,
and measured overhead.

### Limits and errors

Nullable binding options expose the same byte, depth, decoded-node, and
document limits as Rust (`max_input_bytes` / `maxInputBytes`, and the matching
names for the other three). They apply equally to held input and streams;
omission uses the core defaults. Invalid UTF-8, duplicate keys, malformed
syntax, exhaustion, and Field conversion report YAML plus a byte offset.

### `IOBase` and content coding

Generic `from_io` / `into_io` infer YAML and outer compression from the
handle's media type, so `.yaml.gz` is one transparent operation. Python accepts
`PathLike` sources and destinations; JavaScript accepts paths, file
descriptors, file URLs, and streams.

## TOML


TOML is one natural record document backed by the shared Rust codec.

### Text media and Arrow batches

A TOML document is a raw structured value, not line-record media. For streamed
Arrow batches with explicit overwrite and append behavior, use [Text
media](media.md#plain-text-records). Text deliberately refuses keyed
merge because a line has no stable row identity.

### Raw shared-Scalar access

Rust returns `Scalar`; Python and JavaScript redirect native mappings through
the same codec.

=== "Rust"

    ```rust
    use yggdryl::{Scalar};
    use yggdryl::text::toml;

    let value = toml::from_utf8(
        "title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n"
    )?;
    let encoded = toml::into_utf8(&value)?;

    assert_eq!(
        value.get_key_str("title").and_then(Scalar::as_utf8),
        Some("yggdryl")
    );
    assert_eq!(toml::from_utf8(&encoded)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import toml

    source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    natural = toml.loads(source)
    value = toml.loads(source, cls=Scalar)

    assert value.kind == "record"
    assert value.as_py() == natural == {
        "count": 3,
        "owner": {"name": "Ada"},
        "title": "yggdryl",
    }
    assert toml.loads(toml.dumps(value)) == natural
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, toml } = require('yggdryl')

    const source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    const natural = toml.loads(source)
    const value = toml.loads(source, { scalar: true })
    const encoded = toml.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'record')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(toml.loads(encoded), natural)
    ```

Tables become sorted `Record` values. Empty or comment-only TOML is an empty
Record. Python `cls=Scalar` and JavaScript `{ scalar: true }` expose that exact
tree; omitting the selector returns natural mappings. Deterministic order makes
repeated writes byte-identical.

#### One inferring entry point

`yggdryl::from_toml_scalar`, `from_toml_scalar_with_field` and
`into_toml_scalar` are TOML's [inferring entry points](text.md#raw-document-codecs)
over `from_bytes`, `from_bytes_with_field` and `into_utf8`; the answer is a
`Record` because a TOML root is a table. Text that names an existing file is
parsed as TOML, so a bare file name fails as the bare word it is.

=== "Rust"

    ```rust
    use yggdryl::{from_toml_scalar, into_toml_scalar, Scalar};

    let value = from_toml_scalar(
        "title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n"
    )?;
    let encoded = into_toml_scalar(&value)?;

    assert_eq!(from_toml_scalar(encoded.as_bytes())?, value);
    assert_eq!(
        value.get_key_str("title").and_then(Scalar::as_utf8),
        Some("yggdryl")
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import toml

    source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    value = toml.loads(source, cls=Scalar)
    encoded = toml.dumps(value)

    assert toml.loads(encoded, cls=Scalar) == value
    assert value.as_py()["title"] == "yggdryl"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    const value = toml.loads(source, { scalar: true })
    const encoded = toml.dumps(value)

    assert.ok(toml.loads(encoded, { scalar: true }).equals(value))
    assert.equal(value.asJs().title, 'yggdryl')
    ```

### Natural values and exact Fields

TOML proves strings, signed 64-bit integers, 64-bit floats, booleans, arrays,
tables, and its four date/time forms. It has no null, arbitrary-key mapping,
scalar document root, or multi-document stream.

| input or native value | natural TOML behavior |
| --- | --- |
| table / inline table | sorted `Record` |
| date | `Date32` |
| local time | `Time32` or `Time64` |
| local or offset date-time | `DateTime64` with explicit timezone |
| `D128`, `D256` | quoted scale-preserving string |
| bytes / geospatial | quoted base64 string |
| duration | quoted ISO duration |
| null | error |
| integer outside `i64` | error |
| Mapping with non-string keys | error |

There is no private marker envelope or private tagged table. A user key with that
same spelling is ordinary application data. Scalars TOML cannot spell are rejected
before a destination is opened, not encoded into a hidden side format.

TOML's own date/time syntax produces temporal Scalars without a schema. Exact
decimal width and scale, binary, string-encoded temporal values, and Struct
child order require a [`Field`](types.md).

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar};
    use yggdryl::text::toml;

    let amount = Field::new("amount", DataType::decimal128(8, 2)?, false);
    let row = Field::new(
        "row",
        DataType::from_fields([amount])?,
        false,
    );
    let decoded = toml::from_utf8_with_field("amount = '12.50'\n", &row)?;

    assert_eq!(decoded.as_sequence().unwrap()[0], Scalar::d128(1_250, 2));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field, types
    from yggdryl.text import toml

    row = types.struct(
        "row",
        [Field("amount", "decimal128(8, 2)", nullable=False)],
        nullable=False,
    )

    assert toml.loads("amount = '12.50'\n", field=row) == {
        "amount": Decimal("12.50")
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields, toml } = require('yggdryl')

    const row = fields.struct(
      'row',
      [fields.decimal128('amount', 8, 2, { nullable: false })],
      { nullable: false },
    )
    const decoded = toml.loads("amount = '12.50'\n", { field: row })

    assert.equal(decoded.amount.kind, 'd128')
    assert.equal(decoded.amount.unscaled, 1250n)
    ```

A Struct Field canonicalizes its Record input into a row `Sequence` in Rust.
Python and JavaScript restore the field names; Python can also pass both
`field=` and `cls=YourDataclass` to materialize its dataclass wrapper.

#### Dates and times

Every native temporal carries a `TimeUnit` and non-null `Timezone`.
`Timezone::NAIVE` marks a local wall-clock value. TOML offset date-times become
`DateTime64` instants; local date-times use the same variant with `NAIVE`.
Date and time-of-day values are zone-free.

Native values use TOML date/time tokens when their unit and range fit the TOML
grammar. Otherwise they use their natural ISO string or count, which a Field
can interpret exactly. Named zones that TOML cannot express remain ISO strings
instead of being silently rewritten to the zone's current offset.

Bindings use their closest native temporal where it is lossless. The explicit
`Scalar` wrapper retains resolutions and zone semantics the host language
cannot represent.

### Documents and streams

Rust `from_utf8`, `from_bytes`, and `from_reader` decode one TOML document;
`into_utf8`, `into_bytes`, and `into_writer` encode one. The `_all` forms exist
only for generic dispatch and require exactly one value.

Python and JavaScript intentionally do not expose `loads_all`, `dump_all`, or
multi-document streams for TOML. Their `loads` accepts content, paths, and
readers; `dump` returns bytes/text or writes directly.

=== "Rust"

    ```rust
    use yggdryl::text::toml;

    let value = toml::from_utf8("id = 1\n")?;
    let mut destination = Vec::new();
    toml::into_writer(&value, &mut destination)?;

    assert_eq!(toml::from_bytes(&destination)?, value);
    ```

=== "Python"

    ```python
    import io

    from yggdryl.text import toml

    destination = io.BytesIO()
    toml.dump({"id": 1}, destination)

    assert toml.loads(destination.getvalue()) == {"id": 1}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { pathToFileURL } = require('node:url')
    const { toml } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-toml-'))
    const target = path.join(root, 'value.toml')
    toml.dump({ id: 1 }, target)

    assert.deepEqual(toml.load(pathToFileURL(target)), { id: 1 })
    fs.rmSync(root, { recursive: true, force: true })
    ```

### Formatting

TOML keeps objects as inline tables and can lay array items out vertically.
`Formatting::indented(n)` indents those nested items for readability;
`Formatting::compact()` requests no extra layout. Whitespace changes, meaning
does not.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{Scalar};
    use yggdryl::text::toml;

    let value = Scalar::from_record([(
        "items",
        Scalar::from_sequence([
            Scalar::from(1_i64),
            Scalar::from(2_i64),
            Scalar::from(3_i64),
        ]),
    )])?;
    let laid_out =
        toml::into_utf8_with_formatting(&value, Formatting::indented(2))?;
    let compact = toml::into_utf8_with_formatting(&value, Formatting::compact())?;

    assert_ne!(laid_out, compact);
    assert_eq!(toml::from_utf8(&laid_out)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.text import toml

    value = {"items": [1, 2, 3]}
    laid_out = toml.dumps(value, indent=2)
    compact = toml.dumps(value, indent=None)

    assert laid_out != compact
    assert toml.loads(laid_out) == toml.loads(compact) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const value = { items: [1, 2, 3] }
    const laidOut = toml.dumps(value, { indent: 2 })
    const compact = toml.dumps(value, { indent: null })

    assert.notDeepEqual(laidOut, compact)
    assert.deepEqual(toml.loads(laidOut), toml.loads(compact))
    ```

Keys are always quoted, so dots, spaces, and names that resemble TOML syntax
round-trip without changing table structure.

### Placeholders

TOML placeholders are opt-in and must be inside quoted strings. Substitution
runs after parsing and before optional Field interpretation.

=== "Rust"

    ```rust
    use yggdryl::text::{Format, Loading, Placeholders};
    use yggdryl::Scalar;

    let loading = Loading::new().with_placeholders(
        Placeholders::new()
            .with_variable("HOST", Scalar::from("db.internal"))
            .with_variable("PORT", Scalar::from(5432_i64)),
    );
    let value = yggdryl::text::from_utf8_with(
        "host = \"{{ HOST }}\"\nport = \"{{ PORT }}\"\n",
        Format::Toml,
        &loading,
    )?;

    assert_eq!(value.get_key_str("host").and_then(Scalar::as_utf8), Some("db.internal"));
    assert_eq!(value.get_key_str("port"), Some(&Scalar::from(5432_i64)));
    ```

=== "Python"

    ```python
    from yggdryl.text import toml

    document = '[database]\nhost = "{{ HOST }}"\nport = "{{ PORT }}"\n'
    value = toml.loads(
        document,
        placeholders={"HOST": "db.internal", "PORT": 5432},
    )

    assert value["database"] == {"host": "db.internal", "port": 5432}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { toml } = require('yggdryl')

    const document = 'host = "{{ HOST }}"\nport = "{{ PORT }}"\n'
    const value = toml.loads(document, {
      placeholders: { HOST: 'db.internal', PORT: 5432 },
    })

    assert.deepEqual(value, { host: 'db.internal', port: 5432 })
    ```

A supplied mapping wins over environment lookup. The environment is never read
unless explicitly enabled. See
[structured text](text.md#jinja-style-placeholders) for syntax and measured
overhead.

### Limits and errors

Nullable binding options expose Rust's byte, depth, decoded-node, and document
limits under snake-case/camel-case names; omission uses the core defaults.
`validate_for_write` checks the natural TOML projection before writing, so a
null, non-record root, non-string table key, out-of-range integer, or excessive
depth cannot leave a partial destination.

Parse and validation errors name TOML and a byte offset.

### `IOBase` and content coding

Generic `from_io` / `into_io` infer TOML and any outer coding from the handle's
media type. Python accepts `PathLike` sources and destinations; JavaScript
accepts paths, file descriptors, file URLs, and streams.
