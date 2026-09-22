# Reading plain-text records

Lines out of a handle - as native scalars, as Arrow batches - and the Rust converters that read a batch back into lines.

## Contract

| Key | Value |
| --- | --- |
| Owns | `read_records` / `readRecords`, `read_arrow_reader`, `read_arrow_field`, `read_text_lines`, and the Rust `from_arrow_batch` / `from_arrow_reader` |
| Native rows | Python `read_records`, JavaScript `readRecords` - one row per physical line, or per framed record; Rust reads Arrow and crosses with `ArrowScalar::into_scalar` |
| Row shape | a mapping per row in the bindings; an ordered `Scalar::Sequence` under the root in the [value model](../../types/scalar.md) |
| Batches | an [`arrow::BatchReader`](../../arrow/readers.md) over the [row schema](index.md#row-schema); `read_arrow_field` answers it before any byte is read |
| Lazy | one decode, `read_text_lines`, behind both surfaces: a batch is built as the reader is stepped and only the current one is alive |
| Clauses | `read_records` and the Arrow reads answer the [`select` and `where` sections](options.md); `read_text_lines` answers every line, because a `where` may name a column the `select` builds and no line states one |
| Intake | column names match exactly, then ignoring case, then against the spellings each column is commonly written under |
| Charset | bodies and captures cross in the charset the handle's media type [declares](lines.md#declaring-a-charset) |
| Absence | a location that holds nothing yields no rows, and the full schema still comes before iteration |

## Use

Native rows first: Python and JavaScript hand back one mapping per row, and Rust reads Arrow and crosses into the value model in the same call. Nothing in the call names an encoding - the name already did.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::{IOBase as _, IOMedia as _};
    use yggdryl::Url;

    let source = Buffer::from_bytes(b"first\nsecond\nthird\n".to_vec())
        .with_media_type(Url::from_str("file:///app.log")?.media_type());
    let options = source.record_options()?;

    // Rust reads Arrow, then crosses into the value model: one ordered row
    // sequence per line, under the full row schema.
    let rows = source.read_arrow(Some(&options))?.into_scalar()?;
    assert_eq!(rows.len(), 3);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, TextOptions

    source = pathlib.Path(tempfile.mkdtemp()) / "app.log"
    source.write_bytes(b"first\nsecond\nthird\n")

    rows = list(IOBase(source).read_records(options=TextOptions()))
    assert [row["body"] for row in rows] == ["first", "second", "third"]
    # `start_rownum` is unset, so the first line's place is zero, which a
    # `seqnum` states as nothing.
    assert rows[0]["seqnum"] is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, TextOptions } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-'))
    const source = path.join(root, 'app.log')
    fs.writeFileSync(source, 'first\nsecond\nthird\n')

    const rows = [...new IOBase(source).readRecords(new TextOptions())]
    assert.deepEqual(rows.map((row) => row.body), ['first', 'second', 'third'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Rows as Arrow batches

The same records without the crossing: a read returns a reader, and stepping it builds one batch at a time.

=== "Rust"

    ```rust
    use arrow_array::{Array as _, StringArray};
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::{IOBase as _, IOMedia as _};
    use yggdryl::holder::Buffer;
    use yggdryl::Url;

    let source = Buffer::from_bytes(b"first\nsecond\n".to_vec())
        .with_media_type(Url::from_str("file:///app.log")?.media_type());
    let options = source.record_options()?;

    let batch = source.read_arrow_reader(&options)?.next().unwrap()?;
    let bodies = batch
        .column_by_name("body")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();

    assert_eq!(batch.num_rows(), 2);
    assert_eq!((bodies.value(0), bodies.value(1)), ("first", "second"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    source = pathlib.Path(tempfile.mkdtemp()) / "app.log"
    source.write_bytes(b"first\nsecond\n")

    table = IOBase(source).read_arrow_reader().read_all()

    assert table.num_rows == 2
    assert table.column("body").to_pylist() == ["first", "second"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-'))
    const source = path.join(root, 'app.log')
    fs.writeFileSync(source, 'first\nsecond\n')

    const table = new IOBase(source).readArrowReader().intoTable()

    assert.equal(table.numRows, 2)
    assert.deepEqual([...table.getChild('body')], ['first', 'second'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Row numbers and captures

`start_rownum` numbers the first physical line of each record, and `seqnum` is
where the row states it: the same row number, or the zero-based physical index
when `start_rownum` is unset, preserving the gaps blank lines left. There is no
second column beside it, and a count cannot hold a negative number, so a
negative `start_rownum` is refused by name rather than counted down through
zero. `crosscode` is the identifier the read was addressed by, as its canonical
text - the URL where that identifier is a location, which is the common case,
and the name itself where it is a name, never the place a name resolves to -
and its `crosshashcode` seeds the line's current identity. So a line's chain is
what it was read under, moving it refreshes both UUIDs, and a read through a
name is crossed the same wherever the process happened to be running. A row
header therefore cannot declare a `seqnum` or `crosscode` capture, in any case.

A [row header](lines.md#lines) adds one column per other capture, typed from
the regex when `autotype` is on. It is the only thing that adds one: every
added column reaches a record row under the name it is emitted as.

## Reading Arrow back into lines

`into_arrow_batch` / `into_arrow_reader` build batches from lines and
`from_arrow_batch` / `from_arrow_reader` read them back, so a text read
round-trips through Arrow as lines. All four are Rust only: Python and
JavaScript bind none of them.

Column names are matched exactly first, then ignoring case, then against the
spellings each column is commonly written under, keyed by its default name so a
renamed column still finds them. Intake is where flexibility belongs: a batch
another producer wrote names its columns the way that producer named them.
Meaning stays exact: nothing guesses what a value means, only what a column is
called.

| column | also found as |
| --- | --- |
| `crosscode` | `sourceurl`, `url`, `source`, `uri`, `path`, `file`, `location` |
| `seqnum` | `rownum`, `row_number`, `rownumber`, `line_number`, `lineno`, `row` |
| `mimetype` | `bodytype`, `content_type`, `contenttype`, `media_type` |
| `body` | `payload`, `message`, `line`, `text`, `content`, `raw` |
| `dropped_byte_size` | `dropped`, `dropped_bytes`, `truncated_bytes` |
| the other seventeen [event columns](../../graph.md#columns) | their own names only, exactly or ignoring case |

The two spellings a producer wrote the object and the row number under resolve
onto the columns that state them, so a batch written before they were event
facts reads back into the same lines.

A batch carrying the event columns restates their non-null values on each line
read back - the identity a message named as its source survives the round trip -
and a batch without them leaves each line to derive its own facts.

- The column plan is resolved once, at intake: a matched column whose Arrow
  datatype is not the one the options plan for it is refused there, at
  `$.<column>`, before any row. A column the batch does not carry leaves its
  field at the default, because absence is not a failure on the read path.
- `from_arrow_reader` holds one batch and one row cursor and decodes a row only
  when it is pulled. A batch whose schema differs from the reader's declared
  schema, a source error, or any row refusal is answered once and fuses the
  iterator. `from_arrow_batch` reads its one bounded batch into a `Vec`.
- A persisted `seqnum` is `start_rownum` plus the line's index; reading
  subtracts the same configured start and refuses a sequence number before it.
  Without a `seqnum` column - which is what a first line numbered zero states,
  since a place of zero is null - the index is the row's stream ordinal,
  continuous across batches, so physical gaps the read dropped are not
  recovered.
- A persisted `crosscode` that reads as an identifier restores what the line
  was addressed by, so the source survives the round trip in the column that
  names its chain: a URL restores as itself, and a `urn:` or `arn:` restores as
  the name it was, locating itself again wherever it resolves to now. A code
  that is no identifier is an ordinary code and addresses nothing - text
  carrying no scheme is not read as a relative path here, or every stated code
  would come back naming a file. Its `crosshashcode` seed participates in the
  derived `curruuid`, so it is applied before an unstated identity is resolved.
- A null cell stays absent. A malformed present value - a `seqnum` before the
  start, a `dropped_byte_size` that is not a nonnegative `u64`, a `mimetype`
  that does not parse, a null in the required `body` - is refused rather than
  read as zero or dropped. A null event `seqnum` or `crosscode` therefore does
  not mask the value the base fact derives.
- Every row refusal is located by the stream ordinal and the column,
  `$[3].mimetype`, never by a row number an earlier column of the same row
  restored.
- A capture keeps its declared index even where the batch carries no column
  for an earlier one. A typed capture renders through the canonical scalar
  text, so its original spelling is not kept: `0007` read as `int64` comes
  back `7`.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use yggdryl::arrow::batch_reader;
use yggdryl::text::{
    TextBytes, TextLine, TextOptions, from_arrow_batch, from_arrow_reader, into_arrow_batch,
};

let mut options = TextOptions::new().try_with_rowheader(r"^(?<level>[A-Z]+) (?<id>\d+) ")?;
options.start_rownum = Some(10);
// The line reads itself by the options it is built under; stated captures are
// its word over the header it would otherwise match.
let line = TextLine::from_bytes(2, TextBytes::from_bytes("first")?, Arc::new(options.clone()))?
    .with_captures(vec![None, Some(TextBytes::from_bytes("0007")?)])?;
let batch = into_arrow_batch([line], &options)?;

// The stored seqnum is 12, and reading subtracts the start again; the `id`
// capture is int64, so it comes back canonical, still at index 1.
let lines = from_arrow_batch(&batch, &options)?;
assert_eq!(lines[0].index(), 2);
assert_eq!((lines[0].capture(0), lines[0].capture(1)), (None, Some("7")));

// Under a later start the stored row would precede it: refused, located by
// stream ordinal and column.
options.start_rownum = Some(13);
let error = from_arrow_batch(&batch, &options).unwrap_err().to_string();
assert!(error.contains("$[0].seqnum"), "{error}");

// No seqnum column: indices are stream ordinals across batches, and a batch
// with another schema refuses once and fuses the stream.
let bodies = |name: &str, values: Vec<&'static str>| {
    RecordBatch::try_from_iter([(name, Arc::new(StringArray::from(values)) as ArrayRef)])
};
let first = bodies("body", vec!["one", "two"])?;
let batches = batch_reader(
    first.schema(),
    [first, bodies("body", vec!["three"])?, bodies("payload", vec!["four"])?],
);
let mut read = from_arrow_reader(batches, &TextOptions::new())?;
let indices = read
    .by_ref()
    .take(3)
    .map(|line| line.map(|line| line.index()))
    .collect::<Result<Vec<_>, _>>()?;
assert_eq!(indices, [0, 1, 2]);
let error = read.next().unwrap().unwrap_err().to_string();
assert!(error.contains("$[3]"), "{error}");
assert!(read.next().is_none());
```

## Edges

- `read_records` -> Python and JavaScript only; the Rust primitive read surface stays Arrow-native and crosses with `read_arrow(...).into_scalar()`.
- absent resource -> no rows, not an error.
- empty, missing, compressed, local, or foreign Arrow-filesystem resource -> the full schema before iteration.
- `app.log.gz` or a folder mixing plain, gzip, and zstd leaves -> same options, one stream, no reopened handle and no retained prior page. The transport is read one [fetch window](../../holder/iobase/bytes.md#fetch-window) at a time, whatever the decoder pulls.
- Python `read_records()` over a file whose modification time is finer than a microsecond -> the `datetime` a record hands back is floored to the microsecond it can hold, never refused. The batch path carries the full nanosecond reading.
- a shaped read -> its own narrower schema, on [Options](options.md#use).
- a record that states no byte of its own -> no row at all; [a line with no body is no line](options.md#a-line-with-no-body-is-no-line).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test text -- arrow::text batch::text bytes::text::values entry::text::values handle::text leading::text limits::text line::text options::text plan::columns reader::text sep::text
    cargo bench -p yggdryl --bench text -- text_records
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/text/test_init.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/records.test.js
    ```
