# Phase 2 — `FixId` is one `i64`

**Goal.** Pack the identifier into eight bytes so it is its own hash key.

**Depends.** Nothing.

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

**Surface.** The FIX module: the identifier, the branch, the field views,
the registry and its store, and the message. The FIX module's tests, the
Python and Node FIX bindings, the counting-allocator target, the FIX
resolution benchmark group, and the FIX documentation page.

**Never.** Change a serialized shape. `FixId` is derived and never stored,
so a shard written before this phase must load and round-trip
byte-identically after it. That is the test that says the phase is safe.

### Contract

```rust
pub struct FixId(i64);            // ((tag as i64) << 32) | i64::from(xxh32(branch))
pub struct FixBranch { text: SmolStr, digest: u32 }   // text declared first

impl FixId {
    pub const fn standard(tag: i32) -> Self;
    pub fn from_parts(branch: &FixBranch, tag: i32) -> Result<Self>;
    pub const fn tag(self) -> i32;
    pub const fn branch_digest(self) -> u32;
    pub const fn is_standard(self) -> bool;
}
impl FixRegistry {
    pub fn branch_of(&self, id: FixId) -> Option<&FixBranch>;
    pub fn branches(&self) -> impl Iterator<Item = &FixBranch>;
}
```

### Rules

- **P2-R1. `i64`, not `u64`.** A tag is `i32` in `0..=i32::MAX`, so bit 63
  is never set, every identifier is positive, and `Ord` on the `i64` is the
  natural order of the packed pair. The digest zero-extends, so the low half
  compares unsigned.
- **P2-R2. `Copy`, 8 bytes,** `Hash` and `Ord` without touching the heap.
  `FixKey::Id(FixId)` stops borrowing; `next_field_after` takes
  `Option<FixId>` by value.
- **P2-R3. `standard(tag)` becomes `const fn`.** The doc comment
  apologising for `SmolStr`'s `Drop` goes with the field.
- **P2-R4. The branch caches its own digest.** `xxh32`
   runs once, in `FixBranch::from_str`, the only
  constructor. `text` is declared first so derived `Eq`/`Ord` stay
  text-based; the digest is a function of the text so it can only agree.
  `MAX_LENGTH` stays 23 - the digest sits beside `SmolStr`, not inside it.
- **P2-R5. `FixBranch::STANDARD`'s digest is a literal**, pinned by a test
  asserting `xxh32(b"standard")` equals it.
- **P2-R6. `from_parts` is a shift and an or,** and it stays the one place
  the admissibility rule lives, plus one new refusal: a non-standard branch
  whose digest equals the standard branch's is rejected there, so
  `is_standard()` stays total.
- **P2-R6b. The user-defined tag range is `[5000, 40000)`.** The landed rule
  is one `STANDARD_TAG_LIMIT` of 5000 with everything above it treated as
  claimable, and that is wrong at the top: the specification has resumed
  assigning tags at and above 40000, so a vendor branch claiming one would
  collide with the standard dictionary. Replace the single limit with the
  half-open range:

  | tag | whose |
  | --- | --- |
  | `0 ..= 4999` | the specification's - forced to the standard branch |
  | `5000 ..= 39999` | user-defined - any branch may claim it |
  | `40000 ..` | the specification's again - forced to the standard branch |

  So admissibility is `branch.is_standard() || (5000..40_000).contains(&tag)`.
  Two constants replace the one, named for the range they bound, and the
  refusal names both of them and the offending tag.
- **P2-R6c. This is a surface change, not an internal one.** The single
  limit is public, is asserted in the FIX tests, is re-exported by both
  bindings as a module constant, and is quoted in four places on the FIX
  documentation page - including runnable Python and JavaScript examples
  that assert it equals 5000. Replace all of them in this phase; leaving the
  old constant beside the new bounds is exactly the second path N3 forbids.
- **P2-R7. `FixId::branch()` is deleted.** Its eight callers
  all hold the owning field and read `fix:branch` from it, which is where
  the text lives. `branch_digest()` replaces it where only identity matters.
- **P2-R8. `Display`** renders `standard:35` for the standard branch and
  `#7f3a1c02:5001` - digest in lowercase hex - for any other. `from_str`
  still accepts `cme:5001`: it has the text and hashes it. The the FIX module docs
  doctest asserting `from_str("CME:5001").to_string() == "cme:5001"` changes
  with it.
- **P2-R9. The registry keeps `branches: HashMap<u32, FixBranch>`,** filled
  on insert, so every refusal it raises still names `cme:5001`. Only an
  identifier held outside a registry renders as hex.
- **P2-R10. A branch-digest collision is a typed conflict at insert,**
  naming both spellings and the digest. A 32-bit space over a handful of
  branches, but a stated failure rather than two dictionaries silently
  aliasing.
