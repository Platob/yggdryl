# JSON

Read and write JSON as the shared [`Value`](generic.md) - whole, streamed, or one row per line.

=== "Rust"

    ```rust
    use yggdryl::{Value, json};

    let value = Value::from_mapping([
        (Value::from("symbol"), Value::from("AAPL")),
        (Value::from("quantity"), Value::from(100_i64)),
    ])?;

    let encoded = json::to_vec(&value)?;
    assert_eq!(encoded, br#"{"symbol":"AAPL","quantity":100}"#);
    assert_eq!(json::from_slice(&encoded)?, value);
    assert_eq!(json::from_str(r#"{"symbol":"AAPL","quantity":100}"#)?, value);
    assert_eq!(value.get_key_str("symbol"), Some(&Value::from("AAPL")));
    ```

=== "Python"

    ```python
    from yggdryl import json

    value = {"symbol": "AAPL", "quantity": 100}

    encoded = json.dumps(value)
    assert encoded == b'{"symbol":"AAPL","quantity":100}'
    assert json.loads(encoded) == value
    assert json.loads('{"symbol":"AAPL","quantity":100}') == value
    assert json.loads(encoded)["symbol"] == "AAPL"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const value = { symbol: 'AAPL', quantity: 100 }

    const encoded = json.dumps(value)
    assert.equal(encoded.toString(), '{"symbol":"AAPL","quantity":100}')
    assert.deepEqual(json.loads(encoded), value)
    assert.deepEqual(json.loads('{"symbol":"AAPL","quantity":100}'), value)
    assert.equal(json.loads(encoded).symbol, 'AAPL')
    ```

`to_vec` and `from_slice` are the whole-buffer pair. `from_str` passes borrowed UTF-8
through without an intermediate byte buffer, and `into_vec` is `to_vec` taking the value
by value.

Encoding is byte-first on every side: Rust hands back `Vec<u8>`, Python `bytes`,
JavaScript a `Buffer`. Nothing returns a string you then have to encode again. Decoding
takes those bytes back, and in Python and JavaScript also a string holding the same
content.

Mapping order is insertion order and survives the round trip.

## Values JSON has no syntax for

=== "Rust"

    ```rust
    use yggdryl::{Value, json};

    let value = Value::from_mapping([
        (Value::from("payload"), Value::from(vec![0_u8, 1, 255])),
        (Value::from("big"), Value::U128(1_u128 << 127)),
        (Value::from("ratio"), Value::from(f64::NAN)),
    ])?;

    let encoded = json::to_vec(&value)?;
    assert!(String::from_utf8(encoded.clone())?.contains(r#""$yggdryl""#));

    let decoded = json::from_slice(&encoded)?;
    assert_eq!(
        decoded.get_key_str("payload").and_then(Value::as_bytes),
        Some([0_u8, 1, 255].as_slice())
    );
    assert_eq!(
        decoded.get_key_str("big").and_then(Value::as_u128),
        Some(1_u128 << 127)
    );
    assert!(
        decoded
            .get_key_str("ratio")
            .and_then(Value::as_f64)
            .is_some_and(f64::is_nan)
    );
    ```

=== "Python"

    ```python
    import math

    from yggdryl import json

    value = {"payload": b"\x00\x01\xff", "big": 2**127, "ratio": math.nan}

    encoded = json.dumps(value)
    assert b'"$yggdryl"' in encoded

    decoded = json.loads(encoded)
    assert decoded["payload"] == b"\x00\x01\xff"
    assert decoded["big"] == 2**127
    assert math.isnan(decoded["ratio"])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const value = { payload: Buffer.from([0, 1, 255]), big: 2n ** 127n, ratio: NaN }

    const encoded = json.dumps(value)
    assert.ok(encoded.includes('"$yggdryl"'))

    const decoded = json.loads(encoded)
    assert.deepEqual(decoded.payload, Buffer.from([0, 1, 255]))
    assert.equal(decoded.big, 2n ** 127n)
    assert.ok(Number.isNaN(decoded.ratio))
    ```

