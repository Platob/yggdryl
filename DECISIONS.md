# The decisions the TextLine adaptation rests on

Eleven decisions and three amendments, settled while the FIX layer was moved
onto the text reader that landed in `main`, then made text, then given one
namespace. Each is one rule, and the module doc
named beside it is where the rule is written down.

They are here rather than in a scratchpad because they are the contract the work
was reviewed against: a later change that wants to move one of them has to argue
with what is written here, and a reviewer asking "why is it like this" should
find the answer without archaeology.

Decisions 1-7 were settled before the work began, from five reader reports that
measured both sides against the branch's own fixtures. Decision 8 was settled
when the codec's entry point changed, decision 9 when its paths did, and
decision 10 when the line became text, decision 11 when the registry's branches
went - each written, like the others, before the code that keeps it. The
amendments record where a decision turned out to over-claim once it met the
code - which is the part most worth keeping.

## 1. A frame narrows the scan; outside a frame the generic rule stands

**Rule.** `entry_spans` keeps its generic reading for text that merely carries
`k=v` pairs. Where the line carries a located frame, the pairs inside that frame
are the frame's segments cut at their first `=`, and a value inside the frame
ends only at the frame's separator.

**Why.** Both sides are wrong today, on different inputs. The scanner closes a
value at any of eleven bytes, so a FIX value that legitimately holds a space,
a comma, a semicolon or a bracket is truncated: `18=G L`, `48=ABBN SW`,
`58=quoting #A=1 and #B=2` and 71 of the 113 lines of the bridge capture read
differently under it. The codec's own separator choice is wrong the other way:
`separator_of` only ever answers SOH or `|`, so a space-framed or `;`-framed
frame reads as a single pair whose value is the rest of the line, and
`unescaped` rewrites only the earliest escape spelling, so a line mixing `^A`
and `<SOH>` reads as garbage. The frame is what decides a value's extent, and
`locate_frame`/`LineSeparator::for_line` already live in the same file and
already decide it for `inspect`.

This is the one place the scanner is widened, and it is widened because it is
wrong, not because FIX finds it inconvenient. It also fixes the key extent: a
key inside a frame is what precedes the first `=` of its segment, so
`NoAllocs[0].79=ACCT` and `Symbol[0]=AAPL` are pairs again rather than being
dropped by the backwards name-run walk.

**Written in:** `mime_type/line.rs`, on `entry_spans`.
**Fixtures:** a frame whose value holds each of the eleven generic terminators;
a space-framed and a `;`-framed frame; a line mixing `^A`, `\x01` and `<SOH>`;
a pipe-framed line whose value carries a raw SOH; a bracketed and a dotted key;
the same prose line outside a frame, which keeps the generic reading.

## 2. A length-prefixed data field is FIX's reading of what the scanner returned

**Rule.** The scanner never learns FIX's data tags. FIX walks the entries it was
handed, and where an entry's key is one of its data tags whose predecessor
stated a byte count, it re-slices that entry's value forward to the stated
length and drops the entries whose ranges fall inside the widened span.
`TextBytes::slice` stays inside the same page, so this copies nothing.

**Why.** Which tags carry a length is a fact of the FIX dictionary, and
`mime_type::line` holding twenty-one FIX tag numbers is the boundary this work
exists to draw. FIX can answer it from what it already has: the entries are in
line order, and every key and value exposes its page and its offsets.

The nested reading follows the same rule: the widened value is scanned as its
own entries and hung under that entry, which is what `TextEntry::entries`
already means, so the bridge's nested row is judged in its own scope rather than
as siblings of the frame's own tags.

The tag-213 fallback that reads to the trailer when no usable length was stated
is a FIX reading of a FIX trailer. It stays on the FIX side, where it is today.

**Written in:** `fix/codec.rs`, where the data span is taken.
**Fixtures:** the existing `212`/`213` frame, whose entry keys stay the six the
frame wrote and whose `into_bytes` still re-emits the frame byte for byte; a
`213` with no `212`, which takes the trailer fallback; a data value carrying the
frame separator; the bridge document row, whose ~80 nested fills survive.

## 3. A message is the frame; the line is still the line

**Rule.** The line keeps every pair it saw, in front of the frame and after the
checksum alike. A message is bounded by the frame the line carries: it begins
where `locate_frame` found it and ends at the checksum. FIX reads only the
entries whose ranges lie inside that bound.

**Why.** Two owners, two answers, and both are right about their own subject.
`entry_spans` deliberately does not stop where `inspect` stops, because a
transport's prefix is still part of what the line said; a message that swallowed
its log prefix would answer `ts` and `thread` as fields and re-emit them. The
codec already stops at `10=` for exactly this reason. Selecting a range of the
entries is cheap: every key exposes `start()`.

