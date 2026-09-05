# Messages — explicit halves, entries, and the readers

**Goal.** Build a typed, lossless `FixMsg` from key/value pairs, from FIX
text, or from a ULBridge body — then answer the handful of facts a reader
actually asks it for.

**Depends.** Phase 2 (identifier and branch table), Phase 3 (version),
Phase 6 (a dictionary), Phase 8 (packed side and message type, for Phase 9).

> Read `00-contract.md` first: `N1`–`N7`, `L1`–`L2`, precedence, done-when.

---

## Phase 7 — explicit halves, `FixEntry`, `from_pairs`, the text readers

**Surface.** A new entry module inside the FIX module; the registry (public
halves); the message (entries, the builder, three readers). Tests, the
counting-allocator target, a new FIX benchmark group, the FIX page.

**Never.** Write a second parser for anything. The readers split and
delegate; `from_pairs` is the only builder; `Field::scalar` is the only
value contract; the fold is the crate's one fold (P4-R5.2).

### Contract

```rust
impl FixRegistry {
    pub fn get_primitive_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Option<&Field>;
    pub fn primitive_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Result<&Field>;
    pub fn get_nested_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Option<&Field>;
    pub fn nested_field<'k>(&self, key: impl Into<FixKey<'k>>) -> Result<&Field>;
}

pub struct FixEntry {
    pub tag: i32,              // 0 when the key named no field
    pub branch: Option<i32>,   // xxh32 of the branch; None is standard/unresolved
    pub key: SmolStr,          // the key as it arrived; always present
    pub value: SmolStr,        // the value as it arrived; always present
}
impl FixEntry { pub fn id(&self) -> Option<FixId>; }

impl FixMsg {
    pub fn entries(&self) -> &[FixEntry];
    pub fn anomalies(&self) -> impl Iterator<Item = FixAnomaly<'_>>;
    pub fn into_bytes(&self, sep: u8) -> Vec<u8>;
    /// The same, as text; refuses a message carrying a non-printable value.
    pub fn into_text(&self, sep: char) -> Result<String>;

    /// The version this message is expressed in, read back from the fields
    /// that declare it. Derived, never stored (N4).
    pub fn version(&self) -> Option<Version>;
    /// The same message expressed in another version, by lineage alone.
    pub fn convert_into(&self, target: &Version) -> Result<Self>;

    pub fn from_pairs<'a, I>(
        registry: Arc<FixRegistry>, entries: I, branch: Option<&FixBranch>,
        source_version: Option<Version>, target_version: Option<Version>,
    ) -> Result<Self> where I: IntoIterator<Item = (&'a [u8], &'a [u8])>;

    pub fn from_text(text: &str) -> Result<Self>;      // a log line, inferred
    pub fn from_fixtext(
        registry: Arc<FixRegistry>, body: &[u8], sep: u8, branch: Option<&FixBranch>,
        source_version: Option<Version>, target_version: Option<Version>,
    ) -> Result<Self>;
    pub fn from_ultext(
        registry: Arc<FixRegistry>, body: &[u8], branch: Option<&FixBranch>,
        source_version: Option<Version>, target_version: Option<Version>,
    ) -> Result<Self>;
}
```

`FixMsg` gains one field: `entries: Vec<FixEntry>`.

### The halves

- **P7-R1.** The registry's two private position lookups each hard-code the
  same four-way probe — primitive canonical, nested canonical, primitive
  alternate, nested alternate. Expose the halves and compose: `get_field`
  becomes `get_primitive_field(key).or_else(|| get_nested_field(key))`. Each
  half accessor probes both tiers, canonical first. The by-tag, by-id,
  by-name and by-path accessors redirect the same way; none keeps a chain of
  its own.
- **P7-R2. Not tidying.** A transcriber resolving a wire tag wants a scalar;
  today an unknown tag pays all four probes — 32.3 ns hit against 72.2 ns
  miss. `from_pairs` asks only `get_primitive_field`.

### `FixEntry`

- **P7-R3.** `id()` folds tag and branch into a `FixId` with one shift-or
  (P2-R1), so a resolved entry addresses the registry without hashing.
- **P7-R4.** `branch: None` means the standard branch *and* "not resolved
  yet" — why this is not simply a `FixId`. `Option<i32>` costs 8 bytes, not
  4: `0` is a legal digest, so there is no niche.
- **P7-R5. A pair is bytes, not text.** FIX is a byte protocol: a
  `data`-typed value may hold anything, including the separator (P7-R72) and
  bytes that are not UTF-8 at all — a signature, an encrypted block, a
  vendor's XML in some other encoding. So `from_pairs` takes
  `(&[u8], &[u8])`, the readers hand it byte slices, and a separator is a
  `u8` rather than a `char`, which cannot be multi-byte by accident. Where
  bytes must be held as text, P7-R6 says how.
  Everything the parser must *decide* is ASCII by construction — a tag is
  digits, a rendered key folds on ASCII case and ASCII separators (P7-R14) —
  so the whole key path runs on bytes with no UTF-8 validation at all, which
  is faster as well as more correct.
- **P7-R6. Bytes become text by a lossy decode, in one pass, with no
  pre-check.** Where a byte slice has to be held as text, convert it and let
  the conversion decide: valid UTF-8 borrows through unchanged, and a byte
  that is not gets the replacement character. Do **not** scan first to ask
  whether it will succeed — a validity pass over every key and value, to
  answer a question the conversion already answers, is the cost this brief
  moved to bytes to avoid (P7-R5). Nothing is refused for being
  un-decodable, and nothing is dropped: a mangled key is kept, mangled, and
  simply resolves to no field.
- **P7-R7. The value owns.** A `FixMsg` holds its entries and outlives the
  text it was read from, so a borrowed value would force `FixMsg<'a>` on
  every caller and on both bindings, which hold one across an FFI boundary.
  `SmolStr` is that value, as everywhere else in this crate: 23 inline bytes
  cover a side, a price, a symbol and a 21-byte `UTCTimestamp`, so the
  common entry allocates nothing. Readers still split into borrowed
  `(&[u8], &[u8])`; the single materialization is in `from_pairs`.
- **P7-R8. Every field of an entry is present; `tag` is `0` when the key
  named none.** The key is recorded as it arrived — `"54"` for a tag-keyed
  pair, `Side` for a named one, `VenueOwnThing` for one no dictionary
  explains — and `entries` is the wire record, so it says what came in
  unconditionally. `0` is a safe sentinel rather than a hack: the
  specification numbers tags from `1`, so no field can ever carry it, and
  yggfin's own entries use exactly this. A key that literally parses to `0`
  is therefore treated as no tag, which is what it is — tag `0` names no
  field either way, so the sentinel and the parse agree.
- **P7-R9. `id()` answers `None` at tag `0`,** which is the one place the
  sentinel is read, so no other caller compares against it.
- **P7-R10. The value is typed from the raw bytes, never from the entry's
  text.** Typing happens once, in `from_pairs`, through `Field::scalar`,
  and it reads the `&[u8]` the reader split — *before* P7-R6's lossy decode,
  which exists only to fill the entry. Type from the decoded text and a
  `data` field's bytes are destroyed on the way in; the ordering is the
  whole rule.
- **P7-R11. A `data` field's bytes live in the row; its entry is a lossy
  view of them.** `SmolStr` is UTF-8 and a signature is not, so the entry
  holds the decode (P7-R6) while the row holds the bytes — which is where
  that field's datatype puts them anyway (`binary`, P6-R13). `into_bytes`
  reads such a tag from the row, every other tag from its entry, so nothing
  is lost and no type is invented. An entry whose text is lossy is not the
  authority for its field, and `anomalies()` says which entries those are.

