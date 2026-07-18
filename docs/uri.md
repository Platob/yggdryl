# URIs and URLs

yggdryl's core `io` module parses [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986) URIs from scratch
(no `url` crate — the foundation crate stays minimal-dependency) into three value types:

- **`Uri`** — a generic URI split into its components, doubling as a **filesystem path**.
  Every component may be absent, so a bare path (no scheme, no authority) is a perfectly
  good `Uri`.
- **`Url`** — an **absolute** URI: one guaranteed to carry a scheme. The authority stays
  optional, so `mailto:person@example.com` is a valid `Url` with no host.
- **`Authority`** — the `[user[:password]@]host[:port]` component.

All three have **value semantics**: two values are equal iff their canonical strings are
equal, and equal values hash equal, so they work as dict / map keys and set members.

## Uri vs Url

A `Uri` may be scheme-less; a `Url` never is. Converting a `Uri` to a `Url` fails (with a
guided error) when the URI has no scheme; the reverse is infallible.

=== "Python"

    ```python
    from yggdryl.uri import Uri, Url

    absolute = Uri.parse("https://example.com/a/b.txt")
    assert absolute.scheme == "https"
    assert absolute.to_url().scheme == "https"     # Uri -> Url

    relative = Uri.parse("/just/a/path")
    assert relative.scheme is None
    try:
        relative.to_url()                          # no scheme -> guided error
    except ValueError as error:
        assert "absolute" in str(error)

    assert Url.parse("s3://bucket/key").as_uri().host == "bucket"   # Url -> Uri
    ```

=== "Node"

    ```js
    const { Uri, Url } = require('yggdryl').uri

    const absolute = Uri.parse('https://example.com/a/b.txt')
    console.assert(absolute.scheme === 'https')
    console.assert(absolute.toUrl().scheme === 'https')   // Uri -> Url

    const relative = Uri.parse('/just/a/path')
    console.assert(relative.scheme === null)
    try {
      relative.toUrl()                                    // no scheme -> guided error
    } catch (error) {
      console.assert(/absolute/.test(error.message))
    }

    console.assert(Url.parse('s3://bucket/key').toUri().host === 'bucket')  // Url -> Uri
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::{Uri, Url};

    let absolute = Uri::parse_str("https://example.com/a/b.txt").unwrap();
    assert_eq!(absolute.scheme(), Some("https"));
    assert_eq!(absolute.to_url().unwrap().scheme(), "https");   // Uri -> Url

    let relative = Uri::parse_str("/just/a/path").unwrap();
    assert_eq!(relative.scheme(), None);
    assert!(relative.to_url().is_err());                        // no scheme

    let uri: Uri = Url::parse_str("s3://bucket/key").unwrap().into();  // Url -> Uri
    assert_eq!(uri.host(), Some("bucket"));
    ```

## Accessors

Every RFC 3986 component is a read-only accessor. The path additionally exposes filename
helpers — `name` (last segment), `stem` (name minus its last extension), `extension` (last
extension, no dot), and `extensions` (every extension of a multi-dot name, outermost-last).

| Accessor | Meaning | Example (`https://user:pw@host.com:8080/a/b.tar.gz?q=1#frag`) |
| --- | --- | --- |
| `scheme` | the scheme | `"https"` |
| `authority` | the whole authority | `user:pw@host.com:8080` |
| `user` / `password` | userinfo halves | `"user"` / `"pw"` |
| `host` | host (IPv6 kept bracketed) | `"host.com"` |
| `port` | port number | `8080` |
| `path` | POSIX-normalized path | `"/a/b.tar.gz"` |
| `query` / `fragment` | after `?` / `#` | `"q=1"` / `"frag"` |
| `name` | last path segment | `"b.tar.gz"` |
| `stem` | name minus last extension | `"b.tar"` |
| `extension` | last extension | `"gz"` |
| `extensions` | all extensions | `["tar", "gz"]` |

=== "Python"

    ```python
    from yggdryl.uri import Uri

    uri = Uri.parse("https://user:pw@host.com:8080/a/b.tar.gz?q=1#frag")
    assert uri.scheme == "https"
    assert uri.user == "user"
    assert uri.password == "pw"
    assert uri.host == "host.com"
    assert uri.port == 8080
    assert uri.path == "/a/b.tar.gz"
    assert uri.query == "q=1"
    assert uri.fragment == "frag"
    assert uri.name == "b.tar.gz"
    assert uri.stem == "b.tar"
    assert uri.extension == "gz"
    assert uri.extensions == ["tar", "gz"]
    assert uri.authority.host == "host.com"           # Authority value type

    # An absent component reads as None; a directory-like path has no name.
    assert Uri.parse("/a/b/").name is None
    # A leading dot is not an extension separator.
    assert Uri.from_path("/home/.bashrc").extension is None
    # An IPv6 host keeps its brackets.
    assert Uri.parse("http://[::1]:9000/p").host == "[::1]"
    ```