JSON spells objects, arrays, strings, doubles, booleans, and null. [`Value`](generic.md)
also carries bytes, 128-bit integers, exact decimals, the temporals, non-finite floats,
and mappings whose keys are not strings. A temporal writes as its classic ISO string -
`2026-08-15`, `10:00:00.123`, `2026-08-15T12:30:00+02:00[Europe/Paris]`, `PT90S` - the
spelling every other JSON reader already reads, with the fraction printed at the unit's
full width so the digits *are* the unit. Read back without a schema it is that string;
a [`Field`](field.md) or a record class is what recovers the typed reading. Every other
kind above becomes a one-key object under `$yggdryl` holding `version`, `type`, and the
encoded `value`, so the document stays ordinary JSON that any other reader can still
parse.

The envelope is matched exactly on the way back. A different `version`, an unrecognized
`type`, a missing field, or one extra field leaves the object a plain mapping, so
application data that happens to contain a `$yggdryl` key is never reinterpreted.

Every `type` names one `Value` variant, and there is no envelope for a name. A document
from an older producer spelling `type: "tag"` therefore names nothing, so it is not an
envelope and decodes as the ordinary mapping its syntax always was. A caller that wants a
class names it in the call - `cls=` in Python - see [Python](extensions/python.md) and
[JavaScript](extensions/javascript.md).

## Readers and writers

=== "Rust"

    ```rust
    use std::io::Cursor;
    use yggdryl::{Value, json};

    let value = Value::from_mapping([(Value::from("symbol"), Value::from("AAPL"))])?;

    let mut target = Vec::new();
    json::to_writer(&mut target, &value)?;
    assert_eq!(json::from_reader(Cursor::new(&target))?, value);

    let error = json::from_str(r#"{"symbol":"AAPL"} 42"#).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid json data at byte 18: trailing characters after JSON value"
    );
    ```

=== "Python"

    ```python
    import io

    from yggdryl import json

    value = {"symbol": "AAPL"}

    target = io.BytesIO()
    json.dump(value, target)
    assert json.load(io.BytesIO(target.getvalue())) == value

    reason = None
    try:
        json.loads('{"symbol":"AAPL"} 42')
    except ValueError as error:
        reason = str(error)
    assert reason == "invalid json data at byte 18: trailing characters after JSON value"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Readable, Writable } = require('node:stream')
    const { json } = require('yggdryl')

    const value = { symbol: 'AAPL' }

    async function main() {
      const chunks = []
      const target = new Writable({
        write(chunk, _encoding, done) {
          chunks.push(Buffer.from(chunk))
          done()
        },
      })

      await json.dumpStream(value, target)
      assert.deepEqual(await json.loadStream(Readable.from(chunks)), value)

      assert.throws(
        () => json.loads('{"symbol":"AAPL"} 42'),
        /invalid json data at byte 18: trailing characters after JSON value/,
      )
    }

    main()
    ```

`to_writer` takes any `Write` and `from_reader` any `Read`, so neither side needs the
document as one buffer. Python's `dump` and `load` accept the same file objects, binary
or text, and never close one the caller opened. JavaScript's `dumpStream` and
`loadStream` accept Node and WHATWG streams and return promises; a sink that rejects a
frame surfaces as a rejected promise carrying that error.

Reading one value stays exact even from a reader: the decoder consumes the document, then
fails on anything after it that is not JSON whitespace, at that byte. Sources holding
more than one value go through the newline-delimited forms below, or through `Reader` for
values separated by any whitespace.

## Newline-delimited JSON

=== "Rust"

    ```rust
    use yggdryl::{Value, json};

    let rows = [
        Value::from_mapping([(Value::from("id"), Value::from(1_u64))])?,
        Value::from_mapping([(Value::from("id"), Value::from(2_u64))])?,
    ];

    let encoded = json::to_vec_all(&rows)?;
    assert_eq!(encoded, b"{\"id\":1}\n{\"id\":2}\n");
    assert_eq!(json::from_lines_slice(&encoded)?, rows);

    // Blank and CRLF-terminated lines are skipped; two values on one are not.
    assert_eq!(json::from_lines_str("{\"id\":1}\r\n\n{\"id\":2}\n")?, rows);
    assert_eq!(
        json::from_lines_str("{\"id\":1} {\"id\":2}\n")
            .unwrap_err()
            .to_string(),
        "invalid json data at byte 9: trailing characters after JSON value"
    );

    // The writer never needs the rows materialized.
    let mut target = Vec::new();
    json::to_writer_all(&mut target, (0_u64..3).map(Value::from))?;
    assert_eq!(target, b"0\n1\n2\n");
    ```

