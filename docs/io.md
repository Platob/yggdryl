# Byte I/O

`yggdryl::io` is the crate's one storage abstraction: positional reads and writes over anything that holds bytes.

Python and JavaScript expose one handle class, `IOBase`, under the names each language already uses
for a path. The byte contract and the record methods both cross into the bindings; the role traits,
the wrappers, and the streaming adapters stay in Rust, and every section below says which of the
three languages reach it.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};

    let mut handle = Buffer::new();
    handle.pwrite(0, b"symbol,price\n")?;
    handle.pwrite(13, b"AAPL,1\n")?;
    assert_eq!(handle.size(), 20);

    // Two reads at different offsets, in any order: there is no shared cursor.
    let mut tail = [0_u8; 4];
    handle.pread(13, &mut tail)?;
    let mut head = [0_u8; 6];
    handle.pread(0, &mut head)?;
    assert_eq!(&head, b"symbol");
    assert_eq!(&tail, b"AAPL");
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes()
    handle.pwrite(0, b"symbol,price\n")
    handle.pwrite(13, b"AAPL,1\n")
    assert handle.size == 20

    # Two reads at different offsets, in any order: there is no shared cursor.
    assert handle.pread(13, 4) == b"AAPL"
    assert handle.pread(0, 6) == b"symbol"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.pwrite(0, Buffer.from('symbol,price\n'))
    handle.pwrite(13, Buffer.from('AAPL,1\n'))
    assert.equal(handle.size, 20)

    // Two reads at different offsets, in any order: there is no shared cursor.
    assert.equal(handle.pread(13, 4).toString(), 'AAPL')
    assert.equal(handle.pread(0, 6).toString(), 'symbol')
    ```

`IOBase::pread` and `IOBase::pwrite` are the only two methods an implementation must supply for bytes;
everything else on the trait is derived from them. They take an explicit offset rather than sharing a
cursor, so a footer-first container such as Parquet reads its index without seeking, and two readers of
one handle never interfere.

Three invariants hold for every implementation:

- `pread` returns a short count only at the end of the value; a read entirely past `size` returns `0`.
- `pwrite` grows the value when the write extends past the end, and zero-fills the gap an offset beyond
  the current size creates.
- `size` never exceeds `capacity`, and `reserve` changes only `capacity`.

## Built from what you already hold

In Python, `IOBase(...)` accepts more than a path: callers hold open files and streams more often
than the strings that named them, so the constructor takes those directly. A file-like object with a
real filesystem name captures the *location* - nothing is read, per the laziness contract - while a
nameless stream such as `io.BytesIO` captures its *content* into an in-memory handle. Passing
another handle rebuilds it, and passing an in-memory handle captures its content and media type.

```python
import io
import pathlib
import tempfile

from yggdryl import IOBase

# An open file names its own location, so the handle addresses the path.
target = pathlib.Path(tempfile.mkdtemp()) / "quotes.json"
target.write_bytes(b"{}")
with open(target, "rb") as stream:
    handle = IOBase(stream)
assert handle.name == "quotes.json"

# A nameless stream holds only content, so the content is what is taken.
buffered = IOBase(io.BytesIO(b'{"symbol": "AAPL"}'))
buffered.media_type = "application/json"
assert buffered.read_text() == '{"symbol": "AAPL"}'
```

## Laziness

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::{IOKind, local};

    let path = std::env::temp_dir().join("yggdryl-docs-io-lazy.csv");
    let _ = std::fs::remove_file(&path);

    // Constructing touches nothing: no file is created, opened, or mapped.
    let mut handle = local::File::new(&path)?;
    assert!(!handle.exists());

    // Reading something absent yields nothing rather than failing.
    assert_eq!(handle.size(), 0);
    let mut probe = [0_u8; 8];
    assert_eq!(handle.pread(0, &mut probe)?, 0);
    assert_eq!(handle.kind(), IOKind::Unknown);

    // Writing creates the resource, and any parent it needs.
    handle.write_all_bytes(b"symbol,price\n")?;
    assert_eq!(handle.kind(), IOKind::File);
    assert_eq!(handle.read_all_bytes()?, b"symbol,price\n");

    handle.close()?;
    // Teardown through the abstraction: absence is a no-op success.
    handle.remove(false)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())

    # Constructing touches nothing: no file is created, opened, or mapped.
    handle = IOBase(root / "nested" / "lazy.csv")
    assert not handle.exists()

    # Reading something absent yields nothing rather than raising.
    assert handle.size == 0
    assert handle.read_bytes() == b""

    # Writing creates the resource, and any parent it needs.
    handle.write_text("symbol,price\n")
    assert handle.is_file()
    assert handle.read_text() == "symbol,price\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))

    // Constructing touches nothing: no file is created, opened, or mapped.
    const handle = new IOBase(path.join(root, 'nested', 'lazy.csv'))
    assert.ok(!handle.exists())

    // Reading something absent yields nothing rather than throwing.
    assert.equal(handle.size, 0)
    assert.equal(handle.readBytes().length, 0)

    // Writing creates the resource, and any parent it needs.
    handle.writeText('symbol,price\n')
    assert.ok(handle.isFile())
    assert.equal(handle.readText(), 'symbol,price\n')

    fs.rmSync(root, { recursive: true, force: true })
    ```

A handle is a description of where bytes would live, not proof that they do. Constructing one never
fails for a resource that does not exist yet and never pays for one that is never used; non-existence
is resolved at the operation instead. Reads skip, writes create, and `truncate`/`reserve` create too.
That is why a caller can probe a location without a separate existence check, and why the same code
works whether the target is there or not.

Metadata follows the rule: `media_type` is computed when it is asked for, and re-derived after the
bytes change.

