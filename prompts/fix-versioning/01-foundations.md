# Foundations — the version value and the packed identifier

**Goal.** A generic `Version` value with its datatype, and a `FixId` packed
into eight bytes with the branch table hanging off it.

**Depends.** Nothing. The two phases are independent; either order, or both.

> Read `00-contract.md` first: `N1`–`N7`, `L1`–`L2`, precedence, done-when.
> Each `## Phase` is one PR. Rule ids are cited from the other files.

---

## Phase 1 — `Version`: a generic value, datatype, scalar and field

**Goal.** One field answers a version; the datatype layer answers it too.

**Surface.** A new `Version` module among the generic values, exported from
the crate root. The datatype layer gains a variant, its grammar spelling,
Arrow mapping, serde, default and merge/compatibility arms; the generic
values gain a `Scalar` variant and a `DataTypeId`; the field layer gains its
value handling and casts. Tests, the counting-allocator target, the datatype
benchmark, the generic-values and datatype pages.

**Never.** Touch the FIX layer. No FIX spelling, no `FIX.` prefix, no
`Latest`: a caller who has never heard of FIX must be able to use `Version`.

### Contract

```rust
pub struct Version {
    parts: [u16; Version::MAX_PARTS],   // MAX_PARTS = 4, major first
    used: u8,                           // parts the canonical spelling states
    qualifier: Option<Qualifier>,       // { text: SmolStr, pre: bool }
}
impl Version { pub const MIN: Self; pub const MAX: Self; }
```

### Rules

- **P1-R1. Canonical on parse.** Trailing zero components are trimmed:
  `4.4.0` and `4.4` are one value, one spelling. `Display` re-renders exactly
  what `FromStr` accepts.
- **P1-R2. Grammar.** `major(.part)*` then an optional qualifier: appended
  (`5.0SP2`), dot-introduced (`5.0.SP1`) or hyphen-introduced (`1.0.0-rc1`).
  A hyphen is *pre*-release; a dot or nothing is *post*-release. All three
  canonicalize to one spelling.
- **P1-R3. Why three forms.** One FIX version is written four ways:
  Orchestra `FIX.5.0SP2`, yggfin `5.0.SP1`, the `ApplVerID` code set
  `FIX50SP1`, the session line `FIXT1.1`. Four renderings would be four
  values.
- **P1-R4. Bounds and refusals.** A component is decimal, at most
  `u16::MAX`, at most `MAX_PARTS` of them. Over-long input, a non-decimal
  component and an empty qualifier are `Error::Parse` naming the byte
  position.
- **P1-R5. Ordering.** Components numerically, unstated reading zero; then
  qualifier class `pre < none < post`; then the qualifier by ASCII-folded
  alphabetic prefix and *numeric* suffix, so `SP2 < SP10`. `Ord`, `Eq` and
  `Hash` agree.
- **P1-R6. `MIN` and `MAX` are bounds, not meanings.** Both `const`. `MAX`
  is the top of the value space and nothing more: a real latest release is a
  real version and is named as one (P3-R2).
- **P1-R7. No allocation** on parse, compare or render for a qualifier
  inside `SmolStr`'s inline buffer — which every FIX and semver qualifier is.
- **P1-R8. Datatype.** `DataType::Version` beside the other parameter-free
  scalars. `DataTypeId::Version` **appended last** (L2); `ALL` grows by one;
  `as_str` is `"version"`; `kind` is the string family; `fixed_byte_width` is
  `None`. Grammar spelling `version`, no alias, and not a word the Arrow/SQL
  grammar already owns.
- **P1-R9. Arrow representation is `Utf8`,** the canonical text.

#### The datatype layer's invariants

The datatype layer did not move in the refactor and will not: what a
datatype must answer is settled. So this is a list of invariants, not a
sweep over files.

- **P1-R10. The compiler will not find the sites for you.** `DataType` and
  `DataTypeId` are both `#[non_exhaustive]`, and the datatype layer alone
  carries some sixty `_ =>` arms. A new variant **compiles clean while
  behaving wrongly** — it falls into a wildcard and answers whatever the
  fallback answers. A green build is no evidence. Read every wildcard arm in
  the datatype, generic-value, field, Arrow and expression layers and decide
  whether it should now name `Version`. Closest analogue to imitate: a
  parameter-free coded scalar such as `Cfi`.