=== "Python"

    ```python
    from yggdryl import json

    rows = [{"id": 1}, {"id": 2}]

    encoded = json.dumps_all(rows)
    assert encoded == b'{"id":1}\n{"id":2}\n'
    assert list(json.loads_all(encoded)) == rows

    # Blank and CRLF-terminated lines are skipped; two values on one are not.
    assert list(json.loads_all(b'{"id":1}\r\n\n{"id":2}\n')) == rows

    reason = None
    try:
        list(json.loads_all(b'{"id":1} {"id":2}\n'))
    except ValueError as error:
        reason = str(error)
    assert reason == "invalid json data at byte 9: trailing characters after JSON value"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    const rows = [{ id: 1 }, { id: 2 }]

    const encoded = json.dumpAll(rows)
    assert.equal(encoded.toString(), '{"id":1}\n{"id":2}\n')
    assert.deepEqual(json.loadsAll(encoded), rows)

    // Blank and CRLF-terminated lines are skipped; two values on one are not.
    assert.deepEqual(json.loadsAll('{"id":1}\r\n\n{"id":2}\n'), rows)
    assert.throws(
      () => json.loadsAll('{"id":1} {"id":2}\n'),
      /invalid json data at byte 9: trailing characters after JSON value/,
    )
    ```

`to_writer_all` takes any `IntoIterator` of values, so the rows never have to exist as a
slice. Reading is the strict dialect, not a lenient one: exactly one value per line, no
value spanning lines, no second value sharing a line.

The plural entry points in the bindings are always newline-delimited. `dumps_all` and
`loads_all`, `dumpAll` and `loadsAll` write and read JSON Lines, never a JSON array; an
array is a single value and goes through `dumps`. The same pair is reachable from
[text](text.md) as `Format::JsonLines`, which is what a `.jsonl` or `.ndjson` name
resolves to.

Rows arrive one at a time when the source is a stream:

=== "Rust"

    ```rust
    use std::io::Cursor;
    use yggdryl::{Value, json};

    let mut source = Cursor::new(b"{\"id\":1}\n{\"id\":2}\n{\"id\":3}\n");
    let mut rows = json::LinesReader::new(&mut source);

    let first = rows.next().transpose()?.expect("one row");
    assert_eq!(first.get_key_str("id"), Some(&Value::from(1_u64)));
    assert_eq!(rows.byte_offset(), 9);
    assert_eq!(rows.collect::<yggdryl::Result<Vec<_>>>()?.len(), 2);

    // `Reader` splits on any JSON whitespace instead of on newlines.
    let free = json::Reader::new(Cursor::new(b"1 2 3")).collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(free, [Value::from(1_u64), Value::from(2_u64), Value::from(3_u64)]);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import json

    with tempfile.TemporaryDirectory() as directory:
        path = pathlib.Path(directory) / "rows.jsonl"
        json.dump_all([{"id": 1}, {"id": 2}, {"id": 3}], path)

        rows = json.load_all(path)
        assert iter(rows) is rows
        assert next(rows) == {"id": 1}
        assert list(rows) == [{"id": 2}, {"id": 3}]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    let pulls = 0
    async function* chunks() {
      pulls += 1
      yield '{"id":1}\n{"i'
      pulls += 1
      yield 'd":2}\n'
    }

    async function main() {
      const rows = json.loadAllStream(chunks())[Symbol.asyncIterator]()

      assert.deepEqual((await rows.next()).value, { id: 1 })
      assert.equal(pulls, 1)
      assert.deepEqual((await rows.next()).value, { id: 2 })
      assert.equal(pulls, 2)
      assert.equal((await rows.next()).done, true)
    }

    main()
    ```

`LinesReader` owns its reader and yields one `Result<Value>` per row; `byte_offset` is
the cumulative position of the next unread byte, which is what makes a later failure
locatable in the original file. `from_lines_reader_iter` borrows a reader instead and
hands back a [`ValueIter`](text.md), leaving the reader usable afterwards. `Reader` and
`from_reader_iter` are the same two shapes for whitespace-separated values.

The two differ on failure. `LinesReader` reports a bad row and continues with the next
line, since a line is a frame it can resume from; `Reader` has no such boundary and stops
at its first failure.

