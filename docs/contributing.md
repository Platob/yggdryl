# Contributing

Run the checks for whatever you changed, and make code, tests, and documentation agree before handing off.

=== "Rust"

    ```console
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --features "parquet iceberg" -- -D warnings
    cargo test --workspace --all-targets --features "parquet iceberg"
    python scripts/check_docs_examples.py
    python -m mkdocs build --strict
    ```

=== "Python"

    ```console
    cd python
    .venv/Scripts/python -m maturin develop
    .venv/Scripts/python -m pytest
    .venv/Scripts/python -m mypy --strict yggdryl tests/typing_bindings.py tests/types/typing_fields.py
    ```

=== "JavaScript"

    ```console
    npm ci --prefix node
    npm run --prefix node build:debug
    npm test --prefix node
    ```

Start with the repository-wide `AGENTS.md`, which is the normative version of everything below.

## Where things go

```text
rust/src/types/       datatypes, fields, values -> rust/tests/types/       -> docs/types.md
rust/src/holder/      storage implementations  -> rust/tests/holder/      -> docs/holder.md
rust/src/coding/      content codings           -> rust/tests/coding/      -> docs/coding.md
rust/src/media/       record and table formats  -> rust/tests/media/       -> docs/media.md
rust/src/text/        structured text           -> rust/tests/text/        -> docs/text.md
rust/src/uri/         identifiers               -> rust/tests/uri/         -> docs/uri.md
rust/src/arrow/       Arrow boundaries          -> rust/tests/arrow/       -> docs/arrow.md
rust/src/expression/  expressions               -> rust/tests/expression/  -> docs/expression.md
rust/src/xxhash/      hashes                    -> rust/tests/xxhash/       -> docs/xxhash.md
rust/src/fix/         FIX protocol              -> rust/tests/fix/          -> docs/fix.md
```

Each shared trait, enum, or value owns one root `.rs` file. Each implementation family owns one
layer folder. `rust/src/lib.rs` only declares modules and re-exports their public vocabulary.
Python and Node mirror these layer names without reimplementing core behavior.
Benchmarks live in each member's `benchmarks/`; there are no `examples/` directories, because every
runnable example lives in the documentation where all three languages appear side by side.

## What a change must satisfy

**One concept, one home.** A behaviour belongs to exactly one module. If two modules need it, it
moves down to the one they share rather than being copied.

**A trait says what; an enum says which.** Adding a backend or an encoding means implementing the
trait and adding the variant to the [generic enum](types.md), never adding a parallel
dispatch.

**Errors state expected, got, and where.** `expected int64, got utf8` with a path or byte offset. A
message that only names a rule is incomplete.

**Refusals are features.** A cast that widens silently, a schema that accepts a nullable root, or a
handle that double-compresses is worse than an error. Test the refusal.

**Laziness is a contract.** Constructing a handle touches nothing; reading something absent yields
nothing; writing creates. New backends inherit this by implementing the
[role traits](holder.md).

**Never pre-check.** No `exists` before a read, no `contains` before a `get`, no `mkdir` before a
write, no "ensure" step of any kind. Act; branch on the typed
[absence or conflict](holder.md); repair once and retry once. A probe costs a round trip and its answer
is stale before it returns, so the act still has to handle absence and the check bought nothing but
a race. An existence question a *caller* asked - `exists`, `is_dir`, `contains` - is an answer, and
stays public; one of our own on the way to doing something else is a bug.

**Every listing is an iterator.** [`ls`, `glob`, and the predicate listings](holder.md) yield one entry
at a time, the item is a `Result`, and the iterator fuses at the first failure. A return that stays
owned says what bounds it, in a comment or a test; neither means it was missed.

## Documentation is part of the change

Every page mirrors one module folder, opens with one H1 and exactly one sentence, and shows every
example in all three languages unless the module carries the Rust-only note. Every block is executed
by `python scripts/check_docs_examples.py`, so a renamed method breaks the docs exactly as it breaks
the code.

Keep source documentation and Markdown compact: state the contract, one non-obvious edge, and
measured behavior once. Remove repeated rationale, signature restatements, and prose already made
obvious by names or examples; keep safety, streaming, failure, and compatibility guarantees.

When you add or rename a page, update the `mkdocs.yml` navigation and every link to it in the same
change; `mkdocs build --strict` fails otherwise.

## Bindings

Python and JavaScript are views. They may infer inputs at the boundary - a string where a datatype
expression is expected, a path-like where a URL is expected - and must then call the core. A native
error message crosses the boundary unchanged, and each binding stays idiomatic in its own language:
mapping dunders and keyword arguments in Python, generic `from` constructors and `Map` protocols in
JavaScript.
