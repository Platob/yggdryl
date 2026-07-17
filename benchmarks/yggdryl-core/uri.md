# `io::Uri` — benchmark & optimization history

Time **and** memory for the URI base types, in all three languages. Every harness is
dependency-free and finishes in **1–3 s**, so it doubles as fast performance validation.
Allocation *counts* are build-independent (same in debug and release), which is why the
Rust harness — and the deterministic `uri_alloc` test — assert them as a regression guard;
wall-clock is release-only and reported, not asserted.

## Run

```bash
cargo bench -p yggdryl-core --bench uri                 # Rust: Mops/s + allocs/op + bytes/op
cargo test  -p yggdryl-core --test uri_alloc            # deterministic memory budgets (ms)
(cd bindings/python && uv run maturin develop --release && uv run python benchmarks/bench_uri.py)
(cd bindings/node   && npm run build && node --expose-gc benchmark/uri.bench.js)
```

## Rust core (release, counting global allocator)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `Uri::parse` (URL corpus) | 2.50 | 4.18 | 42.1 |
| `Uri::from_path` (Windows corpus) | 16.58 | 1.00 | 30.2 |
| `serialize_bytes` | 6.39 | **1.00** | 47.7 |
| `deserialize_bytes` | 2.26 | 4.18 | 42.1 |
| `serialize + deserialize` | 1.54 | 5.18 | 89.8 |
| accessors (scheme/host/path/name) | 20.66 | **0.00** | 0.0 |
| endpoint (`port_or_default`/host) | 27.27 | **0.00** | 0.0 |
| `copy` (clone) | 3.29 | 4.00 | 41.0 |
| `joinpath` (append segment) | 2.41 | **5.00** | 65.2 |
| `merge_with` (overlay) | 3.07 | 4.00 | 41.0 |
| `to_string` (Display) | 3.64 | 3.36 | 119.4 |
| `HashMap` lookup (`Uri` key) | 2.08 | **2.00** | 95.5 |
| `param` (read, first) | 13.62 | **0.00** | 0.0 |
| `params` (map view) | 5.88 | **1.00** | 160.0 |
| `param_decoded` (clean) | 10.61 | **0.00** | 0.0 |
| `set_param` (update) | 6.91 | **1.00** | 23.0 |
| `set_params` (bulk ×3) | 2.82 | **3.00** | 130.0 |
| `normalize_params` (sort+clean) | 3.34 | **2.00** | 99.0 |

`parse` allocates one `String` per present component (scheme/host/path/query/fragment/…);
that is inherent to an owning split. The accessors return borrows, so they allocate
nothing — the zero-copy hand-off the design promises.

The query-parameter map (`param` / `param_all` / `params` /
`has_param` + `set`/`with`/`remove`/`without`) keeps that discipline: reads are
**zero-copy** views into the raw query, the `Vec` views pre-size to **one** allocation, and
a write rebuilds the query in **one** pre-sized allocation (an absent-key removal is a
0-alloc no-op). The `uri_alloc` test asserts each of these budgets.

The combinators hold the line too: `copy` is a plain clone (one allocation per present
component); `merge_with` overlays components with **no re-parse**, so it allocates exactly
like a copy; and `joinpath` adds **exactly one** allocation over that clone — the single
pre-sized joined path. That last budget is what pins optimization **3** below — back-slash
normalization now *borrows* the clean POSIX case, so joining a clean segment never spends a
throwaway `String`. `uri_alloc` asserts `joinpath == copy + 1` and `merge_with == copy`
directly.

## Bindings (release)

| | Python (vs `urllib`) | Node (vs WHATWG `URL`) |
|--|--|--|
| `parse` | 1.23 Mops/s — **7.6× urllib** | 0.41 Mops/s — 0.35× `URL` |
| `serialize` | 3.53 Mops/s | 0.42 Mops/s |
| bytes / parsed object | 208.6 (tracemalloc peak) | 112.3 (V8 heapUsed) |

Python beats pure-Python `urllib`; Node's built-in `URL` is native C++, so for an
operation this cheap the napi FFI crossing dominates — the wrapper is thin and the number
is honest, not a defect. The memory columns confirm the wrappers add no runaway allocation.

## Optimization history

**1 — pre-sized canonical buffer (`serialize_bytes`, `Eq`).** `serialize_bytes` was
`self.to_string().into_bytes()`; `to_string` grows a `String` from empty, so a long URI
reallocated several times. Building into a buffer pre-sized by an `encoded_len` upper bound
makes it allocate **exactly once**.

| | before | after |
|--|-------:|------:|
| `serialize_bytes` allocs/op | 3.36 | **1.00** |
| `serialize_bytes` Mops/s | 3.37 | **6.39** (1.9×) |

**2 — streaming, allocation-free `Hash`.** `Hash` was `self.to_string().hash(state)` — a
`String` per hash. It now streams the canonical rendering straight into the hasher via a
zero-alloc `fmt::Write` adapter (`HashWrite`) plus a `0xff` terminator, reproducing the
canonical string's own hash exactly. `Uri` as a `HashMap` key:

| | before | after |
|--|-------:|------:|
| `HashMap` lookup allocs/op | 10.09 | **2.00** (5×) |
| `HashMap` lookup Mops/s | 1.03 | **2.08** (2×) |

Both preserve value semantics (equal iff canonical strings equal): all core URI tests
(unit + integration + edge + doctests) and the `uri_alloc` budgets stay green. The residual
2 allocs/op on lookup are `Eq`'s two pre-sized canonical strings — string identity is
required so a password with no user and `user = Some("")` still compare equal, which
component-wise equality would break.

**3 — `Cow` back-slash normalization (`joinpath`, `from_path`, `parse`).** `normalize_slashes`
returned a fresh `String` unconditionally (`path.replace('\\', "/")`), so it allocated even
for a POSIX path with no back-slash to rewrite. It now returns `Cow<str>`, **borrowing** when
the input is already clean; callers that must own (`from_path` / `set_path` / `parse`)
`into_owned()` exactly once (no count change), while `joinpath` builds its joined path from the
borrow — so joining a clean segment costs one allocation, not two.

| | before | after |
|--|-------:|------:|
| `joinpath` allocs over a copy | +2 | **+1** |

The `uri_alloc` test asserts `joinpath == copy + 1`, which would regress to `+2` if
normalization went back to always allocating — the budget guards the optimization.
