# The io root

The **`io` layer**'s root holds the cross-cutting value types every module shares; below it,
[`memory`](memory.md) owns byte access, and the root-level [`uri`](../uri.md) family addresses
the sources. The same folder tree is mirrored across the core, the tests, the benchmarks, and
both extensions.

| Type | What it is |
|---|---|
| `Serializable` | the root byte-codec trait — `serialize_bytes()` / `deserialize_bytes()` are exact inverses; every value type implements it when possible |
| [`Headers`](../headers.md) | the project's **one** metadata map — now a root module beside `uri`; HTTP headers, schema metadata, and source annotations all live here |
| `IOMode` | how a source may be accessed — an int enum: `Read = 1`, `Write = 2`, `ReadWrite = 3`, `Append = 4`, `Overwrite = 5` |
| `IOKind` | what a source is — an int enum: `Unknown = 0` (the default), `Missing = 1`, `File = 2`, `Directory = 3`, `Heap = 4` |
| `IoError` / `Whence` | the guided error family and the seek anchor (`Start` / `Current` / `End`) |

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
    assert IOKind.Unknown == 0 and IOKind.Unknown.exists()  # exists, type undetermined
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
    console.assert(IOKind.Unknown === 0 && ioKindExists(IOKind.Unknown)) // exists, undetermined
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
    assert_eq!(IOKind::default(), IOKind::Unknown); // Unknown = 0, exists but undetermined
    assert!(IOKind::Unknown.exists() && !IOKind::Missing.exists());
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
    use yggdryl_core::uri::Uri;

    fn roundtrip<T: Serializable>(value: &T) -> Result<T, T::Error> {
        T::deserialize_bytes(&value.serialize_bytes())
    }
    let uri = Uri::parse_str("sc://h/p?q=1").unwrap();
    assert_eq!(roundtrip(&uri).unwrap(), uri);
    ```
