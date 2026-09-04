# Foundations — the version value and the packed identifier

**Goal.** Two independent pieces of core: a generic `Version` value with its datatype,
and a `FixId` packed into eight bytes with the branch table that hangs off
it.

**Depends.** Nothing. Phases 1 and 2 do not depend on each other and may run in either
order, or in parallel.

> **Read `00-contract.md` first.** It is short and binding: the never-list
> `N1`–`N7`, the landed facts `L1`–`L2`, the precedence rule, and the command
> block that says a phase is done. Nothing below repeats it.
>
> **Never, in short:** no public symbol or dependency this brief does not
> name; no compatibility shim or second path; no fact stored that is already
> derivable; no widening for the next phase; no `TODO`, `#[allow]` or ignored
> test; and never guess where a rule says refuse, or refuse where it says
> fall through.
>
> **Each `## Phase` below is one PR.** Rule ids (`P4-R8`) are stable across
> the whole brief and are cited from the other files.

---

## Phase 1 — `Version`: a generic value, datatype, scalar and field

**Goal.** A generic version value - major, minor, further numeric parts, an
optional qualifier - with its `DataType`, `Scalar` and `Field` support.

**Surface.** A new `Version` module among the generic values, exported from
the crate root. The datatype layer gains a variant, its grammar spelling, its
Arrow mapping, its serde, its default and its merge/compatibility arms; the
generic values gain a `Scalar` variant and a `DataTypeId`; the field layer
gains its value handling and its casts. Tests, the counting-allocator target,
the datatype benchmark, the generic-values page and the datatype page.

**Never.** Touch `rust/src/fix/`. No FIX spelling, no `FIX.` prefix, no
`Latest` in this phase - a caller who has never heard of FIX must be able to
use `Version`.

### Contract

```rust
// rust/src/generic/version.rs, exported as crate::Version
pub struct Version {
    parts: [u16; Version::MAX_PARTS],   // MAX_PARTS = 4, major first
    used: u8,                           // parts the canonical spelling states
    qualifier: Option<Qualifier>,       // { text: SmolStr, pre: bool }
}
impl Version {
    pub const MIN: Self;   // 0
    pub const MAX: Self;   // every part u16::MAX, no qualifier
}
```

### Rules

- **P1-R1. Canonical on parse.** Trailing zero components are trimmed:
  `4.4.0` and `4.4` are one value with one spelling. `Display` re-renders
  exactly what `FromStr` accepts.
- **P1-R2. Grammar.** `major(.part)*` then an optional qualifier, which may
  be appended (`5.0SP2`), dot-introduced (`5.0.SP1`) or hyphen-introduced
  (`1.0.0-rc1`). A hyphen means *pre*-release; a dot or nothing means
  *post*-release. All three canonicalize to one spelling.
- **P1-R3. Why three forms.** One FIX version is really written four ways:
  Orchestra `FIX.5.0SP2`, yggfin `5.0.SP1`, the `ApplVerID` code set
  `FIX50SP1`, the session line `FIXT1.1`. A value with four renderings is
  four values.
- **P1-R4. Bounds and refusals.** A component is decimal and at most
  `u16::MAX`; at most `MAX_PARTS` of them. Over-long input, a non-decimal
  component and an empty qualifier are `Error::Parse` naming the byte
  position, as every other parser in the repo reports.
- **P1-R5. Ordering.** Components numerically, an unstated component reading
  zero; then qualifier class `pre < none < post`; then the qualifier by
  ASCII-folded alphabetic prefix and *numeric* suffix, so `SP2 < SP10`.
  `Ord`, `Eq` and `Hash` agree.
- **P1-R6. `MAX` is the "newer than anything named" sentinel.** Both bounds
  are `const`.
- **P1-R7. No allocation** on parse, compare or render for a qualifier
  inside `SmolStr`'s inline buffer - which every FIX and semver qualifier
  is.
- **P1-R8. Datatype.** `DataType::Version`, placed beside the other
  parameter-free scalars. `DataTypeId::Version` **appended last** (L2);
  `ALL` grows by one; `as_str` is `"version"`; `kind` is the string family;
  `fixed_byte_width` is `None`. Grammar spelling `version`, no alias, and
  the word is not one the Arrow/SQL grammar already owns.
- **P1-R9. Arrow representation is `Utf8`,** the canonical text.

### The datatype layer's invariants

The datatype layer is the part of this repository that has **not** moved and
will not: its shape is settled, and what a datatype must answer is settled
with it. So this phase is not a sweep over files - it is a list of
invariants the layer already guarantees for every variant, which `Version`
must uphold too.

