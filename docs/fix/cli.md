# CLI

`yggdryl fix` manages the native FIX catalog through explicit `fields`, `components`, and `groups` command trees, with `codesets` beside them for the vocabularies they read by; a message is a component created with `--msgtype`. Rust only: the wheel ships this compiled executable without a Python runtime in its execution path.

## Contract

| Aspect | Rule |
| --- | --- |
| Owner | `yggdryl-cli` parses arguments and renders results; the Rust registry owns schema validation, references, mutations, and persistence |
| Root | `--root`, default `config/fix`; relative locations resolve against the working directory; a folder holding no catalog opens with the crate's built-in definitions rather than failing |
| Categories | `fields`, `components`, `groups`; a message is a component carrying `FIX:msgtype`. `codesets` is not one of them: a set has no tag, no datatype and no reference, so it has verbs of its own |
| Operations | Every category supports `list`, `read`, `create`, `update`, and `delete`; `codesets` supports `list`, `read`, `write` and `delete` |
| Writes | Successful one-shot mutations save automatically; an interactive session saves only with `save` |
| Keys | A field key is a decimal tag or a name; a named category's key is its definition name. The registry is one namespace: a key resolves the same way whatever dictionaries a definition belongs to, and no key spells an identity |
| Dialect | `--dialect NAME` stamps membership (`FIX:sources`) on `ingest`, `create`, and `update`, and records the `sources.json` entry the id names where none is; on `list` it is a filter; `sync`, `read` and `delete` take none |
| Create | Refuses an existing name or field identity |
| Update | Replaces an existing definition completely, preserving identity; omitted metadata is removed |
| Delete | Refuses absence and live references |
| Enums | A named [code set](registry.md#a-field-names-the-code-set-it-reads-by) the dictionary holds: `--codes` on a field names one, `codesets write --codes '<json>'` states its members, and the set is stated before a field names it |
| Direction rules | Tag 385's `FIX:directions` metadata; `--directions` accepts its canonical JSON document |
| Identifiers | A component's direct scalar members; repeat `--identifiers` for names, aliases or decimal tags, resolved by the native setter into member order |
| Output | Plain stable text when redirected; colour where the core's [colour rule](../logging.md#colour) says - a colour terminal, off under `NO_COLOR` or `TERM=dumb`, brought into a pipe by `FORCE_COLOR` or `CLICOLOR_FORCE` - and box drawing and the spinner only on a terminal it colours, so a forced colour never writes frames or box characters into a pipe |
| Workflow | `--annotate`, also enabled by `GITHUB_ACTIONS`, prints workflow findings; failed checks and refused commands exit nonzero |

## Use

Read or search the committed catalog by category:

```bash
yggdryl fix --root config/fix fields list Party --limit 20
yggdryl fix --root config/fix fields read 453 --json
yggdryl fix --root config/fix groups read Parties --json
yggdryl fix --root config/fix components read Party
yggdryl fix --root config/fix components list Order
```

`NoPartyIDs(453)` is an `int32` scalar; `Parties` is a separate Serie definition whose occurrence component is `Party`. Message reads show the native non-null Struct and its full `FIX:msgtype` wire code.

## Install

The published wheel includes the native executable, every namespace in it: `yggdryl market serve --help` is the book display's own usage. From a checkout, Cargo runs the same binary:

```bash
pip install yggdryl
yggdryl fix --help
yggdryl market serve --help
cargo run -p yggdryl-cli -- fix fields list Symbol
```

To include the CLI in a locally built wheel, stage it before building the wheel:

```bash
python scripts/stage_cli.py
maturin build --manifest-path python/Cargo.toml --out dist
```

## Namespaces

`yggdryl` is one binary over three namespaces, each a subcommand owning its own verbs and state; this page is `fix`'s.

| Namespace | Serves | Page |
| --- | --- | --- |
| `fix` | a FIX dictionary: read it, change it, ingest a counterparty's configuration, check what came out - and with no verb, all of that interactively | this page |
| `xmla` | `yggdryl xmla serve`: folders of record media as XML for Analysis catalogs over HTTP | [Provider](../media/xmla.md#provider) |
| `market` | `yggdryl market serve`: tables of market data as the book display - bid and ask candles, books and audits over HTTP - each `--capture` folding a FIX bridge log into the first table before it serves | [Book display](../graph/serve.md) |

## Three category command trees

| Operation | Arguments and behavior |
| --- | --- |
| `<category> list [filter]` | Match name or decimal tag text, ignoring case; `--dialect NAME` keeps only definitions whose `FIX:sources` names that source; `--limit` defaults to 40 |
| `<category> read <key>` | Display one definition; `--json` emits a complete native `Field` document |
| `<category> create <name> <type>` | Create from the native datatype grammar and metadata flags |
| `<category> create --input <file>` | Create from one complete native `Field` JSON document |
| `<category> update <name> <type>` | Replace the complete existing definition |
| `<category> update --input <file>` | Replace from a complete native `Field` JSON document |
| `<category> delete <key>` | Delete the resolved definition, refusing dependents |
| `codesets <list\|read\|write\|delete>` | The vocabularies, keyed by set name rather than by a definition key; see [Code sets](#code-sets) |

A field key is a decimal tag or a name; named categories use their definition name. A decimal key is a tag and never an identity: the canonical holder of the tag answers, then an alternate. Anything else is a name resolved under the registry's one fold, canonical name before alias, so `Desk_Value`, `deskvalue` and `DeskValue` reach one field. A colon-bearing key such as `5001:venue` is a name that nothing holds, and a path such as `Parties[0].PartyID` is not a key here. Reads, deletes and lists consult no membership: `--dialect` on `list` filters the rows by provenance, and nothing else about resolution changes with it.

### Definition flags

| Flag | Applies to |
| --- | --- |
| `--tag N` | Scalar fields, including wire group counters; a crate Map group carries its own reserved tag |
| `--counter N` | Serie/LargeSerie groups identify an existing `int32` scalar; a crate Map group uses its own reserved tag, with no scalar counter |
| `--component NAME` | Groups; identifies the existing occurrence component |
| `--msgtype CODE` | Components; makes the component a message; full nonempty wire text, including spaces |
| `--identifiers MEMBER` | Components; repeat for each direct scalar identifier. Canonical names, aliases and decimal tags resolve once; input order does not change member order |
| `--codes NAME` | Scalar fields; the name of the code set this field reads its values by, which the dictionary must already hold. `codesets write` states the members |
| `--directions JSON` | Tag 385's scalar; its [direction rules](registry.md#a-direction-is-what-the-rules-on-tag-385-read-in-front-of-the-payload) as `FIX:directions`, one entry per code of the set; an empty list removes the property so the crate's defaults read again |
| `--dialect NAME` | Membership: a source this definition belongs to, recorded in `FIX:sources`, its `sources.json` entry created where none is; repeat the flag for several. Ids are lowercased, deduplicated and sorted; an empty id, or one holding a quote, a backslash or a control character, is refused |
| `--description TEXT` | Definition metadata |
| `--required` | Non-null definition; message roots are always non-null |

`--input` replaces positional name/type and all definition flags. Quote datatype expressions containing spaces or shell metacharacters; a group occurrence must be a non-null Struct.

```bash
yggdryl fix --root scratch/catalog fields create NoPartyIDs int32 --tag 453
yggdryl fix --root scratch/catalog fields create PartyID utf8 --tag 448
yggdryl fix --root scratch/catalog components create Party 'struct<PartyID: utf8>' --required
yggdryl fix --root scratch/catalog groups create Parties 'serie<Party: struct<PartyID: utf8> not null>' --counter 453 --component Party
yggdryl fix --root scratch/catalog components create Order 'struct<ClOrdID: utf8>' --msgtype D --identifiers ClOrdID
yggdryl fix --root scratch/catalog codesets write sidecodeset --codes '[{"value":"1","name":"Buy"},{"value":"2","name":"Sell"}]'
yggdryl fix --root scratch/catalog fields create Side utf8 --tag 54 --codes sidecodeset
yggdryl fix --root scratch/catalog codesets write msgdirectioncodeset --codes '[{"value":"R","name":"Receive"},{"value":"S","name":"Send"}]'
yggdryl fix --root scratch/catalog fields create MsgDirection utf8 --tag 385 --codes msgdirectioncodeset --directions '[{"code":"S","patterns":["(?i)^TX\\b"]},{"code":"R","patterns":["(?i)^RX\\b"]}]'
yggdryl fix --root scratch/catalog fields create DeskValue int32 --tag 5001 --dialect venue --dialect Desk
yggdryl fix --root scratch/catalog fields read Desk_Value
yggdryl fix --root scratch/catalog fields list --dialect desk
```

The read shows `identity -630917675`, the signed XXH32 of the tag and the folded name, and `sources desk, venue`, and `sources.json` now holds an entry for each id; the listing filtered on `desk` holds that one row. A field's identity is its tag and its name, so a second field on a held tag under another name is a new definition beside the holder: `fields create OtherName int64 --tag 5001` succeeds, `fields read OtherName` answers it, the bare `5001` keeps answering `DeskValue`, whose `names` entry now lists `OtherName`, and `fields list 5001` shows both rows. The same folded name on the same tag - `desk_value` with `--tag 5001` - is the existing identity and is refused, as is a held name on another tag. `update` replaces membership with what it states: an update without `--dialect` leaves the field a member of nothing, while the catalog entries stay, which [`check`](#schema-check-and-diff) notes where nothing names one.

A datatype expression embeds its child definitions. To preserve explicit canonical field/component/group references, use a resolved native `Field` document, such as the output of `read --json`; the compact unresolved placeholders in [folder storage](store.md#compact-references) are handled by the folder loader.

Identifier selection follows the [component declaration](registry.md): missing,
nested, ambiguous or duplicate members are refused atomically. Repeated flags
replace the declaration as a whole, and a positional `update` without them
removes it. `--input` takes the document's `FIX:identifiers` instead and cannot
be combined with `--identifiers`.

## Code sets

A vocabulary is the dictionary's, not one field's, so it has its own verbs: `list` shows what is held with the size of each set and how many fields read by it, `read` prints the members or the document a store writes, `write` states them, and `delete` takes a set away.

```bash
yggdryl fix --root config/fix codesets list side --limit 20
yggdryl fix --root config/fix codesets read sidecodeset
yggdryl fix --root config/fix codesets read sidecodeset --json
yggdryl fix --root scratch/catalog codesets write sidecodeset --codes '[{"value":"1","name":"Buy"},{"value":"2","name":"Sell"}]'
yggdryl fix --root scratch/catalog codesets write sidecodeset --merge --codes '[{"value":"7","name":"Undisclosed"}]'
yggdryl fix --root scratch/catalog codesets delete sidecodeset
```

`write` replaces the set; `--merge` folds by wire value instead, keeping what the set already held and adding every name, alias and wording the incoming statement brings, so a counterparty's own listing widens the vocabulary rather than replacing it. The crate-owned `marketdatakindcodeset` and `statecodeset` are immutable: only a canonical-preserving no-op succeeds, while replacement, widening, remapping or deletion is refused. `read --json` prints the `{"name": ..., "codes": [...]}` document `codesets/<name>.json` holds, which `write --codes` takes back.

Order matters in one direction only. A set is stated before a field names it, because `fields create Side utf8 --tag 54 --codes sidecodeset` is refused while the dictionary holds no `sidecodeset`; and a set is released after the last field lets go, because `codesets delete` refuses a set a field still reads by and names that field. Nothing else about a field changes with its vocabulary: `fields read Side` prints `codes sidecodeset`, one word, and restating the field never restates the members.

## Review and update a complete definition

Read JSON, edit the document, then replace it with `update --input`; this retains metadata that a positional replacement would omit. A case-only input name keeps the stored canonical spelling and filename, while a changed identity or referenced datatype is refused atomically.

```bash
yggdryl fix --root scratch/catalog components read Party --json > Party.json
# Edit Party.json, preserving its name, datatype, nullability, and identity.
yggdryl fix --root scratch/catalog components update --input Party.json
yggdryl fix --root scratch/catalog components read Party --json
```

Metadata changes refresh resolved references before publication. Delete dependents first; the group refers to its occurrence component and counter:

```bash
yggdryl fix --root scratch/catalog components delete Order
yggdryl fix --root scratch/catalog groups delete Parties
yggdryl fix --root scratch/catalog components delete Party
yggdryl fix --root scratch/catalog fields delete 448
yggdryl fix --root scratch/catalog fields delete 453
```

## Ingest and sync

`ingest PATH...` folds one or more Ullink `CBlock`s into all three categories; `sync DIR` folds another dictionary folder. Both fold, and the fold itself decides what stands: a declaration the dictionary already holds otherwise is passed over and named rather than overwritten, and everything else arrives or merges. `ingest` takes `--dialect NAME`, the id stamped into `FIX:sources` on every field, group, component and message a file produces, standard tags included, because membership means "this source speaks it", and recorded once as the file's entry in `sources.json` - the id, the file's name and the role its root's `type` names, a [`PluginSide`](../types/enum/pluginside.md) (`BUYS` for a `BuySideFIXCPluginCBlock`, `SELL` for a `SellSideFIXCPluginCBlock`, `UKNW` for neither); `sync` takes no `--dialect` at all, because a folder's fields already carry the membership they were written with, and its `sources.json` folds into this one's.

A path to `ingest` is a `.cfb` file, a folder or a glob pattern, handed to the core as the location it is. A folder folds the `.cfb` files directly inside it, the suffix in any case; a file folds whatever it is named. A quoted glob - `'cblocks/*.cfb'`, `'cblocks/**/*.cfb'` - is walked by the core itself: `*` stays inside one name, `**` spans folders, and a private entry is never matched. An unquoted one is expanded by the shell before the command ever sees it. Either way every file parses side by side, on every core, and folds into one staged dictionary in ascending URL order, resolved and committed once - so `cblocks/`, `cblocks/*.cfb` and the shell's own expansion of the glob all answer the same dictionary. A path naming nothing is refused, and so is a run whose paths hold no file at all. Without `--dialect` each file's own stem names its dialect (`MSFIX44.cfb` stamps `msfix44`); with it, every file folds under that one name instead. Where two files type one tag two ways, the first-sorting file's declaration is held: the later file's folds under it, counted as restated, where it states another precision of the held datatype - a CBlock's `float` against a `decimal128`, its `string` against a `ccy` - and is passed over and named with its own URL where it contradicts it, a `boolean` against an `int32`.

```bash
yggdryl fix --root scratch/catalog ingest cblocks/venue.cfb --dialect venue
yggdryl fix --root scratch/catalog ingest 'cblocks/*.cfb'
yggdryl fix --root scratch/catalog ingest 'cblocks/**/*.cfb' --annotate
yggdryl fix --root scratch/catalog ingest cblocks/a.cfb cblocks/b.cfb
yggdryl fix --root scratch/catalog ingest cblocks/
```

One file is one mutation: a file that cannot be read, is not a well-formed CBlock, or whose fold refuses rather than passing a declaration over is left out and named, contributing nothing, while every other file still folds and commits. A path naming nothing is refused, and so is a run whose paths hold no `.cfb` file at all; a location beside others that holds none is named in a reader warning, and a run whose every file is left out exits nonzero once each is named, committing nothing.

`sync` folds a folder holding another dictionary; a `.cfb`, or any location that is not a folder, is refused naming `yggdryl fix ingest` and the role the location turned out to be.

```bash
yggdryl fix --root scratch/catalog sync ../desk/config/fix
```

A run that folds something prints, in order: what the fold did, what the reader warned about while reading the files, what the fold passed over, which files it left out, then what the commit wrote - the commit compares every document with the one it would replace and writes only those whose bytes moved.

```text
✓ 2 file(s): 5 added, 3 merged (1 restated), 1 passed over 340ms
! 1 warning(s) while reading: what a file states that the reader could not keep as stated
· venue.cfb [venue] invalid cfb expression at byte 204: line 4, column 65: expected one of the CBlock types (string, char, integer, float, boolean, utc-date, utc-timestamp, utc-time-only) or a datatype name, got "widget" in "<vocabulary-tag name=\"9850\" alt=\"StartTime\" type=\"widget\">"; the tag is typed string, which every FIX datatype is on the wire
! 1 declaration(s) passed over: the dictionary already declares them otherwise
· venue.cfb [venue] invalid record value at masscancelrejectreason: expected the datatype boolean stored for masscancelrejectreason (532), got int32
✓ committed: 8 written, 0 unchanged, 0 removed
```

`(N restated)` counts the merged fields whose file declared another precision of the stored datatype, and prints only when there are some; `, N file(s) left out` closes the first line only when a file was left out, and `! N file(s) left out: each contributed nothing, and every other file still folded` then heads one `·` line per file - its name, then the refusal it was left out over. A line under the reader's warnings is the core's own sentence: the byte, the line and column it falls on, what was expected and what the file said, the element quoted as the file spells it, then, after the semicolon, what the reader did instead - here the tag is kept, typed string - prefixed by the file's name and the dialect the file was read for, each where the reader has one. The reader's and the passed-over lines print only when there is something to say, and the commit line only when the fold changed the store - folding nothing but what the dictionary already declares leaves them all silent. Under `--annotate` (or `GITHUB_ACTIONS`) each prints as a workflow warning instead of a `·` line - `fix reader` for what the reader warned about, `fix passed over` for what the fold passed over, `fix left out` for a file left out (`::warning title=fix left out::<url>: <reason>`) - `%`, `\r` and `\n` escaped so a reason spanning a document reads as one line:

```text
::warning title=fix reader::venue.cfb [venue] invalid cfb expression at byte 204: line 4, column 65: expected one of the CBlock types (string, char, integer, float, boolean, utc-date, utc-timestamp, utc-time-only) or a datatype name, got "widget" in "<vocabulary-tag name=\"9850\" alt=\"StartTime\" type=\"widget\">"; the tag is typed string, which every FIX datatype is on the wire
::warning title=fix passed over::file:///desk/cblocks/venue.cfb: invalid record value at masscancelrejectreason: expected the datatype boolean stored for masscancelrejectreason (532), got int32
```

`sync` counts its one folder as a `dictionary(s)` rather than a `file(s)`: `✓ 1 dictionary(s): 5 added, 3 merged, 0 passed over 12ms`. Membership never decides how a tag or a name resolves; the version a CBlock's root declares is read past, and a capture is read at the version its own rows or lines state. Every fold is staged and committed once: `ingest` leaves out by name a file it cannot read, parse or fold while every other file folds, `sync` leaves the dictionary exactly as it was when its source's own catalog does not validate, and native [storage](store.md) handles the resulting documents.

## Schema, check, and diff

`schema` renders the fixed capture row through [`fix_schema`](capture.md#the-columns-are-the-folded-names), the same native builder the codec and a reader's `schema()` answer with; `--out` writes native JSON. `--rowheader` prepends capture columns inferred from its regular expression, each named group typed by what its syntax can match: a group matching `2024-02-01 12:34:56.123456` is a microsecond UTC instant and not a string. A capture named after a FIX column is not carried in front and [fills that column](arrow.md#a-column-is-the-caller-speaking-per-row); a `timestamp` capture names no FIX column, so it is carried in front as context and never dates the message. An `mtime` capture leads no column: a read consumes it into each line's `currunix`, which is how `yggdryl::ULBRIDGE_ROWHEADER`, a [bridge log's own header](arrow.md#a-bridge-log-names-what-it-fills), dates each line it matches.

```bash
yggdryl fix --root config/fix schema --out fix-message.json
yggdryl fix --root config/fix schema --rowheader '^(?P<level>[A-Z]+)\s+'
yggdryl fix --root config/fix check
yggdryl fix --root config/fix diff ../desk/config/fix --annotate
```

`check` reports invalid catalog relationships and fails when a finding is an error. It counts each category, then the `codesets` it holds and the `codes` in them, and walks each set once rather than once per field that reads by it - a malformed record is one finding about one vocabulary, not one about each of the hundred fields naming it. A set no field reads by is a note, a value stated twice in one set a warning, and a record the reader stops at a failure, because a borrowed walk ends at a refusal and every code after it is invisible. It counts the `sources` entries of `sources.json` too: an id a definition names in `FIX:sources` that the catalog does not hold is a failure - `names source "venue", which sources.json does not hold` - and an entry no field or definition names is a note under `sources/<id>`, the tier an unread code set takes. `diff` compares category definitions and metadata against another catalog; it is read-only.

## Interactive use

With no command, `yggdryl fix` opens an interactive shell with the same category and code set operations, flags, and native dispatcher. Completion includes the command names, `codesets` among them, the category operations, definition names, and tags.

```bash
yggdryl fix --root config/fix
```

The prompt marks unsaved changes with `*`; `save` writes them, `help` shows the command tree, and `quit` or Ctrl-D exits. Leaving unsaved changes reports that fact; one-shot commands save successful changes automatically.

## Edges

- A catalog root holding no `codesets/`, `fields/`, `components/`, or `groups/` folder loads with the crate's built-in definitions; a read does not create it.
- `create` refuses a duplicate even when its supplied document is identical.
- `update` requires an existing identity and is a full replacement.
- Scalar fields require tags; a named definition whose document states none takes the tag derived from its name, inside `[100000, 1100000)`.
- Wire group counters remain separate `int32` fields. The built-in `metadata` (65036) Map group has no scalar counter; a map's length is its cardinality.
- Deleting a referenced field, component, or group fails before saving, and so does deleting a code set a field still reads by; `codesets delete` names that field.
- `fields create` and `fields update` refuse a `--codes` name the dictionary does not hold, so the set is written first and a field never names a vocabulary nothing states.
- `ingest` always folds, whatever it is pointed at: a declaration the dictionary already holds otherwise is passed over and named, and the rest still arrives or merges, so running it again over an unchanged file leaves the store as it was. The one exception is a definition the fold filed under another name of its structure, where a file states that name with fewer members than the structure: running that file again lands its narrower definition under the name once more, read by no message already folded, and a later file widening it again files the structure under that name in turn ([one structure is one definition](registry.md#what-a-source-says-otherwise-than-the-dictionary-is-passed-over)).
- A coarser datatype folds under the held one and is counted restated rather than passed over: unbounded text, which every FIX datatype is on the wire, beside anything, any number beside any other, an integer beside an enum, one byte layout beside another, a date beside a datetime whatever the zone, one time width beside another. Only a contradiction - a flag against a number, a time of day against an instant, two codes - is passed over. A field a file counts a repeating group by is the one datatype a fold changes: held as unbounded text, a float or another integer width, it is retyped `int32` with a warning, whichever file sorts first.
- What a file states in a way the reader cannot keep - a type word nothing reads, an attribute that will not split, a constraint naming a tag the file never declared - is named with its line and column and what the reader did instead, and the rest of the file still folds; a charset the file cannot be read in is transcribed or read as UTF-8, named the same way. A file that is not well-formed XML, or stops with an element open, is left out and named, and every other file still folds.
- An `ingest` path naming nothing is refused before anything folds, and a run whose paths - files, folders, globs - hold no file at all is refused before anything commits; a `sync` location that is not a folder - a `.cfb` among them, and one that does not exist yet reads as `unknown` - is refused before anything folds, naming `yggdryl fix ingest`.
- A `--dialect` that is empty or holds a quote, a backslash or a control character is refused before a byte of `ingest` folds; without one, each file's own stem names its dialect where the stem reads as an id - opening with an ASCII letter and holding no quote, backslash or control character, its percent escapes decoded, so `Morgan Stanley.cfb` stamps `morgan stanley` - and stamps nothing where it does not: the file folds as a bare vocabulary. `sync` takes no `--dialect`: its folder's fields already carry the membership they were written with.
- Two fields may hold one tag under two names; the bare tag answers the first holder, the store writes the holder first so it survives a reload, a listing filtered on that tag shows both, deleting the holder leaves the other alone on the tag, and neither learns the other's name.
- A file declaring a held tag under a name a third field holds as its canonical name - 541 as `MaturityDate2` where `MaturityDate` holds 541 and `MaturityDate2` holds 9999 - names that third field: the declaration merges into `MaturityDate2`, 541 stays `MaturityDate`'s, and every member of the file reading it reads `MaturityDate2`, so its messages fold rather than being passed over. A name another field holds only as an alias is not this case: the declaration stands beside the holder of its tag.
- A message member is the field it reads before the name it carries: where two files spell one name over two tags - `Urgency` over 61 in one and over 9252 in the other - the second file's member reading a tag the message already reads is that member, and one reading another tag stands beside it as `urgency2`, so nothing is passed over and the message reads both tags whichever file sorts first.
- A field named by nothing but its tag - a CBlock tag that neither an `alt` of its own nor a binding names - is unnamed: another file's field on that tag folds into it, and the first file naming the tag names it, so the members of both read one field.
- `FIX:sources` is written inside each field's document and what is known of the source once, in `sources.json`; the store keeps no per-dialect folder, so a `--dialect` on `create` changes one shard, and `sources.json` only where the id is new.
- Every location this tool is given resolves against the working directory before it becomes a URL, so a bare relative name works wherever a path is taken.
- A malformed code document on `codesets write`, invalid direction rules, unresolved references, a source id that cannot be one, a set name no store could file, and malformed native documents carry native located errors.
- A registry mutation is atomic; persistence publishes separate documents and follows the backend's write semantics.
- Interactive mode requires a terminal; piped one-shot commands emit plain text, coloured only when `FORCE_COLOR` or `CLICOLOR_FORCE` asks.

## Commands

```bash
cargo test -p yggdryl-cli --test fix
cargo test -p yggdryl-cli --test market      # `yggdryl market serve`: the refusals and every example its help states; `-- --ignored` hosts the live display under `--path /book` and at the root
cargo test -p yggdryl-cli --features iceberg --test market -- --ignored   # a ULBridge capture folded into an Iceberg table, then served
cargo clippy -p yggdryl-cli --all-targets -- -D warnings
cargo run -p yggdryl-cli -- fix groups create --help
cargo run -p yggdryl-cli -- market serve --help
```

## Performance

Measured in release mode on Windows, AMD Ryzen 5 150 with 12 logical CPUs and Rust 1.96. Each row launches 20 fresh processes; category reads include loading a local fixture with 100 scalar fields, the code sets they read by, one component, one group, and one message, then printing native JSON.

| Process invocation | Mean elapsed |
| --- | ---: |
| `fix --help` baseline | 49.875 ms |
| `fix fields read 54 --json` | 25.413 ms |
| `fix components read Order --json` | 20.648 ms |
| `fix components read Party --json` | 22.805 ms |
| `fix groups read Parties --json` | 28.229 ms |

These are end-to-end process timings with independent samples; subtracting the help row would not isolate registry cost. The full committed seed has a substantially larger graph than this CLI fixture; its load measurements are on the [store page](store.md#performance).

Regenerate from the repository root:

```bash
cargo bench -p yggdryl-cli --bench fix
```