### `FixMsg` carries its entries

- **P7-R12. `entries` is not the row restated** — the one place this brief
  admits two facts about one thing (N4). The row is the *interpretation*:
  values typed, codes translated, names canonical, groups nested, header
  ordered. `entries` is what *arrived*: raw text, arrival order,
  untranslated, including pairs no dictionary explained. Neither derives
  from the other — a translated `4` cannot say whether the wire carried `4`
  or `PercentageWaivedCashDiscount` — so lossless re-emission is impossible
  from the row alone, which is what makes `into_text` and the round trip
  (P7-R40) work. **Say exactly this in the doc comment**, or a reader will
  assume one is redundant.
- **P7-R13. Populated by `from_pairs`,** and so by all three readers; empty
  for a message built from a schema and a value, in which case `into_text`
  and `anomalies` fall back to the row and say so.

### What a key may be

| key | means |
| --- | --- |
| `54`, `"54"` | a tag, through the strict `parse_tag`, which refuses `+35` and `3x` |
| `Side`, `side`, `SIDE`, `" Side "` | a name, trimmed and folded |
| `msg_type`, `msg-type`, `Msg Type` | the same name: separators fold away too |
| `Instrument.Symbol` | a path, through the existing by-path accessor |
| `PartyID[0]`, `PartyID[1]` | one field, two occurrences, in order |
| `NoPartyIDs[0].PartyID` | a group entry: which group, which occurrence, which member |
| `VenueOwnThing` | an unknown name, **kept** |
| `""`, `"   "` | dropped |

- **P7-R14. Separator folding.** The registry folds ASCII case only today,
  so a renderer emitting `msg_type` or `Msg Type` misses a field that
  exists. Extend the FIX name fold to drop `_`, `-` and space — the fold
  `LOGICAL_NAMES` already uses, so one rule serves both. No two FIX fields
  differ only by a separator or by case; assert that over the committed
  dictionary as the test that lets the change in. This fold is what makes
  P3-R7 free: with it, lower-casing the stored name costs no caller the
  spelling it was written with.
- **P7-R15. An unknown *tag* is kept** as a nullable `utf8` field under its
  decimal spelling — the existing rule.
- **P7-R16. An unknown *name* is kept too,** under its own spelling. Every
  venue sends fields no dictionary has, and dropping them loses data.
  Resolved fields come first in the root, unknown ones after, so the schema
  stays stable when a dictionary later learns the name.
- **P7-R17. Built field names are lower-cased** (P3-R7), including the
  name an unknown key is kept under — `venueownthing`, not `VenueOwnThing`.
  The **entry keeps the spelling that arrived** (P7-R7), because `entries`
  is the wire record and the row is the interpretation (P7-R11); that is
  where the venue's own casing survives, and — with P7-R5 — its own bytes
  too.
- **P7-R18. An empty value drops its pair.** `54=` is a malformed message,
  not an absent side.
- **P7-R19. Order and repetition are the message.** A tag appearing twice
  stays two entries in input order; a map keyed by tag would lose a
  repeating group. P3's duplicate-name suffix rule names the later children.

### Inferring version and branch

- **P7-R20. The *source* version, when the caller names none** — the version
  the arriving message is written in, which decides how its tags are read.
  Each step is a FIX rule, not a heuristic.
  1. **Tag 1128 `ApplVerID`** — the application version: `0`=FIX27,
     `1`=FIX30, `2`=FIX40, `3`=FIX41, `4`=FIX42, `5`=FIX43, `6`=FIX44,
     `7`=FIX50, `8`=FIX50SP1, `9`=FIX50SP2, `10`=FIXLatest — and `FIXLatest`
     resolves to the dictionary's `newest()` pair, never a sentinel
     (P3-R2). It wins because under FIXT.1.1 the session version says
     nothing about the application version. Symbolic spellings (`FIX44`) go
     through `code_by_name`.
  2. **Tag 8 `BeginString`** — `FIX.4.0` … `FIX.4.4` give `4.0` … `4.4`.
     `FIXT.1.1` is a session version and names no application version, so it
     **falls through** rather than being taken literally.
  3. **The branch's own default version**, once the branch is known
     (P2-R12). A dialect that declares it is stating what its counterparty
     speaks — better evidence than a dictionary-wide default.
  4. Otherwise the dictionary's `newest()` — the real newest version and EP
     it holds. Never `Version::MAX`: a sentinel compares wrongly against a
     field genuinely dated at the newest version, and is wrong again the
     next time an EP lands.
- **P7-R21. The *target* version defaults to the source.** It is the
  version the built message is expressed in. Absent, no conversion happens
  and the reader behaves as if there were one version parameter — so the
  common call is unchanged and conversion is opt-in. Given, the parse reads
  the wire at the source and answers a message at the target: a venue that
  speaks 4.2 can be normalized to the dictionary's newest on the way in,
  which is the whole point of two parameters instead of one.
- **P7-R22. Branch, when the caller names none.** The first step is identity,
  not inference.
  1. **The session names the dialect.** `SenderCompID(49)` and
     `TargetCompID(56)` are in every header, and P2-R12 bundles that pair
     onto the branch, so `branch_for_session` answers exactly — one lookup,
     no counting. **Try both orders**: a dictionary declares the session
     from its own side, so an inbound message carries the pair reversed, and
     matching one order silently misses half the traffic.
  2. Resolve every entry in the standard branch; nothing missed means
     standard, with no second pass — one probe per entry.
  3. Otherwise retry only the *missed* tags against each branch the registry
     holds and take the branch resolving the most; a tie goes to the lowest
     branch name; a branch resolving none is never chosen and its tags stay
     unknown.

  A caller who passes a branch gets it, with no guessing. Branch resolution
  runs **before** version step 3, and the two are one pass: 49, 56, 1128 and
  8 are all in the header.

### Building the message

- **P7-R23.** The field is the registry's, cloned, with
  `name_at(source_version)` and `dtype_at(source_version)` where a lineage
  exists, its own name and datatype where it does not. The target version,
  when it differs, is applied afterwards by the converter (P7-R36) — never
  by a second resolution here.
- **P7-R24.** Non-null in this message's schema, because the value is there.
- **P7-R25.** The value passes through the code set first:
  `code_value_at(&source_version, entry.value).unwrap_or(entry.value)`, so
  `CommType=PercentageWaivedCashDiscount` stores `4` and
  `MsgType=NewOrderSingle` stores `D`, while an unexplained spelling is
  carried through untouched (P4-R7).
- **P7-R26.** Then `field.scalar(Scalar::from(translated))`, with P7-R30 on
  refusal. Nothing re-checks what `scalar` answered.
- **P7-R27.** Order is `STANDARD_HEADER_TAGS`, then the body in entry order,
  then `STANDARD_TRAILER_TAGS` — flat, no `StandardHeader` Struct (P6-R16).
- **P7-R28.** `with_registry` finishes it, so existing validation and
  canonicalization are not bypassed.
- **P7-R29. An empty dictionary is a supported input, not an error.** With
  nothing resolvable, every name stays a name, every tag its decimal
  spelling, and lookup by name still finds what was put in — which is what
  makes this usable on a venue whose dictionary is not loaded yet.
