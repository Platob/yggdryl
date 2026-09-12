# CLI

`ygg fix` manages the native FIX catalog through explicit `fields`, `messages`, `components`, and `groups` command trees. Rust only: the wheel ships this compiled executable without a Python runtime in its execution path.

## Contract

| Aspect | Rule |
| --- | --- |
| Owner | `yggdryl-cli` parses arguments and renders results; the Rust registry owns schema validation, references, mutations, and persistence |
| Root | `--root`, default `config/fix`; relative locations resolve against the working directory; a folder holding no catalog opens with only the crate's own fields rather than failing |
| Categories | `fields`, `messages`, `components`, `groups` |
| Operations | Every category supports `list`, `read`, `create`, `update`, and `delete` |
| Writes | Successful one-shot mutations save automatically; an interactive session saves only with `save` |
| Keys | A field key is a decimal tag or a name; a named category's key is its definition name. The registry is one namespace: a key resolves the same way whatever dictionaries a definition belongs to, and no key spells an identity |
| Dialect | `--dialect NAME` stamps membership (`fix:branches`) on `ingest`, `sync`, `create`, and `update`; on `list` it is a filter; `read` and `delete` take none |
| Create | Refuses an existing name or field identity |
| Update | Replaces an existing definition completely, preserving identity; omitted metadata is removed |
| Delete | Refuses absence and live references |
| Enums | Scalar `fix:codes` metadata; `--codes` accepts its canonical JSON document |
| Output | Plain stable text when redirected; terminal styling only when supported and `NO_COLOR` is unset |
| Workflow | `--annotate`, also enabled by `GITHUB_ACTIONS`, prints workflow findings; failed checks and refused commands exit nonzero |

## Use

Read or search the committed catalog by category:

```bash
ygg fix --root config/fix fields list Party --limit 20
ygg fix --root config/fix fields read 453 --json
ygg fix --root config/fix groups read Parties --json
ygg fix --root config/fix components read Party
ygg fix --root config/fix messages list Order
```

`NoPartyIDs(453)` is an `int32` scalar; `Parties` is a separate List definition whose occurrence component is `Party`. Message reads show the native non-null Struct and its full `fix:msgtype` wire code.

## Install

The published wheel includes the native executable. From a checkout, Cargo runs the same binary:

```bash
pip install yggdryl
ygg fix --help
cargo run -p yggdryl-cli -- fix fields list Symbol
```

To include the CLI in a locally built wheel, stage it before building the wheel:

```bash
python scripts/stage_cli.py
maturin build --manifest-path python/Cargo.toml --out dist
```

## Four command trees

| Operation | Arguments and behavior |
| --- | --- |
| `<category> list [filter]` | Match name or decimal tag text, ignoring case; `--dialect NAME` keeps only definitions whose `fix:branches` names that dictionary; `--limit` defaults to 40 |
| `<category> read <key>` | Display one definition; `--json` emits a complete native `Field` document |
| `<category> create <name> <type>` | Create from the native datatype grammar and metadata flags |
| `<category> create --input <file>` | Create from one complete native `Field` JSON document |
| `<category> update <name> <type>` | Replace the complete existing definition |
| `<category> update --input <file>` | Replace from a complete native `Field` JSON document |
| `<category> delete <key>` | Delete the resolved definition, refusing dependents |

A field key is a decimal tag or a name; named categories use their definition name. A decimal key is a tag and never an identity: the canonical holder of the tag answers, then an alternate. Anything else is a name resolved under the registry's one fold, canonical name before alias, so `Desk_Value`, `deskvalue` and `DeskValue` reach one field. A colon-bearing key such as `5001:venue` is a name that nothing holds, and a path such as `Parties[0].PartyID` is not a key here. Reads, deletes and lists consult no membership: `--dialect` on `list` filters the rows by provenance, and nothing else about resolution changes with it.

### Definition flags

| Flag | Applies to |
| --- | --- |
| `--tag N` | Scalar fields, including group counters |
| `--counter N` | Groups; identifies an existing `int32` scalar field |
| `--component NAME` | Groups; identifies the existing occurrence component |
| `--msgtype CODE` | Messages; full nonempty wire text, including spaces |
| `--codes JSON` | Scalar inline enum metadata |
| `--dialect NAME` | Membership: a dictionary this definition belongs to, recorded in `fix:branches`; repeat the flag for several. Names are lowercased, deduplicated and sorted; an empty name or one carrying a comma is refused |
| `--description TEXT` | Definition metadata |
| `--required` | Non-null definition; message roots are always non-null |

