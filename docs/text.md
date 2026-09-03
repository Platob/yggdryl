# Structured text and line media

`yggdryl::text` presents record media first and document codecs second:

- plain text as streamed line records and Arrow batches;
- JSON, JSON Lines, YAML, and TOML as the shared native [`Scalar`](generic.md).

Both use `IOBase` handles, infer their representation from media type, and keep
parsing and encoding in the Rust core.

## Text media and Arrow batches

A `.log` or `.txt` resource is a record encoding. Reads stream records into the
same Arrow `BatchReader` used by IPC and Parquet. The same explicit write entry
points expose overwrite and append; keyed merge is rejected because lines have
no stable identity. No format argument is passed.

=== "Rust"

    ```rust
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::{Buffer, IOBase, IOMedia};
    use yggdryl::Url;

    fn named(name: &str) -> yggdryl::Result<Buffer> {
        Ok(Buffer::new().with_media_type(Url::from_str(&format!("file:///{name}"))?.media_type()))
    }

    let mut source = named("app.log")?;
    source.write_all_bytes(b"first event\nsecond event\n")?;

    let options = source.record_options()?;
    let rows: usize = source
        .read_arrow_reader(&options)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<_, _>>()?;
    assert_eq!(rows, 2);

    let mut target = named("copy.log")?;
    target.overwrite_arrow_reader(source.read_arrow_reader(&options)?, &options)?;
    target.append_arrow_reader(source.read_arrow_reader(&options)?, &options)?;
    assert_eq!(
        target.read_all_bytes()?,
        b"first event\nsecond event\nfirst event\nsecond event\n"
    );

    let merging = options.clone().with_merge_by_names(["message"]);
    let refused = target.merge_arrow_reader(source.read_arrow_reader(&options)?, &merging);
    assert!(refused.unwrap_err().to_string().contains("row identity"));
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pytest

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-"))
    (root / "app.log").write_text("first event\nsecond event\n")

    table = IOBase(root / "app.log").read_arrow_reader().read_all()
    assert table.num_rows == 2
    assert table.column("message").to_pylist() == ["first event", "second event"]

    target = IOBase(root / "copy.log")
    target.overwrite_arrow_table(table)
    target.append_arrow_table(table)
    assert (root / "copy.log").read_text() == (
        "first event\nsecond event\nfirst event\nsecond event\n"
    )

    merging = target.record_options()
    merging.merge_by_names = ["message"]
    with pytest.raises(ValueError, match="row identity"):
        target.merge_arrow_table(table, options=merging)

    shutil.rmtree(root)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-'))
    fs.writeFileSync(path.join(root, 'app.log'), 'first event\nsecond event\n')

    const table = new IOBase(path.join(root, 'app.log')).readArrowReader().intoTable()
    assert.equal(table.numRows, 2)
    assert.deepEqual([...table.getChild('message')], ['first event', 'second event'])

    const target = new IOBase(path.join(root, 'copy.log'))
    target.overwriteArrowTable(table)
    target.appendArrowTable(table)
    assert.equal(
      fs.readFileSync(path.join(root, 'copy.log'), 'utf8'),
      'first event\nsecond event\nfirst event\nsecond event\n',
    )

    assert.throws(
      () => target.mergeArrowTable(table, target.recordOptions().withMergeByNames(['message'])),
      /row identity/,
    )

    fs.rmSync(root, { recursive: true, force: true })
    ```

The default projection includes location, row number, timing, hash, header,
message, offset, and line-count columns. Captures and constant fields add
columns. Writes render `header` plus `message`, or one `utf8` column. Append is
supported; keyed merge is refused because a text line has no stable identity.

### Measured batch operations

The write fixture contains 4,096 rows in one `utf8` message column. Criterion
constructs the stored append/read side outside the timer, then drains or
publishes through the same `IOMedia` methods shown above.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 4,096 | 1.19 ms | 3.45M rows/s |
| `overwrite_arrow_reader` | 4,096 | 59.3 us | 69.1M rows/s |
| `append_arrow_reader` | 4,096 | 87.4 us | 46.9M rows/s |
| keyed `merge_arrow_reader` (upsert) | - | unsupported | no stable row identity |

These are Criterion point estimates from a Windows x86_64 release smoke run
on an AMD Ryzen 5 150 with rustc 1.96.1 (2026-08-23). Regenerate them on the
deployment host with:

```console
cargo bench -p yggdryl --bench io --features parquet -- "io_write_stateful/text"
```

The text fixture is deliberately narrow and should be compared between its own
operations, not directly with the four-column IPC, Parquet, and Avro fixtures.

### Line iteration with `Text`

`Text<H>` wraps an `IOBase` handle without hiding its bytes. `into_text` is
idempotent and adds:

- `read_lines` / `into_read_lines`, yielding one borrowed `TextLine` at a time;
- `write_lines` and `append_lines`, consuming iterables without collecting;
- `read_arrow_lines`, projecting the same records into bounded Arrow batches.

`TextLine` borrows its byte window and computes UTF-8, captures, and hash only
when requested. Call `into_owned` only when a row must outlive that window.
Python and JavaScript expose their native lazy iterator protocols.

`TextLineOptions` is the complete extractor. It can be built in code or parsed
unchanged from a JSON, YAML, or TOML configuration:

