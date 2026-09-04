# Phase 1 — `Version`: a generic value, datatype, scalar and field

**Goal.** A generic version value - major, minor, further numeric parts, an
optional qualifier - with its `DataType`, `Scalar` and `Field` support.

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

## Handoff

Phases 3, 4 and 6 all take `Version` from here. What they rely on:

- `Version::MIN` / `Version::MAX` as bounds of the value space (`P1-R6`).
  Phase 3 does *not* map "FIX Latest" onto `MAX`: it resolves that label to
  the real version and extension pack the dictionary carries (`P3-R1b`).
- `FromStr` accepting all three qualifier forms (`P1-R2`), because the FIX
  layer strips a `FIX.` prefix and hands the rest straight in.
- `Ord` being the only ordering contract (`P1-R5`), which every version
  filter in Phases 3, 4 and 7 leans on.