## Kinds

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::{IOKind, local};

    assert_eq!(Buffer::new().kind(), IOKind::Memory);
    assert!(IOKind::Memory.is_leaf());

    let folder = local::Folder::new(std::env::temp_dir())?;
    assert_eq!(folder.kind(), IOKind::Directory);
    assert!(folder.is_container());

    // Nothing is there, so nothing has decided; a write settles it.
    let absent = local::File::new(std::env::temp_dir().join("yggdryl-docs-io-absent.bin"))?;
    assert_eq!(absent.kind(), IOKind::Unknown);
    assert!(!absent.kind().is_known());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    folder = IOBase(pathlib.Path(tempfile.mkdtemp()))
    assert folder.is_dir()
    assert not folder.is_file()

    # Nothing is there, so nothing has decided; a write settles it.
    leaf = folder / "ticks.csv"
    assert not leaf.exists()
    leaf.write_text("symbol\n")
    assert leaf.is_file()
    assert not leaf.is_dir()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const folder = new IOBase(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')))
    assert.ok(folder.isDir())
    assert.ok(!folder.isFile())

    // Nothing is there, so nothing has decided; a write settles it.
    const leaf = folder.joinpath('ticks.csv')
    assert.ok(!leaf.exists())
    leaf.writeText('symbol\n')
    assert.ok(leaf.isFile())
    assert.ok(!leaf.isDir())

    fs.rmSync(folder.toPath(), { recursive: true, force: true })
    ```

`IOKind` is the vocabulary every backend answers in: `Memory` for bytes with no location, `File` for a
leaf that holds bytes, `Directory` for a container that holds other resources, and `Unknown` for a
location that does not exist yet. A table format adds `Table`, `Namespace`, and `Catalog` - all of
them containers, all of them answered by the value that adds the framing rather than by storage,
which sees three indistinguishable folders. `is_container`, `is_leaf`, and `is_known` are the
questions callers actually ask; the enum is documented with the rest of the shared enums in
[enums.md](enums.md). The bindings expose the questions rather than the enum, as `exists`, `is_dir`,
and `is_file`.

## Bytes or rows

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::{MimeType, local};

    // A leaf answers from its representation, and the two are complements.
    let mut notes = Buffer::new();
    notes.set_media_type(MimeType::PLAIN_TEXT.into());
    assert!(notes.is_atomic());
    assert!(!notes.is_tabular());

    // The name is enough: nothing has been written to this location yet.
    let trades = local::File::new(std::env::temp_dir().join("yggdryl-docs-shape.parquet"))?;
    assert!(trades.is_tabular());
    assert!(!trades.is_atomic());

    // A container is neither one whole byte value nor - with nothing under
    // it - a table.
    let folder = local::Folder::new(std::env::temp_dir())?;
    assert!(!folder.is_atomic());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = IOBase(pathlib.Path(tempfile.mkdtemp()))

    notes = root / "notes.txt"
    assert notes.is_atomic()
    assert not notes.is_tabular()

    # The name is enough: nothing has been written to this location yet.
    trades = root / "trades.parquet"
    assert trades.is_tabular()
    assert not trades.is_atomic()

    assert not root.is_atomic()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = new IOBase(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')))

    const notes = root.joinpath('notes.txt')
    assert.ok(notes.isAtomic())
    assert.ok(!notes.isTabular())

    // The name is enough: nothing has been written to this location yet.
    const trades = root.joinpath('trades.parquet')
    assert.ok(trades.isTabular())
    assert.ok(!trades.isAtomic())

    assert.ok(!root.isAtomic())

    fs.rmSync(root.toPath(), { recursive: true, force: true })
    ```

A handle presents its value one of two ways, and these are how a caller asks which. *Atomic* is the
byte surface - `read_all_bytes` reads the value whole and `write_all_bytes` replaces it whole -
while *tabular* is the record surface, [`read_arrow_batch_reader`](#arrow-batches) and its two
writing siblings. Wherever bytes are held the two are complements; a container that holds neither
rows nor one byte value - a plain folder of logs, a namespace, a catalog - answers `false` to both.

The answers cost as little as they can. The media type settles every leaf and every location nothing
has decided yet, without a call into the backing store, so a name that already spells a record
encoding is answered from the name. `IOKind::Table` settles a table format's folder outright, and
`Namespace` and `Catalog` settle the containers that hold only containers. Only a plain `Directory`
is probed, and the probe stops at the first leaf that settles the question rather than listing the
tree - a folder reads as the table beneath it, and a partitioned tree is one table in one encoding.

`is_tabular` is about the representation, not about this build: a `.parquet` leaf is tabular whether
or not the `parquet` feature is compiled in. [`record_options`](#arrow-batches) is the call that
reports an encoding this build cannot decode, and it names it.

## Whole values

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};

    let mut handle = Buffer::new();
    handle.write_all_bytes(b"symbol,price\n")?;

    // `append` reports the offset the bytes landed at.
    assert_eq!(handle.append(b"AAPL,1\n")?, 13);
    assert_eq!(handle.read_range(0, 6)?, b"symbol");
    // A range past the end yields what exists rather than failing.
    assert!(handle.read_range(100, 4)?.is_empty());
    assert_eq!(handle.read_all_bytes()?.len(), 20);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes()
    handle.write_bytes(b"symbol,price\n")

    # `append` reports the offset the bytes landed at.
    assert handle.append(b"AAPL,1\n") == 13
    assert handle.pread(0, 6) == b"symbol"
    # A range past the end yields what exists rather than raising.
    assert handle.pread(100, 4) == b""
    assert len(handle.read_bytes()) == 20
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.writeBytes(Buffer.from('symbol,price\n'))

    // `append` reports the offset the bytes landed at.
    assert.equal(handle.append(Buffer.from('AAPL,1\n')), 13)
    assert.equal(handle.pread(0, 6).toString(), 'symbol')
    // A range past the end yields what exists rather than throwing.
    assert.equal(handle.pread(100, 4).length, 0)
    assert.equal(handle.readBytes().length, 20)
    ```

`read_all_bytes`, `read_range`, `pwrite_all`, `append`, `write_all_bytes`, and `clear` are the
whole-value conveniences; the bindings spell the first two `read_bytes`/`read_text` and `pread`.
`pread_exact` is the strict form of `pread`: it fails, naming the shortfall, when the value ends
before the buffer is full. The pair that reads and replaces the whole value both say `bytes`
because both are about the bytes rather than the rows; [`is_atomic`](#bytes-or-rows) is how a
caller asks which surface a handle is for.

`copy_into` moves bytes between two handles in chunks, so neither side is buffered whole, and it
carries the media type across. It is `copy_into` in Python and `copyInto` in JavaScript.

## Streaming adapters

!!! note "Rust only"
    The Python and JavaScript packages expose positional reads and writes, not
    the `std::io` adapters over them.

```rust
use std::io::{Read, Write};

use yggdryl::io::{Buffer, IOBase};

let mut handle = Buffer::new();
handle.writer_at(0).write_all(b"symbol,price\n")?;
handle.append(b"AAPL,1\n")?;

let mut text = String::new();
handle.reader_at(13).read_to_string(&mut text)?;
assert_eq!(text, "AAPL,1\n");
```

`reader_at` and `writer_at` borrow the handle as a `Reader`/`Writer` implementing `std::io::Read` and
`std::io::Write`. Each adapter advances its own offset, so a second reader started elsewhere is
unaffected. [Adding and removing a coding](#adding-and-removing-a-coding) moves bytes between two
handles through a coding without going through these adapters, and is in all three languages.

## Cursors

A handle is positional - `pread`/`pwrite` take an offset - so a *position* is
state a caller opts into, not something two readers fight over. A cursor is
that position made explicit: `tell` and `seek` move it, reads and writes
advance it, and two cursors over one resource advance independently.

=== "Rust"

    ```rust
    use std::io::Read;

    use yggdryl::io::{Buffer, IOBase, IOCursor};

    let mut cursor = Buffer::new().cursor();
    cursor.write_next(b"symbol,price\n")?;
    assert_eq!(cursor.tell(), 13);

    cursor.seek_to(7);
    let mut word = [0_u8; 5];
    cursor.read_exact(&mut word)?; // std::io::Read rides the same position
    assert_eq!(&word, b"price");
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes()
    cursor = handle.cursor()
    cursor.write(b"symbol,price\n")

    # The write landed on the handle itself; the position is the cursor's.
    assert handle.read_bytes() == b"symbol,price\n"
    assert cursor.seek(-6, 2) == 7
    assert cursor.read(5) == b"price"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes()
    const cursor = handle.cursor()
    cursor.write(Buffer.from('symbol,price\n'))

    assert.equal(handle.readBytes().toString(), 'symbol,price\n')
    cursor.seek(7)
    assert.equal(cursor.read(5).toString(), 'price')
    ```

In Rust, `IOCursor` is the trait - `tell`, `seek_to`, `seek`, `read_next`,
`write_next` - and `Cursor<H>` is the one wrapper every implementation shares:
built by `IOBase::cursor`/`cursor_at`, it stays a full handle over the same
bytes and implements `std::io::Read`, `Write`, and `Seek` over its own
position, so it goes wherever standard readers go; the owned line iterator is
built on exactly it. In Python and JavaScript the cursor *shares the handle*
- a write through it is a write there - and follows each language's file
conventions: `seek(offset, whence)` and `read(size=-1)` in Python, `seek`,
`tell`, and a `position` property in JavaScript.

## Text records

`into_text` wraps any handle in `Text<H>`, the text-record surface: a wrapping handle in the same
shape as [`Coded`](#coded), [`Gzip`](gzip.md), [`Ipc`](ipc.md), and [`Parquet`](parquet.md). It owns
its inner handle, mirrors every byte method through it, and exposes the raw encoded bytes
unchanged - so a `.log.gz` behind a `Text` can still be copied or uploaded verbatim. It is
idempotent: `handle.into_text().into_text()` is the same handle, not a handle wrapped twice.

A record is a *span of the decoded byte window*, not a `String`. Reading yields `TextLine<'_>`
views that borrow the window and cache what they are asked for - the UTF-8 check, the stable hash,
the regex captures - each computed at most once and never on a record nobody inspects. Bytes stream
through that window, and any content codings the resource's name declares are peeled as *streaming*
decoders: a `trades.jsonl.gz` reads record by record without ever holding the decompressed value,
which is what makes a scan over a compressed log cost a window instead of the log.

By default every line is a record. `linesep` unset is *flexible*: `\n`, `\r\n`, and a lone `\r` all
end a record, mixed in one resource, and the last record needs no terminator. Pinning it - to
`\r\n`, to `\0`, to `\x1e` - makes that terminator exact, so anything else is content. A resource
that does not exist yields no records, exactly as it reads zero bytes.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::text::LineSep;
    use yggdryl::Url;

    let mut handle = Buffer::new()
        .with_media_type(Url::from_str("file:///trades.jsonl.gz")?.media_type())
        .into_text();
    handle.write_all_bytes(&yggdryl::gzip::dump(b"{\"id\":1}\r\n{\"id\":2}\n")?)?;

    // A record borrows the window; `text()` is the checked view of its bytes.
    let mut records = handle.read_lines()?;
    let mut seen: Vec<String> = Vec::new();
    while let Some(record) = records.next() {
        seen.push(record?.text()?.to_owned());
    }
    assert_eq!(seen, ["{\"id\":1}", "{\"id\":2}"]);

    // Pinned, a lone `\n` is content rather than a break.
    let mixed = Buffer::from_bytes(b"lf\ncrlf\r\nlast".to_vec())
        .into_text()
        .with_linesep(LineSep::CRLF);
    let mut pinned = mixed.read_lines()?;
    assert_eq!(pinned.next().unwrap()?.text()?, "lf\ncrlf");
    assert_eq!(pinned.next().unwrap()?.text()?, "last");
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    target = root / "trades.jsonl.gz"
    target.write_bytes(gzip.compress(b'{"id":1}\r\n{"id":2}\n'))

    assert list(IOBase(target).read_lines()) == ['{"id":1}', '{"id":2}']

    # Pinned, a lone `\n` is content rather than a break.
    mixed = root / "mixed.txt"
    mixed.write_bytes(b"lf\ncrlf\r\nlast")
    assert list(IOBase(mixed).read_lines(linesep=r"\r\n")) == ["lf\ncrlf", "last"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const target = path.join(root, 'trades.jsonl.gz')
    fs.writeFileSync(target, zlib.gzipSync('{"id":1}\r\n{"id":2}\n'))

    assert.deepEqual([...new IOBase(target).readLines()], ['{"id":1}', '{"id":2}'])

    // Pinned, a lone `\n` is content rather than a break.
    const mixed = path.join(root, 'mixed.txt')
    fs.writeFileSync(mixed, 'lf\ncrlf\r\nlast')
    assert.deepEqual([...new IOBase(mixed).readLines({ linesep: '\\r\\n' })], [
      'lf\ncrlf',
      'last',
    ])

    fs.rmSync(root, { recursive: true, force: true })
    ```

With a `pattern`, records group into what the pattern opens: one record starts at a matching line
and carries every following line until the next match - the shape of a log whose entries open with
a timestamp and continue with stack traces. Lines before the first match form the first record
rather than being dropped. `logs: true` is the same grouping without an expression at all: a record
opens at every line whose first bytes parse as a timestamp.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::text::TextLineOptions;

    let handle = Buffer::from_bytes(
        b"2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n  at frame one\n\
          2024-02-01 10:00:01.000_000 [ii] [beta] fine\n".to_vec(),
    )
    .into_text_with(TextLineOptions::for_logs());

    let mut records = handle.read_lines()?;
    let first = records.next().unwrap()?;
    // The stack frame joined its entry rather than becoming a record.
    assert_eq!(first.line_count(), 2);
    assert_eq!(first.text()?, "2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n  at frame one");
    assert_eq!(records.next().unwrap()?.line_count(), 1);
    assert!(records.next().is_none());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    target = pathlib.Path(tempfile.mkdtemp()) / "app.log"
    target.write_text(
        "2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n"
        "  at frame one\n"
        "2024-02-01 10:00:01.000_000 [ii] [beta] fine\n"
    )

    # A pattern names the opening line; `logs=True` needs no expression at all.
    entries = list(IOBase(target).read_lines(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}"))
    assert entries == list(IOBase(target).read_lines(logs=True))
    assert len(entries) == 2
    assert entries[0] == "2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n  at frame one"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const target = path.join(root, 'app.log')
    fs.writeFileSync(
      target,
      '2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n' +
        '  at frame one\n' +
        '2024-02-01 10:00:01.000_000 [ii] [beta] fine\n',
    )

    const entries = [...new IOBase(target).readLines('^\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}')]
    assert.deepEqual(entries, [...new IOBase(target).readLines({ logs: true })])
    assert.equal(entries.length, 2)
    assert.equal(entries[0], '2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n  at frame one')

    fs.rmSync(root, { recursive: true, force: true })
    ```

The core also has `into_read_lines`, which consumes the handle so the iterator owns it - the shape
the bindings use, and the one a Rust caller needs when the records outlive the scope that built the
handle. Stacked codings peel outermost first, so a `.jsonl.gz.zst` reads exactly as its name says it
was written.

### Writing records

`write_lines` replaces a resource's records and `append_lines` adds after its end, each record
terminated. Both take an **iterator**, never a `Vec` or an array, for the same reason the record
surface refuses one: a shape requiring materialization cannot describe a resource larger than
memory. Records accumulate in one reused 64 KiB buffer and flush in chunks, so a million-record
write allocates a constant amount and nothing is ever joined into a single value first.

The terminator written is deterministic: `\n` unless `linesep` pins one, whatever the platform. A
write is a *complete* operation, so it publishes when it finishes - a staging backend is not left
holding it.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::text::LineSep;

    let mut handle = Buffer::new().into_text();
    // A lazy iterator, never a collected `Vec`.
    handle.write_lines((0..1_000).map(|index| format!("row-{index}")))?;
    handle.append_lines(["tail"])?;
    assert!(handle.read_all_bytes()?.ends_with(b"row-999\ntail\n"));

    // A pinned terminator is written verbatim and read back exactly.
    let mut pinned = Buffer::new().into_text().with_linesep(LineSep::CRLF);
    pinned.write_lines(["one", "two"])?;
    assert_eq!(pinned.read_all_bytes()?, b"one\r\ntwo\r\n");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    handle = IOBase(root / "out.log")
    # A generator, never a list the binding materializes first.
    handle.write_lines(f"row-{index}" for index in range(1_000))
    handle.append_lines(["tail"])
    assert handle.read_bytes().endswith(b"row-999\ntail\n")
    assert len(list(handle.read_lines())) == 1_001

    pinned = IOBase(root / "crlf.log")
    pinned.write_lines(["one", "two"], linesep=r"\r\n")
    assert pinned.read_bytes() == b"one\r\ntwo\r\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'out.log'))
    // A generator, never an array the binding materializes first.
    handle.writeLines(
      (function* rows() {
        for (let index = 0; index < 1_000; index += 1) {
          yield `row-${index}`
        }
      })(),
    )
    handle.appendLines(['tail'])
    assert.ok(handle.readBytes().toString().endsWith('row-999\ntail\n'))
    assert.equal([...handle.readLines()].length, 1_001)

    const pinned = new IOBase(path.join(root, 'crlf.log'))
    pinned.writeLines(['one', 'two'], { linesep: '\\r\\n' })
    assert.equal(pinned.readBytes().toString(), 'one\r\ntwo\r\n')

    fs.rmSync(root, { recursive: true, force: true })
    ```

### Records as Arrow batches

`read_arrow_lines` projects the same records into a streaming Arrow reader - a *text-record*
surface beside `read_lines`, never a fourth record method: the record surface stays exactly
`read_arrow_batch_reader`, `write_arrow_batch_reader`, and `append_arrow_batch_reader`, and nothing
here touches how a record encoding decodes rows. One batch is in memory at a time, content codings
still decode as streams, and every column is a datatype the strict Iceberg codec accepts **as
declared**, so the parsed batches append into an Iceberg table without a single widening:

| column    | datatype     | meaning |
| --------- | ------------ | ------- |
| `url`     | `utf8`       | the resource's canonical `Url` display |
| `rownum`  | `int64`      | 1-based record index within the resource |
| `date`    | `date32`     | the entry's civil date |
| `time`    | `time64(us)` | the clock reading, truncated - never rounded - to microseconds |
| `unix`    | `int64`      | total nanoseconds since the Unix epoch; naive unless `timezone` is set |
| `hash`    | `int64`      | the stable 64-bit FNV-1a of `message` alone, reinterpreted as `i64` bit for bit - a dedupe and join key that holds across files, runs, and releases |
| `header`  | `utf8`       | the exact text the pattern matched, within the record's opening line |
| `message` | `utf8`       | the record with the header match removed, then trimmed |
| `offset`  | `int64`      | byte offset of the record's first line in the *decoded* stream - the resume key a tailing reader seeks back to |
| `lines`   | `int32`      | how many lines the record spans |

In log mode - `logs: true`, or `Opening::Timestamp` - three more fixed columns follow: `level`,
`logger`, and `thread`, each read from a **closed token table** rather than from an expression, and
each nullable because a line need not carry one.

Then one nullable column per **named capture group**, in group order - the primary custom-field
mechanism. A capture whose whole sub-pattern is one of a closed table of exact spellings types
itself - `(?<thread_id>\d+)` or `[0-9]+` is `int64`, `(?<qty>\d+\.\d+)` is `float64` - and every
other capture is `utf8`, because inference is a deterministic table, never a guess about what a
regex might match. `capture_types` declares a capture's datatype explicitly - `(?<price>[0-9.]+)`
as `decimal(9, 2)`, or an inferred numeric back to `utf8` - and typed columns parse through the one
cast definition as each batch closes, strictly: a captured text the datatype cannot read is an
error, never a silent null. Then the constant `custom_fields` columns a caller stamps onto every
row, typed from their values. Capture declarations and custom columns alike are validated against
the Iceberg vocabulary before anything is read.

A batch closes on whichever bound trips first: `byte_size` counts the *decoded input bytes* of the
records appended - not Arrow buffer memory, so it is not an allocation cap - and `batch_size`
counts rows. Both default (8 MiB, 65,536 rows), so a corpus of long records and a corpus of short
ones both produce batches of a workable size without being told anything.

The entry timestamp is parsed off the front of the matched header - `T`, `t`, or a space between
date and clock, and a fraction whose digits may be grouped with `_`, so `10:00:00.000_000` reads
exactly as `10:00:00.000000`; a malformed timestamp in a *matched* header is a typed error naming
the row and byte position, never a silent null. A record the pattern did not open - the preamble a
rotated file starts with - keeps null `date`/`time`/`unix`/`header` and capture columns and carries
the whole record as `message`.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::text::TextLineOptions;
    use yggdryl::{Url, Value};

    let mut handle = Buffer::new()
        .with_media_type(Url::from_str("file:///app.log.gz")?.media_type());
    handle.write_all_bytes(&yggdryl::gzip::dump(
        b"2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n    at frame one\n\
          2024-02-01 10:00:01.500 [ii] [beta] fill 100 @ 187.23\n",
    )?)?;

    let options = TextLineOptions::with_pattern(
        r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]",
    )?
    .try_with_custom_fields([("venue", Value::from("XNAS"))])?;

    let batches: Vec<_> = handle.read_arrow_lines(&options)?.collect::<Result<_, _>>()?;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 2);

    let names: Vec<&str> = batches[0].schema_ref().fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(
        names,
        ["url", "rownum", "date", "time", "unix", "hash", "header", "message",
         "offset", "lines", "level", "logger", "venue"],
    );
    let unix = batches[0].column(4).as_any().downcast_ref::<arrow_array::Int64Array>().unwrap();
    assert_eq!(unix.value(0), 1_706_781_600_000_000_000);
    let levels = batches[0].column(10).as_any().downcast_ref::<arrow_array::StringArray>().unwrap();
    assert_eq!(levels.value(0), "ee");
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    from yggdryl import IOBase

    target = pathlib.Path(tempfile.mkdtemp()) / "app.log.gz"
    target.write_bytes(gzip.compress(
        b"2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n    at frame one\n"
        b"2024-02-01 10:00:01.500 [ii] [beta] fill 100 @ 187.23\n"
    ))

    reader = IOBase(target).read_arrow_lines(
        r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]",
        custom_fields={"venue": "XNAS"},
    )
    table = reader.read_all()
    assert table.num_rows == 2
    assert table.column_names[-3:] == ["level", "logger", "venue"]
    assert table.to_pydict()["unix"][0] == 1_706_781_600_000_000_000
    assert table.to_pydict()["level"] == ["ee", "ii"]
    assert table.to_pydict()["message"][0] == "boom\n    at frame one"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const target = path.join(root, 'app.log.gz')
    fs.writeFileSync(target, zlib.gzipSync(
      '2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n    at frame one\n' +
        '2024-02-01 10:00:01.500 [ii] [beta] fill 100 @ 187.23\n',
    ))

    const pattern =
      '^\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\S* \\[(?<level>[^\\]]+)\\] \\[(?<logger>[^\\]]+)\\]'
    const table = new IOBase(target)
      .readArrowLines(pattern, { customFields: { venue: 'XNAS' } })
      .toTable()
    assert.equal(table.numRows, 2)
    assert.deepEqual([...table.getChild('level')], ['ee', 'ii'])
    assert.equal([...table.getChild('unix')][0], 1_706_781_600_000_000_000n)
    assert.deepEqual([...table.getChild('venue')], ['XNAS', 'XNAS'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

### A reader is a configuration document

Every option above is one entry in a mapping, so the whole extractor is exactly what a JSON, YAML,
or TOML document parses into. That is the point of the options value: an optimized Arrow reader is
specifiable **from configuration alone** - no Rust, no Python callbacks, no per-row code in any
language - and the same document round-trips back out. Both spellings validate through the one
conversion, so a document is refused exactly where code would be: an unknown key names itself, a
capture type naming a capture the pattern does not have is an error, an unknown zone is an error.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::text::TextLineOptions;
    use yggdryl::Field;

    let document = r#"
    pattern: '^(?<stamp>\S+) \[(?<level>[A-Z]+)\]'
    byte_size: 1048576
    batch_size: 4096
    timestamp_capture: stamp
    timezone: Europe/Paris
    capture_types:
      level: utf8
    custom_fields:
      source: gateway
    "#;

    let options = TextLineOptions::from_value(yggdryl::yaml::from_str(document)?)?;
    assert_eq!(options.byte_size(), Some(1 << 20));
    assert_eq!(options.timestamp_capture(), Some("stamp"));

    // The schema answers from the document alone - no resource in sight - so
    // the table exists before the first log line does.
    let names: Vec<&str> = options.schema().fields().iter().map(Field::name).collect();
    assert_eq!(names[names.len() - 2..], ["level", "source"]);

    let handle = Buffer::from_bytes(b"2024-02-01T10:00:00 [ERROR] boom\n".to_vec());
    let batch = handle.read_arrow_lines(&options)?.next().unwrap()?;
    assert_eq!(batch.num_rows(), 1);

    // And the options project back out, so a reader can be persisted as one.
    assert_eq!(TextLineOptions::from_value(options.to_value())?.pattern(), options.pattern());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, schema_from_pattern, yaml

    document = """
    pattern: '^(?<stamp>\\S+) \\[(?<level>[A-Z]+)\\]'
    byte_size: 1048576
    batch_size: 4096
    timestamp_capture: stamp
    capture_types:
      level: utf8
    custom_fields:
      source: gateway
    """
    options = yaml.loads(document)

    # The schema answers from the document alone, with no resource in sight.
    schema = schema_from_pattern(options=options)
    assert [field.name for field in schema][-2:] == ["level", "source"]

    target = pathlib.Path(tempfile.mkdtemp()) / "app.log"
    target.write_text("2024-02-01T10:00:00 [ERROR] boom\n")
    table = IOBase(target).read_arrow_lines(options=options).read_all()
    assert table.column("source").to_pylist() == ["gateway"]

    # Keywords refine a document rather than replacing it.
    assert IOBase(target).read_arrow_lines(options=options, batch_size=1).read_all().num_rows == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, schemaFromPattern, yaml } = require('yggdryl')

    const options = yaml.loads(
      [
        "pattern: '^(?<stamp>\\S+) \\[(?<level>[A-Z]+)\\]'",
        'byte_size: 1048576',
        'batch_size: 4096',
        'timestamp_capture: stamp',
        'custom_fields:',
        '  source: gateway',
      ].join('\n'),
    )

    // The schema answers from the document alone, with no resource in sight.
    const schema = schemaFromPattern(options)
    assert.equal(String(schema.dataType.get('source').dataType), 'utf8')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const target = path.join(root, 'app.log')
    fs.writeFileSync(target, '2024-02-01T10:00:00 [ERROR] boom\n')
    const table = new IOBase(target).readArrowLines(options).toTable()
    assert.deepEqual([...table.getChild('source')], ['gateway'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

`schema_from_pattern` is the same builder without a reader, so a table can be created from the
projection's root first and the parse appended after. Named captures arrive typed, by the
deterministic sub-pattern table or by declaration:

=== "Rust"

    ```rust
    use arrow_array::Array;
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::text::{TextLineOptions, schema_from_pattern};
    use yggdryl::{DataType, Url};

    let pattern = r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \[(?<thread_id>\d+)\] \((?<log_level>\w+)\) qty=(?<qty>[0-9.]+)";

    // The standalone builder answers the emitted root without a reader:
    // `thread_id` types itself off its own `\d+` sub-pattern.
    let schema = schema_from_pattern(pattern)?;
    assert_eq!(
        schema.get_field_by_name("thread_id").unwrap().data_type(),
        &DataType::Int64
    );

    let mut handle = Buffer::new()
        .with_media_type(Url::from_str("file:///app.log")?.media_type());
    handle.write_all_bytes(b"2024-02-01 10:00:00 [42] (info) qty=1.50 fill\n")?;

    // A declaration types what inference cannot: `qty` lands as a decimal.
    let options = TextLineOptions::with_pattern(pattern)?
        .try_with_capture_types([("qty", DataType::decimal(9, 2)?)])?;
    let batch = handle.read_arrow_lines(&options)?.next().unwrap()?;
    let threads = batch
        .column_by_name("thread_id")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::Int64Array>()
        .unwrap();
    assert_eq!(threads.value(0), 42);
    let quantities = batch
        .column_by_name("qty")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::Decimal128Array>()
        .unwrap();
    assert_eq!(quantities.value(0), 150, "1.50 at scale 2");
    ```

=== "Python"

    ```python
    import decimal
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase, schema_from_pattern

    pattern = (
        r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}"
        r" \[(?<thread_id>\d+)\] \((?<log_level>\w+)\) qty=(?<qty>[0-9.]+)"
    )

    # The standalone builder answers the emitted root without a reader.
    schema = schema_from_pattern(pattern, capture_types={"qty": "decimal(9, 2)"})
    assert str(schema.data_type["thread_id"].data_type) == "int64"

    target = pathlib.Path(tempfile.mkdtemp()) / "app.log"
    target.write_text("2024-02-01 10:00:00 [42] (info) qty=1.50 fill\n")
    table = (
        IOBase(target)
        .read_arrow_lines(pattern, capture_types={"qty": "decimal(9, 2)"})
        .read_all()
    )
    assert table.schema.field("thread_id").type == pa.int64()
    assert table.to_pydict()["thread_id"] == [42]
    assert table.to_pydict()["qty"] == [decimal.Decimal("1.50")]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, schemaFromPattern } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const target = path.join(root, 'app.log')
    fs.writeFileSync(target, '2024-02-01 10:00:00 [42] (info) qty=1.50 fill\n')

    const pattern =
      '^\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2} \\[(?<thread_id>\\d+)\\] \\((?<log_level>\\w+)\\) qty=(?<qty>[0-9.]+)'

    // The standalone builder answers the emitted root without a reader.
    const schema = schemaFromPattern(pattern, { captureTypes: { qty: 'decimal(9, 2)' } })
    assert.equal(String(schema.dataType.get('thread_id').dataType), 'int64')

    const table = new IOBase(target)
      .readArrowLines(pattern, { captureTypes: { qty: 'decimal(9, 2)' } })
      .toTable()
    assert.deepEqual([...table.getChild('thread_id')], [42n])
    assert.deepEqual([...table.getChild('log_level')], ['info'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

### Trimming, and a zone that makes `unix` an instant

`lstrip` and `rstrip` are independent and are **span arithmetic**, not string surgery: they move
the record's own start and end, so the `hash`, the `message`, and the reported spans all describe
the trimmed record and nothing is copied to trim it. Each is `whitespace` (the default, Unicode),
`none`, `ascii`, or `chars:...` naming an explicit set.

`unix` is a naive count by default - the civil reading counted from the epoch, no zone applied,
which is the only honest answer when a log carries no offset. Setting `timezone` makes it a real
instant: **an offset present in the timestamp text always wins**, and the option applies only to
readings that carry none. Near a daylight-saving transition the reading is resolved by an explicit
policy rather than a guess - the *earliest* instant for a local time that occurs twice, a shift
forward for one that does not occur at all - and the zone's offset is cached, because a sorted log
resolves it once or twice per file rather than once per row.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::text::{Strip, TextLineOptions};
    use yggdryl::Timezone;

    let handle = Buffer::from_bytes(b"2024-02-01T00:00:00 [INFO] x\n".to_vec());
    let pattern = r"^(?<stamp>\S+) \[(?<level>[A-Z]+)\]";

    let naive = TextLineOptions::with_pattern(pattern)?;
    let batch = handle.read_arrow_lines(&naive)?.next().unwrap()?;
    let unix = |batch: &arrow_array::RecordBatch| {
        use arrow_array::Array as _;
        batch.column(4).as_any().downcast_ref::<arrow_array::Int64Array>().unwrap().value(0)
    };
    assert_eq!(unix(&batch), 1_706_745_600_000_000_000);

    // Read in Paris, the same civil reading is an hour earlier as an instant.
    let zoned = TextLineOptions::with_pattern(pattern)?
        .try_with_timezone("Europe/Paris".parse::<Timezone>()?)?
        .with_rstrip(Strip::Ascii);
    let batch = handle.read_arrow_lines(&zoned)?.next().unwrap()?;
    assert_eq!(unix(&batch), 1_706_745_600_000_000_000 - 3_600 * 1_000_000_000);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    target = pathlib.Path(tempfile.mkdtemp()) / "app.log"
    target.write_text("2024-02-01T00:00:00 [INFO] x\n")
    pattern = r"^(?<stamp>\S+) \[(?<level>[A-Z]+)\]"

    source = IOBase(target)
    naive = source.read_arrow_lines(pattern).read_all().column("unix").to_pylist()
    zoned = (
        source.read_arrow_lines(pattern, timezone="+02:00", rstrip="ascii")
        .read_all()
        .column("unix")
        .to_pylist()
    )
    assert naive == [1_706_745_600_000_000_000]
    assert zoned == [naive[0] - 2 * 3_600 * 1_000_000_000]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const target = path.join(root, 'app.log')
    fs.writeFileSync(target, '2024-02-01T00:00:00 [INFO] x\n')
    const pattern = '^(?<stamp>\\S+) \\[(?<level>[A-Z]+)\\]'

    const source = new IOBase(target)
    const naive = [...source.readArrowLines(pattern).toTable().getChild('unix')]
    const zoned = [
      ...source
        .readArrowLines(pattern, { timezone: '+02:00', rstrip: 'ascii' })
        .toTable()
        .getChild('unix'),
    ]
    assert.equal(naive[0], 1_706_745_600_000_000_000n)
    assert.equal(zoned[0], naive[0] - 2n * 3_600n * 1_000_000_000n)

    fs.rmSync(root, { recursive: true, force: true })
    ```

The handle's `IOKind` decides the shape, never a second existence check. A leaf parses its own
records; a folder streams the leaf files beneath it in name-sorted order, each opened lazily as the
reader reaches it, each decoded by its *own* media type - a directory mixing `a.log` and
`b.log.gz` reads uniformly - each contributing its own `url` and restarting `rownum` at 1, so
`(url, rownum)` stays a record identity and a batch never spans two leaves. A resource that does
not exist reads as zero batches with the schema still answered, exactly as `read_lines` yields no
records. The consuming `into_arrow_lines` is the same projection when the reader must own the
handle - the shape the bindings hand across FFI.

What that costs on a production-sized directory is measured rather than asserted: the
[benchmarks](benchmarks.md#big-gzip-log-files-end-to-end) page reads a million records across
eight rotated `.log.gz` leaves - 93.6 MiB of text from 16.8 MiB on disk - at ~136,000 rows/s,
with residency bounded by one batch rather than by the corpus, and says plainly which of the
structural deltas its container is too noisy to report.

### Streaming a trading log into a partitioned Iceberg table

The projection's schema is Iceberg-safe as declared, so the whole pipeline is three calls: mark
the partition columns on the schema, create the table through the
[catalog](iceberg.md), and append the *reader itself* - the parse streams straight into the
commit, one batch in memory at a time, and every append is one snapshot whose metadata - summary
counts, one manifest per commit, per-file partition tuples, column bounds - is written correctly
for the next reader to prune with.

=== "Rust"

    ```rust
    use yggdryl::iceberg::Catalog;
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;
    use yggdryl::text::TextLineOptions;
    use yggdryl::Value;

    let root = std::env::temp_dir().join("yggdryl-doc-log-lake");
    let _ = std::fs::remove_dir_all(&root);
    let logs = root.join("incoming");
    std::fs::create_dir_all(&logs)?;
    std::fs::write(
        logs.join("a.log"),
        b"2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n    at frame one\n\
          2024-02-01 10:00:01.000_000 [ii] [beta] fill 100 @ 187.23\n",
    )?;
    std::fs::write(
        logs.join("b.log.gz"),
        yggdryl::gzip::dump(b"2024-02-01 11:00:00.000_000 [ii] [gamma] fill 200 @ 188.01\n")?,
    )?;

    let options = TextLineOptions::with_pattern(
        r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]",
    )?
    .try_with_custom_fields([("venue", Value::from("XNAS"))])?;

    // The options already know the schema, so the table exists - partitioned
    // by `level` - before the first log line is parsed.
    let catalog = Catalog::new(Folder::new(root.join("warehouse"))?);
    catalog.create_table("logs.app", options.schema().with_partition_fields(&["level"])?)?;

    // The append consumes the parse itself: a lazy reader over both leaves,
    // the gzip one decoded as a stream, routed row by row into `level=...`
    // partitions and committed as one snapshot.
    let table = catalog.append("logs.app", Folder::new(&logs)?.into_arrow_lines(&options)?)?;

    let snapshot = table.current_snapshot().expect("one commit");
    assert_eq!(snapshot.operation(), "append");
    assert_eq!(snapshot.summary_value("added-records"), Some("3"));
    assert_eq!(snapshot.summary_value("added-data-files"), Some("2"), "one file per level");

    // A second day appends a second snapshot; the metadata accumulates.
    let mut day_two = yggdryl::io::Buffer::new()
        .with_media_type(yggdryl::Url::from_str("file:///c.log")?.media_type());
    day_two.write_all_bytes(b"2024-02-02 09:30:00.000_000 [ee] [delta] second day\n")?;
    let table = catalog.append("logs.app", day_two.into_arrow_lines(&options)?)?;
    assert_eq!(table.metadata().snapshots.len(), 2);
    assert_eq!(table.current_snapshot().unwrap().summary_value("total-records"), Some("4"));
    assert_eq!(table.manifests()?.len(), 2, "one manifest per append");

    // The partition tuples just committed are what the next read prunes with.
    let plan = table.plan(&[("level", "ee")])?;
    assert_eq!(plan.tasks.len(), 2);
    assert_eq!(plan.files_skipped(), 1, "the ii file is never opened");
    let rows: usize = table
        .scan_where(&[("level", "ii")], None)?
        .map(|batch| batch.map(|b| b.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(rows, 2);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    from yggdryl import DataType, Field, IOBase
    from yggdryl.iceberg import Catalog

    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-"))
    logs = root / "incoming"
    logs.mkdir()
    (logs / "a.log").write_bytes(
        b"2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n    at frame one\n"
        b"2024-02-01 10:00:01.000_000 [ii] [beta] fill 100 @ 187.23\n"
    )
    (logs / "b.log.gz").write_bytes(
        gzip.compress(b"2024-02-01 11:00:00.000_000 [ii] [gamma] fill 200 @ 188.01\n")
    )

    pattern = (
        r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S*"
        r" \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]"
    )
    reader = IOBase(logs).read_arrow_lines(pattern, custom_fields={"venue": "XNAS"})

    # The reader's schema is the table's schema; marking `level` partitions it.
    marked = Field(
        "row",
        DataType.from_fields([Field.from_arrow(field) for field in reader.schema]),
        nullable=False,
    ).with_partition_fields(["level"])
    catalog = Catalog(root / "warehouse")
    catalog.create_table("logs.app", marked)

    # The append consumes the parse itself - lazy, one batch at a time.
    table = catalog.append("logs.app", reader)
    assert [field.name for field in table.spec.fields] == ["level"]
    snapshot = table.current_snapshot
    assert snapshot.operation == "append"
    assert snapshot.summary["added-records"] == "3"
    assert snapshot.summary["added-data-files"] == "2"  # one file per level

    # One partition tuple per data file, straight from the manifest just written.
    partitions = sorted(file.partition[0] for file, _ in table.data_files())
    assert partitions == ["ee", "ii"]

    table = catalog.append(
        "logs.app",
        IOBase(logs / "a.log").read_arrow_lines(pattern, custom_fields={"venue": "XNAS"}),
    )
    assert len(table.snapshots) == 2
    # a.log holds two records, so the totals accumulate 3 + 2.
    assert table.current_snapshot.summary["total-records"] == "5"
    assert table.scan().read_all().num_rows == 5

    import shutil

    shutil.rmtree(root)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { IOBase, iceberg } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-'))
    const logs = path.join(root, 'incoming')
    fs.mkdirSync(logs)
    fs.writeFileSync(
      path.join(logs, 'a.log'),
      '2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n    at frame one\n' +
        '2024-02-01 10:00:01.000_000 [ii] [beta] fill 100 @ 187.23\n',
    )
    fs.writeFileSync(
      path.join(logs, 'b.log.gz'),
      zlib.gzipSync('2024-02-01 11:00:00.000_000 [ii] [gamma] fill 200 @ 188.01\n'),
    )

    const pattern =
      '^\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\S* \\[(?<level>[^\\]]+)\\] \\[(?<logger>[^\\]]+)\\]'
    const reader = new IOBase(logs).readArrowLines(pattern, { customFields: { venue: 'XNAS' } })

    // The reader's root field is the table's schema; marking `level` partitions it.
    const catalog = new iceberg.Catalog(path.join(root, 'warehouse'))
    catalog.createTable('logs.app', reader.field.withPartitionFields(['level']))

    // The append consumes the parse itself - lazy, one batch at a time.
    const table = catalog.append('logs.app', reader)
    const snapshot = table.currentSnapshot
    assert.equal(snapshot.operation, 'append')
    assert.equal(snapshot.summary['added-records'], '3')
    assert.equal(snapshot.summary['added-data-files'], '2') // one file per level

    const again = catalog.append(
      'logs.app',
      new IOBase(path.join(logs, 'a.log')).readArrowLines(pattern, {
        customFields: { venue: 'XNAS' },
      }),
    )
    assert.equal(again.snapshots.length, 2)
    // a.log holds two records, so the totals accumulate 3 + 2.
    assert.equal(again.currentSnapshot.summary['total-records'], '5')
    assert.equal(again.scan().toTable().numRows, 5)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## What the bytes are

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::MimeType;

    // Nothing names an in-memory buffer, so its type comes from its bytes.
    let mut handle = Buffer::from_bytes(br#"{"symbol":"AAPL"}"#.to_vec());
    assert_eq!(handle.media_type().base(), &MimeType::JSON);

    // It is re-derived after the content changes.
    handle.write_all_bytes(b"PAR1payload")?;
    assert_eq!(handle.media_type().base(), &MimeType::PARQUET);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    # Nothing names an in-memory buffer, so its type comes from its bytes.
    handle = IOBase.from_bytes(b'{"symbol":"AAPL"}')
    assert str(handle.media_type.base) == "application/json"

    # It is re-derived after the content changes.
    handle.write_bytes(b"PAR1payload")
    assert str(handle.media_type.base) == "application/vnd.apache.parquet"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, MimeType } = require('yggdryl')

    // Nothing names an in-memory buffer, so its type comes from its bytes.
    const handle = IOBase.fromBytes(Buffer.from('{"symbol":"AAPL"}'))
    assert.ok(handle.mediaType.base.equals(MimeType.JSON))

    // It is re-derived after the content changes.
    handle.writeBytes(Buffer.from('PAR1payload'))
    assert.ok(handle.mediaType.base.equals(MimeType.PARQUET))
    ```

`media_type` is the second thing a handle carries, next to the optional `url` naming where the bytes
live. It answers both questions a caller has: what representation the bytes are, and what content
codings sit on top.

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{Codec, MimeType, Url};

// A declared type wins, and the codings it carries are what `codec` reports.
let named = Buffer::new().with_media_type(Url::from_str("file:///trades.json.gz")?.media_type());
assert_eq!(named.media_type().base(), &MimeType::JSON);
assert_eq!(named.codec(), Codec::Gzip);
```

`codec` reads the last coding out of the media type, which is how compression is never passed as a
separate argument. `set_media_type` declares one explicitly, which is required for a format that
content cannot identify. Both cross into the bindings - `codec` as a read-only property,
`media_type` as a settable one - and the next section is what a caller does with the coding once it
has been read.

## Adding and removing a coding

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::{Codec, Url};

    let mut plain = Buffer::new().with_media_type(Url::from_str("file:///rows.json")?.media_type());
    plain.write_all_bytes(br#"{"symbol":"AAPL"}"#)?;
    // Nothing wraps these bytes, so there is nothing to undo.
    assert_eq!(plain.codec(), Codec::Identity);

    let mut encoded =
        Buffer::new().with_media_type(Url::from_str("file:///rows.json.gz")?.media_type());
    assert_eq!(encoded.codec(), Codec::Gzip);

    // The coding is an argument here, and the target's name is one place to read it from.
    let codec = encoded.codec();
    plain.compress_into(&mut encoded, codec)?;
    assert_eq!(&encoded.read_all_bytes()?[..2], b"\x1f\x8b");

    let mut decoded = Buffer::new();
    encoded.decompress_into(&mut decoded)?;
    assert_eq!(decoded.read_all_bytes()?, plain.read_all_bytes()?);
    assert_eq!(decoded.codec(), Codec::Identity);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    plain = IOBase(root / "rows.json")
    plain.write_bytes(b'{"symbol":"AAPL"}')

    # Nothing wraps these bytes, so there is nothing to undo.
    assert plain.codec is None

    encoded = IOBase(root / "rows.json.gz")
    assert encoded.codec == "gzip"

    # The target's name already said gzip, so nothing here repeats it.
    assert plain.compress_into(encoded) == encoded.size
    assert encoded.read_bytes()[:2] == b"\x1f\x8b"

    decoded = IOBase(root / "roundtrip.json")
    assert encoded.decompress_into(decoded) == 17
    assert decoded.read_bytes() == plain.read_bytes()
    assert decoded.codec is None

    # A target declaring no coding is refused rather than copied unchanged.
    reason = None
    try:
        plain.compress_into(IOBase(root / "copy.json"))
    except ValueError as error:
        reason = str(error)
    assert "expected a target declaring a content coding" in reason
    assert not (root / "copy.json").exists()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const plain = new IOBase(path.join(root, 'rows.json'))
    plain.writeBytes(Buffer.from('{"symbol":"AAPL"}'))

    // Nothing wraps these bytes, so there is nothing to undo.
    assert.equal(plain.codec, null)

    const encoded = new IOBase(path.join(root, 'rows.json.gz'))
    assert.equal(encoded.codec, 'gzip')

    // The target's name already said gzip, so nothing here repeats it.
    assert.equal(plain.compressInto(encoded), encoded.size)
    assert.deepEqual([...encoded.readBytes().subarray(0, 2)], [0x1f, 0x8b])

    const decoded = new IOBase(path.join(root, 'roundtrip.json'))
    assert.equal(encoded.decompressInto(decoded), 17)
    assert.equal(decoded.readText(), '{"symbol":"AAPL"}')
    assert.equal(decoded.codec, null)

    // An in-memory target has no name to declare a coding, so this one is named.
    const memory = IOBase.fromBytes()
    assert.ok(plain.compressInto(memory, 'zstd') > 0)
    assert.equal(memory.codec, 'zstd')

    // A target declaring no coding is refused rather than copied unchanged.
    assert.throws(
      () => plain.compressInto(new IOBase(path.join(root, 'copy.json'))),
      /expected a target declaring a content coding/,
    )
    assert.equal(fs.existsSync(path.join(root, 'copy.json')), false)

    fs.rmSync(root, { recursive: true, force: true })
    ```

`codec` answers what coding the handle's *own name* declares - `rows.json.gz` is gzip - and an
in-memory handle answers off the media type it was told to hold. Absence is spelled as absence:
Python answers `None` and JavaScript `null`, never the string `"identity"`, because the question a
caller is asking here is whether there is anything to undo, and a sentinel spelling of "no" is one
more value every branch has to know about.

`compress_into` and `decompress_into` - `compressInto` and `decompressInto` in JavaScript - move
every byte from one handle into another and add or remove a coding on the way. The coding defaults
to the one a name already declares: the target's for a compress, the source's for a decompress, so
writing a `rows.json` into a `rows.json.gz` never spells gzip twice, and reading it back needs no
argument at all. That works because the transfer records the coding in the target's media type,
which is the same place a located handle reads it from. Naming a coding explicitly overrides the
default - the escape hatch for an in-memory target, which has no name to declare anything, and for
bytes whose name lies about what they hold. `level` is the shared 0-9 scale.

Both bindings refuse a compress into a target that declares no coding rather than writing the bytes
through unchanged, and the refusal names the media type the target does carry. A coding nobody named
is a coding nobody can decode by name later, so a silent identity copy is a failure that surfaces one
reader downstream instead of here. Rust asks for the coding outright, so the question never arises:
there is no name to default from until a caller reads one, which is what the Rust example above does
in the open.

None of this sits on the reading path. A record encoding and a text codec already read *through* the
codings a name declares - `trades.arrows.gz` writes gzipped and reads straight back as batches, and
[text.md](text.md)'s `load` peels the same coding off a `quote.json.gz` - while `read_lines` peels
them as streams rather than as values. These two methods are for moving bytes between
representations: publishing a plain file as a compressed one, or handing an outside tool a form it
can open. What coding sits on the bytes is not something a reader has to think about. The codings
themselves are documented per format in [gzip.md](gzip.md), [zlib.md](zlib.md), and
[zstd.md](zstd.md).

## Open and close

!!! note "Rust and Python"
    The JavaScript package does not expose the scoped pair yet.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, Coded, IOBase};
    use yggdryl::Codec;

    let mut handle = Coded::new(Buffer::new(), Codec::Zstd);
    assert!(!handle.opened());

    handle.open()?;
    assert!(handle.opened());
    handle.write_all_bytes(b"symbol,price\n")?;

    // Closing publishes the pending write and releases the cache.
    handle.close()?;
    assert!(!handle.opened());

    // The handle stays usable; the next read re-materializes.
    assert_eq!(handle.read_all_bytes()?, b"symbol,price\n");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    path = pathlib.Path(tempfile.mkdtemp()) / "trades.csv"

    # `with` is the scoped pair: `__enter__` opens and `__exit__` closes.
    with IOBase(path) as handle:
        handle.write_text("symbol,price\n")
        assert handle.opened
        assert not handle.closed

    # Closing published the bytes at their exact length, which is what another
    # reader needs; the handle stays usable and simply re-materializes.
    assert path.stat().st_size == 13
    assert IOBase(path).read_text() == "symbol,price\n"
    ```

A handle works without `open`: every operation materializes what it needs. Calling it moves that cost
to a known point and keeps the cached state alive across many small operations instead of re-deriving
it per call. Opening an already-open handle is a no-op, and opening a resource that does not exist yet
succeeds without creating it - creation still waits for the first write. `close` flushes and releases
what `open` cached; the handle remains usable afterwards.

The cache is strictly opt-in, and that is the whole contract: **open caches, closed fetches**. A
closed handle re-derives its metadata on every ask, so a resource that changes underneath it is seen
immediately; an open one holds what `open` cached until `close`, which is what makes many small
operations against one resource cheap. Nothing fills the cache as a side effect of an ordinary read,
because a cache nobody asked for is how a handle serves a stale answer.

What is cached depends on the implementation. `Buffer` has nothing to cache, so the trait defaults
apply and `opened` stays `false`. `local::File` caches the descriptor and the memory mapping.
`Coded` caches the decoded value. The media wrappers cache exactly the metadata their format keeps
re-reading: `Ipc` the stream's schema, `Parquet` the footer - so a schema probe, a statistics read,
and a batch read inside one scope pay for the footer once. These are the operations a scoped context
binds to, which is why the bindings can map `__enter__`/`using` onto them directly.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

target = pathlib.Path(tempfile.mkdtemp()) / "lake" / "trades.parquet"
IOBase(target).write_arrow(pa.table({"id": [1, 2], "venue": ["XNAS", "XNYS"]}))

# Metadata-heavy work belongs inside the scope: the schema probe, the
# per-batch reads, and the size checks all reuse what `open` cached, and
# `close` releases it at a known point.
rows = 0
with IOBase(target) as handle:
    field = handle.read_arrow_field()
    for batch in handle.read_arrow_batch_reader():
        rows += batch.num_rows
assert rows == 2

# Outside a scope the same calls still work - each one just fetches fresh,
# which is exactly right for a resource another writer may be changing.
assert IOBase(target).read_arrow_field() == field
```

## Clearing and removing

`clear` and `remove` are the lifecycle pair, and what they mean is stated by kind rather than
implied by a byte operation. `clear` empties the contents and keeps the resource: a leaf keeps
existing with size `0`, a container keeps existing and loses every child recursively. `remove`
deletes the resource itself - completely, whatever "the resource" is for that handle. A wrapping
handle removes what it *wraps*: `Gzip::remove` deletes the `.gz` resource rather than discarding a
decoded buffer, and a media handle's cached schema or footer goes with it.

Two rules hold on both, and they are the reason the pair exists at all:

- **Absence is a no-op success.** Neither call is an error on a resource that is not there, and
  neither creates one. `clear` is not a write.
- **Absence is reached without a probe.** An implementation issues the delete and treats the
  backend's own not-found answer as success. It never calls `kind`, `size`, an exists check, or
  `ls` first to decide whether to proceed - on a remote backend each of those is a second round
  trip, and a recursive delete would become a flood of them. Only not-found maps to success; a
  permission, network, or busy failure stays the typed error it is.

Because absence and successful removal are indistinguishable, `remove` returns nothing rather than
a bool or a count - reporting "did something exist" would force exactly the probe this refuses.
`recursive` is ignored on a leaf, and a container that still has children is refused by name rather
than silently recursed into.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::local::Folder;

    let root = std::env::temp_dir().join(format!("yggdryl-docs-lifecycle-{}", std::process::id()));
    let mut folder = Folder::new(&root)?;
    folder.truncate(0)?;
    folder.child_by_path("a.log")?.write_all_bytes(b"line\n")?;

    // Clearing empties the container and keeps it.
    folder.clear()?;
    assert_eq!(folder.ls(true, false).count(), 0);
    assert_eq!(folder.kind(), yggdryl::IOKind::Directory);

    // Removing deletes it; a second call succeeds, having done nothing. A
    // handle asked for as a container keeps answering `Directory`, because that
    // is what it was asked for - the parent's listing is what shows it gone.
    let leaf = root.join("nested");
    let mut nested = Folder::new(&leaf)?;
    nested.truncate(0)?;
    assert_eq!(folder.ls(false, false).count(), 1);
    nested.remove(false)?;
    nested.remove(false)?;
    assert_eq!(folder.ls(false, false).count(), 0);
    folder.remove(false)?;

    // A wrapping handle removes what it wraps, cache included.
    let mut coded = yggdryl::gzip::Gzip::new(Buffer::new());
    coded.write_all_bytes(b"symbol,price\n")?;
    coded.remove(false)?;
    assert_eq!(coded.size(), 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    handle = IOBase(root / "logs")
    handle.mkdir()
    (handle / "a.log").write_text("line\n")

    # Clearing empties the container and keeps it.
    handle.clear()
    assert list(handle.iterdir()) == []
    assert handle.is_dir()

    # Removing deletes it; a second call succeeds, having done nothing. A
    # handle asked for as a container keeps answering `is_dir`, because that
    # is what it was asked for - the parent's listing is what shows it gone.
    handle.remove()
    handle.remove()
    assert list(IOBase(root).iterdir()) == []

    # A container that still has children is refused rather than recursed into.
    handle.mkdir()
    (handle / "a.log").write_text("line\n")
    try:
        handle.remove()
    except Exception as error:
        assert "children" in str(error)
    handle.remove(recursive=True)
    assert list(IOBase(root).iterdir()) == []

    # The handle stays usable and lazy - a write recreates the resource.
    leaf = IOBase(root / "trades.csv")
    leaf.write_text("symbol,price\n")
    leaf.remove()
    assert not leaf.exists()
    leaf.write_text("symbol,price\n")
    assert leaf.read_text() == "symbol,price\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = path.join(os.tmpdir(), `yggdryl-docs-lifecycle-${process.pid}`)
    const handle = new IOBase(path.join(root, 'logs'))
    handle.mkdir()
    handle.joinpath(['a.log']).writeText('line\n')

    // Clearing empties the container and keeps it.
    handle.clear()
    assert.equal([...handle.ls(true, false)].length, 0)

    // Removing deletes it; a second call succeeds, having done nothing.
    handle.remove()
    handle.remove()
    assert.equal([...new IOBase(root).ls(false, false)].length, 0)

    // The handle stays usable and lazy - a write recreates the resource.
    const leaf = new IOBase(path.join(root, 'trades.csv'))
    leaf.writeText('symbol,price\n')
    leaf.remove()
    assert.equal(leaf.exists(), false)
    leaf.writeText('symbol,price\n')
    assert.equal(leaf.readText(), 'symbol,price\n')
    new IOBase(root).remove(true)
    ```

`Table` decides both deliberately rather than inheriting a folder's answers. `clear` commits one
snapshot carrying no data files, so the table still exists with its schema, properties, and history
and holds zero rows - deleting files behind the manifests would leave exactly the orphaned state a
complete operation must not. `remove` deletes the table's whole location, metadata tree and data
files together, because dropping a table is not emptying it.

## Buffer

!!! note "Rust only"
    The bindings reach the in-memory implementation through
    `IOBase.from_bytes`, not through a `Buffer` class of their own.

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::MimeType;

let mut handle = Buffer::with_capacity(1_024);
handle.reserve(4_096)?;
assert!(handle.capacity() >= 4_096);
// Reserving changes the allocation, never the length.
assert_eq!(handle.size(), 0);

handle.pwrite(0, b"symbol,price\n")?;
assert_eq!(handle.as_slice(), b"symbol,price\n");

// A format the bytes cannot identify is declared rather than guessed.
let csv = Buffer::from_bytes(handle.into_bytes()).with_media_type(MimeType::CSV.into());
assert_eq!(csv.media_type().base(), &MimeType::CSV);
```

`Buffer` is the in-memory implementation, and the one every example and test reaches for. The
allocation doubles rather than growing exactly, so appending in many small writes stays amortized
constant; `reserve` pre-sizes it when the final length is known. `as_slice`, `as_mut_slice`, and
`into_bytes` reach the bytes directly, and taking a mutable slice discards any inferred media type,
because the content's identity may change through it.

A buffer is not stored anywhere, so `url` reports a synthetic `mem:` identity naming the process and
the allocation - enough to tell two live buffers apart in a log, without pretending the bytes live
somewhere. The other implementations in the core are [local.md](local.md), the memory-mapped local
tree, and [arrowfs.md](arrowfs.md), which puts any existing Arrow filesystem - S3, GCS, Azure, or
one you wrote - behind the same trait. Anything else is a sibling module supplying the same three
roles, never a change to this one. Two wrapping handles sit over any of them and are handles
themselves: `Coded` below, and the page cache in [buffered.md](buffered.md).

## Coded

!!! note "Rust only"
    The Python and JavaScript packages do not expose the compression
    wrappers.

```rust
use yggdryl::io::{Buffer, Coded, IOBase};
use yggdryl::{Codec, Level, MimeType, Url};

let inner = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows.gz")?.media_type());
let mut handle = Coded::new(inner, Codec::Gzip).with_level(Level::BEST);

// The wrapper's bytes are decoded, so its media type has the coding removed.
assert_eq!(handle.media_type().base(), &MimeType::ARROW_STREAM);
assert_eq!(handle.media_type().encoding_len(), 0);

let payload = "symbol,price\n".repeat(64).into_bytes();
handle.write_all_bytes(&payload)?;
handle.flush()?;

// Reads decompress; the wrapped handle only ever holds the encoded form.
assert_eq!(handle.read_all_bytes()?, payload);
assert!(handle.handle().size() < payload.len() as u64);
```

`Coded` wraps any handle and presents the decoded bytes: reads decompress, writes compress. It is an
`IOBase` itself, so it goes anywhere a handle goes. The per-format aliases in [gzip.md](gzip.md),
[zlib.md](zlib.md), and [zstd.md](zstd.md) are this type with the codec already chosen.

A content coding is not seekable, which forces two tradeoffs. The decoded value is materialized once
and held until `close`, so positional reads and writes work at all over a compressed payload; and a
write is published to the wrapped handle on `flush` or `close`, not on every `pwrite`. `into_handle`
publishes and returns the wrapped handle. `Codec::Identity` makes the wrapper a pass-through.

## Buffered

!!! note "Rust only"
    The Python and JavaScript packages do not expose the page cache.

```rust
use yggdryl::buffered::BufferedOptions;
use yggdryl::io::{Buffer, IOBase};

let handle = Buffer::from_bytes(vec![4_u8; 4_096]).buffered(BufferedOptions::default());

// The first read fetches the page holding the range; the second is memory.
assert_eq!(handle.read_range(0, 8)?, [4_u8; 8]);
assert_eq!(handle.read_range(2_000, 8)?, [4_u8; 8]);
assert_eq!(handle.cached_pages(), 1);
```

`Buffered` is the other wrapping handle: it serves reads from fixed-size pages under a byte budget
and a time to live, writes through, and pins the value's first and last pages so a footer-first
container never re-reads either end. Everything else it mirrors. [buffered.md](buffered.md) is the
page.

## Roles

!!! note "Rust only"
    The bindings expose one handle class rather than the three role traits
    behind it.

```rust
use yggdryl::io::IOBase;
use yggdryl::{IOKind, MimeType, local};

let path = std::env::temp_dir().join("yggdryl-docs-io-folder");
let _ = std::fs::remove_dir_all(&path);
let mut folder = local::Folder::new(&path)?;

// A container holds no bytes: reads are empty, byte writes are refused.
let mut probe = [0_u8; 4];
assert_eq!(folder.pread(0, &mut probe)?, 0);
assert_eq!(folder.size(), 0);
let refused = folder.pwrite(0, b"x").unwrap_err().to_string();
assert!(refused.contains("got the directory"), "{refused}");

// Truncating to zero is the write that brings a container into being.
folder.truncate(0)?;
assert!(folder.exists());
assert_eq!(folder.kind(), IOKind::Directory);
assert_eq!(folder.media_type().base(), &MimeType::DIRECTORY);
assert_eq!(folder.ls(false, false).count(), 0);

std::fs::remove_dir_all(&path)?;
```

`IOBase` says how to move bytes. The three role traits say what a resource *is*, and each declares only
what the backend alone knows, pre-implementing the rest as methods the backend's `IOBase` impl forwards
to:

- **`IOFolder`** - a container. Declares `folder_url`, `folder_exists`, `create_folder`, `list_folder`.
  Pre-implements `folder_pread` (reads nothing), `folder_pwrite` (refuses, naming the container),
  `folder_truncate` (creates on `0`, errors otherwise), `folder_media_type` (`inode/directory`), and
  `folder_kind` (`Directory`).
- **`IOFile`** - a leaf. Declares `file_url` and `file_exists`. Pre-implements `file_ls` (lists
  nothing), `file_child_by_path` (refuses, naming the file), and `file_kind` (`File` when it exists,
  `Unknown` when it does not).
- **`IOPath`** - a location whose role is not resolved yet. Declares `path_url`, `is_folder`,
  `is_file`. Pre-implements `path_exists`, `path_kind` (`Directory`, `File`, or `Unknown`), and
  `path_media_type` (the container type, or the one the name implies).

```rust
use yggdryl::io::IOBase;
use yggdryl::{IOKind, local};

// A location that arrived from outside answers by looking at what is there.
let existing = local::Path::new(std::env::temp_dir())?;
assert_eq!(existing.kind(), IOKind::Directory);

let undecided = local::Path::new(std::env::temp_dir().join("yggdryl-docs-io-undecided"))?;
assert_eq!(undecided.kind(), IOKind::Unknown);
assert!(undecided.read_all_bytes()?.is_empty());

// A leaf is not a container: it lists nothing and resolves no child.
let leaf = local::File::new(std::env::temp_dir().join("yggdryl-docs-io-leaf.arrows"))?;
assert_eq!(leaf.ls(true, false).count(), 0);
assert!(leaf.child_by_path("nested").is_err());
```

`parent`, `child_by_path`, and `ls` return [generic.md](generic.md)'s `Holder`, which is why a walk over a
tree needs no type parameter. A resource that cannot contain others lists nothing rather than failing,
so a caller can walk without testing each node first. `local::Path` is the reference `IOPath`: it
resolves by looking, and a byte write is what settles an undecided location into a file. A remote
store is the same three roles over a different transport.

## Delegating to a wrapped handle

!!! note "Rust only"
    A backend implements `IOBase` in Rust; neither binding can add one.

```rust
use yggdryl::io::{Buffer, IOBase};

/// A wrapper mirrors the handle's bytes rather than owning bytes of its own.
struct Counted {
    handle: Buffer,
    opens: usize,
}

impl IOBase for Counted {
    yggdryl::delegate_iobase!(handle);

    fn open(&mut self) -> yggdryl::Result<()> {
        self.opens += 1;
        self.handle.open()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut wrapper = Counted {
        handle: Buffer::new(),
        opens: 0,
    };
    wrapper.open()?;
    wrapper.write_all_bytes(b"AAPL")?;

    assert_eq!(wrapper.opens, 1);
    assert_eq!(wrapper.read_all_bytes()?, b"AAPL");
    assert_eq!(wrapper.handle.as_slice(), b"AAPL");
    Ok(())
}
```

`delegate_iobase!` expands to the forwarding bodies for every byte method - `pread`, `pwrite`, `size`,
`capacity`, `reserve`, `truncate`, `url`, `media_type`, `set_media_type`, `flush`, `parent`,
`child_by_path`, `ls`, `kind` - plus `clear` and `remove`, inside an `impl IOBase for` block. It
deliberately does not forward `open`, `opened`, or `close`: a wrapper that caches something of its
own writes those after the invocation, as the example does. [ipc.md](ipc.md),
[parquet.md](parquet.md), and the compression handles are all built this way.

It has two other spellings, for two other shapes:

- `delegate_iobase!(handle, except_lifecycle)` is the same list **without** `clear` and `remove`, for
  a wrapper that holds a cache: it writes that pair itself so the cache is invalidated as part of the
  call rather than left to answer a later read with bytes that are gone. `Ipc`, `Parquet`, and the
  [text handler](#text-records) are all this shape.
- `delegate_iobase!(handle: pread, size, ...)` names the methods to mirror one by one, for a wrapper
  that *changes* one of them - a method cannot be both expanded by the macro and written out
  underneath it. [`Buffered`](buffered.md) is this shape: it owns the two positional primitives, the
  resize that invalidates, the open/close pair, and the `clear`/`remove` pair, and mirrors the rest.
  A method left out of the list falls back to the trait's own default, which for `clear` and `remove`
  means truncating rather than reaching the resource - so leave them out only when writing them.

## Arrow batches

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::{DataType, Url};

    // A non-null struct Field is the schema.
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let arrow_schema = schema.to_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )?;

    // The handle's own media type picks the encoding; no format argument is passed.
    let mut handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
    let options = handle.record_options()?;

    // The write path takes a batch reader and nothing else.
    handle.write_arrow_batch_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
    assert_eq!(handle.read_arrow_field(&options)?, schema);

    // The read path returns one. Batches arrive one at a time, never as a vector.
    let mut rows = 0;
    for batch in handle.read_arrow_batch_reader(&options)? {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    # A PyArrow schema is the schema; the binding imports it once at the boundary.
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    batch = pa.record_batch({"id": [1, 2], "symbol": ["AAPL", None]}, schema=schema)

    # The handle's own media type picks the encoding; no format argument is passed.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    options = handle.record_options()

    # The write path takes a batch reader and nothing else.
    handle.write_arrow_batch_reader(batch, options=options)
    assert handle.read_arrow_field(options=options).name == "row"

    # The read path returns one. Batches arrive one at a time, never as a vector.
    rows = sum(part.num_rows for part in handle.read_arrow_batch_reader(options=options))
    assert rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, fields } = require('yggdryl')

    // A non-null struct Field is the schema.
    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      { nullable: false },
    )

    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
    })

    // The handle's own media type picks the encoding; no format argument is passed.
    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    const options = handle.recordOptions()

    // The write path takes a batch reader and nothing else.
    handle.writeArrowBatchReader(BatchReader.from(table), options)
    assert.ok(handle.readArrowField(options).equals(schema))

    // The read path returns one. Batches arrive one at a time, never as a vector.
    let rows = 0
    for (const batch of handle.readArrowBatchReader(options)) {
      rows += batch.numRows
    }
    assert.equal(rows, 2)
    ```

The record surface is exactly three methods: `read_arrow_batch_reader` returns [arrow.md](arrow.md)'s
`BatchReader`, `write_arrow_batch_reader` replaces or merges, and `append_arrow_batch_reader` adds
after what is there. Between them they are the only place an encoding is decoded and the only place
one is encoded, and every other record operation is expressed through them.
`arrow::batch_reader` is the constructor that turns batches a caller already has - a `Vec`, an array,
a lazily-computed iterator - into one. There is no row-level read or write anywhere: a batch is the
unit.

`record_options` derives the encoding's settings from the handle's media type, so the encoding is
never guessed - it is whatever the handle already says it holds. `read_arrow_field` returns the
canonical non-null struct root `Field` described in [field.md](field.md). All of this is behind the
default `arrow` feature.

Content coding stays the handle's business. A handle named `trades.arrows.zst` round-trips compressed
through the same methods, because the coding is in its media type and no call takes a coding
argument.

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::MimeType;

    // An absent resource holds no batches rather than failing to parse.
    let empty = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    assert_eq!(
        empty.read_arrow_batch_reader(&empty.record_options()?)?.count(),
        0
    );

    // An encoding this build does not implement is named rather than guessed.
    let csv = Buffer::new().with_media_type(MimeType::CSV.into());
    let message = csv.record_options().unwrap_err().to_string();
    assert!(message.contains("text/csv"), "{message}");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pytest

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())

    # An absent resource holds no batches rather than failing to parse.
    empty = IOBase(root / "absent.arrows")
    assert empty.read_arrow_batch_reader().read_all().num_rows == 0

    # An encoding this build does not implement is named rather than guessed.
    csv = IOBase(root / "trades.csv")
    with pytest.raises(ValueError, match="text/csv"):
        csv.record_options()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, MimeType } = require('yggdryl')

    // An absent resource holds no batches rather than failing to parse.
    const empty = IOBase.fromBytes()
    empty.mediaType = MimeType.ARROW_STREAM
    assert.equal([...empty.readArrowBatchReader()].length, 0)

    // An encoding this build does not implement is named rather than guessed.
    const csv = IOBase.fromBytes()
    csv.mediaType = MimeType.CSV
    assert.throws(() => csv.recordOptions(), /text\/csv/)
    ```

The encodings `record_options` can return are the ones this build carries: Arrow IPC always
([ipc.md](ipc.md)), Apache Parquet under the non-default `parquet` feature
([parquet.md](parquet.md)). The settings themselves - schema, root name, cast strictness, batch size,
compression level, match key - are shared across encodings through `IORecordOptions`, documented in
[generic.md](generic.md).

## Rows as record instances

!!! note "Python and JavaScript"
    Rust reads batches; the record-instance layer is what the bindings add on
    top of it.

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.records import record

    @record
    class Trade:
        id: int
        venue: str | None

    root = pathlib.Path(tempfile.mkdtemp())
    handle = IOBase(root / "trades.arrows")
    handle.write_records([Trade(1, "XNAS"), Trade(2, None)])

    # Each stored row becomes one instance, batch by batch - nothing is
    # collected - and rows cast flexibly onto the class: names reconcile,
    # widths convert, missing columns default.
    assert [t.id for t in handle.read_records(Trade)] == [1, 2]

    # Omit the class and one is built at runtime from the stored schema.
    assert [row.venue for row in handle.read_records()] == ["XNAS", None]

    # An absent resource yields no rows, and an empty iterable writes
    # nothing, so conditional pipelines need no existence checks.
    assert list(IOBase(root / "absent.arrows").read_records()) == []
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.arrows'))
    // Plain objects are rows; `writeRecords` is the generic write under
    // the record name.
    handle.writeRecords([
      { id: 1n, venue: 'XNAS' },
      { id: 2n, venue: null },
    ])

    // Plain objects out, streamed batch by batch ...
    assert.deepEqual([...handle.readRecords()].map((row) => row.id), [1n, 2n])

    // ... or instances of any class whose constructor takes the plain row.
    class Trade {
      constructor(row) {
        Object.assign(this, row)
      }
    }
    const trades = [...handle.readRecords(Trade)]
    assert.ok(trades.every((t) => t instanceof Trade))

    // An absent resource yields no records rather than raising.
    assert.deepEqual([...new IOBase(path.join(root, 'absent.arrows')).readRecords()], [])

    fs.rmSync(root, { recursive: true, force: true })
    ```

In Python, `read_records` hands back instances of a [record class](extensions/python.md) - the one you pass,
or one built at runtime from the resource's own schema when you pass none - and `write_records` /
`append_records` take any iterable of instances, inferring the class from the first row. The rows
stream batch by batch through the same native reader every other read uses, casting flexibly onto
the class's schema; `safe` and `errors` say what happens to a value that will not convert. In
JavaScript the same three names read rows as plain objects or through any constructor that takes
one, and the writes widen exactly as `writeArrow` does. In both languages an absent resource reads
as empty and an empty write is a no-op, so nothing needs an existence check first.

## Lazy scans

!!! note "Python only"

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

target = pathlib.Path(tempfile.mkdtemp()) / "trades.parquet"
IOBase(target).write_arrow(pa.table({"symbol": ["AAPL", "MSFT"], "price": [187.23, 402.11]}))

# A local Parquet leaf becomes the real lazy scan - projection and
# predicate pushdown belong to the engine, and the handle publishes its
# bytes at their exact length first so the foreign reader sees a whole file.
lazy = IOBase(target).scan_polars()
assert lazy.select("symbol").head(10).collect().height == 2

# The pyarrow spelling of the same idea, as a dataset Scanner.
scanner = IOBase(target).scan_arrow()
assert scanner.to_table().num_rows == 2

# Anything a foreign scanner cannot mmap - an in-memory buffer, a
# compressed name, an Arrow stream - streams through the native reader
# instead, so both calls answer for every holder.
memory = IOBase.from_bytes()
memory.media_type = "application/vnd.apache.arrow.stream"
memory.write_arrow(pa.table({"symbol": ["AAPL"]}))
assert memory.scan_arrow().to_table().num_rows == 1
```

`scan_polars` hands back a `polars.LazyFrame` and `scan_arrow` a `pyarrow.dataset.Scanner`. A
plain local Parquet resource is scanned natively; everything else reads through the same native
reader every other read uses and arrives as the lazy shape anyway, so callers never branch on
where the bytes live.

## Column pushdown

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::{DataType, MimeType};

    let stored = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Utf8.required_field("venue"),
    ])?
    .required_field("row");
    let arrow_schema = stored.to_arrow_schema()?;

    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
            Arc::new(StringArray::from(vec!["XNAS", "XNAS"])),
        ],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let plain = handle.record_options()?;
    handle.write_arrow_batch_reader(arrow::batch_reader(arrow_schema, [batch]), &plain)?;

    // One of the three columns, declared as this read's schema.
    let wanted = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let projected = handle.read_arrow_batch_reader(&plain.clone().with_schema(wanted))?;
    assert_eq!(projected.schema().fields().len(), 1);
    assert_eq!(projected.map(|batch| batch.unwrap().num_columns()).sum::<usize>(), 1);

    // The resource is unchanged: it still holds all three.
    assert_eq!(handle.read_arrow_field(&plain)?.field_len(), 3);

    // A column it does not hold cannot be projected out of it, so the encoding
    // reads everything and the cast supplies that column as nulls.
    let invented = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("nowhere"),
    ])?
    .required_field("row");
    let widened = handle.read_arrow_batch_reader(&plain.with_schema(invented))?;
    assert_eq!(widened.schema().fields().len(), 2);
    assert_eq!(widened.schema().field(1).name(), "nowhere");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    stored = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("venue", pa.string(), nullable=False),
    ])
    batch = pa.record_batch(
        {"id": [1, 2], "symbol": ["AAPL", "MSFT"], "venue": ["XNAS", "XNAS"]},
        schema=stored,
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.write_arrow_batch_reader(batch)

    # One of the three columns, declared as this read's schema.
    options = handle.record_options()
    options.schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    projected = handle.read_arrow_batch_reader(options=options)
    assert projected.schema.names == ["id"]
    assert projected.read_all().num_columns == 1

    # The resource is unchanged: it still holds all three.
    assert len(handle.read_arrow_field().data_type) == 3

    # A column it does not hold cannot be projected out of it, so the encoding
    # reads everything and the cast supplies that column as nulls.
    options.schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("nowhere", pa.string()),
    ])
    widened = handle.read_arrow_batch_reader(options=options)
    assert widened.schema.names == ["id", "nowhere"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, fields } = require('yggdryl')

    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
      venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
    })

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.writeArrowBatchReader(BatchReader.from(table))

    // One of the three columns, declared as this read's schema.
    const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const options = handle.recordOptions()

    const projected = handle.readArrowBatchReader(options.withSchema(wanted))
    assert.equal(projected.field.dataType.length, 1)
    assert.equal(projected.toTable().numCols, 1)

    // The resource is unchanged: it still holds all three.
    assert.equal(handle.readArrowField().dataType.length, 3)

    // A column it does not hold cannot be projected out of it, so the encoding
    // reads everything and the cast supplies that column as nulls.
    const invented = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('nowhere: utf8?')],
      { nullable: false },
    )
    const widened = handle.readArrowBatchReader(options.withSchema(invented))
    assert.equal(widened.field.dataType.length, 2)
    ```

The schema on the options selects *and* casts, in one pass over the data. The columns it names that
the resource stores are handed to the encoding as its own projection - a Parquet projection mask, an
Arrow IPC projection - so the columns it leaves out are skipped rather than read and discarded.
[parquet.md](parquet.md) is where that also means fewer bytes decoded, because a column chunk is
separately addressable; [ipc.md](ipc.md) saves the decode and the allocation but still reads the
message body, and says so rather than claiming otherwise.

A projection can only drop columns, so the cast does everything else: reordering to the declared
order, converting a stored type into the declared one, and filling a column the resource does not
hold. Each batch is cast as it is pulled, so nothing is collected to do it. With no declared schema
the stored shape is preserved exactly and no cast runs at all.

`read_arrow_field` answers with the same shape this read produces, so the schema a caller reads and
the batches a caller gets can never disagree.

## Appending and merging

A write with no match key replaces the resource, which is what an IPC stream or
a Parquet file natively supports: each carries one schema and one footer. A
match key turns the same call into a merge, and appending is the third method.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::{DataType, Url};

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");
    let arrow_schema = schema.to_arrow_schema()?;
    let rows = |ids: Vec<i64>, symbols: Vec<&'static str>| {
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(symbols)),
            ],
        )
        .expect("a batch matching the root");
        arrow::batch_reader(batch.schema(), [batch])
    };

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
    let options = handle.record_options()?.with_schema(schema.clone());

    // No match key: the resource is replaced.
    handle.write_arrow_batch_reader(rows(vec![1, 2], vec!["AAPL", "MSFT"]), &options)?;

    // Appending reads what is there, chains the new batches after it, and rewrites.
    handle.append_arrow_batch_reader(rows(vec![3], vec!["NVDA"]), &options)?;
    let total: usize = handle
        .read_arrow_batch_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 3);

    // A match key merges: `2` is already stored and updates, `9` is new and appends.
    let merging = options.clone().with_merge_by_names(["id"]);
    handle.write_arrow_batch_reader(rows(vec![2, 9], vec!["MSFT.O", "AMD"]), &merging)?;
    let total: usize = handle
        .read_arrow_batch_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 4);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    rows = lambda ids, symbols: pa.record_batch(
        {"id": ids, "symbol": symbols}, schema=schema
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    options = handle.record_options()
    options.schema = schema

    # No match key: the resource is replaced.
    handle.write_arrow_batch_reader(rows([1, 2], ["AAPL", "MSFT"]), options=options)

    # Appending reads what is there, chains the new batches after it, and rewrites.
    handle.append_arrow_batch_reader(rows([3], ["NVDA"]), options=options)
    assert handle.read_arrow_batch_reader(options=options).read_all().num_rows == 3

    # A match key merges: `2` is already stored and updates, `9` is new and appends.
    merging = handle.record_options()
    merging.schema = schema
    merging.merge_by_names = ["id"]
    handle.write_arrow_batch_reader(rows([2, 9], ["MSFT.O", "AMD"]), options=merging)
    assert handle.read_arrow_batch_reader(options=options).read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, fields } = require('yggdryl')

    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8?')],
      { nullable: false },
    )
    const rows = (ids, symbols) =>
      BatchReader.from(
        new arrow.Table({
          id: arrow.vectorFromArray(ids, new arrow.Int64()),
          symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
        }),
      )

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    const options = handle.recordOptions().withSchema(schema)

    // No match key: the resource is replaced.
    handle.writeArrowBatchReader(rows([1n, 2n], ['AAPL', 'MSFT']), options)

    // Appending reads what is there, chains the new batches after it, and rewrites.
    handle.appendArrowBatchReader(rows([3n], ['NVDA']), options)
    assert.equal(handle.readArrowBatchReader(options).toTable().numRows, 3)

    // A match key merges: `2` is already stored and updates, `9` is new and appends.
    const merging = options.withMergeByNames(['id'])
    handle.writeArrowBatchReader(rows([2n, 9n], ['MSFT.O', 'AMD']), merging)
    assert.equal(handle.readArrowBatchReader(options).toTable().numRows, 4)
    ```

`merge_by_names` is a shared record setting, so it means the same thing on every
encoding. Empty is an overwrite: a declared schema is applied to the incoming
rows, and the result is then cast to the schema the resource already stores when
it stores one - an overwrite replaces rows, not columns, so a caller who really
means to change a stored schema clears the handle first.

Non-empty names the columns that decide whether two rows are the same row. The
key is encoded through Arrow's own row format, so a null key matches another
null key exactly and a composite key compares column by column. A key stored more
than once has every occurrence updated, because a match key is a rule and not a
constraint the stored side was ever checked against; a key arriving more than
once lets the last arrival win.

The merge is streamed over the incoming side: one batch is pulled, matched,
folded in, and dropped before the next is pulled. What has to be held is the
stored side, because updating a row means finding it by key and a reader cannot
be rewound to a row it has already yielded.

Appending streams on both sides - the stored batches are chained ahead of the
incoming ones and encoded as they arrive - and casts the incoming batches to the
target shape first, so data whose schema merely *fits* is accepted.

All three are transactional in the sense that matters: nothing is written until
the new contents are complete, so a failure leaves the resource as it was.

`select_by_names` is the companion narrowing setting, and it works on both
directions. On a read it yields exactly the named columns of the stored rows,
in the order the names are given; on a write - overwrite, merge, or append - it
keeps exactly the named columns of the incoming rows, so what it drops can
never land. Names match ASCII case-insensitively, the way every cast selects,
and a name the rows do not have is an error listing what is there, because a
selection is a claim about the rows rather than a wish. An empty list, the
default, selects everything.

=== "Rust"

    ```rust
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::{arrow, DataType, MimeType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");
    let batch = RecordBatch::try_new(
        arrow::schema_from_field(&schema)?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), Some("MSFT")])),
        ],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?;
    handle.write_arrow_batch_reader(arrow::batch_reader(batch.schema(), [batch]), &options)?;

    // A read narrowed to one column yields one column.
    let selecting = options.with_select_by_names(["symbol"]);
    let first = handle.read_arrow_batch_reader(&selecting)?.next().unwrap()?;
    assert_eq!(first.num_columns(), 1);
    assert_eq!(first.schema().field(0).name(), "symbol");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "orders.arrows")
    handle.write_arrow(pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}))

    # A single setting is its own keyword; every record method takes each
    # options field directly, and `options=` remains for reuse across calls.
    narrowed = handle.read_arrow(select_by_names=["symbol"]).read_all()
    assert narrowed.column_names == ["symbol"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'orders.arrows'))
    handle.writeArrowBatchReader(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
      }),
    )

    const narrowed = handle.recordOptions().withSelectByNames(['symbol'])
    const table = handle.readArrowBatchReader(narrowed).toTable()
    assert.deepEqual(table.schema.fields.map((field) => field.name), ['symbol'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Globbing and Hive partitions

A location can name a set rather than one resource, and a Hive path can name the values its rows
share. `IOBase` reads both, so selecting the parts of a lake to rewrite is a listing, not a scan.

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;

    let root = std::env::temp_dir().join("yggdryl-doc-lake");
    let _ = std::fs::remove_dir_all(&root);
    for year in ["2024", "2025"] {
        let leaf = root.join(format!("year={year}")).join("month=01");
        std::fs::create_dir_all(&leaf)?;
        std::fs::write(leaf.join("part-0.parquet"), b"parquet")?;
    }

    let lake = Folder::new(&root)?;

    // A fixed prefix is descended, not listed and filtered.
    assert_eq!(lake.glob("year=2024/**/*.parquet", false)?.count(), 1);
    assert_eq!(lake.glob("**/*.parquet", false)?.count(), 2);

    // Partition filters select the leaves to overwrite or upsert.
    let selected: Vec<_> = lake
        .children_where(&[("year", "2024")], false)?
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].partitions(), vec![
        ("year".to_owned(), "2024".to_owned()),
        ("month".to_owned(), "01".to_owned()),
    ]);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"
    for year in ("2024", "2025"):
        leaf = root / f"year={year}" / "month=01"
        leaf.mkdir(parents=True)
        (leaf / "part-0.parquet").write_bytes(b"parquet")

    lake = IOBase(root)

    assert len(list(lake.glob("year=2024/**/*.parquet"))) == 1
    assert len(list(lake.rglob("*.parquet"))) == 2

    selected = list(lake.children_where({"year": "2024"}))
    assert len(selected) == 1
    assert selected[0].partitions == (("year", "2024"), ("month", "01"))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'lake')
    for (const year of ['2024', '2025']) {
      const leaf = path.join(root, `year=${year}`, 'month=01')
      fs.mkdirSync(leaf, { recursive: true })
      fs.writeFileSync(path.join(leaf, 'part-0.parquet'), 'parquet')
    }

    const lake = new IOBase(root)

    // A fixed prefix is descended, not listed and filtered.
    assert.equal([...lake.glob('year=2024/**/*.parquet')].length, 1)
    assert.equal([...lake.rglob('*.parquet')].length, 2)

    // Partition filters select the leaves to overwrite or upsert.
    const selected = [...lake.childrenWhere({ year: '2024' })]
    assert.equal(selected.length, 1)
    assert.deepEqual(selected[0].partitions, [
      { column: 'year', value: '2024' },
      { column: 'month', value: '01' },
    ])

    fs.rmSync(root, { recursive: true, force: true })
    ```

A location that *is* a pattern is folder-like before anything touches the backend: `kind` reports
`IOKind::Directory`, and `ls` expands the pattern from the fixed root it was split from instead of
looking for a directory literally named `**`.

`children_where` yields the leaves - never containers - that carry every requested pair. That is what
the three record methods use when a handle addresses a folder, and it is there for a caller who wants
to reach one partition directly instead.

The pairs are sugar. Each one builds `&holder.partition['column'] = 'value'` and the whole thing is
answered by `children_matching`, which takes the [expression](expression.md) language itself: ranges,
null tests, `in` lists, and every other `&holder.*` attribute. There is no second filter behind the
pairs.

## A listing is an iterator

Every listing here yields one entry at a time: `ls`, `glob`, `rglob`, `children_matching`, and
`children_where` all hand back a `Listing`, and none of them decides to build a vector on the
caller's behalf. That is not a style preference. A folder with a million leaves has to be listable
the same way one with three is, and a shape that has to materialize cannot describe a resource
larger than memory - the same argument the three record methods already make about batches.

Three consequences a caller can rely on:

- **Nothing is touched until the first `next`.** Building a listing costs nothing, and taking three
  entries from a folder of a hundred thousand costs three. A glob whose fixed prefix names nothing
  reads no directory beneath it at all.
- **The item is a `Result`, and the iterator is fused after the first one that fails.** A listing
  fails *at* the failing entry, naming it, without discarding what it already yielded and without
  spinning against a backend that is already refusing. A caller who wants a vector writes
  `.collect::<Result<Vec<_>>>()`.
- **Order is deterministic.** One directory's entries are sorted, and a recursive walk yields each
  container immediately before the subtree beneath it. The same listing over the same state yields
  the same sequence twice.

What a recursive walk holds is its *frontier* - one level's cursor per open depth - never its
result. The local backend reads and sorts one directory at a time; an Arrow filesystem answers a
whole prefix in one call, because that is the shape an object store already has, and the laziness
lives inside that one answer.

A few returns in listing positions stay owned, and each says what bounds it: `IOBase::partitions` is
bounded by one URL's path depth, and a report of what an operation just did - the snapshot ids an
expiry removed, the counts a compaction reports - is bounded by the operation. Unbounded by the
resource means an iterator; bounded by the act, or already in hand, may be owned.

In Python the listings are ordinary iterators, so `iterdir`, `glob`, and `rglob` behave exactly as
`pathlib`'s do - wrap one in `list()` when you want a sequence, and remember that `len()` of that
list costs the whole walk. In JavaScript they are iterables: `for...of` walks one, and `[...listing]`
drains it.

## Partition pruning and filtering

One option answers the same equality wherever the value lives.
`filter_partitions` names `(column, value)` pairs, spelled the way partition
paths spell them: a folder read *prunes* - a leaf whose directory names a
different value is never listed or decoded - and a column the data carries is
*filtered* row by row, so a path-partitioned lake and a data-partitioned one
answer identically.

Both halves are one [expression](expression.md), bound once. The pruning half asks the path
(`&holder.partition['year'] = '2024'`) and the filtering half asks the rows (`year = 2024`), with the
text read through the column's own datatype - so a pair on an `int32` column is an integer
comparison, and the pair `("price", "null")` is `price is null` rather than a comparison against four
letters.

=== "Rust"

    ```rust
    use yggdryl::generic::{IORecordOptions, RecordOptions};
    use yggdryl::MimeType;

    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?
        .with_filter_partitions([("year", "2024"), ("month", "01")]);
    // handle.read_arrow_batch_reader(&options)? now reads only the January
    // 2024 leaves, and only their matching rows.
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"
    IOBase(root / "year=2024" / "month=01" / "trades.arrows").write_arrow(
        pa.table({"id": [1, 2]})
    )
    IOBase(root / "year=2024" / "month=02" / "trades.arrows").write_arrow(
        pa.table({"id": [3]})
    )

    lake = IOBase(root)
    options = lake.record_options()
    options.filter_partitions = [("year", "2024"), ("month", "01")]
    reader = lake.read_arrow_batch_reader(options=options)
    assert reader.read_all().num_rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const lake = path.join(root, 'lake')
    new IOBase(path.join(lake, 'year=2024', 'month=01', 'trades.arrows'))
      .writeRecords([{ id: 1n }, { id: 2n }])
    new IOBase(path.join(lake, 'year=2024', 'month=02', 'trades.arrows'))
      .writeRecords([{ id: 3n }])

    const handle = new IOBase(lake)
    const options = handle.recordOptions().withFilterPartitions([
      ['year', '2024'],
      ['month', '01'],
    ])
    assert.equal(handle.readArrowBatchReader(options).toTable().numRows, 2)

    fs.rmSync(root, { recursive: true, force: true })
    ```

Writes into a shared folder also smooth concurrent writers: the listing and
every whole-leaf rewrite retry a bounded number of times with a short growing
pause, so a reader that catches a leaf half-published or two writers racing a
replace settle without surfacing a transient error. An append never retries -
replaying a torn append would duplicate rows - so it fails honestly instead.

## Partition columns in the data

A Hive layout stores a column in the path, so the file under `year=2024/month=01` leaves those values
out of every row. Addressing the folder rather than the file is what puts them back: the three record
methods resolve the leaves themselves, restore the columns their directory names spell out, and route
each row of a write to the leaf its values name.

=== "Rust"

    ```rust
    use yggdryl::generic::{Holder, IORecordOptions, RecordOptions};
    use yggdryl::io::IOBase;
    use yggdryl::{DataType, MimeType};

    let root = std::env::temp_dir().join("yggdryl-doc-partitioned");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("year=2024").join("month=01"))?;

    let schema = DataType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::Int32.required_field("year"),
        DataType::Utf8.required_field("month"),
    ])?
    .required_field("row");
    let arrow_schema = schema.to_arrow_schema()?;
    let batch = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![10, 20])),
            std::sync::Arc::new(arrow_array::Int32Array::from(vec![2024, 2024])),
            std::sync::Arc::new(arrow_array::StringArray::from(vec!["01", "01"])),
        ],
    )?;

    // The rows carry every column; the write drops the two the path spells out.
    let mut lake = Holder::folder(&root)?;
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_schema(schema.clone());
    lake.write_arrow_batch_reader(
        yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        &options,
    )?;

    // Only `price` reached the leaf; the other two are the directory names.
    let leaf = lake.child_by_path("year=2024/month=01/part-0.arrows")?;
    assert_eq!(
        leaf.read_arrow_field(&RecordOptions::for_media_type(leaf.media_type())?)?.field_len(),
        1
    );

    // Reading the folder restores them with their declared types.
    let restored = lake
        .read_arrow_batch_reader(&options)?
        .next()
        .expect("one batch")?;
    assert_eq!(restored.num_columns(), 3);
    assert_eq!(restored.schema().field(1).data_type(), &arrow_schema::DataType::Int32);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase, RecordOptions

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "year=2024" / "month=01").mkdir(parents=True)

    schema = pa.schema([
        pa.field("price", pa.int64(), nullable=False),
        pa.field("year", pa.int32(), nullable=False),
        pa.field("month", pa.string(), nullable=False),
    ])
    batch = pa.record_batch(
        {"price": [10, 20], "year": [2024, 2024], "month": ["01", "01"]},
        schema=schema,
    )

    # The rows carry every column; the write drops the two the path spells out.
    lake = IOBase(root)
    options = RecordOptions("part.arrows")
    options.schema = schema
    lake.write_arrow_batch_reader(batch, options=options)

    # Only `price` reached the leaf; the other two are the directory names.
    leaf = lake / "year=2024" / "month=01" / "part-0.arrows"
    assert len(leaf.read_arrow_field().data_type) == 1

    # Reading the folder restores them with their declared types.
    restored = lake.read_arrow_batch_reader(options=options).read_all()
    assert restored.column_names == ["price", "year", "month"]
    assert restored.schema.field("year").type == pa.int32()

    shutil.rmtree(root)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, RecordOptions, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    fs.mkdirSync(path.join(root, 'year=2024', 'month=01'), { recursive: true })

    const schema = fields.struct(
      'row',
      [Field.from('price: int64'), Field.from('year: int32'), Field.from('month: utf8')],
      { nullable: false },
    )
    const table = new arrow.Table({
      price: arrow.vectorFromArray([10n, 20n], new arrow.Int64()),
      year: arrow.vectorFromArray([2024, 2024], new arrow.Int32()),
      month: arrow.vectorFromArray(['01', '01'], new arrow.Utf8()),
    })

    // The rows carry every column; the write drops the two the path spells out.
    const lake = new IOBase(root)
    const options = RecordOptions.forMimeType(MimeType.ARROW_STREAM).withSchema(schema)
    lake.writeArrowBatchReader(BatchReader.from(table), options)

    // Only `price` reached the leaf; the other two are the directory names.
    const leaf = lake.joinpath('year=2024').joinpath('month=01').joinpath('part-0.arrows')
    assert.equal(leaf.readArrowField().dataType.length, 1)

    // Reading the folder restores them with their declared types.
    const restored = lake.readArrowBatchReader(options).toTable()
    assert.equal(restored.numCols, 3)
    assert.equal(restored.schema.fields[1].type.toString(), 'Int32')
    assert.deepEqual(restored.getChild('month').toArray(), ['01', '01'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

The layout is the authority on which columns are partition columns, because nothing in a batch says
which of its columns belong in a path. A folder whose leaves already spell out `column=value`
partitions by exactly those columns; a folder that spells out nothing takes the layout from the
declared schema, whose [partition-marked fields](field.md#a-field-can-be-a-partition-column) say it;
and a folder with neither is one table in one leaf, named after the encoding. So a tree comes into
being two ways: address one partition directly to create it, or declare the columns on the schema
and let the first write lay the directories out.

=== "Rust"

    ```rust
    use yggdryl::generic::{Holder, IORecordOptions, RecordOptions};
    use yggdryl::io::IOBase;
    use yggdryl::{DataType, MimeType};

    let root = std::env::temp_dir().join("yggdryl-doc-declared-layout");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;

    // Nothing is on disk, so nothing spells a layout. The schema does.
    let schema = DataType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::Int32.required_field("year"),
    ])?
    .required_field("row")
    .with_partition_fields(&["year"])?;
    assert_eq!(schema.partition_field_names().collect::<Vec<_>>(), ["year"]);

    let arrow_schema = schema.to_arrow_schema()?;
    let batch = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![10, 20])),
            std::sync::Arc::new(arrow_array::Int32Array::from(vec![2024, 2024])),
        ],
    )?;

    let mut lake = Holder::folder(&root)?;
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_schema(schema);
    lake.write_arrow_batch_reader(
        yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        &options,
    )?;

    // The directory came from the declaration, and the leaf stores what the
    // path does not carry.
    assert!(root.join("year=2024").is_dir());

    // Reading it back reports the layout without being told it.
    let derived = lake.read_arrow_field(
        &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?,
    )?;
    assert_eq!(derived.partition_field_names().collect::<Vec<_>>(), ["year"]);

    let _ = std::fs::remove_dir_all(&root);
    ```

A declaration that contradicts a stored layout is refused, naming both, because one write cannot mean
two trees: a folder already partitioned by `year` and a schema that marks `venue` disagree about
which columns the leaves are missing, and merging them would leave files whose directory names no
longer say what they left out.

A column the data already carries is left alone rather than rewritten from the directory name, so a
mismatch between the two stays visible instead of being silently papered over. Without a declared
schema the restored values stay text, which is exactly what a directory name holds. A value that is
absent is spelled `null`, which a path cannot distinguish from the four letters - so it is the
declared column that decides, and a nullable one reads the text back as a null.

Routing is bounded over the incoming reader: one batch is pulled, split by partition, and written
before the next is pulled. The price is paid on the other side, because these encodings rewrite a
whole leaf: a partition touched by five batches is rewritten five times. The first batch to reach a
leaf performs the caller's operation and the rest append to it, which is what keeps an overwrite an
overwrite without buffering the whole write first.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/io-rust.ipynb){ download },
[Python](notebooks/io-python.ipynb){ download },
[JavaScript](notebooks/io-javascript.ipynb){ download }.

<!-- /notebooks -->
