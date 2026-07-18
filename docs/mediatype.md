# Media types

A **`MimeType`** is one media type — a `type/subtype` essence (`application/json`) with the
file **extensions** it is known by and the **magic-byte** signatures a file of that type
starts with. A **`MediaType`** is an ordered list of them — the layered description of a
resource (`archive.tar.gz` → `application/x-tar` then `application/gzip`). A **`MimeCatalog`**
resolves a type from a mime string, a file name, an extension, or magic bytes, and a
process-wide default catalog backs the ergonomic lookups.

The io layer uses this to answer "what is this?": every source infers a type from its
`Content-Type` headers, else its address, else the `application/octet-stream` fallback.

## MimeType basics

Parse a mime string (parameters like `; charset=utf-8` are dropped), read its essence / type /
subtype, and fall back to `application/octet-stream` for the unknown.

=== "Python"

    ```python
    from yggdryl.mimetype import MimeType

    m = MimeType.parse("application/json; charset=utf-8")
    assert m.essence == "application/json"   # parameters dropped
    assert m.type == "application"
    assert m.subtype == "json"

    assert MimeType.octet_stream().is_octet_stream()
    ```

=== "Node"

    ```javascript
    const { MimeType } = require('yggdryl').mimetype

    const m = MimeType.parse('application/json; charset=utf-8')
    console.assert(m.essence === 'application/json') // parameters dropped
    console.assert(m.type === 'application')
    console.assert(m.subtype === 'json')

    console.assert(MimeType.octetStream().isOctetStream())
    ```

=== "Rust"

    ```rust
    use yggdryl_core::mimetype::MimeType;

    let m = MimeType::parse_str("application/json; charset=utf-8").unwrap();
    assert_eq!(m.essence(), "application/json"); // parameters dropped
    assert_eq!(m.type_(), "application");
    assert_eq!(m.subtype(), "json");

    assert!(MimeType::octet_stream().is_octet_stream());
    ```

## Resolving a type

Resolve from a file **extension**, a **file name** (its last extension), or the **magic
bytes** at the head of a file, through the default catalog. `guess` always answers: magic wins
when it matches, then the name's extension, else the octet-stream fallback.

=== "Python"

    ```python
    from yggdryl.mimetype import MimeType

    assert MimeType.from_extension("png").essence == "image/png"
    assert MimeType.from_name("report.final.pdf").essence == "application/pdf"
    assert MimeType.from_magic(b"%PDF-1.7").essence == "application/pdf"
    assert MimeType.from_name("Makefile") is None       # no extension

    # guess: magic beats the (wrong) name, and it never returns None.
    png = b"\x89PNG\r\n\x1a\n"
    assert MimeType.guess("mislabeled.txt", png).essence == "image/png"
    assert MimeType.guess("mystery", b"\x00\x01").is_octet_stream()
    ```

=== "Node"

    ```javascript
    const { MimeType } = require('yggdryl').mimetype

    console.assert(MimeType.fromExtension('png').essence === 'image/png')
    console.assert(MimeType.fromName('report.final.pdf').essence === 'application/pdf')
    console.assert(MimeType.fromMagic(Buffer.from('%PDF-1.7')).essence === 'application/pdf')
    console.assert(MimeType.fromName('Makefile') === null) // no extension

    const png = Buffer.from('\x89PNG\r\n\x1a\n', 'binary')
    console.assert(MimeType.guess('mislabeled.txt', png).essence === 'image/png')
    console.assert(MimeType.guess('mystery', Buffer.from([0, 1])).isOctetStream())
    ```

=== "Rust"

    ```rust
    use yggdryl_core::mimetype::MimeType;

    assert_eq!(MimeType::from_extension("png").unwrap().essence(), "image/png");
    assert_eq!(MimeType::from_name("report.final.pdf").unwrap().essence(), "application/pdf");
    assert_eq!(MimeType::from_magic(b"%PDF-1.7").unwrap().essence(), "application/pdf");
    assert!(MimeType::from_name("Makefile").is_none()); // no extension

    let png = b"\x89PNG\r\n\x1a\n";
    assert_eq!(MimeType::guess("mislabeled.txt", png).essence(), "image/png");
    assert!(MimeType::guess("mystery", &[0, 1]).is_octet_stream());
    ```

## A custom registry

Start from the built-in known types (`defaults()`) and `register` your own. A later
registration replaces an earlier one with the same essence, and magic resolution prefers the
**longest** matching signature so a specific prefix beats a shorter shared one.

=== "Python"

    ```python
    from yggdryl.mimetype import MimeType, MimeCatalog

    catalog = MimeCatalog.defaults()
    catalog.register(MimeType("application/x-yggdryl", ["ygg"], [b"YGGD"]))
    assert catalog.from_extension("ygg").essence == "application/x-yggdryl"
    assert catalog.from_mime("application/json").essence == "application/json"
    ```