- **P1-R11. Each invariant is proven by a test,** which is what a wildcard
  cannot satisfy by accident.

  | invariant | what must hold |
  | --- | --- |
  | naming | one canonical spelling; grammar and `Display` round-trip; the folded spelling resolves |
  | identity | `id()` answers the new `DataTypeId`; `as_str`, `kind`, `fixed_byte_width`, `ALL` account for it; `as_u8` keeps its wire contract (L2) |
  | Arrow | one Arrow type and back, losslessly, through a `Field` |
  | serde | the serialized shape is the canonical spelling, and round-trips |
  | value | the value contract checks *and rewrites* into the declared representation (P1-R12) |
  | default | answers a default rather than falling through to one |
  | merge, compatibility | with itself is itself; against a foreign datatype refuses with expected and actual |
  | casts | declared both directions or refused explicitly — never a silent identity (P1-R13) |
  | nestedness | not nested, so a registry places it in the primitive half |
  | rejection | layers that cannot represent it refuse **by name**, with the message they give any other such type |

- **P1-R12. Value contract.** `DataType::scalar` accepts a `Scalar::String`
  that parses and **rewrites** it to `Scalar::Version`, accepts
  `Scalar::Version` unchanged, refuses the rest with expected and actual.
  `Field::scalar` adds nullability and name. Nothing re-checks it.
- **P1-R13. Casts.** `Version → Utf8` renders, `Utf8 → Version` parses,
  `Version → Version` is identity. No numeric casts.
- **P1-R14. Skip what it is not.** Not an ASCII width, not a coded
  vocabulary: those paths gain no arm. Say so, so a reviewer sees the
  omission was decided.

### Decided

- **Utf8 over fixed-width packing.** A qualifier has no length bound and a
  lossy Arrow round trip is unacceptable. *Cost:* Arrow-side lexicographic
  order is not version order — document it, demonstrate it, and make `Ord`
  the only ordering contract.
- **Post-release default.** `5.0SP2` sorts after `5.0` because FIX service
  packs are post-releases. Semver pre-release is reachable through the
  hyphen form only.

### Tests

