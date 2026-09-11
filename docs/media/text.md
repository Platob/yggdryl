# Plain-text records

`TextOptions` reads a handle as physical-line or framed records, writes one line
per row, and converts into the text variant of [`RecordOptions`](options.md).

## Contract

| option | contract |
| --- | --- |
| `rowheader` | byte regex searched once per physical line; in framed mode a match starts a record |
| `framing` | join physical lines into logical records; default `false`, and enabling it requires `rowheader` |
| `leading_fragment` / `leadingFragment` | `keep`, `drop`, or `error` for lines before the first framed header; default `keep` |
| `max_record_byte_size` / `maxRecordByteSize` | retained body byte limit per record, counted in bytes as read; unset is unlimited |
| `lstrip`, `rstrip` | byte regex removed only when its match touches the corresponding physical-line body edge |
| `linesep` | exact terminator; unset accepts LF, CRLF, or CR and writes LF |
| `start_rownum` / `startRownum` | optional signed 64-bit first row number; unset omits the column |
| `parse_mtime` / `parseMtime` | emit `mtime`, filled by the row header's `mtime` capture or by the handle's own modification time; default `true` |
| `parse_mimetype`, `parse_direction` | classify each record and add the column named, off by default |
| `rename_columns` / `renameColumns` | emitted name for a column, keyed by its default name; a key naming no column is refused |
| `lift_names` / `liftNames` | entry paths lifted into columns of their own, each named by its `as` alias where it writes one; unset lifts nothing beyond the row header's captures |
| `autotype` | infer capture datatypes from regex syntax before reading; default `true` |
| `timezone` | zone applied when autotyping offset-free timestamps |

## Use

