# Writing plain-text records

One line per row, out of native scalars or out of Arrow batches, each body plus the terminator.

## Contract

| Key | Value |
| --- | --- |
| Owns | `overwrite_records`, `append_records` and `overwrite_arrow_*`, `append_arrow_*` from [`IOMedia`](../../holder/iobase/records.md) |
| Native rows | each row's non-null `utf8` `body` becomes one line plus the terminator; every other column of the [row schema](index.md#row-schema) is read past |
| Batches | the `body` column in any layout Arrow spells a string in - `utf8`, `large_utf8`, `utf8_view`, or any of those behind a dictionary, unpacked once per batch |
| Intents | overwrite and append; keyed merge is refused, a line having no row identity |
| Terminator | `linesep` when pinned, otherwise LF; an append compares the tail with the terminator as the charset spells it |
| Charset | bodies are rendered through the charset the handle's media type [declares](lines.md#declaring-a-charset), inside the coding writer |
| Refused | a `binary` body column, a null or empty body, a body holding the terminator, a row with no `body` column at all |
| Header | a read takes the [row header](index.md#row-schema) off the body and states its captures as columns, so a write emits the body without it: reading and writing one object back is the payload, not the line |

## Use

A write consumes the `body` column and adds the terminator; a read hands the line back under the same name.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, Scalar, StructType, Url};

    struct Line(&'static str);

    impl From<Line> for Scalar {
        fn from(row: Line) -> Self {
            Scalar::from_sequence([Scalar::from(row.0)])
        }
    }

    let body = DataType::from(StructType::from_fields([DataType::utf8().required_field("body")])?)
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

## Rows as Arrow batches

The same two intents take a batch instead of rows, and read the same one column out of it.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, RecordBatch, StringArray};
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::{IOBase as _, IOMedia as _};
    use yggdryl::holder::Buffer;
    use yggdryl::Url;

    let mut target = Buffer::new().with_media_type(Url::from_str("file:///app.log")?.media_type());
    let options = target.record_options()?;

    let bodies = |values: Vec<&'static str>| {
        RecordBatch::try_from_iter([("body", Arc::new(StringArray::from(values)) as ArrayRef)])
    };
    target.overwrite_arrow_batch(bodies(vec!["first", "second"])?, &options)?;
    target.append_arrow_batch(bodies(vec!["third"])?, &options)?;

    assert_eq!(target.read_all_bytes()?, b"first\nsecond\nthird\n");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    target = IOBase(pathlib.Path(tempfile.mkdtemp()) / "app.log")

    target.overwrite_arrow_table(pa.table({"body": ["first", "second"]}))
    target.append_arrow_table(pa.table({"body": ["third"]}))

    assert target.read_bytes() == b"first\nsecond\nthird\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase, TextOptions } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-'))
    const target = new IOBase(path.join(root, 'app.log'))
    const options = new TextOptions()
    const bodies = (values) =>
      new arrow.Table({ body: arrow.vectorFromArray(values, new arrow.Utf8()) })

    target.overwriteArrowTable(bodies(['first', 'second']), options)
    target.appendArrowTable(bodies(['third']), options)

    assert.equal(target.readText(), 'first\nsecond\nthird\n')

    fs.rmSync(root, { recursive: true, force: true })
    ```

## One column is read, and it is text

Writes stay physical-line operations, consuming the non-null `utf8` `body`
column - `utf8`, `large_utf8` or `utf8_view`, or any of those behind a
dictionary, which is what Arrow JS infers for a plain record's string and is
unpacked once per batch - one spelling here - and appending
the terminator, both written in the charset the handle's media type
[declares](lines.md#declaring-a-charset) and as they are under UTF-8 or US-ASCII. A
batch carrying a `binary` body is refused naming what was
expected: a `binary` column may hold anything, and rendering one would write
bytes no reader of the file could read back as the rows they were.

## Edges

- keyed merge -> refused; a line has no row identity, so overwrite and append are the two intents.
- a `binary` `body` column -> write refused: `expected a utf8 body column, got Binary`.
- a row or batch with no `body` column at all -> refused naming what was expected.
- a null or empty `body` cell -> refused: writing one would put a blank line in the object that reads back as no row at all, and [a line with no body is no line](options.md#a-line-with-no-body-is-no-line).
- `body` holding the terminator -> write refused; with `linesep` unset any LF or CR in a body is the refusal, since a read would split there.
- a dictionary-encoded `utf8` body -> accepted, unpacked once per batch.
- a declared charset -> the body is encoded back into it, or a declared handle would read its own UTF-8 back as legacy bytes.
- a row read under a `rowheader` and written back out -> the header the read took off the body is not written again: a write states the `body` column and nothing derived from it, and a line the header consumed whole states an empty body, which a write refuses.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text -- arrow::text batch::text bytes::text::values entry::text::values handle::text leading::text limits::text line::text options::text plan::columns reader::text sep::text
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/text
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/test_init.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/records.test.js
    ```
