# FIX versioning — the contract every phase obeys

Seven phases across three prompt files, run in order. Each phase is complete
work: it compiles, its tests pass, its docs are written, and the repository
ships at its end.

| file | phases | what |
| --- | --- | --- |
| `01-foundations.md` | 1, 2 | the `Version` value and its datatype; `FixId` packed into an `i64`, with the branch table |
| `02-dictionary.md` | 3, 4, 5, 6 | `fix:lineage`; code sets and spelling translation; the one merge; the generator |
| `03-messages.md` | 7 | the registry's explicit halves, `FixEntry`, `from_pairs`, and the three text readers |

Each `## Phase` inside a file is one PR. Rule ids (`P4-R8`) are stable across
the whole brief and are cited between files.

**Dependency order.** `1 → 3 → 4 → 5`; `1 + 3 + 4 → 6`; `2 + 3 + 6 → 7`.
Phase 2 depends on nothing and may run first or in parallel, so
`01-foundations.md` holds the two phases that block on nobody.

## How to run each prompt

**One phase per session, one phase per PR** - not one *file* per PR: the
files group phases that share a subject, and each `## Phase` inside one is
its own piece of work. Do not start a phase whose dependencies are not
merged.

**Before writing code:** read what the phase's *Surface* block names, then
re-read `AGENTS.md`. Nothing here gives a path: the tree was refactored
after this brief was written, so everything is named by what it does and
found by symbol.

**Precedence.** `AGENTS.md` > this brief > your priors about how FIX is
usually done. A rule marked **Decided** is settled: implement it, and do not
relitigate - the rejected alternative is recorded so you need not rediscover
why it lost. A rule marked **Verify** means the check comes before the code.

**Rules are numbered** (`P4-R3`). Cite the number in commits, PR text and
review replies.

**Never, in any phase:**

- N1. Add a public symbol this brief does not name.
- N2. Add a dependency this brief does not name.
- N3. Add backward compatibility, a deprecated alias, a shim, or a second
  path to an existing behaviour. The repository has one current contract.
- N4. Store a fact that is already derivable, unless a rule says to *and*
  says why.
- N5. Widen a phase because the next one will need it.
- N6. Leave a `TODO`, an `#[allow]`, an ignored test, or a doc example that
  does not run under the repository's docs-example checker.
- N7. Guess where this brief says refuse, or refuse where it says fall
  through.

**A phase is done when** all of this passes:

```console
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --features "parquet iceberg"
cargo check -p yggdryl --no-default-features --lib
RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps
```

plus the repository's docs-example checker and its strict mkdocs build, plus
the benches the phase names, and you have reported exact results,
including anything skipped and why. A phase that misses a number it promised
reports the measurement; it does not drop the promise quietly.

---

## What is landed

Reuse these unchanged, and find them **by symbol**: the tree has been
refactored since this brief was written, so no path would survive and none
is given.

The one part that did **not** move is the datatype layer - the grammar, the
logical-name table, the identity discriminant and the value contract. Its
shape is settled, so Phase 1 states what a datatype must answer as
invariants rather than as a list of places to edit. Everything else here is
named by what it does, not where it lives.

| notion | what it is |
| --- | --- |
| `FixBranch`, `FixId`, `FixKey` | the identity of one FIX field, and the three ways a caller names one |
| the standard-tag rule | one place decides which branch may claim a tag; Phase 2 replaces its single limit with a range (`P2-R6b`) |
| `FixField` / `FixFieldMut`, `FixAliases` | the borrowed views that read and write the `fix:` namespace |
| `FixRegistry` | resolution in tiers over two halves - primitive and nested - with `insert`, `update`, and a private merge helper |
| its store | `from_handle` / `write_into` over one folder handle: a `primitive` and a `nested` tree, a folder per branch, one shard per `tag / 100` bucket |
| `FixMsg` | a value plus the registry that types it |
| `AsciiEnum` and the `field:enum` document | name to ASCII value, packed through the field's own width |
| `DataType`, `LOGICAL_NAMES`, the name fold | the schema grammar, the FIX Latest datatype table it falls back to, and the fold that makes `UTC_Timestamp` and `utctimestamp` one name |
| `DataTypeId` | the parameter-free discriminant, and a wire contract |
| `Scalar` | the one generic value |
| the value contract | one call that checks a value against a datatype and rewrites it into the exact representation that datatype declares |
| the xxhash module | `xxh32`, `xxh3_64`, and a streaming state |
| the counting-allocator target | the process's one global allocator test |
| the committed dictionary | the seed registry read through the store |
| the published resolution numbers | the "Measured resolution cost" table on the FIX documentation page |

Two landed facts constrain several phases:

- **L1.** `FixRegistry::insert` admits only a field carrying `fix:tag`
  (the registry's `insert`). Nothing without a tag enters the registry - not a
  component, not a message root, not a header.
- **L2.** `DataTypeId::as_u8` is `self as u8` and is a documented wire
  contract (it is `self as u8`). A new variant is **appended**, never
  inserted; `DataTypeId::ALL` grows 54 → 55.

---

## Prior art: `Platob/yggfin`

<https://github.com/Platob/yggfin> is a Python FIX stack (`rekep`) over the
same problem, further along. **Do not port its shapes.** It models
components, repeating groups and namespaces as separate directories and
hangs a flat `comp` string off an entry; yggdryl needs none of that - a
component is a Struct field, a group is a List of an `item` Struct, a branch
is a folder. Read it for the *use cases* it was forced to handle. They are
cited below wherever they changed a rule.

| file | what it settles |
| --- | --- |
| `python/tests/fix/test_pairs.py` | every key and value shape `from_pairs` meets (P7) |
| `python/tests/fix/test_message.py`, `test_transcribe.py` | wire token rules (P7) |
| `python/tests/fix/test_entries.py` | code spelling → value translation (P4) |
| `python/tests/fix/test_fields.py` | types the generator must not narrow (P6) |
| `data/fix/sources.json` | provenance: pinned commit, checksums, licence, priority (P6) |
| `data/fix/versions.json` | declared versions, per-version session field order (P3, P6) |
| `docs/fix/repeating-groups.md` | the ULBridge payload shape (P7) |

Clone read-only. It is not a dependency and nothing links against it.

---
