# Plain-text batches

Lines as Arrow batches, and the Rust converters that turn batches back into lines.

## Contract

| Key | Value |
| --- | --- |
| Reads | `read_arrow_reader` yields the [row schema](index.md#row-schema) as batches; `read_arrow_field` answers it before any byte is read |
| Writes | `overwrite_arrow_reader` and `append_arrow_reader` consume the non-null `utf8` `body` column; keyed merge is refused, a line having no row identity |
| Converters | `into_arrow_batch` / `into_arrow_reader` and `from_arrow_batch` / `from_arrow_reader`, Rust only |
| Intake | column names match exactly, then ignoring case, then against the spellings each column is commonly written under |
| Charset | bodies and captures cross in the charset the handle's media type [declares](index.md#declaring-a-charset) |

## Use

The batch surface reads the same records the [scalar surface](scalar.md) does, one batch at a time.

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
| `sourceurl` | `url`, `source`, `uri`, `path`, `file`, `location` |
| `rownum` | `row_number`, `rownumber`, `line_number`, `lineno`, `row` |
| `mtime` | `timestamp`, `time`, `ts`, `written_at`, `event_time` |
| `mimetype` | `bodytype`, `content_type`, `contenttype`, `media_type` |
| `body` | `payload`, `message`, `line`, `text`, `content`, `raw` |
| `dropped_byte_size` | `dropped`, `dropped_bytes`, `truncated_bytes` |
| the sixteen [event columns](../../graph.md#columns) | their own names only, exactly or ignoring case |

A batch carrying the event columns restates them on each line read back - the identity a message named as its source survives the round trip - and a batch without them is read as before, each line deriving its own from its body and instant.

- The column plan is resolved once, at intake: a matched column whose Arrow
  datatype is not the one the options plan for it is refused there, at
  `$.<column>`, before any row. A column the batch does not carry leaves its
  field at the default, because absence is not a failure on the read path.
- `from_arrow_reader` holds one batch and one row cursor and decodes a row only
  when it is pulled. A batch whose schema differs from the reader's declared
  schema, a source error, or any row refusal is answered once and fuses the
  iterator. `from_arrow_batch` reads its one bounded batch into a `Vec`.
- A persisted `rownum` is `start_rownum` plus the line's index; reading
  subtracts the same configured start and refuses a row number before it.
  Without a `rownum` column the index is the row's stream ordinal, continuous
  across batches, so physical gaps the read dropped are not recovered.
- A null cell stays absent. A malformed present value - a `rownum` before the
  start, a `dropped_byte_size` that is not a nonnegative `u64`, a `mimetype`
  that does not parse, a null in the required `body` - is refused rather than
  read as zero or dropped.
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

// The stored rownum is 12, and reading subtracts the start again; the `id`
// capture is int64, so it comes back canonical, still at index 1.
let lines = from_arrow_batch(&batch, &options)?;
assert_eq!(lines[0].index(), 2);
assert_eq!((lines[0].capture(0), lines[0].capture(1)), (None, Some("7")));

// Under a later start the stored row would precede it: refused, located by
// stream ordinal and column.
options.start_rownum = Some(13);
let error = from_arrow_batch(&batch, &options).unwrap_err().to_string();
assert!(error.contains("$[0].rownum"), "{error}");

// No rownum column: indices are stream ordinals across batches, and a batch
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

## Writes

Writes stay physical-line operations, consuming the non-null `utf8` `body`
column - `utf8`, `large_utf8` or `utf8_view`, or any of those behind a
dictionary, which is what Arrow JS infers for a plain record's string and is
unpacked once per batch - one spelling here - and appending
the terminator, both written in the charset the handle's media type
[declares](index.md#declaring-a-charset) and as they are under UTF-8 or US-ASCII. A
batch carrying a `binary` body is refused naming what was
expected: a `binary` column may hold anything, and rendering one would write
bytes no reader of the file could read back as the rows they were.

## Edges

- keyed merge -> unsupported; overwrite and append only, a line having no row identity.
- a `binary` `body` column -> write refused: `expected a utf8 body column, got Binary`.
- `body` holding the terminator -> write refused.
- a dictionary-encoded `utf8` body -> accepted, unpacked once per batch.
- empty, missing, compressed, local, or foreign Arrow-filesystem resource -> the full schema before iteration.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test media text::
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/text
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_text_lines.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/text.test.js
    ```
