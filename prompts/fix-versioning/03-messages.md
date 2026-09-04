# Messages — explicit halves, entries, and the readers

**Goal.** Build a typed, lossless `FixMsg` from key/value pairs, from FIX text, or
from a ULBridge body.

**Depends.** Phase 2 (the identifier and the branch table), Phase 3 (the version) and
Phase 6 (a dictionary to resolve against).

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

## Phase 7 — explicit halves, `FixEntry`, `from_pairs`, and the text readers

**Goal.** Build a typed, lossless `FixMsg` from key/value pairs, from FIX
text, or from a ULBridge body.

**Surface.** A new entry module inside the FIX module; the registry (the
public halves); the message (its entries, the builder and the three
readers). The FIX module's tests, the counting-allocator target, a new FIX
benchmark group, and the FIX documentation page.

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
    pub tag: Option<i32>,      // None when the key named no field
    pub branch: Option<i32>,   // xxh32 of the branch; None is standard/unresolved
    pub key: Option<SmolStr>,  // the arriving key, kept only when it is not the tag
    pub value: SmolStr,        // exactly as it arrived; never absent
}
impl FixEntry { pub fn id(&self) -> Option<FixId>; }

impl FixMsg {
    pub fn entries(&self) -> &[FixEntry];
    pub fn anomalies(&self) -> impl Iterator<Item = FixAnomaly<'_>>;
    pub fn into_text(&self, sep: char) -> String;

    pub fn from_pairs<'a, I>(
        registry: Arc<FixRegistry>, entries: I,
        branch: Option<&FixBranch>, version: Option<Version>,
    ) -> Result<Self> where I: IntoIterator<Item = (&'a str, &'a str)>;

    pub fn from_text(text: &str) -> Result<Self>;
    pub fn from_fixtext(
        registry: Arc<FixRegistry>, text: &str, sep: char,
        branch: Option<&FixBranch>, version: Option<Version>,
    ) -> Result<Self>;
    pub fn from_ultext(
        registry: Arc<FixRegistry>, body: &[u8],
        branch: Option<&FixBranch>, version: Option<Version>,
    ) -> Result<Self>;
}
```

`FixMsg` gains one field: `entries: Vec<FixEntry>`.

### The halves

- **P7-R1.** the registry's two private position lookups - one by identifier, one by
  name - each hard-code the same four-way probe - primitive canonical,
  nested canonical, primitive alternate, nested alternate. Expose the halves
  and compose: `get_field` becomes
  `get_primitive_field(key).or_else(|| get_nested_field(key))`. Each half
  accessor probes both tiers, canonical first. `get_field_by_tag`,
  `get_field_by_id`, `get_field_by_name` and `get_field_by_path` redirect
  the same way; none keeps a probe chain of its own.
- **P7-R2. This is not tidying.** A transcriber resolving a wire tag wants a
  scalar; today an unknown tag pays all four probes - 32.3 ns for a
  primitive hit against 72.2 ns for a miss. `from_pairs` asks only
  `get_primitive_field`, so an unknown tag costs one probe.

### `FixEntry`

- **P7-R3.** `id()` folds tag and branch into a `FixId` with one shift-or
  (P2-R1), so a resolved entry addresses the registry without hashing.
- **P7-R4.** `branch: None` means the standard branch *and* "not resolved
  yet" - which is why this is not simply a `FixId`. `Option<i32>` costs 8
  bytes, not 4: `0` is a legal digest, so there is no niche.
- **P7-R5. The value owns.** A `FixMsg` holds its entries and outlives the
  text it was read from, so a borrowed value would force `FixMsg<'a>` on
  every caller and on both bindings, which hold one across an FFI boundary.
  `SmolStr`'s 23 inline bytes cover a side, a price, a symbol and a 21-byte
  `UTCTimestamp`, so the common entry allocates nothing. Readers still split
  into borrowed `(&str, &str)`; the single materialization is in
  `from_pairs`.
- **P7-R6. `tag` is optional and `key` exists** because an unresolved key
  has no tag. `VenueOwnThing=x` survives in the tree (P7-R12) and `entries`
  is the wire record, so it must hold it too. `key` is `None` for the
  common resolved pair, so nothing is stored twice.
