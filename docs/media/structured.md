# Structured documents

`yggdryl::text` reads and writes JSON, JSON Lines, YAML, and TOML over the shared native [`Scalar`](../types/scalar.md); the bindings only translate native and Arrow values.

## Contract

| | |
| --- | --- |
| Owns | one [`Scalar`](../types/scalar.md) tree for every format, Field, Arrow, and binding; records keep deterministic name order; `text/plain` lines are [plain-text records](text/index.md) instead |
| Core | parsing, inference, casting, and encoding in Rust; bindings translate native records and Arrow holders |
| Returns | native objects; `cls=Scalar` / `{ scalar: true }` answer the lossless core `Scalar`, whose `get`, `path`, `set`, `remove`, and iteration stay exact wrappers |
| Order | parse, then [placeholder](placeholders.md) substitution, then Field interpretation |
| Content | `&str`, `String`, byte slices, and Python `str` are content, never a path; a path is `pathlib.Path`; destination strings are paths |
| Inference | `inferred_scalar_field` / `inferred_array_field` / `inferred_struct_field`, names `value`, `item`, `row`; one path for every runtime |
| `Format` | `Json`, `JsonLines`, `Yaml`, `Toml`; extension, path, MIME, and content sniff share one vocabulary; sniff tries JSON before YAML; anonymous output is JSON |
| `Limits` | input bytes, nesting, decoded nodes, document count, enforced while streaming; four nullable spellings in both bindings; omitted uses the safe core default |
| Errors | name the format and byte offset, cumulative across documents; readers fuse after the first error |
| Coding | `text::from_io` / `into_io` infer format and coding from the handle `MediaType`, so `quotes.json.gz` is JSON through gzip; `from_io_with_field` types strictly |
| Charset | the same plan resolves a [charset](../charset/index.md) from the handle's `MediaType`, or from a leading byte-order mark; the mark is framing at this seam and comes off before the parser sees it, so a `windows-1252` JSON document reads without an argument |

## Formats

| Scheme | Overview | Scalars | Arrow |
| --- | --- | --- | --- |
| JSON, JSON Lines | [JSON](json/index.md) | [read and write](json/scalar.md) | [rows](json/arrow.md) |
| YAML | [YAML](yaml/index.md) | [read and write](yaml/scalar.md) | [rows](yaml/arrow.md) |
| TOML | [TOML](toml/index.md) | [read and write](toml/scalar.md) | [rows](toml/arrow.md) |

[Placeholders](placeholders.md) states the Jinja-style `{{ }}` contract YAML and TOML share.

## Use

One facade reads and writes every format: it infers the format once, then redirects to that implementation. A named source uses its compound suffix; anonymous input is sniffed by the core, JSON before YAML.

=== "Rust"

    ```rust
    use yggdryl::text::{self, Format};
    use yggdryl::Scalar;

    let (format, quote) = text::from_utf8_inferred(r#"{"symbol":"AAPL","price":12.5}"#)?;

    assert_eq!(format, Format::Json);
    assert_eq!(
        quote.get_key_str("symbol").and_then(Scalar::as_str),
        Some("AAPL")
    );
    assert_eq!(text::into_utf8(&quote, format)?, r#"{"price":12.5,"symbol":"AAPL"}"#);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl.text import codec

    quote = codec.from_io('{"symbol":"AAPL","price":12.5}', cls=Scalar)

    assert isinstance(quote, Scalar)
    assert quote["symbol"].as_str() == "AAPL"
    assert quote.path("price").kind == "f64"
    assert quote.set("venue", "XNAS").get("venue").as_str() == "XNAS"
    assert codec.into_io(quote, format="json", utf8=True) == '{"price":12.5,"symbol":"AAPL"}'
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, codec } = require('yggdryl')

    const quote = codec.from('{"symbol":"AAPL","price":12.5}', { scalar: true })

    assert.ok(quote instanceof Scalar)
    assert.equal(quote.get('symbol').asStr(), 'AAPL')
    assert.equal(quote.path('price').kind, 'f64')
    assert.equal(quote.set('venue', 'XNAS').get('venue').asStr(), 'XNAS')
    assert.deepEqual(codec.into(quote, { format: 'json' }), Buffer.from('{"price":12.5,"symbol":"AAPL"}'))
    ```

## Field-directed parsing