=== "Node"

    ```js
    const { Uri } = require('yggdryl').uri

    const uri = Uri.parse('https://user:pw@host.com:8080/a/b.tar.gz?q=1#frag')
    console.assert(uri.scheme === 'https')
    console.assert(uri.user === 'user')
    console.assert(uri.password === 'pw')
    console.assert(uri.host === 'host.com')
    console.assert(uri.port === 8080)
    console.assert(uri.path === '/a/b.tar.gz')
    console.assert(uri.query === 'q=1')
    console.assert(uri.fragment === 'frag')
    console.assert(uri.name === 'b.tar.gz')
    console.assert(uri.stem === 'b.tar')
    console.assert(uri.extension === 'gz')
    console.assert(JSON.stringify(uri.extensions) === '["tar","gz"]')
    console.assert(uri.authority.host === 'host.com')   // Authority value type

    // An absent component reads as null; a directory-like path has no name.
    console.assert(Uri.parse('/a/b/').name === null)
    // A leading dot is not an extension separator.
    console.assert(Uri.fromPath('/home/.bashrc').extension === null)
    // An IPv6 host keeps its brackets.
    console.assert(Uri.parse('http://[::1]:9000/p').host === '[::1]')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::Uri;

    let uri = Uri::parse_str("https://user:pw@host.com:8080/a/b.tar.gz?q=1#frag").unwrap();
    assert_eq!(uri.scheme(), Some("https"));
    assert_eq!(uri.user(), Some("user"));
    assert_eq!(uri.password(), Some("pw"));
    assert_eq!(uri.host(), Some("host.com"));
    assert_eq!(uri.port(), Some(8080));
    assert_eq!(uri.path(), "/a/b.tar.gz");
    assert_eq!(uri.query(), Some("q=1"));
    assert_eq!(uri.fragment(), Some("frag"));
    assert_eq!(uri.name(), Some("b.tar.gz"));
    assert_eq!(uri.stem(), Some("b.tar"));
    assert_eq!(uri.extension(), Some("gz"));
    assert_eq!(uri.extensions(), vec!["tar", "gz"]);
    assert_eq!(uri.authority().unwrap().host(), "host.com");

    assert_eq!(Uri::parse_str("/a/b/").unwrap().name(), None);
    assert_eq!(Uri::from_path("/home/.bashrc").extension(), None);
    assert_eq!(Uri::parse_str("http://[::1]:9000/p").unwrap().host(), Some("[::1]"));
    ```

## Parts and media type

`parts()` bundles the RFC 3986 top-level components into one value (`scheme` / `authority` /
`path` / `query` / `fragment`) for destructuring, and `mime_type()` / `media_type()` infer the
resource's [media type](mediatype.md) from the path — the primary type (with the
`application/octet-stream` fallback), or the layered list a multi-extension name implies. `Url`
mirrors both.

=== "Python"

    ```python
    from yggdryl.uri import Uri

    parts = Uri.parse("https://h:8080/a/b?q=1#f").parts()
    assert parts.scheme == "https" and parts.authority == "h:8080"
    assert parts.path == "/a/b" and parts.query == "q=1"
    assert str(parts) == "https://h:8080/a/b?q=1#f"    # re-renders

    assert Uri.from_path("/x/report.pdf").mime_type().essence == "application/pdf"
    assert Uri.from_path("/x/data.tar.gz").media_type().essences() == \
        ["application/x-tar", "application/gzip"]
    ```

=== "Node"

    ```javascript
    const { Uri } = require('yggdryl').uri

    const parts = Uri.parse('https://h:8080/a/b?q=1#f').parts()
    console.assert(parts.scheme === 'https' && parts.authority === 'h:8080')
    console.assert(parts.path === '/a/b' && parts.query === 'q=1')
    console.assert(parts.toString() === 'https://h:8080/a/b?q=1#f') // re-renders

    console.assert(Uri.fromPath('/x/report.pdf').mimeType().essence === 'application/pdf')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::Uri;

    let parts = Uri::parse_str("https://h:8080/a/b?q=1#f").unwrap().parts();
    assert_eq!(parts.scheme.as_deref(), Some("https"));
    assert_eq!(parts.authority.as_deref(), Some("h:8080"));
    assert_eq!(parts.path, "/a/b");
    assert_eq!(parts.to_string(), "https://h:8080/a/b?q=1#f"); // re-renders

    assert_eq!(Uri::from_path("/x/report.pdf").mime_type().essence(), "application/pdf");
    ```

## Default ports and IPv6 hosts

Two derived helpers answer *"what host and port would this URI actually dial?"* — both
computed **on read**, so they never touch the stored value (its canonical string and bytes
still round-trip unchanged).

- **`port_or_default`** — the explicit `port` if the authority carries one, otherwise the
  scheme's registered default (`https` → 443, `ws` → 80, `postgres` → 5432, …). `default_port`
  exposes just the scheme's default, and the module-level `default_port(scheme)` is the raw
  table (case-insensitive; `None` for a scheme with no registered default, such as `s3`).
- **`host_is_ipv6` / `host_unbracketed`** — the host is stored with its IPv6 brackets
  (`"[::1]"`); `host_unbracketed` strips them (`"::1"`) so the bare address can go straight to
  a socket API, while a reg-name or IPv4 host passes through untouched.

Because the fallback is read-only, `https://h/` and `https://h:443/` stay **distinct** values
(the second wrote the port in) even though both dial port 443.