- **P7-R7. The value is never typed inside the entry.** Typing happens once,
  in `from_pairs`, through `Field::scalar`.

### `FixMsg` carries its entries

- **P7-R8. `entries` is not the row restated,** and this is the one place
  the brief admits two facts about one thing (N4). The row is the
  *interpretation*: values typed, codes translated, names canonical, groups
  nested, header ordered. `entries` is what *arrived*: raw text, arrival
  order, untranslated, including pairs no dictionary explained. Neither
  derives from the other - a translated `4` cannot say whether the wire
  carried `4` or `PercentageWaivedCashDiscount` - so lossless re-emission is
  impossible from the row alone. That is what makes `into_text` and the
  round trip (P7-R28) work. **Say exactly this in the doc comment**, or a
  reader will assume one of the two is redundant.
- **P7-R9. Populated by `from_pairs`, and so by all three readers.** Empty
  for a message built through `new` or `with_registry`; with no entries,
  `into_text` and `anomalies` fall back to the row and say so.

### What a key may be

| key | means |
| --- | --- |
| `54`, `"54"` | a tag, through the strict the strict `parse_tag`, which refuses `+35` and `3x` |
| `Side`, `side`, `SIDE`, `" Side "` | a name, trimmed and folded |
| `msg_type`, `msg-type`, `Msg Type` | the same name: separators fold away too |
| `Instrument.Symbol` | a path, through the existing `get_field_by_path` |
| `PartyID[0]`, `PartyID[1]` | one field, two occurrences, in order |
| `NoPartyIDs[0].PartyID` | a group entry: which group, which occurrence, which member |
| `VenueOwnThing` | an unknown name, **kept** |
| `""`, `"   "` | dropped |

