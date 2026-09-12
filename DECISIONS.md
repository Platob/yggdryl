# The decisions the TextLine adaptation rests on

Twenty-four decisions and three amendments, settled while the FIX layer was
moved onto the text reader that landed in `main`, then made text, then given
one namespace, then met the charset layer, and then - decisions 13 to 24 - had
its messages folded into its components, its direction made FIX's, its rows
read for every message they carry, its bridge configuration read as a plugin,
its two passes made one, and its identities made UUIDs. Each is one rule, and
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
code - which is the part most worth keeping. Decisions 13 to 24 were settled
one per commit from `FIX_DIRECTION_MESSAGES_PROMPT.md`, each written before
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
