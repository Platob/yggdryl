# Text read options

`TextOptions` is every setting a plain-text read takes: what a record is, where it is cut, and how its columns are named.

## Contract

| option | contract |
| --- | --- |
| `rowheader` | byte regex searched once per physical line; in framed mode a match starts a record |
| `framing` | join physical lines into logical records; default `false`, and enabling it requires `rowheader` |
| `leading_fragment` / `leadingFragment` | `keep`, `drop`, or `error` for lines before the first framed header; default `keep` |
| `max_record_byte_size` / `maxRecordByteSize` | retained body byte limit per record, counted in bytes as read - below the transport, so the wire where nothing is declared and the decoded text under a coding or a [declared charset](lines.md#declaring-a-charset); unset is unlimited |
| `lstrip`, `rstrip` | byte regex removed only when its match touches the corresponding physical-line body edge |
| `linesep` | exact terminator; unset accepts LF, CRLF, or CR and writes LF |
| `start_rownum` / `startRownum` | optional signed 64-bit first row number; unset omits the column |
| `parse_mtime` / `parseMtime` | emit `mtime`, filled by the row header's `mtime` capture or by the handle's own modification time; default `true` |
| `parse_mimetype` | classify each record and add the `mimetype` column, off by default; Rust only |
| `dedup_adjacent` | drop a record whose body repeats the previous row's, holding one previous digest and never a set; off by default because it gives up row-in / row-out alignment; Rust only |
| `rename_columns` / `renameColumns` | emitted name for a column, keyed by its default name; a key naming no column is refused |
| `lift_names` / `liftNames` | entry paths [lifted](lines.md#lifting-an-entry-into-a-column) into columns of their own, each named by its `as` alias where it writes one; unset lifts nothing beyond the row header's captures |
| `autotype` | infer capture datatypes from regex syntax before reading; default `true` |
| `timezone` | zone applied when autotyping offset-free timestamps |
| `batch_row_size` / `batchRowSize`, `batch_byte_size` | the [shared batch targets](../options.md), a batch closing on whichever it reaches first; a text read states `35 * 1024` rows and 64 MiB where the options state none |

`Text<H>` is the stateful form: `into_text` / `intoText` binds one `TextOptions`
to a handle, and every record call on it reads by those options without
naming them again.

## Use

The `select` and `where` sections of the options shape a text read as they shape every record read: the row header's captures are columns a `select` reads, a cast or an alias reshapes them, and a `where` naming an alias runs after the projection. The properties beside `options` set the sections without building options first.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::text::TextOptions;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOMedia, Url};

    let source = Buffer::from_bytes(b"[INFO] id=7 first\n[WARN] id=9 second\n[INFO] id=11 third\nplain\n".to_vec())
        .with_media_type(Url::from_str("file:///app.log")?.media_type());
    let mut options = TextOptions::new().try_with_rowheader(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)")?;
    options.start_rownum = Some(1);
    let options = options
        .with_select("cast(rownum as int32) as n, trim(body) as line, level, id * 10 as tenfold")?
        .with_filter("n > 1 and line like '%d' and level is not null")?;

    let batches = source
        .read_arrow_reader(&options.into())?
        .collect::<Result<Vec<_>, _>>()?;
    let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
    assert_eq!(rows, 2);
    assert_eq!(batches[0].schema().field(1).name(), "line");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    path = pathlib.Path(tempfile.mkdtemp()) / "app.log"
    path.write_bytes(b"[INFO] id=7 first\n[WARN] id=9 second\n[INFO] id=11 third\nplain\n")
    table = IOBase(path).read_arrow_reader(
        rowheader=r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)",
        start_rownum=1,
        select="cast(rownum as int32) as n, trim(body) as line, level, id * 10 as tenfold",
        filter="n > 1 and line like '%d' and level is not null",
    ).read_all()
    assert table.schema.names == ["n", "line", "level", "tenfold"]
    assert table.column("line").to_pylist() == ["[WARN] id=9 second", "[INFO] id=11 third"]
    assert table.column("tenfold").to_pylist() == [90, 110]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes(
      Buffer.from('[INFO] id=7 first\n[WARN] id=9 second\n[INFO] id=11 third\nplain\n'),
    )
    handle.mediaType = 'text/plain'
    const table = handle
      .readArrowReader({
        rowheader: '\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+)',
        startRownum: 1n,
        select: 'cast(rownum as int32) as n, trim(body) as line, level, id * 10 as tenfold',
        filter: "n > 1 and line like '%d' and level is not null",
      })
      .intoTable()
    assert.deepEqual(table.schema.fields.map((field) => field.name), ['n', 'line', 'level', 'tenfold'])
    assert.deepEqual([...table.getChild('line')], ['[WARN] id=9 second', '[INFO] id=11 third'])
    assert.deepEqual([...table.getChild('tenfold')], [90n, 110n])
    ```

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
| unbounded `body` | the exact source bytes after first-line header removal and normalization, [as text](lines.md#a-line-is-text) |
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
| what the bound counts | the body in bytes as read, including normalized separators - the bytes below the transport: the wire where nothing is declared, never the [text a stray byte becomes](lines.md#a-line-is-text), and the decoded text under a coding or a [declared charset](lines.md#declaring-a-charset), which is what the transport handed up |
| omitted bytes | reported in `dropped_byte_size`, in the same units |
| `max_row_size`, `max_byte_size` | total result rows and total Arrow result memory; independent and unchanged |

## A line with no body is no line

`body` is the line, so a record that states no byte of its own is not a row:
a blank line, and one the `lstrip`/`rstrip` patterns take whole, is a
separator between records rather than a record, and the reader goes past it.
The numbering does not close over the gap - `rownum` is the physical line's
own - and a count answers exactly what a read answers, because what makes a
line a record is what it cut and never what `max_record_byte_size` kept.

The same rule holds at every other door. [`TextLine`](lines.md#lines) refuses an empty
body wherever one is set, a batch read back into lines is refused at `body`
where a row's cell is absent, null or empty, and a write refuses a row whose
body states nothing - writing one would put a blank line in the object that
reads back as no row at all. A `max_record_byte_size` of `0` with no
`rowheader` is refused before a byte is read, because with nothing to retain
a record by it would answer a line with no body on every row; under a
`rowheader` the header is always retained and the limit reads.

## Edges

- `framing` without `rowheader` -> refused.
- physical-line mode, no match -> body kept, captures null.
- `leading_fragment = error` -> the first physical line before any header fails the read.
- `max_record_byte_size = 0` with a `rowheader` -> valid; the header is retained whole, the body past it is empty and every byte of it is counted as dropped.
- `max_record_byte_size = 0` with no `rowheader` -> refused before a byte is read: there is nothing to retain a record by, and [a line with no body is no line](#a-line-with-no-body-is-no-line).
- a blank line, or one the strips take whole -> no row: it is a separator, the numbering keeps its gap, and a count answers what a read answers.
- `max_record_byte_size` unset -> no `dropped_byte_size` column; set but never exceeded -> null.
- strip match off the physical-line body edge -> nothing removed.
- `autotype = false` or a broad capture (`\S+`) -> `utf8`.
- a rename onto a name another column already emits -> refused when the option is set, naming both.
- a key of `rename_columns` naming no column -> refused.
- `Text` handle -> options only, no line iterator or schema builder.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text -- arrow::text batch::text bytes::text::values entry::text::values handle::text leading::text limits::text line::text options::text plan::columns reader::text sep::text
    cargo bench -p yggdryl --bench text -- text_options
    cargo bench -p yggdryl --bench text -- text_record_framing
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/test_init.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/records.test.js
    ```