The bindings split lazy from buffered by name. `load_all` and `loadAllStream` frame the
source as it arrives and decode one row per pull; `loads_all` and `loadsAll` decode a
buffer already in hand. In both languages a failing row raises out of the iterator and
ends it.

## Limits

=== "Rust"

    ```rust
    use yggdryl::{Limits, json};

    let defaults = Limits::default();
    assert_eq!(defaults.max_depth(), 128);
    assert_eq!(defaults.max_input_bytes(), 64 * 1024 * 1024);
    assert_eq!(defaults.max_nodes(), 1_000_000);
    assert_eq!(defaults.max_documents(), 1_024);

    let tight = Limits::new(2, 1024, 8, 1);
    assert!(json::from_slice_with_limits(b"[[0]]", tight).is_ok());
    assert_eq!(
        json::from_slice_with_limits(b"[[[0]]]", tight)
            .unwrap_err()
            .to_string(),
        "invalid json data at byte 2: nesting depth limit exceeded"
    );

    // No caller limit raises the parser's own ceiling.
    let over = format!(
        "{}0{}",
        "[".repeat(json::MAX_PARSER_DEPTH + 1),
        "]".repeat(json::MAX_PARSER_DEPTH + 1)
    );
    let generous = Limits::new(4096, over.len(), 4096, 1);
    assert!(
        json::from_str_with_limits(&over, generous)
            .unwrap_err()
            .to_string()
            .contains("parser hard limit of 384")
    );
    ```

=== "Python"

    ```python
    from yggdryl import json

    reason = None
    try:
        json.loads(b"[" * 129 + b"0" + b"]" * 129)
    except ValueError as error:
        reason = str(error)
    assert reason == "invalid json data at byte 128: nesting depth limit exceeded"

    reason = None
    try:
        json.dumps_all({"id": index} for index in range(1025))
    except ValueError as error:
        reason = str(error)
    assert reason == "codec collection exceeds the 1024-document limit"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    let deep = 0
    for (let index = 0; index < 60; index += 1) deep = [deep]
    assert.throws(() => json.dumps(deep), /exceeds maxDepth 48/)
    assert.throws(() => json.dumps(deep, { maxDepth: 100 }), /between 1 and 48/)

    let shallow = 0
    for (let index = 0; index < 20; index += 1) shallow = [shallow]
    assert.throws(
      () => json.loads(json.dumps(shallow), { maxDepth: 16 }),
      /invalid json data at byte 16: nesting depth limit exceeded/,
    )

    function* many() {
      for (let index = 0; index < 1025; index += 1) yield { id: index }
    }
    assert.throws(() => json.dumpAll(many()), /codec collection exceeds the 1024-document limit/)
    ```

[`Limits`](text.md) is four numbers applied while decoding input the caller does not
control: nesting depth, input bytes, decoded nodes, and documents. Every decoder here has
a `_with_limits` twin that takes them explicitly; the plain one uses `Limits::default()`.
The encoders take no limits argument and check the default depth, so an over-nested value
fails before any bytes are written.

They are enforced while the value is built, not after it exists. Input size is checked
before parsing starts, depth and nodes during, and the document count as each value is
yielded - so a stream stops on the first row that breaks a limit and reports that row's
offset rather than reading to the end first. `MAX_PARSER_DEPTH` is the implementation
ceiling above that: no caller limit is honoured past it, which keeps an adversarial
`max_depth` from turning nesting into stack exhaustion.

The bindings pin ceilings instead of exposing `Limits`. Python applies the same defaults
with no knob to turn. JavaScript takes `maxDepth` on any call and caps it at 48 in both
directions, because encoding there also has to walk a JavaScript object graph before the
codec sees it.

## Failures carry a byte offset

=== "Rust"

    ```rust
    use yggdryl::json;

    // A duplicate is reported at the second key, not at the object.
    assert_eq!(
        json::from_str(r#"{"symbol":"AAPL","symbol":"MSFT"}"#)
            .unwrap_err()
            .to_string(),
        "invalid json data at byte 17: JSON object contains a duplicate key"
    );

    // Row offsets are cumulative over the whole input, not per line.
    assert_eq!(
        json::from_lines_str("{\"id\":1}\n{bad}\n")
            .unwrap_err()
            .to_string(),
        "invalid json data at byte 10: JSON object key must be a string"
    );
    ```