=== "Python"

    ```python
    from yggdryl.uri import Uri, Url, default_port

    # A scheme's default fills in when no port was written.
    uri = Uri.parse("https://example.com/p")
    assert uri.port is None            # nothing was written
    assert uri.default_port == 443     # the scheme's default
    assert uri.port_or_default == 443  # effective port to connect to
    assert str(uri) == "https://example.com/p"   # read-only: no ":443" added

    # An explicit port wins; a scheme with no default (or none at all) is None.
    assert Uri.parse("https://h:8443/p").port_or_default == 8443
    assert Uri.parse("s3://bucket/key").port_or_default is None
    assert default_port("ws") == 80 and default_port("s3") is None

    # IPv6 hosts: detect and unbracket.
    v6 = Url.parse("https://[2001:db8::1]:8080/p")
    assert v6.host == "[2001:db8::1]"          # stored bracketed
    assert v6.host_is_ipv6
    assert v6.host_unbracketed == "2001:db8::1"  # bare address to dial
    ```

=== "Node"

    ```js
    const { Uri, Url, defaultPort } = require('yggdryl').uri

    // A scheme's default fills in when no port was written.
    const uri = Uri.parse('https://example.com/p')
    console.assert(uri.port === null)            // nothing was written
    console.assert(uri.defaultPort === 443)      // the scheme's default
    console.assert(uri.portOrDefault === 443)    // effective port to connect to
    console.assert(uri.toString() === 'https://example.com/p')   // read-only: no ":443"

    // An explicit port wins; a scheme with no default (or none at all) is null.
    console.assert(Uri.parse('https://h:8443/p').portOrDefault === 8443)
    console.assert(Uri.parse('s3://bucket/key').portOrDefault === null)
    console.assert(defaultPort('ws') === 80 && defaultPort('s3') === null)

    // IPv6 hosts: detect and unbracket.
    const v6 = Url.parse('https://[2001:db8::1]:8080/p')
    console.assert(v6.host === '[2001:db8::1]')          // stored bracketed
    console.assert(v6.hostIsIpv6)
    console.assert(v6.hostUnbracketed === '2001:db8::1') // bare address to dial
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::{default_port, Uri, Url};

    // A scheme's default fills in when no port was written.
    let uri = Uri::parse_str("https://example.com/p").unwrap();
    assert_eq!(uri.port(), None);              // nothing was written
    assert_eq!(uri.default_port(), Some(443)); // the scheme's default
    assert_eq!(uri.port_or_default(), Some(443)); // effective port to connect to
    assert_eq!(uri.to_string(), "https://example.com/p"); // read-only: no ":443" added

    // An explicit port wins; a scheme with no default (or none at all) is None.
    assert_eq!(Uri::parse_str("https://h:8443/p").unwrap().port_or_default(), Some(8443));
    assert_eq!(Uri::parse_str("s3://bucket/key").unwrap().port_or_default(), None);
    assert_eq!(default_port("ws"), Some(80));
    assert_eq!(default_port("s3"), None);

    // IPv6 hosts: detect and unbracket.
    let v6 = Url::parse_str("https://[2001:db8::1]:8080/p").unwrap();
    assert_eq!(v6.host(), Some("[2001:db8::1]"));          // stored bracketed
    assert!(v6.host_is_ipv6());
    assert_eq!(v6.host_unbracketed(), Some("2001:db8::1")); // bare address to dial
    ```

## Building an authority

`Authority` is constructed directly (host required; userinfo and port optional) or as a
bare host. Its constructor is **host-first** in the bindings for ergonomics; the Rust core
takes `(user, password, host, port)`.

=== "Python"

    ```python
    from yggdryl.uri import Authority

    a = Authority("example.com", user="svc", password="secret", port=5432)
    assert str(a) == "svc:secret@example.com:5432"
    assert Authority.from_host("localhost").port is None
    ```

=== "Node"

    ```js
    const { Authority } = require('yggdryl').uri

    const a = new Authority('example.com', 'svc', 'secret', 5432)
    console.assert(a.toString() === 'svc:secret@example.com:5432')
    console.assert(Authority.fromHost('localhost').port === null)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::Authority;

    // Core order is (user, password, host, port).
    let a = Authority::new(Some("svc"), Some("secret"), "example.com", Some(5432));
    assert_eq!(a.to_string(), "svc:secret@example.com:5432");
    assert_eq!(Authority::from_host("localhost").port(), None);
    ```

## Mutators — builder and in-place

Two mutation styles. The **builder** mutators (`with_scheme` … `with_fragment`) return a
new value, so they chain; the **in-place** setters (`set_scheme` … `set_fragment`) mutate
the receiver. Setting a host/port/user/password creates an authority if the URI had none.

=== "Python"

    ```python
    from yggdryl.uri import Uri

    # Builder: each `with_*` returns a fresh Uri.
    built = (
        Uri.from_path("/v1/data")
        .with_scheme("https")
        .with_host("api.example.com")
        .with_port(443)
        .with_query("page=2")
    )
    assert str(built) == "https://api.example.com:443/v1/data?page=2"

    # In-place: `set_*` mutates the receiver.
    u = Uri.parse("https://old.example.com/x")
    u.set_host("new.example.com")
    u.set_fragment("section")
    assert u.host == "new.example.com"
    assert str(u) == "https://new.example.com/x#section"
    ```

