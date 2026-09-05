# FIX versioning — the contract every phase obeys

Nine phases across three prompt files. Each phase is complete work: it
compiles, its tests pass, its docs are written, the repository ships.

| file | phases | what |
| --- | --- | --- |
| `01-foundations.md` | 1, 2 | the `Version` value and its datatype; `FixId` packed into an `i64`, with the branch table |
| `02-dictionary.md` | 3, 4, 5, 6, 8 | `fix:lineage`; code sets and spelling translation; the one merge; the generator; the `Side` and `MsgType` datatypes |
| `03-messages.md` | 7, 9 | the registry's explicit halves, `FixEntry`, `from_pairs`, the three text readers; lifting |

**Order.** `1 → 3 → 4 → 5`; `1 + 3 + 4 → 6`; `4 + 6 → 8`; `2 + 3 + 6 → 7`;
`7 + 8 → 9`. Phase 2 blocks on nobody.

## How to run

- **One phase per PR**, not one file. Each `## Phase` is its own work.
- **No paths anywhere.** The tree was refactored after this was written;
  everything is named by what it does and found by symbol.
- **Precedence.** `AGENTS.md` > this brief > your priors about FIX.
- **Decided** = settled; implement it, do not relitigate. The rejected
  alternative is recorded so you need not rediscover why it lost.
- **Verify** = the check comes before the code.
- **Rule ids** (`P4-R8`) are stable and cited across files. Use them in
  commits, PR text and review replies.
- **Every Decided reason belongs in the doc comment** of the thing it
  decides. A rule explained only here is a rule the next reader undoes.

## Never, in any phase

- **N1.** Add a public symbol this brief does not name.
- **N2.** Add a dependency this brief does not name.
- **N3.** Add a compatibility shim, deprecated alias, or second path to an
  existing behaviour. One current contract.
- **N4.** Store a fact already derivable, unless a rule says to *and* why.
- **N5.** Widen a phase because the next one will need it.
- **N6.** Leave a `TODO`, `#[allow]`, ignored test, or a doc example that
  does not run under the repository's docs-example checker.
- **N7.** Guess where a rule says refuse, or refuse where it says fall
  through.

## Done when

```console
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --features "parquet iceberg"
cargo check -p yggdryl --no-default-features --lib
RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps
```

passes, plus the docs-example checker, the strict mkdocs build, and the
benches the phase names — and you have reported exact results including
anything skipped. A phase that misses a number it promised reports the
measurement; it does not drop the promise.

## What is landed

Reuse unchanged, found by symbol.

| notion | what it is |
| --- | --- |
| `FixBranch`, `FixId`, `FixKey` | one field's identity, and the three ways a caller names one |
| the standard-tag rule | one place decides which branch may claim a tag; P2-R6b replaces its single limit with a range |
| `FixField` / `FixFieldMut`, `FixAliases` | the borrowed views over the `fix:` namespace |
| `FixRegistry` | tiered resolution over a primitive and a nested half, with `insert`, `update`, a private merge helper |
| its store | `from_handle` / `write_into` over one folder handle: two trees, a folder per branch, one shard per `tag / 100` |
| `FixMsg` | a value plus the registry that types it |
| `AsciiEnum`, the `field:enum` document | name to ASCII value, packed through the field's own width |
| `DataType`, `LOGICAL_NAMES`, the name fold | the schema grammar, the FIX Latest datatype table it falls back to, and the fold making `UTC_Timestamp` and `utctimestamp` one name |
| `DataTypeId` | the parameter-free discriminant, and a wire contract |
| `Scalar`, the value contract | one generic value; one call that checks it against a datatype and rewrites it into that datatype's exact representation |
| the xxhash module | `xxh32`, `xxh3_64`, a streaming state |
| the counting-allocator target | the process's one global allocator test |
| the committed dictionary | the seed registry read through the store, at `config/fix` in the repository root — the one path this brief names, because a committed data location is a contract, not a code path (P6-R1b) |
| the published numbers | the "Measured resolution cost" table on the FIX documentation page |

**L1.** `FixRegistry::insert` admits only a field carrying `fix:tag`. Nothing
without a tag enters — not a component, not a message root, not a header.

**L2.** `DataTypeId::as_u8` is `self as u8`, a documented wire contract. A new
variant is **appended**, never inserted; `ALL` grows by one.

## Prior art: `Platob/yggfin`

<https://github.com/Platob/yggfin> is a Python FIX stack over the same
problem, further along. **Do not port its shapes** — it splits components,
groups and namespaces into directories and hangs a flat `comp` string off an
entry; here a component is a Struct, a group a List of an `item` Struct, a
branch a folder. Read it for the use cases it was forced to handle, cited
below wherever they changed a rule.

| file | settles |
| --- | --- |
| `python/tests/fix/test_pairs.py` | every key and value shape `from_pairs` meets (P7) |
| `test_message.py`, `test_transcribe.py` | wire token rules (P7) |
| `test_entries.py` | code spelling → value translation (P4) |
| `test_fields.py` | types the generator must not narrow (P6) |
| `data/fix/sources.json` | provenance: pinned commit, checksums, licence, priority (P6) |
| `data/fix/versions.json` | declared versions, per-version session field order (P3, P6) |
| `docs/fix/repeating-groups.md` | the ULBridge payload shape (P7) |

Clone read-only. Nothing links against it.