1. The grammar, every refusal with its byte position.
2. Trailing-zero canonicalization; `Display`/`FromStr` round trip.
3. `0 < 1.0 < 4.2 < 4.4 < 5.0-rc1 < 5.0 < 5.0SP1 < 5.0SP2 < 5.0SP10 < MAX`.
4. Four spellings of one version parsing equal: `5.0SP1`, `5.0.SP1`,
   `FIX.5.0SP1` (through P3's prefix strip), `FIX50SP1` (through `ApplVerID`).
5. One case per row of P1-R11 — ten tests a wildcard cannot pass by accident.
6. `DataType::scalar` rewriting a `Scalar::String` (P1-R12).
7. Allocation: parse, compare and render allocate nothing.

**Bench.** Parse and compare, in the datatype benchmark target.
**Docs.** The value on the generic-values page; the row on the datatype page.

---

## Phase 2 — `FixId` is one `i64`

**Goal.** Pack the identifier into eight bytes so it is its own hash key,
and make the branch a dialect record.

**Surface.** The FIX module: identifier, branch, field views, registry and
store, message. Its tests, the Python and Node FIX bindings, the
counting-allocator target, the FIX resolution benchmark group, the FIX page.

**Never.** Change an *existing* serialized shape. `FixId` is derived and
never stored, so a shard written before this phase must load and round-trip
byte-identically after it — that test is what says the phase is safe. The
branch manifest (P2-R16) is a **new** leaf and touches no shard.

### Contract

```rust
pub struct FixId(i64);          // ((tag as i64) << 32) | i64::from(xxh32(branch))
pub struct FixBranch { text: SmolStr, digest: u32 }   // text declared first

/// What a branch declares about itself, beside its name.
pub struct FixBranchInfo {
    branch: FixBranch,
    version: Option<Version>,          // the dialect's default FIX version
    ep: Option<u32>,                   // and its extension pack (P3-R4)
    sender_comp_id: Option<SmolStr>,   // the session it speaks as
    target_comp_id: Option<SmolStr>,   // and the one it speaks to
}

impl FixId {
    pub const fn standard(tag: i32) -> Self;
    pub fn from_parts(branch: &FixBranch, tag: i32) -> Result<Self>;
    pub const fn tag(self) -> i32;
    pub const fn branch_digest(self) -> u32;
    pub const fn is_standard(self) -> bool;
}
impl FixRegistry {
    pub fn branch_of(&self, id: FixId) -> Option<&FixBranchInfo>;
    pub fn branch_named(&self, name: &str) -> Option<&FixBranchInfo>;
    pub fn branch_for_session(&self, sender: &str, target: &str) -> Option<&FixBranchInfo>;
    pub fn branches(&self) -> impl Iterator<Item = &FixBranchInfo>;
    pub fn set_branch_info(&mut self, info: FixBranchInfo) -> Result<()>;
}
```

### The identifier

- **P2-R1. `i64`, not `u64`.** A tag is `i32` in `0..=i32::MAX`, so bit 63
  is never set, every identifier is positive, and `Ord` on the `i64` is the
  packed pair's natural order. The digest zero-extends, so the low half
  compares unsigned.
- **P2-R2. `Copy`, 8 bytes,** `Hash` and `Ord` without touching the heap.
  `FixKey::Id(FixId)` stops borrowing; `next_field_after` takes it by value.
- **P2-R3. `standard(tag)` becomes `const fn`** — the `SmolStr` `Drop` that
  prevented it goes with the field.
- **P2-R4. The branch caches its own digest.** `xxh32` runs once, in
  `FixBranch::from_str`, the only constructor. `text` first so derived
  `Eq`/`Ord` stay text-based; the digest is a function of the text so it can
  only agree. `MAX_LENGTH` stays 23 — the digest sits beside `SmolStr`.
- **P2-R5. `STANDARD`'s digest is a literal,** pinned by a test asserting
  `xxh32(b"standard")` equals it.
- **P2-R6. `from_parts` is a shift and an or,** and stays the one place the
  admissibility rule lives, plus one refusal: a non-standard branch whose
  digest equals the standard branch's, so `is_standard()` stays total.
- **P2-R7. The user-defined tag range is `[5000, 40000)`.** One limit of
  5000, with everything above it claimable, is wrong at the top: the
  specification has resumed assigning tags at and above 40000, so a vendor
  branch claiming one collides with the standard dictionary.

  | tag | whose |
  | --- | --- |
  | `0 ..= 4999` | the specification's — forced to the standard branch |
  | `5000 ..= 39999` | user-defined — any branch may claim it |
  | `40000 ..` | the specification's again — forced to standard |

  Admissibility is `branch.is_standard() || (5000..40_000).contains(&tag)`.
  Two constants replace the one; the refusal names both and the tag.
- **P2-R8. That is a surface change.** The single limit is public, asserted
  in the FIX tests, re-exported by both bindings, and quoted in four places
  on the FIX page including runnable Python and JavaScript examples that
  assert it equals 5000. Replace every one; leaving the old constant beside
  the new bounds is the second path N3 forbids.

### What the branch text costs

The digest is one-way, so **a bare `FixId` cannot name its branch**.

- **P2-R9. `FixId::branch()` is deleted.** Its callers all hold the owning
  field and read `fix:branch` from it, where the text lives.
  `branch_digest()` replaces it where only identity matters.
- **P2-R10. `Display`** renders `standard:35` for the standard branch and
  `#7f3a1c02:5001` — digest in lowercase hex — for any other. `from_str`
  still accepts `cme:5001`: it has the text and hashes it. The module
  doctest asserting a `branch:tag` round trip moves with it.

### The branch is a dialect

- **P2-R11. The registry keeps a branch table keyed by digest,** filled on
  insert, so every refusal it raises still names `cme:5001`. Only an
  identifier held outside a registry renders as hex.
- **P2-R12. The table holds what a branch declares.** A non-standard branch
  is one counterparty's dictionary, and three facts travel with it: the FIX
  version it speaks by default, and the `SenderCompID` / `TargetCompID` pair
  identifying its session. A `.cfb` states all three on its root element and
  a vendor orchestration states the version; today all three are dropped and
  rediscovered per message. `FixBranchInfo` is the entry; the bare
  `FixBranch` stays the identity inside it.
- **P2-R13. Everything beyond the name is optional,** and a branch declaring
  nothing is today's behaviour. The standard branch declares nothing: it is
  the specification, not a counterparty.
- **P2-R14. Comp ids are stored as they arrive, compared ASCII-folded.**
  `BANKX` and `bankx` are one session.
- **P2-R15. A branch-digest collision is a typed conflict at insert,**
  naming both spellings and the digest — a stated failure rather than two
  dictionaries silently aliasing.
- **P2-R16. The branch table persists as one manifest beside the trees.**
  The store descends only the two trees, so a leaf beside them is never
  listed (the same property the provenance manifest uses, P6-R8). One
  canonically rendered JSON document ordered by branch name; read back on
  open. **Absence is the empty table** — a dictionary written before this
  phase loads with bare names and behaves as today. A manifest naming a
  branch no field belongs to is a typed error naming it.
- **P2-R17. The process-wide default registry *is* the branch singleton.**
  Parsing needs one shared place to resolve a branch from and the repository
  already has one. Hang the table off it; every reader resolves through it
  without threading anything.

### The indexes

- **P2-R18.**

  ```rust
  ids, alternate_ids:  HashMap<FixId, usize, BuildHasherDefault<Mix>>
  names, aliases:      HashMap<u64,   usize, BuildHasherDefault<Mix>>
  positions_by_id:     Vec<usize>   // ordered by FixId, iteration only
  ```

  An identifier lookup hashes nothing: the key *is* the id.
- **P2-R19. `Mix` finalizes, it does not pass through.** The packed high
  bits are the tag, under 65536 for nearly every field, so the top bytes are
  near-constant — and hashbrown takes its control byte from the top bits. A
  raw pass-through puts every standard field in one control-byte class.
  `Mix` is one multiply-xor-shift finalizer.
- **P2-R20. Name and alias keys stay text,** hashed per probe: ASCII-fold
  into the xxhash streaming state in stack-sized chunks so no length
  allocates, seeded with the branch's `xxh32` so a name cannot be found under
  another branch, with a distinct constant seed per index.
- **P2-R21. A name-digest collision is a loud refusal, never a wrong
  answer.** Two names mapping to one `u64` would silently overwrite.
  `insert` verifies the field at an occupied key really holds it and returns
  a typed conflict otherwise; reads re-check, so a collision degrades to a
  miss. Identifier keys need no check — a `FixId` *is* the key.
- **P2-R22. Ordered iteration keeps its own structure.** The cursor, the
  iterator, `Debug`, `PartialEq` and `write_into` need an ordered walk a hash
  map cannot give. `positions_by_id` is kept sorted by binary-search insert:
  `O(n)` per insert on a dictionary built once and read forever, against
  `O(log n)` node chasing on every read.
- **P2-R23. Order becomes tag-major.** Within one branch it is unchanged, so
  `write_into` still produces **byte-identical shards** — a shard folder is
  one branch, a shard file one `tag / 100` bucket. What moves is the
  cross-branch walk: vendor fields interleave among standard ones by tag.
  Update the "orders branch-major" sentence, the cursor doc and the iterator
  docs, and assert the new order rather than leaving it to whichever map
  iterates first.
- **P2-R24. Binding impact is small and mechanical.** Both bindings only
  parse an id from text and hold one as a cursor; neither renders one back,
  so `Copy` makes those by-value. The one real change is P2-R8: both
  re-export the tag limit and must re-export the two bounds instead.

### Decided

- **A bare `FixId` cannot name its branch** (P2-R9/R8/R9). *Rejected:* a
  process-wide branch intern table so a bare id could render itself — it
  buys prettier `Debug` for a global lock or a leak on the hot path, and the
  registry already knows every branch it holds.
- **One singleton, not two** (P2-R17). *Rejected:* a separate global branch
  registry with its own lock and install path. Two process-wide tables that
  must agree about which dialects exist is two sources of truth and a
  lock-ordering question; a branch has no meaning apart from the fields that
  carry it.

### Tests

1. Packing round trip at `0`, `4999`, `5000`, `39999`, `40000`, `i32::MAX`.
2. Admissibility across the range (P2-R7): a vendor branch refused at
   `4999` and `40000`, admitted at `5000` and `39999`; standard admitted
   everywhere; the refusal names both bounds.
3. `standard(tag)` in a `const` context; the pinned `xxh32(b"standard")`.
4. A branch-digest collision refused, both spellings named; a name-digest
   collision refused.
5. Ordering across tags and across branches.
6. `write_into` byte-identical to a shard written before the change — the
   manifest is a new leaf (P2-R16).
7. No manifest loads with bare names and behaves as before; a manifest
   round-trips canonically; one naming a branch no field belongs to is a
   typed error naming it.
8. `branch_for_session` answers the dialect declaring a `(sender, target)`
   pair, folded, and `None` when none does (P2-R14).
9. Every existing registry test passes, except vendor-branch `Display` and
   cross-branch iteration order.
10. Lookups still allocate nothing; `Mix` control-byte spread over the
    committed dictionary.

**Bench.** Re-run the FIX bench and **replace** the published table. To beat:
32.3 ns primitive tag hit, 93.1 ns nested, 65.8 ns alternate, 72.2 ns miss,
128.1 ns vendor identifier over 1034 fields, 81.8 ns name hit.

---

## Handoff

**From Phase 1.** Phases 3, 4 and 6 take `Version`: `MIN`/`MAX` as bounds of
the value space (P1-R6) — Phase 3 does *not* map "FIX Latest" onto `MAX`, it
resolves that label to the real version and EP the dictionary carries
(P3-R2); `FromStr` accepting all three qualifier forms (P1-R2), since the
FIX layer strips a `FIX.` prefix and hands the rest straight in; `Ord` as
the only ordering contract (P1-R5), which every version filter leans on.

**From Phase 2.** Phase 7 takes `FixId` as a `Copy` 8-byte key that is its
own hash key (P2-R1, P2-R18), which makes `FixEntry::id()` a shift-or
(P7-R3); `branches()` and `branch_for_session` (P2-R11, P2-R12) for branch
inference (P7-R21); `FixBranch`'s cached digest (P2-R4), which
`FixEntry.branch` stores reinterpreted (P7-R4).
