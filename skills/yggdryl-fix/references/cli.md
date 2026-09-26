# yggdryl-fix: the `ygg fix` command line

`ygg` is a compiled Rust executable shipped inside the Python wheel
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
ygg fix --root config/fix fields list Party --limit 20
ygg fix --root config/fix fields read 453 --json
ygg fix --root scratch/catalog fields read Desk_Value
ygg fix --root config/fix groups read Parties --json
ygg fix --root config/fix components read Party
ygg fix --root config/fix components list Order
ygg fix --root scratch/catalog fields list --dialect venue
```

A field key is a decimal tag or a name folded like every lookup (`Desk_Value`,
`deskvalue`, `DeskValue` are one key); a named category's key is its definition
name. A decimal key is always a tag, never an identity; a path such as
`Parties[0].PartyID` is not a key. `--dialect` filters a `list` by the
`FIX:branches` membership and changes no resolution.

## Create, update, delete

```bash
ygg fix --root scratch/catalog fields create NoPartyIDs int32 --tag 453
ygg fix --root scratch/catalog fields create PartyID utf8 --tag 448
ygg fix --root scratch/catalog components create Party 'struct<PartyID: utf8>' --required
ygg fix --root scratch/catalog groups create Parties 'serie<Party: struct<PartyID: utf8> not null>' --counter 453 --component Party
ygg fix --root scratch/catalog components create Order 'struct<ClOrdID: utf8>' --msgtype D --identifiers ClOrdID
ygg fix --root scratch/catalog fields create DeskValue int32 --tag 5001 --dialect venue --dialect desk
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
ygg fix --root scratch/catalog components read Party --json > Party.json
ygg fix --root scratch/catalog components update --input Party.json
```

Delete dependents before what they reference; a referenced definition is
refused:

```bash
ygg fix --root scratch/catalog components delete Order
ygg fix --root scratch/catalog groups delete Parties
ygg fix --root scratch/catalog components delete Party
ygg fix --root scratch/catalog fields delete 448
```

## Code sets

A vocabulary is the dictionary's, stated once under its name and before any
field names it:

```bash
ygg fix --root config/fix codesets list side --limit 20
ygg fix --root config/fix codesets read sidecodeset --json
ygg fix --root scratch/catalog codesets write sidecodeset --codes '[{"value":"1","name":"Buy"},{"value":"2","name":"Sell"}]'
ygg fix --root scratch/catalog fields create Side utf8 --tag 54 --codes sidecodeset
ygg fix --root scratch/catalog codesets write sidecodeset --merge --codes '[{"value":"7","name":"Undisclosed"}]'
```

`write` replaces a set; `--merge` folds by wire value and keeps every spelling
as an alias. `codesets delete` refuses a set a field still reads by and names
that field. The crate-owned `msgcatcodeset` is immutable.

## Ingest, sync, schema, check, diff

```bash
ygg fix --root scratch/catalog ingest cblocks/venue.cfb --dialect venue
ygg fix --root scratch/catalog ingest cblocks/venue.cfb --dialect desk --merge
ygg fix --root scratch/catalog sync ../desk/config/fix
ygg fix --root config/fix schema --out fix-message.json
ygg fix --root config/fix schema --rowheader '^(?P<level>[A-Z]+)\s+'
ygg fix --root config/fix check
ygg fix --root config/fix diff ../desk/config/fix --annotate
```

| Command | Does |
| --- | --- |
| `ingest FILE.cfb` | reads an Ullink CBlock into all three categories; creates by default, `--merge` folds |
| `sync DIR\|FILE.cfb` | always folds another catalog folder or CBlock into this one |
| `schema` | renders the fixed capture row (`fix_schema`); `--rowheader` prepends the typed captures a row header yields; `--out` writes native JSON |
| `check` | validates relationships and code sets; exits nonzero on an error finding |
| `diff DIR` | compares definitions and metadata against another catalog; read-only |

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
  --json` or `schema --out FILE`. Piping a long table into `head` can end the
  process with a broken-pipe panic: redirect to a file instead.
