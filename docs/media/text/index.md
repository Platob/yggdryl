# Plain-text records

`TextOptions` reads a handle as physical-line or framed records and writes one
line per row; it converts into the text variant of [`RecordOptions`](../options.md).

## Contract

| Key | Value |
| --- | --- |
| Owns | `TextOptions`, [`TextLine`](lines.md), `read_text_lines`, `Text<H>`, and the Rust line/batch converters `into_arrow_batch` / `into_arrow_reader` / `from_arrow_batch` / `from_arrow_reader` |
| Handle surface | `overwrite_*`, `append_*`, `read_arrow_reader`, `read_arrow_field` from [`IOMedia`](../../holder/iobase/records.md); `into_text` / `intoText` binds one `TextOptions` to a handle |
| Surfaces | native rows through `*_records` and Arrow batches through `*_arrow_*`; `read_text_lines` is the one decode both go through |
| Clauses | the `where`, the `select` and the row bounds are answered by the record surfaces, over the rows the lines become; the decode itself yields every line it cuts |
| Record | one physical line, or one logical record where [`framing`](options.md#framing) joins them |
| Row | the [row schema](#row-schema): the nineteen [event columns](../../graph.md#columns), then the enabled line columns - `mimetype` under `parse_mimetype`, `body`, `dropped_byte_size` under `max_record_byte_size` - then every row-header capture. The object a line came from is `crosscode`, its row number is `seqnum`, and when it was written is `currunix`: the event states each of them, so no column repeats one |
| Schema | complete before a byte is read: `autotype` types the row header's captures from the regex, and each declares its column whether or not a row matches it |
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
    use arrow_array::{Array as _, StringArray, UInt64Array};
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
    // The nineteen event columns, then the body, then the header's captures.
    assert_eq!(text_batch.schema().fields().len(), 22);
    // The record's place in its chain is its row number, under `seqnum`.
    assert_eq!(
        text_batch
            .column(14)
            .as_any()
            .downcast_ref::<UInt64Array>()
            .unwrap()
            .values(),
        &[1, 3],
    );
    // The body is the record past the header the reader took off it.
    assert_eq!(
        text_batch
            .column(19)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "first\n detail A",
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
        assert [row["seqnum"] for row in rows] == [1, 3]
        assert [row["body"] for row in rows] == [
            "first\n detail A",
            "second\n detail B",
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
    assert.deepEqual(textRows.map((row) => row.seqnum), [1n, 3n])
    assert.deepEqual(
      textRows.map((row) => row.body),
      ['first\n detail A', 'second\n detail B'],
    )
    assert.deepEqual(textRows.map((row) => row.id), [7n, 9n])

    fs.rmSync(textRoot, { recursive: true, force: true })
    ```

## Row schema

The source field is complete before any source bytes are read. It opens with the nineteen [event columns](../../graph.md#columns) in `EventColumn::ALL` order - a line is an [event](lines.md#lines) of the graph, and the FIX row parsed out of it contains the same nineteen under the same names and datatypes, so a message's `srcuuids` joins the line's `curruuid` without a mapping - then the line's own columns.

| column | datatype | value |
| --- | --- | --- |
| `currunix` | `datetime64(ns, UTC)` | required; when the line happened: a stated instant, else the row header's `mtime` capture under `parse_mtime`, else the handle's own modification time, else the epoch. This is where a text read states when a record was written, and there is no column beside it |
| `creaunix` | `datetime64(ns, UTC)` | nullable; a `creaunix` capture as an instant, else what a walk folded, else null |
| `execunix` | `datetime64(ns, UTC)` | nullable; an `execunix` capture as the precise execution instant, else null |
| `recdunix` | `datetime64(ns, UTC)` | nullable; a `recdunix` capture as the precise recording instant, else null |
| `refrecdunix` | `datetime64(ns, UTC)` | nullable; a `refrecdunix` capture or event value naming the observation selected as merge reference, else null; a graph merge persists the latest such recording clock here while `recdunix` keeps the earliest observation |
| `exprtime` | `datetime64(ns, UTC)` | nullable; an `exprtime` capture as an instant, else what a walk folded, else null |
| `prevunix` | `datetime64(ns, UTC)` | nullable; a `prevunix` capture, else what a walk stamped, else null |
| `snapunix` | `datetime64(ns, UTC)` | nullable; a `snapunix` capture, else what a grid stamped, else null |
| `curruuid` | `uuid` | required; the line's identity, the UUIDv7 derived from `currunix`, `seqnum`, and `currhashcode`, with `crosshashcode` as its seed; the nil identity where the instant has no UUIDv7 |
| `crossuuid` | `uuid` | required; the UUIDv8 of `crosshashcode`, or `curruuid` where the line names no cross code |
| `crosscode` | `utf8` | nullable; the canonical text of the object the line was read from, else an explicit event value for an unlocated line, else null |
| `currhashcode` | `uint64` | required; the XXH3-64 of what the line states - the row header's captures, its parents, its state, its place and what it follows - and then its body; the capture that dates the line is left out, because `currunix` is coupled with this code rather than fed into it |
| `crosshashcode` | `uint64` | required; the XXH3-64 of `crosscode` - the source URL's text for a located line - zero where none |
| `prevuuid` | `uuid` | nullable; a `prevuuid` capture, else what a walk stamped, else null |
| `seqnum` | `uint64` | nullable; the row number under `start_rownum`, else the zero-based physical index, unless an event value was stated explicitly; null where zero. This is where a text read states the row number, and there is no column beside it |
| `parentuuids` | `list<uuid>` | nullable; what a walk stated, else null |
| `srcuuids` | `list<uuid>` | nullable; what was stated, else null: a line read from a handle has no source |
| `identifiers` | `map<utf8, utf8>` | nullable; every named capture the line matched, under its name, sorted; null where none |
| `state` | `state` | nullable, and never null on a row the reader wrote: a `state` capture, else `00UNKNOWN` |
| `mimetype` | `utf8` | required, and present only with `parse_mimetype` |
| `body` | `utf8` | required; the retained record past its row header, as text, the edges stripped: decoded at the transport under [a declared charset](lines.md#declaring-a-charset), else [where the line is made](lines.md#a-line-is-text). Empty exactly where the header consumed the line, whose captures are then what it states |
| `dropped_byte_size` | `uint64` | nullable; present only with `max_record_byte_size`, and non-null only when bytes were dropped; counts bytes as read, in the units the limit counts |

Every column says what it holds, and the nineteen a line opens with carry the
same spelling the FIX row shows them under, so one fact is named one way
wherever it is read. `required` is a promise the reader keeps in both
directions: a required column is one a line can always state, and a batch read
back into lines is refused where a required cell is null.

Named `rowheader` captures follow these columns and stay nullable in both modes.
The row header is the only thing that lifts a column out of a line. A capture
named for an event fact the line reads from its header - `state`, `prevuuid`,
`creaunix`, `execunix`, `recdunix`, `refrecdunix`, `exprtime`, `prevunix`, or
`snapunix` - feeds that column and appears beside nothing, as an `mtime`
capture dates the line and feeds `currunix`. A capture named for a fact the
line derives is refused: `seqnum` belongs to the row number, `crosscode` to the
object the line came from, and `currunix`, `curruuid`, `crossuuid`,
`currhashcode`, `crosshashcode`, `parentuuids`, `srcuuids`, and `identifiers`
are reserved the same way. The check ignores case.
[`DataType::from_regex`](../../types/text/string.md#regex-captures) types captures constrained to
booleans, signed 64-bit integers, finite floats, ISO dates, times, and
datetimes. `yggdryl::ULBRIDGE_ROWHEADER` is the header a bridge log writes,
its captures named for the [FIX columns they fill](../../fix/arrow.md#a-bridge-log-names-what-it-fills)
when the read goes on into a FIX batch.

### Classifying each record

`currunix` says when the record was written, and has two sources for one column. A row header that declares an `mtime` capture dates each line from the line itself — so a capture spelled that way is read at `datetime64(ns, UTC)` whatever its own syntax suggests, and a reading that names no offset is resolved through `timezone`. A header that declares no such capture, or a line the header did not match, falls back to [`IOBase::mtime`](../../holder/iobase/bytes.md#modification-time) — the handle's own modification time, read once per read and shared by every row. Neither available is the epoch, which is what an unlocated buffer answers. Turning `parse_mtime` off leaves the instant to the handle alone, and frees the name for an ordinary capture with a column of its own.

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
- an unlocated buffer -> `crosscode` is null unless a caller states one, and `currunix` is the epoch: a buffer has no location and records no modification time; `crosshashcode` is zero and `crossuuid` is the line's own identity.
- an event fact nothing states -> the column's null, but for the ones never null: `currunix` the epoch, `curruuid` the nil identity where the instant has no UUIDv7, `crossuuid` the line's own identity, `currhashcode` the code the line's content digests to, `crosshashcode` zero; and `state`, nullable, is written `00UNKNOWN` all the same.
- a row-header capture named `seqnum` or `crosscode`, in any case -> refused when the header is set, because the reader owns those facts through the row number and the object the line came from.
- a `state`, `prevuuid`, or instant capture the line cannot read at the fact's datatype -> a refusal naming the row, the object and the fact, on the batch as on the reading.
- a negative calculated row number -> cannot supply the event's unsigned `seqnum` and therefore refuses the read by name; the infallible event door falls back to the physical index.
- a row header declaring an `mtime` capture with `parse_mtime` off -> an ordinary capture, typed by its own syntax, with a column of its own.
- a row header that matches a whole line -> an empty `body`, and the captures are what the row states; the refusal for a line with no body reads the bytes as cut, before the header comes off them.
- a line written back out -> its `body`, which no longer carries the header the read took off it.
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
