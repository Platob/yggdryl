# The decisions the TextLine adaptation rests on

Decisions and amendments settled while the FIX layer was
moved onto the text reader that landed in `main`, then made text, then given
one namespace, then met the charset layer, and then - from decision 13 - had
its messages folded into its components, its direction made FIX's, its rows
read for every message they carry, its bridge configuration read as a plugin,
its two passes made one, and its identifiers declared by components. Each is one rule, and
the module doc named beside it is where the rule is written down.

They are here rather than in a scratchpad because they are the contract the work
was reviewed against: a later change that wants to move one of them has to argue
with what is written here, and a reviewer asking "why is it like this" should
find the answer without archaeology.

Decisions 1-7 were settled before the work began, from five reader reports that
measured both sides against the branch's own fixtures. Decision 8 was settled
when the codec's entry point changed, decision 9 when its paths did, and
decision 10 when the line became text, decision 11 when the registry's branches
went, and decision 12 when the charset layer and the text line met in one
merge - each written, like the others, before the code that keeps it. The
amendments record where a decision turned out to over-claim once it met the
code - which is the part most worth keeping. Decisions from 13 follow
`FIX_DIRECTION_MESSAGES_PROMPT.md` one per commit, each written before
its code and each named in the commit that keeps it.

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
today; the rest of the line is not part of that message. (Decision 16 amends
that last sentence: the rest of the line is the next message where it opens a
frame of its own. What a message is bounded by does not move.)

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

**The table.** (Decision 12 moves the table out of `media/text/line.rs` and
into the charset layer, which generates it, and adds the case of a handle that
declares its charset; the rule below is unchanged by it.) Windows-1252 as the
WHATWG encoding standard has it: `0x00`-`0x7F`
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
(Decision 12 gives that input its argument - the handle's media type, which
already owns it everywhere else - and keeps this sentence: there is still no
option.)

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

## 12. One reading of a stray byte, owned by the charset layer; the charset a line is read in is what its handle declares

Settled when this branch's charset layer met decision 10 in one merge. The two
had written the same fact twice and a third fact three ways: `media/text/line.rs`
held a hand-written Windows-1252 table beside the generated one in
`charset/tables.rs`; bytes offered as UTF-8 that are not UTF-8 read per invalid
run as Windows-1252 through the text line and whole as ISO 8859-1 through
`Charset::Utf8.transcribe` (`caf\xC3\xA9 \xE9` is `café é` by the first and
`cafÃ© é` by the second); and `TextOptions::charset` decoded row-header
captures strictly while decision 10 decodes captures and body alike where the
line is made, so the option's own contract - "captures only, `body` stays the
bytes that arrived" - was false the moment the two met. Three rules settle it,
and each was tried against the code by three readers instructed to refute it
before it was written here.

**Rule one - the reading.** Bytes offered as UTF-8, or as US-ASCII, that are
not what they were offered as are read - never refused, never `U+FFFD` - by
one rule written once in the charset layer: every valid UTF-8 run is kept as it
is, and every other byte reads as the character Windows-1252 gives it, as the
WHATWG Encoding Standard tables it - `0x80`-`0x9F` the classic table's
punctuation, currency and letters, `0xA0`-`0xFF` as `U+00A0`-`U+00FF`, and the
five bytes the classic table leaves undefined (`0x81`, `0x8D`, `0x8F`, `0x90`,
`0x9D`) as the C1 controls of the same number. Per invalid run, because a
capture is mostly UTF-8 with a stray byte far more often than it is wholly
Windows-1252 - decision 10's reasoning, now the layer's. The table is the
generated `CP1252`, checked against Python's codec registry in both directions,
and the rule for its five holes is the one the layer already applies to an
unassigned byte of any Windows page (`SingleByte::transcribe_sink`), so nothing
is restated: the reading is `charset::unicode::utf8_transcribe_into`, a walk
over `utf8_chunks()` that copies each valid run and sends each invalid run
through the table's transcribing walk, answering how many bytes it read that
way. `Charset::Utf8.transcribe`, `Charset::Ascii.transcribe` and their
`transcribe_smol` are that function; `TextLine::from_bytes` calls it in the
branch a valid line never enters and holds no table of its own. US-ASCII reads
exactly as UTF-8 - a US-ASCII declaration is a UTF-8 declaration with a
narrower promise, and a broken promise about UTF-8-compatible text has one
rule - so `Ascii.transcribe(b"caf\xC3\xA9")` is `café`, not `cafÃ©`, which the
WHATWG `us-ascii` label (the windows-1252 decoder, whole) would answer; WHATWG
is cited for the byte table and for nothing else. What does not move: `decode`
refuses and names the byte, `decode_lossy` marks it `U+FFFD`, an unassigned
byte of a single-byte charset reads as its C1 control, a lone UTF-16 surrogate
stays `U+FFFD` because no byte-wise reading of it exists, and `Str::from_bytes`
still decodes `utf8`/`ascii` columns strictly and transcribes every other
charset, so no stored value changes.

**Rule two - the owner.** There is no charset option on the text record
reader. The handle's media type is the one owner of "which charset are these
bytes in", as it is for a structured document (`text::io::Plan`) and for
`Transcoded`: the reader reads `Charset::from_media_type(handle.media_type())`
once, where it is built, beside the codings it already reads there, and
nothing past that point holds a `Charset`. Decision 10's sentence stands as
written - a `TextOptions` charset is a knob for a case nobody has - and the
charset contract's two sentences stand with it: a layer that wants a charset
argument wants the wrong seam, and reading a resource in one charset is not a
second option on every reader. The explicit argument the precedence rule names
is `Transcoded::new(handle, charset)` in Rust and `set_media_type` on every
handle in every binding, which exist. `TextOptions::charset`, `set_charset`,
`with_charset`, `TextBytes::decode`, the `charset` field of `TextLines`, the
Python and Node accessors, the pickle item and the options-table row are
deleted; the strict `capture_value`/`row_timestamp` path is `main`'s again.

**Rule three - the declaration is applied at the transport.** Where the
declared charset is neither UTF-8 nor US-ASCII, the reader lays a transcribing
stream decoder over the coding chain at the three places it builds a transport
(`read_owned_arrow_reader_at`, `read_owned_text_lines_at`, `row_size`) - coding
first, charset second, the order `text::io::Plan` already composes in - so the
line splitter, the row header, `lstrip`/`rstrip`, the direction, adjacent
deduplication, classification and the entries all read the declared text, and
`TextLine::from_bytes` finds every line text as read. Under UTF-8 or US-ASCII
nothing is wrapped: the transport is the object `main` builds, and every line
takes `main`'s instructions. The stream transcribes and never refuses, as
decision 10 never refuses: a byte the declared charset leaves unassigned reads
as the C1 control of its number, a lone surrogate as `U+FFFD`, and a Unicode
sequence the source cuts short at its very end as `U+FFFD` for the bytes that
are left. A byte-order mark that names the declared UTF-16 form is taken off;
every other mark is data - a mark of the other endianness under a declaration,
and a UTF-8 mark under UTF-8, US-ASCII or no declaration at all, which stays
the first three bytes of the first line as it does on `main`. The structured
plan strips whatever mark it finds, because a parser would refuse `U+FEFF`; a
line reader reads the wire, and a mark that is not the declared form's own is
a fact of it. The writer follows
the same declaration, or a declared handle would read its own UTF-8 back as
legacy bytes: bodies are rendered through `Charset::writer` around the coding
writer, and an append compares the tail against the terminator as encoded.

Two consequences are the rule and not an accident of it. First, under a
declaration every count the reader takes in bytes - `max_record_byte_size`,
`dropped_byte_size`, the decoded size a limit is measured against - is a count
of the bytes it split, which are the decoded bytes: the units a coding already
gives (a gzip'd capture's "bytes as read" were never its compressed bytes) and
the units `Transcoded` gives. "Bytes as read" means below the transport, and
the charset is transport. Decision 10's "the limit bounds the wire" stands for
the undeclared case, where the transport is the wire. Second,
`decoded_byte_size` keeps one meaning - how many bytes the reader had to read
other than as declared, so a reader auditing a capture can find the lines it
repaired - and is `0` under a declaration: the reader did what the handle said
and repaired nothing, and a file mis-declared as Windows-1252 reads as the
mojibake it was declared to be. One edge is the limit's and not the
declaration's: a `max_record_byte_size` that lands inside a decoded scalar
leaves the stray bytes the cut made, and the line reads and counts them
exactly as an undeclared read would - `Zürich` cut at two decoded bytes reads
`ZÃ` and counts `1`. Whether a resource was declared is the
handle's fact, `MediaType::charset`, and not a per-line count.