=== "Node"

    ```js
    const { Uri } = require('yggdryl').uri

    // Builder: each with* returns a fresh Uri.
    const built = Uri.fromPath('/v1/data')
      .withScheme('https')
      .withHost('api.example.com')
      .withPort(443)
      .withQuery('page=2')
    console.assert(built.toString() === 'https://api.example.com:443/v1/data?page=2')

    // In-place: set* mutates the receiver.
    const u = Uri.parse('https://old.example.com/x')
    u.setHost('new.example.com')
    u.setFragment('section')
    console.assert(u.host === 'new.example.com')
    console.assert(u.toString() === 'https://new.example.com/x#section')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::Uri;

    // Builder: each `with_*` consumes and returns the Uri.
    let built = Uri::from_path("/v1/data")
        .with_scheme("https")
        .with_host("api.example.com")
        .with_port(443)
        .with_query("page=2");
    assert_eq!(built.to_string(), "https://api.example.com:443/v1/data?page=2");

    // In-place: `set_*` mutates the receiver.
    let mut u = Uri::parse_str("https://old.example.com/x").unwrap();
    u.set_host("new.example.com");
    u.set_fragment("section");
    assert_eq!(u.host(), Some("new.example.com"));
    assert_eq!(u.to_string(), "https://new.example.com/x#section");
    ```

## Joining and combining

These combinators build new values from existing ones — all return a copy, so they chain
and never mutate the receiver. They run easy → complex: join or walk the path, overlay whole
components, then clone.

- **`joinpath(segment)`** — joins a segment onto the path, *lexically* (like `pathlib`, no
  `.`/`..` resolution). It gets the seam right: exactly one `/` between base and segment (a
  trailing slash is never doubled), an **absolute** segment (leading `/`) replaces the path,
  an empty segment is a no-op, and under an authority the result stays rooted so a relative
  segment can't fuse into the host. The segment is percent-encoded like `set_path`; the
  query and fragment are left untouched.
- **`parent()` / `parents()`** — the **inverse of `joinpath`**: `parent()` returns this value
  with its last path segment stripped (only the path changes — scheme, authority, query, and
  fragment are kept), or nothing at a **root** (no segment left to strip), so
  `base.joinpath("x").parent()` addresses `base` again. `parents()` walks the ancestor chain
  nearest-first, up to that root. Python returns `parent()` as `Uri | None` and `parents()` as
  a **list**; Node as `Uri | null` and an **array**; Rust as an `Option` and an **iterator**.
- **`merge_with(other)`** — overlays `other` onto this value: each component `other` sets (a
  present scheme/authority/query/fragment, or a non-empty path) wins, otherwise this value's
  is kept. A mechanical component merge with no re-parsing — ideal for applying a small patch
  over a base. `Authority.merge_with` overlays at the field level (patch just the port, say).
- **`copy()`** — an explicit clone, the cross-language name for "duplicate this value".

An [`Authority`] can also be built up with `with_user` / `with_password` / `with_host` /
`with_port` and attached with `with_authority`.

=== "Python"

    ```python
    from yggdryl.uri import Uri, Url, Authority

    # joinpath: one slash at the seam, an absolute segment resets, multi-segment is fine.
    base = Uri.parse("https://api.example.com/v1")
    assert str(base.joinpath("users").joinpath("42")) == "https://api.example.com/v1/users/42"
    assert Uri.from_path("/v1/").joinpath("users").path == "/v1/users"   # not doubled
    assert str(base.joinpath("/reset")) == "https://api.example.com/reset"

    # parent / parents: the inverse of joinpath — walk back up the path.
    assert base.joinpath("x").parent() == base            # strips the joined segment
    file = Uri.parse("https://h/a/b/c.txt?q=1")
    assert file.parent().path == "/a/b"                   # scheme/query kept
    assert str(file.parent()) == "https://h/a/b?q=1"
    assert [p.path for p in file.parents()] == ["/a/b", "/a", ""]  # nearest-first, up to the root
    assert Uri.parse("https://h").parent() is None        # a root has no parent
    # Url mirrors it, keeping the scheme.
    assert Url.parse("https://h/a/b/c.txt").parent() == Url.parse("https://h/a/b")

    # merge_with: apply only the fields the patch sets.
    prod = Uri.parse("https://prod.example.com/v1?trace=1")
    assert str(prod.merge_with(Uri.parse("//staging.example.com"))) \
        == "https://staging.example.com/v1?trace=1"

    # copy is an independent clone; build an authority and attach it.
    auth = Authority.from_host("db.internal").with_user("svc").with_port(5432)
    dsn = Uri.from_path("").with_scheme("postgres").with_authority(auth).with_path("/app")
    assert str(dsn) == "postgres://svc@db.internal:5432/app"
    ```