`--input` replaces positional name/type and all definition flags. Quote datatype expressions containing spaces or shell metacharacters; a group occurrence must be a non-null Struct.

```bash
ygg fix --root scratch/catalog fields create NoPartyIDs int32 --tag 453
ygg fix --root scratch/catalog fields create PartyID utf8 --tag 448
ygg fix --root scratch/catalog components create Party 'struct<PartyID: utf8>' --required
ygg fix --root scratch/catalog groups create Parties 'list<Party: struct<PartyID: utf8> not null>' --counter 453 --component Party
ygg fix --root scratch/catalog messages create Order 'struct<ClOrdID: utf8>' --msgtype D
ygg fix --root scratch/catalog fields create Side utf8 --tag 54 --codes '{"codes":[{"value":"1","name":"Buy"},{"value":"2","name":"Sell"}]}'
ygg fix --root scratch/catalog fields create DeskValue int32 --tag 5001 --dialect venue --dialect Desk
ygg fix --root scratch/catalog fields read Desk_Value
ygg fix --root scratch/catalog fields list --dialect desk
```

The read shows `identity -630917675`, the signed XXH32 of the tag and the folded name, and `dialects desk, venue`; the listing filtered on `desk` holds that one row. A field's identity is its tag and its name, so a second field on a held tag under another name is a new definition beside the holder: `fields create OtherName int64 --tag 5001` succeeds, `fields read OtherName` answers it, the bare `5001` keeps answering `DeskValue`, whose `aliases` entry now names `OtherName`, and `fields list 5001` shows both rows. The same folded name on the same tag - `desk_value` with `--tag 5001` - is the existing identity and is refused, as is a held name on another tag. `update` replaces membership with what it states: an update without `--dialect` leaves the field a member of nothing.

