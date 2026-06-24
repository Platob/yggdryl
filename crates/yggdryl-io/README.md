# yggdryl-io

Dependency-free **IO foundations** for the
[**yggdryl**](https://github.com/Platob/yggdryl) project: an abstract contract
for moving typed values across a byte boundary.

- `ReadBytes` / `WriteBytes` — the byte source and sink primitives, with `&[u8]`
  and `Vec<u8>` as the built-in in-memory ends.
- `Io<T>` — the typed contract layered on top: encode a `T` to bytes (`write`)
  and back (`read`), or read a whole sequence of `T`s as a `stream`.
- `Frames` — the reference `Io` implementation: length-delimited byte frames,
  enough to round-trip and stream values out of the box.
- `BytesIO` — a simple in-memory byte buffer with a cursor, modelled on Python's
  `io.BytesIO`: `read` / `write` / `seek` / `tell` / `getvalue` / `truncate`,
  and a `stream` flag that toggles whether the cursor advances.

```rust
use yggdryl_io::{Frames, Io};

// Encode two frames into a byte sink, then stream them back out.
let mut sink: Vec<u8> = Vec::new();
Frames.write(&mut sink, &b"hello".to_vec()).unwrap();
Frames.write(&mut sink, &b"world".to_vec()).unwrap();

let items: Vec<Vec<u8>> = Frames.stream(&sink[..]).collect::<Result<_, _>>().unwrap();
assert_eq!(items, vec![b"hello".to_vec(), b"world".to_vec()]);
```

`BytesIO` is both a `ReadBytes` and a `WriteBytes`, so it drives any `Io` codec:

```rust
use yggdryl_io::{BytesIO, Whence};

let mut io = BytesIO::from_bytes(b"hello world".to_vec());
assert_eq!(io.read(Some(5)), b"hello");   // cursor now at 5
io.seek(6, Whence::Start).unwrap();
assert_eq!(io.read(None), b"world");
```

An optional `log` feature (off by default) traces read/write/stream calls; with
it disabled the crate stays dependency-free.