- **P2-R11. The indexes.**

  ```rust
  ids:             HashMap<FixId, usize, BuildHasherDefault<Mix>>
  alternate_ids:   HashMap<FixId, usize, BuildHasherDefault<Mix>>
  names:           HashMap<u64,   usize, BuildHasherDefault<Mix>>
  aliases:         HashMap<u64,   usize, BuildHasherDefault<Mix>>
  positions_by_id: Vec<usize>   // ordered by FixId, iteration only
  ```

  An identifier lookup now hashes nothing: the key *is* the id.
- **P2-R12. `Mix` finalizes, it does not pass through.** The packed high
  bits are the tag, under 65536 for nearly every FIX field, so the top bytes
  are near-constant - and hashbrown takes its control byte from the top
  bits. A raw pass-through puts every standard field in one control-byte
  class. `Mix` applies one multiply-xor-shift finalizer.
- **P2-R13. Name and alias keys stay text,** hashed per probe: ASCII-fold
  into the streaming xxh3 state (the xxhash module's streaming state) in stack-sized chunks so
  no length allocates, seeded with the branch's own `xxh32` digest so a name
  cannot be found under another branch, with a distinct constant seed per
  index.
- **P2-R14. A name-digest collision is a loud refusal, never a wrong
  answer.** Two names mapping to one `u64` would silently overwrite in a
  `HashMap<u64, _>`. `insert` verifies the field at an occupied key really
  holds it and returns a typed conflict otherwise; reads re-check the same
  way, so a collision degrades to a miss. Identifier keys need no such
  check - a `FixId` *is* the key, and its only collision is P2-R10's.
- **P2-R15. Ordered iteration keeps its own structure.**
  `next_field_after`, `FixFieldIter`, `Debug`, `PartialEq` and
  `store::write_into` need an ordered walk a hash map cannot give.
  `positions_by_id` is a `Vec<usize>` kept sorted by binary-search insert:
  `O(n)` per insert on a dictionary built once and read forever, against
  `O(log n)` node chasing on every read.
- **P2-R16. Order becomes tag-major.** Within one branch it is unchanged,
  so `write_into` still produces **byte-identical shards** - a shard folder
  is one branch, a shard file one `tag / 100` bucket. What moves is the
  cross-branch walk: vendor fields interleave among standard ones by tag.
  Update the "orders branch-major" sentence in the FIX module docs, the
  `next_field_after` doc, and the `FixFieldIter` docs; assert the new order
  in a test rather than leaving it to whichever map iterates first.
- **P2-R17. Binding impact is small and mechanical.** The Python and Node
  FIX bindings only parse an id from text and hold one as a cursor (passed
  by reference); neither renders one back, so `Copy` simply makes those
  by-value. The one real change is P2-R6c: both bindings re-export the tag
  limit as a module constant, and both must re-export the two bounds
  instead. No new binding *surface* beyond that swap - no `FixId` class on
  either side.

### Decided

- **A bare `FixId` cannot name its branch.** The digest is one-way. Paid
  explicitly through P2-R7/R8/R9. *Rejected:* a process-wide branch intern
  table so a bare id could render itself - it buys prettier `Debug` for a
  global lock or a leak on the hot path, and the registry already knows
  every branch it holds.

### Tests

1. Packing round trip (`from_parts` → `tag()`, `branch_digest()`) at tag
   bounds `0`, `4999`, `5000`, `39999`, `40000`, `i32::MAX`.
1b. Admissibility across the whole range (P2-R6b): a vendor branch is
    refused at `4999` and at `40000`, admitted at `5000` and `39999`; the
    standard branch is admitted everywhere. The refusal names both bounds.
2. `standard(tag)` in a `const` context.
3. The pinned `xxh32(b"standard")` constant.
4. A branch-digest collision refused at insert, both spellings named.
5. A name-digest collision refused.
6. Ordering across tags and across branches.
7. `write_into` byte-identical to a shard written before the change.
8. Every existing registry test passes unchanged, except the two facts that
   genuinely moved: vendor-branch `Display`, and cross-branch iteration
   order.
9. the counting-allocator test target stays green - lookups allocate nothing today
   and must still.
10. `Mix` control-byte spread over the the committed dictionary dictionary, not just
    that lookups answer.

**Bench.** `cargo bench -p yggdryl --bench fix`, then **replace** the
the FIX documentation page table. Numbers to beat: 32.3 ns primitive tag hit, 93.1 ns
nested tag hit, 65.8 ns alternate tag hit, 72.2 ns miss, 128.1 ns vendor
identifier hit over 1034 fields, 81.8 ns name hit.

---

## Handoff

Phase 7 is the only consumer, and takes three things:

- `FixId` as a `Copy` 8-byte key that is its own hash key (`P2-R1`, `P2-R11`),
  which is what makes `FixEntry::id()` a shift-or (`P7-R3`).
- `FixRegistry::branches()` (`P2-R9`), which Phase 7's branch inference walks
  (`P7-R16`).
- `FixBranch`'s cached `xxh32` digest (`P2-R4`), which `FixEntry.branch`
  stores reinterpreted (`P7-R4`).
