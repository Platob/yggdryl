# Buffer

`Buffer` is the in-memory handle every example and test reaches for.

## Contract

| key | value |
| --- | --- |
| Owns | `yggdryl::holder::Buffer` |
| Bindings | Rust, and Python as `yggdryl.holder.Buffer`, the class `IOBase.from_bytes` answers with; JavaScript has one `IOBase` and reaches it through `fromBytes` |
| Construction | `Buffer::with_capacity`, `Buffer::from_bytes` |
| Direct bytes | `as_slice`, `as_mut_slice`, `into_bytes` |
| Growth | the allocation doubles, so many small appends stay amortized constant; `reserve` pre-sizes a known final length |
| Media type | declared with `with_media_type`, never guessed |
| Siblings | [Local](local.md), [Filesystems](filesystems.md) |
| Wrappers | [Buffered](buffered.md), `Coded` in [Coding](../../coding/index.md) |

## Use

Capacity, direct slices, and a declared media type are Rust only.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
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

## In Python

`IOBase.from_bytes` answers a `Buffer`. The class names the implementation and adds nothing to [`IOBase`](../iobase/bytes.md).

```python
from yggdryl import IOBase
from yggdryl.holder import Buffer

handle = IOBase.from_bytes(b"symbol,price\n")
assert type(handle) is Buffer
assert handle.read_bytes() == b"symbol,price\n"

# Raw bytes declare no format, so the media type stays generic.
assert str(handle.media_type) == "application/octet-stream"
assert handle.url.scheme == "mem"
```

## Edges

- `as_mut_slice()` -> discards any inferred media type; the content's identity may change through it.
- `url()` -> a synthetic `mem:` identity naming the process and the allocation; the bytes are stored nowhere.
- `Buffer(...)` in Python -> `TypeError`; the class has no constructor, and `IOBase.from_bytes` is how one is built.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::conformance
    cargo bench --bench holder --features parquet -- 'fs_bytes/.*/buffer'
    cargo bench --bench holder --features parquet -- 'io_buffered/.*/buffer'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py -k memory_handle
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern "memory handle" "node/tests/holder/io.test.js"
    npm run --prefix node bench:holder:io
    ```
