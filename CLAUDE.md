# yggdryl — contributor & agent instructions

**Keep all new code uniform with the existing patterns.** Before adding anything,
read the nearest existing example and mirror its structure, naming, error
handling, and doc style. Consistency across the Rust core and the two bindings is
the top priority — a reader should not be able to tell which type they are
looking at from the shape of the code.

## Architecture

- `crates/yggdryl/` — the pure-Rust core. **All logic and the canonical tests
  live here.** It has no dependencies.
- `bindings/python/` (PyO3/maturin) and `bindings/node/` (napi-rs) are **thin
  wrappers**. They only translate types/errors and call the core; they contain no
  logic. Anything added to the core must be surfaced in *both* bindings.

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
| Read query parameters | `params()` → `map<str, list<str>>` |
| Replace the whole query | `with_params(map)` |
| Add/replace one parameter | `add_param(key, values)` |

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
cargo test -p yggdryl
(cd bindings/python && maturin develop && pytest)
(cd bindings/node && npm run build && npm test)
```

All five must pass. Do not bump the version while the base is still being built.
