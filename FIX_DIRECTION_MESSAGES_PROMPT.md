# FIX: direction, components, many messages a row, plugins, one enriching pass, identifiers and uuids

Twelve pieces of work from `main` (PR #107 merged at `aa6566f6`, four gates
green there; the corpus anonymized at `ab124516`), on one new branch, each a
design change with its
own decision in `DECISIONS.md` written before the code that keeps it, in this
order because each one moves what the next one reads:

1. The registry's messages fold into its components: one type map, simple ->
   field, struct -> component, list -> group.
2. `msgdirection` leaves the general datatype vocabulary and becomes FIX's,
   owned by the registry as `MsgDirection` on tag 385.
3. A direction is read off a row body by configurable regex rules the
   registry holds, not by a hand table in the text layer.
4. `parse_line` answers an iterator of messages: a row yields none, one or
   many.
5. A ULBridge configuration document is read without its envelope, one
   flat message per plugin, and any other JSON is skipped; the `ULCONFIG`
   media type goes.
6. The `ulconfig` vocabulary is renamed `plugin`: a plugin is a FIX session
   endpoint as the bridge hosting it reports it, and its handling is FIX's,
   generic over the bridge.
7. The plugin-configuration message type and its struct are registered
   automatically, and the enriching iterator remembers each configuration it
   meets by plugin id and fills the `SenderCompID`/`TargetCompID` a later
   message on that plugin left blank.
8. `into_latest` goes; `enrich` is the one pass that restates a message
   under the latest definitions, keeps every alias spelling beside the
   canonical value so the entries are the most complete they can be, and
   then fills what the message implied.
9. A component names its identifiers (`fix:identifiers`, derived from FIX's
   own families), and a crate field `altids` carries every identifier a
   message stated, filled by `enrich`.
10. `id`, `persistentid`, `instid` become `uuid`, `puuid`, `instuuid`, typed
    `uuid` and shaped as RFC 9562 UUIDs.
11. The lifecycle keeps its alive events by `puuid`, scopes their
    identifiers by `instuuid`, and corrects a `puuid` that `altids` proves
    belongs to an alive chain.
12. Two crate fields, `prevtimestamp` and `prevuuid`, name the previous
    message of the chain.

Read `CHARSET_READ_PATH_PROMPT.md` (the call/allocation work on the same
doors) afterwards, not before: pieces 4, 6, 8, 9 and 11 move the codec's
entry points, their names and the per-row stages it measures, so its
baselines are taken again once this lands.

## Outcome

- `FixCategory` has three variants; the shipped dictionary has no
  `messages/` directory; `msgtypes()` answers the components that carry
  `fix:msgtype`; every count pin, provenance checksum and the explorer say the
  new shape.
- `DataType::MsgDirection` does not exist; `yggdryl::fix::MsgDirection` does,
  registered on tag 385 with FIX's own code set and the regex rules that read
  it, defaulting to today's verbs; the text reader has no `direction`
  column, no `parse_direction`, no marker stripping.
- `FixCodec::parse_line`, `parse_text_line`, `parse_plugin_line` answer
  `FixMessages` that can be empty or long: a prose line yields nothing, a
  two-frame line two messages, a wildcard configuration read one per plugin.
- No message carries `MBean`, `Operation`, `Status` or `Error`; nothing
  public spells `ulconfig` or `UlPlugin` (`Plugin`, `Plugins`,
  `parse_plugin_line`, `fix/plugin`, dialect `plugin`); a configuration
  message is typed by a registered message type and its struct is a
  registered component; a JSON body that is not a Jolokia `request`+`value`
  answer yields nothing.
- `FixMsg::into_latest` does not exist; `FixCodec::enrich_message(s)` restate,
  duplicate under aliases, remember plugin configurations and fill, in that
  order, and the equivalence snapshot's `enrich` group pins the result.
- Every component carries `fix:identifiers` and every enriched message
  `altids`; `uuid`, `puuid`, `instuuid` are `uuid`-typed RFC 9562 values; the
  lifecycle keys its chains by `puuid`, scopes identifiers by `instuuid`,
  corrects a `puuid` through `altids` and stamps `prevtimestamp`/`prevuuid`;
  the crate has twenty-three fields and every count pin says so.
- The screenshot lines below are in `rust/tests/fix/ulbridge.log` and in the
  snapshot, and every number that moved in the snapshot is listed in the
  commit that regenerated it.

## Read first

- `AGENTS.md` in full; Gate 1 is §2 with the four MSRV checks
  (`cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl`
  at `--all-targets`, `--no-default-features --lib`,
  `--no-default-features --features object --lib`, and
  `cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets --features iceberg`).
  The crate denies `unsafe_code`. `.api-inventory.txt` and
  `.api-bindings.txt` are hand-maintained and move in the same commit as the
  name they list.