| option | purpose |
| --- | --- |
| `opening` / `logs` / `pattern` | choose where a record begins |
| `header` | parse the opening line separately from its message |
| `linesep`, `lstrip`, `rstrip` | control record boundaries and trimming |
| `timestamp_capture`, `timezone` | turn a captured wall time into an instant |
| `capture_types` | strictly type named captures |
| `custom_fields` | append constant columns |
| `byte_size`, `batch_row_size` | close a batch when the first bound is reached |

`Opening`, `TextLineOptions`, and `TextOptions` are ordinary Rust values:
cloning preserves the complete declaration, and `Eq`, `Ord`, and `Hash`
compare the declaration rather than compiled regex or schema caches. Their
`stable_hash()` methods provide the same deterministic identity across runs;
for a pattern, its source text is the declared identity.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};

    let handle = Buffer::from_bytes(b"first event\nsecond event\n".to_vec()).into_text();
    let mut records = handle.read_lines()?;

    assert_eq!(records.next().unwrap()?.text()?, "first event");
    assert_eq!(records.next().unwrap()?.text()?, "second event");
    assert!(records.next().is_none());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    with tempfile.TemporaryDirectory() as directory:
        source = pathlib.Path(directory) / "app.log"
        source.write_bytes(
            b"2026-08-01 [ERROR] failed\n  detail\n"
            b"2026-08-01 [INFO] ready\n"
        )
        records = list(
            IOBase(source).read_lines(r"^\d{4}-\d{2}-\d{2} \[[A-Z]+\]")
        )

        assert len(records) == 2
        assert "detail" in records[0]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const target = path.join(root, 'app.log')
    fs.writeFileSync(target, 'first event\nsecond event\n')

    const handle = new IOBase(target)
    assert.deepEqual([...handle.readLines()], ['first event', 'second event'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

Unset `linesep` accepts LF, CRLF, and lone CR; writes use LF. Compressed names
such as `app.log.gz`, `app.log.zz`, and `app.log.zst` decode as streams. The
reader reuses one byte window and retains only an unfinished cross-chunk row.
Arrow projection consumes the same reader and emits each bounded batch when it
closes. An absent resource yields an empty iterator.

### First-item latency

The `lines_identity` Criterion group measures `stable_hash()` for an opening
rule, a complete line extractor, text record options, and the enclosing
`RecordOptions`; each configured value is built outside the timed loop.

`lines_first` measures construction plus one result: one borrowed line, or one
1,024-row Arrow batch from a 50,000-row corpus. `local` is a located file,
OS-cache-warm after setup and Criterion warm-up; the Arrow reader reopens and
owns that handle, so it streams without snapshotting the resource. `memory` is
the direct borrowed-line control. `snapshot` is the separately named
unlocated-`Buffer` Arrow fallback, which must copy its encoded value into an
owned reader before returning.

```console
cargo bench -p yggdryl --bench text -- lines_first --noplot
```

Observed 2026-08-23 on Windows 11 x86_64, AMD Ryzen 5 150 (6 cores / 12
threads), rustc 1.96.1, release profile. Cells are Criterion median point
estimates from the generated `new/estimates.json` files:

| coding | first line, local | first line, memory | first Arrow batch, local | first Arrow batch, snapshot |
| --- | ---: | ---: | ---: | ---: |
| plain | 5.14 us | 6.58 us | 22.7 ms | 10.1 ms |
| gzip | 65.5 us | 62.0 us | 20.1 ms | 4.57 ms |
| zlib | 56.0 us | 58.5 us | 20.4 ms | 4.33 ms |
| zstd | 192 us | 224 us | 21.0 ms | 4.82 ms |

The snapshot numbers are an ownership baseline, not a claim about file
streaming: their source bytes are already resident, and compressed snapshots
copy fewer encoded bytes. The local Arrow cases exercise the production-shaped
reader ownership path, not cold-disk latency. Their 20--23 ms first batches
arrive after 1,024 rows rather than waiting for the remaining 48,976 rows or
retaining decoded pages.

### Dimensions and opened sessions

`row_size` and `column_size` describe the complete text media, ignoring
selection and read limits. A fresh row count walks record boundaries without
building Arrow arrays. Column count comes from the configured Struct field and
does not read bytes.

`open` caches the resolved field, coding plan, and requested dimensions until
`close`. Writes and option changes invalidate those values. Closed calls are
always fresh.

The benchmark uses 65,536 records and compares the borrowed count with the full
ten-column Arrow projection:

```console
cargo bench -p yggdryl --bench io --features parquet -- io_dimensions/text --warm-up-time 0.2 --measurement-time 0.5 --sample-size 10
```

One local Windows x86_64 release smoke run (Criterion point estimates;
regenerate on the deployment host):

| operation | estimate |
| --- | ---: |
| fresh `row_size` extractor walk | 4.64 ms |
| opened `row_size` cache hit | 3.76 ns |
| fresh `column_size` configured field | 4.36 ns |
| opened `column_size` cache hit | 4.27 ns |
| `record_options` resolution | 382 ns |
| `is_io` media capability | 2.21 ns |
| fresh `read_arrow_field` | 1.26 ms |
| opened `read_arrow_field` | 1.40 ms |
| full `read_arrow_reader` row decode | 46.3 ms |

The fresh count was about ten times faster than Arrow decoding; opened counts
and configuration-only width paths were nanosecond-scale.

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

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/text.ipynb){ download },
[Python](notebooks/python/text.ipynb){ download },
[JavaScript](notebooks/javascript/text.ipynb){ download }.

<!-- /notebooks -->