=== "Node"

    ```js
    const { Uri, Url, Authority } = require('yggdryl').uri

    // joinpath: one slash at the seam, an absolute segment resets, multi-segment is fine.
    const base = Uri.parse('https://api.example.com/v1')
    console.assert(base.joinpath('users').joinpath('42').toString() === 'https://api.example.com/v1/users/42')
    console.assert(Uri.fromPath('/v1/').joinpath('users').path === '/v1/users')   // not doubled
    console.assert(base.joinpath('/reset').toString() === 'https://api.example.com/reset')

    // parent / parents: the inverse of joinpath — walk back up the path.
    console.assert(base.joinpath('x').parent().equals(base))       // strips the joined segment
    const file = Uri.parse('https://h/a/b/c.txt?q=1')
    console.assert(file.parent().path === '/a/b')                  // scheme/query kept
    console.assert(file.parent().toString() === 'https://h/a/b?q=1')
    console.assert(JSON.stringify(file.parents().map(p => p.path)) === '["/a/b","/a",""]')  // nearest-first
    console.assert(Uri.parse('https://h').parent() === null)       // a root has no parent
    // Url mirrors it, keeping the scheme.
    console.assert(Url.parse('https://h/a/b/c.txt').parent().equals(Url.parse('https://h/a/b')))

    // mergeWith: apply only the fields the patch sets.
    const prod = Uri.parse('https://prod.example.com/v1?trace=1')
    console.assert(prod.mergeWith(Uri.parse('//staging.example.com')).toString()
      === 'https://staging.example.com/v1?trace=1')

    // copy is an independent clone; build an authority and attach it.
    const auth = Authority.fromHost('db.internal').withUser('svc').withPort(5432)
    const dsn = Uri.fromPath('').withScheme('postgres').withAuthority(auth).withPath('/app')
    console.assert(dsn.toString() === 'postgres://svc@db.internal:5432/app')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::{Authority, Uri, Url};

    // joinpath: one slash at the seam, an absolute segment resets, multi-segment is fine.
    let base = Uri::parse_str("https://api.example.com/v1").unwrap();
    assert_eq!(base.joinpath("users").joinpath("42").to_string(), "https://api.example.com/v1/users/42");
    assert_eq!(Uri::from_path("/v1/").joinpath("users").path(), "/v1/users");   // not doubled
    assert_eq!(base.joinpath("/reset").to_string(), "https://api.example.com/reset");

    // parent / parents: the inverse of joinpath — walk back up the path.
    assert_eq!(base.joinpath("x").parent().unwrap(), base);        // strips the joined segment
    let file = Uri::parse_str("https://h/a/b/c.txt?q=1").unwrap();
    assert_eq!(file.parent().unwrap().path(), "/a/b");            // scheme/query kept
    assert_eq!(file.parent().unwrap().to_string(), "https://h/a/b?q=1");
    let ancestors: Vec<String> = file.parents().map(|p| p.path().to_string()).collect();
    assert_eq!(ancestors, vec!["/a/b", "/a", ""]);               // nearest-first, up to the root
    assert!(Uri::parse_str("https://h").unwrap().parent().is_none()); // a root has no parent
    // Url mirrors it, keeping the scheme.
    assert_eq!(Url::parse_str("https://h/a/b/c.txt").unwrap().parent().unwrap().to_string(), "https://h/a/b");

    // merge_with: apply only the fields the patch sets.
    let prod = Uri::parse_str("https://prod.example.com/v1?trace=1").unwrap();
    assert_eq!(
        prod.merge_with(&Uri::parse_str("//staging.example.com").unwrap()).to_string(),
        "https://staging.example.com/v1?trace=1",
    );

    // copy is an independent clone; build an authority and attach it.
    let auth = Authority::from_host("db.internal").with_user(Some("svc")).with_port(Some(5432));
    let dsn = Uri::default().with_scheme("postgres").with_authority(Some(auth)).with_path("/app");
    assert_eq!(dsn.to_string(), "postgres://svc@db.internal:5432/app");
    ```

## Params — the query as a map

**`query` names the raw string; `params` names the map.** `query()` / `set_query()` read and
write the query **string** verbatim, while the `param*` family exposes it as an ordered **map**
with full CRUD — in Python also through the dict protocol (`uri["key"]`, `uri["key"] = value`,
`del uri["key"]`, `"key" in uri`). Writes **percent-encode** keys and values for storage — a
value containing `&`, `=`, `#`, or a space is stored safely — and rebuild the query in a single
pre-sized allocation. Reads return the **decoded** value by default (zero-copy when there is
nothing to decode); pass `encoded=True` (Node: a `true` second argument) for the raw stored
form. Components are stored encoded generally: `set_path`, `set_query`, `set_fragment`,
`set_user`, `set_password`, and `from_path` encode too, while `parse` trusts its
already-encoded input.

- **Read** — `param(key)` (first value), `param_all(key)` (every value of a
  repeated key), `params()` (the **grouped map** in first-appearance key order — each key
  mapped to **all** its values, so a repeated key like `?a=1&a=3` round-trips as one entry rather
  than colliding: a `dict[str, tuple[str, …]]` in Python, an ordered `Map<string, string[]>` in
  Node), `has_param(key)`. In the Rust core the grouped view is `params_grouped()`
  (`Vec<(&str, Vec<&str>)>`); `params()` there returns the raw ordered `(key, value)` pairs.
- **Create / update** — `set_param(key, value)` updates the first occurrence in place,
  drops later duplicates, or appends when absent (creating the query if there was none);
  `with_param` is the chainable builder form.
