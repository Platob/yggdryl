# Holder

Every storage implementation is reached through the positional `IOBase` contract.

## Pages

| page | owns |
| --- | --- |
| [Bytes](iobase/bytes.md) | `pread`/`pwrite`, streams, cursors |
| [Records](iobase/records.md) | Arrow batches, pushdown, partitions |
| [Values](iobase/values.md) | bytes, digests, structured scalars |
| [Local](backends/local.md) | `Path`, `Folder`, mapped `File` |
| [Buffer](backends/buffer.md) | in-memory bytes |
| [Buffered](backends/buffered.md) | the page cache |
| [Filesystems](backends/filesystems.md) | Arrow-style `FileSystem` |

## Contract

| key | value |
| --- | --- |
| Owns | one enum over every `IOBase` implementation |
| Variants | local `Buffer`/`Folder`/`Path`/`File`, `fs`, `Buffered`/`Text`/`Media` |
| `Holder::local` | `Holder::Path`, the unresolved role |
| `buffer` / `folder` / `file` | commit to a role |
| Lazy | construction touches no filesystem |
| `Holder::open` | wraps IPC, Parquet, Avro, text, then opens |
| Idempotent | `into_text`, `buffered`, `into_media` never stack |
| Hierarchy | `parent`, `child_by_path`, `ls` return `Holder` |

## Use

Rust only.

```rust
use yggdryl::holder::Holder;
use yggdryl::holder::local::Folder;

// Generic construction records the location without probing its role.
let directory = Holder::local(Folder::temporary()?.path()?)?;
assert!(matches!(directory, Holder::Path(_)));

let missing = Holder::local(Folder::temporary()?.path()?.join("yggdryl-generic-doc.bin"))?;
assert!(matches!(missing, Holder::Path(_)));
```

## Hierarchy

```rust
use yggdryl::holder::Holder;
use yggdryl::IOBase;
use yggdryl::holder::local::Folder;

let root = Holder::folder(Folder::temporary()?.path()?)?;
assert!(root.is_container());

// A child need not exist. Naming one yields a leaf handle, and nothing is created.
let leaf = root.child_by_path("yggdryl-generic-child.bin")?;
assert!(matches!(leaf, Holder::File(_)));
assert!(!leaf.is_container());
assert_eq!(leaf.size(), 0);
```

## Roles

Rust only. Bindings expose one handle class.

```rust
use yggdryl::IOBase;
use yggdryl::{IOKind, MimeType};
use yggdryl::holder::local;

let path = local::Folder::temporary()?.path()?.join("yggdryl-docs-io-folder");
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

| trait | declares | pre-implements |
| --- | --- | --- |
| `IOFolder` | `folder_url`, `folder_exists`, `create_folder`, `list_folder` | `folder_pread`, `folder_pwrite`, `folder_truncate`, `folder_media_type`, `folder_kind` |
| `IOFile` | `file_url`, `file_exists` | `file_ls`, `file_child_by_path`, `file_kind` |
| `IOPath` | `path_url`, `is_folder`, `is_file` | `path_exists`, `path_kind`, `path_media_type` |

```rust
use yggdryl::IOBase;
use yggdryl::{IOKind};
use yggdryl::holder::local;

// A location that arrived from outside answers by looking at what is there.
let existing = local::Path::new(local::Folder::temporary()?.path()?)?;
assert_eq!(existing.kind(), IOKind::Directory);

let undecided = local::Path::new(local::Folder::temporary()?.path()?.join("yggdryl-docs-io-undecided"))?;
assert_eq!(undecided.kind(), IOKind::Unknown);
assert!(undecided.read_all_bytes()?.is_empty());

// A leaf is not a container: it lists nothing and resolves no child.
let leaf = local::File::new(local::Folder::temporary()?.path()?.join("yggdryl-docs-io-leaf.arrows"))?;
assert_eq!(leaf.ls(true, false).count(), 0);
assert!(leaf.child_by_path("nested").is_err());
```

## Delegating to a wrapped handle

Rust only. Neither binding can add a backend.

```rust
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::Buffer;

/// A wrapper mirrors the handle's bytes rather than owning bytes of its own.
struct Wrapped {
    handle: Buffer,
}

impl IOMedia for Wrapped {
    yggdryl::delegate_iomedia!(handle);
}

impl IOBase for Wrapped {
    yggdryl::delegate_iobase!(handle);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut wrapper = Wrapped {
        handle: Buffer::new(),
    };
    wrapper.open()?;
    wrapper.write_all_bytes(b"AAPL")?;

    assert_eq!(wrapper.opened(), wrapper.handle.opened());
    assert_eq!(wrapper.read_all_bytes()?, b"AAPL");
    assert_eq!(wrapper.handle.as_slice(), b"AAPL");
    Ok(())
}
```

| spelling | forwards |
| --- | --- |
| `delegate_iobase!(handle)` | storage contract, `open`, `opened`, `close`; no records |
| `delegate_iomedia!(handle)` | dimensions, options, reads, typed writes |
| `delegate_iobase!(handle, except_lifecycle)` | omits `clear`, `remove`, `is_atomic`, `is_tabular`, `is_io` |
| `delegate_iobase!(handle: pread, size, ...)` | only the named methods |

## Edges

- `pwrite` on a container -> refused, naming the directory; `truncate(0)` creates it.
- JSON, directories, unknown media -> `Holder::open` leaves them raw.
- A method omitted from the list form -> the trait default, so `clear` truncates.
- `Ipc`, `Parquet`, the text handler -> `except_lifecycle`; [Buffered](backends/buffered.md) -> the list form.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib holder::
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::
    cargo bench --bench holder --features parquet
    ```