=== "Node"

    ```javascript
    const { MimeType, MimeCatalog } = require('yggdryl').mimetype

    const catalog = MimeCatalog.defaults()
    catalog.register(new MimeType('application/x-yggdryl', ['ygg'], [Buffer.from('YGGD')]))
    console.assert(catalog.fromExtension('ygg').essence === 'application/x-yggdryl')
    console.assert(catalog.fromMime('application/json').essence === 'application/json')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::mimetype::{MimeCatalog, MimeRegistry, MimeType};

    let mut catalog = MimeCatalog::defaults();
    catalog.register(MimeType::new("application/x-yggdryl", ["ygg"], [b"YGGD".to_vec()]));
    assert_eq!(catalog.from_extension("ygg").unwrap().essence(), "application/x-yggdryl");
    assert_eq!(catalog.from_mime("application/json").unwrap().essence(), "application/json");
    ```

## MediaType — the layered list

Parse a comma-separated mime list, or build one from a file's extensions — each extension maps
to its type, so a wrapped file lists the content type then its encodings. `primary()` is the
first (most specific) type.

=== "Python"

    ```python
    from yggdryl.mediatype import MediaType

    m = MediaType.parse("application/json, text/html")
    assert m.primary().essence == "application/json"
    assert m.essences() == ["application/json", "text/html"]

    tgz = MediaType.from_extensions(["tar", "gz"])
    assert tgz.essences() == ["application/x-tar", "application/gzip"]
    ```

=== "Node"

    ```javascript
    const { MediaType } = require('yggdryl').mediatype

    const m = MediaType.parse('application/json, text/html')
    console.assert(m.primary().essence === 'application/json')
    console.assert(JSON.stringify(m.essences()) === '["application/json","text/html"]')

    const tgz = MediaType.fromExtensions(['tar', 'gz'])
    console.assert(JSON.stringify(tgz.essences()) === '["application/x-tar","application/gzip"]')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::mediatype::MediaType;

    let m = MediaType::parse_str("application/json, text/html").unwrap();
    assert_eq!(m.primary().unwrap().essence(), "application/json");
    assert_eq!(m.essences(), vec!["application/json", "text/html"]);

    let tgz = MediaType::from_extensions(["tar", "gz"]);
    assert_eq!(tgz.essences(), vec!["application/x-tar", "application/gzip"]);
    ```

## On a URI or a source

A [`Uri`](uri.md) infers a type from its path, and every io source infers one from its
`Content-Type` headers, else its address, else octet-stream — always an answer.
`ensure_content_type` memoizes the inference into the source's [headers](headers.md) (only when
absent), so later reads come straight from the map.

=== "Python"

    ```python
    from yggdryl.uri import Uri
    from yggdryl.memory import Heap

    assert Uri.from_path("/data/report.pdf").mime_type().essence == "application/pdf"
    assert Uri.from_path("/x/archive.tar.gz").media_type().essences() == \
        ["application/x-tar", "application/gzip"]

    h = Heap()
    assert h.mime_type().is_octet_stream()          # no headers, no address extension
    hdrs = h.headers                                # a copy; write it back to store
    hdrs.set_content_type("application/json")
    h.set_headers(hdrs)
    assert h.mime_type().essence == "application/json"  # headers win
    ```

=== "Node"

    ```javascript
    const { Uri } = require('yggdryl').uri
    const { Heap } = require('yggdryl').memory

    console.assert(Uri.fromPath('/data/report.pdf').mimeType().essence === 'application/pdf')

    const h = new Heap()
    console.assert(h.mimeType().isOctetStream())     // no headers, no address extension
    const hdrs = h.headers                           // a copy; write it back to store
    hdrs.setContentType('application/json')
    h.setHeaders(hdrs)
    console.assert(h.mimeType().essence === 'application/json') // headers win
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::Uri;
    use yggdryl_core::io::memory::{Heap, IOBase};

    assert_eq!(Uri::from_path("/data/report.pdf").mime_type().essence(), "application/pdf");

    let mut h = Heap::new();
    assert!(h.mime_type().is_octet_stream());        // no headers, no address extension
    h.headers_mut().set_content_type("application/json");
    assert_eq!(h.mime_type().essence(), "application/json"); // headers win
    ```

## Value semantics

`MimeType` and `MediaType` are value types: equal, hashable, and byte-serializable. A
`MimeType`'s canonical byte form is its essence; a `MediaType`'s is the comma-joined essences —
`serialize_bytes` / `deserialize_bytes` are inverses, so a type is a stable map key and travels
over a wire identically in every language.