A line carrying two frames reads as the first, which is what the codec answers
today; the rest of the line is not part of that message.

**Written in:** `fix/codec.rs`, on the line entry point.
**Fixtures:** a frame behind a `k=v` log prefix; trailing pairs after the
checksum; two frames on one line; a frame with no checksum, which ends at the
line.

## 4. The entry records that the line marked a key; FIX judges the mark

**Rule.** `PairSpan` and `TextEntry` carry whether the line wrote a `#` in front
of the key, exactly as `LineEntry` already does on the `inspect` path. The key
range stays stripped, so every path that lifts a bridge key by name keeps
working. Every judgment built on the mark - the duplicate, the twin, the group
stem, the stated absence - stays on the FIX side, where it is FIX's reading of
what arrived.

**Why.** The mark is a fact about what the line wrote, and it is the one fact the
scanner currently destroys: after stripping, `#ORDERID=123` and `ORDERID=123`
are byte-identical, and the judgment that tells a duplicate from a restatement
has nothing left to read. Putting the `#` back into the key range would make
every other caller lift bridge keys under a marked name; probing the page behind
the key would read the same bytes a second time. One bool per entry, beside the
`nested` that is already there for this kind of mixed-form reading, is the
smallest true answer.

**Written in:** `media/text/entry.rs`, on `TextEntry`, and `fix/entry.rs`, for
what FIX does with it.
**Fixtures:** a bare pair and its marked restatement with equal bytes; with
differing bytes; a marked group stem; a marked key with no bare twin; a stated
absence under each null spelling.

## 5. An arrival entry is a range of the line, so a rendered path is not one

**Rule.** The arrival record keeps what arrived: a packed occurrence is recorded
as the pair the bridge wrote, which is a range of the line. Unpacking it into
`NOPARTYIDS[0].PARTYID` and its siblings is a reading that builds fields, not a
second arrival, so no entry carries a key that appears nowhere in the line.

**Why.** `FixEntry`'s keys become ranges of the page, and `render_members`
synthesizes dotted keys that no range can name: keeping them would mean one
fresh allocation per rendered key, which is the opposite of what this change is
for. Re-emission is also more faithful under this rule, not less, because the
packed pair goes back out as the bridge wrote it.

**Written in:** `fix/entry.rs`, on what an arrival is.
**Fixtures:** the packed-occurrence rows in the dataset and codec suites, whose
entry lists change to the packed pair and whose row values must not move; the
re-emission round trip over the whole capture.

## What each decision costs

| decision | where the code changes | what it must not move |
| --- | --- | --- |
| 1 | `mime_type/line.rs` only | every non-framed line reads as it does today |
| 2 | `fix/` only, plus a public way to scan a `TextBytes` | the `212`/`213` fixtures and the byte-for-byte re-emission |
| 3 | `fix/` only | the line's own entries, and the lifted columns built from them |
| 4 | one bool through `PairSpan`, `TextEntry` | key ranges stay stripped, so lifting is unchanged |
| 5 | `fix/` only | the row values a packed occurrence fills |

## 6. One spelling for a position: FIX takes the grammar's

**Rule.** A repeating-group occurrence is addressed as `Parties[0].PartyID`, the
spelling the one path grammar already writes. FIX's own `Parties.0.PartyID` -
a bare decimal segment read as an index - goes, and every call site, document
and binding test that writes it is re-spelled in the same change.

**Why.** The bare decimal already means something else one layer down: a text
line's entries reach the entry keyed `55` through the path `55`, and reserve
`[1]` and `[-1]` for position. Keeping FIX's reading would make one spelling
mean a key on the text side and an index on the FIX side, which is the second
reading the contract forbids. The grammar is not too narrow here - it already
writes positions, and FIX simply wrote them differently.

What must survive, and does, is FIX's *name* resolution: alias before exact,
the fold that drops case and separators, and the branch tiers (the tiers went
with decision 11; the rest stands). That belongs in how a segment resolves a
name against the registry, not in a second parser.

**Written in:** `fix/msg.rs`, on the path navigator, and `docs/fix/message.md`.
**Fixtures:** every `by_path`/`get_by_path` site in the FIX suites, the Python
and Node path tests, and a case proving a bare decimal now reaches a child
literally named `0` and nothing else, rather than silently answering none.

## 7. What "prove it" means here, since the pins the plan assumes do not exist

The allocation suite counts registry and message lookups and parses its one line
outside every counted closure; the call-count suite counts a registry folder and
the text reader separately and never the two composed. Both claims in the plan
are therefore new work, not existing pins:

- an allocation case that builds a message from one line inside the counted
  closure, at more than one pair count, so "reading a message from a line copies
  none of the bytes the entries name" is asserted rather than argued;
- a call-count case that reads one capture as text and then as FIX under one
  counted holder, so "one decode" is asserted;
- the equivalence harness, which is the thing worth more than either: before any
  behaviour moves, pin what the codec answers today over every fixture on this
  branch - the entry keys and values, the row values, the re-emitted bytes and
  the digest per message - and keep that green through every step.

## Amendment to decision 1, after implementing it

A `;`-framed frame is NOT fixed, and decision 1 over-claimed when it said so.
`LineSeparator::for_line` answers SOH, `|`, an escaped SOH, else whitespace -
never `;` - and the codec's own `separator_of` never answered `;` either,
whatever its doc said. So `8=FIX.4.4;35=D;11=A;10=123` reads as one pair whose
value is the rest of the line, which is what `inspect` and the codec both
already answer: the scanner stops being a third reading, which is the part that
mattered. Ranking `;` into the separator vocabulary would split a
whitespace-framed value that legitimately holds one - `58=a;b` - and that shape
is already documented as one value. If a `;`-framed frame is ever really wanted,
it needs a rank below whitespace and `for_line` needs a different shape; that is
a separate change with its own argument.

Space-framed frames ARE fixed: four pairs where the codec read one.

Also noted, and accepted: making the mixed-spelling line read as one frame meant
`LineSeparator::Marker` stops carrying one pinned spelling and means an escaped
SOH however the line spells it. That widens `inspect` and `classify`, not only
`entry_spans` - still inside `mime_type/line.rs`, but wider than decision 1's
cost table said. It is the same "one vocabulary" the marker table already
claimed, and the classifier's new answer is asserted rather than assumed.

## Second amendment to decision 1: what "carries a frame" means

The first implementation gated the frame rule on `locate_frame`, and reviewers
found that it answers `Some` for any line holding a single pair, because it ends
in `msgtype.or(first)`. Under that gate there is no "outside a frame" at all,
so decision 1's two halves could not both hold: every generic `k=v` line read
under the frame rule, and `host=srv1, port=8080, mode=fast` became one pair.

The gate is now: **a frame is a run of pairs the line named a separator for** -
a raw SOH, any escaped spelling of it, or a pipe. Whitespace ranks by position
among them but never names a frame on its own, so a line that named nothing is
read exactly as it was before any of this. That is a fact `LineSeparator`
already answers, so it adds no second notion, and it needs no list of dialect
markers inside the scanner.

Two consequences accepted with it: `LineSeparator::for_line` now ranks its
candidates by position rather than branching on numeric or symbolic, so a
space-framed frame whose value holds a pipe is no longer re-cut at that pipe;
and the classifying walk and the entry walk now share one reading of a segment,
parting on exactly one documented answer, a field given no value.

Still true and still documented rather than fixed: a line whose only pipe stands
in front of its first pair is not framed, and a `;`-framed frame answers one
pair, per the first amendment.

## What step 2 measured, and the claim it corrected

The allocation pin did not fall by two per pair; it rose by five per message
(22/37/87 becomes 27/42/92 at 4/16/64 pairs). The reason is honest and worth
keeping: the pin's own fixture writes four-byte keys and one-byte values, both
of which fitted `SmolStr`'s inline buffer, so the copies the entry stopped
making were never allocations on that line. The five that appeared are the page
the line is copied into plus the lists the two-stage read holds.

So the zero-copy claim is asserted where it is actually visible - every key and
value is the line's own bytes at the line's own offsets - rather than through a
count that cannot see it. A data field carrying three kilobytes now costs what a
two-byte `Side` costs, where an owned copy charged for every byte.

The page copy itself is step 3's to remove: a caller holding a decoded line
already owns that page and hands it over, instead of the codec paying for one
because it was handed a bare slice.

## 8. A per-row parameter is a row-header capture, resolved once

**Rule.** What a row states about its line arrives as the line's own typed
fields and its row-header captures, never as a name looked up in a map per row:

| what the record read | where it comes from now |
| --- | --- |
| the payload column | `line.body()` - a range of the page the reader already holds |
| the clock column | `line.timestamp()`, already nanoseconds UTC |
| `direction` | `line.direction()`, then the payload's own reading, then the codec's |
| `beginstring`, `pluginid`, every other fill | a row-header capture, resolved once |