- **P1-R10. The compiler will not find the sites for you.** `DataType` and
  `DataTypeId` are both `#[non_exhaustive]`, and the datatype layer alone
  carries on the order of sixty `_ =>` wildcard arms. A new variant
  therefore **compiles clean while behaving wrongly**: it falls into a
  wildcard and silently answers whatever the fallback answers. Treat a green
  build as no evidence at all. Find the sites by reading every wildcard arm
  in the datatype, generic-value, field, Arrow and expression layers and
  deciding, for each, whether it should now name `Version`. The closest
  existing analogue to imitate is a parameter-free coded scalar such as
  `Cfi`.
- **P1-R11. Each invariant below is proven by a test, not by a match arm.**
  A test is what a wildcard cannot satisfy by accident.

  | invariant | what must hold |
  | --- | --- |
  | naming | one canonical spelling; grammar and `Display` round-trip; the folded spelling resolves |
  | identity | `id()` answers the new `DataTypeId`; `as_str`, `kind`, `fixed_byte_width` and `ALL` all account for it; `as_u8` keeps its wire contract (L2) |
  | Arrow | maps to exactly one Arrow type and back, losslessly, through a `Field` |
  | serde | the serialized shape is the canonical spelling, and it round-trips |
  | value | the value contract checks *and rewrites* into the declared representation (P1-R12) |
  | default | it answers a default value rather than falling through to one |
  | merge and compatibility | merging with itself is itself; against a foreign datatype it refuses with expected and actual |
  | casts | declared in both directions or refused explicitly - never a silent identity (P1-R13) |
  | nestedness | not nested, so a registry places it in the primitive half |
  | rejection | the layers that cannot represent it - the table formats, the row codecs - refuse it **by name**, with the message they give any other type they cannot carry |

- **P1-R12. Value contract.** `DataType::scalar` accepts a `Scalar::String`
  that parses and **rewrites** it to `Scalar::Version`, accepts
  `Scalar::Version` unchanged, and refuses everything else with expected and
  actual. `Field::scalar` is that plus nullability and name. Nothing
  re-checks what `scalar` answered.
- **P1-R13. Casts.** `Version → Utf8` renders, `Utf8 → Version` parses,
  `Version → Version` is identity. No numeric casts.
- **P1-R14. Skip what it is not.** A version is neither an ASCII width nor a
  coded vocabulary, so the ASCII-packing and code-vocabulary paths gain no
  arm. Saying so is part of the work: a reviewer must be able to see the
  omission was decided, not missed.

### Decided

- **Utf8 over a fixed-width packing.** A qualifier has no length bound and a
  lossy Arrow round trip is unacceptable. *Cost:* Arrow-side lexicographic
  order is **not** version order. Say so in the docs, demonstrate it in a
  test, and make `Ord` on `Version` the only ordering contract.
- **Post-release default.** `5.0SP2` sorts *after* `5.0` because FIX service
  packs are post-releases. Semver pre-release ordering is reachable through
  the hyphen form, and only through it.

### Tests

1. The grammar, including every refusal with its byte position.
2. Trailing-zero canonicalization; `Display`/`FromStr` round trip.
3. The ordering table:
   `0 < 1.0 < 4.2 < 4.4 < 5.0-rc1 < 5.0 < 5.0SP1 < 5.0SP2 < 5.0SP10 < MAX`.
4. Four spellings of one version parsing equal: `5.0SP1`, `5.0.SP1`,
   `FIX.5.0SP1` (through P3's prefix strip), `FIX50SP1` (through the
   `ApplVerID` code set).
5. One case per row of the P1-R11 invariant table - ten tests, each of which
   a wildcard arm cannot pass by accident.
6. `DataType::scalar` rewriting a `Scalar::String` (P1-R12).
7. Allocation case: parse, compare and render allocate nothing.

**Bench.** Parse and compare, in the datatype benchmark target.
**Docs.** The value on the generic-values page; the datatype row on
the datatype page.

---

## Phase 2 — `FixId` is one `i64`

**Goal.** Pack the identifier into eight bytes so it is its own hash key.

**Surface.** The FIX module: the identifier, the branch, the field views,
the registry and its store, and the message. The FIX module's tests, the
Python and Node FIX bindings, the counting-allocator target, the FIX
resolution benchmark group, and the FIX documentation page.

**Never.** Change an *existing* serialized shape. `FixId` is derived and
never stored, so a shard written before this phase must load and round-trip
byte-identically after it. That is the test that says the phase is safe. The
branch manifest this phase adds (P2-R18) is a **new** leaf beside the trees
and touches no shard, which is what keeps that test meaningful.

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
/// What a branch declares about itself, beside its name.
pub struct FixBranchInfo {
    branch: FixBranch,
    version: Option<Version>,          // the dialect's default FIX version
    ep: Option<u32>,                   // and its extension pack (P3-R3)
    sender_comp_id: Option<SmolStr>,   // the session this dictionary speaks as
    target_comp_id: Option<SmolStr>,   // and the one it speaks to
}

