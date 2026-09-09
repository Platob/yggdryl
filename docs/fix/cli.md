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
| `<category> list [filter]` | Match name or decimal tag text, ignoring case; `--branch` filters; `--limit` defaults to 40 |
| `<category> read <key>` | Display one definition; `--json` emits a complete native `Field` document |
| `<category> create <name> <type>` | Create from the native datatype grammar and metadata flags |
| `<category> create --input <file>` | Create from one complete native `Field` JSON document |
| `<category> update <name> <type>` | Replace the complete existing definition |
| `<category> update --input <file>` | Replace from a complete native `Field` JSON document |
| `<category> delete <key>` | Delete the resolved definition, refusing dependents |

Fields accept a tag, identifier such as `5001:cme`, scalar name, or resolvable path at read intake; named categories use their definition name. Without `--branch`, reads and deletes use the registry's best match and lists include all branches; create/update positional input defaults to the standard branch. Pass `--branch ''` to pin the standard branch on a shell that preserves empty arguments.

### Definition flags

| Flag | Applies to |
| --- | --- |
| `--tag N` | Scalar fields, including group counters |
| `--counter N` | Groups; identifies an existing `int32` scalar field |
| `--component NAME` | Groups; identifies the existing occurrence component |
| `--msgtype CODE` | Messages; full nonempty wire text, including spaces |
| `--codes JSON` | Scalar inline enum metadata |
| `--branch NAME` | Named dialect, subject to native tag-range rules |
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
```

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

`ingest` reads an Ullink CBlock into all four categories, replacing matching definitions by default; `--merge` uses the native metadata fold. `sync` always folds a catalog directory or `.cfb` file, and refuses other location types.

```bash
ygg fix --root scratch/catalog ingest cblocks/venue.cfb --branch venue
ygg fix --root scratch/catalog ingest cblocks/venue.cfb --branch venue --merge
ygg fix --root scratch/catalog sync ../desk/config/fix
ygg fix --root scratch/catalog sync cblocks/venue.cfb
```

A `.cfb` synchronization without `--branch` derives the dialect from its filename stem through `add_cfb_file`; folder synchronization keeps the branches declared by that catalog. Every in-memory fold is atomic, and native [storage](store.md) handles the resulting documents.

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
- Scalar fields require tags; named definitions do not acquire synthetic tags.
- Group count fields remain separate `int32` values and are not replaced by lists.
- Deleting a referenced field, component, or group fails before saving.
- `ingest` creates by default and merges only when asked, because a new counterparty is a new catalog and a revised configuration is a change to one that exists; `sync` always folds.
- `sync` of a location that is neither a folder nor a `.cfb` is refused, naming the location and the role it turned out to be; a location that does not exist yet is `unknown` and refused the same way.
- `sync` of a `.cfb` whose stem is not a branch is refused rather than folded into one, exactly as [`FixField::from_cfb_file`](registry.md#folding-a-second-source-in) refuses it.
- Every location this tool is given resolves against the working directory before it becomes a URL, so a bare relative name works wherever a path is taken.
- Invalid inline enums, unresolved references, contradictory branches, and malformed native documents carry native located errors.
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
