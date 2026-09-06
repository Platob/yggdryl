# CLI

`ygg` is everything a desk does to a [dictionary](registry.md) from a terminal: read it, search it, change it, ingest a counterparty's configuration into it, print the [row shape](capture.md) it produces, and check that what came out is right. Rust only.

Two audiences, one implementation. A person at a prompt and a workflow gating a pull request run the same code, and the only difference is that one of them gets colour.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | argument parsing, terminal drawing, the interactive line editor; no protocol logic of its own |
| Binary | `ygg`, from the `yggdryl-cli` workspace member, so a library consumer carries none of it |
| Root | `--root`, defaulting to `config/fix`; a folder with no dictionary in it opens empty rather than failing |
| Writes | only `set`, `rm` and `ingest`, and only after the command that changed something asked to save |
| Colour | on where stdout is a terminal and `NO_COLOR` is unset; box drawing and animation follow the same test |
| Annotates | `--annotate`, on by itself under `GITHUB_ACTIONS`; findings become workflow annotations and a failure becomes a non-zero exit |

## Use

```bash
cargo run -p yggdryl-cli -- --root config/fix list symbol
```

| Command | What it does |
| --- | --- |
| `list [filter]` | every field whose name or tag contains the filter |
| `show <key>` | one field in full: identity, lineage, code set |
| `set <name> <type> --tag N` | create or replace a field |
| `rm <key>` | remove a field |
| `ingest <path.cfb>` | read an Ullink `CBlock` in, creating or `--merge`ing |
| `schema` | the one row shape a whole capture lands in |
| `check` | what the dictionary is wrong about |
| `diff <other>` | what changed against another dictionary |
| `shell` | all of the above, interactively, with completion |

A key is a tag, an identifier (`5001:cme`), a name, or a branch-qualified dotted path - the same four the registry takes, coerced by the same code.

## The schema command dumps the row

`schema` prints the fixed row a capture lands in, built by [`fix_schema`](capture.md#the-columns-are-the-tags) from the dictionary that was loaded. A schema printed here and a schema a reader answers `schema()` with are the same object built by the same code, never two spellings of one intention.

```bash
cargo run -p yggdryl-cli -- schema --out schemas/fix-message.json
```

With `--rowheader`, the columns a capture supplies lead the row. The regex is the one a text read frames lines with, and its named captures become columns ahead of the FIX ones, typed by what their syntax can match - so a group matching `2024-02-01 12:34:56.123456` becomes a microsecond UTC timestamp column and not a string.

```bash
cargo run -p yggdryl-cli -- schema \
  --rowheader '^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{6})\s+\[(?P<threadname>[^\]]+)\]\s+(?P<level>[A-Z]+)\s+'
```

JSON when it goes to a file, because that is what a downstream consumer reads and it is the crate's own serialization rather than a rendering of it. A table when it goes to a terminal, because nobody reads six thousand lines of JSON at a prompt.

## Check and diff are for a workflow

`check` reports what a dictionary is wrong about and `diff` reports what one changed against another. Both print a table at a prompt and workflow annotations under a runner, and `check` exits non-zero when a finding is an error - so a pull request that breaks the dictionary fails on the same command a person debugs with.

```bash
cargo run -p yggdryl-cli -- check
cargo run -p yggdryl-cli -- diff config/fix --annotate
```

A rename shows as a rename rather than as an addition beside a removal, because the two dictionaries are compared field by field on identity.

## The shell completes from the dictionary

A dictionary of six thousand fields is not something anyone remembers the spelling of, so the shell completes from the dictionary itself rather than from a fixed word list - tags, names, and the commands that take them. Tab completes the common prefix and shows the alternatives; the arrows walk what has already been asked; `ctrl-d` leaves, and leaving with unsaved changes says so.

```bash
cargo run -p yggdryl-cli -- shell
```

The prompt carries a `*` while anything is unsaved. Every shell command is the same function the flag reaches, so there is no second path where an interactive command could drift from the one it mirrors.

## Edges

- A folder holding no `primitive/` or `nested/` tree opens as an empty dictionary; a folder holding the retired layout is refused with the URL named.
- `ingest` creates by default and merges only when asked, because a new counterparty is a new dictionary and a revised configuration is a change to one that exists.
- `set` lower-cases the name it is given, because a dictionary folds names once and a field spelled two ways is one field.
- Redirected or piped, every command prints plain, stable, greppable text: no colour, no box drawing, no spinner.
- `shell` needs a terminal; there is nothing to edit a line with where there is none.

## Commands

```bash
cargo build -p yggdryl-cli
cargo run -p yggdryl-cli -- --help
cargo clippy -p yggdryl-cli --all-targets
```
