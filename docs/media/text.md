# Plain-text records

`TextOptions` reads a handle as physical-line or framed records, writes one line
per row, and converts into the text variant of [`RecordOptions`](options.md).

## Contract

| option | contract |
| --- | --- |
| `rowheader` | byte regex searched once per physical line; in framed mode a match starts a record |
| `framing` | join physical lines into logical records; default `false`, and enabling it requires `rowheader` |
| `leading_fragment` / `leadingFragment` | `keep`, `drop`, or `error` for lines before the first framed header; default `keep` |
| `max_record_byte_size` / `maxRecordByteSize` | retained decoded-body byte limit per record; unset is unlimited |
| `lstrip`, `rstrip` | byte regex removed only when its match touches the corresponding physical-line body edge |
| `linesep` | exact terminator; unset accepts LF, CRLF, or CR and writes LF |
| `start_rownum` / `startRownum` | optional signed 64-bit first row number; unset omits the column |
| `parse_mtime` / `parseMtime` | emit `mtime`, filled by the row header's `mtime` capture or by the handle's own modification time; default `true` |
| `parse_mimetype`, `parse_msgtype`, `parse_direction` | Rust-only; classify each record and add the column named, off by default |
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
        b"[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B".to_vec(),
    )
    .with_media_type(Url::from_str("file:///app.log")?.media_type());

    let mut text_options = TextOptions::new();
    text_options.start_rownum = Some(1);
    text_options.set_rowheader(Some(r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+) "))?;
    text_options.set_framing(true);
    let text_source = text_source.into_text_with(text_options);
    let record_options = text_source.record_options()?;

    let text_batch = text_source
        .read_arrow_reader(&record_options)?
        .next()
        .unwrap()?;
    assert_eq!(text_batch.schema().fields().len(), 6);
    assert_eq!(
        text_batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values(),
        &[1, 3],
    );
    assert_eq!(
        text_batch
            .column(3)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .value(0),
        b"first\n detail A",
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, TextOptions

    with tempfile.TemporaryDirectory() as directory:
        source = pathlib.Path(directory) / "app.log"
        source.write_bytes(
            b"[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B"
        )

        options = TextOptions()
        options.start_rownum = 1
        options.rowheader = r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+) "
        options.framing = True

        handle = IOBase(source).into_text(options)
        rows = list(handle.read_records())
        assert [row["rownum"] for row in rows] == [1, 3]
        assert [row["body"] for row in rows] == [
            b"first\n detail A",
            b"second\n detail B",
        ]
        assert [row["id"] for row in rows] == [7, 9]
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
    fs.writeFileSync(
      textSource,
      '[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B',
    )

    const textOptions = new TextOptions()
    textOptions.startRownum = 1n
    textOptions.rowheader = '^\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+) '
    textOptions.framing = true

    const textHandle = new IOBase(textSource).intoText(textOptions)
    const textRows = [...textHandle.readRecords()]
    assert.deepEqual(textRows.map((row) => row.rownum), [1n, 3n])
    assert.deepEqual(
      textRows.map((row) => Buffer.from(row.body).toString()),
      ['first\n detail A', 'second\n detail B'],
    )
    assert.deepEqual(textRows.map((row) => row.id), [7n, 9n])

    fs.rmSync(textRoot, { recursive: true, force: true })
    ```

## Row schema

The source field is complete before any source bytes are read.

| column | datatype | value |
| --- | --- | --- |
| `url` | `url` | nullable; the source location, and null for an unlocated buffer |
| `rownum` | `int64` | present only when `start_rownum` is set; first value is exactly that setting |
| `mtime` | `datetime64(ns, UTC)` | nullable; present unless `parse_mtime` is off |
| `direction` | `msgdirection` | nullable; present only with `parse_direction` |
| `mimetype` | `utf8` | present only with `parse_mimetype` |
| `msgtype` | `utf8` | nullable; present only with `parse_msgtype`; complete wire code |
| `body` | `binary` | required retained record bytes |
| `dropped_byte_size` | `uint64` | nullable; present only with `max_record_byte_size`, and non-null only when bytes were dropped |

Named `rowheader` captures follow these columns and stay nullable in both modes.
[`DataType::from_regex`](../types/text.md) types captures constrained to
booleans, signed 64-bit integers, finite floats, ISO dates, times, and
datetimes.

### Classifying each record

`mtime` says when the record was written, and has two sources for one column. A row header that declares an `mtime` capture dates each line from the line itself, and the capture fills the column rather than appearing beside it — so a capture spelled that way is read at `datetime64(ns, UTC)` whatever its own syntax suggests, and a reading that names no offset is resolved through `timezone`. A header that declares no such capture, or a line the header did not match, falls back to [`IOBase::mtime`](../holder/iobase/bytes.md#modification-time) — the handle's own modification time, read once per read and shared by every row. Neither available is null, which is what an unlocated buffer answers. Turning `parse_mtime` off removes the column, and frees the name for an ordinary capture.

The three classification columns are the [capture readings](../fix/registry.md#classifying-a-captured-line) run over each record's body: what the line is, the message type it declares, and which way it moved. They need no dictionary and cost one shallow scan per record, which is why they are opt-in per column — a read that only needs rows should not pay for them. Rust only: neither binding reaches these flags today.

`parse_direction` also takes the marker off the body, because a verb in front of the payload is transport prose rather than payload. The `msgtype` column preserves the complete UTF-8 code, including `ConfigurationPlugin` and composite codes such as `P Report Ack`. [Registry-owned message definitions](../fix/registry.md) resolve these codes to their Struct fields.

[FIX decoding](../fix/decode.md) consumes these captured records through a lazy
`FixMessages` iterator. One bulk ULconfig response can yield several flat
messages; each retains the originating capture columns. The text reader itself
still emits one row per framed record.

Measured in [Classifying a capture](../fix/registry.md#classifying-a-capture).

## Framing

`framing` joins physical lines into one logical record. A `rowheader` match
closes the active record and starts the next one.

| in framed mode | result |
| --- | --- |
| first line | the complete `rowheader` match is removed from `body` |
| later nonmatching lines | appended to the same `body`, separated by one `\n` |
| LF, CRLF, or CR terminator | normalized to that separator, adding no trailing byte |
| EOF without a final terminator | the active record is still emitted |
| end of a handle or folder leaf | framing state ends, so records never join across source objects |
| `rownum` | the record's first physical line number, a kept leading fragment included |
| unbounded `body` | the exact source bytes after first-line header removal and normalization |
| `lstrip`, `rstrip` | cut from each physical line before it is joined |

Rust selects `LeadingFragment::{Keep, Drop, Error}`; Python and JavaScript use
the corresponding lowercase property values.

| `leading_fragment` | lines before the first framed header |
| --- | --- |
| `keep` (default) | emitted as one record with null captures |
| `drop` | drained |
| `error` | fails on the first physical line |

## Bounded records

`max_record_byte_size` emits the exact bounded prefix and drains the rest
without retaining it.

| fact | value |
| --- | --- |
| what the bound counts | the decoded body, including normalized separators |
| omitted bytes | reported in `dropped_byte_size` |
| `max_row_size`, `max_byte_size` | total result rows and total Arrow result memory; independent and unchanged |

## Writes

Writes stay physical-line operations, consuming the non-null Binary `body`
column and appending the terminator.

## Edges

- `framing` without `rowheader` -> refused.
- physical-line mode, no match -> body kept, captures null.
- `leading_fragment = error` -> the first physical line before any header fails the read.
- `max_record_byte_size = 0` -> valid; empty prefix, whole body counted as dropped.
- `max_record_byte_size` unset -> no `dropped_byte_size` column; set but never exceeded -> null.
- strip match off the physical-line body edge -> nothing removed.
- `autotype = false` or a broad capture (`\S+`) -> `utf8`.
- an unlocated buffer -> `url` and `mtime` are both null: a buffer has no location and records no modification time, and neither the empty string nor a clock reading is one.
- Python `read_records()` over a file whose modification time is finer than a microsecond -> the `datetime` a record hands back is floored to the microsecond it can hold, never refused. The batch path carries the full nanosecond reading.
- a row header declaring an `mtime` capture with `parse_mtime` off -> an ordinary capture, typed by its own syntax.
- empty, missing, compressed, local, or foreign Arrow-filesystem resource -> the full schema before iteration.
- `body` holding the terminator -> write refused.
- keyed merge -> unsupported; overwrite and append only.
- `app.log.gz` or a folder mixing plain, gzip, and zstd leaves -> same options, one stream, no reopened handle and no retained prior page. The transport is read one [fetch window](../holder/iobase/bytes.md#fetch-window) at a time, whatever the decoder pulls.
- `Text` handle -> options only, no line iterator or schema builder.

## Commands

The `text_record_framing` group compares physical-line and framed reads over
short, multiline, and oversized 4 KiB-capped corpora. JavaScript numbers include
the IPC copy; Python adds an `re` plus PyArrow baseline.

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::text::tests
    cargo bench -p yggdryl --bench text -- text_records
    cargo bench -p yggdryl --bench text -- text_record_framing
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