- `DECISIONS.md`: twelve decisions and three amendments. Decision 3 (a
  message is the frame; "a line carrying two frames reads as the first" is
  the sentence piece 4 amends), decision 8 (per-row facts are captures
  resolved once, applied as `RowExtras`), decision 10 (a line is text),
  decision 11 (one namespace; messages' codes in one map; the store layout;
  the fold table), decision 12 (no charset past the reader's door).
- `rust/tests/fix/equivalence.snapshot`: 264 messages over 113 lines of
  `rust/tests/fix/ulbridge.log` plus the fixture groups, 11 775 lines pinning
  `.type`, `.digest`, `.wire`, every `.entry[i]` and `.field.<column>`, and
  `.messages` per line (262 lines answer 1, `ulbridge[098]` answers 2).
  Regenerated only under `YGGDRYL_FIX_EQUIVALENCE_WRITE=1`. Pieces 2, 4, 5,
  7 and 8 each move it on purpose (6 moves no line of it); the rule is that
  a regeneration ships in the
  commit of the decision that moved it, with the moved lines named in the
  commit message, and never to make an unrelated test pass. The `unread`
  assertion (`equivalence.rs:620-629`) names `frames[028]`, `frames[031]`,
  `verbatim[028]`, `verbatim[031]` by index, so fixtures may only be appended
  to `frames()`.
- `docs/fix/lifecycle.md` (the Contract table, `:5-20`) and the crate's own
  columns (`docs/fix/capture.md:360-386`): pieces 9 to 12 amend both.
- The code each piece moves is named under it.

## 1. Messages are components

**Today.** `rust/src/fix_category.rs:21-36` - `FixCategory { Fields,
Messages, Components, Groups }`, `ALL` in that order, `as_str` the storage
folder names, `FromStr` refusing with "fields, messages, components, or
groups" (the same text again in `store.rs:369`). A message is a Struct
`Field` whose only difference from a component is the `fix:msgtype` marker:
`catalog.rs:361-372` (`definition_category`), `msgtype.rs:74-107` (`MsgType`
wraps a non-null Struct that must carry `fix:msgtype`). The catalog is one
`Vec<Definition{category, DefinitionField::Plain|Group|Message}>` keyed by
`(category, folded-name digest)` (`catalog.rs:83-96`), sorted by `(category,
name)` (`:292-296`), partitioned per category for iteration (`:116-135`);
message codes live in one map (`:154-248`, decision 11); `register_msgtype`
(`crated.rs:543-630`) creates an empty message in `Messages` when no message
owns a code. The store writes `<category>/<name>.json` with `LOAD_ORDER
[Fields, Components, Groups, Messages]` (`store.rs:16-22`, `:599-659`); the
snapshot has exactly four keys (`:357-409`); `merge_catalog` folds
`[Components, Groups, Messages]` (`catalog.rs:1287-1311`); `Hash` walks
`[Messages, Components, Groups]` (`registry.rs:1496-1513`); the generator
assigns derived tags walking components, groups, messages
(`scripts/generate_fix_dictionary.py:1449-1482`). The shipped dictionary:
6 241 fields in 65 shards, 181 messages, 747 components (580 of them group
occurrences), 580 groups; every message carries `fix:msgtype`, no component
does. Counts are pinned in `rust/tests/fix/store.rs:238-241`,
`rust/src/fix/tests.rs:4509-4512`, `python/tests/fix/test_fix.py:319`,
`node/tests/fix/fix.test.js:256,757,944,950`, `docs/fix/store.md:199`,
`docs/assets/fix.json` (181 `fix:msgtype`), `config/fix/provenance.json`
(a sha256 per document) and the byte-identical write-back in
`rust/tests/fix/dictionary.rs:282`. The CLI has a `messages` subcommand tree
(`cli/src/fix.rs:95-130`); Python and Node take the category as a string
(`python/src/fix.rs:396-481`, `node/src/fix.rs:175-291`); the `iobase_calls`
store pin counts four category roots (`child_by_path=5` on write, `=4` on
load, `rust/tests/iobase_calls.rs:46-77`).

**Rule to write (decision 13).** `FixCategory { Fields, Components, Groups }`.
A message is a component that carries `fix:msgtype`; `MsgType` still wraps
one, `msgtypes()` iterates the components that carry the marker,
`register_msgtype` creates its empty struct in `Components`, and the
message-code map is unchanged. The type map is the whole rule: a scalar
datatype is a field, a `Struct` is a component, a `List`/`LargeList` of a
non-null `Struct` is a group; `definition_category` answers by shape alone
and `check_shape` per category. The store writes `components/<name>.json`
for messages too, `LOAD_ORDER [Fields, Components, Groups]`, the snapshot has
three keys and refuses `messages` by name. The generator writes the 181
message documents into `components/` unchanged in content - a moved file
keeps its stated `fix:tag`, so no derived tag moves and provenance changes
only by path - and `--check` verifies the new layout. Decide and pin the one
collision the fold can create: a message and a component with one folded
name (census both sets first; if any pair exists, the generator's existing
`Message` suffix is the answer and the census is the fixture).

**What moves.** `fix_category.rs`, `catalog.rs` (partition, indexes,
`merge_catalog` order, `definition_category`), `store.rs` (layout, snapshot
keys, `LOAD_ORDER`), `registry.rs` (`Hash` walk, `get_field_by_path` roots),
`crated.rs` (`register_msgtype`), `cfb.rs:539-563,2126-2197` (CFB produces
messages as msgtype-marked components), `scripts/generate_fix_dictionary.py`
(render into `components/`, `--check`), `config/fix/messages/*` -> `config/fix/components/`,
`provenance.json`, `scripts/build_docs_fix.js:219-262` (KPIs: count messages
as components with `fix:msgtype`), `docs/assets/fix.json`, the CLI trees and
`ingest` walk (`cli/src/fix.rs:385-405`), `cli/tests/fix.rs:105-200`, the
bindings' category strings and every test that spells `"messages"`
(`python/tests/fix/test_catalog.py`, `node/tests/fix/catalog.test.js`,
`typing_bindings.py:1393-1412`, `fix.types.ts:373-382`), `.api-inventory.txt:822-854`,
`.api-bindings.txt:87-88`, docs (`docs/fix/index.md:39,364-421`,
`registry.md` Use/lookup table/Insert-update-remove/Registering a message
type, `store.md:10-19,158-174,199,377`, `cli.md`, `explorer.md`), the store
call pin (`child_by_path=4` on write, `=3` on load), `stable_hash` of every
registry (the walk order changes: say so, pin the new value).

## 2. `msgdirection` is FIX's, and the registry owns it

**Today.** `DataType::MsgDirection` (`rust/src/types/dtype.rs:124`, "transport
rather than FIX") is `DataTypeId::MsgDirection`, discriminant 58 of the
61-entry `ALL` (`datatype_id.rs:165-226`) - a wire contract, since
`Scalar::write_bytes` tags every value with it (`:310-316`); kind `Code`,
width 4, name `msgdirection`; `Code::MsgDirection` in the `#[non_exhaustive]`
`Code` enum pinned at 32 bytes (`types/string/code.rs:827-834`); the value
type `MsgDirection` with `SENT`/`RECV`, `from_spelling`, `infer_bytes`,
`split_bytes`, `at_payload` and the 13-row `VERBS` marker table
(`code.rs:481-795`); `StringEnum::DIRECTIONS`, `DataType::CODES`,
`LOGICAL_NAMES`, the serde tag, `MsgDirectionType`/`MsgDirectionField`, the
Arrow extension `yggdryl.msgdirection` (and the legacy `yggdryl.direction`
import, `codes.rs:264-269`), `dtype_rank 54`. The text layer carries it:
`TextLine::direction` (`line.rs:37,214-232`), `TextOptions::parse_direction`
(`options.rs:138-140`), the `direction` column typed `MsgDirection`
(`plan.rs:116-123`), the marker stripped after the strips (`arrow.rs:773-782`),
`batch.rs:140-144,404-416`. In the FIX layer the home is FIX's own tag 385
(`MSGDIRECTION_TAG_NAME`, `crated.rs:137-139`, appended by `fix_schema_tags`),
typed `msgdirection` in the shipped dictionary since 5.0.2 with FIX's own
code set `R = Receive`, `S = Send` (`config/fix/fields/3.json:1155-1166`) - a
vocabulary disjoint from the column's `SENT`/`RECV`; `FixCodec::direction`/
`with_direction` (a pin defaulting to `SENT`), `CaptureRole::Direction`
(ignored on the line door, `codec.rs:787-791`), and the batch reader's
precedence: the row's `direction` column, else the verb in front of the
payload, else the pin (`batch.rs:634-643`). Bindings: `FixCodec(direction=)`
crossing `from_spelling`, `MimeType.infer_bytes_direction`,
`TextLine.direction`, `DataType("msgdirection")`, `types.msgdirection()`;
`parse_direction` was never bound. Pins: `ALL.len() == 61` and every
discriminant by index (`datatype_id.rs:615,672-716`); `DIRECTIONS ==
["RECV","SENT"]` with no `UNKNOWN`; `fix_schema_tags()` ending in 385
(`test_fix.py:1690-1694`, `fix.test.js:1335-1339`); the marker table cases
(`rust/tests/types/datatype/coded.rs:341-392`); the batch precedence
(`rust/tests/fix/pipeline.rs:373-377`, `capture.rs:376-379`,
`batch.rs:147-172`); the snapshot carries no `.field.msgdirection` (the line
door never fills 385).

**Rule to write (decision 14).** A direction is a FIX fact and nothing else
has one. `DataType::MsgDirection`, `Code::MsgDirection`, the value type in
`types/string`, `DIRECTIONS`, the `msgdirection` code name, the typed field,
the Arrow extension, the text reader's `direction` column, `parse_direction`,
`TextLine::direction` and the marker stripping are deleted. Tag 385 is typed
by the dictionary as it types any coded field - a `state`-like code column
over FIX's own code set - and `yggdryl::fix::MsgDirection` is the registry's
reading of it: the code set on tag 385 (`R`/`S` from the specification,
extendable like any code set) beside the rules of piece 3, held by
`FixRegistry`, answered by `registry.msgdirection()` and compiled once by
`FixCodec::with_registry`. A message carries its direction as tag 385 with
the code the rules matched (`S` or `R`), filled on every door - the line door
too - and the batch reader's precedence becomes: a stated 385 in the row,
else the rules over the row's prefix, else the codec's pin, which is now a
code of the set. The `DataTypeId` discriminant 58 is retired, not reused:
`ALL` shrinks to 60 and the pin test states every byte explicitly (the
index-equals-byte rule ends here; say so in the doc of `as_u8`). The Arrow
extension name `yggdryl.msgdirection` stops being written; a stored column
carrying it or `yggdryl.direction` imports as the `fixed_size_binary(4)` it
is (say so in `docs/types/text.md` edges). The bindings lose the datatype and
the `direction=` spelling table; `FixCodec(direction=)` takes a code of the
set; `TextLine.direction` goes.

**What moves.** Every surface the explorer listed for `MsgDirection` (the
`types/` files above, `mime_type/line.rs:1093-1129` keeps `payload` for
piece 3), `media/text/{line,options,plan,arrow,batch}.rs`, `fix/{crated,
schema,build,codec,batch,ulbridge}.rs`, `python/src/{fix,enums,text_line,
types/*}.rs`, `python/yggdryl/types/codes.py`, `node/src/{fix,enums,text_line,
types/*}.rs`, `node/fields.js`, `config/fix/fields/3.json` (tag 385's dtype
under the generator's `CODED_TAGS`, `scripts/generate_fix_dictionary.py:153-157`),
docs (`docs/types/codes.md` "Which way a line moved", `docs/fix/registry.md:893,938-940`,
`docs/fix/arrow.md`, `docs/fix/capture.md`, `docs/media/text.md`,
`docs/fix/encode.md:81`), both inventories, the tests named above. The
equivalence snapshot gains `.field.msgdirection` on every line whose prefix
carries a verb (the bridge's `recv <<`, `sent >>`), which is the first
deliberate regeneration.

## 3. Direction rules are configuration

**Today.** `VERBS` is a `const` of 13 `(bytes, direction, bare)` rows read in
the prose in front of the payload (`payload(line)` from
`mime_type/line.rs:1117`), with `opens_marker`/`closes_marker`/`bare_marker`
deciding word boundaries and the bracketed bare forms `[OUT]`/`(in)`; a
prefix carrying both verbs or neither answers nothing; a ULCONFIG document
states its own half (an echoed `request` key answers `RECV`, `code.rs:632-638`).

**Rule to write (decision 15).** The rules are the registry's. `MsgDirection`
holds, per code of tag 385's set, an ordered list of regex rules applied to
the row's prefix (the bytes before the payload, exactly the bound `payload`
answers today); the first rule that matches names the code; two codes
matching answers nothing, as today. The default rules reproduce the verb
table and its bracket forms as three or four `regex::bytes` patterns per code
(`(?i)(^|[\s\[\(<])(sending|sent|send|outbound|outgoing)([\s\]\)>:,]|$)` and
the bare `(^|[\[\(])(out)([\]\):]|$)` shape), so every case in
`rust/tests/types/datatype/coded.rs:341-392` moves to `rust/tests/fix/` and
keeps its answer. They are stored as the tag-385 field's `fix:directions`
metadata (a JSON list of `{code, patterns}`), so a dictionary ships them, a
registry merges them like any `fix:` property, `ygg fix fields update 385`
edits them, and the bindings read and write them as a list. The codec
compiles them once (`with_registry`); nothing per row builds a regex. The
ULCONFIG statement rule (piece 5 removes the document's envelope, and with it
the echoed `request` key) becomes one more default rule on the prefix, not a
special case. Pin: the moved marker cases; a rule added through the registry
changing what a line answers; a dictionary without the property answering
the defaults; a prefix matching two codes answering nothing.

## 4. A row yields none, one or many messages

**Today.** `parse_line(&self, row: &[u8]) -> Result<FixMessages>`
(`codec.rs:701-703`) and `parse_text_line` (`:773-822`) already answer
`FixMessages`, but `FixMessages` (`messages.rs:6-59`) has three sources -
`Empty`, `One(Option<Result<FixMsg>>)`, `Configs` - and only a ULBridge
configuration document yields more than one. `parse_page_with`
(`:876-910`) dispatches on one scan: a tag-keyed first entry -> `frame_with`
-> one; an XML document -> `fixml_with` -> one; `{`/`[` -> `ulconfig_with`;
else `bridge_with` -> one - so a prose line with no pairs builds a message
named `unknown` with no entries (`rust/tests/fix/codec.rs:60-67`), and the
corpus's `URI:`, `Path-Info:`, `Execution time:` lines each pin `.messages 1`
of type `unknown`. A second frame is unreachable: `locate_frame`
(`mime_type/line.rs:218-231`) returns at the first `8=`, and
`frame_arrivals` (`codec.rs:1788-1832`) breaks at the first unmarked `10=`
and drops the rest; a checksum-less first frame absorbs the tail as duplicate
tags; a bridge row absorbs a trailing numeric frame (`:989-993`). Decision 3
promised a two-frame fixture and none exists. `parse_fix_line`,
`parse_ullink_line`, `parse_fixml_line`, `parse_pairs` answer one `FixMsg`.
Cardinality pins: `rust/tests/fix.rs:134-176` (`one_line`, "a singleton
fixture yields exactly one message", used across `codec.rs`, `lift.rs:20-257`,
`pipeline.rs`), a-row-in-is-a-row-out (`batch.rs:257-273`, Python
`test_batch.py:235-237`, Node `batch.test.js:280-283`), `dataset.rs:40-55`
(`ROWS = LINES + 1`), the bench cardinality guards (`node/benchmarks/fix.js:203`,
`python/benchmarks/fix.py:265`), byte-in-byte-out per line
(`batch.rs:726-755`), the docs examples (`docs/fix/decode.md:29-57`,
`capture.md:35-37`, `lifecycle.md:48-114`, `registry.md:707-756`).

**Rule to write (decision 16, amending decision 3).** A row is read for every
message it carries. A frame ends at its checksum, and the pairs after it
begin the next message where they open a frame (an unmarked `8=`, else the
`35=` rule `locate_frame` already applies); a checksum-less frame ends where
the next frame opens, and only the tail of the last frame is its own. A
bridge row (named keys) is one message and a numeric frame behind it is a
second. A row that opens no frame, states no bridge pair and carries no
document yields nothing: `unknown` names a frame without `35=`, never a line
without a frame. `FixMessages` gains a lazy source over the page that
re-enters `frame_with` at the next frame's offset (no collection - the
inventory pins "no collection of ULconfig results", keep it); an `Err` still
fuses. Every message of one row shares the row's `RowExtras` and direction
(decision 8; `ulconfig_with`'s `RowStamp::retained` is the pattern). The
single-dialect doors (`parse_fix_line`, `parse_ullink_line`, `parse_fixml_line`,
`parse_pairs`) keep answering one message and refuse a body holding two
frames by name ("expected one frame, got a second at byte N"): they are the
byte doors a caller uses when it holds one frame. The batch reader expands
as it expands MBeans today (carried columns repeated, the byte charge on
index 0), `write_arrow_reader` writes one line per message (a two-frame
source line comes back as two lines - say so in `docs/fix/arrow.md`), the
classifier stays first-frame (its `mimetype`/`msgtype` columns describe the
line's first frame; say so in `docs/media/text.md`). The bench guards count
messages, not lines. Pin: two frames on one line (both messages, each
bounded, both re-emitting their own bytes); a bridge row then a frame; a
checksum-less frame then a frame; a prose line answering nothing through
every door; the corpus lines whose `.messages` moves from 1 to 0 listed in
the regeneration commit; `one_line` renamed to what it asserts and kept.

## 5. A configuration document without its envelope, and JSON that is skipped

**Today.** A line holding the ULBridge namespace is classified
`text/ulconfig` (`mime_type.rs:190,449`, `mime_type/line.rs:94-156,1138-1165`);
`UlPlugin::into_fixmsg` (`ulbridge.rs:838-870`) writes the envelope first -
`MBean` (20001), `Operation` (20002), `Status` (20003), `Error` (20004) from
`request.mbean`, `request.type`, `status`, `error` - then the attributes, each
under its tag from the table at `ulbridge.rs:180-430` (20010 `SessionInterface`
… 20047), with FIX's own `SenderCompID`, `TargetCompID`, `BeginString` on 49,
56, 8. `UlPlugins` (`:982-1075`) walks a single read (one plugin), a wildcard
read (one per ObjectName under `value`), a bulk array, and an error-only
answer, which "still yields its envelope" as a message. A JSON body that is
none of these is an `Error::Parse` from `parse_ulconfig_line` (an `Err` item
through `parse_line`, one empty message through `parse_text_line`). The
corpus (`ulbridge.log:94-100`) holds one exchange - `URI`, `Path-Info`,
`Request`, `Execution time`, a single-plugin `Response`, a wildcard
`Response` of two `ConfigurationPlugin`s, an error `Response` for
`Missing_Plugin` - pinned as `unknown`-typed messages (`snapshot:8218-8367`).

**Rule to write (decision 17).** The envelope is transport. `MBean`,
`Operation`, `Status`, `Error` are deleted with tags 20001-20004; the plugin's
ObjectName stays the `SessionInterface` attribute (20010); a message is the
plugin's attributes and nothing the Jolokia answer wrapped them in. A read
that answers no plugin - an error, an empty wildcard - yields nothing. A JSON
body that is not a Jolokia `request`+`value` answer whose `value` holds
ULBridge ObjectNames yields nothing, through every door, without an error:
the body is the row's and the row said nothing FIX can read. `MimeType::ULCONFIG`
is deleted; the classifier answers `application/json` for a JSON body and
the codec recognizes the answer's shape itself (one probe, the
`request`/`value` keys and the namespace, at the offset `ulconfig_span`
already finds). `UlPlugin`/`UlPlugins` stay the public reading of a document
(`from_json_bytes`), minus their envelope, until piece 6 renames them. Pin:
the corpus exchange (the
four prose lines yield nothing; the single read one message with no
20001-20004; the wildcard read two; the error read nothing); a bare
`{"a":1}` body yielding nothing; the byte-for-byte re-emission of a
configuration message unchanged (the entries are what arrived).

## 6. `ulconfig` is spelled `plugin`

**Today.** The reading of a configuration document is spelled after the
product that wrote it: `MimeType::ULCONFIG` (`text/ulconfig`; gone with
piece 5), `UlPlugin`/`UlPlugins` (`ulbridge.rs:638,982`, re-exported at the
crate root with `ULBRIDGE_DIALECT`, `ULBRIDGE_ROWHEADER`, `ULBRIDGE_TAG_MIN`,
`fix_ulbridge_fields` and the four envelope tag names, `fix/mod.rs:196-199`),
`FixCodec::parse_ulconfig_line` (`ulbridge.rs:1110`), `ulconfig_with`
(`:1121`), `FixMessages::from_ulconfigs` and `Source::Configs`
(`messages.rs:9,34`), the classifier's `read_ulconfig`, `ulconfig_at`,
`ulconfig_span`, `ulconfig_msgtype`, `ulconfig_answered`
(`mime_type/line.rs:148,1138,1153,1268,1334`), the bench group `fix/ulconfig`
(`rust/benchmarks/fix/ulconfig.rs:53`, `node/benchmarks/fix.js:337-342`,
`python/benchmarks/fix.py`), the pins
`parsed_ulconfig_wildcards_iterate_without_allocating_results` and the three
`... ULCONFIG` free probes (`rust/tests/allocations.rs:392-408`), the test
helper `one_ulconfig_line` (`rust/tests/fix.rs:136,171`) and the `ULCONFIG`
fixtures (`rust/tests/fix/capture.rs:41-50,258,421-430`, `dataset.rs:846`,
`pipeline.rs:355`, `rust/tests/types/enums/mime.rs:22,168,182`), the
bindings' `fix.UlPlugin`/`UlPlugins` and `parse_ulconfig_line`/`parseUlconfigLine`
(`python/src/fix.rs`, `python/yggdryl/fix.py:27-29,119-140`,
`_native.pyi:4823-4917`, `node/src/fix/ulplugin.rs`,
`node/index.d.ts:1232,4457-4516`, `test_media.py:28`, `media.test.js:24`),
the docs (`docs/fix/capture.md:112,923-941`, `docs/extensions/python.md:1514`,
`docs/extensions/javascript.md:846,1007`, `docs/benchmarks.md:16`,
`docs/media/text.md:167`, `docs/fix/registry.md:901`),
`.api-inventory.txt:697,800-820,924`, `.api-bindings.txt:82-121,315-360`.
The attribute fields 20010-20047 carry `fix:dialect` `ulbridge`
(`ULBRIDGE_DIALECT`) and are registered at runtime by
`fix_ulbridge_fields`/`with_ulbridge_fields`, not shipped in `config/fix`;
the row-header capture `ULBRIDGE_ROWHEADER` reads ULBridge's own log line.

**Rule to write (decision 18).** A plugin is a FIX session endpoint as the
bridge hosting it reports it - comp ids, begin string, hosts, ports, state,
sequence numbers - and its handling is FIX's, generic over the bridge:
ULBridge's Jolokia answer is one producer of it. Every name spelling
`ulconfig`, `UlPlugin` or `ULCONFIG` is spelled `plugin`/`Plugin`:
`yggdryl::fix::Plugin` and `Plugins`, `FixCodec::parse_plugin_line`,
`plugin_with`, `FixMessages::from_plugins` and `Source::Plugins`,
`plugin_at`/`plugin_span`/`plugin_msgtype`, the bench group `fix/plugin`,
the pins and probes `... PLUGIN`, `one_plugin_line`, `fix.Plugin`/`Plugins`,
`parse_plugin_line`/`parsePluginLine`, the docs' "ULconfig" wording, the
module file (`fix/plugin.rs` holds the reading; the row-header capture stays
with the bridge's line reading). The attribute fields become dialect
`plugin` (`PLUGIN_DIALECT`, `PLUGIN_TAG_MIN`, `fix_plugin_fields`,
`with_plugin_fields`); their tags 20010-20047 do not move.
`ULBRIDGE_ROWHEADER` keeps its name: it reads ULBridge's log line, which is
the product's. A rename is the whole commit: every door answers byte for
byte what it answered (the snapshot is unmoved), every pinned cost is
unmoved, and `git grep -i ulconfig` answers nothing but `DECISIONS.md`.

**What moves.** The files named above, both inventories, the `stable_hash`
of a registry carrying the attribute fields (the dialect text is hashed:
say so, pin the new value), and the docs that spell the membership
(`docs/fix/capture.md:676-745`, `docs/extensions/python.md:1500-1505`:
`dialects() == ["ulbridge"]` becomes `["plugin"]`).

## 7. The plugin configuration is a registered message, and the enricher remembers it

**Today.** ULBridge ingestion registers scalar fields only
(`fix_ulbridge_fields`, `with_ulbridge_fields`, `ulbridge.rs:478-503`;
`fix_plugin_fields`/`with_plugin_fields` after piece 6); a
configuration message is typed `unknown` (no `35=`); the classifier reads a
`type=Plugin`/`ConfigurationPlugin` off the ObjectName for its `msgtype`
column (`mime_type/line.rs:1268-1280`) but no message type is registered.
`enrich_messages` (`codec.rs:1392-1400`) is a stateless `map`. The row header
of a bridge log captures `pluginid` (`ULBRIDGE_ROWHEADER`, `ulbridge.rs:118`),
filled into the crate's `PluginId` field (65xxx, `crated.rs`) by
`CaptureRole::Fill`; a configuration message's own `Name` attribute (20012)
is that plugin's name; a FIX frame the same plugin logged (`[SmartTrade_OrderRouting] (DEBUG) Sending : 8=FIX.4.4|…|49=BRK.PROD.TRD|56=ST.PROD|…`)
carries 49/56 itself, and a bridge row (`RouteMessage : ACCOUNT=…`) does not.

**Rule to write (decision 19).** A plugin configuration is a message type of
the crate's own: `pluginconfig`, a component carrying `fix:msgtype` with a
user-defined code (FIX reserves `U*` for user messages; `UCFG`, decided in
the decision) whose members are the plugin attributes (20010-20047) and
FIX's 8, 49, 56, registered by `fix_plugin_fields`/`with_plugin_fields`
beside the fields - and, since a codec cannot write a shared registry, the
crate registers it wherever it registers its own fields (`fix_crate_fields`
gains it; every registry has it, as every registry has `pluginid`). A
configuration message carries `35=UCFG` as a built child and its `.type` is
`pluginconfig`. The enriching iterator is stateful: it remembers every
`pluginconfig` message it passes by `Name` (20012), and for every later
message whose `PluginId` capture folds equal to a remembered name it fills
`SenderCompID` (49) and `TargetCompID` (56) when absent, from the
configuration - never overwriting, as `enrich` never does - and `BeginString`
(8) the same way. A configuration seen later replaces the remembered one. A
message with no `PluginId`, or one no configuration named, is untouched. The
memory is the iterator's and dies with it; the batch reader's
`enrich_messages_arrow_reader` carries it across batches. Pin: the corpus
(the `SmartTrade_OrderRouting` heartbeat keeps its own 49/56; a bridge row on
`Virtu_TritonBlack_TradeCapture` gains none because no configuration named
that plugin in the log; add one that does, per the fixtures below, and pin
the fill); a configuration then a bare row on the same plugin; two
configurations for one plugin, the later winning; the fill never overwriting
a stated 49.

## 8. `enrich` is the one pass; `into_latest` goes

**Today.** `FixMsg::into_latest(self) -> Result<Self>` is `latest::restate`
(`msg.rs:810`, `latest.rs:899-935`): per level, canonicalize each child to a
registry field by tag, else name/alias, else the decimal the name spells;
merge children reaching one field into the most complete (canonical child
kept, a merged-away child dropped when null or equal, kept untouched when it
disagrees; `latest.rs:72-82,521-585`); apply `fix:replacements` in ascending
tag order (`:588-753`, `docs/fix/message.md:495-508`); stamp the crate
`version` (65001) with `registry.newest()`; row-only, entries untouched,
idempotent (`rust/tests/fix/latest.rs:367-394`). `enrich`
(`enrich.rs:367-638,769-812`) is a static RULES table walked once, never
overwriting, reading by tag (`get_by_tag`) - so a child stored under an
alias spelling with no tag is invisible to it until `into_latest`
canonicalizes it (`msg.rs:856-902`, `rust/tests/fix/latest.rs:108-131`);
it consults the registry only for a code set's group and never for lineage,
aliases or version. Aliases: `fix:aliases` (42 shipped fields, one
lineage-derived old spelling each, e.g. `lastqty` ← `lastshares`),
`set_lineage` rewriting them from the lineage (`field.rs:775-840`), the
registry answering an alias on lookup, the builder storing parsed children
under the canonical name. Bindings: `FixMsg.into_latest`/`intoLatest`
(cloning); `FixCodec.enrich_message(s)`, `enrich_messages_arrow_reader`; no
CLI. Docs: `docs/fix/message.md:490-560,677-687`,
`docs/fix/registry.md:626-669`, `docs/fix/capture.md:538-580`,
`docs/fix/arrow.md:494-600` (stages: enrich, then latest, then lifecycle).
The snapshot's `enrich` group (23 lines) pins `enrich` output; its `latest`
group (2 lines) never ran `into_latest`, so restatement is pinned only by
`rust/tests/fix/latest.rs` (883 lines) and the binding tests. Lifecycle reads
what enrich fills (`lifecycle.rs:343-370,395-411`).

**Rule to write (decision 20).** One pass, three steps, in order, on one
message: restate (what `latest::restate` does today, unchanged in what it
decides), then complete under aliases - every field the message states is
also stated under each alias spelling the registry gives it, the same value,
the same tag, beside the canonical child, so a consumer addressing the old
name or the new finds one value and the entries are the most complete the
dictionary allows - then fill (the RULES table, and piece 7's memory). The
alias twins are row children, not entries: `into_bytes` re-emits the wire as
received. `into_row` writes the column of the tag once (the canonical child;
say in `schema.rs` which child a tag's column reads when several carry it).
`FixMsg::into_latest` is deleted, `latest.rs` becomes the first step of
`enrich.rs` (one module or two, one door), `FixCodec::enrich_message(s)` and
`enrich_messages_arrow_reader` are the surface, the bindings lose
`into_latest`/`intoLatest`, and the docs' three stages become two: enrich,
then lifecycle. Idempotence stays: a second pass is equal. Pin: every case
of `rust/tests/fix/latest.rs` through `enrich_message`; the `enrich` group
of the snapshot regenerated and its moved lines named (the restatement now
runs there: `version` becomes newest, `150=1` becomes `F`, and so on - list
them); an alias twin present after enrichment (`lastqty` and `lastshares`
both stated, one value); `version` stamped once; lifecycle unchanged on the
enriched output; the bindings' `enrich_messages` on a mixed stream.

## 9. Identifiers are the component's, and `altids` carries them

**Today.** No dictionary property says which fields identify a message. The
lifecycle hard-codes five (`CHAIN_TAGS: [41, 11, 37, 526, 198]`,
`lifecycle.rs:71`, joined in that order by `chain_keys`, `:303-318`),
`enrich` fills none, and a consumer wanting an execution's or a quote's
identifiers reads the tags itself. The shipped dictionary names 86 fields
whose folded name ends in an identifier family - `clordid`, `origclordid`,
`secondaryclordid`, `orderid`, `secondaryorderid`, `listid`, `quoteid`,
`quotereqid`, `quoterespid`, `quoteentryid`, `execid`, `execrefid`,
`secondaryexecid`, `tradeid`, `secondarytradeid`, `tradereportid`,
`tradereportrefid`, `firmtradeid`, `regulatorytradeid`, `allocid`,
`secondaryallocid`, `individualallocid`, and their `side`, `leg`, `ref`,
`orig`, `affected` spellings, plus twenty-odd `*reportid` of status and
administrative reports. The `fix:` vocabulary is `field.rs:24-51` (`tag`,
`tags`, `aliases`, `nulls`, `lineage`, `codes`, `replacements`, `counter`,
`component`, `field`, `branches`, `msgtype`), every key an accessor pair,
never spelled by a caller. The crate's fields are twenty (65000-65019,
`crated.rs:60-127`; `CRATE_TAG_MAX` 65100), pinned as a count and as a list
(`python/tests/fix/test_fix.py:55` `CRATED = 20`,
`node/tests/fix/fix.test.js:1335-1339` `schemaTags().slice(-21)`,
`rust/tests/fix/digest.rs:271-273,344`, `docs/fix/capture.md:366-385`,
`.api-inventory.txt:700-725`), and none is a map, though
`DataType::map_of(key, value, keys_sorted)` (`types/nested/dtypes.rs:521`),
`Scalar::from_mapping` (`types/scalar.rs:640`) and the Arrow crossing
(`types/arrow.rs:865`) exist.

**Rule to write (decision 21).** A component says which of its members
identify the message: `fix:identifiers`, a list of member names (a tag
given at the door resolves to the name, as `fix:replacements` resolves), in
the order the component lists them, an accessor pair like every other key,
merged like every other `fix:` property, edited by `ygg fix components
update`. The generator derives the shipped lists from FIX's own families:
read the FIX Latest online field reference (fiximate.fixtrading.org, the
FIX Trading Community's) for the identifiers of orders (`ClOrdID`,
`OrigClOrdID`, `SecondaryClOrdID`, `OrderID`, `SecondaryOrderID`, `ListID`,
`QuoteID`, `QuoteReqID`, `QuoteRespID`, `QuoteEntryID`), of executions and
trades (`ExecID`, `ExecRefID`, `SecondaryExecID`, `TradeID`,
`SecondaryTradeID`, `TradeReportID`, `TradeReportRefID`, `FirmTradeID`,
`RegulatoryTradeID`) and of allocations (`AllocID`, `SecondaryAllocID`,
`IndividualAllocID`); settle the suffix families in the decision; write
them as one table in `scripts/generate_fix_dictionary.py` beside
`CODED_TAGS`; derive `fix:identifiers` for every component as the members
whose folded name is in a family; and let `--check` prove it. A status or
administrative report's `*ReportID` is not an order's identifier: the
families are listed, not guessed, and the census of every shipped
component's list is the fixture. Beside it the crate gains `altids` (65020,
`AltIds`, `map<utf8, utf8>` with sorted keys): every identifier the message
states at its own level, keyed by the canonical field name, the value as
the message spelled it; group members are not flattened (say so). `enrich`
fills it in the fill step from the message's component's `fix:identifiers`
- never overwriting a stated `altids`, a second pass equal - and a message
whose type no component names carries none. Pin: the shipped lists of
`newordersingle`, `executionreport`, `tradecapturereport`, `quote` and
`allocationinstruction` verbatim; `altids` on the corpus's execution reports
through the snapshot's `enrich` group (regenerated, lines named); an
`altids` stated on the way in untouched; the crate count twenty-one at this
piece, in every place listed above.

## 10. `uuid`, `puuid`, `instuuid`

**Today.** The three lifecycle columns are `instid` (65016), `id` (65017),
`persistentid` (65018), typed `fixed_size_binary(16)` (`crated.rs:440-462`):
`instid` the xxh128 digest of market, classification, ISIN-else-symbol and
currency (`lifecycle.rs:333-370`); `id` and `persistentid` a `TxHash` - the
eight-byte big-endian microsecond instant, then the eight-byte xxh3 digest
(`txhash/value.rs:206-213`; `persistent_digest`, `lifecycle.rs:386-395`) -
so they sort by the market's clock. The crate has a `uuid` datatype
(`DataType::uuid()`, `types/uuid/dtypes.rs:46`; `Uuid(u128)` with
`from_bytes`/`into_bytes`, `types/uuid/scalars.rs:15-36`; Arrow
`FixedSizeBinary(16)` under `UUID_EXTENSION_NAME`, `types/arrow.rs:406,535`).
The names cross as `ID_TAG_NAME`, `INSTID_TAG_NAME`, `PERSISTENTID_TAG_NAME`
(`fix/mod.rs:173-176`, `lib.rs:79-84`, `.api-inventory.txt:719-721`; those
`.api-bindings.txt` lists cross the bindings); pins:
`rust/tests/fix/lifecycle.rs` (ordering by bytes, `:64-103`),
`rust/tests/fix/digest.rs:271-273,344`, `python/tests/fix/test_fix.py`
(eight spellings), `node/tests/fix/fix.test.js` (six),
`docs/fix/lifecycle.md:5-20` (the Contract table), `docs/fix/capture.md:382-384`,
`docs/fix/arrow.md` (four).

**Rule to write (decision 22).** The three are UUIDs: `uuid` (65017,
`Uuid`), `puuid` (65018, `PUuid`), `instuuid` (65016, `InstUuid`), typed
`uuid`, tags unchanged, RFC 9562 shapes: `uuid` and `puuid` are version 7 -
the 48-bit unix millisecond instant, the 12-bit sub-millisecond fraction of
the microsecond instant, the variant, then 62 bits of the xxh3 digest - so
they still sort by the market's clock and two messages a microsecond apart
still differ; `instuuid` is version 8 over the xxh128 digest with the
version and variant bits set. The exact instant is `timestamp`'s fact and
the impact clock's (tags 60/52), read again where needed; the module doc
stops saying an id "carries" it. `ID_TAG_NAME`, `PERSISTENTID_TAG_NAME`,
`INSTID_TAG_NAME` become `UUID_TAG_NAME`, `PUUID_TAG_NAME`,
`INSTUUID_TAG_NAME`; nothing keeps the old spelling; a stored dataset
carrying `id`/`persistentid`/`instid` columns is read as the binary it is
(say so under Edges in `docs/fix/lifecycle.md`). Pin: every lifecycle test
under the new names with the version and variant nibbles asserted; the
`uuid` order by time kept (`lifecycle.rs:118`'s `<` survives); the `uuid`
extension on the column `write_arrow_reader` writes; each binding reading a
`uuid` as its UUID text.

## 11. The lifecycle keeps its chains by `puuid` and scopes them by `instuuid`

**Today.** `FixLifecycle` (`lifecycle.rs:135-297`) holds
`chains: Vec<Option<Chain>>` with `Chain { persistent, keys }`,
`keys: HashMap<SmolStr, usize>` from identifier text to chain, and `free`
holes; `fill` joins through `chain_keys` (the five hard tags), stamps the
three columns, closes on a terminal state (`is_terminal`); a message
already carrying a `persistentid` "joins nothing new" (`:214-217`) - it is
neither looked up nor checked - and a `ClOrdID` reused on another
instrument joins the first instrument's chain. `FixCodec::lifecycle` and
the bindings' `lifecycle(messages)` wrap it (`.api-bindings.txt:113,352`);
`docs/fix/lifecycle.md:160-176` says the chain is the identifiers joined.

**Rule to write (decision 23).** A chain is one event's life - an order, a
quote, a trade report - known by its `puuid`, its identifiers scoped by its
`instuuid`: `chains: HashMap<Uuid, Chain>` (or today's vector with an index
by `puuid`; the decision's), `keys: HashMap<(Option<Uuid>, SmolStr), Uuid>`
keyed by the instrument and the identifier text, the identifiers read from
`altids` (piece 9) when stated, else derived by the one function `enrich`
uses - one owner of "which members identify a message"; the lifecycle takes
a registry for it, as it does today. Joining, in order: a stated `puuid`
naming an alive chain joins it directly; else the message's identifiers
under its `instuuid` are looked up and, where they reach a chain, the
message's `puuid` is set to the chain's - a stated `puuid` is the one column
this pass overwrites, because a chain's identity is the chain's fact, not
the message's (the Stated row of the Contract table says exactly this);
else a stated `puuid` opens a chain under itself and an unstated one opens a
chain under a new version-7 uuid. Identifiers reaching two alive chains are
one event seen twice: the decision says whether the later folds into the
earlier (keys re-pointed, the earlier `puuid` on every later message) or the
first wins, and pins it. Everything else stands: terminal states close,
`clear` forgets, entries untouched, a second pass equal. Pin: a stated
foreign `puuid` corrected through `altids`; one `ClOrdID` on two
instruments opening two chains; two chains met by one message resolved as
decided; `alive()` over the corpus stream equal to today's count (the
corpus states no `puuid`).

## 12. `prevtimestamp` and `prevuuid`

**Today.** The crate's one "previous" fact is `prevpluginid` (65010, "the
plugin a message came through before this one", `crated.rs:98`,
`docs/fix/capture.md:376`); a consumer wanting the previous message of a
chain sorts by `id` and lags. `timestamp` (65003) is the row's clock, a
`DateTime64` (`crated.rs:316`).

**Rule to write (decision 24).** Two crate fields, `prevtimestamp` (65021,
`PrevTimestamp`, the type `timestamp` has) and `prevuuid` (65022,
`PrevUuid`, `uuid`): the `timestamp` and `uuid` of the previous message of
the same chain, stamped by the lifecycle pass from what the chain last saw
(`Chain` gains the pair), null on the first message of a chain, never
overwritten. Twenty-three crate fields: `CRATED = 23`, `schemaTags().slice(-24)`
(65000..65022, then 385 where piece 2 left it), the capture table, the
inventory. Pin: a three-message chain carrying the two previous stamps in
order; the first message null on both; a second pass equal; the crate count
in every place listed under piece 9.

## The fixtures to add to `rust/tests/fix/ulbridge.log`

Transcribed from photographs of a production bridge log; every value is as
read from the screen and a checksum is what the wire said, which the codec
does not verify. Append them as lines 114 onwards, in this order, one line
each (the JSON answers are one line each; `\n` inside `InitFileContent` is
the two-character escape as the log wrote it). They exercise, in order: a
single-plugin read of a live session (`type=Plugin`, with 49/56/8,
`InitFileContent`, `Enrichments`, `ExtendedActions`, `Resources`), the same
answer logged twice, the Jolokia prose lines (`URI`, `Path-Info`, `Request`,
`Execution time`) that yield nothing, a second session with `Type: I`, a
network-usage prose line, a heartbeat the bridge *sent*, an execution report
it *received*, the bridge row it routed, the post-enrichment row with
`ULFROMSESSIONNAME`, three prose lines, and a wildcard read mixing
`ConfigurationPlugin` and `Plugin` entries with `State: stopped`,
`CFBInfos`, `NeedCFBReload` and timers with a `timezone`.

```text
2026-08-14 03:03:13.314 [23] [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_POSTTRADE,plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","BeginString":"FIX.4.2","Category":"InterBridge","PrimaryHost":"localhost","CurrentPort":7061,"TargetCompID":"ULB_PTBDG","BackupHost":null,"Prefix":"","Guid":"ULMSG_BROKER_TO_POSTTRADE","LogLevel":-1,"Name":"ULMSG_BROKER_TO_POSTTRADE","PriorityLevel":5,"Version":"2.0.3","ExtendedActions":[{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"send-test-request","description":"Send a test request message. This action is available only when the adapter is logged.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"test-request-id","description":"The outgoing test request ID to send","defaultValue":"PING"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"stop","description":"Stops the adapter with a customizable logout text.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"logout-text","description":"The logout text","defaultValue":"halt asked"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"sequence-reset-reset","description":"Issue a Sequence Reset-Reset message with new sequence number","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"new-seq-no","description":"New outgoing sequence number","defaultValue":"492"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"hot-reset","description":"Sends a FIX logon message with the ResetSeqNum tag (141) set to Y. This action is available only when the adapter is logged.","parameters":[],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"synchronize","description":"Synchronizes routes of incoming messages.","parameters":[],"enabled":false}],"BinaryName":"ULMsg.jar","ClassName":"ULMsg","CurrentHost":"localhost","OutgoingMsgSeqNum":490,"ClassHierarchy":[{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.AbstractSessionInterface","classRevision":"1.103"},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.commons.standard.StandardPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.toolkit.plugins.common.core.BPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.ULFixSession","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.ULMsg","classRevision":null}],"RevisionInformation":"$Revision: 2.0.3 $ $Date: 2019/11/15 15:03:06 $","BackupPort":-1,"Comment":"","InitFileContent":"[class]\nname = com.ullink.ulbridge2.plugins.ULMsg\n\n[session]\nHeartBeat = 30\nSenderCompID = ${ULMSG_BROKER_TO_POSTTRADE.session.sendercompid}\nTargetCompID = ${ULMSG_BROKER_TO_POSTTRADE.session.targetcompid}\ntype = A\n\n[connection/1]\nhost = ${ULMSG_BROKER_TO_POSTTRADE.session.host}\nport = ${ULMSG_BROKER_TO_POSTTRADE.session.port}\n\n[category]\nname = InterBridge\n\n[transport]\ntimeout = 60000\nbuffer-size = 100000000\n\n[options]\nenable-synchro = false\nqueue-size = 10000\n\n[attachment-condition]\n0 = if (true) then &SetEnv\n1 = if true then &ULMSG_Copy_Enrichment\n\n[timers/plugin_ULMSG_BKRBDG_TO_PTBDG_timer_1_0]\naction = doStart\ntrigger = 0 30 22 ? * SUN\n\n[timers/plugin_ULMSG_BKRBDG_TO_PTBDG_timer_2_0]\naction = doStop();doReset()\ntrigger = 0 30 04 ? * SAT\n","Enrichments":[{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":0,"action":"unmodified","enrichmentName":"&SetEnv","sessionInterfaceName":"ULMSG_BROKER_TO_POSTTRADE","condition":"if (true)"},{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":1,"action":"unmodified","enrichmentName":"&ULMSG_Copy_Enrichment","sessionInterfaceName":"ULMSG_BROKER_TO_POSTTRADE","condition":"if true"}],"PrimaryPort":7061,"Type":"A","LoadIsolation":0,"Suffix":"","State":"logged","NotificationsStatus":false,"NeedReload":false,"Resources":[{"$type":"com.ullink.ulbridge.objects.ResourceInfo","name":"ulfixsession-cm-extension","version":"4.4.0","targetProducts":["ul-configuration-manager"]}],"IncomingMsgSeqNum":489,"MinimumBridgeRevision":"20050101000000"},"status":200}
2026-08-14 03:03:13.314 [23] [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_POSTTRADE,plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","BeginString":"FIX.4.2","Category":"InterBridge","PrimaryHost":"localhost","CurrentPort":7061,"TargetCompID":"ULB_PTBDG","BackupHost":null,"Prefix":"","Guid":"ULMSG_BROKER_TO_POSTTRADE","LogLevel":-1,"Name":"ULMSG_BROKER_TO_POSTTRADE","PriorityLevel":5,"Version":"2.0.3","ExtendedActions":[{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"send-test-request","description":"Send a test request message. This action is available only when the adapter is logged.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"test-request-id","description":"The outgoing test request ID to send","defaultValue":"PING"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"stop","description":"Stops the adapter with a customizable logout text.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"logout-text","description":"The logout text","defaultValue":"halt asked"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"sequence-reset-reset","description":"Issue a Sequence Reset-Reset message with new sequence number","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"new-seq-no","description":"New outgoing sequence number","defaultValue":"492"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"hot-reset","description":"Sends a FIX logon message with the ResetSeqNum tag (141) set to Y. This action is available only when the adapter is logged.","parameters":[],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"synchronize","description":"Synchronizes routes of incoming messages.","parameters":[],"enabled":false}],"BinaryName":"ULMsg.jar","ClassName":"ULMsg","CurrentHost":"localhost","OutgoingMsgSeqNum":490,"ClassHierarchy":[{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.AbstractSessionInterface","classRevision":"1.103"},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.commons.standard.StandardPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.toolkit.plugins.common.core.BPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.ULFixSession","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.ULMsg","classRevision":null}],"RevisionInformation":"$Revision: 2.0.3 $ $Date: 2019/11/15 15:03:06 $","BackupPort":-1,"Comment":"","InitFileContent":"[class]\nname = com.ullink.ulbridge2.plugins.ULMsg\n\n[session]\nHeartBeat = 30\nSenderCompID = ${ULMSG_BROKER_TO_POSTTRADE.session.sendercompid}\nTargetCompID = ${ULMSG_BROKER_TO_POSTTRADE.session.targetcompid}\ntype = A\n\n[connection/1]\nhost = ${ULMSG_BROKER_TO_POSTTRADE.session.host}\nport = ${ULMSG_BROKER_TO_POSTTRADE.session.port}\n\n[category]\nname = InterBridge\n\n[transport]\ntimeout = 60000\nbuffer-size = 100000000\n\n[options]\nenable-synchro = false\nqueue-size = 10000\n\n[attachment-condition]\n0 = if (true) then &SetEnv\n1 = if true then &ULMSG_Copy_Enrichment\n\n[timers/plugin_ULMSG_BKRBDG_TO_PTBDG_timer_1_0]\naction = doStart\ntrigger = 0 30 22 ? * SUN\n\n[timers/plugin_ULMSG_BKRBDG_TO_PTBDG_timer_2_0]\naction = doStop();doReset()\ntrigger = 0 30 04 ? * SAT\n","Enrichments":[{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":0,"action":"unmodified","enrichmentName":"&SetEnv","sessionInterfaceName":"ULMSG_BROKER_TO_POSTTRADE","condition":"if (true)"},{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":1,"action":"unmodified","enrichmentName":"&ULMSG_Copy_Enrichment","sessionInterfaceName":"ULMSG_BROKER_TO_POSTTRADE","condition":"if true"}],"PrimaryPort":7061,"Type":"A","LoadIsolation":0,"Suffix":"","State":"logged","NotificationsStatus":false,"NeedReload":false,"Resources":[{"$type":"com.ullink.ulbridge.objects.ResourceInfo","name":"ulfixsession-cm-extension","version":"4.4.0","targetProducts":["ul-configuration-manager"]}],"IncomingMsgSeqNum":489,"MinimumBridgeRevision":"20050101000000"},"status":200}
2026-08-14 03:03:13.319 [23] [Jolokia] (DEBUG) URI: /jolokia/read/com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin
2026-08-14 03:03:13.319 [23] [Jolokia] (DEBUG) Path-Info: read/com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin
2026-08-14 03:03:13.319 [23] [Jolokia] (DEBUG) Request: JmxReadRequest[attribute=null, objectName = com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin]
2026-08-14 03:03:13.319 [23] [Jolokia] (DEBUG) Execution time: 0 ms
2026-08-14 03:03:13.319 [23] [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BRK","BeginString":"FIX.4.2","Category":"InterBridge","PrimaryHost":"prod-ul-bridge-dmz","CurrentPort":7062,"TargetCompID":"ULB_DMZ","BackupHost":null,"Prefix":"","Guid":"ULMSG_BROKER_TO_DMZ","LogLevel":-1,"Name":"ULMSG_BROKER_TO_DMZ","PriorityLevel":5,"Version":"2.0.3","ExtendedActions":[{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"send-test-request","description":"Send a test request message. This action is available only when the adapter is logged.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"test-request-id","description":"The outgoing test request ID to send","defaultValue":"PING"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"stop","description":"Stops the adapter with a customizable logout text.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"logout-text","description":"The logout text","defaultValue":"halt asked"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"sequence-reset-reset","description":"Issue a Sequence Reset-Reset message with new sequence number","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"new-seq-no","description":"New outgoing sequence number","defaultValue":"491"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"hot-reset","description":"Sends a FIX logon message with the ResetSeqNum tag (141) set to Y. This action is available only when the adapter is logged.","parameters":[],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"synchronize","description":"Synchronizes routes of incoming messages.","parameters":[],"enabled":false}],"BinaryName":"ULMsg.jar","ClassName":"ULMsg","CurrentHost":"prod-ul-bridge-dmz","OutgoingMsgSeqNum":489,"ClassHierarchy":[{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.AbstractSessionInterface","classRevision":"1.103"},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.commons.standard.StandardPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.toolkit.plugins.common.core.BPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.ULFixSession","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.ULMsg","classRevision":null}],"RevisionInformation":"$Revision: 2.0.3 $ $Date: 2019/11/15 15:03:06 $","BackupPort":-1,"Comment":"","InitFileContent":"[class]\nname = com.ullink.ulbridge2.plugins.ULMsg\n\n[session]\nIP_machine = ${ULMSG_BROKER_TO_DMZ.session.ip_machine}\nIP_port = ${ULMSG_BROKER_TO_DMZ.session.ip_port}\nVersion = FIX.4.2\nHeartBeat = 30\nSenderCompID = ${ULMSG_BROKER_TO_DMZ.session.sendercompid}\nTargetCompID = ${ULMSG_BROKER_TO_DMZ.session.targetcompid}\nretrytimeout = 10\ntype = I\npersistence = DATABASE\n\n[category]\nname = InterBridge\n\n[transport]\ntimeout = 60000\nbuffer-size = 100000000\n\n[options]\nenable-synchro = false\nqueue-size = 10000\n\n[routing]\ndestination = @DMZ_Broker_Routing\n\n[attachment-condition]\n0 = if (true) then &SetEnv\n\n[timers/plugin_ULMSG_ULMSG_BROKER_TO_DMZ_timer_1_0]\naction = doStart\ntrigger = 0 25 04 ? * SAT\n\n[timers/plugin_ULMSG_BROKER_TO_DMZ_timer_2_0]\naction = doStop();doReset()\ntrigger = 0 35 22 ? * SUN\n\n[timers/plugin_ULMSG_BROKER_TO_DMZ_daily_hot_Reset]\naction = com.ullink.ulbridge.sessioninterfaces.plugins:type=Plugin,name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX.executeExtendedAction(\"hot-reset\", [])\ntrigger = 0 00 23 ? * MON-FRI\n","Enrichments":[{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":0,"action":"unmodified","enrichmentName":"&SetEnv","sessionInterfaceName":"ULMSG_BROKER_TO_DMZ","condition":"if (true)"}],"PrimaryPort":7062,"Type":"I","LoadIsolation":0,"Suffix":"","State":"logged","NotificationsStatus":false,"NeedReload":false,"Resources":[{"$type":"com.ullink.ulbridge.objects.ResourceInfo","name":"ulfixsession-cm-extension","version":"4.4.0","targetProducts":["ul-configuration-manager"]}],"IncomingMsgSeqNum":489,"MinimumBridgeRevision":"20050101000000"},"status":200}
2026-08-14 03:03:13.776 [50] [NetworkUsageMessageInterceptor] (INFO) network-Dashboard module network usage[sent=1510822, received=73903]
2026-08-14 03:03:16.016 [449] [SmartTrade_OrderRouting] (DEBUG) Sending : 8=FIX.4.4|9=62|35=0|49=BRK.PROD.TRD|56=ST.PROD|34=14599|52=20260814-01:03:16|10=071|
2026-08-14 03:03:17.910 [15333-e7254b22:9f00927396:2141] [Virtu_TritonBlack_TradeCapture] (INFO) Receiving : 8=FIX.4.2|9=0327|35=8|34=2141|49=ITGADC|56=CLIENTFIS|52=20260814-01:03:17|1=client|77=O|55=1605|22=4|48=TW0001605004|15=TWD|59=6|76=RJEA|32=24000|31=39.9|20=0|150=1|39=1|11=20260814_TP1_CLIENT_1013|37=20260814_TP1_CLIENT_1013|17=2062|54=1|38=3000000|14=230000|151=2770000|6=39.9136956521739|40=1|30=RJEA|60=20260814-01:03:17|109=apclient_trade|10=103|
2026-08-14 03:03:17.911 [15333-e7254b22:9f00927396:2141] [Virtu_TritonBlack_TradeCapture] (DEBUG) RouteMessage : ACCOUNT=client|AVGPX=39.9136956521739|CLORDID=20260814_TP1_CLIENT_1013|CUMQTY=230000|CURRENCY=TWD|EXECBROKER=RJEA|EXECID=2062|EXECTYPE=trade|ISINCODE=TW0001605004|LASTMKT=RJEA|LASTPX=39.9|LASTSHARES=24000|LEAVESQTY=2770000|MSGTYPE=executionreport|OPENCLOSE=open|ORDERID=20260814_TP1_CLIENT_1013|ORDERQTY=3000000|ORDSTATUS=partfilled|ORDTYPE=market|SECURITYID=TW0001605004|SIDE=buy|SYMBOL=1605|TIMEINFORCE=gtd|TRANSACTTIME=20260814010317000|
2026-08-14 03:03:17.911 [15333-e7254b22:9f00927396:2141] [Virtu_TritonBlack_TradeCapture] (INFO) Filtering - Message for KRM22
2026-08-14 03:03:17.911 [15333-e7254b22:9f00927396:2141] [ULBridge] (DEBUG) Result of message post-enrichment : ACCOUNT=client|AVGPX=39.9136956521739|CLORDID=20260814_TP1_CLIENT_1013|CONVERSATIONID=5b3bf9f0-872e-4727-acb8-da21d5248074|CUMQTY=230000|CURRENCY=TWD|ENV=PROD|EXECBROKER=RJEA|EXECID=2062|EXECTYPE=trade|FILTER=yes|ISINCODE=TW0001605004|LASTMKT=RJEA|LASTPX=39.9|LASTSHARES=24000|LEAVESQTY=2770000|MSGTYPE=executionreport|OPENCLOSE=open|ORDERID=20260814_TP1_CLIENT_1013|ORDERQTY=3000000|ORDSTATUS=partfilled|ORDTYPE=market|SECURITYID=TW0001605004|SIDE=buy|SYMBOL=1605|TIMEINFORCE=gtd|TRANSACTTIME=20260814010317000|ULFROMSESSIONNAME=Virtu_TritonBlack_TradeCapture|ULTOSESSIONNAME=|
2026-08-14 03:03:17.911 [15333-e7254b22:9f00927396:2141] [EnrichmentManager] (INFO) Enrichment execution[&SetEnv,&Virtu_TritonBlack_TradeCapture]
2026-08-14 03:03:17.911 [15333-e7254b22:9f00927396:2141] [AliasManager] (INFO) Resolve @ULFilter->ULFilter (1us)
2026-08-14 03:13:13.131 [23] [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_BDG_DMZ_PCO,plugin-type=FIX,type=ConfigurationPlugin":{"Comment":"","Category":"InterBridge","Enrichments":[{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":0,"action":"unmodified","enrichmentName":"&SetEnv","sessionInterfaceName":"ULMSG_BROKER_BDG_DMZ_PCO","condition":"if (true)"},{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":1,"action":"unmodified","enrichmentName":"&ULMSG_Add_Fields","sessionInterfaceName":"ULMSG_BROKER_BDG_DMZ_PCO","condition":"if (true)"}],"Prefix":"","Name":"ULMSG_BROKER_BDG_DMZ_PCO","LoadIsolation":0,"Suffix":"","PriorityLevel":5,"Version":"2.0.3","ExtendedActions":[{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"send-test-request","description":"Send a test request message. This action is available only when the adapter is logged.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"test-request-id","description":"The outgoing test request ID to send","defaultValue":"PING"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"stop","description":"Stops the adapter with a customizable logout text.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"logout-text","description":"The logout text","defaultValue":"halt asked"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"sequence-reset-reset","description":"Issue a Sequence Reset-Reset message with new sequence number","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"new-seq-no","description":"New outgoing sequence number","defaultValue":"525"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"hot-reset","description":"Sends a FIX logon message with the ResetSeqNum tag (141) set to Y. This action is available only when the adapter is logged.","parameters":[],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"synchronize","description":"Synchronizes routes of incoming messages.","parameters":[],"enabled":false}],"State":"logged","BinaryName":"ULMsg.jar","ClassName":"ULMsg","NeedReload":false,"ClassHierarchy":[{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.AbstractSessionInterface","classRevision":"1.103"},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.commons.standard.StandardPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.toolkit.plugins.common.core.BPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.ULFixSession","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.ULMsg","classRevision":null}],"RevisionInformation":"$Revision: 2.0.3 $ $Date: 2019/11/15 15:03:06 $","MinimumBridgeRevision":"20050101000000"},"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,plugin-type=FIX,type=ConfigurationPlugin":{"Comment":"","Category":"InterBridge","Enrichments":[{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":0,"action":"unmodified","enrichmentName":"&SetEnv","sessionInterfaceName":"ULMSG_BROKER_TO_DMZ","condition":"if (true)"}],"Prefix":"","Name":"ULMSG_BROKER_TO_DMZ","LoadIsolation":0,"Suffix":"","PriorityLevel":5,"Version":"2.0.3","ExtendedActions":[{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"send-test-request","description":"Send a test request message. This action is available only when the adapter is logged.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"test-request-id","description":"The outgoing test request ID to send","defaultValue":"PING"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"stop","description":"Stops the adapter with a customizable logout text.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"logout-text","description":"The logout text","defaultValue":"halt asked"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"sequence-reset-reset","description":"Issue a Sequence Reset-Reset message with new sequence number","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"new-seq-no","description":"New outgoing sequence number","defaultValue":"511"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"hot-reset","description":"Sends a FIX logon message with the ResetSeqNum tag (141) set to Y. This action is available only when the adapter is logged.","parameters":[],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"synchronize","description":"Synchronizes routes of incoming messages.","parameters":[],"enabled":false}],"State":"logged","BinaryName":"ULMsg.jar","ClassName":"ULMsg","NeedReload":false,"ClassHierarchy":[{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.AbstractSessionInterface","classRevision":"1.103"},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.commons.standard.StandardPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.toolkit.plugins.common.core.BPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.ULFixSession","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.ULMsg","classRevision":null}],"RevisionInformation":"$Revision: 2.0.3 $ $Date: 2019/11/15 15:03:06 $","MinimumBridgeRevision":"20050101000000"},"com.ullink.ulbridge.sessioninterfaces.plugins:name=GLMX_BANK_FIX44_DropCopy,plugin-type=FIX,type=Plugin":{"SenderCompID":"CLIENT_BANK","BeginString":"FIX.4.4","Category":"Drop Copy","PrimaryHost":"127.0.0.1","CurrentPort":9905,"TargetCompID":"GLMX","BackupHost":null,"Prefix":"","Guid":"GLMX_BANK_FIX44_DropCopy","LogLevel":-1,"Name":"GLMX_BANK_FIX44_DropCopy","PriorityLevel":5,"Version":"3.10.2","ExtendedActions":[{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"start-and-reset-seq-num-flag","description":"Starts the adapter and sets the ResetSeqNum tag (141) to Y in the FIX logon message. This action is available only when the adapter is stopped.","parameters":[],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"set-sequence-numbers","description":"Sets the value of the incoming and outgoing FIX sequence numbers for this adapter. This action is available only when the adapter is stopped.","parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"incoming","description":"The incoming FIX sequence number to set","defaultValue":"1"},{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"outgoing","description":"The outgoing FIX sequence number to set","defaultValue":"1"}],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"reload-cfb","description":"Reloads the .cfb file without stopping the underlying FIX connection.","parameters":[],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"Enter filter mode","description":"Enter filter mode","parameters":[],"enabled":true},{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"Exit filter mode","description":"Exit filter mode","parameters":[],"enabled":true}],"BinaryName":"FIXCMSPlugin.jar","ClassName":"FIXCMSPlugin","CurrentHost":"127.0.0.1","OutgoingMsgSeqNum":1,"ClassHierarchy":[{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.AbstractSessionInterface","classRevision":"1.103"},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.commons.standard.StandardPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.toolkit.plugins.common.core.BPlugin","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.ULFixSession","classRevision":null},{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","className":"com.ullink.ulbridge2.plugins.FIXCMSPlugin","classRevision":null}],"RevisionInformation":"$Revision: 3.10.2 $ $Date: 2021/05/03 09:32:55 $","BackupPort":-1,"Comment":"","InitFileContent":"[class]\nname = com.ullink.ulbridge2.plugins.FIXCMSPlugin\n\n[session]\nIP_machine = ${GLMX_BANK_FIX44_DropCopy.session.ip_machine}\nIP_port = ${GLMX_BANK_FIX44_DropCopy.session.ip_port}\nHeartBeat = 30\nretrytimeout = 30\ntype = I\npersistence = DATABASE\ncheckCompID = Y\nSenderCompID = ${GLMX_BANK_FIX44_DropCopy.session.sendercompid}\nTargetCompID = ${GLMX_BANK_FIX44_DropCopy.session.targetcompid}\nVersion = FIX.4.4\n\n[ssl]\nenabled = ${GLMX_BANK_FIX44_DropCopy.ssl.enabled}\n\n[logon]\n553 = ${GLMX_BANK_FIX44_DropCopy.logon.553}\n554 = ${GLMX_BANK_FIX44_DropCopy.logon.554}\n\n[cms]\ncfb-name = cms/GLMX_FIX44_DropCopy.cfb\nbytecode-cache-directory = ${ulbridge.cms.bytecode-cache-directory}\n\n[options]\nstrict-grammar = false\ntime-precision = 6\n\n[category]\nname = Drop Copy\n\n[attachment-condition]\n0 = if (true) then &SetEnv\n1 = if ($MSGTYPE=\"tradecapturereport\") then &GLMX_FIX44_DropCopy_Handle_Repo\n2 = if ($MSGTYPE=\"executionreport\" and $SECURITYTYPE=\"REPO\") then &Force_IRIS_ByPass\n\n[timers/GLMX_BANK_FIX44_DropCopy_StopReset]\naction = doStop ; doReset\ntrigger = 0 0 23 ? * MON-FRI;\ntimezone = Europe/Zurich\n\n[timers/GLMX_BANK_FIX44_DropCopy_Start]\naction = doStart\ntrigger = 0 0 4 ? * MON-FRI;\ntimezone = Europe/Zurich\n","Enrichments":[{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":0,"action":"unmodified","enrichmentName":"&SetEnv","sessionInterfaceName":"GLMX_BANK_FIX44_DropCopy","condition":"if (true)"},{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":1,"action":"unmodified","enrichmentName":"&GLMX_FIX44_DropCopy_Handle_Repo","sessionInterfaceName":"GLMX_BANK_FIX44_DropCopy","condition":"if ($MSGTYPE=\"tradecapturereport\")"},{"$type":"com.ullink.ulbridge.objects.EnrichmentSIAttachment","index":2,"action":"unmodified","enrichmentName":"&Force_IRIS_ByPass","sessionInterfaceName":"GLMX_BANK_FIX44_DropCopy","condition":"if ($MSGTYPE=\"executionreport\" and $SECURITYTYPE=\"REPO\")"}],"NeedCFBReload":false,"PrimaryPort":9905,"Type":"I","LoadIsolation":0,"Suffix":"","State":"stopped","NotificationsStatus":false,"NeedReload":false,"Resources":[{"$type":"com.ullink.ulbridge.objects.ResourceInfo","name":"ulfixsession-cm-extension","version":"4.7.0","targetProducts":["ul-configuration-manager"]}],"IncomingMsgSeqNum":1,"CFBInfos":[["GLMX_FIX44_DropCopy","$Revision: 1.53 $ $Date: 2025.10.27 17:10:18 $","Standard Buy Side 4.4"]],"MinimumBridgeRevision":"20090514000000"}},"status":200}
```

What they must answer, once pieces 1 to 8 are in: the two identical
`Response` lines are one `pluginconfig` message each (same digest), typed
`pluginconfig`, with `49=ULB_BKRBDG`, `56=ULB_PTBDG`, `8=FIX.4.2`, `20012=ULMSG_BROKER_TO_POSTTRADE`,
`20027=7061`, `20019=logged`, no 20001-20004, and direction `R` (a Jolokia
answer is received); the four prose lines nothing; the DMZ answer one
message with `Type` `I` and its timers verbatim in `InitFileContent`; the
network line nothing; the heartbeat one message, direction `S` from
`Sending`, its own 49/56; the execution report one message, direction `R`,
type `executionreport`; the `RouteMessage` bridge row one message (its
`MSGTYPE=executionreport` naming the type, `ISINCODE` filling `isincode`),
direction `R` because no verb stands in the prefix and the row's default
is the codec's (decide and pin whether a bridge row inherits the receiving
frame's direction on the same plugin - it does not today); `Filtering`,
`Enrichment execution`, `Resolve` nothing; the post-enrichment row one
message with `ULFROMSESSIONNAME` reaching `sendersessionname` through the
crate's alias; the wildcard answer three `pluginconfig` messages, two
`ConfigurationPlugin`s with no 49/56 and one `Plugin` with `49=CLIENT_BANK`,
`56=GLMX`, `State stopped`, `CFBInfos` as its canonical JSON text. For piece
7 add, after the heartbeat, a single-plugin answer for
`SmartTrade_OrderRouting` and then a bridge `RouteMessage` row on that same
plugin id with no 49/56, and pin that enrichment fills them from the answer.

## Refuted before, still standing

- Sharing one page across a `parse_lines` batch (decision 2's re-slicing);
  a page per *line* is decision 5.
- Hoisting the separator decision out of the per-line loop (`ulbridge.log`
  lines 7, 9 and 57).
- Collapsing the `Vec` stages between the entry tree and the builder
  (decision 7's pin).

## How to prove it

Per piece: the decision written first; Gate 1 whole (`fmt`, clippy crate
and workspace `-D warnings`, the default and `parquet iceberg` suites with
`--no-fail-fast`, the doctests, the bench check, `cargo doc -D warnings`, the
four MSRV checks, `--test iobase_calls --test allocations`, the equivalence
tripwire `cargo test --locked -p yggdryl --test fix the_codec_answers_what_it_answered`,
`generate_charset_tables.py --check`, `check_charset_interop.py`,
`generate_fix_dictionary.py --check`); then both bindings rebuilt (the wheel
with `maturin build --locked --manifest-path python/Cargo.toml`, the addon
with `npm run --prefix node build`; `pytest python/tests`, `mypy --strict`,
`npm test --prefix node`, `git diff --exit-code -- node/index.js node/index.d.ts`,
`build_docs_playground.js --check`, `build_docs_fix.js --check`) and Gate 4
(`check_docs_examples.py` for `rust`, `python`, `javascript`; `mkdocs build --strict`).
The snapshot is regenerated once per piece that moves it, with the moved
lines named in the commit; the corpus lines above are appended before piece
4 so every later regeneration shows them. The crate-count pins (`CRATED`,
`schemaTags().slice(-N)`, `digest.rs:344`, the capture table, the inventory)
move under pieces 9 and 12 only, and each of those commits names them. `CHARSET_READ_PATH_PROMPT.md` is
run afterwards with its baselines re-taken.

## Do not

- Do not regenerate `equivalence.snapshot` to make a test pass; regenerate it
  because a decision moved it, in that decision's commit, naming the lines.
- Do not reuse `DataTypeId` 58 or renumber any discriminant; retire it.
- Do not keep an alias, a redirect or a second spelling for anything deleted
  (`MimeType::ULCONFIG`, `UlPlugin`, `parse_ulconfig_line`, `parse_direction`,
  `into_latest`, tags 20001-20004, `FixCategory::Messages`); the bindings
  lose them by name.
- Do not renumber a crate tag: the three renamed columns keep 65016-65018,
  the new ones take 65020-65022 in that order, and `id`, `persistentid`,
  `instid` survive nowhere, not as aliases.
- Do not put a per-row branch on a charset, and do not reach for `unsafe`.
- Do not land two pieces in one commit.
