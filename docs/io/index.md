# The io root

Everything in yggdryl lives under one **`io` layer**. Its root holds the cross-cutting value
types every module shares; below it, [`memory`](memory.md) owns byte access and
[`uri`](uri.md) owns addressing. The same folder tree is mirrored across the core, the tests,
the benchmarks, and both extensions.

| Type | What it is |
|---|---|
| `Serializable` | the root byte-codec trait — `serialize_bytes()` / `deserialize_bytes()` are exact inverses; every value type implements it when possible |
| `Headers` | the project's **one** metadata map — ordered, ASCII-case-insensitive, multi-value, byte-capable; HTTP headers, schema metadata, and source annotations all live here |
| `IOMode` | how a source may be accessed — an int enum: `Read = 1`, `Write = 2`, `ReadWrite = 3`, `Append = 4`, `Overwrite = 5` |
| `IOKind` | what a source is — an int enum: `Missing = 0`, `File = 1`, `Directory = 2`, `Heap = 3` |
| `IoError` / `Whence` | the guided error family and the seek anchor (`Start` / `Current` / `End`) |

## Headers — the one metadata map

`Headers` follows HTTP conventions: insertion-ordered, name-matching is ASCII-case-insensitive,
a name may repeat (multi-value), and both names and values may be raw bytes with `&str`
conveniences on top. `insert` replaces, `append` keeps. It round-trips through an HTTP text
form (`parse_http` / `to_http_bytes`) and a byte codec (`serialize_bytes` /
`deserialize_bytes`) that preserves arbitrary bytes, order, and duplicates. In Python it is a
mutable mapping (dict protocol, unhashable like `dict`); in Node the same capability is named
methods.

=== "Python"

    ```python
    from yggdryl.io import Headers

    h = Headers()
    h.insert("Content-Type", "application/json")
    h.append("Set-Cookie", "a=1")
    h.append("Set-Cookie", "b=2")

    assert h.get("content-type") == "application/json"   # case-insensitive
    assert h.get_all("set-cookie") == ["a=1", "b=2"]      # multi-value
    assert "Content-Type" in h and len(h) == 3            # dict protocol
    h["X-Trace"] = "abc"                                  # insert via dunder
    del h["Set-Cookie"]                                   # removes every occurrence

    round = Headers.deserialize_bytes(h.serialize_bytes())
    assert round == h
    ```

=== "Node"

    ```js
    const { Headers } = require('yggdryl').io

    const h = new Headers()
    h.insert('Content-Type', 'application/json')
    h.append('Set-Cookie', 'a=1')
    h.append('Set-Cookie', 'b=2')

    console.assert(h.get('content-type') === 'application/json')  // case-insensitive
    console.assert(h.getAll('set-cookie').length === 2)           // multi-value
    console.assert(h.contains('Content-Type') && h.len() === 3)

    const round = Headers.deserializeBytes(h.serializeBytes())
    console.assert(round.equals(h))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::Headers;

    let mut h = Headers::new();
    h.insert(Headers::CONTENT_TYPE, "application/json");
    h.append("Set-Cookie", "a=1");
    h.append("Set-Cookie", "b=2");

    assert_eq!(h.get("content-type"), Some("application/json")); // case-insensitive
    assert_eq!(h.get_all("set-cookie"), vec!["a=1", "b=2"]);      // multi-value
    assert_eq!(Headers::deserialize_bytes(&h.serialize_bytes()).unwrap(), h);
    ```

Every byte source carries one — see [`headers()` on the memory contract](memory.md#metadata-mode-and-kind).

## IOMode and IOKind — int enums with parsers

Both are wire-stable int enums with explicit parsers. The core names the exact input type
(`parse_str`, `from_u8`); the bindings add one generic, type-inferring `parse` that dispatches
on the runtime type.

=== "Python"

    ```python
    from yggdryl.io import IOMode, IOKind

    assert IOMode.Read == 1 and IOMode.Overwrite == 5     # int enum
    assert IOMode.parse("rw") == IOMode.ReadWrite         # str -> parse_str
    assert IOMode.parse(4) == IOMode.Append               # int -> from_u8
    assert IOMode.Append.is_writable() and not IOMode.Read.is_writable()

    assert IOKind.parse("dir") == IOKind.Directory
    assert not IOKind.Missing.exists()
    ```

=== "Node"

    ```js
    const { IOMode, IOKind, parseIoMode, parseIoKind, ioModeIsWritable, ioKindExists } =
      require('yggdryl').io

    console.assert(IOMode.Read === 1 && IOMode.Overwrite === 5)  // int enum
    console.assert(parseIoMode('rw') === IOMode.ReadWrite)       // string form
    console.assert(parseIoMode(4) === IOMode.Append)             // numeric form
    console.assert(ioModeIsWritable(IOMode.Append))

    console.assert(parseIoKind('dir') === IOKind.Directory)
    console.assert(!ioKindExists(IOKind.Missing))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::{IOKind, IOMode};

    assert_eq!(IOMode::Read.to_u8(), 1);
    assert_eq!(IOMode::parse_str("rw").unwrap(), IOMode::ReadWrite);
    assert_eq!(IOMode::from_u8(4).unwrap(), IOMode::Append);
    assert!(IOMode::Append.is_writable());

    assert_eq!(IOKind::parse_str("dir").unwrap(), IOKind::Directory);
    assert!(!IOKind::Missing.exists());
    ```

An unknown name or value is a guided error naming every accepted token.

## Serializable — one byte codec everywhere

Whenever a public type carries a value, it implements `Serializable`: `serialize_bytes()`
renders the canonical byte form in one pre-sized allocation and `deserialize_bytes()` is the
exact inverse. Equality, hashing, pickling (Python), and `serializeBytes`/`deserializeBytes`
(Node) all agree with it — one identity across every language.

=== "Python"

    ```python
    import pickle
    from yggdryl.memory import Heap
    from yggdryl.uri import Uri

    uri = Uri.parse("sc://h/p?q=1")
    assert Uri.deserialize_bytes(uri.serialize_bytes()) == uri
    assert pickle.loads(pickle.dumps(uri)) == uri     # pickle rides the same codec

    heap = Heap(b"payload")
    assert Heap.deserialize_bytes(heap.serialize_bytes()) == heap
    ```

=== "Node"

    ```js
    const { Uri } = require('yggdryl').uri
    const { Heap } = require('yggdryl').memory

    const uri = Uri.parse('sc://h/p?q=1')
    console.assert(Uri.deserializeBytes(uri.serializeBytes()).equals(uri))

    const heap = new Heap(Buffer.from('payload'))
    console.assert(Heap.deserializeBytes(heap.serializeBytes()).equals(heap))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::Serializable;
    use yggdryl_core::io::uri::Uri;

    fn roundtrip<T: Serializable>(value: &T) -> Result<T, T::Error> {
        T::deserialize_bytes(&value.serialize_bytes())
    }
    let uri = Uri::parse_str("sc://h/p?q=1").unwrap();
    assert_eq!(roundtrip(&uri).unwrap(), uri);
    ```
