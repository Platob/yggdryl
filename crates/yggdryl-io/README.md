# yggdryl-io

Dependency-free **IO foundations** for the
[**yggdryl**](https://github.com/Platob/yggdryl) project: an abstract contract
for moving typed values across a byte boundary.

- `ReadBytes` / `WriteBytes` — the byte source and sink primitives, with `&[u8]`
  and `Vec<u8>` as the built-in in-memory ends.
- `Io<T>` — the typed contract layered on top: encode a `T` to bytes
  (`write` / `to_bytes`) and back (`read` / `from_bytes`), or read a whole
  sequence of `T`s as a `stream`.
- `Frames` — the reference `Io` implementation: length-delimited byte frames,
  enough to round-trip and stream values out of the box.

```rust
use yggdryl_io::{Frames, Io};

// Encode two frames into a byte sink, then stream them back out.
let mut sink: Vec<u8> = Vec::new();
Frames.write(&mut sink, &b"hello".to_vec()).unwrap();
Frames.write(&mut sink, &b"world".to_vec()).unwrap();

let items: Vec<Vec<u8>> = Frames.stream(&sink[..]).collect::<Result<_, _>>().unwrap();
assert_eq!(items, vec![b"hello".to_vec(), b"world".to_vec()]);
```

An optional `log` feature (off by default) traces read/write/stream calls; with
it disabled the crate stays dependency-free.