=== "Rust"

    ```rust
    use arrow_array::{Array as _, Int64Array, StringArray};
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
        assert [row["rownum"] for row in rows] == [1, 3]
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
    assert.deepEqual(textRows.map((row) => row.rownum), [1n, 3n])
    assert.deepEqual(
      textRows.map((row) => row.body),
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
| `body` | `utf8` | required; the retained record as text, [decoded where the line is made](#a-line-is-text) |
| `dropped_byte_size` | `uint64` | nullable; present only with `max_record_byte_size`, and non-null only when bytes were dropped; counts bytes as read |

Named `rowheader` captures follow these columns and stay nullable in both modes,
and the columns `lift_names` [lifts](#lifting-an-entry-into-a-column) follow
the captures.
[`DataType::from_regex`](../types/text.md) types captures constrained to
booleans, signed 64-bit integers, finite floats, ISO dates, times, and
datetimes. `yggdryl::ULBRIDGE_ROWHEADER` is the header a bridge log writes,
its captures named for the [FIX columns they fill](../fix/arrow.md#a-bridge-log-names-what-it-fills)
when the read goes on into a FIX batch.

### Classifying each record

`mtime` says when the record was written, and has two sources for one column. A row header that declares an `mtime` capture dates each line from the line itself, and the capture fills the column rather than appearing beside it — so a capture spelled that way is read at `datetime64(ns, UTC)` whatever its own syntax suggests, and a reading that names no offset is resolved through `timezone`. A header that declares no such capture, or a line the header did not match, falls back to [`IOBase::mtime`](../holder/iobase/bytes.md#modification-time) — the handle's own modification time, read once per read and shared by every row. Neither available is null, which is what an unlocated buffer answers. Turning `parse_mtime` off removes the column, and frees the name for an ordinary capture.

The two classification columns are the [capture readings](../fix/registry.md#classifying-a-captured-line) run over each record's body: what the line is, and which way it moved. They need no dictionary and cost one shallow scan per record, which is why they are opt-in per column — a read that only needs rows should not pay for them.

`parse_direction` also takes the marker off the body, because a verb in front of the payload is transport prose rather than payload. The reader states no message type of its own: a line's type is what its frame says, and reading a frame is the [codec's](../fix/decode.md) work rather than the classifier's.

[FIX decoding](../fix/decode.md) consumes these captured records through a lazy
`FixMessages` iterator. One bulk ULconfig response can yield several flat
messages; each retains the originating capture columns. The text reader itself
still emits one row per framed record.

Measured in [Classifying a capture](../fix/registry.md#classifying-a-capture).

## Lines

`read_text_lines` is the one decode entry point. Every record method routes
through it, so a caller reading lines and a caller reading batches read one
decode rather than two.

A line is a struct, not a map: `index`, `url`, `timestamp`, `bodytype`, `body`,
`direction`, `dropped_byte_size`, `decoded_byte_size`, the row header's
`captures` in the order the expression declares them, and the `entries` it
carries. Each field already holds what its column holds, so building a batch
reads the struct rather than re-deriving a datatype per value.

`TextLine::from_bytes(index, body)` makes one from the bytes the reader cut, and
is where those bytes [become text](#a-line-is-text); `set_body` and
`set_captures` / `with_captures` take bytes the same way, and the Python and
JavaScript constructors take a `str` / `string` body beside bytes. `set_body`,
`with_captures` and `body_bytes` are Rust-only. `body` answers `&str`,
`capture(index)` answers `Option<&str>`, and `body_bytes` answers the body as
the range of its page for a reader that works in offsets.

`timestamp` counts nanoseconds UTC in 128 bits, wider than the column it fills.
A 64-bit nanosecond count runs out in 2262, so a capture reading past that has
somewhere to land; narrowing happens once, where the column is built, and a
count that will not fit is refused by name rather than truncated.

=== "Rust"

    ```rust
    use yggdryl::media::text::{TextOptions, read_text_lines};
    use yggdryl::{FieldPath, holder::Buffer};

    let capture = Buffer::from_bytes(b"8=FIX|55=AAPL\n35=D|55=MSFT\n".to_vec());
    let options = TextOptions::new().try_with_lift_names(["55"])?;

    let symbol = FieldPath::from_str("\"55\"")?;
    let mut read = Vec::new();
    for line in read_text_lines(&capture, &options)? {
        let line = line?;
        read.push((
            line.index(),
            line.get_entry_by_path(&symbol)
                .map(|entry| entry.value().into_owned()),
        ));
    }
    assert_eq!(
        read,
        [(0, Some("AAPL".to_owned())), (1, Some("MSFT".to_owned()))]
    );
    ```

=== "Python"

    ```python
    from yggdryl import TextOptions
    from yggdryl.holder import Buffer

    capture = Buffer.from_bytes(b"8=FIX|55=AAPL\n35=D|55=MSFT\n")
    options = TextOptions()
    options.lift_names = ["55"]

    read = [
        (line.index, line.get_entry_by_path("55").value)
        for line in capture.read_text_lines(options=options)
    ]
    assert read == [(0, "AAPL"), (1, "MSFT")]
    ```

=== "JavaScript"

    ```javascript
    const { IOBase, TextOptions } = require('yggdryl')

    const capture = IOBase.fromBytes(Buffer.from('8=FIX|55=AAPL\n35=D|55=MSFT\n'))
    const options = new TextOptions()
    options.liftNames = ['55']

    const read = []
    for (const line of capture.readTextLines(options)) {
      read.push([Number(line.index), line.getEntryByPath('55').value])
    }
    console.assert(JSON.stringify(read) === JSON.stringify([[0, 'AAPL'], [1, 'MSFT']]))
    ```

### A line is text

A line's body is text, and it is made so where the line is made. The reader
forces UTF-8: a body that is valid UTF-8 - every line of every capture this
crate holds - stays the range of the page it was read into and costs nothing
beyond the validation. A body that is not is decoded once, for that line, into
a page of its own: every valid UTF-8 run is kept as it is, and every byte of
every invalid run is read as the character Windows-1252 gives it, as the WHATWG
encoding standard tables it - `0x80`-`0x9F` as that table's punctuation,
currency and letters, `0xA0`-`0xFF` as `U+00A0`-`U+00FF`, and the five bytes
the classic table leaves undefined (`0x81`, `0x8D`, `0x8F`, `0x90`, `0x9D`) as
the C1 controls of the same number rather than a refusal. A byte the wire held
is a fact, and the reader never writes `U+FFFD` for one, because a replacement
character is the absence of a fact where the line had one. There is no charset
option: Windows-1252 is the one decode of a stray byte that loses nothing, since
it maps every byte to one character.

The decode is per byte rather than per line. A capture is mostly UTF-8 with an
odd Latin-1 byte far more often than it is wholly Windows-1252 - a name a
Windows tool wrote into a log a Linux service otherwise wrote in UTF-8 - and
decoding a valid `é` (`C3 A9`) as `Ã©` because a lone `0xE9` stands elsewhere
on the line would destroy what was right to repair what was wrong. A wholly
Windows-1252 line has no valid multi-byte run to keep and reads byte for byte
either way. The row header's captures take the same decode, because a line
half text would be two readings of one line, so a capture holding an invalid
byte reaches its typed column decoded rather than refused. The header is
still matched against the bytes as read, and a Unicode class matches
characters: `[^\]]+` never matches a byte that is not one, so a header that
must capture such a byte spells the class in bytes, `(?-u:[^\]]+)`.

The decode comes last. Everything the reader does before the line exists - the
row-header match, `lstrip` / `rstrip`, the direction, `max_record_byte_size`,
`dropped_byte_size`, adjacent deduplication - is a fact of the bytes as read,
in bytes as read: the byte limit bounds the wire, not the decode, so a body of N
wire bytes decodes to as many as 3N bytes of text, and `dropped_byte_size`
stays a wire count. A limit that cut a multi-byte character in two leaves its
orphan bytes invalid, and they decode as the Windows-1252 characters they are -
`E2 82` reads `â‚` - which is the honest answer to a reader that asked for N
bytes and got them.

`decoded_byte_size` says how many bytes of the line as read - body and captures
together - were not UTF-8 and were decoded; `0` for a line that was text as
read. It is the one fact the decode keeps, so a reader auditing a capture can
find the lines the reader repaired without decoding them again.

=== "Rust"

    ```rust
    use yggdryl::media::text::{TextBytes, TextLine};

    let line = TextLine::from_bytes(0, TextBytes::from_bytes(b"58=caf\xe9|10=0|")?)?;
    assert_eq!(line.body(), "58=café|10=0|");
    assert_eq!(line.decoded_byte_size(), 1);

    let text = TextLine::from_bytes(1, TextBytes::from_bytes("58=café|10=0|")?)?;
    assert_eq!(text.body(), line.body());
    assert_eq!(text.decoded_byte_size(), 0);
    ```

=== "Python"

    ```python
    from yggdryl import TextLine

    line = TextLine(0, b"58=caf\xe9|10=0|")
    assert line.body == "58=café|10=0|"
    assert line.decoded_byte_size == 1

    text = TextLine(1, "58=café|10=0|")
    assert text.body == line.body
    assert text.decoded_byte_size == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { TextLine } = require('yggdryl')

    const line = new TextLine(0, Buffer.from('58=caf\xe9|10=0|', 'latin1'))
    assert.equal(line.body, '58=café|10=0|')
    assert.equal(line.decodedByteSize, 1)

    const text = new TextLine(1, Buffer.from('58=café|10=0|'))
    assert.equal(text.body, line.body)
    assert.equal(text.decodedByteSize, 0)
    ```

The codec's own byte doors - `parse_fix_line`, `parse_ullink_line`,
`parse_pairs` - take bytes as given and decode nothing, so a caller holding the
wire still reads it as the wire; a message read from a line the text reader made
reads that line's text, and [re-emits](../fix/encode.md) it. A data field whose
stated length reaches no boundary of the decoded line is not honoured, exactly
as any stated length that reaches no boundary is not: the value stays what the
frame cut, and the line's `decoded_byte_size` says the line was decoded.

### Entries and paths

`entries` is the key/value tree the line itself wrote down, keyed and valued by
ranges of the page the line was read into. It is the one thing on the decode
path that allocates, so it is built only when a column reads an entry or a
caller asks: a read whose columns never touch one never pays for it, and the
benchmark reports both.

`TextEntry::key` and `value` answer text, infallibly - `Cow<str>` in Rust,
`str` in Python, a string in JavaScript - borrowed wherever the range is text,
which on a line the reader made is always: the line [was text](#a-line-is-text)
before it was scanned, and the scanner cuts a range at `=`, at a separator, at
whitespace and at the punctuation a transport closed a line with, all of them
ASCII, so no range it cuts ever divides a character. A range a caller built
from bytes of their own that are not text is answered as the lossy decode of
it, owned. `key_bytes` and `value_bytes` answer the ranges themselves, for a
reader that works in offsets - the FIX codec re-slicing a data field to the
length its `Len` field stated, or re-emitting a frame byte for byte.

`TextEntries::from_bytes` is that walk, and it takes a `TextBytes` rather than
a slice because every key and value it answers is a range of the page those
bytes already point into. It is how a reader holding one field's value - a FIX
data field carrying a whole row - reads that value's own pairs through the same
walk the line was read by, rather than writing a second one. `None` where the
bytes state no pair at all, which is the absence `TextLine::entries` carries for
a line nothing asked a tree of. `TextEntries::from_bytes_direct` is the same
walk stopped at one level - every pair the bytes state, none descended into -
for a reader that reads a nested value by rules of its own, as the FIX codec
reads a data field to the length it stated; the tree would be a second reading
of the same bytes, paid on every value holding an `=` and then thrown away.

An entry is addressed by [`FieldPath`](../types/paths.md), the crate's one path
grammar: `.name` for a child, `[0]` and `[-1]` for a position, `['key']` for a
key, and a quoted name for one carrying a dot. `get_entry_by_path` answers
`None` for a miss, because a path naming something a line did not carry is the
ordinary case; `entry_by_path` raises instead, naming the path. `set_entry_by_path`
creates what is not there, and `remove_entry_by_path` takes one away.

Lookup is a linear scan per level. That is correct rather than a compromise: a
line carries tens of pairs, not thousands, an index would cost an allocation per
line to save scanning a handful of entries, and the scan runs over ranges of one
page. Resolve a path once and reuse it; the column plan already does.

### Where a pair ends

A frame decides, and a frame is a run of pairs the line named a separator for -
a `SOH` raw or escaped, or a pipe. Named means used: a byte a line merely holds
inside a value separated nothing, so the candidate that wins is the earliest one
with a field after it, or the one the line closed on as a wire message does.
`MSGTYPE=P Report Ack|SYMBOL=AAPL|` therefore reads on its pipe and keeps
`P Report Ack` whole, while in `8=FIX.4.4 35=D 58=a|b 10=0` the pipe stands
inside a value and separates nothing, so nothing was named and the loose rule
below reads the line.

Inside a frame the pairs are that frame's segments cut at their first `=`, and a
value ends only at that separator, so `8=FIX.4.4|18=G L|48=ABBN SW|10=0|`
carries `G L` and `ABBN SW` whole, and `58=` is a pair carrying nothing rather
than no pair at all. A key inside a frame is a key however the writer spelled
it - `NoAllocs[0].79`, `#INSTRUMENT[DESCRIPTION]`, `Msg Type` and a second `#`
on a key already marked are all names, the space among them because the FIX name
fold ignores it exactly as it ignores `_`. It is still a run of name bytes and
stops at the first byte no name holds, so a log's remark reading `sent >> seq=7`
after the frame states only `seq`. The space costs one thing and the cost is
stated rather than dodged: a remark spelled in nothing but words is such a run,
so `trailing note=x` after a frame is a field keyed `trailing note`. No rule
available to a scanner separates it from `Msg Type` - the scanner holds no
dictionary - and a reader that holds one answers nothing for it.