The codec is told the capture names once - `FixCodec::with_capture_names`,
taking what `TextOptions::capture_names` answers - and resolves each there to
the one role it plays: the dialect, the version, the clock, the direction, or a
fill and the field it fills (the dialect role went with decision 11: a
`pluginid` capture is a fill and selects nothing). Per line it reads a capture
by position. A line carrying no captures states nothing, and nothing is
silence, never an error.

**Why not a lifted entry.** A lift reaches into the body, and the body is the
message. A fact read out of it is already a FIX field, which the codec fills
from its tag; reading one to decide how to read the rest is the message
configuring its own reading. And on the shape this exists for - a ULLINK bridge
line - `pluginid` is in the row header, so no lift reaches it at all.

**Why not a `TextOptions` FIX builds.** A capture's spelling is the caller's log
format, not FIX's. FIX building the options means FIX shipping one row-header
regex per log vendor, which is the dialect table in the wrong layer.

**What this deletes.** `fix/record.rs` whole: `parse_record_with`, the `Scalar`
map walk behind it, `DEFAULT_PAYLOAD_COLUMN`, and `FixCodec::parse_text_record`
/ `parse_text_records` with their two bindings. `RowParameters` goes as a pure
duplicate of `RowExtras` - the same four members, one converting to the other.

**What survives, and where it moves.** `BEGINSTRING_COLUMN`, `CLOCK_COLUMN`,
`DIRECTION_COLUMN`, `PLUGINID_COLUMN`, `is_parameter` and `version_of` move into
`fix/batch.rs`, the one reader that still has columns to name. That is not a
second answer to this decision: a batch *is* columns, and `Columns::resolve`
already resolves every one of them once per reader and indexes per row, which is
what the rule asks for. The line path names none of them.

**The page copy goes with it.** `parse_line_with` copies its bare slice into a
fresh page; `parse_text_line` hands over the page the line already holds, so the
five allocations step 2 added per message come back off.

**Fixtures:** the pipeline suites, which must read the same values through the
line entry point as they read through the record one; a line whose captures name
a dialect the dictionary declares and one whose captures name nothing; a line
with no captures at all; the allocation pin, which must fall back to at most what
it cost before step 2.

---

## Judgment held for the step-2 fix round: the space in a key stays

Two reviewers asked for `b' '` to be dropped from `is_segment_key` and the
`trailing note=x` counterexample restored. That option is wrong, and the proof
is upstream: `rust/tests/fix/codec.rs:365` runs `Msg Type=D` through `one_line`
- through the scanner - and requires the message to resolve msgtype `D`. That
line is on `origin/main` at the same number and entered in b3b4d59, so it
predates this branch. Narrowing the key bytes regresses a behaviour main
already has.

So the right resolution is the reviewers' other option: keep the widening and
put the real answer on the record. What stops a sentence from being a key is
not the space - a renderer spells `Msg Type`, and the fold that reads it as
`MsgType` ignores a space exactly as it ignores `_` - but the bytes a name
never holds. The guarding test already demonstrates it: ` sent >> seq=7` yields
only `seq=7`, because `>` is not a key byte. The doc's "never a sentence" is
the overreach, not the code.

What must land with it: the doc at line 736 and 775-779 rewritten to say that,
and `trailing note=x` added back beside `Msg Type=D` with its honest new
reading, so a two-word key inside a frame is a documented cost rather than an
accident.

## 9. One path grammar, two subjects: a schema has no positions, a value does

**Rule.** `FixMsg::get_by_path`/`by_path` and `FixRegistry::get_field_by_path`/
`field_by_path` take a resolved [`FieldPath`] and stop splitting strings. The
grammar is the crate's one grammar; what differs is what each walks:

| segment | against a message's value | against the registry's schema |
| --- | --- | --- |
| `Field(name)` | the child that name reaches, by FIX's own resolution | the same, a list recursed through transparently |
| `Index(i)` | the `i`-th occurrence a group holds | the item the list declares, because every occurrence has it |
| `Key(text)` | the child that text names | the same |

**Why an index resolves against a schema at all.** A schema states one item
type for a list, so `Parties[0].PartyID` and `Parties[7].PartyID` name the same
field. Answering the item rather than refusing means one spelling addresses a
member on both sides, which is what decision 6 asked for and what the old pair
could not do: the registry needed `Parties.PartyID` where the message needed
`Parties.0.PartyID`, and neither string worked on the other side.

