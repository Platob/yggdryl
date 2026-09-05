# Plain-text records

`TextOptions` reads and writes a handle as line records, converting into the
text variant of [`RecordOptions`](options.md).

## Contract

| option | contract |
| --- | --- |
| `rowheader` | byte regex searched once per line; named captures append nullable columns |
| `lstrip`, `rstrip` | byte regex removed only when its match touches the corresponding body edge |
| `linesep` | exact terminator; unset accepts LF, CRLF, or CR and writes LF |
| `with_rownum` / `withRownum` | optional signed 64-bit first row number; unset omits the column |
| `autotype` | infer capture datatypes from regex syntax before reading; default `true` |
| `timezone` | zone applied when autotyping offset-free timestamps |

## Use

=== "Rust"

    ```rust
    use arrow_array::{Array as _, BinaryArray, Int64Array};
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::{IOBase as _, IOMedia as _};
    use yggdryl::holder::Buffer;
    use yggdryl::media::text::TextOptions;
    use yggdryl::Url;

    let text_source = Buffer::from_bytes(
        b"  [INFO] id=7 first  \r\n[WARN] id=9 second\n".to_vec(),
    )
    .with_media_type(Url::from_str("file:///app.log")?.media_type());

    let mut text_options = TextOptions::new();
    text_options.with_rownum = Some(1);
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
        options.with_rownum = 1
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
    textOptions.withRownum = 1n
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

## Row schema

| column | datatype | value |
| --- | --- | --- |
| `url` | `utf8` | source URL, or an empty string for an unlocated buffer |
| `rownum` | `int64` | present only when `with_rownum` is set; first value is exactly that setting |
| `body` | `binary` | line bytes without the record terminator |

[`DataType::from_regex`](../types/text.md) types captures constrained to
booleans, signed 64-bit integers, finite floats, ISO dates, times, and
datetimes.

## Writes

Writes consume the non-null Binary `body` column and append the terminator.

## Edges

- `rowheader` match -> cut from `body` before `lstrip` / `rstrip`.
- no match -> body kept, captures null.
- strip match off the body edge -> nothing removed.
- `autotype = false` or a broad capture (`\S+`) -> `utf8`.
- empty or unopened resource -> the full schema anyway.
- `body` holding the terminator -> write refused.
- keyed merge -> unsupported; overwrite and append only.
- `app.log.gz` or a mixed folder -> same options; no prior page retained, only the unfinished line fragment.
- `Text` handle -> options only, no line iterator or schema builder.

## Commands

JavaScript numbers include the IPC copy; Python adds an `re` plus PyArrow
baseline.

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::text::tests
    cargo bench -p yggdryl --bench text -- text_records
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/text
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/text
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_text_lines.py
    python/.venv/bin/python python/benchmarks/media/text.py --min-time 0.05 --repeat 3
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    npm run --prefix node bench:media:text -- --records 5000 --iterations 3
    ```