- **P7-R10. Separator folding.** The registry folds ASCII case only today
  (the FIX module states so), so a renderer emitting `msg_type` or `Msg Type` misses a
  field that exists. Extend the FIX name fold to drop `_`, `-` and space -
  the fold `LOGICAL_NAMES` already uses (the datatype grammar's own name fold), so one
  rule serves both. No two FIX fields differ only by a separator; assert
  that over the generated the committed dictionary as the test that lets the change in.
- **P7-R11. An unknown *tag* is kept** as a nullable `utf8` field under its
  decimal spelling - `FixMsg`'s existing rule.
- **P7-R12. An unknown *name* is kept too,** as a nullable `utf8` field
  under its own spelling. Every venue sends fields no dictionary has, and
  dropping them loses data. Resolved fields come first in the root, unknown
  ones after, so the schema stays stable when a dictionary later learns the
  name.
- **P7-R13. An empty value drops its pair.** `54=` is a malformed message,
  not an absent side.
- **P7-R14. Order and repetition are the message.** A tag appearing twice
  stays two entries in input order; a map keyed by tag would lose a
  repeating group. P3's duplicate-name suffix rule names the second and
  later children.

### Inferring version and branch

- **P7-R15. Version, when the caller names none.** Each step is a FIX rule,
  not a heuristic.
  1. **Tag 1128 `ApplVerID`** - the application version. Code set:
     `0`=FIX27, `1`=FIX30, `2`=FIX40, `3`=FIX41, `4`=FIX42, `5`=FIX43,
     `6`=FIX44, `7`=FIX50, `8`=FIX50SP1, `9`=FIX50SP2, `10`=FIXLatest -
     and `FIXLatest` resolves to the dictionary's `newest()` pair, the real
     version and EP, never a sentinel (`P3-R1b`). It
     wins because under FIXT.1.1 the session version says nothing about the
     application version. The symbolic spelling (`FIX44`) is accepted
     through P4's `code_by_name`.
  2. **Tag 8 `BeginString`** - `FIX.4.0` … `FIX.4.4` give `4.0` … `4.4`.
     `FIXT.1.1` is a session version and names no application version, so it
     **falls through** rather than being taken literally.
  3. **The branch's own default version**, once the branch is known
     (P2-R9b). A dialect that declares it is stating what its counterparty
     speaks, which is better evidence than a dictionary-wide default.
  4. Otherwise the dictionary's `newest()` - the real newest version and
     extension pack it holds, which is what a message carrying no version
     marker means. Never `Version::MAX`: a sentinel compares wrongly against
     a field genuinely dated at the newest version, and is wrong again the
     next time an extension pack lands.

  Branch resolution therefore runs **before** step 3, and the two are one
  pass: read tags 49 and 56 while reading 1128 and 8, since all four are in
  the header.
- **P7-R16. Branch, when the caller names none.** In this order, and the
  first two are identity rather than inference:
  1. **The session names the dialect.** `SenderCompID(49)` and
     `TargetCompID(56)` are in the standard header of every message, and
     P2-R9b bundles that pair onto the branch. If a dialect declares it,
     `branch_for_session` answers exactly - no counting, no guessing, one
     lookup. **Try both orders**: a dictionary declares the session from its
     own side, so an inbound message carries the declared pair reversed, and
     matching only one order silently misses half the traffic.
  2. Resolve every entry in `FixBranch::STANDARD`; nothing missed means
     standard, and there is no second pass - the common case costs one probe
     per entry.
  3. Otherwise retry only the *missed* tags against each branch the registry
     holds (`branches()`, free from P2-R9) and take the branch resolving the
     most; a tie goes to the lowest branch name, so the answer is
     deterministic; a branch resolving none is never chosen and its tags
     stay unknown.

  A caller who passes a branch gets it, with no guessing at all.

### Building the message

One pass over the resolved entries:

- **P7-R17.** The field is the registry's, cloned, with `name_at(version)`
  and `dtype_at(version)` where a lineage exists, and the field's own name
  and datatype where it does not.
- **P7-R18.** It is non-null in this message's schema, because the value is
  present.
- **P7-R19.** The value passes through the code set first:
  `code_value_at(&version, entry.value).unwrap_or(entry.value)`, so
  `CommType=PercentageWaivedCashDiscount` stores `4` and
  `MsgType=NewOrderSingle` stores `D`, while an unexplained spelling is
  carried through untouched (P4-R7).
- **P7-R20.** Then `field.scalar(Scalar::from(translated))`, with P7-R24 on
  refusal. Nothing re-checks what `scalar` answered.
- **P7-R21.** Order is `STANDARD_HEADER_TAGS`, then the body in entry order,
  then `STANDARD_TRAILER_TAGS` - flat, no `StandardHeader` Struct (P6-R11).
- **P7-R22.** `FixMsg::with_registry` finishes it, so existing validation
  and canonicalization are not bypassed.
- **P7-R23. An empty dictionary is a supported input, not an error.** With
  nothing resolvable, every name stays a name, every tag stays its decimal
  spelling, and `by_name` still finds what was put in - which is what makes
  this usable on a venue whose dictionary is not loaded yet.
- **P7-R24. A value that will not type is null, not a failure.**
  `field.scalar` refuses a value the datatype cannot hold - a `BodyLength`
  that is not digits, a mangled timestamp. That must not cost the message:
  (a) the row's field is **null**; (b) the raw text stays in `entries`
  exactly as it arrived; (c) the refusal is reported through `anomalies()`;
  (d) `from_pairs` still answers `Ok`. A parse error is raised only for
  input that is not a message at all. A null nobody can explain is worse
  than the value that actually arrived.

### Groups

- **P7-R25. Repeating groups are in scope, because the key carries the
  location.** A key spelled `NoPartyIDs[0].PartyID` states group, occurrence
  and member, so no grammar is needed - and yggdryl already has the pieces:
  a group is a List of a non-null `item` Struct, and
  `Field::set_field_by_path` writes into one. `from_pairs` builds **real
  nesting** from indexed keys, where yggfin keeps a flat `comp` string
  because its field model has no list of structs to put it in.
- **P7-R26. Out of scope: inferring a group from repetition alone.** Bare
  `448=A`, `448=B` with no index and no group key produces two sibling
  occurrences of `PartyID`, not a reconstructed `NoPartyIDs`. Reassembling
  that needs the message grammar, which is the `.cfb` phase's. Say so in the
  module docs.

### Encode direction

- **P7-R27. Wire spellings belong to the FIX layer, never to
  `DataType::scalar`.** yggfin pins them: a float is never exponent notation
  (`1e-7` writes `0.0000001`), a `UTCTimestamp` is
  `20260821-10:30:00.123456`, a date is `20260821`, a time is
  `10:30:00.000000`, a boolean is `Y` or `N`.
  **Verify first** whether `DataType::scalar` accepts a `Scalar::String` for
  `Boolean` and the temporals at all (the crate's one value contract). Where it does
  not, the FIX layer parses the wire spelling into the right `Scalar` before
  calling `scalar`; where it does, check the spelling it accepts is FIX's.
  Either way the generic value contract learns no FIX spelling -
  `LOGICAL_NAMES` is deliberately a *type* table, not a *value* one.
- **P7-R28. The two ways in must agree.**
  `from_text(built.into_text('|')) == built` is a test in this phase.

### The three readers, one builder

`from_pairs` borrows both halves of a pair, so a splitting iterator feeds it
with no copy. Each reader splits, rewrites its dialect into the key forms
`from_pairs` already understands, and hands the iterator over. One nesting
builder, one fold, one code translation under all three.

- **P7-R29. `from_text` picks the dialect by one token, never by sniffing.**
  Take the bytes before the first `=`: all ASCII digits means
  `from_fixtext`, and the separator is SOH when the text holds one and `|`
  otherwise; anything else means `from_ultext`. `from_text` is the
  convenience over `FixRegistry::global()` with everything inferred. Empty
  text is a typed error, not an empty message.
- **P7-R30. `from_fixtext`.** Split on `sep` with `memchr`, then each
  segment at its first `=`. A trailing empty segment is tolerated - a wire
  message ends with the separator. A segment with no `=` is dropped, as an
  empty key is. Duplicate tags stay in arrival order. Every key and value is
  a slice of the input.

#### `from_ultext`

ULBridge writes names, not tags, and packs a repeating group into one pair
(yggfin, `docs/fix/repeating-groups.md`):

```text
#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=SYNTH-01<sub>PARTYIDSOURCE=shortcodeid<sub>PARTYROLE=executingsystem|
```

where `<sub>` is `\x04\x03`, EOT then ETX.

- **P7-R31.** Pairs split on `|`, then at the first `=`. Keys are names in
  any case and reach their field through the P7-R10 fold.
- **P7-R32.** A key opening with `#` names a group: `#NOPARTYIDS=1` is the
  counter; `#NOPARTYIDS[0]=…` is entry 0, and its *value* is a run of member
  pairs.
- **P7-R33.** Members inside an entry split on `\x04\x03`.
- **P7-R34. And sometimes on nothing at all.** ULBridge may omit the
  separator after the first member while keeping the index:
  `#NoPartyIDs[0]=PartyID=P-1PartyIDSource=DPartyRole=3`. Split by scanning
  for the next member name the group's own field declares, taking the
  **longest declared match** so `PartyIDSource` beats `PartyID`. Only that
  group's declared members are candidates, which keeps the scan bounded and
  the result explainable.
- **P7-R35.** Residue that will not split stays as one unknown key,
  verbatim. Never dropped, never fatal.
- **P7-R36.** Indices may be partial or out of order - `[2]` before `[0]`,
  with gaps. Occurrences are built by index, not arrival; a gap is a null
  occurrence.
- **P7-R37.** It then rewrites into the key forms `from_pairs` takes -
  `#NOPARTYIDS[0]=PARTYID=…` becomes `("NoPartyIDs[0].PartyID", "…")` - and
  builds no tree of its own. Values translate through the code set like any
  other, so `PARTYIDSOURCE=shortcodeid` stores `P` and
  `PARTYROLE=executingsystem` stores `16`, while
  `PARTYROLE=orderoriginatorsystem` is stored verbatim under tag 452.

#### Token rules both readers obey

Each row is a case in yggfin's `test_message.py` or `test_transcribe.py` -
a line some venue really sent.

| # | rule | why |
| --- | --- | --- |
| P7-R38 | A token splits at its **first** `=` only. | `Text=a;b` is one value with a semicolon, not two fields. |
| P7-R39 | `G[0]=M=v` and `G[0].M=v` are one field, two prints. | A group has one shape; two spellings must not make two. |
| P7-R40 | `#` marks where a key **starts**, not which field it is. | `#54=x` is a rendered key spelled with digits, **not** tag 54. |
| P7-R41 | `#A=1#B=2` has no separator: the next `#` ends the previous value. | ULBridge omits separators; the marker is the boundary. |
| P7-R42 | Tag mode is ASCII digits only. | A bracket, dot or `#` means a rendered key, so `453[0]` is never tag 453. |
| P7-R43 | A digit key overflowing `i32` is not a tag. | An epoch-millis key looks like digits; `parse_tag` already drops it. |
| P7-R44 | Trim ASCII whitespace only. | A non-breaking space is part of the value; trimming Unicode returns a tag never sent. |
| P7-R45 | Nothing after `10=<checksum>` is part of the message. | Log lines carry pair-shaped noise after the trailer. |
| P7-R46 | One `a=b` alone is a sentence, not a message. | Require two tokens, or an `8=`/`35=` lead, so prose does not parse. |
| P7-R47 | Two values under one key stay two. | It is a group or a rewrite; collapsing picks one, and picking is a guess. |
| P7-R48 | "Not a message" and "a message that said nothing" are different answers. | The empty message is `Ok` with no entries; unparseable input is an error. |

#### `data` fields are read by length, not by separator

- **P7-R49.** FIX types a field `data` **because its value may contain the
  separator**. `RawData(96)`, `XmlData(213)`, `SecureData(91)` and
  `Signature(89)` each follow a length field - `RawDataLength(95)`,
  `XmlDataLen(212)`, `SecureDataLen(90)`, `SignatureLength(93)` - and that
  length, not the next SOH, says where the value ends. A reader that
  tokenizes first loses the message.
- **P7-R50.** The registry says which tags are `data` (`DataType::Binary`
  after P6-R8), so nothing hard-codes the four pairs.
- **P7-R51.** When the stated length and the next separator disagree, **take
  the separator**: a writer that miscounted has stated two things and the
  delimiter is the safer. Record it through `anomalies()`.
- **P7-R52.** Venues put `NAME=VALUE` pairs inside `XmlData(213)` though the
  standard calls it an XML stream. Not this phase's job - the value is kept
  whole - but a nested pair addressed `XmlData.ClOrdID` must later resolve
  the way `NoPartyIDs.PartyID` does.

#### Anomalies are derived, never a second state

- **P7-R53.** A counter disagreeing with the entries it introduces, a group
  that would not split cleanly, a value that would not type - all real, none
  fatal. `anomalies()` derives them on demand by comparing the counter value
  (an ordinary value at its own tag) with the List's length, the way `FixId`
  is derived rather than stored. No error channel on `FixMsg`, nothing to
  keep in step, and a caller who never asks pays nothing.

### Optimization the phase is judged on

- **P7-R54.** One `Vec<FixEntry>`, one `Vec<Field>`, one `Vec<Scalar>`, each
  reserved from the iterator's `size_hint` before the walk. No per-entry
  `String`, no per-entry map. The `Vec<FixEntry>` is the one the message
  keeps: built once, moved in, never cloned.
- **P7-R55.** A resolved entry allocates nothing - integers for `tag` and
  `branch`, `None` for `key`, and a value inside `SmolStr`'s inline buffer.
- **P7-R56.** Only `get_primitive_field` is probed for scalars; the nested
  half is reached only for a counter tag.
- **P7-R57.** Header ordering reads a precomputed tag-to-position table, not
  a scan of `STANDARD_HEADER_TAGS` per entry.
- **P7-R58.** The readers copy nothing: every key and value is a slice of
  the input and splitting uses `memchr`.

### Tests

**Keys and values.**
1. Tag-keyed and name-keyed pairs producing the identical message.
2. `" Side "`, `msg_type`, `msg-type`, `MSG_TYPE`, `Msg Type` all reaching
   their field (P7-R10).
3. `Instrument.Symbol` resolving through the path.
4. `PartyID[0]` and `PartyID[1]` staying two ordered occurrences.
5. `NoPartyIDs[0].PartyID` building a List of one `item` Struct, not a flat
   name (P7-R25).
6. An unknown name surviving beside a known one, known first (P7-R12).
7. An unknown tag kept as `utf8` under its decimal name (P7-R11).
8. Empty and blank keys dropped; an empty value dropped (P7-R13).
9. The same pairs built against an empty registry (P7-R23).

**Version and branch.**
10. `ApplVerID` beating `BeginString`; `BeginString="FIXT.1.1"` falling
    through to Latest; an explicit `version` overriding both (P7-R15).
11. Branch inference picking the vendor dictionary that resolves the misses,
    the tie rule, and an explicit branch suppressing inference (P7-R16).
11b. A declared `(sender, target)` pair selecting its dialect directly, in
     the declared order and reversed, and folded (P7-R16.1, P2-R9d).
11c. A branch's declared default version losing to a message's own
     `ApplVerID` and winning over the dictionary's `newest()` (P7-R15.3).
12. Tag 32 keyed `LastShares` in a `4.2` message and `LastQty` in a Latest
    one, both answering the same value.
13. Header and trailer ordering with a body field interleaved in the input.

**Token rules.**
14. Every row of P7-R38…R48, one case each.
15. `#54=x` reaching the field whose *rendered key* is `54`, never tag 54.
16. `G[0]=M=v` and `G[0].M=v` answering equal messages.
17. A lone `a=b` refused as not-a-message, while an empty-but-valid message
    answers `Ok` with no entries.
18. A `data` field whose value contains the separator, read by its length
    field; a miscounted length taking the separator and appearing in
    `anomalies()` (P7-R49, P7-R51).
19. A `BodyLength` of `abc` nulling that field while the raw text stays in
    `entries` (P7-R24).
20. Tag 555 at two nesting levels in one TradeCaptureReport, neither
    guessed.

**Codes.**
21. `("CommType", "PercentageWaivedCashDiscount")` and
    `("13", "percentage_waived_cash_discount")` both storing `4`.
22. `("MsgType", "NewOrderSingle")` storing `D`; `("CommType", "4")`
    unchanged; an unexplained spelling stored verbatim.
23. A name added after the message's inferred version refusing to translate.

**Readers and entries.**
24. The ULBridge payload verbatim, with `\x04\x03` and with the separator
    omitted, both producing one `NoPartyIDs` occurrence of four members.
25. `PARTYIDSOURCE` translating while `PARTYROLE=orderoriginatorsystem`
    survives untranslated.
26. Out-of-order and gapped indices (P7-R36).
27. A counter of `2` against one entry appearing in `anomalies()` while the
    message still reads.
28. `entries()` holding every pair in arrival order with the untranslated
    spelling, beside a row holding the translated code (P7-R8).
29. An unresolved key in `entries` with `tag` `None` and `key` set (P7-R6).
30. `from_fixtext` over SOH-separated and `|`-separated captures of one
    message answering equal messages.
31. `from_text` picking the dialect from `35=D|…` against `MSGTYPE=D|…`.
32. `from_text(built.into_text('|')) == built` (P7-R28).

**Halves.**
33. Each half accessor answering only from its half, over a registry holding
    a scalar and a group that would both match a key.
34. `get_field` answering exactly what it answered before, over every
    existing case.

**Bench.** A NewOrderSingle of ~15 pairs, an ExecutionReport of ~30, and a
300-pair message; tag-keyed against name-keyed; branch and version given
against inferred; the readers benched beside `from_pairs` so the split cost
is visible separately from the build. Report per-message and per-pair cost;
table on the FIX documentation page, which also gains the two half-probe rows.

**Allocations.** A 30-pair tag-keyed build of short values allocates the
three reserved vectors and nothing per entry (P7-R54, P7-R55).

---

## Handoff

### From Phase 7

Last phase of this brief.

What is deliberately left for the `.cfb` phase (`CFB_IMPLEMENTATION_PROMPT.md`)
and stated in the module docs rather than started here:

- reassembling a repeating group from bare tag repetition, which needs the
  message-type grammar (`P7-R26`);
- rewriting a message between two FIX versions, which the lineage carries
  the facts for but does not perform (`P3-R12`);
- parsing `NAME=VALUE` pairs nested inside `XmlData(213)` (`P7-R52`).