Everywhere else - a sentence, a transport's prefix in front of a frame, a bare
run of attributes, a line that ran its fields together with spaces - a value
ends at the first byte that could end a field, because nothing said which byte
separates two of them. `host=srv1, port=8080` is two pairs and not one; the
same `58=quoting #A=1 and #B=2` that is one Text field inside a frame is three
pairs where a transport wrote it as prose; and `Symbol[0]=AAPL` standing on its
own states no pair at all, because what a frame widens is the key inside it and
not the walk that finds a frame. A value that states pairs of its own is read
as a tree under its entry either way, which is what `entries` has always meant:
the frame says where the Text field ends, and the field's own text says what
hangs beneath it.

`marked` says the line wrote a `#` in front of the key. The key itself is
stripped of it, so a path lifts the name the writer gave the field, and the mark
rides beside the pair instead - two entries differing only in it are two values,
and a bridge restating `#ORDERID=123` under an `ORDERID=123` it already sent is
telling the reader something. What that means is a dialect's reading of the
mark, not the text reader's. An entry a caller created, or one rebuilt from a
lifted column, is unmarked.

!!! note "Rust-only"
    `marked` has no Python or Node getter yet. Both bindings already show the
    mark - an entry renders as the line wrote it, `#` and all, and two entries
    differing only in the mark compare unequal - so read it there off the
    rendered pair until the getter lands with the rest of the FIX work.

