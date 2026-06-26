# Media types

Media-type detection comes in two layers: [`MimeType`](#mimetype) is a single MIME
type (`image/png`, `application/json`) backed by a mutable global registry of file
extensions and magic-byte signatures, and [`MediaType`](#mediatype) is an ordered
**stack** of them describing a layered file — `data.csv.gz` is `[Csv, Gzip]`. Every
snippet below comes in **Python / Node / Rust**; pick one tab and the page follows.

## MimeType

A `MimeType` is one common MIME type with an `Other` escape hatch for anything not
built in. Construct it from a string (a full `type/subtype`, a `;parameter`-bearing
header value, or a short name), from parts, from an extension, or from magic bytes.

=== "Python"

    ```python
    import yggdryl

    m = yggdryl.MimeType("application/json")
    assert m.mime == "application/json"
    assert m.type == "application"
    assert m.subtype == "json"
    assert m.is_known

    # Parameters are dropped; case is normalised; short names resolve too.
    assert yggdryl.MimeType("Text/HTML; charset=utf-8").subtype == "html"
    assert yggdryl.MimeType("zstd").mime == "application/zstd"   # subtype/extension alias
    assert yggdryl.MimeType.from_parts("text", "csv") == yggdryl.MimeType("text/csv")
    ```

=== "Node"

    ```javascript
    const { MimeType } = require("yggdryl");

    const m = new MimeType("application/json");
    m.mime;     // "application/json"
    m.type;     // "application"
    m.subtype;  // "json"
    m.isKnown;  // true

    new MimeType("Text/HTML; charset=utf-8").subtype;        // "html"
    new MimeType("zstd").mime;                                // "application/zstd"
    MimeType.fromParts("text", "csv").equals(new MimeType("text/csv")); // true
    ```

=== "Rust"

    ```rust
    use yggdryl_core::MimeType;

    let m = MimeType::from_str("application/json")?;
    assert_eq!(m, MimeType::Json);
    assert_eq!(m.mime(), "application/json");
    assert_eq!(m.type_(), "application");
    assert_eq!(m.subtype(), "json");
    assert!(m.is_known());

    // Parameters are dropped; short names resolve via extension or subtype.
    assert_eq!(MimeType::from_str("Text/HTML; charset=utf-8")?, MimeType::Html);
    assert_eq!(MimeType::from_str("zstd")?, MimeType::Zstd);
    assert_eq!(MimeType::from_parts("text", "csv"), MimeType::Csv);
    ```

!!! note "Short names vs. full MIME strings"
    A slash-less token (`"json"`, `"gz"`, `"gzip"`, `"zstd"`) is treated as a short
    **name** and resolved by file extension first, then MIME subtype. A full
    `type/subtype` that is well-formed but not built in becomes `Other` (kept
    verbatim); an unknown short name is an error.

### From an extension

`from_extension` looks up the registry, normalising case and any leading dots. It
returns the matching type or null when nothing is registered.

=== "Python"

    ```python
    assert yggdryl.MimeType.from_extension("parquet").mime == "application/vnd.apache.parquet"
    assert yggdryl.MimeType.from_extension(".GZ").mime == "application/gzip"
    assert yggdryl.MimeType.from_extension("nope") is None
    ```

=== "Node"

    ```javascript
    MimeType.fromExtension("parquet").mime; // "application/vnd.apache.parquet"
    MimeType.fromExtension(".GZ").mime;     // "application/gzip"
    MimeType.fromExtension("nope");         // null
    ```

=== "Rust"

    ```rust
    assert_eq!(MimeType::from_extension("parquet"), Some(MimeType::Parquet));
    assert_eq!(MimeType::from_extension(".GZ"), Some(MimeType::Gzip));
    assert_eq!(MimeType::from_extension("nope"), None);
    ```

### Magic-byte sniffing

`from_magic` matches the registry's magic-byte signatures against a buffer's leading
bytes — recognising container and columnar formats (ZIP, gzip, Parquet, Arrow IPC,
PNG, …). Signatures may sit at a fixed offset: tar's `ustar` lives at byte 257, and
AVIF's `ftypavif` brand at byte 4 (matched ahead of MP4's generic `ftyp`). Formats
without a stable signature — Brotli, HEIC, AAC, Opus, Matroska, and the ZIP-container
documents (`.docx`/`.xlsx`/`.pptx`/`.epub`) — are recognised by extension only.

=== "Python"

    ```python
    assert yggdryl.MimeType.from_magic(b"PK\x03\x04\x14").subtype == "zip"
    assert yggdryl.MimeType.from_magic(b"\x1f\x8b\x08\x00").mime == "application/gzip"
    assert yggdryl.MimeType.from_magic(b"PAR1\x15\x04").mime == "application/vnd.apache.parquet"
    assert yggdryl.MimeType.from_magic(b"\xfd7zXZ\x00\x00").mime == "application/x-xz"
    assert yggdryl.MimeType.from_magic(b"not magic") is None
    ```

=== "Node"

    ```javascript
    MimeType.fromMagic(Buffer.from("PK\x03\x04\x14")).subtype;          // "zip"
    MimeType.fromMagic(Buffer.from([0x1f, 0x8b, 0x08, 0x00])).mime;     // "application/gzip"
    MimeType.fromMagic(Buffer.from("PAR1\x15\x04")).mime;               // "application/vnd.apache.parquet"
    MimeType.fromMagic(Buffer.from("\xfd7zXZ\x00\x00", "binary")).mime; // "application/x-xz"
    MimeType.fromMagic(Buffer.from("not magic"));                        // null
    ```

=== "Rust"

    ```rust
    assert_eq!(MimeType::from_magic(b"PK\x03\x04\x14"), Some(MimeType::Zip));
    assert_eq!(MimeType::from_magic(b"\x1f\x8b\x08\x00"), Some(MimeType::Gzip));
    assert_eq!(MimeType::from_magic(b"PAR1\x15\x04"), Some(MimeType::Parquet));
    assert_eq!(MimeType::from_magic(b"\xfd7zXZ\x00\x00"), Some(MimeType::Xz));
    assert_eq!(MimeType::from_magic(b"not magic"), None);
    ```

### Extensions and the octet-stream fallback

`extensions` lists every registered extension (the first is canonical); `extension`
is that first one, or null. `Other` types carry none. The conventional fallback for
failed inference is the default, `application/octet-stream`.

=== "Python"

    ```python
    jpeg = yggdryl.MimeType.from_extension("jpg")
    assert jpeg.extensions == ["jpg", "jpeg"]
    assert jpeg.extension == "jpg"
    assert yggdryl.MimeType("application/x-custom").extensions == []

    assert yggdryl.MimeType.default().mime == "application/octet-stream"
    assert (yggdryl.MimeType.from_path("notes") or yggdryl.MimeType.default()).mime \
        == "application/octet-stream"
    ```

=== "Node"

    ```javascript
    const jpeg = MimeType.fromExtension("jpg");
    jpeg.extensions;  // ["jpg", "jpeg"]
    jpeg.extension;   // "jpg"
    new MimeType("application/x-custom").extensions; // []

    MimeType.default().mime; // "application/octet-stream"
    (MimeType.fromPath("notes") ?? MimeType.default()).mime; // "application/octet-stream"
    ```

=== "Rust"

    ```rust
    assert_eq!(MimeType::Jpeg.extensions(), vec!["jpg", "jpeg"]);
    assert_eq!(MimeType::Jpeg.extension(), Some("jpg".to_string()));
    assert!(MimeType::Other("x/y".to_string()).extensions().is_empty());

    assert_eq!(MimeType::default(), MimeType::OctetStream);
    assert_eq!(MimeType::from_extension("nope").unwrap_or_default(), MimeType::OctetStream);
    ```

### Category — what a type *is*

Every type plays a broad **category**, grouping it by how its bytes are consumed
rather than by `type/subtype`: `"blob"` (the default — an opaque, random-access byte
holder: images, audio, video, PDFs, archives, fonts), `"directory"` (`inode/directory`),
`"tabular"` (CSV, Parquet, Arrow, Avro, NDJSON), `"code"` (source code, markup and
config — a programming language, HTML/CSS, JSON/XML/YAML/TOML, Markdown), and `"codec"`
(a compression codec: gzip, bzip2, zstd, brotli, xz, lz4). The category lives in the
same registry as the extensions/magic, so `register` can set it.

Programming languages are first-class built-ins (all in the `"code"` category):
Python (`.py`), Rust (`.rs`), TypeScript (`.ts`), Go (`.go`), C (`.c`/`.h`), C++
(`.cpp`/…), C# (`.cs`), Java (`.java`), Ruby (`.rb`), PHP (`.php`), shell
(`.sh`/`.bash`), Swift (`.swift`), Kotlin (`.kt`), SQL (`.sql`), Lua (`.lua`),
Perl (`.pl`), Scala (`.scala`), R (`.r`), Dart (`.dart`) and Haskell (`.hs`).

=== "Python"

    ```python
    assert yggdryl.MimeType("image/png").category == "blob"      # the default
    assert yggdryl.MimeType("text/csv").category == "tabular"
    assert yggdryl.MimeType("application/gzip").category == "codec"
    assert yggdryl.MimeType("application/json").category == "code"

    py = yggdryl.MimeType.from_extension("py")
    assert py.mime == "text/x-python"
    assert py.category == "code"
    ```

=== "Node"

    ```javascript
    new MimeType("image/png").category;      // "blob" (the default)
    new MimeType("text/csv").category;       // "tabular"
    new MimeType("application/gzip").category; // "codec"
    new MimeType("application/json").category; // "code"

    const py = MimeType.fromExtension("py");
    py.mime;     // "text/x-python"
    py.category; // "code"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Category, MimeType};

    assert_eq!(MimeType::Png.category(), Category::Blob);   // the default
    assert_eq!(MimeType::Csv.category(), Category::Tabular);
    assert_eq!(MimeType::Gzip.category(), Category::Codec);
    assert_eq!(MimeType::Json.category(), Category::Code);

    let py = MimeType::from_extension("py").unwrap();
    assert_eq!(py, MimeType::Python);
    assert_eq!(py.category(), Category::Code);
    ```

## The mutable registry

Extensions and magic bytes live in a **process-wide** registry. Register (or replace)
a type so subsequent `from_extension` / `from_magic` lookups recognise it; unregister
it by its canonical string; `reset_registry` restores the built-in defaults. The
optional `category` (default `"blob"`) sets what [category](#category-what-a-type-is)
the registered type reports.

=== "Python"

    ```python
    assert yggdryl.MimeType.from_extension("ygg") is None
    yggdryl.MimeType.register("application/x-yggdryl", ["ygg"], [b"YGG1"], category="code")

    m = yggdryl.MimeType.from_extension("ygg")
    assert m.mime == "application/x-yggdryl"
    assert m.category == "code"
    assert yggdryl.MimeType.from_magic(b"YGG1\x00").mime == "application/x-yggdryl"

    assert yggdryl.MimeType.unregister("application/x-yggdryl")
    assert yggdryl.MimeType.from_extension("ygg") is None
    ```

=== "Node"

    ```javascript
    MimeType.fromExtension("ygg"); // null
    MimeType.register("application/x-yggdryl", ["ygg"], [Buffer.from("YGG1")], "code");

    const m = MimeType.fromExtension("ygg");
    m.mime;     // "application/x-yggdryl"
    m.category; // "code"
    MimeType.fromMagic(Buffer.from("YGG1\x00")).mime; // "application/x-yggdryl"

    MimeType.unregister("application/x-yggdryl"); // true
    MimeType.fromExtension("ygg");                // null
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Category, MimeType, Signature};

    assert_eq!(MimeType::from_extension("ygg"), None);
    MimeType::register(
        "application/x-yggdryl", &["ygg"], &[Signature::prefix(b"YGG1")], Category::Code,
    );

    let m = MimeType::from_extension("ygg").unwrap();
    assert_eq!(m, MimeType::Other("application/x-yggdryl".to_string()));
    assert_eq!(m.category(), Category::Code);
    assert_eq!(MimeType::from_magic(b"YGG1\x00"), m.clone().into());

    assert!(MimeType::unregister("application/x-yggdryl"));
    assert_eq!(MimeType::from_extension("ygg"), None);
    ```

!!! tip "Signatures at an offset"
    In Rust a magic signature is a `Signature`: `Signature::prefix(bytes)` matches at
    the start, `Signature::at(offset, bytes)` at a fixed offset. The bindings take the
    raw `bytes`/`Buffer` and match them as a prefix. `category` is one of `"blob"` /
    `"directory"` / `"tabular"` / `"code"` / `"codec"` (in Rust, the `Category` enum).

## MediaType

A `MediaType` is an ordered stack of `MimeType`s, innermost content first. Build it
from a path, one extension, or many; `first`/`last` pick the content and container
ends, `category` reports the outermost layer's [category](#category-what-a-type-is)
(`codec` for `data.csv.gz`, `tabular` for `data.csv`), and rendering produces the
canonical dotted extension chain (`csv.gz`).

=== "Python"

    ```python
    stack = yggdryl.MediaType.from_path("data.csv.gz")
    assert [t.mime for t in stack.types] == ["text/csv", "application/gzip"]
    assert stack.first.mime == "text/csv"
    assert stack.last.mime == "application/gzip"
    assert stack.category == "codec"   # the outermost layer (gzip)
    assert len(stack) == 2
    assert str(stack) == "csv.gz"

    # From extensions (unknown ones are skipped); explicit construction.
    assert [t.mime for t in yggdryl.MediaType.from_extensions(["csv", "nope", "gz"]).types] \
        == ["text/csv", "application/gzip"]
    assert yggdryl.MediaType([yggdryl.MimeType("text/csv")]).first.mime == "text/csv"
    ```

=== "Node"

    ```javascript
    const { MediaType, MimeType } = require("yggdryl");

    const stack = MediaType.fromPath("data.csv.gz");
    stack.types.map((t) => t.mime); // ["text/csv", "application/gzip"]
    stack.first.mime;               // "text/csv"
    stack.last.mime;                // "application/gzip"
    stack.category;                 // "codec" (the outermost layer, gzip)
    stack.length;                   // 2
    stack.toString();               // "csv.gz"

    MediaType.fromExtensions(["csv", "nope", "gz"]).types.map((t) => t.mime);
    // ["text/csv", "application/gzip"]
    new MediaType([new MimeType("text/csv")]).first.mime; // "text/csv"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Category, MediaType, MimeType};

    let stack = MediaType::from_path("data.csv.gz");
    assert_eq!(stack.types(), [MimeType::Csv, MimeType::Gzip]);
    assert_eq!(stack.first(), Some(&MimeType::Csv));
    assert_eq!(stack.last(), Some(&MimeType::Gzip));
    assert_eq!(stack.category(), Category::Codec); // the outermost layer (gzip)
    assert_eq!(stack.len(), 2);
    assert_eq!(stack.to_str(true), "csv.gz");

    // From extensions (unknown ones are skipped); explicit construction.
    assert_eq!(
        MediaType::from_extensions(&["csv", "nope", "gz"]).types(),
        [MimeType::Csv, MimeType::Gzip],
    );
    assert_eq!(MediaType::new(vec![MimeType::Csv]).first(), Some(&MimeType::Csv));
    ```

A leading dot makes the first segment a dotfile stem (`.bashrc` → empty), a trailing
dot or extension-less name yields an empty stack, and only the file name is read (a
directory that looks like an extension is ignored). An empty stack defaults to
`[OctetStream]`.

### Compound archive extensions

A single contracted token expands to the same stack as its dotted form, so `app.tgz`
is `[Tar, Gzip]` — exactly `app.tar.gz`. The canonical rendering is always the dotted
chain (`tar.gz`), never the contraction.

| Contraction | Expands to | Stack |
| --- | --- | --- |
| `.tgz` / `.taz` | `.tar.gz` | `[Tar, Gzip]` |
| `.tbz` / `.tbz2` / `.tb2` | `.tar.bz2` | `[Tar, Bzip2]` |
| `.txz` | `.tar.xz` | `[Tar, Xz]` |
| `.tzst` | `.tar.zst` | `[Tar, Zstd]` |

=== "Python"

    ```python
    tgz = [t.mime for t in yggdryl.MediaType.from_path("app.tgz").types]
    assert tgz == ["application/x-tar", "application/gzip"]
    assert tgz == [t.mime for t in yggdryl.MediaType.from_path("app.tar.gz").types]
    assert str(yggdryl.MediaType.from_path("app.tgz")) == "tar.gz"
    ```

=== "Node"

    ```javascript
    const tgz = MediaType.fromPath("app.tgz").types.map((t) => t.mime);
    // ["application/x-tar", "application/gzip"]
    MediaType.fromPath("app.tgz").toString(); // "tar.gz"
    ```

=== "Rust"

    ```rust
    assert_eq!(
        MediaType::from_path("app.tgz").types(),
        [MimeType::Tar, MimeType::Gzip],
    );
    assert_eq!(MediaType::from_path("app.tgz").to_str(true), "tar.gz");
    ```

### Newly supported common types

Beyond the classics, the registry recognises a broad set of modern formats:

- **Text / data** — `yaml` / `yml` (`application/yaml`), `toml`, `ndjson` / `jsonl`
  (`application/x-ndjson`), `rtf`.
- **Compression / archives** — `xz` (`application/x-xz`), `lz4` (`application/x-lz4`),
  plus the compound forms above.
- **Documents** — `epub`, and the OOXML `docx` / `xlsx` / `pptx` (ZIP containers, so
  matched by extension).
- **Image** — `avif` (`image/avif`, sniffed via its `ftypavif` brand), `heic` / `heif`.
- **Audio / video** — `aac`, `opus`, and `mkv` (`video/x-matroska`).

=== "Python"

    ```python
    assert yggdryl.MimeType.from_extension("yaml").mime == "application/yaml"
    assert yggdryl.MimeType.from_extension("toml").mime == "application/toml"
    assert yggdryl.MimeType.from_extension("avif").mime == "image/avif"
    assert yggdryl.MimeType.from_extension("mkv").mime == "video/x-matroska"
    ```

=== "Node"

    ```javascript
    MimeType.fromExtension("yaml").mime; // "application/yaml"
    MimeType.fromExtension("toml").mime; // "application/toml"
    MimeType.fromExtension("avif").mime; // "image/avif"
    MimeType.fromExtension("mkv").mime;  // "video/x-matroska"
    ```

=== "Rust"

    ```rust
    assert_eq!(MimeType::from_extension("yaml"), Some(MimeType::Yaml));
    assert_eq!(MimeType::from_extension("toml"), Some(MimeType::Toml));
    assert_eq!(MimeType::from_extension("avif"), Some(MimeType::Avif));
    assert_eq!(MimeType::from_extension("mkv"), Some(MimeType::Matroska));
    ```

## Inferred on a URL

`Uri`/`Url` infer a `media_type()` (the full stack) or `mime_type()` (the outermost
type) from their path — `archive.tar.gz` is `[Tar, Gzip]`. See [URI & URL](url.md).

=== "Python"

    ```python
    url = yggdryl.Url("https://h/dump/archive.tar.gz")
    assert [t.mime for t in url.media_type().types] == ["application/x-tar", "application/gzip"]
    assert url.mime_type().mime == "application/gzip"
    assert yggdryl.Uri("https://h/page").media_type() is None
    ```

=== "Node"

    ```javascript
    const url = new Url("https://h/dump/archive.tar.gz");
    url.mediaType().types.map((t) => t.mime); // ["application/x-tar", "application/gzip"]
    url.mimeType().mime;                       // "application/gzip"
    new Uri("https://h/page").mediaType();     // null
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{MediaType, MimeType, Url};

    let url = Url::from_str("https://h/dump/archive.tar.gz")?;
    assert_eq!(MediaType::from(&url).types(), [MimeType::Tar, MimeType::Gzip]);
    ```

For the HTTP side — where `Content-Type` and `Content-Encoding` combine into a stack —
see [Request & Response](../http/request-response.md); to act on the container type
(decompress a gzip layer), see [Compression](compression.md).