- **P7-R30. A value that will not type is null, not a failure.**
  `field.scalar` refuses a value the datatype cannot hold — a `BodyLength`
  that is not digits, a mangled timestamp — and that must not cost the
  message: (a) the row's field is **null**; (b) the raw text stays in
  `entries` as it arrived; (c) the refusal is reported through
  `anomalies()`; (d) `from_pairs` still answers `Ok`. A parse error is
  raised only for input that is not a message at all. A null nobody can
  explain is worse than the value that actually arrived.

### Converting between versions

- **P7-R31. `convert_into` is lineage-driven and nothing else.** Every
  question it asks is one the field already answers: `defined_at`,
  `name_at`, `dtype_at`, and the code set's version filter. It evaluates no
  expression, consults no mapping table, and invents no value. That is what
  keeps it inside this phase rather than the CBlock brief's normalization
  layer (P3-R16).
- **P7-R32. It borrows and answers a new message.** The original stays
  valid — a caller often needs both. Target equal to source is a clone, and
  cheap: check before walking.
- **P7-R33. Per field, in this order.**

  | at the target | what happens |
  | --- | --- |
  | defined, same name and type | carried over untouched |
  | defined, renamed | the child takes `name_at(target)` |
  | defined, retyped | the value goes back through the value contract at the new type; a refusal nulls it (P7-R30) |
  | **not defined** | dropped from the row, kept in `entries`, reported through `anomalies()` |
  | a code not valid at the target | the raw value is kept, and an anomaly is reported — never a substituted code |

- **P7-R34. The fields that declare the version are rewritten to it.**
  `BeginString(8)` and `ApplVerID(1128)`, where the message carries them, so
  a converted message does not lie about what it is. `FixMsg::version()`
  reads them back, which is why it is derived and not stored (N4).
- **P7-R35. `entries` are regenerated from the converted row,** so
  `into_text` emits the target dialect — which is the reason to convert at
  all. The conversion is therefore *not* lossless in the `entries` sense:
  the wire record of the source is replaced by the wire record of the
  target, and anything that could not convert is carried as an unknown entry
  so nothing is silently dropped (P7-R16). Say so in the doc comment; the
  round-trip guarantee (P7-R40) applies to the converted message.
- **P7-R36. Parsing at a target equals parsing then converting.**
  `from_pairs(.., source, target)` answers what
  `from_pairs(.., source, source).convert_into(target)` answers. State it,
  test it, and implement it once — the reader calls the converter rather
  than growing a second conversion path (N3).

### Decided

- **Bytes in, `SmolStr` stored, lossily, and no new type.** The door is
  bytes, so nothing is validated up front and no legal message is refused;
  the entry is the crate's own inline string, so the common entry allocates
  nothing; and the one thing that string cannot hold — a `data` field's
  bytes — is held by the row, which its datatype already types as `binary`
  (P7-R10). *Rejected:* a bespoke small-bytes value — a type the crate does
  not otherwise need, to carry four tags that are usually absent.
  *Rejected:* `Arc<[u8]>` in the entry — it allocates on every entry and
  throws away the inline win P7-R7 exists for. *Rejected:* refusing an
  un-decodable key or value — that drops data for a malformation the
  replacement character already describes.
- **The readers are byte-native and `from_text` is the one text door.** A
  `&str` reader that splits and then converts to bytes pays UTF-8 validation
  for nothing. `&str` is `&[u8]` for free, so the text convenience costs one
  coercion and the byte path stays the real one.

### Groups

- **P7-R37. In scope, because the key carries the location.**
  `NoPartyIDs[0].PartyID` states group, occurrence and member, so no grammar
  is needed — and a group is a List of a non-null `item` Struct with a
  by-path setter that writes into one. `from_pairs` builds **real nesting**
  from indexed keys, where yggfin keeps a flat `comp` string because its
  field model has no list of structs to put it in.
- **P7-R38. Out of scope: inferring a group from repetition alone.** Bare
  `448=A`, `448=B` with no index and no group key produces two sibling
  occurrences of `PartyID`, not a reconstructed `NoPartyIDs`. That needs the
  message grammar, which is the CBlock brief's. Say so in the module docs.

### Encode direction

- **P7-R39. Wire spellings belong to the FIX layer, never to the generic
  value contract.** yggfin pins them: a float is never exponent notation
  (`1e-7` writes `0.0000001`), a `UTCTimestamp` is
  `20260821-10:30:00.123456`, a date `20260821`, a time `10:30:00.000000`, a
  boolean `Y` or `N`. **Verify first** whether the value contract accepts a
  `Scalar::String` for `Boolean` and the temporals at all; where it does
  not, the FIX layer parses the wire spelling into the right `Scalar` first;
  where it does, check the spelling it accepts is FIX's. Either way the
  generic contract learns no FIX spelling — `LOGICAL_NAMES` is deliberately
  a *type* table, not a *value* one.
- **P7-R40. The two ways in must agree.** `into_bytes(sep: u8)` is the
  emit, since a message that carries a `data` field cannot round-trip
  through text; `into_text` stays as the convenience for a printable message
  and refuses one that is not. The invariant is
  `from_fixtext(built.into_bytes(b'|'), b'|') == built`, and it is a test in
  this phase.

### The three readers, one builder

`from_pairs` borrows both halves of a pair, so a splitting iterator feeds it
with no copy. Each reader splits, rewrites its dialect into the key forms
`from_pairs` understands, and hands the iterator over. One nesting builder,
one fold, one code translation under all three.

- **P7-R41. A row is a log line, so find the frame before reading it.** A
  capture line is a message wrapped in whatever the process printed around
  it: `sending >> 8=FIX.4.2|…|10=203| << queued seq=1092`. Locate the frame
  first — `8=` at a token boundary, else `35=`, else the first pair-shaped
  run — and read from there; everything before is a prefix, and P7-R68 ends
  the message at the checksum, so everything after is noise. Reading from
  byte zero would take `sending >> 8` as the first key and pick the wrong
  dialect on nearly every real line.
- **P7-R42. The dialect is decided once, from the frame, and never
  re-sniffed.** Take the bytes before the first `=` **of the frame**: all
  ASCII digits means `from_fixtext` — separator SOH (`0x01`) when the body
  holds one, else `|` (`0x7C`); anything else means `from_ultext`. Once
  decided it holds for the whole body, so a `#` or a `<` inside a *value* is
  part of that value: `58=quoting #A=1 and #B=2` is one Text field, not the
  start of a UL group — the same rule P7-R61 gives `;`. `from_text` takes a
  `&str` because a log line is text and `&str` is `&[u8]` for free.
- **P7-R43. A FIX frame carrying named keys needs no third reader.** A line
  can be a FIX envelope whose body mixes numeric and named keys —
  `8=FIX.4.4|35=D|11=ORDER-1|SYMBOL=AAPL|SIDE=1|10=000`, or the same with
  `#`-marked ones under `35=UL`. A classifier may call that a protocol of
  its own; this layer does not, because `from_pairs` already takes a tag key
  or a name key (P7-R14) and the frame already said which reader splits. One
  reader, mixed keys, no third path (N3).
- **P7-R44. No message type is `unknown`, and nothing is skipped.** A
  message is typed by `35=` in a FIX frame or `MSGTYPE=` in a UL one. Where
  a row carries neither, **build it anyway**: every pair that parsed becomes
  a field, the entries record the whole row, and the root is named
  `unknown`. Nothing is dropped and no row is refused — fill what can be
  filled, and let the type say what could not. `unknown` is a safe name for
  the same reason `0` is a safe tag (P7-R8): every `MsgType` the code set
  declares is one or two characters, so none can collide with it, and a
  caller separating real messages from log noise filters on the type rather
  than on a rule buried in the reader.
