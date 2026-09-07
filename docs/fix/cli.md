# CLI

`ygg` is the yggdryl command line, one namespace per subcommand. `ygg fix` is everything a desk does to a [dictionary](registry.md) from a terminal: read it, search it, change it, ingest a counterparty's configuration into it, print the [row shape](capture.md) it produces, and check that what came out is right. Rust only.

Two audiences, one implementation. A person at a prompt and a workflow gating a pull request run the same code, and the only difference is that one of them gets colour.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | argument parsing, terminal drawing, the interactive line editor; no protocol logic of its own |
| Binary | `ygg`, from the `yggdryl-cli` workspace member, so a library consumer carries none of it |
| Ships in | the `yggdryl` wheel, as `<version>.data/scripts/ygg`, which an installer puts on PATH; `pip install yggdryl` therefore answers `ygg` with the compiled binary and no Python in the run path |
| Namespace | one subcommand per namespace, each owning its own verbs and its own state; `fix` is the only one today |
| Root | `ygg fix --root`, defaulting to `config/fix`; global to the namespace, so it may be given before or after the verb; a folder with no dictionary in it opens empty rather than failing |
| Writes | only `set`, `rm` and `ingest`, and only after the command that changed something asked to save |
| Colour | on where stdout is a terminal and `NO_COLOR` is unset; box drawing and animation follow the same test |
| Annotates | `--annotate`, on by itself under `GITHUB_ACTIONS`; findings become workflow annotations and a failure becomes a non-zero exit |

## Install

`pip install yggdryl` puts `ygg` on PATH. The wheel carries the compiled binary beside the extension module, so the command is the same native executable `cargo build` produces and starts no interpreter to run.

```bash
pip install yggdryl
ygg fix --root config/fix list symbol
```

From a checkout, `cargo run` is the same tool without installing anything.

```bash
cargo run -p yggdryl-cli -- fix --root config/fix list symbol
```

Building a wheel that answers `ygg` is two steps, because maturin copies the binary rather than building it: `scripts/stage_cli.py` puts it in `python/wheel-data/scripts/`, and maturin copies that directory into the wheel. Staging nothing is a working build with no command in it, which is what a contributor who only wants the extension module gets.

```bash
python scripts/stage_cli.py
maturin build --manifest-path python/Cargo.toml --out dist
```

## Use

| Command | What it does |
| --- | --- |
| `list [filter]` | every field whose name or tag contains the filter |
| `show <key>` | one field in full: identity, lineage, code set |
| `set <name> <type> --tag N` | create or replace a field |
| `rm <key>` | remove a field |
| `ingest <path.cfb>` | read an Ullink `CBlock` in, creating or `--merge`ing |
| `sync <source>` | fold another dictionary or a `CBlock` in, whichever the location is |
| `schema` | the one row shape a whole capture lands in |
| `check` | what the dictionary is wrong about |
| `diff <other>` | what changed against another dictionary |
| *no verb* | all of the above, interactively, with completion |

A key is a tag, an identifier (`5001:cme`), a name, or a branch-qualified dotted path - the same four the registry takes, coerced by the same code.

## The schema command dumps the row

`schema` prints the fixed row a capture lands in, built by [`fix_schema`](capture.md#the-columns-are-the-tags) from the dictionary that was loaded. A schema printed here and a schema a reader answers `schema()` with are the same object built by the same code, never two spellings of one intention.

```bash
cargo run -p yggdryl-cli -- fix schema --out schemas/fix-message.json
```

With `--rowheader`, the columns a capture supplies lead the row. The regex is the one a text read frames lines with, and its named captures become columns ahead of the FIX ones, typed by what their syntax can match - so a group matching `2024-02-01 12:34:56.123456` becomes a microsecond UTC timestamp column and not a string.

```bash
cargo run -p yggdryl-cli -- fix schema \
  --rowheader '^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{6})\s+\[(?P<threadname>[^\]]+)\]\s+(?P<level>[A-Z]+)\s+'
```

JSON when it goes to a file, because that is what a downstream consumer reads and it is the crate's own serialization rather than a rendering of it. A table when it goes to a terminal, because nobody reads six thousand lines of JSON at a prompt.

## Sync reads whatever the location is

`sync` folds another source into the dictionary, and the location decides which reader answers it: a folder is another dictionary, a `.cfb` is one counterparty's vocabulary, and anything else is refused rather than guessed at. A `CBlock` declares no media type of its own, so the extension is the only thing that says what the bytes are before they are read.

```bash
cargo run -p yggdryl-cli -- fix sync ../desk/config/fix
cargo run -p yggdryl-cli -- fix sync cblocks/bloomberg.cfb
```

Both arrive through [`FixRegistry::add_fields`](registry.md#folding-a-second-source-in), so a tag this dictionary lacks is added and one it holds keeps every key only it declares - and because that fold is one mutation, a source it refuses leaves the dictionary exactly as it was. The counts printed are what was added and what was folded.

With no `--branch`, a `CBlock`'s stem names the dialect its user-range tags belong to: `bloomberg.cfb` reads into the branch `bloomberg`. A folder says nothing to `--branch`; the fields it holds carry the branch they were written with.

## Check and diff are for a workflow

`check` reports what a dictionary is wrong about and `diff` reports what one changed against another. Both print a table at a prompt and workflow annotations under a runner, and `check` exits non-zero when a finding is an error - so a pull request that breaks the dictionary fails on the same command a person debugs with.

```bash
cargo run -p yggdryl-cli -- fix check
cargo run -p yggdryl-cli -- fix diff config/fix --annotate
```

A rename shows as a rename rather than as an addition beside a removal, because the two dictionaries are compared field by field on identity.

## Naming no verb opens the shell

`ygg fix` with no verb is the interactive shell rather than a usage error: every verb above is reachable from inside it, so a caller who names none is asking for all of them. A dictionary of six thousand fields is not something anyone remembers the spelling of, so the shell completes from the dictionary itself rather than from a fixed word list - tags, names, and the commands that take them. Tab completes the common prefix and shows the alternatives; the arrows walk what has already been asked; `ctrl-d` leaves, and leaving with unsaved changes says so.

```bash
cargo run -p yggdryl-cli -- fix
```

The prompt carries a `*` while anything is unsaved. Every shell command is the same function the flag reaches, so there is no second path where an interactive command could drift from the one it mirrors.

## Edges

- A folder holding no `primitive/` or `nested/` tree opens as an empty dictionary; a folder holding the retired layout is refused with the URL named.
- `ingest` creates by default and merges only when asked, because a new counterparty is a new dictionary and a revised configuration is a change to one that exists; `sync` always folds, because keeping in step with a source is not the same as taking one in for the first time.
- `sync` of a location that is neither a folder nor a `.cfb` -> refused, naming the location and the role it turned out to be; a location that does not exist yet is `unknown` and refused the same way.
- `sync` of a `.cfb` whose stem is not a branch -> refused rather than folded into one, exactly as [`FixField::from_cfb_file`](registry.md#folding-a-second-source-in) refuses it.
- Every location this tool is given is resolved against the working directory before it becomes a URL, so a bare relative name works wherever a path is taken.
- `set` lower-cases the name it is given, because a dictionary folds names once and a field spelled two ways is one field.
- Redirected or piped, every command prints plain, stable, greppable text: no colour, no box drawing, no spinner.
- `ygg fix` with no verb needs a terminal; there is nothing to edit a line with where there is none.

## Commands

```bash
cargo build -p yggdryl-cli
cargo run -p yggdryl-cli -- --help
cargo clippy -p yggdryl-cli --all-targets
python scripts/stage_cli.py --clear
```