### Lifting an entry into a column

`lift_names` names the entry paths that become columns of their own. The column
exists in the schema whether or not any row carries that entry — a row without
it is null — so the schema is still complete before a byte is read, exactly as
`autotype` already guarantees for captures.

A lifted column takes the path's [alias](../types/paths.md#aliases) where it
writes one, and the last segment's own name otherwise. `"55" as symbol` selects
and names in one breath, which is also how two paths ending in the same segment
are told apart. `rename_columns` still renames it like any other column.

A lifted column is `utf8`, nullable: it holds the entry's value as the line's
text holds it, and a row not carrying the entry is null.

The two options have one job each and meet only at the compiled column plan:
renaming decides what a column is called and never whether one exists, lifting
decides which entry paths become columns and never what they are called.

### What is copied

A body that was UTF-8 as read is a range of the page the line was read into, so
a decoded line copies no byte of it, and the Arrow value it lands in is built
from that same page. A record joining several physical lines is assembled into
a page of its own and copies once, and so is a line that [was not
UTF-8](#a-line-is-text): decoded into a page of its own, once, for that line.
Every value crossing into Python or JavaScript is copied by contract: `str` and
strings own their text, and the `bytes` and `Buffer` the `_bytes` accessors
answer own their bytes.

## Reading Arrow back into lines

`from_arrow_batch` and `from_arrow_reader` are the reverse direction, so a text
read can round-trip through Arrow and come back as lines.

Column names are matched exactly first, then ignoring case, then against the
spellings each column is commonly written under — `payload`, `message`, `line`,
`text`, `content` and `raw` all reach `body`; `source`, `uri`, `path`, `file`
and `location` all reach `url`; `timestamp`, `time`, `ts`, `written_at` and
`event_time` all reach `mtime`. Intake is where flexibility belongs: a batch
another producer wrote names its columns the way that producer named them.
Meaning stays exact — a matched column is read at the datatype its own column
declares, and nothing guesses what a value means, only what a column is called.

A column the batch does not carry leaves that field at its default rather than
failing, because absence is not a failure on the read path anywhere else here.

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
| unbounded `body` | the exact source bytes after first-line header removal and normalization, [as text](#a-line-is-text) |
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
| what the bound counts | the body in bytes as read, including normalized separators - the wire, never the [decoded text](#a-line-is-text) |
| omitted bytes | reported in `dropped_byte_size`, in bytes as read |
| `max_row_size`, `max_byte_size` | total result rows and total Arrow result memory; independent and unchanged |

## Writes

Writes stay physical-line operations, consuming the non-null `utf8` `body`
column - `utf8`, `large_utf8` or `utf8_view`, or any of those behind a
dictionary, which is what Arrow JS infers for a plain record's string and is
unpacked once per batch - one spelling here - and appending
the terminator. A batch carrying a `binary` body is refused naming what was
expected: a `binary` column may hold anything, and rendering one would write
bytes no reader of the file could read back as the rows they were.

## Edges

- `framing` without `rowheader` -> refused.
- physical-line mode, no match -> body kept, captures null.
- `leading_fragment = error` -> the first physical line before any header fails the read.
- `max_record_byte_size = 0` -> valid; empty prefix, whole body counted as dropped.
- `max_record_byte_size` unset -> no `dropped_byte_size` column; set but never exceeded -> null.
- strip match off the physical-line body edge -> nothing removed.
- `autotype = false` or a broad capture (`\S+`) -> `utf8`.
- classification columns ahead of the captures -> the captures keep the types their patterns gave them; a `thread` capture is `utf8` whatever the classification read before it.
- an unlocated buffer -> `url` and `mtime` are both null: a buffer has no location and records no modification time, and neither the empty string nor a clock reading is one.
- Python `read_records()` over a file whose modification time is finer than a microsecond -> the `datetime` a record hands back is floored to the microsecond it can hold, never refused. The batch path carries the full nanosecond reading.
- a row header declaring an `mtime` capture with `parse_mtime` off -> an ordinary capture, typed by its own syntax.
- empty, missing, compressed, local, or foreign Arrow-filesystem resource -> the full schema before iteration.
- a rename onto a name another column already emits -> refused when the option is set, naming both.
- a lifted path no line carries -> that column is null in every row; the column still exists in the schema.
- a lifted path whose last segment names nothing -> refused when the option is set.
- `body` holding the terminator -> write refused.
- a `binary` `body` column -> write refused: `expected a utf8 body column, got Binary`.
- a body or capture that is not UTF-8 -> [decoded](#a-line-is-text), never refused and never `U+FFFD`; `decoded_byte_size` counts the bytes it took.
- `max_record_byte_size` cutting inside a multi-byte character -> the orphan bytes decode as the Windows-1252 characters they are; the count dropped is still the wire count.
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