=== "Python"

    ```python
    from yggdryl import json

    reason = None
    try:
        json.loads('{"symbol":"AAPL","symbol":"MSFT"}')
    except ValueError as error:
        reason = str(error)
    assert reason == "invalid json data at byte 17: JSON object contains a duplicate key"

    reason = None
    try:
        list(json.loads_all(b'{"id":1}\n{bad}\n'))
    except ValueError as error:
        reason = str(error)
    assert reason == "invalid json data at byte 10: JSON object key must be a string"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { json } = require('yggdryl')

    assert.throws(
      () => json.loads('{"symbol":"AAPL","symbol":"MSFT"}'),
      /invalid json data at byte 17: JSON object contains a duplicate key/,
    )
    assert.throws(
      () => json.loadsAll('{"id":1}\n{bad}\n'),
      /invalid json data at byte 10: JSON object key must be a string/,
    )
    ```

Every decode failure is `Error::Codec { format: "json", position, reason }`, and
`position` is a byte offset into the input the caller handed over - not a line and
column, and not an offset into some internal read buffer. That holds for the streaming
readers too, where the offset stays cumulative over everything consumed so far. The same
text reaches Python as a `ValueError` and JavaScript as an `Error`, which is why the
three tabs above assert the identical string.

Duplicate object keys are rejected rather than silently collapsed, and keys are compared
after their escapes are decoded, so `"a"` and `"\u0061"` name the same key.

## A compound filename carries the coding

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::text::{dump, load};
    use yggdryl::{Url, Value};

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.json.gz")?.media_type());

    let value = Value::from_mapping([(Value::from("symbol"), Value::from("AAPL"))])?;
    dump(&mut handle, &value)?;

    // The stored bytes really are gzip, and reading them back is symmetric.
    assert_eq!(&handle.as_slice()[..2], &[0x1F, 0x8B]);
    assert_eq!(load(&handle)?, value);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import json

    with tempfile.TemporaryDirectory() as directory:
        path = pathlib.Path(directory) / "trades.json"

        json.dump({"symbol": "AAPL"}, path)
        assert json.load(path) == {"symbol": "AAPL"}

        # A str that is not an existing file is content, not a location.
        assert json.load('{"symbol":"AAPL"}') == {"symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { pathToFileURL } = require('node:url')
    const { json } = require('yggdryl')

    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-json-'))
    try {
      const file = path.join(directory, 'trades.json')

      json.dump({ symbol: 'AAPL' }, file)
      assert.deepEqual(json.load(file), { symbol: 'AAPL' })
      assert.deepEqual(json.load(pathToFileURL(file)), { symbol: 'AAPL' })

      // A string that is not an existing file is content, not a location.
      assert.deepEqual(json.load('{"symbol":"AAPL"}'), { symbol: 'AAPL' })
    } finally {
      fs.rmSync(directory, { force: true, recursive: true })
    }
    ```

An [`IOBase`](io.md) handle already knows where its bytes are and what they are, so
[`text::load` and `text::dump`](text.md) take that one handle and read both halves off
its media type: `trades.json.gz` decompresses and parses without a caller naming gzip,
and dumping back to the same handle recompresses it. `Plan` is the value that pairing
produces - `Plan::infer` reads the declared media type, and `Plan::detect` prefers the
payload's own magic bytes for the coding, so a handle labelled plain `.json` whose bytes
are really gzip still decodes. The codings live in [gzip](gzip.md), [zlib](zlib.md), and
[zstd](zstd.md), selected at runtime by [`Codec`](enums.md).

Python and JavaScript take a location too - a `str`, a `pathlib.Path`, or any
`os.PathLike` on one side, a path string, a `file:` `URL`, or a file descriptor on the
other - and a string that names no existing file is read as content rather than as a
path, which is how the same argument can be either. What they do not apply yet is the
coding half: `trades.json.gz` reaches them as gzip bytes and fails to parse, so
decompress it first or go through the Rust handle.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/json-rust.ipynb){ download },
[Python](notebooks/json-python.ipynb){ download },
[JavaScript](notebooks/json-javascript.ipynb){ download }.

<!-- /notebooks -->
