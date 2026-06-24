# yggdryl — contributor & agent instructions

**Keep all new code uniform with the existing patterns.** Before adding anything,
read the nearest existing example and mirror its structure, naming, error
handling, and doc style. Consistency across the Rust core and the two bindings is
the top priority — a reader should not be able to tell which type they are
looking at from the shape of the code.

## Architecture

- `crates/yggdryl-core/` — dependency-free foundations: the `FromInput` /
  `ToOutput` traits and percent-encoding.
- `crates/yggdryl-version/` — the standalone `Version` type.
- `crates/yggdryl-media/` — the `MimeType` enum (single MIME types, backed by a
  mutable global registry of extensions/magic bytes) and the `MediaType` stack
  (an ordered `Vec<MimeType>`, e.g. `csv.gz` → `[Csv, Gzip]`). **All media-type
  logic lives here.**
- `crates/yggdryl-url/` — the `Uri`/`Url` types and the canonical URL tests, built
  on and re-exporting `yggdryl-core` (and `yggdryl-media` for the inferred
  `media_type()` accessor). **All URL logic lives here.**
- `bindings/python/` (PyO3/maturin) and `bindings/node/` (napi-rs) are **thin
  wrappers**. They only translate types/errors and call the core; they contain no
  logic. Anything added to the core must be surfaced in *both* bindings.

### One module per type, everywhere

Code is organised the same way in every language: **one file per type**, with a
small glue file tying them together. Don't grow a single big file.

- Rust: one crate per concern (`yggdryl-core`, `yggdryl-version`,
  `yggdryl-media`, `yggdryl-url`).
- Each binding: `src/uri.rs`, `src/url.rs`, `src/version.rs`, `src/mime.rs`,
  `src/media.rs` per type, with
  `src/lib.rs` holding only shared helpers (error conversion, `hash_str`,
  percent-encoding free functions) and the module registration. Per-type
  wrappers keep their `inner` field `pub(crate)` so sibling modules can convert.

### Cross-language replication rule

The Rust core is the source of truth, but the languages move together. **When you
add or change behaviour in Rust, immediately replicate it in the Python and Node
extensions; when you change an extension, fold the behaviour back into the Rust
core and the other extension.** Adapt to each language's idioms (Python dunders /
keyword defaults, JS camelCase / `Option<bool>` defaults) but keep the surface
and semantics identical, so the three codebases stay coherent and a change is
never half-applied.

## Naming conventions (cross-language)

These names are identical in Rust, Python and JS (JS uses camelCase):

| Concept | Name |
| --- | --- |
| Construct from a string | `from_str(value, safe)` |
| Construct from a component mapping | `from_mapping(fields, safe)` |
| Construct from any supported input | `from_` (Rust trait `FromInput`) |
| Construct from explicit parts | `from_parts(...)` |
| Independent / overriding copy | `copy(...)` — every field optional, omitted fields come from `self` |
| Single-field functional update | `with_<field>(value)` returns a new value |
| Clear an optional field | `without_<field>()` |
| Read query parameters | `params(decode=true)` → `map<str, list<str>>` |
| Replace the whole query | `with_params(map, encode=true)` |
| Add/replace one parameter | `add_param(key, values, encode=true)` |
| Query-param CRUD | `get_param` / `set_param` / `set_params` (bulk) / `remove_param` / `remove_params` (bulk) / `clear_params` |
| Scheme split (`https+zip`) | `scheme_base()` / `scheme_ext()` |
| Type conversions | `to_uri` / `from_uri` / `to_url` / `from_url` |
| Single MIME type | `MimeType` enum; `from_str` / `from_mapping` / `from_extension(ext)` / `from_magic(bytes)` / `from_path(path)`; `.mime` / `type` / `subtype` / `extension(s)` |
| Global MIME registry | `MimeType.register(mime, extensions, magic)` / `unregister(mime)` / `reset_registry()` |
| Layered media type (extension stack) | `MediaType` = ordered `[MimeType, …]`; `from_str` / `from_mapping` / `from_extension` / `from_extensions` / `from_path`; `.types` / `first` / `last` |
| Inferred media/MIME type on a URI/URL | `media_type()` → `MediaType` stack or null; `mime_type()` → outermost `MimeType` or null (Rust also has `MediaType::from(&uri)`) |
| Octet-stream fallback | `MimeType.default()` = `application/octet-stream`; `MediaType.default()` = `[OctetStream]` (Rust `Default`, so `from_*(...).unwrap_or_default()`) |

Rules:
- Parsing entry points are `from_*`, never `parse*` (the public API does not use
  the word "parse").
- Every `from_*` takes a `safe` boolean: `true` = full validation, `false` =
  fast, lenient parse. Bindings default `safe` to `true`.
- `with_*` / `without_*` / `copy` are **non-mutating** and return a new value.
- URL-safe `percent_encode` / `percent_decode` are the only encoding helpers;
  modifiers that build query strings percent-encode their inputs.

## Patterns to mirror

- **Errors**: one `enum` per type (`UriError`, `UrlError`, …) implementing
  `Display` + `std::error::Error`, with `From` conversions between layers. Core
  errors map to `ValueError` (Python) / thrown `Error` (Node).
- **Docs**: every public item has a `///` doc comment; types carry a runnable
  doctest. Match the existing terse style.
- **Bindings**: each wrapper method is one or two lines delegating to
  `self.inner`. Use `#[pyo3(signature = ...)]` / napi `Option<T>` for defaults.

## Required checks before committing

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
(cd bindings/python && maturin develop && pytest)
(cd bindings/node && npm run build && npm test)
```

All five must pass. Do not bump the version while the base is still being built.

## Code-coherence review (after every implementation)

Once the change compiles and the checks pass, do a final coherence pass before
committing — treat it as a required step, not an optional polish:

1. **No redundancy** — fold duplicated logic into one place; a new `from_*`
   should delegate to an existing one (e.g. `from_extension` → `from_extensions`
   → `from_path`) rather than re-implement it. Don't add a second API that
   merely restates an existing one.
2. **Cross-language parity** — the same surface and semantics exist in the Rust
   core and *both* bindings (adapting only to each language's idioms); a change
   is never half-applied.
3. **One concern per file/type** — the new code lives in the right crate/module
   and mirrors the structure of its neighbours (naming, error handling, doc
   style, terseness).
4. **Readability** — names match the conventions table, every public item has a
   `///` doc, and a reader cannot tell which type they are looking at from the
   shape of the code.

If any point fails, fix it before committing.
