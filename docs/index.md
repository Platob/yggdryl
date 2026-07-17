# yggdryl

A Rust library with **Python** and **Node.js** extensions. All logic lives in the Rust core
(`yggdryl-core`); the bindings are thin wrappers, so the three languages behave identically —
every feature is added to the core first, then mirrored, method-for-method, in both extensions.

The core is the **`io` layer**: the abstract [memory-access contract](io/memory.md) (`IOBase`
with the `Cursor`/`Slice` wrappers and the in-heap `Heap` source) and the [URI/URL
family](io/uri.md) (`Uri` / `Url` / `Authority`) that addresses those sources — plus the [shared value
types at the `io` root](io/index.md) (`Serializable`, `Headers`, `IOMode`, `IOKind`). Both bindings mirror it in full; more sources plug in against the
same contract as the library grows.

## Install

=== "Python"

    ```bash
    uv pip install yggdryl
    ```

=== "Node"

    ```bash
    npm install yggdryl
    ```

=== "Rust"

    ```bash
    cargo add yggdryl-core
    ```

## Version

The one value every surface exposes — the minimal end-to-end example that the Python and
Node extensions both wire through to the Rust core.

=== "Python"

    ```python
    import yggdryl

    print(yggdryl.version())  # -> "0.1.1"
    ```

=== "Node"

    ```js
    const yggdryl = require('yggdryl')

    console.log(yggdryl.version()) // -> "0.1.1"
    ```

=== "Rust"

    ```rust
    fn main() {
        println!("{}", yggdryl_core::version()); // -> "0.1.1"
    }
    ```

## URIs and URLs

RFC 3986 URIs, absolute URLs, and authorities — parsed from scratch, doubling as
POSIX-normalized filesystem paths, with value semantics (equal, hashable, and
byte-serializable) across all three languages. See [URIs and URLs](io/uri.md).

=== "Python"

    ```python
    from yggdryl.uri import Uri

    uri = Uri.parse("https://user:pw@example.com:8080/a/b.tar.gz?q=1#frag")
    assert uri.host == "example.com"
    assert uri.extensions == ["tar", "gz"]
    ```

=== "Node"

    ```js
    const { Uri } = require('yggdryl').uri

    const uri = Uri.parse('https://user:pw@example.com:8080/a/b.tar.gz?q=1#frag')
    console.assert(uri.host === 'example.com')
    console.assert(JSON.stringify(uri.extensions) === '["tar","gz"]')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::uri::Uri;

    let uri = Uri::parse_str("https://user:pw@example.com:8080/a/b.tar.gz?q=1#frag").unwrap();
    assert_eq!(uri.host(), Some("example.com"));
    assert_eq!(uri.extensions(), vec!["tar", "gz"]);
    ```