Dumps use ordinary format values; exact values without native syntax become scaled-decimal, base64, or ISO strings. A schemaless read returns only what the grammar proves, so pass a `Field` for exact types: [JSON](json/index.md#natural-values-and-exact-fields) reads `"12.50"` under `decimal128(8, 2)` as `D128(1250, 2)`, a Python `Decimal("12.50")`, and a JavaScript `d128` scalar, and [YAML](yaml/index.md#natural-values-and-exact-fields) and [TOML](toml/index.md#natural-values-and-exact-fields) read their own spellings the same way.

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

JSON Lines is collection-valued, JSON and TOML return one value, and YAML can stay lazy on the explicit stream path. Field casting and the four decode limits survive redirection.

## Arrow rows

A document is not a record encoding: [`RecordOptions`](options.md) names none of these formats, so `read_arrow_reader` and the three write intents refuse the name. `read_arrow` and `write_arrow` are the one bridge, and the two directions are asymmetric because the formats are.

| Direction | JSON, TOML | JSON Lines, YAML |
| --- | --- | --- |
| Read | one document, then one batch | every document, then one batch |
| Write | one document, rows held | streamed, one batch of rows at a time |

A document has no frame to read a prefix of, so a read holds the parsed document; a write has one wherever the format is document-per-row, and there nothing but the current batch is held. A structured document is written whole, so only `IOMode::Overwrite` applies. Both calls are Rust and Python; JavaScript binds neither. Each format's rules are on its own page: [JSON](json/arrow.md), [YAML](yaml/arrow.md), [TOML](toml/arrow.md).

## Formatting

`Formatting` changes bytes, never meaning; `Indent` is `Default`, `None`, `Spaces(n)`, or `Tabs`. Bindings spell `indent=2` / `{ indent: 2 }`, `None` / `null`, and `"\t"`; the same value carries the [coded handle](../coding/index.md) compression level.

| format | `Indent` rule |
| --- | --- |
| JSON | compact by default; spaces for pretty output |
| YAML | two-space block style by default; `None` selects flow style |
| TOML | indentation only for nested readability |

=== "Rust"

    ```rust
    use yggdryl::text::{json, toml, yaml, Formatting};
    use yggdryl::Scalar;

    let value = Scalar::from_record([(
        "child",
        Scalar::from_record([("id", Scalar::from(1_i64))])?,
    )])?;

    // JSON is compact by default and spaces each level out under an indent.
    assert_eq!(json::into_utf8(&value)?, r#"{"child":{"id":1}}"#);
    assert_eq!(
        json::into_utf8_with_formatting(&value, Formatting::indented(2))?,
        "{\n  \"child\": {\n    \"id\": 1\n  }\n}",
    );

    // YAML is two-space block style by default; `compact` selects flow style.
    assert_eq!(yaml::into_utf8(&value)?, "child:\n  id: 1\n");
    assert_eq!(
        yaml::into_utf8_with_formatting(&value, Formatting::compact())?,
        "{child: {id: 1}}\n",
    );

    // TOML lays nested values out for readability, and reads back the same value.
    let laid_out = toml::into_utf8_with_formatting(&value, Formatting::indented(2))?;
    assert_eq!(toml::from_utf8(&laid_out)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.text import json, toml, yaml

    value = {"child": {"id": 1}}

    # JSON is compact by default and spaces each level out under an indent.
    assert json.dumps(value, indent=None) == b'{"child":{"id":1}}'
    assert json.dumps(value, indent=2) == b'{\n  "child": {\n    "id": 1\n  }\n}'

    # YAML is two-space block style by default; None selects flow style.
    assert yaml.dumps(value, indent=4) == b"child:\n    id: 1\n"
    assert yaml.dumps(value, indent=None) == b"{child: {id: 1}}\n"

    # TOML lays nested values out for readability, and reads back the same value.
    assert toml.loads(toml.dumps(value, indent=2)) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json, toml, yaml } = require('yggdryl')

    const value = { child: { id: 1 } }

    // JSON is compact by default and spaces each level out under an indent.
    assert.deepEqual(json.dumps(value, { indent: null }), Buffer.from('{"child":{"id":1}}'))
    assert.deepEqual(
      json.dumps(value, { indent: 2 }),
      Buffer.from('{\n  "child": {\n    "id": 1\n  }\n}'),
    )

    // YAML is two-space block style by default; null selects flow style.
    assert.deepEqual(yaml.dumps(value, { indent: 4 }), Buffer.from('child:\n    id: 1\n'))
    assert.deepEqual(yaml.dumps(value, { indent: null }), Buffer.from('{child: {id: 1}}\n'))

    // TOML lays nested values out for readability, and reads back the same value.
    assert.deepEqual(toml.loads(toml.dumps(value, { indent: 2 })), value)
    ```

## Edges

- `25:30:00` as a time of day -> `01:30:00`; hours fold modulo the day up to `99`.
- `2026-08-17T24:00:00` as a datetime -> the 18th at midnight.
- `26:03:04`, `P1DT2H3M4S`, `PT93784S` as a duration -> one count; written back as `PT<seconds>S`.
- Minutes and seconds in any clock spelling -> always under sixty; only hours fold, up to `99`.
- A time of day or duration with a zone -> refused; both must be naive.
- Empty or positional rows without a `Field` -> ambiguous; an explicit `Field` is required.
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
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text
    python/.venv/bin/python -m pytest python/tests/text/test_codec_facade.py python/tests/text/test_codec_fields.py python/tests/text/test_codec_native_returns.py python/tests/text/test_codec_options.py
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/text
    npm run --prefix node bench:text
    ```

## Performance

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