**What survives, per segment.** FIX's name resolution is the behaviour this
must not lose: the codec's dialect first and then the standard branch
(`known_by_name`; superseded by decision 11, under which `known_by_name` is
the one namespace's lookup), alias before exact, the fold that drops case and
separators, and the registry's canonical spelling before a folded child
(`child_index`).
None of that is parsing, so none of it belongs in the grammar; it is how a
`Field` segment resolves a name against a dictionary.

**What goes.** The bare decimal as an index - `Parties.0.PartyID` - and the
`split('.')` in both navigators. A bare decimal now reaches a child literally
named `0` and nothing else, which is what it means one layer down, where a text
line's entry keyed `55` is reached by the path `55`.

**The whole-string-first trick costs nothing to lose.** Both navigators try the
entire string as a name before splitting. No name in the committed dictionary
holds a dot - checked, zero of them - so a one-segment path is exactly that
lookup.

**Fixtures:** every `by_path`/`get_by_path`/`field_by_path` site in the FIX
suites re-spelled; a case proving a bare decimal reaches a child named `0` and
answers none otherwise; a case proving one spelling reaches the same member
through the message and through the registry.

## 10. A line is text, and what arrived as something else is decoded where the line is made

**Rule.** A `TextLine`'s body is text. `TextLine::body` answers `&str`, and it
can, because the line is made text where it is made: `TextLine::from_bytes`
and `set_body` take the bytes the reader cut, and where those are not valid UTF-8
they are decoded once into a page of their own - every valid UTF-8 run kept as
it is, and every byte of every invalid run read as the character Windows-1252
gives it. A body that was valid UTF-8, which is every line of every capture this
crate holds, costs nothing: the same page, the same range, no byte read twice
beyond the validation. A body that was not costs one allocation, for that line.

The row-header captures take the same decode, because a capture is text the
header wrote and a line half text would be two readings of one line. The
refusal a non-UTF-8 capture used to raise - `expected a UTF-8 row-header
capture` - goes, with the branch that raised it.

The Arrow twin follows: the row's `body` column is `utf8`, required, and a
lifted entry column is `utf8`, nullable. What the reader answers as text it
declares as text; a `binary` column over bytes proven to be text would be a
second reading, and every consumer of it - the FIX batch reader, the row
writer, a caller reading a frame - would validate again what the reader
already validated. Writes consume only a non-null `utf8` body, and a batch
carrying a `binary` body is refused naming what was expected.

`TextEntry` stays a pair of ranges of the page (shape C of the three the
handover named): `key()` and `value()` answer `Cow<str>` infallibly, borrowed
wherever the range is text and owned only where it is not, and `key_bytes()`
and `value_bytes()` answer the ranges for the reader that needs offsets - the
FIX codec re-slicing a data field to the length its `Len` field stated, or
re-emitting a frame byte for byte. On a line the reader made, the borrowed
case is the only case: the scanner cuts a range at `=`, at a separator, at
whitespace and at the punctuation a transport closed a line with, all of
them ASCII, so no range it cuts ever divides a character. The owned case is
left for a page a caller built from bytes of their own.

`TextLine::decoded_byte_size` says how many bytes of the line as read were not
UTF-8 and were decoded; `0` for a line that was text as read. It is the one
fact the decode keeps, so a reader auditing a capture can find the lines the
reader repaired without decoding them again.

**The table.** Windows-1252 as the WHATWG encoding standard has it: `0x00`-`0x7F`
are themselves, `0xA0`-`0xFF` are `U+00A0`-`U+00FF`, and `0x80`-`0x9F` are the
classic table's punctuation, currency and letters, with the five bytes the
classic table leaves undefined - `0x81`, `0x8D`, `0x8F`, `0x90`, `0x9D` - read
as the C1 controls of the same number rather than refused. A byte the wire
held is a fact, and the reader never writes `U+FFFD` for one, because a
replacement character is the absence of a fact where the line had one.

**Why per byte rather than per line.** A capture is mostly UTF-8 with an odd
Latin-1 byte far more often than it is wholly Windows-1252 - a name a
Windows tool wrote into a log a Linux service otherwise wrote in UTF-8 - and
decoding a valid `é` (`C3 A9`) as `Ã©` because a lone `0xE9` stands elsewhere
on the line would destroy what was right to repair what was wrong. A wholly
Windows-1252 line has no valid multi-byte run to keep and decodes byte for
byte, so the per-byte rule reads it exactly as a per-line rule would. What
neither rule can do is tell `C3 A9` written as two Latin-1 letters from one
UTF-8 `é`; the reader takes the UTF-8 reading, because it is the encoding
the row's schema declares.

**Why one rule and no option.** A `TextOptions` charset would be a knob for a
case nobody has: this crate's captures are UTF-8 with stray bytes, and
Windows-1252 is the one decode of a stray byte that loses nothing, since it
maps every byte to one character. A capture in another encoding whole is a
different input with its own argument, and it can have its own option then.

**Why the decode comes last.** Everything the reader does to a line's bytes
before the line exists - the row-header match, `lstrip`/`rstrip`, the
direction, `max_record_byte_size`, `dropped_byte_size`, adjacent
deduplication - is a fact of the bytes as read, in bytes as read, and stays
so. A limit stated in bytes bounds the wire, not the decode: a body of N
wire bytes decodes to as many as 3N. A truncation that cut a multi-byte
character in two leaves its orphan bytes invalid, and they decode as the
Windows-1252 characters they are - `E2 82` reads `â‚` - which is the honest
answer to a reader that asked for N bytes and got them.

**What it collides with, and how each is settled.**

- *Decision 2's stated length.* A data field's `Len` counts wire bytes, and
  the codec re-slices the value to it in page offsets. On a line the reader
  made, the page is the wire wherever the wire was text, so the length lands
  where it landed before - the 264 messages of the equivalence snapshot among
  them. Where the wire was not text, the decoded value is longer than its
  stated length, the length reaches no boundary the frame stated, and it is
  not honoured - exactly as any stated length that reaches no boundary is not
  (`fd76e05`): the value stays what the frame cut. That is a loss, and it is
  named here rather than hidden: a binary data field on a line that was not
  UTF-8 is read as text to the separator, and the line's
  `decoded_byte_size` says the line was decoded. The codec's own byte doors -
  `parse_fix_line`, `parse_ullink_line`, `own_pairs` - take bytes as given
  and decode nothing, so a caller holding the wire still reads it as the wire.
- *Byte-for-byte re-emission.* `FixMsg::into_bytes` re-emits the entries, and
  the entries are ranges of the line's page, so a message read from a text
  line re-emits that line's text: the wire as received wherever the wire was
  text, and the decode of it where it was not. `.wire` in the snapshot cannot
  move, because the corpus is UTF-8 throughout.
- *The `Lossy` anomaly.* It keeps its meaning - a value that is not text
  reaches the row as a decode of it - and it keeps its fixture, which goes
  through the byte door. It cannot arise from a line the reader made, because
  that line was text before the codec read it; that a line was decoded is the
  line's fact, `decoded_byte_size`, and not a message's.
- *Decision 5.* An entry is still a range of the line, and the line is still
  the line; the decode happens before the line exists, so nothing here makes
  an entry carry bytes the line does not.

**Written in:** `media/text/line.rs`, on `TextLine`, with the table beside
the decode; `media/text/entry.rs`, on the two accessors; `docs/media/text.md`.
**Fixtures:** a line with one Latin-1 byte among UTF-8, decoded to the one
character; a wholly Windows-1252 line; each of the five bytes the classic
table leaves undefined; a line `max_record_byte_size` cut inside a
character; a capture holding an invalid byte, reaching its typed column
decoded; a data field carrying an invalid byte read through the text reader,
which stays as the frame cut it and raises no `Lossy`, beside the same line
through `parse_fix_line`, which reads the wire and does; the equivalence
snapshot, unmoved; the Python and Node record dictionaries, answering `str`.

**What it costs.**

| where | what changes | what must not move |
| --- | --- | --- |
| `media/text/line.rs` | `body()` answers `&str`, `body_bytes()` the range; `from_bytes`/`set_body` decode; `decoded_byte_size` | the page and offsets of every UTF-8 line |
| `media/text/arrow.rs`, `plan.rs`, `batch.rs` | `body` and lifted columns `utf8`; the writer consumes `utf8`; captures decoded at `convert` | wire-byte counts: `dropped_byte_size`, `max_record_byte_size` |
| `media/text/entry.rs` | `key()`/`value()` answer `Cow<str>`; `key_bytes()`/`value_bytes()` the ranges | every range the scanner cuts |
| `fix/` | call sites read the ranges through the `_bytes` accessors | the equivalence snapshot; the byte-door fixture in `tests/fix/message.rs` |
| Python, Node | `body`, `key`, `value`, captures answer text; `key_bytes`/`value_bytes` beside them; `decoded_byte_size` | argument order and error semantics |
| docs, `AGENTS.md` | `body: utf8`; the examples read text | |

## 11. A field is its tag and its name; a dictionary is a membership, not a namespace

**Rule.** A FIX field is identified by its tag and its name together, and by
nothing else. `FixId` is one `i32`: the signed XXH32, under a fixed seed, of
the tag's four little-endian bytes followed by the name under the crate's one
fold - ASCII case dropped, `_`, `-` and space dropped - so `Msg_Type`,
`msgtype` and `MsgType` under tag 35 are one id, and the id agrees with what
every name lookup already answers. It is derived on every read from
`fix:tag` and the field's name, never written into a dictionary shard, for
the reason `FixField::id` gives today: the registry, the catalog, the store
and the CBlock reader rename a field after it is built, and a stored id
would go stale where a derived one cannot. Wherever an id crosses a boundary
- `FixKey::Id`, `FixMsg::get_by_id`, Python, Node, a row - it is that
integer, rendered as its decimal; `FixKey::from(i32)` keeps meaning a tag,
and an id is always spelled `FixKey::Id`, because one integer must not have
two readings. `FixMsg::lift_source` answers the tag a facet was read from,
because a lift source is declared by tag and the tag is the whole of what
it is; the field that tag names is the registry's to answer.

The branch goes. `FixBranch`, the branch half of the old identity, the
`tag:branch` spelling, the admissibility rule that let a dialect claim only
`5000..40000`, the branch table and its order, the four-tier resolution
(standard canonical, named canonical, standard alias, named alias), every
`Option<&FixBranch>` parameter, the codec's pin and its per-row dialect, the
builder's and the message's own-branch step, the store's `branches.json`
manifest and per-branch folders, and the `branches` array of the JSON
snapshot are all deleted. What a dictionary contributes is recorded on the
field it contributed to: `fix:branches` is a comma-separated list of
dialect names, each validated as an alias is (non-empty, no `,`), folded by
ASCII case once, deduplicated under the crate fold and kept sorted so that
two registries built from the same dictionaries in any order hash alike.
`FixField::branches` and `has_branch` read it, `FixFieldMut::set_branches`
and `add_branch` write it, `merge_with` unions it, and a message root the
codec builds carries none, because a message is not a dictionary member.
`FixRegistry::dialects` lists the distinct names any field or definition
carries. Membership is provenance a caller filters on; resolution never
consults it.

**What one namespace means for a field that arrives.** The identity is the
pair, so three things can happen when a field is added or merged:

| the registry holds | the arrival | what happens |
| --- | --- | --- |
| the same tag under the same folded name | the same field | update: `merge_with` unions its tags, aliases, membership and the rest, as today |
| the same tag under another name | a new field | it is registered under its own id, *and* the holder gains the arrival's name as an alias; a bare wire tag keeps answering the first holder, the newcomer is reached by its name or its id |
| the same folded name under another tag | the same field spelled with another tag | it merges into the holder, which gains the tag as an alternate; no second field |
| neither | a new field | it is inserted as it arrived |

A name is what identifies a field to a reader, so a new name on a held tag
is a new thing a dialect defined over a tag it reused; a tag is what
identifies a field on the wire, so a held name on a new tag is the same
thing spelled with another number. The asymmetry is the rule and not an
accident of it. Two identities hashing to one id - possible, since the id
is 32 bits - is a typed conflict on insert, exactly as a digest collision is
today, and every id hit is rechecked against the tag and the fold before it
counts.

**Resolution without tiers.** Canonical before alternate for tags, canonical
name before alias for names, the fold always, and nothing else:
`get_field_by_id` is exact; `get_field_by_tag` answers the canonical holder
of a tag, then an alternate; `get_field_by_name` answers the canonical fold,
then an alias fold; `get_field_by_path` is decision 9 unchanged. On a
message, `known_by_tag` and `known_by_name` are those two lookups, and
`child_index` keeps the registry's canonical spelling before an exact child
before the one folded child. Message codes live in one map under the same
rule as fields - a definition re-declaring a code under the same folded name
folds into the stored one, under another name it is a second message reached
by its name. The bare code answers the message named as tag 35's code set
names the code - what the specification calls it, and what a reader of
`35=D` means - else the first in name order: both are facts of the catalog's
content rather than of the order it was built in, so a dictionary folded,
stored and loaded answers the same message, and a venue's second message on
a specification code never takes that code from the specification's. The
counter tables are keyed by counter tag, since every caller holds a tag.
`known_msgtype` and `known_group` go with the tiers they existed for.

**The codec.** No pin, no dialect: `FixCodec::with_branch`, `dialect_of`
and `tier` go, `RowTier` is the declared message alone, and `RowExtras`,
`RowStamp`, `Builder` and `FixMsg` lose their branch member. A row's
`pluginid` capture fills the crate's `pluginid` field and selects nothing.
A dialect's default FIX version loses its home with the branch: the
version a row reads at is the row's own `beginstring` capture or column,
else the `FixCodec::with_version` pin, else what the line states -
`ApplVerID`, then `BeginString` - else the registry's newest, exactly as
before with the dialect's default step taken out; a CBlock file that
declared one is read under the pin the caller states.

**Crate fields as tuples.** Every field this crate defines is declared once
as a `(tag, name)` pair - `PLUGINID_TAG_NAME`, `TIMESTAMP_TAG_NAME`,
`VERSION_TAG_NAME` and the rest - because that pair is what identifies it,
and a caller that names one of them names both halves. The bare `*_TAG`
constants and `TIMESTAMP_NAME` are deleted with the branch. ULBridge's own
fields keep their tags from `ULBRIDGE_TAG_MIN` and carry
`fix:branches=ulbridge`; that spelling is `ULBRIDGE_DIALECT`, which replaces
`ULBRIDGE_BRANCH`. The derived definition-tag block stays, as it never
depended on a branch; `USER_TAG_MIN`/`USER_TAG_MAX` and `is_admissible` go,
since nothing gates a tag on its dictionary any more.

**Ingest and the store.** `FixRegistry::from_cfb_file(handle, dialect)` and
`FixField::from_cfb_file` stamp the dialect on every field, group, component
and message the file produces - standard tags included, since membership
means "this dictionary speaks it"; `add_cfb_file(handle, dialect)` takes
the dialect or the file's stem and merges under the table above; the
`aliases` parameter goes with `dialect_of`. The store writes
`fields/<shard>.json` and `<category>/<name>.json` and nothing keyed by
branch; a placeholder does not copy the target's membership, as it does not
copy its tag. The shipped dictionary under `config/fix` changes in no file,
because it never declared a branch and the id is not stored.

**Order.** Tag-major, then the tag's holder, then id, so `FixFieldIter` and
the bindings' documented ascending iteration keep their order on a registry
of any size - and so the store, which writes in that order and loads in
file order, hands the bare tag back to the field that held it: the holder
is decided by arrival and survives a round trip because it is written first.

**What it collides with.** Decisions 6 and 9 name "the codec's dialect first
and then the standard branch" among what survives: that clause is
superseded here, and the rest of both - alias before exact, the one fold,
canonical spelling before folded child, the list-transparent descend, the
unique-head rule - is what this decision keeps. `FixRegistry::stable_hash`
moves, because the branches it folded in are gone. The Iceberg snapshot
branches are another word and untouched.

**Written in:** `fix/mod.rs`, on `FixId`; `fix/field.rs`, on `fix:branches`;
`fix/registry.rs`, on the insert rule; `docs/fix/registry.md`.
**Fixtures:** the three rows of the table, each through `add_field` and
through `merge_with`, with the alias and the alternate asserted on the
holder and the bare tag answering the first holder; two identities forced
onto one id refused; `Msg_Type`, `msgtype` and `MsgType` one id; a CBlock
merged under a dialect stamping every field it touched and unioning onto a
standard one; the one message-code namespace; iteration order over a
registry holding two fields on one tag; the equivalence snapshot, unmoved;
the allocation pins over the unseeded fold, still zero.

**What it costs.**

| where | what changes | what must not move |
| --- | --- | --- |
| `fix/mod.rs` | `FixBranch` deleted; `FixId` one `i32` with `of(tag, name)`; the colon grammar gone | `FixKey`'s three doors; `From<i32>` = a tag |
| `fix/field.rs` | `fix:branch` -> `fix:branches`; the branch-identity refusal in `merge_with` gone | the tag-must-agree refusal; every other `fix:` key |
| `fix/registry.rs`, `catalog.rs`, `memo.rs` | one namespace: tag, alternate, name, alias and id indexes, unseeded; the table above; counters by tag; one code map | `identity_of`'s borrowed read; zero allocations per lookup |
| `fix/codec.rs`, `build.rs`, `msg.rs`, `batch.rs`, `ulbridge.rs`, `lift.rs`, `anomaly.rs`, `latest.rs` | no branch member, pin, tier or dialect; `pluginid` a fill; version as stated above | the wire bytes, the entries, the row values, the equivalence snapshot |
| `fix/cfb.rs`, `store.rs` | dialect stamped on every produced field; no manifest, no branch folders, no snapshot `branches` | the shard arithmetic; provenance checks; the shipped files |
| `fix/crated.rs` | `*_TAG_NAME` tuples; `ULBRIDGE_DIALECT` | the tags themselves |
| tests, benchmarks | the branch pins go or are re-spelled; `numeric_branch.rs` goes | `tier_order_never_lets_an_alternate_key_shadow_a_canonical_one`, the path and fold cases |
| Python, Node, CLI | `field.fix.id` an int, `fix.branches` a list, `--dialect`; every `branch=` gone | argument order and error semantics elsewhere |
| docs | eleven pages and the explorer re-spelled | |