- **Delete** — `remove_param(key)` drops every occurrence (returning whether any were);
  `without_param` is the builder form. Removing the last parameter clears the query.
- **Bulk update** — `set_params(pairs)` applies many `(key, value)` updates in a single
  rebuild (last value wins per key), far cheaper than a loop of `set_param`;
  `with_params` is the builder form.
- **Normalize** — `normalize_params()` drops empty tokens and **stable-sorts** parameters by
  key (repeated keys preserved, not merged); `with_normalized_params` is the builder form.

=== "Python"

    ```python
    from yggdryl.uri import Uri

    uri = Uri.parse("http://h/p?a=1&b=2&a=3")
    assert uri.param("a") == "1"                      # first occurrence wins
    assert uri.param_all("a") == ["1", "3"]           # every value, in order
    assert uri.params() == {"a": ("1", "3"), "b": ("2",)}  # grouped: key -> tuple of values

    uri.set_param("a", "9")                           # update (later dupes dropped)
    uri.set_param("c", "7")                           # create (appended)
    uri["d"] = "4"                                    # dict protocol: set
    assert uri["d"] == "4" and "d" in uri             # dict protocol: get / contains
    del uri["d"]                                      # dict protocol: delete
    assert uri.query == "a=9&b=2&c=7"
    assert uri.remove_param("b") is True              # delete
    assert uri.query == "a=9&c=7"

    chained = Uri.parse("http://h/p").with_param("x", "1").without_param("x")
    assert chained.query is None
    ```

=== "Node"

    ```js
    const { Uri } = require('yggdryl').uri

    const uri = Uri.parse('http://h/p?a=1&b=2&a=3')
    console.assert(uri.param('a') === '1')                      // first occurrence wins
    console.assert(JSON.stringify(uri.paramAll('a')) === '["1","3"]')
    // grouped: an ordered Map<string, string[]> (key -> all its values, first-appearance order)
    const qp = uri.params()
    console.assert(JSON.stringify([...qp]) === '[["a",["1","3"]],["b",["2"]]]')

    uri.setParam('a', '9')                                      // update
    uri.setParam('c', '7')                                      // create (appended)
    console.assert(uri.query === 'a=9&b=2&c=7')
    console.assert(uri.removeParam('b') === true)               // delete
    console.assert(uri.query === 'a=9&c=7')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::Uri;

    let mut uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    assert_eq!(uri.param("a"), Some("1"));            // first occurrence wins
    assert_eq!(uri.param_all("a"), vec!["1", "3"]);   // every value, in order
    assert_eq!(uri.params(), vec![("a", "1"), ("b", "2"), ("a", "3")]); // raw pairs
    assert_eq!( // grouped: key -> all values (what the bindings' params returns)
        uri.params_grouped(),
        vec![("a", vec!["1", "3"]), ("b", vec!["2"])],
    );

    uri.set_param("a", "9");                          // update (later dupes dropped)
    uri.set_param("c", "7");                          // create (appended)
    assert_eq!(uri.query(), Some("a=9&b=2&c=7"));
    assert!(uri.remove_param("b"));                   // delete
    assert_eq!(uri.query(), Some("a=9&c=7"));
    ```

Bulk update then normalize:

=== "Python"

    ```python
    from yggdryl.uri import Uri

    uri = Uri.parse("http://h/p?c=3&a=1&b=2")
    uri.set_params([("a", "9"), ("d", "4")])   # bulk: a updated, d appended
    assert uri.query == "c=3&a=9&b=2&d=4"
    uri.set_params(list({"e": "5"}.items()))   # a dict via .items()
    uri.normalize_params()                            # sort by key, drop empties
    assert uri.query == "a=9&b=2&c=3&d=4&e=5"
    ```

=== "Node"

    ```js
    const { Uri } = require('yggdryl').uri

    const uri = Uri.parse('http://h/p?c=3&a=1&b=2')
    uri.setParams([['a', '9'], ['d', '4']])          // bulk: a updated, d appended
    console.assert(uri.query === 'c=3&a=9&b=2&d=4')
    uri.setParams(Object.entries({ e: '5' }))        // an object via Object.entries
    uri.normalizeParams()                                  // sort by key, drop empties
    console.assert(uri.query === 'a=9&b=2&c=3&d=4&e=5')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::Uri;

    let mut uri = Uri::parse_str("http://h/p?c=3&a=1&b=2").unwrap();
    uri.set_params(&[("a", "9"), ("d", "4")]);   // bulk: a updated, d appended
    assert_eq!(uri.query(), Some("c=3&a=9&b=2&d=4"));
    uri.normalize_params();                             // sort by key, drop empties
    assert_eq!(uri.query(), Some("a=9&b=2&c=3&d=4"));
    ```

Percent-encoding — values are stored encoded and decoded on read:

=== "Python"

    ```python
    from yggdryl.uri import Uri

    uri = Uri.parse("http://h/p").with_param("q", "a b&c")
    assert uri.query == "q=a%20b%26c"                    # stored percent-encoded
    assert uri.param("q") == "a b&c"               # decoded by default
    assert uri.param("q", encoded=True) == "a%20b%26c"   # raw stored form
    assert uri.params() == {"q": ("a%20b%26c",)}   # grouped map: stored (encoded) tuples
    ```