impl FixRegistry {
    pub fn branch_of(&self, id: FixId) -> Option<&FixBranchInfo>;
    pub fn branch_named(&self, name: &str) -> Option<&FixBranchInfo>;
    pub fn branch_for_session(&self, sender: &str, target: &str) -> Option<&FixBranchInfo>;
    pub fn branches(&self) -> impl Iterator<Item = &FixBranchInfo>;
    pub fn set_branch_info(&mut self, info: FixBranchInfo) -> Result<()>;
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
- **P2-R9. The registry keeps a branch table keyed by digest,** filled on
  insert, so every refusal it raises still names `cme:5001`. Only an
  identifier held outside a registry renders as hex.
- **P2-R9b. A branch is a dialect, so the table holds what it declares.** A
  non-standard branch is not just a namespace for tags - it is one
  counterparty's dictionary, and three facts travel with it: the FIX version
  it speaks by default, and the `SenderCompID` / `TargetCompID` pair that
  identifies its session. A `.cfb` states all three on its root element, a
  vendor orchestration states the version, and today all three are dropped
  on the floor and rediscovered per message. Bundle them on the branch
  instead: `FixBranchInfo` is the entry, and the bare `FixBranch` stays the
  identity inside it.
- **P2-R9c. Every field of `FixBranchInfo` beyond the name is optional,** and
  a branch that declares nothing is exactly today's behaviour. The standard
  branch declares nothing: it is the specification, not a counterparty.
- **P2-R9d. Comp ids are stored as they arrive, compared ASCII-folded.** A
  venue that writes `BANKX` and one that writes `bankx` are one session.
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
- **P2-R18. The branch table persists as one manifest beside the trees.**
  The store descends only the `primitive` and `nested` trees, so a leaf
  beside them is never listed - the same property the provenance manifest
  relies on (P6-R5). Write the branch table there as one canonically
  rendered JSON document, ordered by branch name so a diff is reviewable;
  read it back on open. **Absence is the empty table**, which is the
  laziness contract every handle in this repository already keeps: a
  dictionary written before this phase loads with bare branch names and
  behaves exactly as it does today. A manifest naming a branch no field
  belongs to is a typed error naming it - a dialect with no fields is a
  configuration mistake, not an empty dictionary.
- **P2-R19. The process-wide default registry *is* the branch singleton.**
  Parsing needs one shared place to resolve a branch from, and this
  repository already has exactly one: the process default, resolved once on
  first use and installable by a caller. Hang the branch table off it and
  every reader - `from_pairs`, the text readers, a transcriber - resolves
  through the same table without threading anything.

### Decided

- **A bare `FixId` cannot name its branch.** The digest is one-way. Paid
  explicitly through P2-R7/R8/R9. *Rejected:* a process-wide branch intern
  table so a bare id could render itself - it buys prettier `Debug` for a
  global lock or a leak on the hot path, and the registry already knows
  every branch it holds.
- **One singleton, not two.** P2-R19 puts the branch table on the existing
  process-wide default registry. *Rejected:* a separate global branch
  registry with its own lock and its own install path. Two process-wide
  tables that must agree about which dialects exist is two sources of truth
  and a lock-ordering question nobody wants; a branch has no meaning apart
  from the fields that carry it, so it belongs where they do.

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
7. `write_into` byte-identical to a shard written before the change - the
   branch manifest is a new leaf and changes no shard (P2-R18).
7b. A dictionary with no manifest loads with bare branch names and behaves
    as before; a manifest round-trips canonically; a manifest naming a
    branch no field belongs to is a typed error naming it.
7c. `branch_for_session` answers the dialect declaring a `(sender, target)`
    pair, ASCII-folded, and `None` when none does (P2-R9d).
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

### From Phase 1

Phases 3, 4 and 6 all take `Version` from here. What they rely on:

- `Version::MIN` / `Version::MAX` as bounds of the value space (`P1-R6`).
  Phase 3 does *not* map "FIX Latest" onto `MAX`: it resolves that label to
  the real version and extension pack the dictionary carries (`P3-R1b`).
- `FromStr` accepting all three qualifier forms (`P1-R2`), because the FIX
  layer strips a `FIX.` prefix and hands the rest straight in.
- `Ord` being the only ordering contract (`P1-R5`), which every version
  filter in Phases 3, 4 and 7 leans on.

### From Phase 2

Phase 7 is the only consumer, and takes three things:

- `FixId` as a `Copy` 8-byte key that is its own hash key (`P2-R1`, `P2-R11`),
  which is what makes `FixEntry::id()` a shift-or (`P7-R3`).
- `FixRegistry::branches()` (`P2-R9`), which Phase 7's branch inference walks
  (`P7-R16`).
- `FixBranch`'s cached `xxh32` digest (`P2-R4`), which `FixEntry.branch`
  stores reinterpreted (`P7-R4`).
