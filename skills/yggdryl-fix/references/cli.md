# yggdryl-fix: the `yggdryl fix` command line

`yggdryl` is a compiled Rust executable shipped inside the Python wheel
(`pip install yggdryl` puts it on `PATH`); from a checkout, `cargo run -p
yggdryl-cli -- fix ...` runs the same binary. It manages a stored dictionary -
the shard tree `FixRegistry::from_handle` reads - and validates it; it does not
decode captures (that is the codec's job in code).

## Global flags

| Flag | Meaning |
| --- | --- |
| `--root DIR`, `-r DIR` | the dictionary folder; default `config/fix`, resolved against the working directory. A folder holding no catalog opens with the crate's built-in definitions; a read creates nothing |
| `--annotate` | print findings as GitHub workflow annotations (on by itself under `GITHUB_ACTIONS`) |
| (no command) | an interactive shell with the same commands; `save` writes, `*` in the prompt marks unsaved changes |

One-shot mutations save on success; a refused command exits nonzero and
changes nothing.

## Read and search

```bash
yggdryl fix --root config/fix fields list Party --limit 20
yggdryl fix --root config/fix fields read 453 --json
yggdryl fix --root scratch/catalog fields read Desk_Value
yggdryl fix --root config/fix groups read Parties --json
yggdryl fix --root config/fix components read Party
yggdryl fix --root config/fix components list Order
yggdryl fix --root scratch/catalog fields list --dialect venue
```

A field key is a decimal tag or a name folded like every lookup (`Desk_Value`,
`deskvalue`, `DeskValue` are one key); a named category's key is its definition
name. A decimal key is always a tag, never an identity; a path such as
`Parties[0].PartyID` is not a key. `--dialect` filters a `list` by the
`FIX:branches` membership and changes no resolution.

## Create, update, delete

```bash
yggdryl fix --root scratch/catalog fields create NoPartyIDs int32 --tag 453
yggdryl fix --root scratch/catalog fields create PartyID utf8 --tag 448
yggdryl fix --root scratch/catalog components create Party 'struct<PartyID: utf8>' --required
yggdryl fix --root scratch/catalog groups create Parties 'serie<Party: struct<PartyID: utf8> not null>' --counter 453 --component Party
yggdryl fix --root scratch/catalog components create Order 'struct<ClOrdID: utf8>' --msgtype D --identifiers ClOrdID
yggdryl fix --root scratch/catalog fields create DeskValue int32 --tag 5001 --dialect venue --dialect desk
```

| Flag | Applies to |
| --- | --- |
| `--tag N` | scalar fields (group counters included) |
| `--counter N`, `--component NAME` | Serie/LargeSerie groups: the `int32` counter tag and the occurrence component |
| `--msgtype CODE` | components: makes the component a message |
| `--identifiers MEMBER` | components: repeat per direct scalar identifier (name, alias or tag) |
| `--codes NAME` | scalar fields: the code set the field reads by; the set must already exist |
| `--directions JSON` | tag 385: its `FIX:directions` rules |
| `--dialect NAME` | membership stamped in `FIX:branches`; repeat for several |
| `--description TEXT`, `--required` | definition metadata; non-null definition |
| `--input FILE` | one complete native `Field` JSON document instead of the positional name, type and flags |

`update` replaces a definition whole - omitted metadata is removed - so edit
the `read --json` document and feed it back:

```bash
yggdryl fix --root scratch/catalog components read Party --json > Party.json
yggdryl fix --root scratch/catalog components update --input Party.json
```

Delete dependents before what they reference; a referenced definition is
refused:

```bash
yggdryl fix --root scratch/catalog components delete Order
yggdryl fix --root scratch/catalog groups delete Parties
yggdryl fix --root scratch/catalog components delete Party
yggdryl fix --root scratch/catalog fields delete 448
```

## Code sets

A vocabulary is the dictionary's, stated once under its name and before any
field names it:

```bash
yggdryl fix --root config/fix codesets list side --limit 20
yggdryl fix --root config/fix codesets read sidecodeset --json
yggdryl fix --root scratch/catalog codesets write sidecodeset --codes '[{"value":"1","name":"Buy"},{"value":"2","name":"Sell"}]'
yggdryl fix --root scratch/catalog fields create Side utf8 --tag 54 --codes sidecodeset
yggdryl fix --root scratch/catalog codesets write sidecodeset --merge --codes '[{"value":"7","name":"Undisclosed"}]'
```

`write` replaces a set; `--merge` folds by wire value and keeps every spelling
as an alias. `codesets delete` refuses a set a field still reads by and names
that field. The crate-owned `msgcatcodeset` and `statecodeset` are immutable.

## Ingest, sync, schema, check, diff

```bash
yggdryl fix --root scratch/catalog ingest cblocks/venue.cfb --dialect venue
yggdryl fix --root scratch/catalog ingest 'cblocks/*.cfb'
yggdryl fix --root scratch/catalog ingest 'cblocks/**/*.cfb' --annotate
yggdryl fix --root scratch/catalog sync ../desk/config/fix
yggdryl fix --root config/fix schema --out fix-message.json
yggdryl fix --root config/fix schema --rowheader '^(?P<level>[A-Z]+)\s+'
yggdryl fix --root config/fix check
yggdryl fix --root config/fix diff ../desk/config/fix --annotate
```

| Command | Does |
| --- | --- |
| `ingest PATH...` | folds one or more `.cfb` files or glob patterns into all three categories, in one staged dictionary and one commit |
| `sync DIR` | always folds another dictionary folder into this one; no `--dialect`, and a `.cfb` is refused naming `ingest` |
| `schema` | renders the fixed capture row (`fix_schema`); `--rowheader` prepends the typed captures a row header yields; `--out` writes native JSON |
| `check` | validates relationships and code sets; exits nonzero on an error finding |
| `diff DIR` | compares definitions and metadata against another catalog; read-only |

`ingest` takes one or more `.cfb` files or glob patterns - quote a glob
(`'cblocks/*.cfb'`, `'cblocks/**/*.cfb'`) to have the core walk it, or let the
shell expand one; either way every matched file parses side by side and folds
into one staged dictionary in ascending URL order. Without `--dialect` each
file's own stem names its dialect (`MSFIX44.cfb` stamps `msfix44`); where two
files disagree about one tag, the first-sorting file's declaration is held and
the later one is passed over and named with its file. A path matching
nothing, or naming a folder, is refused.

A run that folds prints, in order: what the fold did, what was passed over
(only when something was), then what the commit wrote (only when the store
changed; a commit writes only the documents whose bytes moved):

```text
✓ 2 file(s): 5 added, 3 merged, 1 passed over 340ms
! 1 declaration(s) passed over: the dictionary already declares them otherwise
· venue.cfb [venue] invalid record value at masscancelrejectreason: expected the datatype utf8 stored for masscancelrejectreason (532), got int32
✓ committed: 8 written, 0 unchanged, 0 removed
```

Under `--annotate` (or `GITHUB_ACTIONS`) each drop prints as a workflow
warning instead of the `·` line, `%`, `\r` and `\n` escaped:

```text
::warning title=fix passed over::file:///desk/cblocks/venue.cfb: invalid record value at masscancelrejectreason: expected the datatype utf8 stored for masscancelrejectreason (532), got int32
```

`sync` names its source `dictionary(s)` rather than `file(s)`, and takes no
`--dialect`: a folder's fields already carry the membership they were written
with.

## Gotchas

- `--root` defaults to `config/fix` relative to the working directory: run from
  the folder holding it, or pass `--root` explicitly.
- `create` refuses an existing name or identity even when identical; `update`
  refuses absence.
- A second field on a held tag under another name is a new definition beside
  the holder; the bare tag keeps answering the first holder.
- The stored tree is generated: prefer these commands (or `FixRegistry.commit`)
  over hand edits, and run `check` before committing a change.
- Output is a table for people; for tools use `read --json`, `codesets read
  --json` or `schema --out FILE`. A reader closing the pipe early (`| head`)
  ends the command quietly with status 0.