- **P7-R45. A truncated `BeginString` resolves to no version.** `8=FIX4` is
  a real thing in real captures; the nearest version is a guess, and this
  brief does not guess (P7-R26 already falls through for `FIXT.1.1`). It
  falls through to the next tier, and the frame is still found: a truncated
  BeginString is a bad version, not a bad message.
- **P7-R46. A UL body may be separated by spaces.** `After Enrichment ->
  ACCOUNT=ACCT-000117 CLIENTID=MCFP2 VENUE=XPAR` is a UL row with a space
  where another writes `|`. The `ultext` separator is whichever of `|` or
  space the frame uses, chosen once by P7-R42 and then held — never both, or
  a value containing a space would split.
- **P7-R47. `from_fixtext` takes a body and a `u8`.** Split on `sep` with
  `memchr`, then each segment at its first `=`. A trailing empty segment is
  tolerated — a wire message ends with the separator. A segment with no `=`
  is dropped, as an empty key is. Duplicate tags stay in arrival order.
  Every key and value is a slice of the input, and none of it is validated
  as UTF-8 (P7-R5).

#### `from_ultext`

ULBridge writes names, not tags, and packs a repeating group into one pair:

```text
#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=SYNTH-01<sub>PARTYIDSOURCE=shortcodeid<sub>PARTYROLE=executingsystem|
```

where `<sub>` is `\x04\x03`, EOT then ETX.

- **P7-R48.** Pairs split on `|`, then at the first `=`. Keys are names in
  any case and reach their field through the P7-R14 fold.
- **P7-R49.** A key opening with `#` names a group: `#NOPARTYIDS=1` is the
  counter; `#NOPARTYIDS[0]=…` is entry 0, its *value* a run of member pairs.
- **P7-R50.** Members inside an entry split on `\x04\x03`.
- **P7-R51. And sometimes on nothing at all.** ULBridge may omit the
  separator after the first member while keeping the index:
  `#NoPartyIDs[0]=PartyID=P-1PartyIDSource=DPartyRole=3`. Split by scanning
  for the next member name the group's own field declares, taking the
  **longest declared match** so `PartyIDSource` beats `PartyID`. Only that
  group's declared members are candidates, which bounds the scan and keeps
  the result explainable.
- **P7-R52.** Residue that will not split stays as one unknown key,
  verbatim. Never dropped, never fatal.
- **P7-R53.** Indices may be partial or out of order — `[2]` before `[0]`,
  with gaps. Occurrences are built by index, not arrival; a gap is null.
- **P7-R54.** It rewrites into the key forms `from_pairs` takes —
  `#NOPARTYIDS[0]=PARTYID=…` becomes `("NoPartyIDs[0].PartyID", "…")` — and
  builds no tree of its own. Values translate like any other, so
  `PARTYIDSOURCE=shortcodeid` stores `P` and `PARTYROLE=executingsystem`
  stores `16`, while `PARTYROLE=orderoriginatorsystem` is stored verbatim
  under tag 452.

#### Reading a stream of rows

A capture is millions of lines, and calling a singular reader per line
re-does per-message what is constant for the whole run. One type carries
that state.

```rust
pub struct FixReader { /* registry, pinned branch and versions, one split buffer */ }

impl FixReader {
    pub fn new(registry: Arc<FixRegistry>) -> Self;
    pub fn branch(self, branch: &FixBranch) -> Self;          // pin, no inference
    pub fn source_version(self, version: Version) -> Self;    // pin
    pub fn target_version(self, version: Version) -> Self;    // pin

    pub fn texts<'a, I>(&'a mut self, rows: I) -> impl Iterator<Item = Result<FixMsg>> + 'a
    where I: IntoIterator<Item = &'a [u8]> + 'a;
    pub fn fixtexts<'a, I>(&'a mut self, rows: I, sep: u8) -> impl Iterator<Item = Result<FixMsg>> + 'a
    where I: IntoIterator<Item = &'a [u8]> + 'a;
    pub fn ultexts<'a, I>(&'a mut self, rows: I) -> impl Iterator<Item = Result<FixMsg>> + 'a
    where I: IntoIterator<Item = &'a [u8]> + 'a;
    pub fn rows<'a, I, R>(&'a mut self, rows: I) -> impl Iterator<Item = Result<FixMsg>> + 'a
    where I: IntoIterator<Item = R> + 'a, R: IntoIterator<Item = (&'a [u8], &'a [u8])>;
}
```

- **P7-R55. Lazy, one row at a time, with backpressure** — the repository's
  standing rule for streaming iterators. Nothing is collected up front, the
  source is pulled only as the consumer pulls, and a `FixReader` over an
  unbounded source runs in bounded memory.
- **P7-R56. A row almost never fails, and a failure never ends the run.**
  Nearly everything a capture holds becomes a message: a row with no type is
  `unknown` (P7-R44), a value that will not type is null (P7-R30), a group
  that will not split stays whole (P7-R47). What is left — input that is not
  a row at all — is an `Err` item carrying it, and the stream continues,
  because one corrupt line must not end a run over ten million. A caller who
  wants to stop at the first failure already has the vocabulary —
  `collect::<Result<Vec<_>, _>>()`, or `take_while` — and the reader grows
  no mode for it. The posture runs the whole way down: a bad value never
  costs a message, a bad message never costs the stream.
- **P7-R57. Pinning is what the type buys.** `branch` and the two versions
  pinned once skip P7-R20 and P7-R22 for every row — and a capture is one
  session, so pinning is the normal case, not an optimization. Unpinned, the
  reader infers per row exactly as the singular readers do, and answers the
  same messages.
- **P7-R58. Only the split buffer is reused.** The three vectors of P7-R80
  are the message's own and move into it, so they cannot be pooled without
  giving `FixMsg` a borrow. What the reader holds across rows is the
  registry handle, the pinned resolutions, and one buffer the splitters
  write pairs into and the builder reads back. Pooling the rest would end in
  a `FixMsg<'a>`, which P7-R7 refuses for the same reason.
- **P7-R59. The singular readers are the stream of one.** `from_text` and
  its two siblings are a `FixReader` over one row, so there is one parsing
  path and no chance of the two disagreeing. Assert it: a row read singly
  and the same row read through the reader answer equal messages.
- **P7-R60. There is no `convert_all` and no `lift_all`.** Converting a
  stream is `.map(|m| m?.convert_into(&target))` and lifting one is
  `.map(|m| m?.lift())`; neither holds state across items, so neither earns
  a symbol (N1, N5). Only the readers do, because only they have something
  to keep.

#### Token rules both readers obey

Each row is a case in yggfin's `test_message.py` or `test_transcribe.py` — a
line some venue really sent.