**Why the transport and not the line, and not a resolved reading at the
assembly.** The charset contract forbids a record layer branching on a
charset per row, and the rule is structural: a `Charset` compare per line is
not measurable against the validation of a kilobyte line (three orders below
criterion's noise on `text_lines/decode`), and it is refused all the same,
because a `Charset` in `TextLine`, in `TextLines` or in `line.rs` is a second
place the fact lives. A per-line page (decode after the split, into a page of
its own) would also match the row header against wire bytes the handle has
just said are not UTF-8, so `\S+` in Unicode mode never matches `ü` and the
capture is silently null, and it would pay two allocations and a second walk
on every declared line holding a high byte. A reading resolved once and
applied where the splitter copies each part into the line's vector keeps wire
units but puts a second unit into eight arithmetic sites of the reader's
hottest loop, each a place where one missed site is silent mojibake, and
cannot read UTF-16 at all. The transport touches nothing per line, reads
UTF-16 because it decodes before it splits, composes with the codings the way
the structured reader already does, and is the one door `owned_handle` cannot
bypass, because it copies the media type onto the handle it re-opens. Its
cost, stated: two 64 KiB buffers and one `Box` per declared reader - and one
more allocation, per reader and never per line, where a resource runs past one
window and its first chunk decodes longer than the window it was reserved for
- and two
`memcpy` passes over every byte of a declared resource (ASCII runs by the
word-at-a-time scan, high bytes by one table entry each); a mostly-ASCII
Western export pays those passes where a per-line page would borrow every
all-ASCII line, and a Cyrillic log pays no allocation per line where a per-line
page would pay two.

**What it decides that decision 10 left open.** Decision 10 could not tell
`C3 A9` written as two Latin-1 letters from one UTF-8 `é` and took the UTF-8
reading because it is what the row's schema declares. With a declaration the
reader can tell, and does: under `text/plain;charset=windows-1252` those bytes
read `Ã©`, under `windows-1251` `ГЁ`. The undeclared case keeps decision 10's
rule unchanged.

**What it collides with, and how each is settled.**

- *Decision 10's "no option".* Kept, literally: the sentence stays in
  `DECISIONS.md` and in `docs/media/text.md`, and the merged tree carries no
  option.
- *Decision 10's "the decode comes last".* Kept for the undeclared case and
  for every count's meaning; under a declaration the transport decodes below
  everything, and the counts follow the transport as they already follow a
  coding. The docs row that says "counted in bytes as read" says what "as
  read" means.
- *Decision 10's table paragraph and fixtures.* The table's owner moves; not
  one of its six fixtures moves in value, and the one that asserted the hand
  table (`windows_1252(byte) == byte as char` for the five holes) asserts the
  generated table instead (`Charset::Cp1252.scalar_of(byte).is_none()`).
  Equivalence rests on two facts. One the standard library guarantees and
  the table's walk does not even lean on: `utf8_chunks()` never puts a byte
  below `0x80` in an invalid run, and the walk reads an ASCII byte as itself
  before it reaches a table slot, so the walk is one entry per byte with
  `char::from(byte)` for the five width-0 holes - exactly the deleted `match`.
  The other is pinned at the layer: the walk is byte-wise over the whole
  buffer, never the chunked `Decoder`, which would call `E2 82` at the end of
  a line pending and refuse at `finish` where decision 10 reads `â‚`.
- *The charset contract's "non-UTF-8 offered as UTF-8 as ISO 8859-1".*
  Changed to rule one, in `docs/charset/index.md`, the `Charset::transcribe`
  doc and the `AGENTS.md` Charsets bullet; `Utf8.transcribe(b"caf\xe9") ==
  "café"` and `Cp1252.transcribe(b"ok\x81") == "ok\u{81}"` hold; what moves was
  pinned nowhere: `Utf8.transcribe(b"caf\xC3\xA9 \xE9")` from `cafÃ© é` to
  `café é`, `Utf8.transcribe(b"\x93x\x94")` from two C1 controls to `“x”`,
  `Ascii.transcribe(b"\x80")` from `U+0080` to `€`.
- *`Transcoded` refuses where the record reader transcribes.* Two doors, two
  contracts, both stated: a document read whole and addressed at random names
  damage by position (`Error::Codec`, through `Transcoded` and `text::from_io`),
  and a line is a wire fact a capture reader never refuses a whole file for.
- *A mis-declared handle.* A media type set from a stale `Content-Type` -
  `charset=iso-8859-1` over UTF-8 bytes, a common server default - now reads
  the mojibake it declares where `main` repaired nothing and read `é`. Named
  as the hazard it is, in `docs/media/text.md`, and pinned; the override is the
  handle's media type or `Transcoded::new(handle, Utf8)`.
- *The known bypass.* `Transcoded<File>` and `Coding<File>` delegate
  `bound_location` to the file they wrap, and `owned_handle` re-opens that
  location raw under the wrapper's stripped media type, so a record read
  through either wrapper over a local file decodes nothing. Pre-existing, on
  both sides of the merge; not fixed here, named in the hand-off with its fix
  (a byte-changing wrapper answers no bound location), and the reason
  `docs/charset/transcoded.md` now points a declared handle at its media type.
- *The `Decoder`'s refusal position.* `push_sink` reported a byte position
  relative to the chunk, not to the first byte the decoder was fed as its doc
  promises. Fixed beside the transcribing mode, since the same lines change;
  pinned.

**Written in:** `charset.rs` (the module doc's intake list, the `transcribe`
doc, `transcriber`), `charset/unicode.rs` (`utf8_transcribe_into`),
`charset/decoder.rs` (the transcribing mode), `media/text/line.rs` (on
`decoded`), `media/text/arrow.rs` (the three transport sites and the writer),
`docs/charset/index.md`, `docs/media/text.md`, `AGENTS.md` Charsets.
**Fixtures:** at the layer, the six decision-10 lines re-run through
`Utf8.transcribe` unmoved in value, the mixed line, the `0x80`-`0x9F` row
against the WHATWG index entry by entry, `58=\xE2\x82` read as `â‚` with count
`2`, `Ascii` on `caf\xC3\xA9` and on `\x80`, `transcribe_into` counting `1` for
`caf\xe9` under UTF-8 and `0` for all-ASCII under any charset; through the
reader, a `text/plain;charset=windows-1251` buffer reading `Москва` where
`text/plain` reads `Ìîñêâà`, `C3 A9` under a declared `windows-1252` reading
`Ã©`, body and captures alike as `utf8` with `decoded_byte_size` `0`, a
Unicode-class header capturing `Zürich` under a declaration, `0x81` under a
declared `windows-1252` reading `U+0081` with no refusal, a UTF-16LE capture
with its mark splitting into two rows with the mark gone, a declared
`iso-8859-1` handle over UTF-8 bytes reading mojibake by declaration, a byte
limit under a declaration counting decoded bytes, a declared buffer round-
tripping through the writer with one wire byte per scalar, a `Transcoded`
buffer needing nothing, a declared `us-ascii` handle over `caf\xe9` reading
`café` with `decoded_byte_size` `1`; in the counting allocator, an absolute
pin on `read_text_lines` over UTF-8 lines at 16 and at 1 024 rows with the
per-line slope stated, and a declared read costing three allocations over the
UTF-8 read of the same lines under one window and four past it, at 16 and at
1 024 rows; the equivalence snapshot, unmoved;
the Python and Node record dictionaries answering `str` for a declared body.

**What it costs.**

| where | what changes | what must not move |
| --- | --- | --- |
| `charset/unicode.rs`, `charset.rs` | `utf8_transcribe_into`; `Utf8`/`Ascii` `transcribe`, `transcribe_smol` and `transcribe_sink` read through it; `Charset::transcriber` beside `decoder` | `decode`, `decode_lossy`, `decode_into`, every table, every single-byte `transcribe`, the all-ASCII borrow, `INLINE_CAPACITY` |
| `charset/decoder.rs`, `charset/reader.rs` | a transcribing mode that never refuses; positions rebased on `consumed`; `Reader` generic over its source so `Send` follows it | `Decoder`'s strict mode and its carry; `Charset::reader` |
| `media/text/line.rs` | `windows_1252` deleted; `decoded` calls `utf8_transcribe_into` in the branch a valid line never enters | `from_bytes`/`set_body`/`set_captures` and their contract; the page and offsets of every UTF-8 line; every decision-10 fixture |
| `media/text/options.rs`, `bytes.rs` | `charset` and its accessors deleted; `TextBytes::decode` deleted | every other option; `as_str` |
| `media/text/arrow.rs` | the charset resolved once beside the codings at the three transport sites and laid over the coding chain when not UTF-8/US-ASCII, the mark taken off a declared Unicode form; `TextLines.charset` deleted; `encoded_bodies` and the append tail through the declaration | `RawRows`, `convert`, the `Utf8` transport object, the Arrow schema (`body: utf8`), every UTF-8 corpus fixture |
| Python, Node | `TextOptions.charset` and the pickle item deleted; nothing added | `set_media_type`/`setMediaType`, `MediaType.charset`, argument order and error semantics elsewhere |
| docs, `AGENTS.md` | the rule stated once in `docs/charset/index.md` and pointed to; the options row deleted; a "declaring a charset" section on the text page; the intake list and the last Charsets bullet re-spelled | "There is no charset option"; the precedence bullet |

**How it lands.** Three commits, so each number has one cause: the merge,
with this decision written and the option deleted; the reading moved into the
layer; the declaration at the transport. Each is measured against the one
before it on `text_lines`, `text_record_framing`, `text_scan`, `fix/line` and
`fix/pipeline`, named baselines, a quiet box; `text_scan` and `fix/line` never
see a charset and are the controls, and a regression above criterion's noise on
`text_lines/decode/*`, `text_record_framing/oversized/*` or
`fix/pipeline/{text_read,parse_text_arrow_reader,parse_lines}` falsifies the
commit that shows it.

## 13. A message is a component that names its code; the catalog has three categories

**Rule.** `FixCategory` is `Fields`, `Components`, `Groups`. A message is a
component whose `fix:msgtype` names the wire code it answers, and nothing
else tells the two apart. The type map is the whole rule for what a
definition is: a scalar datatype is a field, a `Struct` is a component, a
`List` or `LargeList` of a non-null `Struct` is a group. `definition_category`
answers by shape alone, `check_shape` holds each category to its shape, and
the marker decides only whether a component is also a message. `MsgType`
still wraps one; `msgtypes()` iterates the components carrying the marker in
name order and `msgtype_at` indexes that list; `register_msgtype` creates its
empty Struct in `Components`; the message-code map is decision 11's, unmoved
- one namespace of codes, the bare code answering the holder tag 35's set
names, else the first in name order. The store writes
`components/<name>.json` for a message as for any component, loads
`[Fields, Components, Groups]`, and a snapshot has exactly those three keys:
`messages` is refused by name, as any key outside the three is. The
generator writes the 181 message documents into `components/`, byte for byte
what they were: a moved document keeps the `fix:tag` it states, so no derived
tag moves and provenance changes only by path, and `--check` proves the
layout.

**Why.** A message was a Struct `Field` whose only difference from a
component was the marker - `definition_category` and `MsgType::from_field`
said so in code - and the catalog keyed both by `(category, folded name)`, so
the category was the marker restated as a namespace. Two namespaces for one
shape cost every walk a third arm: `get_field_by_path` tried three roots,
`Hash`, `merge_catalog`, `definition_tag_in_use`, `LOAD_ORDER`, the CLI's
`ingest` and the generator each listed the categories in an order of their
own, and a component that gained the marker had to move between namespaces
rather than gain a property. One category makes the marker a property of a
component, which is what it is, and a reader asking "which components are
messages" asks the marker rather than a folder.

**The collision the fold could create.** A message and a component with one
folded name would now be one key. Census of the shipped dictionary: none of
the 181 message names folds equal to any of the 747 component names, because
the generator reserves every published name before it claims one and
suffixes a message `Message` on collision (`claim`). The census is the
fixture: the shipped registry holds 928 components, 181 of them carrying
`fix:msgtype`, and 580 groups, pinned wherever the old three counts were.
Were a pair ever to collide, the generator's `Message` suffix is the answer
and that pin is where it would show.

**What moves.**

| where | what changes | what must not move |
| --- | --- | --- |
| `fix_category.rs` | three variants, `ALL: [Self; 3]`, the refusal names three | `as_str` = the folder name; `Serialize`/`Deserialize` |
| `fix/catalog.rs` | `DefinitionField::from_field` reads the marker under `Components`; `index_messages` keeps the ordered list `msgtypes`/`msgtype_at` read; `message_position` answers only a marked component; `check_shape`, `definition_category`, `merge_catalog`, `definition_tag_in_use` two categories | the code map, the alias map, the counter map, `(category, name)` keys |
| `fix/store.rs` | `LOAD_ORDER` three; the snapshot's three keys; `messages` refused by name | the shard arithmetic, compact references, the resolver, the crate-field rule |
| `fix/registry.rs` | `Hash` walks `[Components, Groups]`; `get_field_by_path` tries two roots | every field index; `FixFieldIter` order |
| `fix/crated.rs`, `fix/cfb.rs` | `register_msgtype` and the CBlock root land in `Components` | the code registration; the scope suffix rule |
| `scripts/generate_fix_dictionary.py` | messages rendered into `components/`, assigned their derived tags last as before, so every stated tag stays; `--check` walks the three folders and reports a `messages/` folder as unexpected | every document's bytes; `provenance.json`'s checksums (paths move) |
| `config/fix/messages/*` | moved to `config/fix/components/` | content |
| `scripts/build_docs_fix.js`, `docs/assets/fix.json` | `catalog` has three keys; `kpi.messages` counts components carrying `fix:msgtype` | every other KPI |
| `cli/` | no `messages` command tree; `--msgtype` on `components create` makes a message, which is required; `ingest` walks three | every other command |
| Python, Node | `"messages"` is refused as a category; every test re-spelled | argument order and error semantics |
| tests, benchmarks | `child_by_path=4` on write, `=3` on load; counts 928/580; every `Messages` re-spelled | the equivalence snapshot; the allocation pins |
| docs | eight pages re-spelled: three categories, a message is a marked component | |

**`stable_hash` moves.** The registry hash walked `[Messages, Components,
Groups]` and now walks `[Components, Groups]` with the messages among the
components in name order, so every registry holding a message hashes
differently. Said here rather than hidden: the committed dictionary's hash is
pinned as a literal in `rust/tests/fix/dictionary.rs`, and the pin is what a
later change to the walk has to move on purpose.

**Written in:** `fix_category.rs`, on the enum; `fix/catalog.rs`, on
`definition_category`; `docs/fix/registry.md`.
**Fixtures:** the census above as a count pin; a component given the marker
through `update_definition` answering `msgtype` afterwards and a message
whose marker is removed answering none; `msgtype_at` over the committed
dictionary agreeing with `msgtypes()`; a snapshot carrying a `messages` key
refused by name; the store writing a message under `components/` and reading
it back equal; the CLI creating a message under `components` with
`--msgtype`; the store call pin; the equivalence snapshot, unmoved.

## 14. A direction is a FIX fact, and the registry owns it

**Rule.** Which way a message moved is FIX's fact, published at tag 385
as `MsgDirection` with the code set `R = Receive`, `S = Send`, and nothing
else in the crate has one. `DataType::MsgDirection`, `DataTypeId::MsgDirection`,
`Code::MsgDirection`, the `types::MsgDirection` value type with its `SENT`/`RECV`
constants, `from_spelling`, `infer_bytes`, `infer_text`, `split_bytes`,
`split_text`, `at_payload` and the verb table, `StringEnum::DIRECTIONS`, the
`msgdirection` code name and its `DataType::CODES` and `LOGICAL_NAMES` rows,
the serde tag, `MsgDirectionType`/`MsgDirectionField`, the Arrow extension
`yggdryl.msgdirection` and the `yggdryl.direction` import, the text reader's
`direction` column, `TextOptions::parse_direction`, `TextLine::direction`/
`set_direction`, the marker stripping and the `direction`/`dir`/`way` column
aliases are deleted. Tag 385 is typed by the dictionary as it types every
coded field - `utf8` carrying its `fix:codes` - and `yggdryl::fix::MsgDirection`
is the registry's reading of it: the code set tag 385 declares, extendable
like any code set, beside the reading that names one code from the prose in
front of a payload (today's verb table and the document statement, moved
whole; decision 15 replaces them with rules the dictionary carries), held by
`FixRegistry`, answered by `registry.msgdirection()`, and compiled once where
a codec takes its registry (`FixCodec::new`). A registry without tag 385
answers the specification's set, `R` and `S`, so a direction is never
silently absent from a dictionary that did not ship the field.

A message carries its direction as tag 385, a built child holding the code
the reading matched, filled on every door - `parse_line`, `parse_text_line`,
`parse_fix_line`, `parse_ullink_line`, `parse_ulconfig_line`, the batch
reader - and the batch reader's precedence is: a stated 385 in the row, else
the reading over the row's prefix, else the codec's pin, which is a code of
the set. `FixCodec::direction` answers `Option<&str>`, the pin's code;
`FixCodec::try_with_direction(code)` takes any spelling tag 35's rules
already accept for a code - the value, the name, an alias - resolves it
against the set once and refuses one the set does not hold, naming the set;
`None` or the empty text is no pin. The default pin is `S`: a session's own
log is written by the side doing the sending. The line door fills only what
the reading matched and takes no pin, as today: a line that states nothing
states nothing, and the pin is the batch reader's answer for a column every
row must have. A bridge row does not inherit the direction of the frame the
same plugin logged before it: a row states its own facts (decision 8), so a
`RouteMessage` row with no verb answers the pin on the batch door and nothing
on the line door. Named here because the prompt left it to decide, and pinned.

`DataTypeId` retires discriminant 58 rather than reusing it: the enum states
every discriminant explicitly, `ALL` is sixty entries, `Url` stays 59 and
`Isin` 60, and the pin test states every byte rather than the index. The
rule that a variant's byte is its index in `ALL` ends with this decision, and
`as_u8`'s doc says so. `yggdryl.msgdirection` is no longer written; a stored
column carrying it or `yggdryl.direction` imports as the `fixed_size_binary(4)`
it is, which `docs/types/text.md` says under Edges.

**Why.** A direction was two vocabularies for one fact: the text reader's
`SENT`/`RECV` column, a packed datatype the crate invented, and FIX's own
tag 385 with `R`/`S`, which the shipped dictionary already typed under the
crate's datatype so the two disagreed on every value. Every capture reader
in the crate reads a FIX log, and the verb in front of a frame is the
transport saying which way the frame moved - which is what FIX's field
means. One owner, FIX's, with FIX's codes; the text reader keeps to what a
line is, and a datatype that existed for one column goes with the column.
The registry owns the reading because the code set is the registry's: a
dictionary that extends tag 385's set names the codes the reading can
answer, and decision 15 puts the rules beside them.

**What moves.**

| where | what changes | what must not move |
| --- | --- | --- |
| `datatype_id.rs`, `types/` | the variant, the value type, the code, the extension, the serde tag, the typed field, `DIRECTIONS`, `CODES`, `LOGICAL_NAMES` gone; explicit discriminants | every other discriminant; every other code |
| `media/text/` | `direction` column, `parse_direction`, `TextLine::direction`, the strip, the aliases gone | every other column; the body's bytes now keep the verb, which is prose before the payload |
| `fix/direction.rs` (new) | `MsgDirection`: the code set on 385 and the reading, compiled once | the verb table's answers, now spelled `S`/`R` |
| `fix/codec.rs`, `batch.rs`, `build.rs` | 385 filled as a child on every door; the pin a code; `DIRECTION_COLUMN` is the 385 column | the entries, the wire, every other row column |
| `config/fix/fields/3.json`, the generator | tag 385 typed `utf8`; `CODED_TAGS` loses 385 | every other document |
| Python, Node | `FixCodec(direction=)` a code; `MimeType.infer_*_direction`, `TextLine.direction`, `types.msgdirection`, `DataType("msgdirection")` gone | argument order and error semantics elsewhere |
| docs | `docs/types/codes.md` loses its section; `docs/fix/registry.md`, `arrow.md`, `capture.md`, `media/text.md`, `encode.md`, `types/text.md` re-spelled | |

**Written in:** `fix/direction.rs`, on `MsgDirection`; `datatype_id.rs`, on
`as_u8`; `docs/fix/registry.md`.
**Fixtures:** every case of `rust/tests/types/datatype/coded.rs`'s direction
test moved to `rust/tests/fix/direction.rs` with its answer spelled as a code;
the batch precedence with a stated `msgdirection` column, a verb, and the pin;
a pin refused by name; a registry without 385 answering `S`/`R`; `ALL.len()
== 60` with every byte stated; a stored `yggdryl.msgdirection` column
importing as its storage; the equivalence snapshot regenerated: every line
whose prefix carries a verb, and every document that states its half, gains
`.field.msgdirection`, and nothing else moves.

## 15. Direction rules are configuration the dictionary carries on tag 385

**Rule.** The rules that name a code of tag 385's set from the prose in
front of a payload are the registry's, stored on tag 385's field as
`fix:directions`: one canonical document
`{"directions":[{"code":"S","patterns":["..."]},...]}`, an entry per code in
the order the dictionary lists them, each pattern a `regex::bytes`
expression applied to the prefix - the bytes before the payload, exactly
the bound `payload_at` answers, so a verb inside a payload is still the
payload's word. A code matches where any of its patterns matches; exactly
one matching code names the direction; two or more answer nothing, as
today; none answers nothing. An entry's code is any spelling of a code of
the set - the value or the name, resolved through the one resolution a pin
goes through, `MsgDirection::code`, by the setter when it admits the rule
and by the reading once when it compiles it - so each code is named once
under any spelling, and `S` beside `Send` is one code named twice. A rule
naming no code of the set, a second rule naming a code already named, or a
pattern the regex crate refuses, is refused by `set_directions` and, where
a hand edit slipped it past the door, dropped by the reading with a
warning that is that refusal word for word: the setter is the door, and a
dictionary edited by hand degrades to fewer rules - down to none - rather
than to a wrong reading. Where the field carries no property the defaults
answer, keyed by the set's `Send` and `Receive` codes so an extended set
still reads `sending >>` as its own `Send`; a property the field carries
reads by what it states, whatever survives, and never by the defaults.
They are, per code, three patterns:

| code | pattern | reads |
| --- | --- | --- |
| Send | `(?i)(?:^\|[\s\[(<])(?:sending\|sent\|send\|outbound\|outgoing)(?:[\s\])>:,]\|$)` | the spelled verbs, opened by the start, whitespace or `[`, `(`, `<`, closed by the end, whitespace or `]`, `)`, `>`, `:`, `,` |
| Send | `(?i)(?:^\|[\[(])out(?:[\]):]\|$)` | the bare word, only bracketed: `[OUT]`, `(out)`, `OUT:` |
| Send | `(?i)(?:^\|\s)request:` | the half a Jolokia exchange states in its prose: a request went out |
| Receive | `(?i)(?:^\|[\s\[(<])(?:receiving\|received\|receive\|recv\|inbound\|incoming)(?:[\s\])>:,]\|$)` | the spelled verbs |
| Receive | `(?i)(?:^\|[\[(])in(?:[\]):]\|$)` | the bare word, only bracketed: `(in)`, `[IN]` |
| Receive | `(?i)(?:^\|\s)response:` | an answer came back |

The `Response:`/`Request:` rules replace the document statement decision 14
kept: a bridge configuration document no longer states its own half
through the echoed `request` key - the envelope is prose in front of the
payload, read by the same rules as every other prose, and decision 17 takes
the echoed key away with the envelope. `MsgDirection::stated`,
`line::payload`'s second answer and `line::ulconfig_answered` are deleted.
The property is an accessor pair like every `fix:` key - `directions()`,
`set_directions`, `remove_directions` on the field's FIX view, `FixDirection`
the owned rule, `FixDirectionEntry` the borrowed one, `FixDirections` and
`FixPatterns` the walks - merged like `fix:replacements` (incoming wins
whole: a rule table is one statement, and two tables have no order between
them), edited through `--directions` on `ygg fix fields create` and `update`, read and written
by the bindings as a list on the field's `fix` view. `MsgDirection::directions`
answers the rules in force - the property's, or the defaults - as data, so
what a dictionary reads by is never hidden in Rust. `FixCodec::new` compiles
every pattern once through `FixRegistry::msgdirection`, and a row applies the
compiled patterns to its prefix allocating nothing - the allocation pins on
the reading hold; nothing per row builds a regex. The committed
dictionary does not carry the property: the defaults have one owner, the
crate, and a dictionary that ships a table states its own.

**Why.** A hand table in the crate was one product's verbs written into
Rust; a bridge that logs `TX`/`RX`, or a venue whose capture prefixes its
lines with `>>>`/`<<<`, could only be read by editing the crate. The set is
the dictionary's already (decision 14), so the reading that names one of
its codes belongs beside it: a dictionary that extends the set with a third
code says how a line names it in the same document, and a registry merge
carries the table as it carries every other `fix:` property.

**What the rules cost.** Decision 14's table let a bare `in` or `out`
count against the opposite verb wherever a word boundary held, while
selecting only when bracketed - a three-valued marker. A rule matches or
does not, so the bare pattern keeps the bracketed shape and only that: a
whole-word `out` in English selects nothing, as today, and a bridge's
`(DEBUG) IN : ...` enrichment trace stays unread, as today. What goes is the
veto: `sending in session 3` answers `Send` and `received out of order`
answers `Receive`, where decision 14 answered nothing, because the verb in
front of the payload is the verb and the English `in`/`out` beside it is no
longer a marker that can contradict it. Those two pins move and say why.

**Written in:** `fix/direction.rs`, on `MsgDirection` and its defaults;
`fix/directions.rs`, on the document; `fix/field.rs`, on `fix:directions`;
`docs/fix/registry.md`.
**Fixtures:** every case of `rust/tests/fix/direction.rs` under the defaults,
the two moved cases spelled with their new answers; a rule added through
`set_directions` on tag 385 changing what a line answers, and the verb table
no longer applying under it; a dictionary without the property answering
the defaults, as data and as readings; a prefix matching two codes
answering nothing; a `Response:` line answering `R` and a `Request:` line
`S` with no document behind either, and a bare document answering nothing;
an entry naming the code by its name; a pattern the regex crate refuses,
refused by the setter and dropped by the reading; the merge rule; the CLI
flag; the bindings reading and writing the list; the equivalence snapshot
regenerated: `ulbridge[095]` and `ulbridge[117]` (`Request: JmxReadRequest[...]`)
gain `.field.msgdirection` `S`, `bridge[005]` (the bare configuration
document on the line door) loses its `R`, and nothing else moves.

## 16. A row yields none, one or many messages, and `unknown` names a frame

**Rule.** A row is read for every message it carries, and a row that carries
none yields none. Amending decision 3, which said a line carrying two frames
reads as the first: a frame still begins where it opens and ends at its
checksum, and the pairs behind that checksum begin the next message where
they open a frame of their own. What opens one is the rule `locate_frame`
already applies, read over the row's own entries so a mark is a mark: an
unmarked `8=`, and where the rest of the run states none, an unmarked `35=`.
A frame that states no checksum ends where the next unmarked `8=` opens -
only an `8=`, because a frame's own `35=` stands behind its `8=` and a second
`35=` inside an open frame is a duplicate tag, not a new message. The tail of
the last frame is the last frame's, as decision 3 said.

`unknown` names a frame, a bridge row or a document that states no type -
never a line that states no frame. A row that opens no frame, states no
bridge pair and carries no document yields nothing at all: no message, and on
the batch door no row. What makes a run of named pairs a bridge row rather
than prose carrying an `=` is decision 1's second amendment, already the
crate's vocabulary for it: the line named a separator for the run - a pipe, a
`SOH`, or one of the spellings a log escapes it with, never whitespace - or
the bridge marked one of its keys. A numeric frame needs neither: a run of
tag-keyed pairs is FIX whatever separated it, so a space-separated frame
still reads. So `heartbeat emitted seq=7` and `... ExecId=[00011377094XEEA0
OrderId=[00026877712XOEA0` state no message, where they used to state an
`unknown` carrying `seq` and `ExecID(17)`; `ACCOUNT=A1|SIDE=1` still states
one. A bridge row is one message and an opener-headed numeric frame behind it
is a second; a tag run behind a bridge row that opens no frame stays part of
it, which is the mixed form a bridge writes.

Every message of one row shares the row's version, clock, fills and direction
(decision 8), retained once per row exactly as a configuration document's are
(`RowStamp`, moved beside `RowExtras` in `fix/build.rs` as the owned twin it
is). `FixMessages` gains a lazy source over the page the row already holds,
re-entering the frame reader at each frame's offset: nothing is collected,
each message owns the ranges of its own frame, and an `Err` still fuses the
iterator. A row of one frame keeps the eager path, so the allocation pins on
a single-frame line do not move. The entry-less `unknown` a line used to
build survives for one case and one only: a payload that was there and would
not parse - a malformed document - which a batch must not fail on. A row that
carried nothing to read is a different fact and answers nothing.

The single-dialect byte doors - `parse_fix_line`, `parse_ullink_line`,
`parse_fixml_line`, `parse_pairs` - keep answering one message and refuse a
body that holds a second: `invalid fix expression at byte N: expected one
frame, got a second`, positioned at the byte the second frame opens at (the
second root element for FIXML, the second `8=` behind a checksum for pairs).
They are the doors a caller uses when it holds one frame, and a caller
holding two has a row, not a frame.

The batch reader expands a row into its messages as it already expands
MBeans - the carried columns repeated, the row's byte charge on the first -
and a row yielding nothing yields no output row, so a capture of ten million
lines answers one row per *message*, not one per line; the text reader is
what answers one row per line. `write_arrow_reader` writes one line per
message, so a source line holding two frames comes back as two. The
classifier's `mimetype` and `msgtype` columns describe the line's first
frame, which is what a classifier can know without parsing.

**Why.** Two frames on one line is a shape captures really hold - a relay
that batched two messages into one log write - and reading only the first
silently drops the second: a count that says 113 rows for a capture holding
114 messages is wrong in the direction nobody checks. The opposite error was
the same mistake mirrored: a sentence with no frame in it became a message
named `unknown` carrying whatever `=` the prose happened to hold, so a
capture's message count was the line count by construction and `ExecId=` in
an English sentence became `ExecID(17)`. A reader that answers "this line
carried no message" is the one that can be counted on, and the line is still
the line - the text reader answers every one of them.

**What moves.**

| where | what changes | what must not move |
| --- | --- | --- |
| `fix/messages.rs` | the lazy frames source; `none()` | the fuse, the ULconfig source, no collection |
| `fix/codec.rs` | `frame_end`/`next_frame` beside `bounded`; the page dispatch; the four doors' refusals; `empty_with` gone | the frame's own bounds, the mixed form, a prefix's pairs staying prose |
| `fix/build.rs` | `RowStamp` moved beside `RowExtras` | what a stamp holds |
| `mime_type/line.rs` | `names_separator`, the one reading of decision 1's amendment a caller outside the scanner needs | `locate_frame`, `inspect`, the classifier's first-frame answer |
| the corpus | every prose line's `.messages` 1 -> 0 | every framed line |

**Written in:** `fix/codec.rs`, on `parse_line`; `fix/messages.rs`;
`docs/fix/decode.md`, `docs/fix/arrow.md`, `docs/media/text.md`.
**Fixtures:** two frames on one line, both bounded, each re-emitting its own
bytes; a bridge row then a frame; a checksum-less frame then a frame; a
marked `#8=`/`#10=` inside a bridge row opening and closing nothing; a prose
line answering nothing through every door; the four doors refusing a second
frame by byte; the corpus lines whose `.messages` moves from 1 to 0, named in
the regeneration commit; `one_line` renamed to what it asserts and kept.

## 17. The envelope is transport, and a body FIX cannot read is silence

**Rule.** A configuration message is the plugin's attributes and nothing the
Jolokia answer wrapped them in. `MBean`, `Operation`, `Status` and `Error` -
tags 20001 to 20004 - are deleted with the four constants naming them; the
ObjectName a read answered for stays where it always belonged, on the
`SessionInterface` attribute (20010). The four tags are retired rather than
reused, exactly as decision 14 retired a discriminant: a capture written last
year holds `MBean` on 20001, and a dictionary that gave 20001 to something
else would read that column as the new field rather than as the old one.
`ULBRIDGE_TAG_MIN` therefore stays 20001 - it is the floor of the range this
dictionary claims, not the smallest tag it happens to define.

A read that answers no plugin answers no message: an error-only answer, a
wildcard selecting nothing. Decision 16 gave a row that ability, and this is
the same fact one layer down - a document that named no configuration states
none, and the row is silent rather than carrying an envelope with nothing
inside it.

A JSON body that is not a Jolokia answer - a `request` beside a `value` whose
ObjectNames name the ULBridge namespace - yields nothing, through every door,
and refuses nothing: `{"a":1}` is a row's own bytes and the row said nothing
FIX can read. That is what the codec is for, and being unable to read a body
is not an error in it, so `parse_ulconfig_line` answers `FixMessages` rather
than a `Result` of one: the door has no refusal left to spell, and a
conversion's own is still an item of the iterator. Bytes that are not JSON at
all are the same silence, because a body this reader cannot read is a body
that named no configuration however it failed to be one.

`MimeType::ULCONFIG` is deleted with the media type `text/ulconfig`. A JSON
body is `application/json`, which is what it is; what makes one *this*
reader's is a shape, and a shape is the codec's to recognize rather than a
classifier's to name. The codec probes it once, at the offset the namespace
scan already finds, so the reading costs what the classification cost and
nothing is looked for twice. `UlPlugin` and `UlPlugins` stay the public
reading of a document, minus the envelope, until decision 18 renames them.

**Why.** `MBean` and `Operation` are what the transport asked, `Status` and
`Error` how the asking went; none of them is a fact about the plugin the
answer carried, and a message that states them makes a column out of the
question rather than the answer. The ObjectName is different - it names the
configuration itself - and it was already the `SessionInterface` attribute,
so the envelope was restating it. The media type went the same way: a
classifier that answers `text/ulconfig` has parsed the body far enough to
know it is a Jolokia answer, which is a reading, and the crate already holds
that reading in one place. `application/json` is what a stranger to this
bridge sees, and it is true.

**Written in:** `fix/ulbridge.rs`, on `UlPlugin`; `fix/codec.rs`, where a
document reaches a door; `mime_type.rs`; `docs/fix/capture.md`.
**Fixtures:** the corpus exchange - its four prose lines yielding nothing
(decision 16 already), the single read one message carrying no 20001-20004,
the wildcard read two, the error read none; a bare `{"a":1}` yielding nothing
through `parse_line`, `parse_text_line`, `parse_ulconfig_line` and the batch
door, and refusing nothing; a JSON body classifying as `application/json`;
the byte-for-byte re-emission of a configuration message unchanged, because
the entries are what arrived and the envelope was never one of them.

## 18. A plugin is FIX's, and ULBridge is one producer of one

**Rule.** A plugin is a FIX session endpoint as the bridge hosting it reports
it: comp ids, begin string, hosts, ports, state, sequence numbers. None of
that is ULBridge's - it is what a FIX session is, seen from whatever hosts it
- and the reading of it is FIX's, generic over the bridge. ULBridge's Jolokia
answer is one producer of such a report, and the crate is named for the thing
reported rather than for the one product that reports it today.

So every name spelling `ulconfig`, `UlPlugin` or `ULCONFIG` is spelled
`plugin` or `Plugin`: `yggdryl::fix::Plugin` and `Plugins`,
`FixCodec::parse_plugin_line`, `plugin_with`, `FixMessages::from_plugins` and
`Source::Plugins`, `plugin_at`, `plugin_span`, `plugin_msgtype`, the bench
group `fix/plugin`, the allocation probes, `sole_plugin_line`, the bindings'
`fix.Plugin`/`Plugins` and `parse_plugin_line`/`parsePluginLine`, and the
module file: `fix/plugin.rs` holds the reading. The attribute fields become
dialect `plugin` - `PLUGIN_DIALECT`, `PLUGIN_TAG_MIN`, `fix_plugin_fields`,
`with_plugin_fields` - and their tags 20010 to 20047 do not move, because a
tag is an identity and a rename is not a renumbering.

Two names stay ULBridge's, because what they read really is the product's.
`ULBRIDGE_ROWHEADER` reads ULBridge's own log line, whose shape no other
bridge writes. The namespace `com.ullink.ulbridge` is a vendor string in a
document, not a name of ours to choose.

A rename is the whole commit and nothing else travels with it: every door
answers byte for byte what it answered, the equivalence snapshot does not
move, every pinned cost stays where it was, and `git grep -i ulconfig`
answers nothing outside this file. The one value that does move is the
`stable_hash` of a registry carrying the attribute fields, because the
dialect text is hashed: `ulbridge` and `plugin` are different bytes, so the
digest is different and the pin is restated rather than explained away.

**Why.** A name that says who wrote a thing rather than what it is puts the
producer in the type system, and the next producer then arrives as either a
second reading or a lie. Everything the reading does with a plugin - typing
its attributes against a dictionary, crossing to a `FixMsg` and back, filling
comp ids from it (decision 19) - is true of a plugin however it was reported,
and none of it consults ULBridge. Keeping the product's name on it would make
decision 19's `pluginconfig` message a ULBridge message type, which it is
not.

The dialect is the same argument one level down: `fix:branches = ulbridge`
says these fields belong to a product, when what they belong to is the
reading of a plugin. A second bridge reporting the same endpoint would have
to choose between a membership naming someone else's product and a second
membership for the same fields.

**Written in:** `fix/plugin.rs`, the module the reading moves to;
`fix/codec.rs` and `fix/messages.rs`, where the doors and the source are
spelled; `mime_type/line.rs`, where the document is located;
`docs/fix/capture.md` and `docs/fix/registry.md`.
**Fixtures:** every existing pin, renamed and otherwise unmoved - the
snapshot byte for byte, the allocation counts, the corpus row counts, the
doors' answers; `dialects()` answering `["plugin"]`; the registry
`stable_hash` restated to what the new dialect text digests to; and
`git grep -i ulconfig` answering nothing but this file. The three pins
decision 17 left on the retired media type - that `text/ulconfig` parses as a
stranger and that no binding carries a `ULCONFIG` constant - go with the
spelling: a crate that never had the name has nothing to say about it, an
unknown name is already pinned on one that was never ours, and the vocabulary
count guards the constant list.

## 19. A plugin configuration is a message, and the enricher remembers one

**Rule.** A plugin configuration is a message type of the crate's own:
`pluginconfig`, a component carrying `fix:msgtype` with the user-defined code
`UCFG`. FIX reserves `U` for user-defined messages, and `UCFG` is this
crate's, chosen here so that no dictionary has to be edited for a
configuration to read as what it is. Its members are the plugin attributes
(20010 to 20047) beside FIX's `BeginString` (8), `SenderCompID` (49) and
`TargetCompID` (56), which is the whole of what a configuration states.

A configuration message carries `35=UCFG` as a built child rather than as a
pair: the arrival record is what the document stated, and the exchange never
sent a `35=`. So the wire re-emits byte for byte as it did (decision 17), and
the type is a fact the crate adds rather than one the document made. Its
`.type` is `pluginconfig`, the name, where a message typed off the wire keeps
whatever the wire spelled - `D`, `8`, `ExecutionReport`. The two are
different facts: a spelling a row wrote is the row's word and is kept
verbatim; a code the crate supplies is the crate's, and the crate knows the
name it registered it under.

The component is registered twice over, because two callers need it and
neither can wait for the other. `with_plugin_fields` registers it beside the
attribute fields, where it belongs. `FixRegistry::new` registers it too, so
every registry has it exactly as every registry has `pluginid`: a codec
meeting a configuration cannot write a shared registry, so the message type
has to be there before the first document arrives. Registering the component
does not register its members as dictionary fields - a component is a
definition and holds its members by value - so a registry that never called
`with_plugin_fields` still types a `UCFG` message from the component's own
members, and still answers no `plugin` dialect and no `CurrentPort` by name.
Those are two different questions and they keep two different answers.

**The enricher remembers.** `enrich_messages` is stateful: it holds the last
`pluginconfig` it passed for each plugin, keyed by the folded `Name` (20013 -
not 20012, which is `PluginType`). For every later message whose `PluginId`
capture folds equal to a remembered name it fills `SenderCompID` (49) and
`TargetCompID` (56) from that configuration, and only where the message
states none of its own: a fill never lands over a value the message already
stated, which is what makes the pass idempotent and is what `enrich` has
always done. A configuration seen later replaces the one remembered for its
name. A message with no `PluginId`, and one whose `PluginId` no configuration
has named, are untouched.

`BeginString` (8) is not among them, and cannot be. Every built message fills
it non-null from the version the row was read at - it is one of the five
columns a message always has - so a configuration's begin string would never
find a message stating none, and a rule that can never fire has no business
being written down. What a plugin's configuration says about its version is
already what the row says about it.

The memory is the iterator's and dies with it. `enrich_message`, the door
that takes one message, stays stateless: one message is not a stream and has
nothing to remember from. `enrich_messages_arrow_reader` carries one memory
across every batch it reads, because a capture split into batches is one
capture.

**Why.** A configuration that reads as `unknown` is a row the dictionary has
no opinion about, and everything downstream then has to special-case it by
shape rather than by type. Registering it makes it addressable the way every
other message is: `35=` answers, the schema has a column, a lifecycle can
skip it by type rather than by guessing.

The fill is the other half. A bridge log states a plugin's comp ids once, in
the configuration it printed at startup, and then writes ten million lines
that name only the plugin. Those lines are about a session whose two ends are
known - the reader just read them - and leaving them null makes every
consumer join back to a configuration it would have to have kept. The
never-overwrite rule keeps this honest: a line that stated its own comp ids
keeps them, so the fill can only ever add what the capture already implied.

**Written in:** `fix/plugin.rs`, where the component is built and the code
lives; `fix/registry.rs`, where every registry gains it; `fix/enrich.rs` and
`fix/codec.rs`, where the pass becomes stateful; `fix/batch.rs`, where the
memory crosses batches; `docs/fix/capture.md`.
**Fixtures:** a configuration message typed `pluginconfig` with `35=UCFG`
answering, and its wire re-emitting byte for byte without a `35=`; the same
document read by a registry that never called `with_plugin_fields`, typing
its attributes from the component's members while still answering no `plugin`
dialect; the corpus, where the `SmartTrade_OrderRouting` frame keeps the
49/56 it stated and a row naming a plugin no configuration named gains
nothing; a configuration then a bare row on one plugin, the row filled; two
configurations for one plugin, the later winning; a stated 49 never
overwritten; and the memory crossing a batch boundary in
`enrich_messages_arrow_reader`.

## 20. Enrichment is one pass, and it fills everything one message implies

**Rule.** There is one enriching pass and it is `FixCodec::enrich_message`,
`enrich_messages` and `enrich_messages_arrow_reader`. Three steps run on one
message, in this order.

**Restate.** What `latest::restate` decides today, unchanged: each child
canonicalized to the registry field its tag, name, alias or decimal spelling
reaches; children reaching one field merged into the most complete;
`fix:replacements` applied in ascending tag order; the crate `version`
stamped with the registry's newest. `FixMsg::into_latest` is deleted and
`latest.rs` becomes the pass's first step, because restatement was never a
stage a caller should be able to skip: every rule below reads by tag, and a
child stored under an alias with no tag is invisible until it has been
canonicalized. A pass whose first step is optional is two passes.

**Complete - and completion is a read.** A consumer addressing `lastshares`
finds the `lastqty` the message states, because `FixMsg::get_by_name`
resolves the spelling through the registry, which answers a field for any
alias it holds. That is already true and nothing needs writing to make it so.

No alias twin is added as a second child, and the reason is not economy.
Two of the forty-two shipped aliases are another field's canonical name -
`quoteackstatus` is an alias of `quotestatus` (297) and the name of 1865,
`tradetype` an alias of `bidtradetype` (418) and the name of 3006 - so a
message stating both would hold two children of one name and
`DataType::from_fields` would refuse the whole pass. A twin cannot cross the
batch door: `fix_schema` names a column for a canonical field and for no
alias, so `enrich_messages_arrow_reader` would discard on the way out what
`enrich_messages` had just added, and the two doors would stop being one
pass. And `get_by_tag` answers the earliest child on a tag, so where a twin
was placed would silently decide what every column, every rule, every lift
and the lifecycle read. A spelling is a way of asking, not a thing to store.

**Fill.** What the message implies and does not state, never over anything it
states - the rule that makes the pass idempotent. Three sources, in order,
because each may feed the next: the composed keys the row carried, then the
rules the specification licenses, then the plugin configuration this stream
has already passed (decision 19).

**Composed keys.** A bridge writes a field under its own namespace -
`TECH.CLIENTID`, `ULLINK.INSTRUMENTID`, `FIRM.ORIG.ULFROMSESSIONNAME`,
`OMSVENDOR.CALC.EXECBROKER`. Where the last dotted segment of a child's name
resolves to a dictionary field and that field is absent, the composed key
fills it, and the filled child takes the field's tag so every rule below can
read it.

One voice or silence. Where a row names one absent field under several
composed keys and they do not agree, none of them fills it. This is not a
precaution: on nine lines of the committed corpus `FIRM.ORIG.CLIENTID` is
`3000090.006` and `ULLINK.CLIENTID` is `trader1` with `CLIENTID` absent -
a firm account number and a trader login, and nothing in the row says which
one the field means. Filling from either would invent a fact; filling from
neither states what the row actually settled, which is nothing.

Never overwriting is load-bearing here too, and the same corpus proves it:
`CLIENT.SYMBOL` is `XAU` where `SYMBOL` is `XAU/USD`, and
`OMSVENDOR.CALC.EXECBROKER` is `SWXCCP` where `EXECBROKER` is
`2003103.001`. A namespace's spelling of a fact is not the fact.

**The rules the specification licenses.** Beside what the table already
derives, and each only where every input is stated and the output is not:
`LastPx` from `LastSpotRate` plus `LastForwardPoints`, and `BidPx` and
`OfferPx` from their own spot and forward points, because a forward price is
quoted that way and the points are already in price units; `PeggedPrice`
from `PeggedRefPrice` plus the signed `PegOffsetValue`; `LastMultipliedQty`
from `LastQty` times `ContractMultiplier`, `TotalTradeMultipliedQty` from
`TotalTradeQty` the same way, and `MinPriceIncrementAmount` from
`MinPriceIncrement` times `ContractMultiplier`; `TotalGrossTradeAmt` from
`LastPx` times `TotalTradeQty`; `OrderQty` from `CumQty` plus `CxlQty`,
which is what a canceled order's two halves add to;
`OrigSendingTime` from `SendingTime` on a possible duplicate, which is what
the session layer says one is; and `CurrencyCodeSource` as ISO 4217 wherever
a `Currency` is stated, because that is the only source FIX's own
`Currency` field is written in.

What is deliberately not filled: any amount whose scale depends on a
convention the message does not state. `GrossTradeAmt` from a percent-of-par
price needs a division by one hundred that the specification writes in price
units and leaves to the reader; an FX gross amount is a product or a
quotient depending on `SettlPriceFxRateCalc`, and absent that tag the
quoting convention decides; `NetMoney` needs `Commission` resolved through
`CommType` and every `MiscFeeAmt` through `MiscFeeBasis`. A capture reader
that guesses a notional is worse than one that leaves it null.

Nor anything that needs a second message. `OrigClOrdID` from the request a
report answers, `ListID` from the list an order belonged to, a bust's effect
on `CumQty` - each is a fact about a chain rather than about a message, and
chains are the lifecycle's (decisions 21 to 24).

**Why.** Three passes a caller composes is three chances to compose them
wrongly, and the order was never free: restatement has to precede filling
because filling reads by tag, and the plugin memory has to follow the rules
because a rule may state what the memory would otherwise fill. Making the
order a fact of the pass rather than a convention in the documentation is
the whole change. The docs' three composable stages become two: enrich, then
lifecycle.

**What one pass made visible.** Composing the two steps on both doors
exposed a divergence that was always there and that nothing asserted,
because nobody had composed them: the fixed schema carries no column for
`ExecBroker` (76), `ClientID` (109), or five other tags the shipped
dictionary's `fix:replacements` read - 92, 166, 370, 439, 440. Those rules
synthesize a root `parties` group and its `NoPartyIDs` counter, and those
two do have columns. So the line door restated a parsed message and
answered two parties, while the batch door restated a message rebuilt from
a row whose 76 and 109 were dropped on the way in, and answered none. Four
corpus lines showed it - 111, 122, 123 and 125 - and it reproduces at the
commit before this one, through `into_latest`.

A row is a projection, and a field with no column cannot carry what
restatement would have derived from it. But a row carries more than its
columns: it carries the arrival record whole, and the record is what the
document stated. So the pass opens on the record - every leaf pair it names
that the message holds no child for is taken back before restatement runs -
and three of the four lines close, because their 76 and 109 arrived as pairs
and a pair is an entry. Only a leaf: a pair that headed a subtree is that
subtree's, and recovering a group's counter without its occurrences would
state a count of nothing, which is a worse answer than the silence the
projection left. Restating before the projection instead, in the parse
door, was tried and is wrong: restatement rewrites values as well as deriving
them - `ExecType(150)` `2` becomes `F` - so a parse door that restated would
answer something the line parse door does not, and the two parse doors
agreeing is the older promise. It would also put enrichment outside the
enriching pass, which is the thing this decision exists to stop.

Line 111 stays, and it is the honest bound: its body arrived as a FIXML
document behind `XmlData(213)`, and the reader lifts the document's fields
into the message without recording them as entries - the record of a
document-bodied message is its envelope's ten pairs and the document whole.
So 76 and 109 are in no column and in no entry, and nothing can rebuild them
from the row. Closing that means recording what a document stated as entries
of its own, which moves every `.entry[i]` the equivalence snapshot pins and
is its own decision rather than a corner of this one. It is written down as
a list of two pairs in `dataset.rs` and asserted to be exactly the set that
diverges, so the day a row carries one of them the list stops being true and
says so. Widening the schema is the other repair not taken: it would change
three public arrays whose length is in their type and would still leave the
next replacement rule outside the list.

**Written in:** `fix/enrich.rs`, which gains the recovery off the arrival
record and then the restatement as its first two steps, and the composed
keys as the first of its fills; `fix/latest.rs`, which
stops being a door; `fix/msg.rs`, where `into_latest` is deleted;
`fix/codec.rs` and `fix/batch.rs`, the three doors; both bindings;
`docs/fix/message.md`, `docs/fix/capture.md`, `docs/fix/arrow.md`.
**Fixtures:** every case of `rust/tests/fix/latest.rs` through
`enrich_message`; the `enrich` group of the snapshot regenerated with its
moved lines named; `lastshares` reading the `lastqty` a message states, and
no second child under either name; `version` stamped once and a second pass
equal to the first; the corpus's `TECH.ACCOUNT` filling an absent `ACCOUNT`
and its `CLIENTID` staying absent under two disagreeing namespaces;
`CLIENT.SYMBOL` never landing over a stated `SYMBOL`; one fixture per
new rule, with the excluded conventions stated as the reason a neighbouring
amount stays null; and the corpus read both ways, row by row and in batches,
required to agree on every tag but the two pairs written down as what a
document-bodied row cannot carry.

## 21. A component declares its identifiers; a Map group carries their values

**Rule.** `fix:identifiers` is an ordered list of the component's own scalar
member names. Its setter accepts a member's name, alias or decimal tag,
resolves it against that component once, and stores canonical names in
component order. An absent member, a nested member, an empty spelling, an
embedded comma, or two spellings resolving to one member is a located typed
refusal; failure changes nothing. Empty input removes the property.
`identifiers()` borrows the existing `FixSpellings` iterator. On merge the
incoming list wins whole, as a replacements or directions document does:
two ordered declarations are not an unordered union.
The same owner normalizes stored metadata after references resolve, so the
final member order owns the stored order on create, reload and merge.

**The families.** The generator owns one explicit suffix table, beside
`CODED_TAGS`: `clordid`, `origclordid`, `secondaryclordid`, `orderid`,
`secondaryorderid`, `listid`, `quoteid`, `quotereqid`, `quoterespid`,
`quoteentryid`, `execid`, `execrefid`, `secondaryexecid`, `tradeid`,
`secondarytradeid`, `tradereportid`, `tradereportrefid`, `firmtradeid`,
`regulatorytradeid`, `allocid`, `secondaryallocid`, `individualallocid`.
Matching the folded name's suffix includes the published side, leg, ref,
orig and affected forms; a general `id` or `reportid` suffix is not a family.
The FIX Latest field reference supplies the meanings, including
[ClOrdID](https://fiximate.fixtrading.org/en/FIX.Latest/tag11.html),
[ExecRefID](https://fiximate.fixtrading.org/en/FIX.Latest/tag19.html),
[RegulatoryTradeID](https://fiximate.fixtrading.org/en/FIX.Latest/tag1903.html)
and [IndividualAllocID](https://fiximate.fixtrading.org/en/FIX.Latest/tag467.html).
Every generated component, including a group's occurrence and a marked
message, declares its direct scalar identifiers in member order. No group
is traversed to invent a root identifier. Empty membership means no property.
The generated dictionary census and five literal message lists pin this rule.

**The group.** `altids` (`AltIds`, 65020) is a nullable
`map<utf8, utf8>` with sorted keys, registered in `Groups`, never `Fields`.
A Map's occurrence is its existing non-null entries Struct. Crate-owned
Map groups address their mapping through their own counter: `fix:tag` and
`fix:counter` must agree, lie in the crate's reserved range, and collide
with no scalar field; scalar alternate tags cannot claim that counter either.
Their cardinality is already the mapping's length,
so no second scalar counter or count state is introduced. Persisted
declarations omit builtin groups, as they omit builtin scalars; a component's
reference to one resolves against the registry-owned builtin on reload.
The crate-tag listing has
twenty-one definitions: twenty scalar fields and this group. Tags 65021 and
65022 remain available for decision 24. Ordinary List/LargeList groups keep
their derived definition tag and separate Int32 wire counter; a nested
datatype is still refused in `Fields` without exception.

One occurrence accessor serves List, LargeList and Map. A reconstructed
group preserves its layout and a Map's sortedness; its entries and key never
become nullable. A persisted Map retains the entries Struct required by its
datatype; a referenced entries component carries only its reference metadata
and resolves through the same occurrence owner as a List item. Component
updates refresh it, metadata overrides fail, and an update that cannot retain
the two-member non-null-key Map shape fails atomically. Ordinary stored
references still require the Null placeholder. `GROUP_TAGS` still selects
the three dictionary groups:
the crate-tag enumeration already selects 65020. Schema construction asks
for a scalar and a group independently, so the absence of a scalar cannot
hide a group. The native Mapping and Arrow Map doors carry `altids`; its
key and value gain no invented numeric FIX tags or numeric delimiter.
The native name door prefers a canonical Map name over a scalar alias;
identical folded canonical scalar/Map names are a conflict in either
registration order, because the message door cannot distinguish them.
Builtin construction and registration failures are reported rather than
silently swallowed, and a census proves every builtin is registered in its
own category. Persisted references resolve builtin definitions from their
registry owner when the store omits them; an incoming document cannot
override a builtin by restating its name under another tag.

The generated explorer catalog enumerates the live native categories, not
only the compact persistence document: persistence deliberately omits
builtins. It keeps each native compact document where one exists and uses
the native Field document for an implicit definition. This preserves stored
references without rebuilding a schema and includes the scalar builtins,
`altids`, and `pluginconfig` under their actual categories. Displayed counts
come from those same collections; shipped and live inventories stay distinct.

**The fill.** A registered message definition compiles its identifier
selection once; per-message work reads the selected fields and moves their
values without parsing metadata, resolving input spellings or rebuilding a schema.
An exact canonical member name wins. A renamed member is reached by its tag
only where that tag is unique in both the component and the row; an ambiguous
tag selects nothing rather than assigning one member another's value.
One core implementation answers that selection for enrichment and the
lifecycle that follows. After restatement and the existing fills, enrichment
builds `altids` from identifiers stated at this message's own level, keyed by
canonical field name and retaining each identifier's text. Null identifiers
contribute nothing. A known component with no stated identifiers yields an
empty map. Non-text scalar identifiers use the existing UTF-8 scalar
conversion's canonical spelling, never a second identifier renderer. An
unrepresentable spelling propagates that conversion's typed refusal at the
identifier's field path; it is not silently omitted. An
unknown message type yields no map. A stated non-null map,
including an empty one, is preserved; a second enrichment is equal.

The producer sorts keys and makes them unique before the map crosses
`Field::scalar`. This is a producer invariant, not an assumption that
`schema::fitted` checks it: that function accepts an already matching
datatype ID. `column_value` gains no derivation for 65020; enrichment owns
the fill and the group column simply carries it. Arrival entries, wire bytes
and the arrival digest do not change.

**Python exchange.** A sorted Map must retain its flags through batch and
stream export, not only a standalone Field. Arrow 59.2's aggregate Field
C Schema exporter overwrites the Map sortedness flag. The Python boundary
therefore reuses the core's existing recursive Field exporter and the
existing C Data array importer for every batch. A private, thread-safe
iterator owns the native reader, resolves its root and transport metadata
once, pulls no batch at construction, and exports one batch per request.
Exhaustion or an error fuses and releases that reader. Native failures retain
the C Stream boundary's exception categories: invalid input is `ArrowInvalid`
(also a `ValueError`), with distinct I/O, memory and not-implemented errors.
Foreign array-import errors retain their own exception, never a blanket
`ArrowException`. Arrays share their buffers, including
nested Maps; batch metadata and zero-column row counts are preserved.
Standalone batches use the same exporter. There is no FIX-specific bridge,
foreign cast, alternate schema builder, or unsafe callback. Pins cover
sortedness, nested flags, transport metadata, row counts, shared buffers,
lazy pulls, failure fusion, and consumption from a Python worker thread.
The import direction already preserves the flags. Its redundant second
datatype import and per-field schema reconstruction are removed: a foreign
Field or Schema is imported once, then resolved by the native owner.

**Pins.** Atomic property refusal and canonicalization; merge precedence;
the full generated membership census and literal lists for `newordersingle`,
`executionreport`, `tradecapturereport`, `quote`, `allocationinstruction`;
Map category, counter and key invariants; unchanged List counter refusals;
exactly one nullable sorted `altids` column; map-preserving group merge;
scalar-member and entries-component reference refresh and store round trips;
native row and Arrow round trips; preserved stated maps, empty and unknown
cases, no flattened group identifiers, second-pass equality; allocation-free
borrowed declaration/compiled selection at several corpus sizes. The
equivalence regeneration changes only enriched projections and appended
identifier fixtures, including `enrich.ulbridge[...]` for the capture's
execution reports, with every changed key named in this decision's commit.

**Validation mode.** The required all-target test command must not run
benchmark measurements. Two custom median gates bypassed Criterion's test
mode: local-filesystem parity, and cast-plan parity in optimized tests.
Their shared mode predicate permits custom timing only in an optimized
invocation explicitly carrying `--bench`, without `--test` or `--list`.
Profiling-only and rejected-all invocations also perform no custom timing.
Test mode retains transfer-size/content and cast-result assertions without
warm-up, median samples or throughput thresholds. The benchmark thresholds
and full benchmark fixtures remain unchanged. Mode combinations are pinned;
this is a validation-harness correction, not a storage-performance change.

## 22. Lifecycle identities are UUID values, packed by the UUID owner

**Rule.** The lifecycle columns are `instuuid` (65016, `InstUuid`), `uuid`
(65017, `Uuid`) and `puuid` (65018, `PUuid`), all `DataType::Uuid`.
Their constants are `INSTUUID_TAG_NAME`, `UUID_TAG_NAME`, `PUUID_TAG_NAME`.
The replaced names and constants are removed, not retained as aliases.
The tags, twenty scalar definitions, and twenty-one total crate definitions
do not move. An older dataset's explicitly binary fields remain binary;
their old names do not acquire a conversion or a new registry meaning.

`Uuid::from_v7(unix_micros, payload)` and `Uuid::from_v8(payload)` own the
packing in `types/uuid/scalars.rs`. They read no clock, select no hash, and
allocate nothing on success. The existing packed value, parser, serde and
Arrow extension remain their owners. The layout follows
[RFC 9562 sections 5.7 and 5.8](https://www.rfc-editor.org/rfc/rfc9562.html#section-5.7),
using [section 6.2's fractional-clock method](https://www.rfc-editor.org/rfc/rfc9562.html#section-6.2).
For a nonnegative microsecond instant `m`, version 7 contains `m / 1000` in
its first 48 bits, `floor((m % 1000) * 4096 / 1000)` in its 12 fraction bits,
version 7, variant `10`, and the payload's low 62 bits. The accepted inclusive
range is `0..=281474976710655999` microseconds. Outside it the constructor
returns `InvalidRecord`, naming the expected range and actual instant at `$`;
the lifecycle locates that refusal at the column it was about to stamp.
There is no wrap, clamping, wall-clock substitution, or timestamp decoder.
Version 8 replaces only the four version bits and two variant bits of the
supplied 128-bit payload; the other 122 bits are unchanged.

**Inputs.** This piece changes representation, not the original lifecycle
recipes. The impact clock remains tag 60, then 52, then the message's market
timestamp, then the epoch, expressed in microseconds. `uuid` uses that clock
and xxh3 of the arrival digest's big-endian bytes. A new `puuid` uses that
clock and xxh3 of the unmodified instrument digest, one `0x1f` byte, and the
first chain identifier's bytes. `instuuid` is version 8 over the existing
xxh128 instrument digest. Its version/variant masking must not change the
raw instrument bytes fed into `puuid`. Streaming the same hash input removes
temporary concatenation buffers without changing that input or algorithm.
These deterministic hashes are not cryptographic or a claim that collisions
cannot occur. The exact instant remains the timestamp's fact; the UUID is
ordered by that clock, not a replacement timestamp accessor.

**Atomicity.** Resolve the chain to join and all fallible stamps before
publishing any chain or identifier-index mutation. Apply the message's own
atomic `set_many` once; only its success attaches keys, opens a chain, or
closes a terminal one. A failed new-chain stamp leaves no chain; a failed
join introduces no alias and cannot close an existing chain. Stated UUIDs
remain untouched and the original second-pass behavior remains until
decision 23 changes chain indexing. No arrival entry or emitted wire changes.

**Pins.** Exact version-7 fractional vectors, minimum and maximum instants,
negative/overflow refusals, and ordering through every microsecond and a
millisecond rollover even under opposing payloads; version-8 bit masking and
the RFC illustrative vector; native UUID value and canonical text/storage
round trips; zero-allocation construction. Every lifecycle identity uses the
new names and asserts version/variant bits, keeps byte ordering and replay,
and tests clock precedence and atomic failure. The row and record-writer
doors retain the UUID Arrow extension. Registry hashes change deliberately
with the renamed typed definitions; the equivalence snapshot stays unchanged.

Per the user's revised sequence, the remaining story lands in Rust before
Python, Node, and the final lightweight documentation pass. The later
message-identity decision will remove `msghash` entirely at the user's latest
direction; this representation piece does not pre-empt that later change.

## 23. A lifecycle chain is keyed by its UUID; identifiers belong to an instrument

**Rule.** `FixLifecycle` keeps `HashMap<Uuid, Chain>` and an identifier index
`HashMap<(Option<Uuid>, SmolStr), Uuid>`. A chain owns every scoped key attached
to it, and no copy of the UUID already naming it in the map. Held facts are
bounded by live chains and their distinct identifiers; reusable map capacity
follows the peak live set, not the number of completed events. Closing or
clearing removes both owners' corresponding entries without promising a
capacity shrink. There are no vector holes, free list, or unscoped index.

The effective instrument is a non-null stated `instuuid`, else the UUID the
existing instrument recipe derives, else absent. Absence is its own scope.
Stated `altids`, including an empty map, is authoritative. Only absent/null
`altids` uses the registered message's compiled identifier selection. Unknown
message types supply no fallback identifiers; no hard-tag list is retained.
`MsgType::identifier_mapping` owns conversion to canonical UTF-8 and sorted
member names, reusing `identifier_values`; enrichment and lifecycle call it.
Non-null map/UUID values of the wrong shape fail rather than becoming absence.
A stated map is a native Mapping with unique ascending UTF-8 string keys and
UTF-8 string or null values. Incorrect entries/order fail at their located
`altids` entry; no sorting, retyping or fallback hides them. A stated map is
borrowed; no schema or value is re-inferred. Stated UUIDs require the native
Uuid value, not new version/variant validation; the packed values accepted by
decision 22 remain accepted.

Map member-name order determines identifier priority, equally for stated and
derived maps. Values are case-sensitive UTF-8; null/empty values contribute
nothing, but nonempty text is neither trimmed nor case-folded. Equal values
under different member names are one key. Nested identifiers remain nested.
Identifier keys clone the value's compact shared string handle instead of
copying long text. Index admission removes duplicate occurrences in linear
expected time; a new chain sheds excess input-vector capacity so repeated
values cannot retain storage proportional to their occurrence count.

**Joining.** In order: a stated `puuid` naming a live chain joins directly;
else the first scoped identifier naming a live chain wins and corrects even
a foreign stated `puuid`; else a stated `puuid` opens under itself; else a
message with identifiers generates a version-7 chain UUID. A message with
neither identifiers nor a stated `puuid` opens no chain. Explicit direct joins
can attach identifiers in another instrument scope to the same chain.

When identifiers reach two live chains, the first wins; the chains do not
merge, and a key already owned by another live chain is not stolen. Only
unowned keys attach to the selected chain. A terminal message closes only
the selected chain, removing all its scoped keys. A first terminal message
receives its stamps but retains no new chain or index entries.

**Generated identity.** Decision 22's raw instrument digest is replaced here
by the authoritative effective scope in the chain payload. The exact xxh3
input is a presence byte (`0x00` when absent, `0x01` when present), the
present UUID's sixteen big-endian bytes, `0x1f`, then the first identifier's
UTF-8 bytes. The existing `Xxh3` streams this input; the unchanged impact
clock and `Uuid::from_v7` pack it. Two explicitly different scopes with the
same identifier/clock must not collapse because their raw instrument fields
are absent or disagree with the stated scope. Timestamp selection, hash
algorithm and UUID packing otherwise remain decision 22's. A generated UUID
colliding with an unrelated live chain is a located `$.puuid` refusal, not an
implicit direct join. These deterministic hashes are not collision-free.
The `puuid` field description names the lifecycle-owned recipe rather than
duplicating its byte layout; its old raw-instrument description is removed.
The two registry stable-hash pins move with that field metadata, not the
arrival-equivalence snapshot.

**Atomicity and replay.** Resolve all keys, UUIDs, collision checks and
message writes before publishing any chain/index mutation. The existing
atomic `set_many` preparation admits a crate-private field check immediately
after resolving each target, before its one scalar canonicalization. Lifecycle
uses it to require a native UUID target even in a custom message registry;
coercion into text or bytes cannot publish a chain that replay cannot read.
This one checked write is the last fallible step. Stated `uuid` and `instuuid` stay
untouched; only `puuid` may be corrected by a known chain. Stated `puuid`s
rebuild state on replay instead of bypassing the lifecycle, so a fresh second
pass returns equal messages and the same live count. Arrivals, wire and
arrival digests remain untouched.

**Pins.** Foreign and direct stated joins; two instruments reusing one ID;
equal/different stated scopes independent of raw instrument fields; absent
scope; first-wins conflicts without alias theft; terminal cleanup and
reopening; clear and replay; empty/stated maps and compiled fallback parity;
integer-to-text identifiers, located invalid text/shape and atomic refusal;
duplicates, null/empty/case/whitespace, unknown types and no nested promotion.
The 129-line capture yields 83 messages and retains its four final live chains
through the direct door (144 lines, 95 messages and five chains since the
cancel/reject flow of decision 38's corpus entry, whose request opens a chain
its instrument-less reject never meets). Enrichment derives terminal state at
lines 35 and 73 where the raw messages omit it. Its new final count is three,
not the old four (four with that fifth chain): the old hard-tag fallback
reopened the closed ABBN.S chain from the untyped FIXML at line 101. That
message states no MsgType and therefore has no declared identifier selection.
Line 102 likewise supplies no selection; its HOLN chain opens from the typed
order at line 107. Neither an order-only fallback nor a fabricated document
type preserves the old enriched count.
Pin these exact capture facts and each door's full replay independently;
the direct and enriched messages do not state the same terminal information.
No equivalence snapshot regeneration is expected for this lifecycle-only
change.

## 24. A chain carries only its previous message's clock and UUID

**Rule.** Append `prevupdatedat` (65021, `PrevUpdatedAt`) and `prevuuid`
(65022, `PrevUuid`) to the crate definitions. The former has the existing
`timestamp` datatype, DateTime64 nanoseconds in UTC; the latter is Uuid.
Both are nullable. There are twenty-three crate definitions: twenty-two
scalar fields and the existing altids Map group. Existing tags, identities,
group counters and DataType discriminants do not change.

Each live Chain holds exactly its last successful message's timestamp and
native UUID beside its owned identifier keys. No history or previous-row
copy is retained. The previous clock is the existing row-clock owner's
`stamped_clock` result: a stated capture timestamp, otherwise the message's
existing market-clock reading, otherwise the epoch. It is not the lifecycle's
microsecond impact clock and is never reconstructed from UUID bits. Restate
it through the existing nanosecond UTC datatype once when a selected chain
will remain live, refusing an unrepresentable clock before publishing any
chain/index changes. Orphan and terminal messages retain no current history
pair, so they do not acquire a needless history-clock conversion or refusal.
This reuses the clock owner's existing wire-input behavior; it adds no second
validation of text that owner already discarded. Non-null history clock
conversion failures are located at `$.timestamp`.

**Stamps.** Resolve the selected chain by decision 23 before preparing its
previous pair. Fill each absent/null previous field independently from that
pair; preserve each non-null stated value independently.
Stated `prevuuid` must be a native Uuid; stated `prevupdatedat` must be a
native DateTime64 in nanoseconds and UTC. Wrong shapes/parameters refuse at
their field, rather than coercing a statement or bypassing validation.
A first message or one belonging to no chain carries null previous fields
where none were stated. A previously stated previous value is not evidence
that the current message happened earlier or later; stream arrival order
alone advances the chain. The stored last pair always comes from the current
message itself, never from its previous fields.

The existing checked atomic write path validates the resolved native target
for every generated identity/previous stamp, with one target resolution and
one scalar canonicalization. The check receives the resolved input key, so
each stamp requires its own intended datatype, including a first-message
null; neither a target's renamed column nor a whitelist of both layouts can
select its meaning. Prepare any retained current clock, UUID and previous
stamps before this write. Only its success can attach keys, advance the last
pair, open a chain or close one. A refusal on any stamp cannot advance
history or close the selected chain. A terminal message receives its previous
pair before the chain and all its keys are forgotten; reopening or clear
starts without history. A first terminal event retains no chain.
Replay means a fresh or cleared lifecycle over the same stamped stream.
An earlier message fed to an already advanced lifecycle is a new arrival,
not a rewind of its state.

**Pins.** Three-message ordering; first nulls; independent stated previous
values; capture clock distinct from impact clock, nanosecond precision and
epoch fallback; direct joins and foreign correction use the selected chain's
history; separate scopes have separate histories; terminal/clear/reopen;
malformed/coercible targets, first-null wrong layouts and unrepresentable
live-history clocks are atomic; terminal/orphan messages keep the UUID
clock range without needless history conversion; full replay including Arrow
batches at multiple row boundaries. Exact field types, tags, displays, counts,
schema suffix and new registry hashes are pinned. The snapshot is unchanged
because its reader does not run lifecycle.

The clock declaration is shared with the existing clock owner, and fixed
schema construction resolves its tag list once with capacity derived from
the real definition counts. This does not change schema order or selection.
The later updatedat/grid/code-only identity rules remain separate work.

## 25. Hash implementations share one hashing owner

**Rule.** Move the complete Rust implementations and their unit tests to
`hashing::xxhash` and `hashing::txhash` under `rust/src/hashing/`. The parent
module explains the byte/value digest, instrument/message identity and
time-ordered pair, and owns no second dispatcher. Retire the old root module
paths entirely; no aliases or forwarding modules remain. Shared root digest
vocabulary stays in `digest.rs`, as the repository contract requires; its
implementation imports the actual hashing owner. Existing named root field
type re-exports continue to name the same owning types, not a second module.

Move the structural/display stable-hash adapters from `text/display.rs` to
private `hashing/stable.rs`, with crate-private access through `hashing`.
Remove their old text and crate-root re-exports and update every caller.
The existing explicit little-endian structural sink also implements
`fmt::Write`, replacing the separate display-only sink without changing a
byte fed in either mode. Keep both existing vector tests beside that owner;
text retains only rendering and bounded-error helpers. Trait implementations
remain on the values they describe, delegating to the hashing owner.

Keep `Scalar::stable_hash` and `FieldRecord::stable_hash` as the same public
methods, but locate their implementations beside the existing canonical
scalar/row digest methods in `hashing/xxhash/scalar.rs`. A typed row still
feeds its borrowed cells and builds no temporary sequence. Value-specific
recipes (version-tail folding and FIX definition-tag placement) stay with
their values, using the existing one-shot hashes when their complete input
slice is already available. In particular, version-tail hashing no longer
allocates a streaming state. Pin zero allocations for nonnumeric suffixes
at several input sizes and retain the exact folded-version/tag vectors.

This is a location change, not a new hash contract: algorithms, seeds,
canonical scalar bytes, arrival digests, UUID packing, raw TxHash bytes,
ordering, null/default behavior and errors do not change. A TxHash remains
its existing raw time/digest pair, not an RFC 9562 UUID. Any later UUID
projection is a separate decision. Existing digest/row/stream/holder pins
must remain exact, and the equivalence snapshot must not move.

Correct the touched raw-layout prose: byte ordering matches time ordering
within a fixed unit and algorithm and one sign range; negative instants sort
after nonnegative ones in the existing two's-complement bytes. Derived value
ordering compares unit before its count; it does not normalize instants.
The existing negative-boundary tests pin this bound, not a changed layout.

The benchmark sources mirror the hashing layer. One `hashing` driver
replaces the two old drivers, with xxhash/txhash submodules and one shared
deterministic payload implementation. Existing case names, fixture bytes,
smoke bounds and cost assertions remain unchanged. Compile and run only
the suite's untimed fixture checks; measure no benchmarks.

Rust imports, rustdoc examples, inventory and workspace consumers use the
new core paths in this commit. The user's Rust-first sequence defers host
module regrouping and parity suites until the Python and then Node stages.
Their compile-time core imports change now only as required by Gate 1.

The final documentation stage consolidates the six xxhash/txhash pages
into `docs/hashing.md`, one Hashing navigation item. Rewrite incoming links
and delete the old pages; do not keep redirect pages or old anchors. Keep
the complete canonical encoding, time/overflow, holder and language-bound
contracts, and historical measurement provenance without rerunning it.

**Pins.** Existing canonical digest vectors, seeded/secret and chunked
equivalence, logical cross-width hashes, Arrow exchanges, holder and
allocation/call-count assertions, and raw TxHash layouts remain exact.
The old Rust module paths disappear from source and API inventory. Gate 1
is whole; binding/documentation checks remain in their user-ordered stages.
All twelve FIX pieces are now landed, so remove the spent root prompts
`FIX_DIRECTION_MESSAGES_PROMPT.md` and `CHARSET_READ_PATH_PROMPT.md` in this
commit. Their complete records remain recoverable from Git history.

## 26. Settled clocks and code define message identity and grid snapshots

**Rule.** This supersedes the identity and clock rules of decisions 22–24;
those decisions remain historical records, never compatibility modes.
The user retires msghash: uuid is the only stored message hash identity.

### Vocabulary and ownership

- Rename tag 65003 timestamp to updatedat, including its public constant and
  FIX accessors; no alias. Retire msghash/tag65000 without reusing the slot.
- Generic TextLine.timestamp and ULBRIDGE_ROWHEADER's timestamp capture remain
  capture context, not FIX aliases. Delete FIX's special Clock capture role and
  private CLOCK_COLUMN mapping; neither holder mtime nor capture-header time
  overrides the event's60>52 rule. Explicit resolved FIX clock fields use the
  ordinary fill path. Fixed FIX rows no longer claim the name timestamp, so
  carrying batches retain that ordinary capture column and hash it as context.
- Add createdat65023, code65024, snapshotat65025. Clocks are DateTime64(ns,UTC),
  code is UTF-8; all are non-null. Empty code is the explicit unknown name.
- FixMsg directly holds updatedat, createdat, uuid and puuid as private,
  exact-layout Scalar values. Borrowed accessors/get_by_tag answer those
  fields without registry lookup, metadata read or child scan. The eager row
  contains derived mirrors; one finalizer publishes mirrors and hard values
  together. Clone copies settled values and never reads a clock.
- This finalizer serves initial intake, mutation, enrichment and row exchange;
  there is no separately stored content digest and no second hashing engine.
  Keep existing equality over schema/value/registry, not identity alone.
- Resolve mandatory roles once at each schema boundary. Explicit fix:tag wins
  and malformed metadata propagates; absent tags use the existing numeric-tag
  parser or registry canonical-name/alias resolution. Do not parse paths or
  infer a role merely from its datatype. Correctly tagged renamed fields keep
  their names/positions; a name claiming a different mandatory role than its
  explicit tag refuses. Duplicate mandatory holders refuse even when equal/null.
  Ordinary duplicate-tag semantics do not change. Fresh recognized tagless
  fields gain their resolved tag on the staged schema; only genuinely missing
  roles are appended. The retired name timestamp receives no special lookup.
- FixMsg::remove becomes fallible if needed to refuse removal of mandatory
  fields explicitly; never silently restore a removed field using now.

### Clocks and initial intake

- Seed standard SendingTime52 and TransactTime60 definitions into FixRegistry::new, outside
  fix_crate_fields: existing registry Tag/Name/Id indexes and builder memo
  remain the sole resolver. Loaded dictionary definitions supply their metadata.
  TransactTime is optional in messages: its seed types a stated60 once, not
  an extra always-filled cell. Delete the old interior UTF-8 wire-clock reparse.
- Registry removal/override remains generic; FIX intake validates the required
  layouts and refuses missing/mistyped declarations. Do not add a parallel
  fallback resolver or classify standard52 as a crate tag.
- Stored-catalog readers load through the same private base constructor before
  installing missing standard clock seeds: shipped52/60 documents must not
  collide with preinstalled defaults, and duplicate stored documents still
  refuse. Public create_definition never becomes an overwrite operation.
- FixCodec exposes a validated optional default_sending_time in ns/UTC.
  None means one lazy UTC-now read for each genuinely new undated message.
  SendingTime precedence: valid message52, valid carrier52, configured default,
  now. A malformed stated clock is an error, not an absent clock to overwrite.
  Deferred namespace and carrier-clock conversion use the same resolved FIX
  version as direct entries, including version-qualified enum aliases; the
  builder carries that version rather than recovering or guessing it later.
  Equal raw namespace voices from different source versions must also agree
  after typing: conflicting meanings fill nothing, while equivalent literal
  instants coalesce. Compare only distinct source versions of raw-consensus
  critical candidates; raw-conflicting or overridden voices remain untyped.
  A conversion failure in a surviving candidate remains a located refusal.
- snapshotat is the real event instant: explicit valid snapshotat, otherwise
  TransactTime60 then settled SendingTime52. Initial updatedat and createdat
  each default to that instant; valid explicit values are retained.
- No wall clock participates after this initial boundary. Mutations and
  enrichment carry resolved clocks unless explicitly changed. Parsing fresh
  undated bytes without a fixed default is intentionally not deterministic;
  replay carries the settled message/row or uses the same configured default.
- Existing namespace composition runs before mandatory default selection, so
  a composed SendingTime remains a stated source and never loses to now.
  Preserve its existing implementation rather than add a second name reader.
  Its two callers are codec build branches: relocate it onto existing Built,
  between Builder::finish and FixMsg::from_built. Borrow existing fields/cells,
  type only winners, and rebuild only when one lands; no FieldRecord allocation,
  unsealed FixMsg, mode flag or duplicate composition pass.
- The shared scanner retains each segment's original value end privately.
  FIX intake projects that original span only for a key resolving to code,
  including registry aliases and namespace candidates, through the existing
  bounded key memo. Public TextEntries and prose trimming are unchanged;
  no delimiter is rediscovered and no second parser or cache is introduced.
  Nonempty whitespace code bypasses the capture-wide empty spelling, while
  ordinary fields and clocks retain their declared absence semantics.
- Preserve parser cardinality and typed errors: empty payload stays no message;
  best-effort malformed-body fallback finalizes only its accepted message once.
  No caller-controlled layout reaches empty_with.expect, and clock errors
  cannot be caught and replaced by an empty message.
- Use the two existing error channels: outer Result is payload discovery/syntax,
  FixMessages items carry materialization and mandatory-contract failures.
  Single-frame/bridge fast paths wrap their build Result like lazy multi-frame
  sources already do. Parse XML attributes before materialization; allocate
  the TextBytes page before any syntax-recovery closure. No new error enum.
- Builder retains only the first root mandatory-value conversion error until
  finish instead of converting it to Null; ordinary fields/groups keep their
  existing best-effort typing. Pin declared-null spellings separately from
  malformed values, including early empty-value and cleaning filters, so invalid
  input cannot disappear before the checked conversion.
- Plugin configuration attributes remain document-owned; synthetic SendingTime
  is not exported as a plugin attribute. Wire emission and arrival digest still
  read only the immutable arrival record, not synthetic defaults.

### Content identity

- puuid is UUIDv8 packing of unseeded XXH3-128 of exact code UTF-8 bytes alone.
  Unknown code gets the same deterministic hash of empty bytes, not nil and
  not a live unnamed chain. Nonempty whitespace is a real distinct name.
- Canonical content is the actual named semantic row cells sorted by exact
  field-name UTF-8 bytes, with existing Scalar Record framing and recursive
  canonical value feed. Omitted differs from present-null; empty differs from
  null. Names, values and nested member order matter; root column order does
  not. Schema metadata itself is not content.
- Exclude uuid (self-reference), updatedat and createdat by resolved tag;
  exclude reserved arrival-materialization columns because they are the
  separately carried wire record, not interpreted message fields. Include
  every other row cell, including code/puuid/snapshotat/52, identifiers,
  history, unknown semantic fields and retained capture context/body.
- Reuse the FIX column plan and its bounded cache for canonical content order;
  share one depth-aware named-record feed with Scalar rather than allocate a
  Record, serialize JSON or route through Arrow digest-holder selection.
  Cache pointer identity is fast; equal reconstructed layouts may require a
  structural comparison, so claim no zero-cost schema comparison.
  Since mandatory-role planning may resolve registered aliases, the one-entry
  cache also retains and keys the registry Arc, not only Fields storage. This
  prevents a schema from reusing another registry's interpretation.
- An unseeded transient XXH64 feeds TxHash at updatedat's exact nanosecond count;
  only its UUID projection is stored. Arrival FixMsg::digest and dedup keep
  their existing distinct contract.
- Externally stated uuid/puuid are assertions and must match the computed
  values. Ordinary content mutation recomputes them; it does not validate the
  previous identity as if the caller had newly asserted it. Explicit identity
  writes must match the complete candidate state. Publish atomically.

### UUID packing

- Add TxHash::into_uuid using RFC9562 UUIDv8: preserve all 64 signed nanosecond
  bits, order-preserving via sign-bit flip, plus the low 58 digest bits around
  the required version/variant bits. A private Uuid helper owns packing.
- Let t=(ns as u64) XOR 2^63. Before from_v8 fixes reserved bits, payload is
  (t>>16)<<80 | ((t>>4)&0xfff)<<64 | (t&0xf)<<58 | digest_low58.
- Reuse checked unit conversion; refuse counts not representable in i64 ns
  and digests without a 64-bit payload. Algorithm is not encoded. This is a
  lossy 58-bit content fingerprint, not cryptographic uniqueness or an inverse
  of TxHash. Existing raw TxHash bytes and derived ordering are unchanged.
- Pin MIN/-1/0/1/MAX, 15→16 and 65535→65536, equivalent units, overflow,
  version/variant, intentional high-digest-bit truncation, zero allocations.

### Row exchange

- A replayable FIX row carries all seven fields: updatedat,createdat,uuid,
  puuid,code,snapshotat,SendingTime52, each with an exact non-null layout.
  Validate presence/uniqueness/types before record canonicalization can invent
  defaults. Row export/import never reads now or recovers clocks from UUID.
- Partial business projections remain supported if they retain this bundle;
  missing bundle members produce a typed located refusal. Plain Arrow can
  still hold arbitrary projections, but those are not replayable FIX rows.
- Canonicalize exactly the values Arrow will receive, then recompute projected
  identities AFTER capture-column overlay. Reuse the row Vec, not a second
  FixMsg/Record/JSON tree. from_row verifies these identities.
  A matching DataTypeId alone is not proof of matching units/scales/parameters:
  replace that fitting shortcut with the existing Field::scalar contract.
  Unrepresentable projected values refuse with their location before hashing.
- Adding null padding, renaming columns, regrouping members, deriving values
  or carrying capture context may change uuid; unchanged named content does
  not. Projection does not change the source message, and retained code keeps
  the chain name. Second export under the same schema is identical.
- The four pre-existing invalid group fixtures, frames/verbatim 028 and 031,
  now refuse at row export through Field::scalar rather than at later import.
  The indexed-symbol fixture lift 012 also refuses at $.symbol: its two-value
  Sequence cannot fit the dictionary's string column. The former fitting
  fallback silently replaced that stated value with null; that loss is no
  longer accepted. Pin these five refusals and their exact locations, retaining
  their type, wire, digest and arrival-entry pins. They have no valid projected
  row to pin, so their former field lines are removed only with this decision's
  snapshot change. A separate semantic pin proves the list survives in the
  source message and its own schema, and a failed projection changes neither.

### Lifecycle and grid in this decision

- Update obsolete identity writers now: lifecycle writes canonical code and
  other facts, and the single message finalizer derives uuid/puuid. It must
  not write old version7 message/chain identities beside the new rule.
- Explicit nonempty code selects its live chain globally, even across
  instrument scopes. Otherwise scoped identifiers find a live chain's code;
  absent a match, the first identifier derives an unambiguous readable code
  '<scope UUID or ->/<exact identifier>'. No identifier means code stays empty
  and opens no chain. Different explicit codes never merge or steal keys.
- Chains retain their code (the hash cannot recover it) and reject an actual
  different-code hash collision atomically. Terminal/reopen/clear retains the
  established live-only storage bound. Reusing code means the same puuid but
  starts a fresh live incarnation; no historical tombstones are kept.
- A positive snapshot interval X defaults to 1_000_000_000 ns, a second; the
  configurable interval is settled before live chains exist. Grid instant
  g=floor(t/X)*X uses i128 Euclidean arithmetic and checked i64 conversion.
  Exact boundaries open their bucket; never use now for grid alignment.
- Cadence belongs only to FixLifecycle: DEFAULT_INTERVAL_NS, interval_ns,
  set_interval_ns and the usual fallible consuming setter. Repeating the
  current interval is a no-op even with live chains; changing it requires none.
  FixCodec::lifecycle retains its default-cadence convenience. A singular
  snapshot returns Result<Option<FixMsg>>; consuming snapshots over Result
  messages owns the configured lifecycle and filters only successful None
  answers, directly composing with existing codec messages/arrow_reader doors.
  No new iterator class, second cadence setting or hidden row representation.
  Preserve the existing full lifecycle stream's per-item errors: yield source
  or transition errors without advancing lifecycle state, and permit the next
  input when the source does. Fuse only exhaustion, not an error value. The
  caller can collect Result to stop on error; add no hidden failure bit.
- A single transition powers full fill and zero-or-one snapshot output. It
  handles every accepted message once, including suppressed and terminal ones;
  every returned full message's updatedat becomes g and its UUID is finalized
  after every code/history/grid stamp. snapshotat keeps the real instant.
  Both doors commit that exact final pair; presentation does not change state.
- Per live chain retain highest consumed bucket. An already-aligned updatedat
  produces no snapshot but consumes its bucket; equal/older buckets suppress.
  Only the first accepted arrival above the high-water may emit, and only if
  it was off-grid on arrival. No pending full message/timer/backfill. Unknown
  code emits no lifecycle snapshot (no shared suppression chain for H('')).
- History follows every accepted message, not only emitted snapshots; a
  previous UUID may point outside the snapshot stream. Closed/reopened chains
  forget suppression/history: uniqueness is bounded to one live incarnation.
  Decision24's stated non-null previous values remain authoritative. Only
  lifecycle-filled previous values necessarily reference the actual preceding
  finalized full message; state always remembers that full message's own pair.
- Late arrivals can move previous-message time backward while high-water stays
  at its maximum. A suppressed terminal still closes. A named standalone
  terminal may emit its off-grid result and retains no chain state. Truncation
  can put updatedat before createdat and snapshotat; no opposite inequality is
  enforced. Reopening the same code in the same bucket may emit again.
- Decision27 remains separate: carry createdat from the first chain message,
  with replay/history/grid integration pins on top of these settled identities.

### Pins and owning snapshot changes

Mandatory defaults/clock precedence and malformed declarations/statements;
atomic setters/removers; direct borrowed hard-field access and allocation cost;
stable named-content ordering/nulls; code-only puuid; UUID packing vectors;
full/narrow rows, tampering, capture overlays, second-export identity;
enrichment idempotence and unchanged wire/arrival digest; deterministic replay
with fixed intake clocks; two messages/one bucket; already-aligned suppression;
negative/boundary/overflow arithmetic; terminal/reopen/clear and no-name bound.

Regenerate equivalence ONLY after intentional changes and exact moved keys are
catalogued, with WRITE=1 in this decision's commit. Never exempt the new clocks
or identities from the tripwire to conceal nondeterminism. Registry hash pins
change for the renamed/added/retired definitions and seeded standard clocks.

## 27. A live chain carries its first creation instant

NEXT_STEPS section 3, after decision 26; msghash stays retired. The settled
message identity and epoch grid are unchanged.

- A live incarnation retains the exact native createdat from its first
  successfully accepted message. First means arrival order, not minimum
  event time or the truncated grid. Later stated creation instants do not
  replace this fact.
- Every later message selecting that live chain receives that same createdat,
  including late, already-aligned, suppressed and terminal arrivals. Explicit
  code still selects globally; otherwise existing scoped identifier ownership
  selects the chain. An unrelated or occupied identifier cannot transfer its
  creation instant to another chain.
- Unnamed messages and standalone terminal messages preserve their own settled
  creation instant and retain no creation state. Closing or clearing forgets
  the instant with the live chain. Reopening the same code/puuid establishes a
  new live incarnation from that new first message, even in the same bucket.
- Carry the creation instant in the existing atomic lifecycle stamp, before
  the message identity finalizer. No new writer, clock read or dispatch. The
  chain publishes only after the complete message succeeds, so a failed first
  or subsequent arrival cannot establish, replace, close or advance creation
  state. State grows by one native Scalar clock per live chain, no historical
  record, pending message, extra container or tombstone.
- Decision 26 continues to own updatedat truncation, snapshotat preservation,
  code/puuid, named-content UUID and previous-message history. Since createdat
  is excluded from content identity and already present in every message,
  carrying it alone does not change uuid or puuid. Previous stamps still refer
  to the finalized preceding full message, not the filtered snapshot stream.
- A fresh or cleared lifecycle replaying the same settled raw stream produces
  exactly the same full results and snapshots. Replaying the already-filled
  full stream through a fresh or cleared full lifecycle is also identical;
  its aligned inputs emit no snapshots while rebuilding the same live state.
  Feeding an earlier message into already-advanced state remains a new arrival,
  never an implicit rewind.

Pins: distinct explicit creation instants (first not the minimum or grid);
explicit-code cross-scope joins and occupied-key isolation; aligned-first,
suppressed and late arrivals; terminal/reopen/clear/no-name lifetimes; failed
first/subsequent publication; raw and already-filled replay; equal projection
through Arrow; the 83-message capture (95 since decision 38's corpus entry)
and its existing exact live-chain counts.
Extend the existing lifecycle benchmark inputs/assertions without executing
measurements. Update Rust ownership docs and inventory; binding additions
remain deferred until the complete Rust story per the user's stage order.

No equivalence regeneration is expected: that tripwire does not run lifecycle,
and neither registry definitions nor canonical digest bytes change here.

## 28. One composable pipeline and one arrival record

NEXT_STEPS section 4 and the user's tag-zero instruction. The existing doors
are the pipeline; add no alternate converter, entry family or representation.

- parse_text_lines accepts owned or borrowed TextLine and fallible versions;
  enrich_messages, lifecycle and arrow_reader accept owned FixMsg or its
  Result. Standard Into<Result<T>> intake moves the original typed error unchanged,
  borrows lines without cloning them, and resolves before invoking the stage.
  The parsing target Result<T> fixes inference with T: Borrow<TextLine>;
  local From implementations wrap owned/borrowed lines in Ok, while fallible
  input uses the standard identity conversion. Ownership-required doors fix
  T to TextLine or FixMsg. No clone, custom trait or intermediate value family.
  Stream enrichment remembers successful messages only. Source/parse/enrich
  errors remain items, and exhaustion is fused. Arrow converters keep their
  completed prefix then error and fuse, as their existing batch contract says.
- TextLine's Arrow reverse reader holds one batch and one row cursor, never
  a Vec of decoded lines for that batch. Resolve its column plan once at
  intake and use the existing indexed Arrow value reader rather than making
  one-row array slices. Both directions accept fallible and infallible owned
  lines where ownership is required; the single-batch door may collect its
  explicitly bounded answer. A changed source schema refuses and fuses.
  Every row refusal is located by the stream ordinal and column, never by a
  row number an earlier column of the same row restored.
- A persisted rownum is start_rownum plus the original physical index; reverse
  conversion subtracts that same configured start and refuses underflow.
  Without that column, indices are continuous stream ordinals across batches,
  not recovered physical positions: dropped physical gaps are unrecoverable.
  Captures keep their declared index even across projected holes. Typed
  captures render through the canonical core scalar text implementation;
  their original lexical spelling is not promised after typing (0007 becomes
  7). Missing cells remain absent; malformed present values refuse rather than
  silently becoming zero or disappearing.
- nofixentries alone carries the complete recursive arrival record.
  Delete nounmappedfixentries, its exported constant, secondary projection,
  filtering traversal and identity exemption. No alias, duplicate column or
  new "unmapped" family. Unresolved names AND unresolved numeric wire keys
  have entry tag 0, preserving the exact raw key, value, order and children.
  Known scalar and group keys retain their resolved canonical positive tags;
  an occurrence key (indexed or packed) names no field and also records 0.
  The builder's existing resolution owns that distinction; no second lookup
  walk. Members of an unresolved counter still nest under its entry: while
  building, the unresolved key carries its parsed number negated, which no
  resolved tag can meet, and finish publishes it as 0. Dynamic semantic
  columns keep their existing raw names and values. The digest's envelope
  exclusion reads resolved tags, so under a registry that does not define a
  header tag that key is an unresolved arrival hashed with its raw key.
  Tag 0 means unresolved arrival provenance, not a registry identity:
  fix:tag, fix:counter and the existing fix:tags alternate-tag list admit
  positive i32 values only; FixId::of refuses zero for the same reason.
- Arrival serialization retains three materialized entry levels and the
  existing canonical JSON folded subtree beyond them. Its inverse validates
  every folded entry's four-member shape, nonnegative i32 or null tag (null
  reads as 0, as the nullable materialized tag column does), text/null
  key/value and sequence-or-binary children, refusing malformed present data,
  an undecodable folded leaf included, at the arrival path instead of
  silently dropping it. Empty binary is an
  empty subtree; nonempty binary must decode to an entry sequence.
- Retain decision 20's exact LIFTED_OUT_OF_A_DOCUMENT bound: FIXML behind
  XmlData(213) is recorded as envelope pairs plus the whole document, not
  invented child arrivals. Closing it requires an emission provenance model
  so re-emission does not duplicate the document, outside this converter
  change. The suite asserts the exact divergent pairs, never a broad exemption.
  Non-UTF8 entry bytes likewise retain the existing explicit UTF8 row bound.
- The 129-line/83-message capture (144 lines and 95 messages since decision
  38's corpus entry) pins direct lazy composition and both Arrow
  directions, one decode, source error identity with fusion, restored row
  numbers, bodies and capture presence; the existing row-by-row/batch
  agreement keeps carried/lifted columns and LIFTED_OUT_OF_A_DOCUMENT exact.
  Synthetic sources pin per-door fusion, bounded first-row work at three
  widths, capture holes and located refusals. The existing pipeline and
  text-line benchmark groups gain the composed decoded-line read and the
  reverse reader's first row and round trip; no benchmark measurements.
  Binding and documentation updates follow the full Rust story.

Unknown numeric entry tags deliberately change from their unregistered
numbers to zero, so their arrival digest now includes the raw key under the
existing zero-tag framing. Only those entry lines and their enclosing digest
lines may move in equivalence; semantic rows, wire and message counts must
remain unchanged. Review exact moved keys before the sole WRITE=1 regeneration
in this decision's commit, and name those keys in its message.

## 29. A scalar is one enum, and every width is a variant of it

NEXT_STEPS section 5, which the user extended from four families to all five:
`Integer`, `Floating`, `Decimal`, `Temporal` and `Nested` were an extra enum
between `Scalar` and the width that holds the value, and every match paid for
the level. Decision 28 is the last word on the FIX pipeline; nothing here
changes a FIX reading.

- Their 28 variants become direct `Scalar` variants under the names the
  families already used, which are also what `Scalar::kind()`, the serde tags
  and AGENTS "Generic scalar" spell: `I8`, `I16`, `I32`, `I64`, `U8`, `U16`,
  `U32`, `U64`, `I128`, `U128`; `F16`, `F32`, `F64`; `D32`, `D64`, `D128`,
  `D256`; `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64`, `Duration32`,
  `Duration64`, `Interval`; `Sequence`, `Mapping`, `Record`. `Scalar` has 38
  variants. The five family enums are deleted outright - no alias, no
  re-export, no `Integer`-shaped grouping left behind - and
  `Scalar::Integer(Integer::I32(Int32(2)))` reads `Scalar::I32(Int32(2))`.
- The short names are kept rather than DataType's long `Int32`/`Float64`/
  `Decimal128`: one scalar vocabulary already exists and is wire-visible, so a
  second spelling would be a rename, not a flattening. Nested keeps
  `Sequence`/`Mapping`/`Record`, because a field narrows a sequence to a list,
  fixed-size list, struct or union and no one DataType names it. `I128`/`U128`
  mirror `DataTypeId`, which has them where Arrow's DataType has none.
- `Code` and `Geospatial` are not intermediate width families of this kind and
  stay as they are; `DataTypeKind`, `TemporalFamily` and `Scalar::family()`
  classify values rather than hold them and stay too.
- `std::mem::size_of::<Scalar>()` stays pinned by a const assertion at its
  measured post-change size, with the size named in the commit; the family
  enums' own size assertions go with them and leaf assertions stay.
- Nothing serialized moves: serde tags and shapes, `value_rank` numbers,
  stable hashes, Arrow projection and Python pickle state already speak leaf
  names. Cross-width value semantics stay exactly as they are - `I32(7)` equals
  `U8(7)`, `F32(1.5)` equals `F64(1.5)`, `D32(1250, 2)` equals `D256(125, 1)`,
  in equality, order and hash - computed by crate-private readers in each
  width's own module instead of by a family type. `Hash` keeps its exact
  bytes: `Interval`, `Sequence`, `Mapping` and `Record` still feed the
  discriminant their retired enum wrote, so hashes derived over `Hash` (a
  message's, a plugin's) keep their pinned values. Debug loses the family
  level; no test pins that text, and its one host-visible reader is Python's
  `Bounds` repr.
- Each family method is settled once:
  - `Integer::as_i128`/`as_u128` already live on `Scalar`; `is_negative` and
    `magnitude` become one crate-private sign-and-magnitude reader for
    ordering, hashing and defaults; `into_scalar` is `From`, `stable_hash` is
    `Scalar::stable_hash`. `Scalar::as_integer` goes; `is_integer`,
    `as_i128`, `as_u128`, `as_i64`, `as_u64` answer across widths.
  - `Floating::as_f64`/`as_f32`/`as_f16` already live on `Scalar`;
    `bit_width` goes, because the variant, `kind()` and `id()` state the
    width; `Scalar::as_float` goes.
  - `Decimal::coefficient`/`scale` go; `Scalar::as_decimal` already answers
    the unscaled coefficient and scale at any width.
  - `Temporal::family`, `unit`, `timezone` and `count` become `Scalar`
    readers answering `None` for a non-temporal: `temporal_family`,
    `temporal_unit`, the existing `temporal_timezone`, and `temporal_count`
    (an interval answers its nanosecond component, as before). `bit_width`
    goes for the reason floats lose it. `as_temporal`, `as_date`, `as_time`,
    `as_datetime` and `as_duration` returned the family and go; `is_temporal`
    and `temporal_family` classify, the per-width `as_date32`..`as_duration64`
    tuples stay, and an interval is matched as `Scalar::Interval` directly.
  - `Nested::len`/`is_empty` go; `Scalar::len`/`is_empty` already answer.
  - `FieldScalar` and `UncheckedFieldScalar` drop `as_integer`, `as_float` and
    `as_temporal` with them.
  - `From<Integer>`/`From<Floating> for Scalar` go; the leaf `From` impls stay.
- A width leaf is its own family, as `Boolean` and `Uuid` already are:
  `ScalarValue::Family = Self` and the leaf implements `ScalarFamily`. The
  per-kind operation traits (`IntegerValue`, `TemporalValue`, ...) stay.
- A family `Display` that callers borrowed becomes the leaf's own `Display`,
  reached through one crate-private reader on `Scalar`, never through a
  per-site width table.
- Both bindings adapt their matches and readers in this commit so the
  workspace compiles; no host-visible spelling changes, so `.api-bindings.txt`
  does not move. `.api-inventory.txt` retires every family entry (including
  its already-stale `Float`, `struct Integer` and `TemporalRef` lines) and
  adds the temporal readers; AGENTS.md names the new cross-width readers.
  `docs/types/scalar.md` follows in the documentation stage.

Pins: the size assertion; cross-width equality, order and hash for integers,
floats and decimals; every leaf's serde round trip under its unchanged tag;
the temporal readers on every temporal variant including an interval; the
unchanged `value_rank` sweep. No equivalence movement is expected, since no
FIX reading renders a family spelling; if it moves, it is a defect.

## 30. The documentation states the current contract, in lighter examples

NEXT_STEPS section 6, and the documentation stage decisions 21 to 29 deferred
until the Rust story, Python and Node were complete.

- Hashing: decision 25's consolidation lands - the six `docs/xxhash/*` and
  `docs/txhash/*` pages become `docs/hashing.md`, one Hashing navigation item,
  keeping the complete canonical encoding, time/overflow, holder and language
  contracts. Incoming links are rewritten and the old pages deleted; no
  redirect page or old anchor is kept. Python and JavaScript examples use the
  host paths the bindings now own, `yggdryl.hashing.xxhash`/`txhash` and
  `hashing.xxhash`/`txhash`.
- Every page names only current API: the FIX pages state the crate's 24
  scalar fields and the `altids` group, the settled clocks and identities,
  grid snapshots and chain creation instants, the one `nofixentries` record
  with tag 0 for unresolved arrivals and positive registry tags, and the
  composable pipeline doors; the text page states decision 28's reverse
  reader rules; the scalar page states decision 29's flat variants and
  temporal readers; the extension pages state the bound members. The docs
  FIX manifest is regenerated by its generator, which loses its `unmapped`
  projection with the column it mirrored.
- Lighter, never weaker. An example still runs, still asserts, still stands
  alone and keeps its Rust/Python/JavaScript parity (Rust-only stays
  explicit). The levers are: merge adjacent blocks that build the same
  fixture to show one flow into one block; use the smallest fixture that still
  exercises the contract (a few rows, one short capture line) instead of
  corpus-sized ones; drop a round trip a page repeats from the page that owns
  it and link there; remove blocks restating what a neighbouring block already
  asserted. A contract only an example demonstrated moves into prose beside
  the surviving example rather than disappearing.
- `scripts/check_docs_examples.py` counts per language are reported before
  and after in the commit, as counts, not timings; Gate 4 runs whole.

## 31. A code is the text it is; an identifier is the bits it is

**Rule.** A registered code stores as Arrow's `Utf8`, under its own
`yggdryl.*` extension name, holding the ASCII text a value is. The eight -
`country`, `currency`, `mic`, `cfi`, `isin`, `side`, `state`, `timeinforce` -
no longer ride `FixedSizeBinary(n)`, so nothing pads a cell to the width and
nothing trims a cell back. The width each standard fixes stays, as a maximum
the value rule enforces rather than a layout a column enforces:
`DataType::code_width` and `DataTypeId::code_width` own it, `fixed_byte_width`
answers `None` for a code, and `code_for_extension` loses the width argument
that used to make a name and a storage agree.

What tells a code from the text beside it is the extension *name*, never the
storage. `yggdryl.currency` over `Utf8` with an empty document is a currency;
the same `Utf8` under `yggdryl.string` is the string its document describes;
under no name at all it is plain text. `yggdryl.currency` over any other
storage is a foreign field wearing our name and imports as that storage - the
rule decision 14 already applied to a retired code, so a column written under
the old fixed width imports as the `fixed_size_binary(n)` it is, and there is
no reader that turns it back into a code.

`ascii_packed` is unchanged in every integer it answers. It pads a value with
trailing NUL to the width and reads it big-endian, and that padding belongs to
the packing rather than to a column: a `field:enum` document written before
this change names the same members after it, and a stable hash hashes the same
bits. A fixed string pads into `fixed_byte_width`; a code pads into
`code_width`.

A UUID does not move, and the asymmetry is the decision. `arrow.uuid` is the
canonical Arrow extension and its `FixedSizeBinary(16)` is not ours to
redefine the way a `yggdryl.*` name is; the value is 128 bits in which every
byte carries identity and no repertoire makes it text; sixteen bytes beat the
thirty-six a spelling costs, and decision 23's chains key on it. What a code
gains by moving - Parquet's `String` logical type, byte-array statistics,
dictionary encoding, a reader outside this crate that already knows the
column - a UUID would pay 20 bytes a row and one canonical extension for.

Instead the two families meet in one cast tier. A recognized UUID source is a
`StringSource` the single string ingest reads, so an identifier column casts
into every string datatype - any layout, charset or bound - where only plain
unbounded text worked before, and the second renderer that served that one
case is deleted. A fixed slot of any width casts *into* `uuid` as the spelling
it holds; sixteen bytes stay the identifier, because a trailing `0x00` there
is identity rather than padding. A bounded byte target reads its cells whether
the bound is a maximum or a fixed width, so a cell that misses the width is
refused naming the field, the row and both lengths rather than by Arrow's own
builder complaining about a slice.

**Why.** A currency is not a byte slot; it is a value from a published
registry that happens to be short ASCII. Storing it padded made the crate
invent a layout - the same objection decision 14 raised against a packed
datatype the crate invented - and every consumer outside Arrow already read it
as text: Avro spelled it `string`, Iceberg spelled it `string`, a partition
directory spelled it `USD`. Only Arrow disagreed, and Parquet inherited the
disagreement: a code column carried no logical type at all, so a reader
outside this crate saw an untyped `FIXED_LEN_BYTE_ARRAY`, and its footer
bounds carried the slot's NUL for every value short of the width. `version`
and `url` were already identities over text under their own extension names;
a code is the same shape and now says so.

**What moves.**

| where | what changes | what must not move |
| --- | --- | --- |
| `types/arrow.rs`, `arrow/field.rs` | the eight storage arms answer `Utf8`; recognition matches `Utf8` and the name alone | every other extension; the dictionary peel |
| `types/string/codes.rs`, `datatype_id.rs` | `code_width` on both; `fixed_byte_width` drops its code arms; `code_for_extension(name)`; `ascii_padded` gone with the padding it wrote | every discriminant; every packed integer; `ALL.len() == 60` |
| `types/string/casts.rs`, `types/cast.rs`, `uuid/casts.rs` | `ingest_code_array` builds text and shares a passing column; `StringSource::Uuid`; `ArrayCastKind::UuidText` and `render_uuid_text` gone; a fixed slot of any width enters `uuid` | the one UUID rule; `safe` and strict semantics |
| `types/bytes/casts.rs` | a fixed width is filled and refused here, named and located | the maximum's own refusal; the plan-time refusal for two declared widths |
| `types/merge.rs`, `budget/limits.rs`, `media/partition.rs`, `arrow/value.rs`, `bytes/scalars.rs` | a code is bounded variable text everywhere it was a fixed slot | every other pairing |
| `uuid/dtypes.rs`, `uuid/scalars.rs` | `Uuid::TEXT_LEN` and `Uuid::render` into a caller's slot; the JSON, YAML, TOML, serde and expression writers drop one allocation each | the canonical spelling; the packed value |
| Python, Node | `code_width`/`codeWidth` bound beside `code_name`; `fixed_byte_width` `None` for a code | every other accessor |
| docs | `types/codes.md` restated whole; `types/uuid.md` gains the rendering and the readings; `cast.md`, `field.md`, `text.md`, `datatype.md`, `index.md`, `arrow/schema.md`, the playground manifest | every other page |

**Written in:** `types/string/codes.rs`, on the module; `types/arrow/field.rs`,
on `recognized_arrow_extension`; `types/uuid/dtypes.rs`, on `uuid_rendered`;
`AGENTS.md`, under Strings and Bytes; `docs/types/codes.md`.
**Fixtures:** every code storage pin in `rust/tests/types/datatype/coded.rs`
restated as text, plus a code and a UUID read into every string and byte
datatype and back; a code column through Parquet, asserting the `String`
logical type and the byte-array bounds a padded column never earned; a text
column every cell of which passes, shared rather than copied; the typed-field
downcast, which would have caught the eight markers still naming
`FixedSizeBinaryArray`; `fixed_size_binary(8)` refused naming both sides;
`code_width` beside `fixed_byte_width` in the three languages; the UUID
rendering benchmarked against the owned string it replaces.

## 32. A line is the range of the window it was read into

**Rule.** The splitter's window is the page every line cut from it is a range
of. It is filled while nothing points into it and handed out only once the
fill is complete, so a line costs one reference count rather than a copy of
its bytes: the header off its front, the strips off both edges, the byte
limit off its tail and every named capture are offsets into that page. A line
takes a vector of its own only where it cannot be a range - one that spanned
two windows, and one whose row header matched in the middle rather than at an
edge. A window a line still names is never written over: the refill takes a
fresh one and moves the open tail into it.

**Why.** `media/text/bytes.rs` already said this - "The text reader fills one
page-sized buffer, seals it, and hands out ranges into it, so a line costs one
reference count rather than a copy of its own bytes" - and the reader did not
do it. Every physical line was assembled into a vector of its own, the
retained body was copied out of that into a second vector, the record copied
that into a third, and the third was boxed into a page for that line alone:
four allocations and three copies of every line's bytes, with four more per
declared capture. The counting allocator pinned it at four a line and twelve
for a header declaring two captures, and callgrind put 28% of an end-to-end
text read inside `malloc`/`free` and 7% more inside `memcpy`. The window was
already there, already one buffer, already filled in complete units; only the
sealing was missing.

Retention is the cost this trades against, and it is stated rather than
hidden: a caller that keeps its lines keeps the windows they name, which is
one page per 64 KiB of object read rather than one page per line of it. A
caller that drops each line as it reads it - the fold, the Arrow builder, the
codec - lets the splitter write the window over again and allocates nothing at
all past the constant. A line that has not ended takes its vector before the
refill rather than after, so a record larger than the window does not make the
splitter take a fresh window for every part of it.

**Written in:** `media/text/reader.rs`, on the module and on `Lines::rewind`;
`media/text/arrow.rs`, on `Held`.
**Fixtures:** in the counting allocator, `read_text_lines` over 16 and over
1 024 rows at one constant and no slope, and a second case pinning what
keeping every line costs - the constant, the collector's own vector, and two
per window - and that the bodies of 1 024 rows name no more pages than the
object has windows; through the reader, every existing line, framing, strip,
limit, capture, charset and entries fixture unmoved.

**What it costs.**

| where | what changes | what must not move |
| --- | --- | --- |
| `media/text/reader.rs` | the window behind a shared handle; `LinePart` carries a range; `rewind` takes a fresh window where the current one is named; the pinned multi-byte searcher built once per read | the flexible and single-byte scans; the overlap rule; where an over-long line is cut |
| `media/text/arrow.rs` | `Held`; `RawRow` carries `TextBytes`; `header_match` reads into the reader's own landing place and cuts each capture out of the line's page | `RawRows`' framing, dedup, limit and leading-fragment rules; every error and its location |
| `media/text/line.rs`, `bytes.rs`, `batch.rs` | `shared_url`; captures read where they stand; an owned vector becomes a page; the URL column clones a count | `TextLine`'s contract; `TextBytes`' identity, `from_bytes`, `from_page` |
| tests | the two allocation pins above, restated | every behaviour fixture |

**How it lands.** Measured against the commit before it on `text_lines`,
`text_records`, `text_record_framing`, `text_batch` and `text_scan`, named
baselines, beside the counting allocator and callgrind - because wall clock on
a shared box moved an untouched scan by 4% in both directions across two runs,
and the allocation and instruction counts did not move at all.

## 33. A scalar variant is spelled as its datatype, and geospatial flattens too

Decision 29 flattened the five width families into `Scalar` and kept their
short names, on the reasoning that a second spelling would be a rename rather
than a flattening. The rename is what is wanted: `Scalar::I8` sits beside
`DataTypeId::Int8`, `DataType::Int8` and the parser's `int8`, and `i8` is a
spelling the datatype grammar does not accept at all. Decision 29's
flattening stands; only its naming clause and its geospatial exception move.

- Every width variant takes its datatype's spelling: `Int8`..`Int64`,
  `UInt8`..`UInt64`, `Int128`, `UInt128`; `Float16`, `Float32`, `Float64`;
  `Decimal32`, `Decimal64`, `Decimal128`, `Decimal256`. The leaf type, the
  `DataTypeId` and the variant are now one identifier, so
  `integer_scalar_value!`, `floating_value!`, `decimal_value!` and
  `width_value_from!` lose the arguments that repeated it.
- `Geospatial` goes the way the width families went. `Scalar::Geometry(Geometry)`
  and `Scalar::Geography(Geography)` hold their leaves directly, each leaf is
  its own `ScalarFamily`, and the enum is deleted outright. `Code` is the one
  family enum left, because a code is an identity over a registry rather than a
  width of one thing.
- Cross-reading value semantics are unchanged, and that is what keeps the
  flattening honest: geometry and geography share `value_rank` 14 and compare
  by their WKB, exactly as `Geospatial` did, through a `geospatial_bytes`
  reader beside `float_value` and `decimal_value`. One payload is one value
  under both readings, in equality, order and hash.
- The wire vocabulary does not follow the Rust spelling. `Scalar::kind()` and
  the serde tags keep `i8`..`d256`, which decision 29 pinned as already
  wire-visible and which `every_width_leaf_round_trips_under_its_unchanged_tag`
  still holds. The one tag that does move is geometry's: `geospatial` was the
  family's name on the wire for a value the datatype grammar, the `DataTypeId`
  and the digest feed all call `geometry`, and with the family gone there is
  nothing left to call it. `kind()` and the serde tag both read `geometry`;
  `geography` was already itself.
- `i256` moves from a root file into `types/`, where the decimals that need it
  live, and is spelled like the native integer it extends. Its unsigned
  magnitude becomes `u256` rather than a file of free `[u64; 4]` helpers: every
  wide add, multiply and division was already unsigned arithmetic with the sign
  handled around it, so the pair is the shape the code had. `u256` is a value
  in its own right - parse, render, compare, checked arithmetic, serde, stable
  hash - and `i256::unsigned_abs` answers it, which is how the signed minimum
  gets a magnitude at all.

**Written in:** `types/scalar.rs`, on `Scalar` and `ScalarFamily`;
`types/geospatial/scalars.rs`, on `geospatial_value!`; `types/i256.rs`, on the
module and both structs.
**Fixtures:** `a_geospatial_value_is_its_own_kind_over_its_bytes` and
`the_structural_wire_round_trips_a_geospatial_value` restate the moved tag;
`every_width_leaf_round_trips_under_its_unchanged_tag` and the pinned stable
hashes are unmoved; `types/i256/tests.rs` keeps every signed fixture and adds
the unsigned boundaries, division identity, byte round trip and serde.

## 34. A canonical text value the crate already owns is a datatype of its own

A time zone, a MIME type and a media type were vocabularies the crate parsed,
canonicalized and rendered for its own routing, and nothing else. A column of
them had to be `utf8` - which is to say the validation, the canonical spelling
and the identity were dropped at the column boundary and re-derived by whoever
read it. `Url` and `Version` already answered this: a value that parses,
canonicalizes and renders itself is its own datatype.

- `DataType::Timezone`, `DataType::MimeType` and `DataType::MediaType`, each
  parameter-free, each `DataTypeKind::Text`, each stored as Arrow `Utf8` under
  its own extension name (`yggdryl.timezone`, `yggdryl.mimetype`,
  `yggdryl.mediatype`), each with a typed field marker, a prebuilt shared
  field, an ingest cast and a default. `Scalar::Timezone`, `Scalar::MimeType`
  and `Scalar::MediaType` hold the crate's own value types, not a second
  spelling of them: a zone read out of a column is the zone a `datetime64`
  column declares, and a media type read out of one is what `RecordOptions`
  routes a record read on.
- `Scalar::MediaType` is behind an `Arc`, as `Scalar::Url` is, because a base,
  a charset and a coding list are wider than the 48-byte scalar. The other two
  ride inline.
- Not registered codes. A code is at most twelve US-ASCII bytes so it never
  touches the heap, and its identity is a published registry; an IANA zone name
  and a parameterised media type are neither, and a media type has no
  borrowable canonical text at all - it renders one.
- They are appended, three ways, because three numberings are wire contracts:
  `DataTypeId` takes 61, 62 and 63; `DataType` takes its three variants at the
  end of the enum, because `Hash` is derived there and a stored schema digest
  is over the derived discriminant; `Scalar::value_rank` takes 21, 22 and 23,
  the next free numbers. `dtype_rank` takes 59, 60 and 61.
- Intake follows each value, not a new rule. A zone takes an alias, a case and
  a fixed offset; a MIME type refuses a name that is not `type/subtype`; a
  media type infers, so its intake is total and unrecognized text answers the
  default base rather than refusing - it is also the crate's filename and
  content-negotiation reader, and a column of them holds what that answers.
- Merging is the `Version`/`Url` rule extended: only with itself. Merging into
  text would drop exactly the canonicalization that makes the column worth
  declaring, and a MIME type and a media type never merge into each other.
- `timezone.rs` moves from a root file into `types/timezone/`, beside the
  temporal datatypes that declare a zone and the datatype that now holds one.
  The value's crate-root export is unchanged. `MimeType` and `MediaType` keep
  their root files, as `Url` keeps `uri/`: the value is the media layer's
  routing vocabulary and `types/{mime_type,media_type}/` holds only what makes
  it a datatype.

**Written in:** `types/timezone/mod.rs`, `types/mime_type/mod.rs` and
`types/media_type/mod.rs`, on each module; `types/scalar.rs`, on
`text_scalar_value!`, which is the one shape `Version`, `Timezone` and
`MimeType` now share instead of three copies of it.
**Fixtures:** `types/tests/timezone.rs` and `types/tests/media.rs` - identity,
canonicalization, ordering and hashing, the structural wire, the Arrow
extension round trip, a text column ingested with a located bad row, defaults,
merges and typed fields, and that the value is the one the rest of the crate
already routes on; the digest corpus and the prebuilt-field allocation pin
both name all three.
**Bindings:** `types.timezone` / `fields.timezone`, `types.mimetype` /
`fields.mimetype` and `types.mediatype` / `fields.mediatype`, built the way
`url` is: each value crosses as its canonical text, so neither language grows
a wrapper class and neither can hold a spelling the core did not produce.

## 35. A test in `src` names what only the crate can see

Two thirds of the Rust suite lived in `src` under `#[cfg(test)]`, and most of
it had no reason to: it drove `DataType`, `Field`, `Scalar`, the Arrow
boundary, the handles and the record formats through the same doors a caller
has - from inside, where a private helper is one `super::` away and nothing
says which contract is under test. A suite that can reach anything proves
nothing about what is reachable.

- About 1 600 tests move into `rust/tests/`, one top-level theme per subtree,
  `#[path]` modules mirroring `src/`. Every one reaches the crate as
  `yggdryl::` and no other way.
- What stays in `src` stays for one stated reason each, written in the module
  doc: the tested item is not reachable from outside. `media/iceberg` (nine
  private modules and every `TableMetadata` field), `holder/object` (signing,
  XML, the dialects), `holder/zip` (the format readers), `fix` (the identifier
  control byte, the lineage and code-set renderers, `FixMsg::from_parts`),
  `types/value` (`canonicalize_dtype_value`), `types/temporal/iso` (the ISO
  readers), `types/timezone` (the bundled registry), and about twenty
  single-item pins - `value_rank`, `leaf_display`, `low_64`,
  `algorithm_of_width`, `accepts_time`, `read_at`, `convert`, `order`,
  `open_builder`, `home_from`, `matches_segment`, `utf8_transcribe_into`,
  `from_decimal_text`, `rendered_len`, `commit_arrow_readers`, `merged`,
  `folder_reader`, `schema_json_from_field`, `Schema::names`, `Ipc`'s handle
  field, and `Metadata`'s shared pointer.
- A fixture stops borrowing the code under test where the move made that
  possible: the geometry default is compared against stated `POINT EMPTY`
  bytes, the Avro container fixture writes its own zig-zag `long`, an Arrow
  array is narrowed with a local `downcast`, a row is read through a
  one-element slice and `scalar_value`, and an equality-hash check uses
  `DefaultHasher` rather than the crate's private stable hash.
- One fixture is deliberately written twice. `Counting` - the handle that
  mirrors a `Buffer` and tallies what reaches it - lives in `tests/support/`
  for the three themes that assert against it, and a smaller one lives beside
  the page-eviction pins, which need it *and* the private `read_at`. An
  integration test cannot see a `#[cfg(test)]` item, and making the instrument
  public to share it would put a measuring device in the crate's API.
- Two defects the move surfaced, both invisible from inside: the exported
  `impl_default_iomedia!` expanded to `$crate::iobase::…`, a private module, so
  it never compiled outside the crate at all; and the two coupled-digest column
  readings matched `DigestAlgorithm` exhaustively, which a caller cannot do
  because the enum is `#[non_exhaustive]`.

**Written in:** AGENTS "Where a test lives"; each `src` suite's module doc.
**Fixtures:** every test name in the tree before the move is in it after -
3 288 before, 3 291 after, the three added being pins the split created.

## 36. One term tree, two clauses, one plan, and a best-effort rule

The expression layer is rebuilt around one vocabulary in every language:
`Term` is the tree, `Filter` is a `where` clause over it, `Selector` is a
`select` clause that is also the one owner of a schema declaration, `Plan` is
the sections of one read or write, and `Expression` is whichever of those a
piece of text is - a clause, a plan, or a `;`-separated sequence. `Statement`,
`BoundStatement`, `Order`, `Direction`, `NullsOrder`, the `ApplyExpression`
traits and the holder `Selector` enum (now `Attribute`) are gone.

- A projection carries a term, an alias, an optional declared datatype with
  `null`/`not null`, and `with (...)` metadata, so a `Selector` is a `create
  table` column list. `Selector::from_field` is lossless and `into_field` writes
  the selector back as each column's `transform:expression`; a `Field` is a plan
  holder, `Plan::from_field` is its `create` section, and a partition
  declaration is a transform of one source read through the feature-free
  `types/protocol/partition.rs`.
- Every application lives in `expression/`: `apply_field`, `apply_scalar`,
  `apply_arrow_reader` as the primary streamed path with `apply_arrow_batch`
  derived from it (one batch is a one-batch reader, and a batch returned
  unchanged is the caller's own), `apply_arrow_array`, and `apply_records` over
  native rows bound once against a declared schema or the first record's own.
  `explain` draws every layer as a UTF-8 tree; a bound tree adds datatype,
  nullability and cost.
- A plan spells `create [target] (schema) [with (...)]`, a write verb with an
  optional target and `by (keys)`, `select`, `from target | (plan)`, `where`,
  `order by`, `limit` and `offset`. `insert into`, `insert overwrite`, `upsert
  into ... by` and `delete from` are the four verbs; `append to`, `overwrite`,
  `replace into` and `merge into ... on` read as them and print canonically.
  A location is a quoted URL or a catalog path whose parts may be quoted with
  `"`, backticks or `[...]`; a target carries `with (...)` properties, and
  `Holder::from_url(url, properties)` is the one builder every target and every
  `IOBase`/`IOMedia` from a URL goes through. `Plan::execute` reads the source
  with the read sections pushed into the media, orders and slices what comes
  back, and writes where the plan says; a plan with no source starts from the
  empty stream, so `create` alone writes a schema and `delete` reads its target.
- Record options are the split sections of that plan, stored apart for
  isolation - `field`, `filter`, `selector`, `merge_by`, `name`, the row bounds
  - with `plan()`/`set_plan` composing and splitting; `dtype`, `metadata`,
  `select_by_names`, `merge_by_names`, `filter_partitions`, `partition_filter`
  and `partition_predicate` are gone, and a media reads `partition_pairs`,
  `apply_columns` and `apply_arrow_expressions` instead.
- Simplification is exact under three-valued logic and reaches a fixed point:
  `a = 1 or a = 2` is `a in (1, 2)`, a self alias is dropped, a same-type cast
  is skipped; binding settles constant subtrees and orders `and` cheapest-first.
- The best-effort rule, now an AGENTS.md invariant: a constant coerces into the
  operand it meets (text parsed as the column's own type), operands with no
  common type compare as text, a declared column casts safely unless it is `not
  null`, a missing store reads as the empty stream, and an error names what
  could not be done. DuckDB's SQL and Python expression API are the reference
  for spellings and aliases; the crate's abstractions carry the behaviour.
- Bindings: Python `Term`, `Bound`, `Filter`, `Selector`, `BoundSelector`,
  `Plan`, `Expression`, `Records`, `Bounds`, with `field`/`filter`/`selector`/
  `merge_by`/`plan`/`partition_pairs` on the options and `merge_by` on the
  Iceberg table; JavaScript the same classes in camelCase, the loader widening
  `bind` parameters, the three Arrow holders, and `applyRecords` rows, and
  `Records` iterating as JavaScript does.

Pins: the corpus round trip of every term, clause, plan and sequence spelling;
the two tiers agreeing on every operator; every simplification answering what
the original answered; a declared `not null` column refused by name; a field
round-tripping through a selector; a plan creating, inserting, upserting,
deleting and reading one store; the sliced reader crossing batch boundaries as
views; a holder built from a URL with properties; the split option sections
composing into one plan and back.

## 37. Scalar is the boundary, options take properties, and the function set has one door

Everything a binding hands the expression layer crosses as one `Scalar`:
`Scalar.from_` in Python and `Scalar.from` in JavaScript read the host value,
and `Term`, `Filter`, `Selector`, `Projection`, `Plan` and `Expression` read
that scalar with `from_scalar` - text parses, a sequence is projections or
conditions, a mapping is aliases, null is the identity, anything else the
literal it is - so a list of names, a dict of aliases and the text of a clause
mean the same thing in every language and no binding re-implements a parser.
Record options expose the same door as `set_*_scalar` / `with_*_scalar`. A
columnar host object - a pyarrow container, a pandas or polars frame, a numpy
array, an Arrow C exporter - lands as `Scalar::Arrow(ArrowScalar)` with its
buffers shared rather than its rows walked; `ArrowValue` is renamed
`ArrowScalar`, `read_arrow` / `write_arrow` take record options, and
`write_arrow` redirects each shape to the primitive that takes it as it stands.

Every record read and write takes `options` and, beside it, the option
properties by name - `**properties` in Python, a plain object in JavaScript -
set on a copy by their own setters; `...` and `undefined` mean not given and
are skipped, `None` and `null` clear. The `select` section is spelled `select`
on the options, as its clause is, and `apply_datatype` is the schema question
`apply_field` is derived from.

The closed function set keeps its promise by taking one door rather than a
registry inside the grammar: `namespace.name(...)` is a user-defined function,
registered once per process with a `FunctionSignature` that is itself a struct
`Field` - one child per parameter, a `function:default` making a parameter
optional, the return as `function:returns` - typed and called by the scalar
and vectorized tiers through that signature, and unknown to the statistics
tier, so nothing is ever pruned by it. A call over plain columns is stored as
a column's `transform:function` and `transform:sources`, the shape a partition
spec already has; any other term stays `transform:expression`. Python
registers a callable with `@user_defined_function` / `@user_defined_filter`,
row-wise under one interpreter attachment per batch or `vectorized` over
pyarrow arrays; JavaScript parses the spelling and refuses it by name at bind.

A `where` that names a column only the `select` publishes runs after the
projection, as DuckDB lets a `where` read an alias; every other `where` runs
first, where it prunes. The text reader's location column is `sourceurl`.

Pins: a selector, a filter and a plan read from text, a list, a dict, null and
a literal answering the same value in every language; a columnar object
crossing `Scalar.from_` as an Arrow scalar with its field; a property beside
`options` landing on a copy and an Ellipsis skipped; the two tiers agreeing on
a user function, its defaults filled and its null rule kept; a signature
round-tripping through its field; a stored call reading back as its function
and sources; an unregistered name refused where it is typed; a text read
shaped by a select over the row header's captures and a where over an alias.

## 38. `prevupdatedat`, `timepartition`, and the rules a registry carries

Settled with the user's request to rename two crate columns and to keep the
enriching pass while it moves out of Rust into the registry. Each part of
this entry is appended by the change that lands it; the first two are below.

### The previous clock is `prevupdatedat`

**Rule.** Tag 65021 is `prevupdatedat` (`PrevUpdatedAt`), never
`prevtimestamp`: the value it carries is the previous message's `updatedat`
(decision 26 renamed the clock and this follows it). Same datatype, same
position, same nullability, same stamps; no alias, no second spelling
anywhere - Rust constants, Python, JavaScript, the CLI, the docs, the
inventories and the equivalence snapshot all read the new name.

### The partition is an hour, typed as the clock it is cut from

**Rule.** Tag 65004 is `timepartition` (`TimePartition`): `updatedat`
floored to the hour, as `DateTime64(ns, UTC)` - the exact layout every FIX
clock has - rather than the whole seconds `unixpartition` held. A partition
value is compared and ranged over, and an instant floored to its hour ranges
exactly as the clock it was cut from, reads as a date in every catalog, and
needs no arithmetic to compare against a clock.
`FixMsg::time_partition()` answers it (Python `time_partition`, JavaScript
`timePartition`); `DEFAULT_PARTITION_SECONDS` stays the one width, an hour.

The crate field declares what it is in the protocols every catalog reads:
`field:partition` marks it as the column a layout is cut on, so an Iceberg
spec built from the fixed schema partitions by identity on it and prunes on
its bounds; `partition:sources = ["updatedat"]` names what it reads; and its
derivation is the expression layer's own `transform:expression =
"truncate(updatedat, 'hour')"`, so `Field::apply_arrow_batch` fills a batch
that lacks the column with exactly what `time_partition` answers for a row.
The Iceberg `truncate[3600]` declaration goes with the integer it described.

**What moves.** The equivalence snapshot moves on every row's
`timepartition` (an instant where an integer was) and on every row's `uuid`:
the message identity digests the named row content (decision 26), the column
is part of that content, and both its name and its value changed. Nothing
else moves - entries, wire, digests and every other column are byte for
byte what they were.

**Written in:** `fix/crated.rs`, `fix/schema.rs`, `fix/lifecycle.rs`, both
bindings, `docs/fix/*`, `DECISIONS.md` 24 and 26 restated under the new
names.
**Fixtures:** `rust/tests/fix/schema.rs` - the fixed schema partitions by
identity on `timepartition`, a batch missing the column is filled with the
hour floor of `updatedat`, a pre-epoch clock lands in the hour that contains
it; the regenerated equivalence snapshot with the two moved keys named.

### The corpus

**Rule.** `rust/tests/fix/ulbridge.log` is the one capture every FIX suite
and benchmark reads, and it now ends on fifteen more lines of the bridge's
own, transcribed from its log and anonymized the way the rest was
(`FIRM.*`, `FIRMA9120`, `ULMSG_DMZ_FIRM`, `ULB_FIRM`, `FIRMB`, `FIRMAPRD`,
`OMSVENDOR`, `trader3`, `0102TRADER3`; instruments, venues and vendor
session names kept; `9=`, `212=` and `10=` recomputed over the `|`
separator so every frame is consistent): a cancel request (`35=F`), its
cancel reject (`35=9`, `434=1`, `39=8`, `58=`), the reject routed, enriched
with five regulatory clocks and forwarded as a bridge row - `#`-marked keys,
ten packed parties, two alternate identifiers, `FIRM.`, `OMSVENDOR.` and
`ULLINK.` composed keys, once in parentheses after prose and once bare - the
`35=UL` frame the row goes out in with an exact `212=`, and two Fidessa
heartbeats, one saying `43=N`. The corpus is 144 lines and 95 messages;
three of the new lines are prose and carry none.

**What the lines pin, as the codec reads them today.** The frames are typed
by their codes and the bridge rows by the spelling they call themselves,
`cancelreject`; `ORDSTATUS=rejected` and `39=8` both read as the state
`8`, on the message and in the crate's `state` column; a `#`-marked key
with no bare twin is the key it marks, a marked twin restating the bare
bytes is dropped, and `#SYMBOL=TW0002454006` beside `SYMBOL=2454` stays
its own verbatim key. The ten parties replicate into `parties`, each role
typed where FIX spells the bridge's word and null where it does not
(`orderoriginatorsystem`, `buyside`), and `party` answers a role one party
bears and none a role two bear. `#NOTRDREGTIMESTAMPS=4` over five indexed
occurrences is a real bridge anomaly kept verbatim: the row holds the five
occurrences, the counter holds the stated four, never renumbered, and
`anomalies()` reports `Miscounted { 768, "trdregtimestamps", 4, 5 }`.
`OMSVENDOR.ORDERQTY` fills `OrderQty` and `OMSVENDOR.TIMEINFORCE` fills
`TimeInForce` (decision 20), while `FIRM.ACRONYM` and `ULLINK.INSTRUMENTID`
name no field and stay under their namespaces. The prose-wrapped row and
the bare row read as one message with one digest; two sessions' heartbeats
digest equal, because a digest reads past the header and a heartbeat is
nothing else. The batch door agrees with the line door on every tag of the
new lines, and `LIFTED_OUT_OF_A_DOCUMENT` stays the two pairs it was.

**What moves.** Every pinned count: 129 lines to 144 and 83 messages to 95
in `rust/tests/fix/dataset.rs` (its `SILENT` list gains lines 136, 137 and
139), `lifecycle.rs`, `lifecycle_previous.rs`, `rust/tests/iobase_calls.rs`
and the pipeline benchmark's `95 * REPEATS`; the live-chain counts to five
direct and four enriched, because the cancel request opens a chain under
the instrument its `22=4|48=` names and the reject, naming the order but no
instrument, scopes its keys under nothing and never meets it; the
equivalence snapshot gains the keys `ulbridge[129]` to `ulbridge[143]` and
no existing key moves. The codec needed no change.

**Written in:** `rust/tests/fix/ulbridge.log`, `dataset.rs`, `lifecycle.rs`,
`lifecycle_previous.rs`, `rust/tests/iobase_calls.rs`,
`rust/benchmarks/fix/pipeline.rs`, `docs/fix/capture.md`, `decode.md`,
`arrow.md`.
