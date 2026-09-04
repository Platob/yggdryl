# Structured text and plain-text records

`yggdryl::text` owns two related surfaces:

- `text/plain` as physical-line records through the ordinary media methods;
- JSON, JSON Lines, YAML, and TOML over the shared native [`Scalar`](generic.md).

Parsing, schema inference, casting, and encoding stay in the Rust core. Python
and JavaScript only translate their native record and Arrow holders.

## Plain-text records

A plain-text row always begins with this required schema:

| column | datatype | value |
| --- | --- | --- |
| `url` | `utf8` | source URL, or an empty string for an unlocated buffer |
| `rownum` | `int64` | one-based physical line number, restarted for each leaf |
| `body` | `binary` | line bytes without the record terminator |

Use `TextOptions` with the ordinary `read_arrow_reader` / `readArrowReader` or
`read_records` / `readRecords` methods. `Text` retains those options for
generic handles; it adds no line iterator, schema builder, or read/write
vocabulary.

`TextOptions` is flat and converts into the text variant of `RecordOptions` at
the generic dispatch boundary:

| option | contract |
| --- | --- |
| `rowheader` | byte regex searched once per line; named captures append nullable columns |
| `lstrip`, `rstrip` | byte regex removed only when its match touches the corresponding body edge |
| `linesep` | exact terminator; unset accepts LF, CRLF, or CR and writes LF |
| `autotype` | infer capture datatypes from the first batch; default `true` |
| `timezone` | zone applied when autotyping offset-free timestamps |

When `rowheader` matches, its complete match is removed from `body`. Edge
stripping runs afterward. A line without a match keeps its body and receives
null capture values.

Autotyping recognizes booleans, signed 64-bit integers, finite floats, ISO
dates, times, and timestamps. Types are fixed after the first
`batch_row_size` rows (or the shared default batch size). A later
incompatible value is an error naming its row and capture. Set
`autotype = false` to keep every capture as UTF-8. An empty resource still
answers the complete schema, with capture columns as UTF-8.

=== "Rust"

    ```rust
    use arrow_array::{Array as _, BinaryArray, Int64Array};
    use yggdryl::generic::IORecordOptions as _;
    use yggdryl::io::{Buffer, IOBase as _, IOMedia as _};
    use yggdryl::text::TextOptions;
    use yggdryl::Url;

    let text_source = Buffer::from_bytes(
        b"  [INFO] id=7 first  \r\n[WARN] id=9 second\n".to_vec(),
    )
    .with_media_type(Url::from_str("file:///app.log")?.media_type());

    let mut text_options = TextOptions::new();
    text_options.set_rowheader(Some(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)"))?;
    text_options.set_lstrip(Some(r"^\s+"))?;
    text_options.set_rstrip(Some(r"\s+$"))?;
    let text_source = text_source.into_text_with(text_options);
    let record_options = text_source.record_options()?;

    let text_batch = text_source
        .read_arrow_reader(&record_options)?
        .next()
        .unwrap()?;
    assert_eq!(text_batch.schema().fields().len(), 5);
    assert_eq!(
        text_batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values(),
        &[1, 2],
    );
    assert_eq!(
        text_batch
            .column(2)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .value(0),
        b"first",
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, TextOptions

    with tempfile.TemporaryDirectory() as directory:
        source = pathlib.Path(directory) / "app.log"
        source.write_bytes(b"  [INFO] id=7 first  \r\n[WARN] id=9 second\n")

        options = TextOptions()
        options.rowheader = r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)"
        options.lstrip = r"^\s+"
        options.rstrip = r"\s+$"

        handle = IOBase(source).into_text(options)
        rows = list(handle.read_records())
        assert [row["rownum"] for row in rows] == [1, 2]
        assert [row["body"] for row in rows] == [b"first", b"second"]
        assert [row["id"] for row in rows] == [7, 9]

        target = IOBase(pathlib.Path(directory) / "copy.txt")
        target.overwrite_records(
            ({"body": row["body"]} for row in rows),
            options=TextOptions(),
        )
        assert target.read_bytes() == b"first\nsecond\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, TextOptions } = require('yggdryl')

    const textRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-'))
    const textSource = path.join(textRoot, 'app.log')
    fs.writeFileSync(textSource, '  [INFO] id=7 first  \r\n[WARN] id=9 second\n')

    const textOptions = new TextOptions()
    textOptions.rowheader = '\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+)'
    textOptions.lstrip = '^\\s+'
    textOptions.rstrip = '\\s+$'

    const textHandle = new IOBase(textSource).intoText(textOptions)
    const textRows = [...textHandle.readRecords()]
    assert.deepEqual(textRows.map((row) => row.rownum), [1n, 2n])
    assert.deepEqual(
      textRows.map((row) => Buffer.from(row.body).toString()),
      ['first', 'second'],
    )
    assert.deepEqual(textRows.map((row) => row.id), [7n, 9n])

    const textTarget = new IOBase(path.join(textRoot, 'copy.txt'))
    textTarget.overwriteRecords(
      textRows.map((row) => ({ body: row.body })),
      new TextOptions(),
    )
    assert.equal(textTarget.readBytes().toString(), 'first\nsecond\n')

    fs.rmSync(textRoot, { recursive: true, force: true })
    ```