| # | rule | why |
| --- | --- | --- |
| P7-R61 | A token splits at its **first** `=` only. | `Text=a;b` is one value with a semicolon, not two fields. |
| P7-R62 | `G[0]=M=v` and `G[0].M=v` are one field, two prints. | A group has one shape; two spellings must not make two. |
| P7-R63 | `#` marks where a key **starts**, not which field it is. | `#54=x` is a rendered key spelled with digits, **not** tag 54. |
| P7-R64 | `#A=1#B=2` has no separator: the next `#` ends the previous value. | ULBridge omits separators; the marker is the boundary. |
| P7-R65 | Tag mode is ASCII digits only. | A bracket, dot or `#` means a rendered key, so `453[0]` is never tag 453. |
| P7-R66 | A digit key overflowing `i32` is not a tag. | An epoch-millis key looks like digits; `parse_tag` already drops it. |
| P7-R67 | Trim ASCII whitespace only. | A non-breaking space is part of the value; trimming Unicode returns a tag never sent. |
| P7-R68 | Nothing after `10=<checksum>` is part of the message. | Log lines carry pair-shaped noise after the trailer. |
| P7-R69 | One `a=b` alone is not refused; it is an `unknown` message of one field. | `heartbeat emitted seq=7` is data too, and the type is what says it was not addressed to this reader (P7-R44). |
| P7-R70 | Two values under one key stay two. | It is a group or a rewrite; collapsing picks one, and picking is a guess. |
| P7-R71 | A row with no pairs is an `unknown` message with no entries, not an error. | Empty *input* is a typed error; a line that simply held nothing is `Ok` and says so (P7-R44). |

#### `data` fields are read by length, not by separator

- **P7-R72.** FIX types a field `data` **because its value may contain the
  separator**. `RawData(96)`, `XmlData(213)`, `SecureData(91)` and
  `Signature(89)` each follow a length field — `RawDataLength(95)`,
  `XmlDataLen(212)`, `SecureDataLen(90)`, `SignatureLength(93)` — and that
  length, not the next SOH, says where the value ends. A reader that
  tokenizes first loses the message.
- **P7-R73.** The registry says which tags are `data` (`DataType::Binary`
  after P6-R13), so nothing hard-codes the four pairs.
- **P7-R74.** When the stated length and the next separator disagree, **take
  the separator**: a writer that miscounted has stated two things and the
  delimiter is the safer. Record it through `anomalies()`.
#### `XmlData(213)` that is not XML

The standard calls tag 213 an XML stream; venues put a whole pair-shaped
payload in it — commonly under a vendor `MsgType` of `UL`, where the envelope
carries a handful of session tags and everything that matters is inside 213,
as numeric `tag=value` pairs, ULBridge `NAME=VALUE` pairs, or a mix.

- **P7-R75. The payload's own shape decides, not the message type.** Look at
  the first non-space byte: `<` means XML, and the value stays whole.
  Anything else means pairs, and P7-R41's one-token rule then picks the
  dialect — all ASCII digits before the first `=` is `fixtext`, otherwise
  `ultext`. That is the rule the readers already use, so nothing new is
  guessed, and a venue spelling the envelope with some `MsgType` other than
  `UL` works without a table of venues. `UL` is worth knowing as the common
  case; it is not the trigger.
- **P7-R76. The inner pairs are nested, not flattened.** They go to the same
  builder with their keys prefixed — `11=abc` inside 213 becomes
  `("xmldata.11", "abc")` — so `XmlData.ClOrdID` resolves the way
  `NoPartyIDs.PartyID` does (P7-R35) and no new machinery exists. A value
  inside something belongs to that something, which is P9-R7 seen from the
  other end: flattening would let an inner `Price` answer as the message's
  own.
- **P7-R77. Only tag 213 is descended into.** `RawData(96)`,
  `SecureData(91)` and `Signature(89)` are opaque by intent, and an
  encrypted block that happens to contain an `=` must never be read as
  pairs. 213 is the one the standard documents as a text stream, so it is
  the one exception — named, not generalized.
- **P7-R78. Reformatting is a read; the emit is untouched.** The entry for
  tag 213 keeps the whole payload as it arrived, so `into_bytes` re-emits it
  byte-for-byte and `XmlDataLen(212)` still describes it: no length is
  recomputed and no round trip changes (P7-R12, P7-R38). A payload that will
  not split cleanly stays whole and reports (P7-R79), and nesting past a
  documented guard is refused the same way, so a payload carrying another
  213 cannot recurse without bound.

#### Anomalies are derived, never a second state

- **P7-R79.** A counter disagreeing with the entries it introduces, a group
  that would not split cleanly, a value that would not type — all real, none
  fatal. `anomalies()` derives them on demand by comparing the counter value
  (an ordinary value at its own tag) with the List's length, the way `FixId`
  is derived rather than stored. No error channel on `FixMsg`, nothing to
  keep in step, and a caller who never asks pays nothing.

### Optimization the phase is judged on

- **P7-R80.** One `Vec<FixEntry>`, one `Vec<Field>`, one `Vec<Scalar>`, each
  reserved from the iterator's `size_hint` before the walk. No per-entry
  `String`, no per-entry map. The `Vec<FixEntry>` is the one the message
  keeps: built once, moved in, never cloned.
- **P7-R81.** A resolved entry allocates nothing — integers for `tag` and
  `branch`, `None` for `key`, a value inside `SmolStr`'s inline buffer.
- **P7-R82.** Only `get_primitive_field` is probed for scalars; the nested
  half is reached only for a counter tag.
- **P7-R83.** Header ordering reads a precomputed tag-to-position table, not
  a scan per entry.
- **P7-R84.** The readers copy nothing: every key and value is a slice of
  the input, and splitting uses `memchr`.

### Tests

**Keys and values.**
1. Tag-keyed and name-keyed pairs producing the identical message.
2. `" Side "`, `msg_type`, `msg-type`, `MSG_TYPE`, `Msg Type` all reaching
   their field (P7-R14).
3. `Instrument.Symbol` resolving through the path.
4. `PartyID[0]` and `PartyID[1]` staying two ordered occurrences.
5. `NoPartyIDs[0].PartyID` building a List of one `item` Struct (P7-R37).
6. An unknown name surviving beside a known one, known first (P7-R16); an
   unknown tag kept as `utf8` under its decimal name (P7-R15).
6b. Every built child's name is lower-case, an unknown key included, while
    its entry keeps the arrival casing (P7-R17) — and a `MsgType` value
    keeps its case through all of it (P8-R3).
6c. A `data` field holding bytes that are not UTF-8 — a signature — survives
    the whole path: typed as bytes in the row and re-emitted by `into_bytes`
    from there (P7-R10, P7-R40), while every other value in the same message
    round-trips through its entry.
6d. A key that is not UTF-8 is kept, decoded lossily, resolves to no field,
    and its pair still becomes an unknown child — nothing is dropped and
    nothing is refused (P7-R6).
6e. Every entry carries its key and a tag: `"54"` and `54` for a tag-keyed
    pair, `Side` and `54` for a named one, `VenueOwnThing` and `0` for an
    unknown one — and `id()` answers `None` for the last (P7-R8, P7-R9).
    A line carrying a literal `0=x` also answers tag `0`.
6f. A valid-UTF-8 value is converted without a validity pass of its own and
    without copying, and a `data` field is typed from the raw bytes rather
    than from its lossy text (P7-R6, P7-R9).
7. Empty and blank keys dropped; an empty value dropped (P7-R18).
8. The same pairs built against an empty registry (P7-R29).

**Version and branch.**
9. `ApplVerID` beating `BeginString`; `BeginString="FIXT.1.1"` falling
   through; an explicit `source_version` overriding both (P7-R20).
10. A declared `(sender, target)` pair selecting its dialect directly, in
    the declared order and reversed, folded (P7-R22.1, P2-R14).
11. A branch's declared default losing to a message's own `ApplVerID` and
    winning over `newest()` (P7-R20.3).
12. Tag-count inference picking the vendor dictionary, the tie rule, and an
    explicit branch suppressing inference (P7-R22.3).