A datatype expression embeds its child definitions. To preserve explicit canonical field/component/group references, use a resolved native `Field` document, such as the output of `read --json`; the compact unresolved placeholders in [folder storage](store.md#compact-references) are handled by the folder loader.

## Review and update a complete definition

Read JSON, edit the document, then replace it with `update --input`; this retains metadata that a positional replacement would omit. A case-only input name keeps the stored canonical spelling and filename, while a changed identity or referenced datatype is refused atomically.

```bash
ygg fix --root scratch/catalog components read Party --json > Party.json
# Edit Party.json, preserving its name, datatype, nullability, and identity.
ygg fix --root scratch/catalog components update --input Party.json
ygg fix --root scratch/catalog components read Party --json
```

Metadata changes refresh resolved references before publication. Delete dependents first; the group refers to its occurrence component and counter:

```bash
ygg fix --root scratch/catalog messages delete Order
ygg fix --root scratch/catalog groups delete Parties
ygg fix --root scratch/catalog components delete Party
ygg fix --root scratch/catalog fields delete 448
ygg fix --root scratch/catalog fields delete 453
```

## Ingest and sync

`ingest` reads an Ullink CBlock into all four categories, replacing matching definitions by default; `--merge` uses the native metadata fold. `sync` always folds a catalog directory or `.cfb` file, and refuses other location types. Both take `--dialect NAME`: the dictionary name stamped into `fix:branches` on every field, group, component and message the file produces, standard tags included, because membership means "this dictionary speaks it".

```bash
ygg fix --root scratch/catalog ingest cblocks/venue.cfb --dialect venue
ygg fix --root scratch/catalog ingest cblocks/venue.cfb --dialect desk --merge
ygg fix --root scratch/catalog sync ../desk/config/fix
ygg fix --root scratch/catalog sync cblocks/venue.cfb
ygg fix --root scratch/catalog sync cblocks/venue.cfb --dialect desk
```

After the first line, `fields read 10001` shows `dialects venue` and `fields list --dialect venue` lists every tag the file declared, `8` and `35` among them; the merge under `desk` unions the membership to `desk, venue`. An `ingest` without `--dialect` stamps nothing. A `.cfb` synchronization without `--dialect` takes the file's stem through [`add_cfb_file`](registry.md#folding-a-second-source-in) where it reads as a name - non-empty and opening with a letter - so `venue.cfb` stamps `venue`, and a stem that is not a name stamps nothing rather than refusing. A folder synchronization takes no name: its fields carry the membership they were written with, and the fold unions it onto what the catalog holds. Membership never decides how a tag or a name resolves; the version a CBlock's root declares is read past, and a capture read under it pins [`with_version`](message.md). Every in-memory fold is atomic, and native [storage](store.md) handles the resulting documents.

## Schema, check, and diff

`schema` renders the fixed capture row through [`fix_schema`](capture.md#the-columns-are-the-folded-names), the same native builder the codec and a reader's `schema()` answer with; `--out` writes native JSON. `--rowheader` prepends capture columns inferred from its regular expression, each named group typed by what its syntax can match: a group matching `2024-02-01 12:34:56.123456` is a microsecond UTC instant and not a string. A capture named after a FIX column is not carried in front; `timestamp` is the row's clock and [lands in that column](arrow.md#a-column-is-the-caller-speaking-per-row), and `yggdryl::ULBRIDGE_ROWHEADER` is a [bridge log's own header](arrow.md#a-bridge-log-names-what-it-fills) written that way.

```bash
ygg fix --root config/fix schema --out fix-message.json
ygg fix --root config/fix schema --rowheader '^(?P<level>[A-Z]+)\s+'
ygg fix --root config/fix check
ygg fix --root config/fix diff ../desk/config/fix --annotate
```

`check` reports invalid catalog relationships and fails when a finding is an error. `diff` compares category definitions and metadata against another catalog; it is read-only.

## Interactive use

With no command, `ygg fix` opens an interactive shell with the same category operations, flags, and native dispatcher. Completion includes category names, operations, definition names, and tags.

```bash
ygg fix --root config/fix
```

The prompt marks unsaved changes with `*`; `save` writes them, `help` shows the command tree, and `quit` or Ctrl-D exits. Leaving unsaved changes reports that fact; one-shot commands save successful changes automatically.

## Edges

- A catalog root holding no `fields/`, `components/`, `groups/`, or `messages/` folder loads with only the crate's own fields; a read does not create it.
- `create` refuses a duplicate even when its supplied document is identical.
- `update` requires an existing identity and is a full replacement.
- Scalar fields require tags; a named definition whose document states none takes the tag derived from its name, inside `[100000, 1100000)`.
- Group count fields remain separate `int32` values and are not replaced by lists.
- Deleting a referenced field, component, or group fails before saving.
- `ingest` creates by default and merges only when asked, because a new counterparty is a new catalog and a revised configuration is a change to one that exists; `sync` always folds.
- `sync` of a location that is neither a folder nor a `.cfb` is refused, naming the location and the role it turned out to be; a location that does not exist yet is `unknown` and refused the same way.
- `sync` of a `.cfb` whose stem does not read as a name - empty, or not opening with a letter - stamps no membership, exactly as [`FixField::from_cfb_file`](registry.md#folding-a-second-source-in) stamps none; a `--dialect` that is empty or carries a comma is refused before a byte is folded, on `ingest` and `sync` alike.
- Two fields may hold one tag under two names; the bare tag answers the first holder, the store writes the holder first so it survives a reload, and a listing filtered on that tag shows both. Deleting the holder leaves the other alone on the tag.
- `fix:branches` is written inside each field's document; the store keeps no manifest and no per-dialect folder, so a `--dialect` on `create` changes one shard and nothing else.
- Every location this tool is given resolves against the working directory before it becomes a URL, so a bare relative name works wherever a path is taken.
- Invalid inline enums, unresolved references, a dialect name that cannot be a membership, and malformed native documents carry native located errors.
- A registry mutation is atomic; persistence publishes separate documents and follows the backend's write semantics.
- Interactive mode requires a terminal; piped one-shot commands emit plain text.

## Commands

```bash
cargo test -p yggdryl-cli --test fix
cargo clippy -p yggdryl-cli --all-targets -- -D warnings
cargo run -p yggdryl-cli -- fix groups create --help
```

## Performance

Measured in release mode on Windows, AMD Ryzen 5 150 with 12 logical CPUs and Rust 1.96. Each row launches 20 fresh processes; category reads include loading a local fixture with 100 scalar fields, inline enums, one component, one group, and one message, then printing native JSON.

| Process invocation | Mean elapsed |
| --- | ---: |
| `fix --help` baseline | 49.875 ms |
| `fix fields read 54 --json` | 25.413 ms |
| `fix messages read Order --json` | 20.648 ms |
| `fix components read Party --json` | 22.805 ms |
| `fix groups read Parties --json` | 28.229 ms |

These are end-to-end process timings with independent samples; subtracting the help row would not isolate registry cost. The full committed seed has a substantially larger graph than this CLI fixture; its load measurements are on the [store page](store.md#performance).

Regenerate from the repository root:

```bash
cargo bench -p yggdryl-cli --bench fix
```