=== "Node"

    ```js
    const { Uri } = require('yggdryl').uri

    const uri = Uri.parse('http://h/p').withParam('q', 'a b&c')
    console.assert(uri.query === 'q=a%20b%26c')                 // stored percent-encoded
    console.assert(uri.param('q') === 'a b&c')            // decoded by default
    console.assert(uri.param('q', true) === 'a%20b%26c')  // raw stored form
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::Uri;

    let uri = Uri::parse_str("http://h/p").unwrap().with_param("q", "a b&c");
    assert_eq!(uri.query(), Some("q=a%20b%26c"));                     // stored encoded
    assert_eq!(uri.param("q"), Some("a%20b%26c"));             // stored form
    assert_eq!(uri.param_decoded("q").as_deref(), Some("a b&c")); // decoded
    ```

## Windows → POSIX path normalization

Paths are standardized to **POSIX forward slashes**: a Windows drive path (`C:\…`), a UNC
path (`\\server\share`), or any back-slashed input has every `\` rewritten to `/` on the
way in. A single ASCII letter + `:` + slash is a **drive letter kept in the path**, never a
one-letter scheme — so `C:\Users\x\a.tar.gz` parses as a scheme-less path, and examples
that need a real scheme use a multi-letter one.

=== "Python"

    ```python
    from yggdryl.uri import Uri

    drive = Uri.parse(r"C:\Users\x\archive.tar.gz")
    assert drive.scheme is None                     # drive letter, not a scheme
    assert drive.path == "C:/Users/x/archive.tar.gz"
    assert drive.extensions == ["tar", "gz"]

    assert Uri.parse(r"\\server\share\file.txt").path == "//server/share/file.txt"  # UNC
    assert Uri.from_path(r"a\b\c").path == "a/b/c"
    ```

=== "Node"

    ```js
    const { Uri } = require('yggdryl').uri

    const drive = Uri.parse('C:\\Users\\x\\archive.tar.gz')
    console.assert(drive.scheme === null)           // drive letter, not a scheme
    console.assert(drive.path === 'C:/Users/x/archive.tar.gz')
    console.assert(JSON.stringify(drive.extensions) === '["tar","gz"]')

    console.assert(Uri.parse('\\\\server\\share\\file.txt').path === '//server/share/file.txt')  // UNC
    console.assert(Uri.fromPath('a\\b\\c').path === 'a/b/c')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::Uri;

    let drive = Uri::parse_str(r"C:\Users\x\archive.tar.gz").unwrap();
    assert_eq!(drive.scheme(), None);               // drive letter, not a scheme
    assert_eq!(drive.path(), "C:/Users/x/archive.tar.gz");
    assert_eq!(drive.extensions(), vec!["tar", "gz"]);

    assert_eq!(Uri::parse_str(r"\\server\share\file.txt").unwrap().path(), "//server/share/file.txt");
    assert_eq!(Uri::from_path(r"a\b\c").path(), "a/b/c");
    ```

## Value semantics and the byte codec

Both `Uri` and `Url` round-trip through bytes — `serialize_bytes()` is the canonical
string's UTF-8, and `deserialize_bytes(bytes)` is its exact inverse. Equality and hashing
follow the same canonical string, so two values are equal iff their bytes are equal.
(`Authority` compares and hashes by its canonical string too, and serializes to those
same canonical bytes — it
pickles through its four components in Python.)

=== "Python"

    ```python
    import pickle
    from yggdryl.uri import Uri, Url

    uri = Uri.parse("sc://host/path?q#f")
    assert uri.serialize_bytes() == b"sc://host/path?q#f"
    assert Uri.deserialize_bytes(uri.serialize_bytes()) == uri
    assert pickle.loads(pickle.dumps(uri)) == uri     # __reduce__ round-trips

    # Value semantics: equal values dedup in a set.
    assert Url.parse("https://h/a") == Url.parse("https://h/a")
    assert len({Uri.parse("http://h/x"), Uri.parse("http://h/x")}) == 1

    # A non-UTF-8 payload raises a guided error.
    try:
        Uri.deserialize_bytes(bytes([0xff, 0xfe]))
    except ValueError as error:
        assert "utf" in str(error).lower()
    ```

=== "Node"

    ```js
    const { Uri, Url } = require('yggdryl').uri

    const uri = Uri.parse('sc://host/path?q#f')
    console.assert(uri.serializeBytes().equals(Buffer.from('sc://host/path?q#f')))
    console.assert(Uri.deserializeBytes(uri.serializeBytes()).equals(uri))

    // Value semantics: equal values agree on equals() and hashCode().
    console.assert(Url.parse('https://h/a').equals(Url.parse('https://h/a')))
    console.assert(Uri.parse('http://h/x').hashCode() === Uri.parse('http://h/x').hashCode())

    // A non-UTF-8 payload throws a guided error.
    try {
      Uri.deserializeBytes(Buffer.from([0xff, 0xfe]))
    } catch (error) {
      console.assert(/utf/i.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::{Uri, Url};

    let uri = Uri::parse_str("sc://host/path?q#f").unwrap();
    assert_eq!(uri.serialize_bytes(), b"sc://host/path?q#f");
    assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);

    // Value semantics.
    assert_eq!(Url::parse_str("https://h/a").unwrap(), Url::parse_str("https://h/a").unwrap());

    // A non-UTF-8 payload is a guided error.
    assert!(Uri::deserialize_bytes(&[0xff, 0xfe]).is_err());
    ```

## Portable addresses (relocatable pickling)

A `file://` address is machine-specific — a path under your home or temp directory need not
exist on another host. `to_portable_str()` rewrites such an address relative to a well-known
root: one under the **current user's home** directory folds to a leading `~`, and one under the
**system temp** directory folds to a leading `$TMP` (the **longest** matching root wins, so a
temp dir nested under home — as on Windows — still folds to `$TMP`). Every other URI — a
different scheme, or a file path outside both roots — is its exact string. `from_portable_str(s)`
is the exact inverse: it expands `~` / `$TMP` against **this** environment's home / temp roots,
so a URI addressing `~/data` on one machine reconstructs under another machine's home. In Python
this is the form `Uri` / `Url` pickling reduces through, so `pickle.loads(pickle.dumps(uri))`
relocates the same way. `Url` mirrors both methods.

=== "Python"

    ```python
    import pickle
    from yggdryl.uri import Uri, Url

    # A file:// address under HOME folds to a leading ~, and round-trips through it.
    home_file = Uri.from_portable_str("~/data/report.csv")   # expands ~ against this machine's home
    assert home_file.scheme == "file"
    assert home_file.to_portable_str() == "~/data/report.csv"    # folds back to ~

    # A temp path folds to $TMP (the longest matching root wins).
    tmp_file = Uri.from_portable_str("$TMP/cache.bin")
    assert tmp_file.to_portable_str() == "$TMP/cache.bin"

    # Every other URI is its exact string — nothing to relocate.
    assert Uri.parse("https://h/p").to_portable_str() == "https://h/p"
    assert Url.parse("https://h/p").to_portable_str() == "https://h/p"   # Url mirrors it

    # Pickling reduces through the portable form, so a home path reconstructs
    # under the receiving machine's home.
    assert pickle.loads(pickle.dumps(home_file)) == home_file
    ```

=== "Node"

    ```js
    const { Uri, Url } = require('yggdryl').uri

    // A file:// address under HOME folds to a leading ~, and round-trips through it.
    const homeFile = Uri.fromPortableString('~/data/report.csv')   // expands ~ against this machine's home
    console.assert(homeFile.scheme === 'file')
    console.assert(homeFile.toPortableString() === '~/data/report.csv')   // folds back to ~

    // A temp path folds to $TMP (the longest matching root wins).
    const tmpFile = Uri.fromPortableString('$TMP/cache.bin')
    console.assert(tmpFile.toPortableString() === '$TMP/cache.bin')

    // Every other URI is its exact string — nothing to relocate.
    console.assert(Uri.parse('https://h/p').toPortableString() === 'https://h/p')
    console.assert(Url.parse('https://h/p').toPortableString() === 'https://h/p')  // Url mirrors it
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::{Uri, Url};

    // A file:// address under HOME folds to a leading ~, and round-trips through it.
    let home_file = Uri::from_portable_str("~/data/report.csv").unwrap();
    assert_eq!(home_file.scheme(), Some("file"));
    assert_eq!(home_file.to_portable_str(), "~/data/report.csv"); // folds back to ~

    // A temp path folds to $TMP (the longest matching root wins).
    let tmp_file = Uri::from_portable_str("$TMP/cache.bin").unwrap();
    assert_eq!(tmp_file.to_portable_str(), "$TMP/cache.bin");

    // Every other URI is its exact string — nothing to relocate.
    assert_eq!(Uri::parse_str("https://h/p").unwrap().to_portable_str(), "https://h/p");
    assert_eq!(Url::parse_str("https://h/p").unwrap().to_portable_str(), "https://h/p"); // Url mirrors it

    // A local file path can be built straight from disk, then relocated.
    let f = Uri::from_file_path("/tmp/out.log");
    assert_eq!(Uri::from_portable_str(&f.to_portable_str()).unwrap(), f);
    ```

## Guided errors

A malformed scheme, an out-of-range port, non-UTF-8 bytes, or a scheme-less string handed
to `Url` all raise the same guided message across the three languages (`ValueError` in
Python, a thrown `Error` in Node).

=== "Python"

    ```python
    from yggdryl.uri import Uri, Url

    try:
        Uri.parse("https://host:99999/")     # port out of range
    except ValueError as error:
        assert "99999" in str(error)

    try:
        Url.parse("/no/scheme")              # not absolute
    except ValueError as error:
        assert "absolute" in str(error)
    ```

=== "Node"

    ```js
    const { Uri, Url } = require('yggdryl').uri

    try {
      Uri.parse('https://host:99999/')       // port out of range
    } catch (error) {
      console.assert(/99999/.test(error.message))
    }

    try {
      Url.parse('/no/scheme')                // not absolute
    } catch (error) {
      console.assert(/absolute/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_core::uri::{Uri, Url};

    assert!(Uri::parse_str("https://host:99999/").is_err());   // port out of range
    assert!(Url::parse_str("/no/scheme").is_err());            // not absolute
    ```
