# Phase 5 — `FixFieldMut::merge_with`

**Goal.** One optimized, FIX-aware merge of two definitions of one field.

**Depends.** Phases 3 and 4.

> **Read `00-contract.md` first.** It is short and binding: the never-list
> `N1`–`N7`, the landed facts `L1`–`L2`, the precedence rule, and the command
> block that says a phase is done. Nothing below repeats it.
>
> **Never, in short:** no public symbol or dependency this brief does not
> name; no compatibility shim or second path; no fact stored that is already
> derivable; no widening for the next phase; no `TODO`, `#[allow]` or ignored
> test; and never guess where a rule says refuse, or refuse where it says
> fall through.

---

**Surface.** The FIX field views (the new merge) and the registry (its
`update` now calls it, and its private merge helper is deleted). The FIX
module's tests, the counting-allocator target, the FIX mutation benchmark
group, and the FIX documentation page.

**Never.** Leave the registry's private `merge` helper in place (N3), or
add a priority or source field to any core type.

### Contract

```rust
impl FixFieldMut<'_> {
    /// Folds another definition of the same field into this one.
    pub fn merge_with(&mut self, other: &FixField<'_>) -> Result<()>;
}
```

### Why the current path is replaced

the registry's private `merge` helper's `merge` builds a new `Metadata`, walks it into the field
with `set_metadata`, then reads back and rewrites `fix:tags` and
`fix:aliases` - three metadata rewrites and a `Vec<String>` of every key.
`ProtocolFieldMut::merge_with`  is worse: it
collects every held property name into an owned `String`, then scans
`O(n*m)`.

### Rules

- **P5-R1. Per-key rules, because "merge" alone decides nothing.**

  | key | rule |
  | --- | --- |
  | `fix:branch`, `fix:tag` | MUST agree; a disagreement is a typed refusal naming both. Identity is not merged. |
  | `fix:tags` | union, incoming first, order kept, deduplicated |
  | `fix:aliases` | union, ASCII-folded comparison, incoming first - then **rewritten from the merged lineage** so P3-R8b still holds |
  | `fix:description` | **never compared.** Incoming wins when it has one; stored is kept when it does not |
  | `fix:lineage` | merged by `since`: union, incoming wins an equal `since`, re-sorted oldest-first, re-validated against the merged name and datatype (P3-R8a) |
  | `fix:codes` | merged by wire value: incoming wins a shared value, stored keeps codes only it has, pedigree carried through, re-rendered canonically once |
  | any other `fix:` key | incoming wins; stored keeps what only it has |

- **P5-R2. Descriptions are never compared** because a description is the
  longest value a field carries and comparing two costs more than the write
  it would save.
- **P5-R3. One metadata write.** Build the merged map, write it once, never
  touch the field between reads. Three rewrites and their `invalidate_arrow`
  calls collapse into one.
- **P5-R4. No key-name allocation.** The `fix:` key set is a `const` list beside the
  field views; walk it, never collect held names into `String`s.
- **P5-R5. Atomic.** A refusal leaves the field exactly as it was.
- **P5-R6. `FixRegistry::update` calls it,** and the private `merge` is
  deleted. No second merge path survives.

### Decided

- **Precedence is the caller's ordering, not a field on the merge.** Several
  sources describe one tag - FIX Latest, a QuickFIX dictionary, a vendor
  orchestration - and yggfin resolves it with a `priority` per source in
  `sources.json`. The generator merges lowest priority first, so the
  highest-priority source is the last `incoming` and wins by P5-R1. One
  concept, in the one place that knows about sources.

### Tests

1. Every row of P5-R1 as its own case.
2. Two fields with different long descriptions: the incoming's survives and
   nothing else moved.
3. A tag disagreement refused, both sides named.
4. A merge that leaves the field byte-identical when the incoming adds
   nothing.
5. Allocation case bounding a merge of two realistic fields.

**Bench.** The new merge against the deleted one's behaviour, over the
the committed dictionary dictionary, in the FIX mutation benchmark group.

---

## Handoff

Phase 6 is the consumer: the generator merges several sources describing
one tag, and relies on `merge_with` doing it in one pass with the per-key
rules of `P5-R1`. Precedence is expressed as merge order, lowest priority
first (`P6-R10`), so nothing in the core learns about sources.