Writes consume the non-null Binary `body` column and append the configured
terminator. A body containing that terminator is refused. Overwrite and append
use the generic media methods; keyed merge remains unsupported for plain text.

Content codings belong to the handle. Thus `app.log.gz` and a folder mixing
plain and gzip leaves use the same options and stream decoded rows without
retaining prior pages. The line splitter retains only the unfinished fragment
needed across byte chunks.

### Measuring the boundary

The three benchmark targets use the same generic record methods. Python also
includes an equivalent `re` plus PyArrow baseline; JavaScript numbers include
the copied IPC crossing required by Arrow JS.

```console
cargo bench -p yggdryl --bench text
cd python
.venv/Scripts/python benchmarks/text.py --min-time 0.2 --repeat 7
cd ..
npm run --prefix node bench:text
```

## Raw shared-Scalar access

`Scalar` is the one Rust value tree used by JSON, YAML, TOML, Fields, Arrow, and
both extensions. Records use deterministic name order.

=== "Rust"

    ```rust
    use yggdryl::{json, Scalar};

    let quote = json::from_utf8(r#"{"symbol":"AAPL","price":12.5}"#)?;

    assert_eq!(
        quote.get_key_str("symbol").and_then(Scalar::as_utf8),
        Some("AAPL")
    );
    assert_eq!(json::into_utf8(&quote)?, r#"{"price":12.5,"symbol":"AAPL"}"#);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, json

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

### Typed `Scalar` families

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
        Scalar::I8(-1).checked_add(&Scalar::U8(2))?,
        Scalar::I16(1),
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

### Scalar and Arrow boundary costs

These release smoke runs measure the accessors above and keep conversion setup
outside the timed loop. They were recorded on Windows x86_64; regenerate them
on the deployment host before comparing releases.

```console
cargo bench -p yggdryl --bench datatype -- value
cd python
.venv/Scripts/python benchmarks/values.py --iterations 10000
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

## Field-directed parsing

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
and a duration writes back as `PT<seconds>S`.

Pass a `Field` when exact types are required. Parsing happens first, optional
placeholder substitution happens second, and the Field interprets and
canonicalizes the resulting natural value last.

=== "Rust"

    ```rust
    use yggdryl::{json, DataType, Field, Scalar};

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

    from yggdryl import Field, Scalar, json

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

## Raw document codecs

| format | documents | natural root |
| --- | --- | --- |
| JSON | one; JSON Lines for many | any JSON value |
| YAML | one or more | any YAML value |
| TOML | exactly one | a string-key record |

Rust names the transport in `from_utf8`, `from_bytes`, `from_reader`,
`into_utf8`, `into_bytes`, and `into_writer`. `_all` covers JSON streams, JSON
Lines, and YAML documents. `Json`, `JsonLines`, `Yaml`, and `Toml` implement the
same `TextCodec` contract.

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
    assert_eq!(value.get_key_str("id"), Some(&Scalar::U64(1)));
    assert_eq!(text::into_utf8(&value, format)?, r#"{"id":1}"#);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, codec

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

### Natural-codec boundary costs

One Windows x86_64 release run used the same eight-leg field-class document
for every Python format and the same larger natural document for every Node
format. These fixtures compare routes within a runtime, not Python against
Node. Regenerate with `python benchmarks/codecs.py --iterations 10000` and
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

## Formatting

`Formatting` changes bytes, never meaning. Its `Indent` is `Default`, `None`,
`Spaces(n)`, or `Tabs`:

- JSON defaults to compact and uses spaces for pretty output;
- YAML defaults to two-space block style; `None` selects flow style;
- TOML uses indentation only for nested readability.

=== "Rust"

    ```rust
    use yggdryl::text::Formatting;
    use yggdryl::{json, Scalar};

    let value = Scalar::from_record([("id", Scalar::I64(1))])?;
    let pretty =
        json::into_utf8_with_formatting(&value, Formatting::indented(2))?;

    assert_eq!(pretty, "{\n  \"id\": 1\n}");
    assert_eq!(json::from_utf8(&pretty)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import json

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

## Placeholders

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
    assert_eq!(value.get_key_str("port"), Some(&Scalar::I64(8080)));
    ```

=== "Python"

    ```python
    from yggdryl import yaml

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

## Limits and errors

`Limits` bounds input bytes, nesting, decoded nodes, and document count.
Readers enforce the bounds while streaming and fuse after the first error.
Both bindings expose nullable spellings of all four limits; omitted values use
the same safe core defaults.

Codec errors name the format and byte offset. Offsets are cumulative for
multi-document readers, so failures remain locatable in the original input.
Duplicate keys, invalid UTF-8, unsupported natural shapes, and Field conversion
fail rather than silently coercing a value.

## `IOBase` and content coding

`Format` has `Json`, `JsonLines`, `Yaml`, and `Toml`; extension, path, and MIME
inference share one vocabulary. Content inference tries JSON before YAML
because most JSON is valid YAML. In Python a `str` is document content, so use
`pathlib.Path` for a path; destination strings are unambiguous paths.

`text::from_io` and `text::into_io` infer both format and content coding from a
handle's `MediaType`. Thus `quotes.json.gz` parses JSON through gzip without a
format or coding argument. `from_io_with_field` applies the same strict typing
as in-memory loaders. Whole-value writes publish when complete; reader/writer
functions stream directly and leave record and listing APIs lazy.