13. Tag 32 keyed `LastShares` in a `4.2` message and `LastQty` in a newest
    one, both answering the same value.
14. Header and trailer ordering with a body field interleaved in the input.

**Versions in and out.**
14b. An absent `target_version` changes nothing: the message equals the one
     built with a single version parameter (P7-R21).
14c. A 4.2 wire message parsed with `target_version` at the dictionary's
     newest answers a message whose tag 32 child is named `LastQty`, while
     the same parse without a target names it `LastShares`.
14d. `from_pairs(.., source, target)` equals
     `from_pairs(.., source, source).convert_into(target)` (P7-R36).
14e. `convert_into` with target equal to source is an unchanged clone
     (P7-R32), and `convert_into` on a message with no lineage anywhere
     changes nothing.
14f. A field defined at the source but not at the target is dropped from the
     row, present in `entries`, and reported in `anomalies()` (P7-R33).
14g. A retype whose value no longer fits nulls that field and reports, and
     does not fail the conversion (P7-R33, P7-R30).
14h. A code valid at the source and not at the target keeps its raw value
     and reports, and is never substituted (P7-R33).
14i. `BeginString` and `ApplVerID` are rewritten to the target, and
     `version()` reads the target back (P7-R34).
14j. `into_text` after a conversion emits the target dialect, and
     `from_text` of that answers the converted message (P7-R35, P7-R40).

**Token rules.**
15. Every row of P7-R61…R71, one case each.
16. `#54=x` reaching the field whose *rendered key* is `54`, never tag 54;
    `G[0]=M=v` and `G[0].M=v` answering equal messages.
17. A lone `a=b` answering an `unknown` message of one field (P7-R69), and a
    line holding no pairs answering `Ok` with no entries (P7-R71) — while
    empty *input* is still the typed error.
18. A `data` field whose value contains the separator, read by its length
    field; a miscounted length taking the separator and appearing in
    `anomalies()` (P7-R72, P7-R74).
19. A `BodyLength` of `abc` nulling that field while the raw text stays in
    `entries` (P7-R30).
20. Tag 555 at two nesting levels in one TradeCaptureReport, neither
    guessed.

**`XmlData`.**
20b. A `MsgType=UL` envelope whose 213 holds numeric `tag=value` pairs, and
     one whose 213 holds ULBridge `NAME=VALUE` pairs, both descended into
     and nested under `xmldata` (P7-R75, P7-R76).
20c. A 213 whose first byte is `<` kept whole (P7-R75), and the same content
     under a `MsgType` that is not `UL` still descended into — the shape
     decides, not the type.
20d. An inner `Price` answering as `xmldata.price` and never as the
     message's `price` facet (P7-R76, P9-R7).
20e. `RawData`, `SecureData` and `Signature` never descended into, even
     when their bytes contain an `=` (P7-R77).
20f. `into_bytes` re-emitting 213 byte-for-byte with `XmlDataLen` unchanged
     (P7-R78), and a payload that will not split staying whole and
     reporting.

**Codes.**
21. `("CommType", "PercentageWaivedCashDiscount")` and
    `("13", "percentage_waived_cash_discount")` both storing `4`;
    `("MsgType", "NewOrderSingle")` storing `D`; `("CommType", "4")`
    unchanged; an unexplained spelling stored verbatim.
22. A name added after the message's inferred version refusing to translate.

**Readers and entries.**
23. The ULBridge payload verbatim, with `\x04\x03` and with the separator
    omitted, both producing one `NoPartyIDs` occurrence of four members.
24. `PARTYIDSOURCE` translating while `PARTYROLE=orderoriginatorsystem`
    survives untranslated.
25. Out-of-order and gapped indices (P7-R53); a counter of `2` against one
    entry appearing in `anomalies()` while the message still reads.
26. `entries()` holding every pair in arrival order with the untranslated
    spelling, beside a row holding the translated code (P7-R11); an
    unresolved key with `tag` `None` and `key` set (P7-R8).
27. `from_fixtext` over SOH-separated and `|`-separated captures of one
    message answering equal messages; `from_text` picking the dialect from
    `35=D|…` against `MSGTYPE=D|…`.
28. `from_fixtext(built.into_bytes(b'|'), b'|') == built` (P7-R40), and
    `into_text` refusing a message that carries a non-printable value.

**Capture lines.** One row per shape a real capture holds, asserted for what
the reader builds — `SOH` is `\x01`, and the classifier labels in the last
column are a *consumer's* taxonomy, not a third reader (P7-R43).

| row | reads as | type |
| --- | --- | --- |
| `sending >> 8=FIX.4.2\|9=176\|35=D\|10=203\| << queued seq=1092` | fixtext, framed at `8=`, tail dropped | `D` |
| `recv 8=FIX4^A9=61^A35=0^A10=017^A on session 3` | fixtext, SOH-separated, truncated `BeginString` | `0` |
| `raw 8=FIX.4.4{SOH}9=224{SOH}35=8{SOH}10=118{SOH}` | fixtext | `8` |
| `8=FIX.4.4\|35=8\|58=quoting #A=1 and #B=2\|10=1\|` | fixtext; the `#`s are inside Text(58) | `8` |
| `sending >> 8=FIX.4.2\|35=UL\|#SYMBOL=TTF\|#SIDE=1\|10=044\|` | fixtext frame, `#`-marked named keys | `UL` |
| `8=FIX.4.4\|35=D\|11=ORDER-1\|SYMBOL=AAPL\|SIDE=1\|10=000` | fixtext frame, mixed numeric and named keys | `D` |
| `toBridge #ISINCODE=XX\|#SYMBOL=TTF\|#SIDE=1` | ultext, no FIX frame | `unknown` |
| `ACCOUNT=A1\|MSGTYPE=D\|CLORDID=ORDER-1\|SYMBOL=AAPL\|SIDE=1` | ultext, type from `MSGTYPE=` | `D` |
| `After Enrichment -> ACCOUNT=ACCT-000117 CLIENTID=MCFP2 VENUE=XPAR` | ultext, space-separated | `unknown` |
| `Referential(dbi\|equity\|dbi;GB00BN7SWP63_XLON_GBX\|[quantity-type=])` | pairs it can find, nothing more | `unknown` |
| `<Order ClOrdID='XML-1'>…</Order>` | not pairs; kept whole | `unknown` |
| `Receiving XmlApi: <Execution ExecID='E1'>…</Execution>` | the same, behind a prefix | `unknown` |
| `Message rejected because : ignoring OMSSales expiry message` | no pairs | `unknown`, no entries |
| `no level printed by this plugin` | no pairs | `unknown`, no entries |
| `heartbeat emitted seq=7` | one pair, kept | `unknown` |

28g. Every row above builds — none is skipped and none is an `Err` (P7-R44).
28h. The four framed rows resolve their type from `35=`, the `MSGTYPE=` row
     from its key, and the rest answer `unknown` (P7-R44).
28i. `58=quoting #A=1 and #B=2` is one Text field, not three (P7-R42).
28j. `8=FIX4` finds its frame and answers no version, falling through to the
     next tier (P7-R45).
28k. The space-separated row splits on space and the `|` rows on `|`, each
     chosen once (P7-R46).
28l. The two rows whose payload opens with `<` keep it whole (P7-R70).

**Streaming.**
28b. A row read singly and through a `FixReader` answer equal messages
     (P7-R59).
28c. Input that is not a row at all yields one `Err` item carrying it, and
     the rows after it still read (P7-R56) — while every row of the capture
     table above yields `Ok`, none of them being a failure.
