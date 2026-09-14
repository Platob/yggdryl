# Plain-text lines

One line per row, in and out: the record surface a `text/plain` handle answers without an Arrow type in the call.

## Contract

| Key | Value |
| --- | --- |
| Reads | Python `read_records`, JavaScript `readRecords` - one row per physical line, or per framed record; Rust reads Arrow and crosses with `ArrowScalar::into_scalar` |
| Writes | `overwrite_records`, `append_records`; each row's non-null `utf8` `body` becomes one line plus the terminator |
| Row | the [row schema](index.md#row-schema): `sourceurl`, `rownum`, `body`, `mtime`, plus every row-header capture and lifted entry |
| Terminator | `linesep` when pinned, otherwise LF on write; a read accepts LF, CRLF, or CR |
| Charset | the body crosses in the charset the handle's media type [declares](index.md#declaring-a-charset) |
| Merge | refused: a line has no row identity |

## Use

A write consumes the `body` column and adds the terminator; a read hands the line back under the same name.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, Scalar, Url};

    struct Line(&'static str);

    impl From<Line> for Scalar {
        fn from(row: Line) -> Self {
            Scalar::from_sequence([Scalar::from(row.0)])
        }
    }

    let body = DataType::from_fields([DataType::utf8().required_field("body")])?
        .required_field("row");
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///app.log")?.media_type());
    let options = handle.record_options()?.with_field(body);

    handle.overwrite_records([Line("first"), Line("second")], &options)?;
    handle.append_records([Line("third")], &options)?;
    assert_eq!(handle.read_all_bytes()?, b"first\nsecond\nthird\n");

    // Rust reads Arrow, then crosses into the value model: one ordered row
    // sequence per line, under the full row schema.
    let rows = handle.read_arrow(None)?.into_scalar()?;
    assert_eq!(rows.len(), 3);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, TextOptions

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "app.log")
    options = TextOptions()

    handle.overwrite_records([{"body": "first"}, {"body": "second"}], options=options)
    handle.append_records([{"body": "third"}], options=options)
    assert handle.read_bytes() == b"first\nsecond\nthird\n"

    rows = list(handle.read_records(options=options))
    assert [row["body"] for row in rows] == ["first", "second", "third"]
    # `start_rownum` is unset, so there is no `rownum` column to read.
    assert "rownum" not in rows[0]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, TextOptions } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-'))
    const handle = new IOBase(path.join(root, 'app.log'))
    const options = new TextOptions()

    handle.overwriteRecords([{ body: 'first' }, { body: 'second' }], options)
    handle.appendRecords([{ body: 'third' }], options)
    assert.equal(handle.readText(), 'first\nsecond\nthird\n')

    const rows = [...handle.readRecords(options)]
    assert.deepEqual(rows.map((row) => row.body), ['first', 'second', 'third'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Row numbers and captures

`start_rownum` numbers the first physical line of each record and adds the `rownum` column; unset, the column is absent. A [row header](index.md#lines) adds one column per capture, typed from the regex when `autotype` is on, and a [lifted entry](index.md#lifting-an-entry-into-a-column) adds one more. Every added column reaches a record row under the name it is emitted as.

## Edges

- keyed merge -> refused; a line has no row identity, so overwrite and append are the two intents.
- `body` holding the terminator -> write refused.
- a row without a `body` column -> refused naming what was expected.
- `read_records` -> Python and JavaScript only; the Rust primitive read surface stays Arrow-native.
- Python `read_records()` over a file whose modification time is finer than a microsecond -> the `datetime` a record hands back is floored to the microsecond it can hold, never refused. The batch path carries the full nanosecond reading.
- absent resource -> no rows, not an error.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test media text::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_text_lines.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/text.test.js
    ```
