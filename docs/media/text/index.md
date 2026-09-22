# Plain-text records

`TextOptions` reads a handle as physical-line or framed records and writes one
line per row; it converts into the text variant of [`RecordOptions`](../options.md).

## Contract

| Key | Value |
| --- | --- |
| Owns | `TextOptions`, [`TextLine`](lines.md), `read_text_lines`, `Text<H>`, and the Rust line/batch converters `into_arrow_batch` / `into_arrow_reader` / `from_arrow_batch` / `from_arrow_reader` |
| Handle surface | `overwrite_*`, `append_*`, `read_arrow_reader`, `read_arrow_field` from [`IOMedia`](../../holder/iobase/records.md); `into_text` / `intoText` binds one `TextOptions` to a handle |
| Surfaces | native rows through `*_records` and Arrow batches through `*_arrow_*`; `read_text_lines` is the one decode both go through |
| Record | one physical line, or one logical record where [`framing`](options.md#framing) joins them |
| Row | the [row schema](#row-schema): the nineteen [event columns](../../graph.md#columns), then the enabled line columns - `sourceurl`, `rownum` under `start_rownum`, `mtime` under `parse_mtime`, `mimetype` under `parse_mimetype`, `body`, `dropped_byte_size` under `max_record_byte_size` - then every row-header capture and lifted entry |
| Schema | complete before a byte is read: `autotype` types the captures from the regex and a [lifted path](lines.md#lifting-an-entry-into-a-column) declares its column whether or not a row carries it |
| Terminator | `linesep` when pinned, otherwise LF on write; a read accepts LF, CRLF, or CR |
| Charset | bodies and captures cross in the charset the handle's media type [declares](lines.md#declaring-a-charset) |
| Lazy | a batch is built as the reader is stepped, and every reading of a line is resolved on its first ask, once |
| Merge | refused: a line has no row identity, so overwrite and append are the two intents |
| Batch targets | the [shared targets](../options.md), a batch closing on whichever it reaches first; a text read states `35 * 1024` rows and 64 MiB where the options state none |
| Bindings | Rust: `TextOptions`, `TextLine`, `Text<H>`, `read_text_lines`, the four converters; Python: `TextOptions`, `read_records`, `read_text_lines`; JavaScript: `TextOptions`, `readRecords`, `readTextLines` |

## Pages

Two surfaces answer the same lines - native rows, with no Arrow type in the
call, and Arrow batches - and each page takes both, split by direction, so one
page answers how lines are read and one how they are written.

| Page | Owns |
| --- | --- |
| [Read](read.md) | lines as native scalars, lines as Arrow batches, row numbers and captures, batches read back into lines |
| [Write](write.md) | overwrite and append, as native rows and as batches |
| [Lines](lines.md) | `TextLine`: the body as text, the declared charset, the entry tree, lifted columns |
| [Options](options.md) | every `TextOptions` setting: shaping, framing, bounded records |

The three [structured schemes](../structured.md) parse one `Scalar` over the
same machinery; a `text/plain` handle is records instead.

## Use

=== "Rust"

    ```rust
    use arrow_array::{Array as _, Int64Array, StringArray};
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::{IOBase as _, IOMedia as _};
    use yggdryl::holder::Buffer;
    use yggdryl::text::TextOptions;
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
    assert_eq!(text_batch.schema().fields().len(), 25);
    assert_eq!(
        text_batch
            .column(20)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values(),
        &[1, 3],
    );
    assert_eq!(
        text_batch
            .column(22)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "[INFO] id=7 first\n detail A",
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
            "[INFO] id=7 first\n detail A",
            "[WARN] id=9 second\n detail B",
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
      textRows.map((row) => row.body),
      ['[INFO] id=7 first\n detail A', '[WARN] id=9 second\n detail B'],
    )
    assert.deepEqual(textRows.map((row) => row.id), [7n, 9n])

    fs.rmSync(textRoot, { recursive: true, force: true })
    ```

## Row schema

The source field is complete before any source bytes are read. It opens with the nineteen [event columns](../../graph.md#columns) in `EventColumn::ALL` order - a line is an [event](lines.md#lines) of the graph, and the FIX row parsed out of it contains the same nineteen under the same names and datatypes, so a message's `srcuuids` joins the line's `curruuid` without a mapping - then the line's own columns.

| column | datatype | value |
| --- | --- | --- |
| `currunix` | `datetime64(ns, UTC)` | required; when the line happened: a stated instant, else `mtime`, else the epoch |
| `creaunix` | `datetime64(ns, UTC)` | nullable; a `creaunix` capture as an instant, else what a walk folded, else null |
| `execunix` | `datetime64(ns, UTC)` | nullable; an `execunix` capture as the precise execution instant, else null |
| `recdunix` | `datetime64(ns, UTC)` | nullable; a `recdunix` capture as the precise recording instant, else null |
| `refrecdunix` | `datetime64(ns, UTC)` | nullable; a `refrecdunix` capture or event value naming the observation selected as merge reference, else null; a graph merge persists the latest such recording clock here while `recdunix` keeps the earliest observation |
| `exprtime` | `datetime64(ns, UTC)` | nullable; an `exprtime` capture as an instant, else what a walk folded, else null |
| `prevunix` | `datetime64(ns, UTC)` | nullable; a `prevunix` capture, else what a walk stamped, else null |
| `snapunix` | `datetime64(ns, UTC)` | nullable; a `snapunix` capture, else what a grid stamped, else null |
| `curruuid` | `uuid` | required; the line's identity, the UUIDv7 its microsecond `currunix` and `currhashcode` derive; the nil identity where the instant has no UUIDv7 |
| `crossuuid` | `uuid` | required; the UUIDv8 of `crosshashcode`, or `curruuid` where the line names no cross code |
| `crosscode` | `utf8` | nullable; an explicit event value, else the canonical text of `sourceurl`, else null for an unlocated line |
| `currhashcode` | `uint64` | required; the XXH3-64 of the cross code, the row number and the body |
| `crosshashcode` | `uint64` | required; the XXH3-64 of `crosscode`, zero where none |
| `prevuuid` | `uuid` | nullable; a `prevuuid` capture, else what a walk stamped, else null |
| `seqnum` | `uint64` | nullable; the row number under `start_rownum`, else the zero-based physical index, unless an event value was stated explicitly; null where zero |
| `parentuuids` | `list<uuid>` | nullable; what a walk stated, else null |
| `srcuuids` | `list<uuid>` | nullable; what was stated, else null: a line read from a handle has no source |
| `identifiers` | `map<utf8, utf8>` | nullable; every named capture the line matched, under its name, sorted; null where none |
| `state` | `state` | nullable, and never null on a row the reader wrote: a `state` capture, else `00UNKNOWN` |
| `sourceurl` | `url` | nullable; the source location, and null for an unlocated buffer |
| `rownum` | `int64` | required, and present only when `start_rownum` is set; first value is exactly that setting |
| `mtime` | `datetime64(ns, UTC)` | nullable; present unless `parse_mtime` is off |
| `mimetype` | `utf8` | required, and present only with `parse_mimetype` |
| `body` | `utf8` | required, and never empty; the whole retained record as text, the row header included and the edges stripped: decoded at the transport under [a declared charset](lines.md#declaring-a-charset), else [where the line is made](lines.md#a-line-is-text) |
| `dropped_byte_size` | `uint64` | nullable; present only with `max_record_byte_size`, and non-null only when bytes were dropped; counts bytes as read, in the units the limit counts |

Every column says what it holds, and the nineteen a line opens with carry the
same spelling the FIX row shows them under, so one fact is named one way
wherever it is read. `required` is a promise the reader keeps in both
directions: a required column is one a line can always state, and a batch read
back into lines is refused where a required cell is null.

Named `rowheader` captures follow these columns and stay nullable in both modes,
and the columns `lift_names` [lifts](lines.md#lifting-an-entry-into-a-column) follow
the captures. A capture named for an event fact the line reads from its header -
`state`, `prevuuid`, `creaunix`, `execunix`, `recdunix`, `refrecdunix`, `exprtime`, `prevunix`,
or `snapunix` - feeds that column and appears beside nothing, as an `mtime`
capture feeds `mtime`. A capture named for a fact the line derives is refused:
`seqnum` belongs to `rownum` / the physical index, `crosscode` belongs to
`sourceurl`, and `currunix`, `curruuid`, `crossuuid`, `currhashcode`,
`crosshashcode`, `parentuuids`, `srcuuids`, and `identifiers` are reserved the
same way. The check ignores case.
[`DataType::from_regex`](../../types/text/string.md#regex-captures) types captures constrained to
booleans, signed 64-bit integers, finite floats, ISO dates, times, and
datetimes. `yggdryl::ULBRIDGE_ROWHEADER` is the header a bridge log writes,
its captures named for the [FIX columns they fill](../../fix/arrow.md#a-bridge-log-names-what-it-fills)
when the read goes on into a FIX batch.

### Classifying each record

`mtime` says when the record was written, and has two sources for one column. A row header that declares an `mtime` capture dates each line from the line itself, and the capture fills the column rather than appearing beside it — so a capture spelled that way is read at `datetime64(ns, UTC)` whatever its own syntax suggests, and a reading that names no offset is resolved through `timezone`. A header that declares no such capture, or a line the header did not match, falls back to [`IOBase::mtime`](../../holder/iobase/bytes.md#modification-time) — the handle's own modification time, read once per read and shared by every row. Neither available is null, which is what an unlocated buffer answers. Turning `parse_mtime` off removes the column, and frees the name for an ordinary capture.

The classification column is the [capture reading](../../fix/registry.md#classifying-a-captured-line) run over each record's body: what the line is. It needs no dictionary and costs one shallow scan per record, which is why it is opt-in — a read that only needs rows should not pay for it. Which way a line moved is FIX's own fact, tag 385, and the [codec](../../fix/decode.md) reads it from the prose in front of the payload; the reader states no direction of its own and takes nothing off the body.

The reader states no message type of its own either: a line's type is what its frame says, and reading a frame is the [codec's](../../fix/decode.md) work rather than the classifier's. The scan stops at the first frame it locates, so on a line carrying two of them the `mimetype` a row holds — and the message code that same reading infers — describes the first frame and no other, which is what a classifier can answer without parsing.

[FIX decoding](../../fix/decode.md) consumes these captured records through a lazy
`FixMessages` iterator, which answers none, one or many messages a record: a
line carrying two frames yields both, a JSON document yields one message
stating nothing whatever the document names, and a line the codec finds no
message in yields none. Each retains the originating capture columns, and a
[FIX batch](../../fix/arrow.md#one-row-per-message) is therefore one row per
message. The text reader is what answers one row per line: it emits every
framed record, whatever the codec would go on to make of it.

Measured in [Classifying a capture](../../fix/registry.md#classifying-a-capture).

## Edges

- keyed merge -> refused; a line has no row identity, so overwrite and append are the two intents.
- classification columns ahead of the captures -> the captures keep the types their patterns gave them; a `thread` capture is `utf8` whatever the classification read before it.
- an unlocated buffer -> `sourceurl`, `crosscode`, and `mtime` are null: a buffer has no location and records no modification time; `crosshashcode` is zero and `crossuuid` is the line's own identity.
- an event fact nothing states -> the column's null, but for the ones never null: `currunix` the epoch, `curruuid` the nil identity where the instant has no UUIDv7, `crossuuid` the line's own identity, `currhashcode` the code of the cross code, the row and the body, `crosshashcode` zero; and `state`, nullable, is written `00UNKNOWN` all the same.
- a row-header capture named `seqnum` or `crosscode`, in any case -> refused when the header is set, because the reader owns those facts through `rownum` / `index` and `sourceurl`.
- a `state`, `prevuuid`, or instant capture the line cannot read at the fact's datatype -> a refusal naming the row, the object and the fact, on the batch as on the reading.
- a negative calculated `rownum` -> cannot supply the event's unsigned `seqnum` and therefore refuses a line batch; the infallible event door falls back to the physical index.
- a row header declaring an `mtime` capture with `parse_mtime` off -> an ordinary capture, typed by its own syntax.
- a line batch written to an [Iceberg](../iceberg/index.md) table -> the schema goes through `into_scheme_compat(&Scheme::ICEBERG)` first, as a FIX row's does: the two `uint64` codes have no Iceberg type and widen to `decimal(20, 0)`.
- reading, absence and transports -> [Read](read.md#edges); writes -> [Write](write.md#edges); the value tree and the charset -> [Lines](lines.md#edges); every setting -> [Options](options.md#edges).

## Commands

The `text_record_framing` group compares physical-line and framed reads over
short, multiline, and oversized 4 KiB-capped corpora. JavaScript numbers include
the IPC copy; Python adds an `re` plus PyArrow baseline.

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test text -- batch display io line::internal loading reader::internal
    cargo test --features "parquet iceberg" -p yggdryl --test text -- arrow::text batch::text bytes::text::values entry::text::values handle::text leading::text limits::text line::text options::text plan::columns reader::text sep::text
    cargo bench -p yggdryl --bench text -- text_records
    cargo bench -p yggdryl --bench text -- text_record_framing
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/text
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/text
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/test_init.py
    python/.venv/bin/python python/benchmarks/media/text.py --min-time 0.05 --repeat 3
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/records.test.js
    npm run --prefix node bench:media:text -- --records 5000 --iterations 3
    ```