28d. An unbounded source is consumed lazily: a reader over an infinite
     iterator, pulled ten times, touches ten rows (P7-R55).
28e. Pinned and unpinned readers answer the same messages over one capture
     (P7-R57).
28f. `collect::<Result<Vec<_>, _>>()` over a capture with one bad row
     answers `Err`, which is how a caller opts into stopping (P7-R56).

**Halves.**
29. Each half accessor answering only from its half, over a registry holding
    a scalar and a group that would both match a key.
30. `get_field` answering exactly what it answered before, over every
    existing case.

**Bench.** A NewOrderSingle of ~15 pairs, an ExecutionReport of ~30, and a
300-pair message; tag-keyed against name-keyed; branch and version given
against inferred; the readers benched beside `from_pairs` so the split cost
is visible separately. Then the stream: rows per second over a 100k-row
capture, pinned against inferred, and against calling the singular reader in
a loop — the number that says whether `FixReader` earns its existence
(P7-R57). Table on the FIX page, which also gains the two half-probe rows.

**Allocations.** A 30-pair tag-keyed build of short values allocates the
three reserved vectors and nothing per entry (P7-R80, P7-R81).


---

## Phase 9 — lifting: the fields a reader actually asks for

**Goal.** Answer the handful of facts every consumer wants — who, what, how
much, when — without each one re-walking groups and components.

**Depends.** Phase 7 (a built message), Phase 8 (packed side and message
type).

**Surface.** A lift module inside the FIX module: the rules table and the
accessors over it. Tests, a benchmark group, the FIX page.

**Never.** Store a lifted value on the message (N4). Every answer is derived
on demand from the row that is already there.

### Contract

```rust
/// One lifted facet: where it may come from, in priority order.
pub struct FixLift { facet: &'static str, sources: &'static [FixSource] }

impl FixMsg {
    /// The one value a facet names, or `None` when the message does not
    /// answer it unambiguously.
    pub fn lifted(&self, facet: &str) -> Option<&Scalar>;
    /// Every facet this message answers, for a batch writer.
    pub fn lift(&self) -> impl Iterator<Item = (&'static str, &Scalar)>;
    /// The party bearing one role, which is how a party is addressed.
    pub fn party(&self, role: &str) -> Option<FixParty<'_>>;
    /// One regulatory timestamp by its type.
    pub fn trd_reg_timestamp(&self, kind: &str) -> Option<&Scalar>;
    /// Which lane a side belongs to, or neither (P9-R13).
    pub fn side_direction(&self) -> Option<FixDirection>;
}
```

### Rules

- **P9-R1. A lift answers only when the answer is unambiguous.** This is the
  whole design. yggfin states it three ways from production: *one occurrence
  is one value, and one value is what a column can answer with*; *a tag that
  repeats belongs to a repeating group, so no occurrence is the line's*; *a
  multi-leg order has no one symbol, and saying so is the honest column*. So
  two candidate occurrences answer `None`, never the first — and a caller
  that wants a specific one addresses it (`party`, `trd_reg_timestamp`).
- **P9-R2. The rules are a table, not code per facet.** A `FixLift` is a
  facet name and an ordered list of sources; a source is a tag, a path, or a
  group selector. Resolution takes the first source the message answers.
  Adding a facet is a row.
- **P9-R3. Rules may be conditioned on `MsgType`, and on nothing else.** A
  price means different things in an ExecutionReport and a Quote, and the
  message type is the one fact that says which. No venue conditions, no
  branch conditions, no value conditions — those are the expression layer
  this brief keeps out (P3-R16).
- **P9-R4. The facets, drawn from the Orchestra registry rather than
  invented.**

  | facet | sources, in order |
  | --- | --- |
  | `id` | `ClOrdID(11)`, `OrderID(37)`, `QuoteID(117)`, `TradeID(1003)` — by `MsgType` (P9-R3) |
  | `secondaryid` | `SecondaryClOrdID(526)`, `SecondaryOrderID(198)`, `OrigClOrdID(41)` |
  | `execid` | `ExecID(17)` |
  | `symbol` | `Symbol(55)`, then `SecurityID(48)` with `SecurityIDSource(22)` |
  | `side` | `Side(54)`, packed (P8-R2); else derived (P9-R15) |
  | `price` | `Price(44)`, `LastPx(31)` — by `MsgType` |
  | `bidpx` | `BidPx(132)`; else derived (P9-R14) |
  | `askpx` | `OfferPx(133)`; else derived (P9-R14) |
  | `bidsize` | `BidSize(134)`; else derived (P9-R14) |
  | `asksize` | `OfferSize(135)`; else derived (P9-R14) |
  | `quantity` | `OrderQty(38)`, `LastQty(32)`, `Quantity(53)`, `CumQty(14)`, `LeavesQty(151)` — by `MsgType` |
  | `quantitytype` | `QtyType(854)`, then the deprecated `QuantityType(465)` (P9-R10) |
  | `currency` | `Currency(15)`, then `SettlCurrency(120)` |
  | `transacttime` | `TransactTime(60)`, then `SendingTime(52)` |
  | `status` | `OrdStatus(39)`, `ExecType(150)`, `QuoteStatus(297)` |

  Facet names obey the field-name rule (P3-R7): lowercase letters and
  digits, no separators, so a lifted column sits beside a field name with no
  change of style. Where a facet names one FIX field it *is* that field
  case-folded — `bidpx`, `execid`, `transacttime`; `askpx` and `asksize` are
  the two exceptions, because FIX spells that lane `Offer` and every reader
  calls it the ask.

  A source that resolves to no field in the dictionary is skipped, not an
  error: a facet is a best answer, not a schema.
- **P9-R5. Parties are addressed by role, never by position.** The Parties
  group is counter `NoPartyIDs(453)` over `PartyID(448)`,
  `PartyIDSource(447)`, `PartyRole(452)`, `PartyRoleQualifier(2376)` and the
  nested `PtysSubGrp`. Every message carries several occurrences, so
  `lifted("party")` is meaningless and is not a facet: `party(role)` walks
  the occurrences, matches `PartyRole` through the code translation (P4-R5),
  and answers the one bearing it — or `None` when none or several do.
- **P9-R6. Regulatory timestamps are addressed by type.**
  `NoTrdRegTimestamps(768)` over `TrdRegTimestamp(769)`,
  `TrdRegTimestampType(770)`, `TrdRegTimestampOrigin(771)`, plus the desk and
  NBBO members later versions added. `trd_reg_timestamp(kind)` matches the
  type through the same code translation, so a caller asks for
  `"ExecutionTime"` and not for `1`.
- **P9-R7. A lift never reaches into a repeating group for a scalar facet.**
  A `price` inside a leg or an underlying is that leg's, not the message's.
  Facet sources address the flat level only; anything deeper is reached by
  path (P7-R37) or by the two role-addressed accessors.
- **P9-R8. Lifting is version-aware for free.** It addresses tags, and the
  message's own fields already carry the names and types its version gave
  them (P7-R23), so a 4.2 message lifts `quantity` from tag 32 whether that
  tag is called `LastShares` or `LastQty`.
- **P9-R9. Nothing is enriched into the row.** `lift()` yields borrowed
  values for a batch writer to place in its own columns; the message is
  unchanged, `entries` are unchanged, and two lifts of one message are the
  same walk twice with no cached state to go stale.

#### A quantity carries its unit

- **P9-R10. A facet prefers the source the specification has not
  deprecated.** `QuantityType(465)` is superseded by `QtyType(854)`, and a
  venue predating the change carries only the old one. A facet's sources are
  therefore current-first, and the lineage already says which is which: a
  source whose entry is `deprecated` at the message's version is tried only
  after every source that is not (P3-R9). One rule, not a special case for
  this pair — the specification does this repeatedly.
- **P9-R11. A quantity is answered with its unit, never alone.**
  `Quantity(53)` is "overall/total quantity", and `QtyType` says what of:
  `1` shares, `2` bonds, `3` current face, `4` original face, `5` currency,
  `6` contracts, `7` other, `8` par. Five of those eight are not a share
  count, so a consumer summing `quantity` across messages without its type
  adds shares to dollars. `lift()` yields `quantitytype` whenever it yields
  `quantity` and the message states one; a number whose unit is unstated is
  answered as exactly that, never assumed to be shares.
- **P9-R12. A quantity in currency is denominated by the currency facet.**
  Where `quantitytype` is `5`, the number is money, and `currency` — the
  instrument's `Currency(15)`, else the `SettlCurrency(120)` it settles in —
  is what it is money *of*. A reading, not a conversion: nothing is
  re-denominated, and a message stating a currency quantity and no currency
  answers the quantity alone (P9-R1).

#### Enriching a side against its lane

A quote states two prices and no side; an order states one price and a
side. They are two shapes of one fact, and each can answer for the other —
but only where the answer is forced, never where it is likely.

- **P9-R13. Direction is an explicit table, never inferred from a
  spelling.** A `const` list over the standard `SideCodeSet` says which
  codes buy, which sell, and which do neither:

  | direction | codes |
  | --- | --- |
  | buys | `Buy`, `BuyMinus` |
  | sells | `Sell`, `SellPlus`, `SellShort`, `SellShortExempt`, `SellUndisclosed` |
  | neither | `Cross`, `CrossShort`, `CrossShortExempt`, `Undisclosed`, `AsDefined`, `Opposite`, and every code not listed |

  A cross is both sides at once and `Opposite` means "whatever the other leg
  was", so neither has a lane; a code the table does not name is `neither`,
  never guessed. Direction is **not** taken from the symbolic name's
  prefix — `SellShortExempt` starting with `Sell` is a fact about English,
  and P4's Decided refuses that reasoning. Orchestra does not publish
  direction, so this table is domain knowledge, written out where a reviewer
  can check it.
- **P9-R14. A side and a price fill their lane — on an order, not on a
  fill.** A buy order at `P` is a party willing to pay `P`, which is a bid;
  a sell order at `P` is an offer. So where the message is an order and
  carries `Price(44)` with a side whose direction is a lane, `bidpx` or
  `askpx` answers that price, and `OrderQty(38)` fills the matching size
  the same way. **`LastPx(31)` never projects**: a fill's price is a traded
  price, not a quote lane, and putting it on one would state a quote that
  never existed. `MsgType` is what tells the two apart (P9-R3).
- **P9-R15. One lane implies a side; two lanes imply nothing.** A quote
  carrying only `BidPx` is a party bidding, so `side` answers `Buy`; only
  `OfferPx`, and it answers `Sell`. A two-sided quote carries both lanes, so
  no single side is the message's — it answers `None`, which is P9-R1 again
  and is the case that makes this rule safe to have at all.
- **P9-R16. Enrichment fills, never overwrites, and never writes.** A
  derived answer is offered only where the message states nothing: a quote
  that carries `BidPx` *and* `Side` answers both as stated, and a
  disagreement between them is reported through `anomalies()` rather than
  resolved. Nothing derived is stored (P9-R9), so a derived `bidpx` and a
  stated one are indistinguishable to a reader and neither can go stale.

### Decided

- **Ambiguity answers nothing.** *Rejected:* taking the first occurrence,
  which is what makes a lifted column quietly wrong on exactly the messages
  that matter — a multi-leg order, a two-sided quote, a trade with several
  parties.
- **The direction table is explicit and short.** *Rejected:* deriving
  buy/sell from the code's symbolic name, or from its wire value's ordering.
  Both are guesses that would silently mis-lane a vendor's own side, and a
  mis-laned side is a wrong price on the wrong book.
- **The table lives in the crate, not in the dictionary.** *Rejected:*
  storing lift rules as metadata on fields. A facet is a *consumer's*
  question, not a property of a FIX field, and putting it in the dictionary
  would make every generated shard carry one reader's opinion.

### Tests

1. Each facet in P9-R4 lifted from a message that answers it, and `None`
   from one that does not.
2. Two occurrences of a facet source answering `None`, not the first
   (P9-R1) — the multi-leg order is the case to write.
3. `MsgType` conditioning: `price` from `LastPx` in an ExecutionReport and
   from `Price` in a NewOrderSingle (P9-R3).
4. `party("ExecutingBroker")` answering the right occurrence out of four;
   `None` when the role is absent and when two occurrences share it
   (P9-R5).
5. A role given as `1` and as its symbolic name reaching the same party
   (P9-R5, P4-R5).
6. `trd_reg_timestamp("ExecutionTime")` by name and by code (P9-R6).
7. A price inside a leg not lifted as the message's `price` (P9-R7).
8. A 4.2 message lifting `quantity` from tag 32 named `LastShares`, and a
   newest one from tag 32 named `LastQty` (P9-R8).
9. `lift()` twice answers identically and mutates nothing (P9-R9).
10. A facet whose source resolves to no field in the dictionary is skipped,
    not an error (P9-R4).
11. `Quantity(53)` lifted as `quantity` with `QtyType(854)` as its unit; a
    message carrying only the deprecated `QuantityType(465)` answering from
    that instead, and one carrying both preferring `854` (P9-R10).
12. A `quantitytype` of `5` read against `Currency(15)`, and against
    `SettlCurrency(120)` when the first is absent; a currency quantity with
    no currency stated answering the quantity alone (P9-R12).
13. A quantity whose message states no type answering the quantity with no
    unit, and saying so (P9-R11).
14. A buy order with `Price` answers `bidpx` and no `askpx`; a sell order
    the reverse; `OrderQty` fills the matching size (P9-R14).
15. An ExecutionReport with `LastPx` and a side answers neither lane
    (P9-R14) — the case that keeps a traded price off a book.
16. A cross, an `Undisclosed` and an unlisted vendor side each answer no
    lane (P9-R13).
17. A one-sided quote answers `Buy` from `BidPx` alone and `Sell` from
    `OfferPx` alone; a two-sided quote answers no side (P9-R15).
18. A message stating both `BidPx` and a contradicting `Side` answers both
    as stated and reports the disagreement (P9-R16).

**Bench.** `lift()` over an ExecutionReport against reading each facet by
tag, so the table's cost is visible.

---

## Handoff

Last phase of this brief. Deliberately left to the CBlock brief and stated
in the module docs rather than started here: reassembling a repeating group
from bare tag repetition, which needs the message-type grammar (P7-R38); and
the *expression-driven* normalization layer with its conditions, lookups and
value mappings, which needs an evaluator (P3-R16). Two things that once
looked like that layer's are done here instead: the lineage-driven half of
transcoding, in `convert_into` (P7-R31), and pair-shaped `XmlData(213)`,
which needs no evaluator because the payload's own shape decides (P7-R75).

**From Phase 9.** Nothing in this brief consumes it: lifting is the outermost
layer, and a batch writer or a column projection is the next thing that would.
