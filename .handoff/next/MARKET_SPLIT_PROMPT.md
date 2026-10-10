Prompt: split yggdryl into six crates - the split first, the adaptations after
You are working in the yggdryl repository: the Rust core in rust/, the Python view in python/, the Node view in node/ and the yggdryl CLI in cli/. Read every path below relative to your clone's root.
What you have, and what you do not
You have a clone of https://github.com/Platob/yggdryl and this prompt. You have nothing else: no file outside the repository, no branch that was never pushed, no earlier conversation, no AWS credential and no registry token. You may be on Linux in a sandbox; every command below is POSIX shell.
The base. main at 2ae975674, the squash merge of PR #206. Its tree is byte-identical to d4a6a9245, where every count and file:line anchor below was taken, so each anchor is exact at 2ae975674. Check one with git show 2ae975674:<file> | sed -n '<line>p', after Start here's first step has made the clone whole; git fetch origin pull/206/head fetches d4a6a9245 itself if you want it. Once an edit moves a line, re-anchor by symbol (grep -n 'fn <name>'), never by line. A later main is merged in as D1 says.
The counts. A count given with its command is exactly what that command prints at 2ae975674; a move from it larger than the commits since explain is a finding. A count marked "about" is orientation only, from a broader regex this prompt does not give: the slice that needs it (S0 step 2, for S1's) records its own command and count in DESIGN.md as the baseline, and compares nothing against the "about" figure. The commands use these variables, defined once per shell:
K='Country|Ccy|Mic|Cfi|Isin|Cusip|Sedol|Bbg|Figi|Ric|Forex|Unit|Lei|Bic|Elf|Dti|Fisn|MarketDataKind|MarketDataType|Side|TimeInForce|PluginSide'
MOVED="rust/src/fix rust/src/country rust/src/mic rust/src/isin_registry"
for f in ccy country mic cfi isin cusip sedol bbg figi ric forex unit lei bic elf dti fisn code \
    marketdatakind marketdatatype side timeinforce pluginside fix_category \
    idkey identifier idtype idsource securityid eusipa limit isin_registry; do MOVED="$MOVED rust/src/$f.rs"; done
for f in arrow book candle facts iterator kind market market_column market_data \
    operation operation_column serve trade view; do MOVED="$MOVED rust/src/graph/$f.rs"; done
STAY=$(printf ' :(exclude)%s' $MOVED)   # used unquoted: `-- rust/src $STAY` is what stays in the core
K is the 22 market kinds. MOVED is the 50 paths that leave rust/src at S4 (graph/mod.rs splits, and counts as staying). K also matches English and non-market items (Unit, Side, Country in prose), so a count over it is an upper bound whose lines are read by hand.
.handoff/ is outside every count. From S0 this prompt, DESIGN.md, the handoff and S7's script and listings are tracked, and they spell the very words being counted. Every repo-wide count, recount, sweep and done-when grep in this prompt therefore runs with -- . ':(exclude).handoff' (or a narrower pathspec), and no script edits a file under .handoff/ other than the one it was written to produce. At 2ae975674 the exclusion changes no count.
Other prompts. .handoff/next/ already holds CONTINUE.md, DEDUP_REVIEW_PROMPT.md and STRUCT_SPEC.md, from other phases. Ignore them; CONTINUE.md's numbered items are not your task.
The program's own files are tracked. Everything a later session needs is committed on the program branch (D1) with the slice that writes it:
this prompt, as .handoff/next/MARKET_SPLIT_PROMPT.md (S0 writes it, since the repository does not hold it yet);
the handoff, .handoff/next/MARKET_SPLIT_NEXT.md;
the design folder .handoff/split/: DESIGN.md with its decision ledger, and from S7 the rename script and its listings.
Nothing the program needs lives in an untracked or git-excluded file. S9 deletes .handoff/split/ and both MARKET_SPLIT_*.md files.
Start here
Make the clone whole. A sandbox clone may be shallow or single-branch, and then 2ae975674, the other branches and the merges below are missing:
git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'
if [ "$(git rev-parse --is-shallow-repository)" = true ]; then git fetch --unshallow origin; else git fetch origin; fi
git cat-file -e '2ae975674^{commit}'   # must succeed before any anchor is read
Find the program branch by its S0 commit, never by its name (D1 lets the harness name it):
S0=$(git log --remotes --format=%H -1 --grep="^Pin the market kinds' wire and cost contracts\$")
[ -n "$S0" ] && git branch -r --contains "$S0"
gh pr list --state open --draft --search 'in:title "Split yggdryl into six crates"' --json number,headRefName
If the text you were given names the program branch, that is it, once git branch -r --contains "$S0" lists it.
Else one branch listed is the program branch. Several listed: the head of the open draft PR titled "Split yggdryl into six crates" among them; if that does not decide it, stop before any edit and ask the user, naming every candidate.
Nothing found ($S0 empty and no such PR): this is the first session. It runs S0 and S1, then stops. $S0 empty while such a PR is open: stop and ask the user.
A program branch exists: git switch <branch> && git merge origin/main (D1; <branch> without its origin/). Read .handoff/next/MARKET_SPLIT_PROMPT.md, .handoff/next/MARKET_SPLIT_NEXT.md and .handoff/split/DESIGN.md from it; where they differ from the text you were given, the committed files hold. Run the slice the handoff's Next names, and stop at it.
Before anything else, read AGENTS.md at the root. It is the contract, and this prompt only applies it. Read these sections in full:
the opening; Workflow with Pace and Common changes; Smoke loop; Always;
§1 Layout and Where a test lives; Ownership;
Patterns (One type one file; DataType, Field, Scalar; Intake; Zero copy; Serie is the collection);
Public vocabulary; Generic scalar; Datatypes parsers errors; IOMedia and records; Strings; Arrow and allocation;
all of §2, §3, §4 (Node is the essential view, AGENTS.md:2434: it gains a door only when the user names that feature in Node), §5 and §6.
Tooling
Install what is missing; assume nothing is present.
Rust. rustup toolchain install stable --profile minimal --component clippy,rustfmt and rustup toolchain install 1.94.0 --profile minimal (the workspace's MSRV, checked by the job at .github/workflows/ci.yml:86-95). Builds need a C compiler, and cargo check --workspace needs a python3 on PATH for pyo3's build script. Run cargo fetch --locked once; the third-party sources cited below are then under ${CARGO_HOME:-$HOME/.cargo}/registry/src/index.crates.io-*/.
Python 3.11 or later (CI uses 3.12). Create python/.venv with python3 -m venv python/.venv (on Debian or Ubuntu that needs the python3-venv package), or with uv venv --seed --python 3.12 python/.venv (--seed, so the environment has the pip the next line runs). Below, PY=python/.venv/bin/python (on Windows, python/.venv/Scripts/python.exe), and every Python command runs as $PY.
Install CI's set: $PY -m pip install "maturin>=1.15,<2" "pyarrow>=18" "pytest>=8" "mypy>=1.15" "pandas>=2" "polars>=1" tzdata xxhash "requests>=2.32" -r requirements-docs.txt (ci.yml:373-376; requirements-docs.txt pins mkdocs 1.6.1 and mkdocs-material 9.7.7).
Without pandas, polars, tzdata, xxhash or requests, a suite skips silently (python/tests/test_http.py:1048 skips its interop tests without requests). AGENTS §3 counts a silent skip as a failed check.
maturin develop runs as VIRTUAL_ENV=python/.venv $PY -m maturin develop -m python/Cargo.toml.
Node 22, as CI's node job (ci.yml:589), and never below 21: npm test runs node --test "tests/**/*.test.js" (node/package.json:67), a glob node --test expands only from Node 21. Then npm ci --prefix node once.
GitHub. Push rights on Platob/yggdryl, and gh or the harness's GitHub tool: git push -u origin <branch>, gh pr create --draft --base main --head <branch> --title 'Split yggdryl into six crates', gh run list --branch <branch>, gh run watch <id>, gh run view <id> --log-failed. D1 says what to do when a push or a PR is refused.
A failed fetch is a blocker. If cargo fetch --locked, the pip install or npm ci fails, no smoke check can run: stop before S0 step 2 and report it under the handoff's Blockers (in the final reply, if nothing could be pushed). D11's and D12's "skipped by name" covers only their registry metadata reads.
Goal
The workspace has outgrown one crate. rust/src is 394,119 lines (git ls-files 'rust/src/*.rs' | xargs cat | wc -l), of which about 89.6k are the market area: fix/ 50.6k, graph/ 23.5k, the ISIN registry 4.2k, the codes 5.5k, the enum leaves 2.0k and the identifiers 3.7k.
Split it into six crates and two market binding packages, settling each layer before the next. Every slice is one commit that builds and passes CI. The split comes first, so the core shrinks and every later build and test iteration on it is faster; the adaptations come after, each in the crate that owns the code by then.
The program
The crates
Each crate is a workspace member laid out as the core is (src/, tests/ mirroring src/, benchmarks/), and each is published to crates.io under the workspace version.
crate
path
holds
depends on
yggdryl
rust/
The core. From S1 it adds the registered-kind shape (DataType::Market, Field::Market, Scalar::Market, Serie::Market(MarketSerie)) and the one register; from S2, the media register (D21). It keeps State and the event vocabulary the text medium implements (D4).
-
yggdryl-market
rust/market
The 17 registered codes (ccy, country, mic, cfi, isin, cusip, sedol, bbg, figi, ric, forex, unit, lei, bic, elf, dti, fisn) with code.rs, country/tables.rs and mic/tables.rs. The enum leaves marketdatakind, marketdatatype, side and timeinforce. The identifiers (idkey, identifier, idtype, idsource, securityid), eusipa and limit. The ISIN registry (isin_registry.rs, isin_registry/, the seed). All of graph/ except element.rs, column.rs and element_column.rs. The code behind the CLI's market command.
yggdryl
yggdryl-fix
rust/fix
The whole FIX implementation: all of rust/src/fix/ (FixRegistry, FixCodec, FixMsg, the fixed row, crated.rs, the CBlock reader, the store) and its generated constants.rs, plus fix_category.rs, pluginside.rs (a FIX session notion, pluginside.rs:1-2), the FixField view, State's FIX tables, and whatever D10 moves. It owns scripts/generate_fix_dictionary.py, config/fix/, the crate dump, the dictionary hash pin, the fix and fix_allocations benches, the FIX pipeline cost pins, docs/fix/, skills/yggdryl-fix, the FIX parts of docs/assets/fix.json and playground.json, and the code behind the CLI's FIX commands.
yggdryl, yggdryl-market
yggdryl-avro
rust/avro
All of avro/, including its scalar codec, with its own snap (D16).
yggdryl
yggdryl-parquet
rust/parquet
parquet/
yggdryl
yggdryl-iceberg
rust/iceberg
iceberg/, and s3tables/ behind its own s3tables feature (D15).
yggdryl, yggdryl-avro, yggdryl-parquet
The core package excludes the crate folders. They sit inside rust/. At S4 and again at S6, run cargo package --list -p yggdryl --allow-dirty. If it lists anything under market/, fix/, avro/, parquet/ or iceberg/, add an exclude to rust/Cargo.toml, anchored at the package root. src/avro/ and its siblings are module folders and stay until their slice moves them.
Each new manifest (S4, S6) carries:
version, edition, rust-version, license and repository as .workspace = true, and its own description, keywords, categories and README. crates.io refuses an upload without a description, while cargo publish --dry-run only warns, so a rehearsal would pass and the release would fail half out.
The core's lint posture unless S0 records otherwise: no [lints] workspace = true (the core has none) and #![deny(unsafe_code)] at the crate root (rust/src/lib.rs:11). Opting in would put the moved code under the workspace's pedantic clippy (Cargo.toml:50-52, ci.yml:62) for the first time.
Features forwarding to the core: yggdryl-market has http = ["yggdryl/http"], internals = ["yggdryl/internals"] and, for its Iceberg-writing tests until S6, iceberg = ["yggdryl/iceberg"]; yggdryl-fix has the same plus yggdryl-market/<feature>. From S6 the media crates own their features.
Cargo.lock changes with each new member and is committed (CI passes --locked).
The binding packages
runtime
package
what it is
Python
yggdryl (PyPI, as today)
The one native module yggdryl._native (python/pyproject.toml:81), linking every crate. From S4 its module init calls each crate's install() before it registers a class. It carries the core facades, and Avro, Parquet and Iceberg as today (U4).
Python
yggdryl-market (PyPI; import path yggdryl.market)
Pure Python (py3-none-any) at python/market/. It holds the market and FIX facades over yggdryl._native (yggdryl.market, yggdryl.market.fix, yggdryl.market.graph, ...) and depends on yggdryl==<version>. The core's python/yggdryl/__init__.py stops re-exporting market names (:23, 29, 32, 125-160, 230-234, 306-308; line 229 imports State, which D4 keeps in the core).
Node
yggdryl (npm, as today)
The one addon and the core JavaScript.
Node
yggdryl-market (npm; name per D12)
JavaScript only, at node/market/. It holds the market and FIX adapters moved out of node/binding.js (:5566-5700, 7171-7172), plus book.js, book.d.ts and book/, and declares an exact peerDependencies.yggdryl.
CLI
yggdryl-cli (unpublished)
One yggdryl binary over every crate, staged into the core wheel as today (scripts/stage_cli.py:125, ci.yml:169).
Versions. One workspace version (Cargo.toml:13) covers every artifact: Cargo pins =<version>, declared once in [workspace.dependencies]; Python pins yggdryl==<version>; npm pins the peer "yggdryl": "<version>". Release preflight checks that they all agree, and checks every name on every registry: all are published or none are (.github/workflows/release.yml:82-95, 141-180, 931-935). AGENTS §6's bump list gains the market manifests in S5 and the media manifests in S6.
The order, decided by the user
The user's last word on order: "first do the split then adaptations so it can improve the core iterations for implementations."
The split. Code moves into its crate as it stands. Behaviour, error text, wire formats, costs, public names and spellings are unchanged - RecordOptions keeps its name through every move - except where an extension point replaces a per-kind or per-medium variant with its registered one. The points land first, in place, so each move is a move only.
The adaptations come after the split has landed, each done in the crate that owns the code by then: first the RecordOptions -> MediaOptions rename, then the planned expression series, whose pushdown the media crates implement through the media point.
No slice puts the rename or a planned series before a move.
The slices
S
the commit
proved by
S0
design; the Contracts pins, this prompt and .handoff/split/DESIGN.md, as their own commit "Pin the market kinds' wire and cost contracts"
the pins green on today's tree; pushed; CI read green
S1
the market extension point; the 22 market kinds claimed through it while still in rust/src
S0's pins byte-identical; the cost and size gates hold; rust/tests/market_register.rs proves the claim and every refusal
S2
the media extension point (D21); Avro, Parquet, Iceberg and S3 Tables claimed through it in place, with today's pushdown; the ISIN registry store on the table-format register
iobase_calls unmoved; every medium's harness; both feature lanes; the exchange jobs
S3
the remaining seams, in place: D5, D6, D9, D10
cargo check --workspace --all-targets; the dictionary hash and crate dump unmoved; the whole run
S4
yggdryl-market and yggdryl-fix created and their files moved; bindings and CLI re-pointed, still one package each; tests, benches, generators, docs, skills, inventories, AGENTS.md and CI updated
market and fix whole runs in both lanes; cost rows unmoved; dictionary hash and crate dump unmoved; Python's test_fix.py GIL pin; Node's book.test.js against the built CLI; cargo package --list; maturin sdist
S5
the yggdryl-market Python and npm packages; release.yml and ci.yml learn the market and FIX artifacts
the packages' suites in CI; a release rehearsal (workflow_dispatch, which publishes nothing: release.yml:107-112), run only on the user's go
S6
yggdryl-avro, yggdryl-parquet and yggdryl-iceberg with s3tables, one commit, in that dependency order inside it
each crate's suite at format v3; the fastavro, PyIceberg and Spark exchanges; the iobase_calls Parquet rows; rust/tests/s3tables/catalog.rs's request counts; the MSRV job retargeted (ci.yml:85-95); cargo package --list
S7
RecordOptions renamed MediaOptions across the core and every crate (U7)
the sweep's grep empty; every cost pin's count unchanged and its diff the rename alone; the whole run
S8a
D18: SerieKind, Serie::from_holder, into_held/into_chunked, apply_serie as the one evaluator with its narrow landing, and the media seam
see S8
S8b
the planned expression series (U6): the five lazy verbs, their pushdown, and the plan sourced on a Serie
see S8
S9
final sweep; merge readiness
the user's go bumps the version, merges and releases; never the session's
Why S4 creates both crates in one commit. No tree in between would build. fix/ names the codes, the identifiers, the ISIN registry (about 31 lines; fix/codec.rs holds Arc<Mutex<IsinRegistry>> at :527, 743, 1078, 1087 and its IsinTable snapshot at :75, 533) and graph/ (in 10 files, git grep -l '\bgraph::' -- rust/src/fix; about 152 lines), and the core cannot depend on yggdryl-market. The user's order (market first, FIX next) holds in the dependency, because yggdryl-fix sits on yggdryl-market. If the diff is too large to review, S4a moves everything into rust/market, FIX included, and S4b moves rust/market/src/fix out to rust/fix; that re-points the bindings, docs and inventories twice. S0 records which shape S4 takes.
Why S6 is one commit. The core's iceberg/ names crate::avro and crate::parquet on 27 lines (git grep -nE 'crate::(avro|parquet)' -- rust/src/iceberg: manifest.rs 15, scan.rs 7, table.rs 4, statistics.rs 1), and the core cannot depend on a crate that depends on it. No tree with Avro or Parquet moved and Iceberg still in the core builds, so the user's order is the dependency order inside one commit. If that diff is too large to review, the only buildable split reverses the order: S6a yggdryl-iceberg, reaching the core's Avro and Parquet through their public modules and D6; S6b yggdryl-parquet; S6c yggdryl-avro; each re-points yggdryl-iceberg. Put that to the user before taking it.
Why the media point lands before any move. It is designed once (D21): it hands a medium its options whole, and the medium reads today's pushdown out of them. The moved media keep that pushdown, and S8 builds on it without a second change to the point.
Merging. Every push to main releases (release.yml:39-41), and the release learns the market crates only at S5 and the media crates at S6. No slice merges to main on its own; the branch merges when the user says so.
Scope
In, for the first session: S0 (design and the pins commit) and S1 (the market extension point). S0 designs the media point (D21) too, because both points share D8's mechanism; S2 builds it. The session ends when S1 is pushed and its CI run read green, or when a blocker stops it, and then writes the handoff. Do not start S2, even with time left. A later session runs the one slice its handoff names.
Out: every slice past the one in hand, and these phases, planned outside this repository with nothing of them in the tree. Do not start them or look for their files:
a dtype -> data_type rename (not landed: rust/src/field.rs still has fn dtype);
an instrument-registry change that replaces the ISIN registry; it will rebase onto the paths S4 moves (record that among DESIGN.md's risks);
parse-time instruments with host defaults;
a lifecycle optimization;
a filesystem audit; new market leaves;
the keyed serie holders, the remaining Arrow doors and the ArrowCastOptions removal (S8 says what each is).
No live AWS resource is touched. Nothing is published.
Rules that govern this work (AGENTS.md)
No back-compat (Always). A moved or renamed name is deleted where it was, together with its parser spelling, test and doc, in the same commit: no alias, no re-export from the old path, no shim. A wire format is an external contract, not back-compat; the formats that stay byte-identical are listed under Contracts.
One owner per fact, one spelling per verb (§1 invariants). A crate boundary adds no second parser, schema, dispatcher or registry. The core's register is the one place a registered kind or medium is claimed (D7, D8).
One commit per slice, in layer order. Each slice runs Rust core -> Python -> Node -> docs and lands as one commit (Workflow; Pace "One commit"). CI's docs-examples and inventory jobs fail a commit whose docs lag its code.
Smoke while you build. Run the narrowest command after each edit, and write the refusal and the pinned count before the happy path.
Pace.
A rename or move that reaches every caller is one script of exact-string edits, each anchor asserting one match, driven by cargo check --workspace --all-targets --all-features --keep-going --message-format=short (--all-features so the internals modules and mod internal tests are checked) until that is clean.
Workers get disjoint file sets. No cargo fmt --all while workers edit.
Anything over a minute is one chained background script with one log.
If you drive workers through a resumable workflow, never edit a constant that a finished agent's prompt embeds; add a new constant for later phases.
The model fits the step. Design, and the edits no script makes, stay in the foreground. If your harness can spawn subagents with a model choice, reviews and adversarial verification run on opus, and checks that re-run a command and report, and mechanical sweeps, run on sonnet; fan-out over disjoint files is encouraged. Without subagents, run the review in the foreground as a separate pass and say so in the handoff.
Cost pins are design answers. rust/tests/allocations.rs, rust/tests/iobase_calls.rs and the benchmarks are never re-pinned from a sweep.
Tests live in tests/ only. No #[cfg(test)], #[test] or mod tests under rust/src, rust/*/src, python/src, node/src or cli/src; grep for them before every commit. Anything a caller cannot reach is pinned through internals.
Iceberg tests create format version 3, unless the contract under test names another version, in which case a comment says which.
Node is the essential view (§4). The split slices only re-spell and re-pin the Node doors that exist. The user named Node for the market package, so S5 moves the existing market and FIX doors into it and adds none.
Wide in, typed through. A registered kind is resolved once at intake into a &'static descriptor, and that descriptor is carried. A name, tag or extension name is looked up only at intake; the byte-to-descriptor table is an O(1) array read, allowed anywhere.
Never touch live AWS resources. No aws CLI, no YGGDRYL_S3TABLES_ARN, and never run python/tests/medallion.py with an s3tables:// location or an ARN (its use over local warehouse folders inside the test suite is fine); only the fakes and CI's exchange servers.
Never publish. No cargo publish without --dry-run, no npm publish, maturin publish or twine. No push to main, no tag, and no release run without the user's go.
Decisions taken (the user's; not reopened)
U1, the crates are as tabled above. FIX gets its own crate; the user preferred that to FIX inside market.
U2, naming. The core extension point is Scalar::Market and Serie::Market(MarketSerie), with DataType::Market and Field::Market as counterparts.
U3, the media leave the core through one media extension point, designed together with the market one. Both use one mechanism unless S0's design shows they cannot; D8 records what they share.
U4, bindings. Python and Node each get a yggdryl.market package, published on its own. FIX ships inside them by default; D11 confirms this, or records a reason for separate FIX packages. The existing yggdryl packages keep exposing Avro, Parquet and Iceberg through the crates, with no new binding package.
U5, order: the split first, the adaptations after (The order, above): the extension points (market S1, media S2, the seams S3) -> yggdryl-market and yggdryl-fix (S4) -> their binding packages (S5) -> yggdryl-avro, yggdryl-parquet, yggdryl-iceberg with s3tables (S6, one commit in that order) -> the rename (S7) -> the planned expression series (S8).
U6, the planned expression series (SelectSerie, FilterSerie and the rest) are finished inside this program, after the split, in the crates that own the code; the media crates implement their pushdown through the media point.
U7, the rename. RecordOptions becomes MediaOptions everywhere, after the split, as one sweep over the core and every crate, with no alias.
Decisions the session takes before code
S0 writes each decision into .handoff/split/DESIGN.md, with its evidence and the alternative it refused, and keeps a ledger there as D# | decision | evidence | slice. Every later slice updates the ledger in its own commit.
D1-D8 and D19-D21 bind S1 and S2. The rest bind later slices, but are taken now so that S1's shape does not have to change again.
D1, base, branch and PR. PR #206 merged as 2ae975674 on origin/main.
The first session branches split/crates from origin/main, records HEAD, pushes S0's commit (git push -u origin split/crates) and opens one draft PR for the program, titled so Start here can find it (gh pr create --draft --base main --head split/crates --title 'Split yggdryl into six crates'). CI runs on pull_request only (ci.yml:3-9). Never mark the PR ready and never merge it.
If the harness refuses the name. If the harness assigns a branch and refuses pushes to any other, that branch is the program branch. Name it in DESIGN.md, in the handoff and in the final reply; Start here finds it by its S0 commit whatever it is called. If no PR can be opened or no CI run read, stop after the pushed S0 commit and report the blocker.
The branch outlives sessions. Each session starts with git fetch origin, checks out the program branch and merges origin/main into it, never rebasing a pushed slice; it resolves conflicts by re-running the slice's sweep over the merged tree and records the merge in the handoff. No slice starts behind origin/main.
The version. The branch carries 0.1.21, which main already released, and from S5 preflight refuses a half-out version. The user's go at S9 bumps the version on the branch (AGENTS §6's list as S5 and S6 extend it), then merges.
D2, the market shape. Recommended: a closed shape over registered static descriptors.
MarketKind. One static per kind holds:
its DataTypeId byte, canonical name and Arrow extension name, as const items (D3);
its storage: Text { width }, Code8 or Code16; its member table, for an enum;
its order and hash positions (D19);
function pointers for reading text, the canonical check (is_canonical, the Forex canonicalization) and the rank (CodeValue::rank, is_real).
The variants.
DataType::Market(MarketType(&'static MarketKind)), compared by byte and hashed per D19.
Field::Market(FieldOf<MarketType>).
Scalar::Market(MarketScalar { kind, value: Text(Str) | Code(u16) }). Its fields are private: a market crate builds one only through its descriptor's validating door, so no crate lays out an unchecked value.
Serie::Market(MarketSerie), with MarketSerie = Text(Arc<Utf8StringSerie>) | Code8(Arc<UInt8Serie>) | Code16(Arc<UInt16Serie>), the storage today's 23 variants already share (serie.rs:539-595).
Sizes. Scalar stays 48 bytes (scalar.rs:331) and Serie 40 (serie.rs:677). Compile it before believing it.
Refused: a trait-object variant per root. It allocates an Arc per cell read, breaking a_leaf_cell_read_allocates_nothing (rust/tests/allocations.rs:6540). It makes every compare and hash a vtable call, and it cannot implement Value (value/mod.rs:78-90).
Refused: plain leaves with the identity in metadata. That moves the value-stream bytes, the digest feed and the ranks. It remains what an unregistered kind falls back to at Arrow import.
D3, DataTypeId. Today it is a fieldless #[repr(u8)] enum (datatype_id.rs:28-32).
Recommended: a newtype DataTypeId(u8). The core ids become associated consts that keep their CamelCase names; a derived PartialEq, Eq keeps them usable as patterns, so most of the 487 DataTypeId:: lines in 31 files (git grep -n 'DataTypeId::' -- rust/src) still compile. kind() is read from the byte range. Debug is hand-written to print the name, so no error text moves. For a registered byte, as_str and arrow_extension_name answer through the register, so they are not const there; the const contexts that read a name (arrow/extension.rs:57's ExtensionType::NAME, the FAMILY consts at typed.rs:1012 and value/mod.rs:1088) read the kind's own const instead.
The 22 market ids are not core consts. E0116 forbids adding consts to DataTypeId from yggdryl-market, so S1 deletes DataTypeId::Isin ... DataTypeId::PluginSide and each kind answers its own (Isin::ID, MarketKind::id). The 243 lines that name one (git grep -nE "DataTypeId::($K)\b" -- rust/src rust/tests: 80 in rust/src, 163 in rust/tests) are re-spelled once, by script, with the 15 the same pattern finds outside those folders: 11 in .api-inventory.txt, and one each in docs/types/codes/ccy.md, docs/types/codes/country.md, docs/types/datatype.md and skills/yggdryl-types/SKILL.md.
The listing. DataTypeId::ALL (97 entries, datatype_id.rs:270) stays the core's own ids. DataTypeId::all() answers the core ids plus the claimed bytes in today's declaration order (codes between MediaType and Uuid, enums after State). It is what python/src/lib.rs:431-434 and node/src/enums/vocabulary.rs:26-27 read, so yggdryl.enums.DATA_TYPE_IDS and enums.dataTypeIds keep 97 members in today's order (python/tests/test_version.py:314, python/tests/test_datatype.py:1618-1619, node/tests/fields.test.js:320).
The alternative keeps the enum, with the market bytes as reserved members; the core then still names every code.
Spike it in S0 and record the cargo check result.
D4, what of the market set the core keeps.
State stays a concrete core leaf, with enum_leaf! kept for it alone. TextLine holds a State and implements graph::Event (text/line.rs:11-12, 1313, 1427-1454), and the text row is laid out from ElementColumn::ALL and EventColumn::ALL (text/plan.rs:20, 96-114).
graph/element.rs, graph/column.rs and graph/element_column.rs therefore stay in the core as yggdryl::graph, which then holds only the event vocabulary. yggdryl_market::graph re-exports nothing of it.
State's FIX tables (from_fix_status, FIX_STATUS_TAGS, from_fix_msgtype; state.rs:315-361) go to the FIX module as free functions in S3, and to yggdryl-fix in S4. The Python classmethods State.from_fix_status and State.from_fix_msgtype (python/yggdryl/state.py:84; natives python/src/state.rs:48-55, registered at python/src/lib.rs:620-621) redirect to those functions in S3 and move to yggdryl.market.fix in S5, with no alias.
All 17 codes are market.
D5, constructor spelling (S3). The 18 impl DataType blocks in the code files (ccy.rs:50, isin.rs:311, code.rs:229, ...) cannot cross a crate (E0116). Recommended: the market type's own associated items, such as Isin::dtype() and Isin::field(name), in whatever word the dtype rename has reached by then; no extension trait kept just to preserve DataType::isin(). DataType::is_code, code_width and code_name stay in the core and read the descriptor. S1 leaves the constructors where they are; S3 applies the spelling in place, so that S4 is moves only.
D6, the implementer surface.
What market code reaches. These crate-private core items: typed::define_field_types! (about 23 uses); text::expected_got (about 45); folds_equal/normalized/fold_digest (about 62, lib.rs:333); logging::warning::warned! (about 18); path::Path (about 15; private module, lib.rs:123); parallel::ordered (about 7; private, lib.rs:119); text::elide_to, integer_from_text_as, hashing::stable_hash_of, arrow::rows::reader, ascii_text, string::str_from_value; the code.rs helpers (:88-409); graph/element.rs's InstantSequence (:791), feed_event_facts (:740), crosshash (:515), canonicalize_uuids (:457) and right_is_reference (:568); element_column::whole_u64 and digest_u64 (:242, 257); http::server::normalize_path (http/server.rs:1207).
The media crates need more: about 25 items for Avro, 22 for Parquet, 55 for Iceberg and 12 for S3 Tables.
The constraint. AGENTS forbids raising an item to pub inside a published module.
Recommended: one documented public module of forwarding pub fns and pub uses for exactly the listed items, with its own .api-inventory.txt section. Proof::Proven never crosses it; only an Unproven landing door may. media_serie! (media_serie.rs:455) crosses as a public macro, or is replaced by a generic.
Timing. S1 publishes what S1 needs, S3 the market and FIX list, S6 the media list (in place until then, the media need none). Each slice derives its exact list from a scratch git mv plus cargo check -p <crate> --message-format=short (each E0603/E0624), then discards the scratch.
D7, registration.
Install. Each crate has an explicit, idempotent install() (a OnceLock), called from each binding's module init and from the CLI's main before any data comes in. No link-time registration: it is not impossible (unsafe is an AGENTS-reviewed exception: the core denies it at lib.rs:11 and allows it in four reviewed modules, local/file.rs, serie/bytes.rs, serie/variant.rs, spill.rs), but inventory and linkme would add a fifth for a convenience install() already gives.
Who installs. A value built through a market type (Isin::dtype()) carries its &'static MarketKind and needs no register. Only intake reads the register: a parsed name, LOGICAL_NAMES, a value-stream byte, a serde tag, an Arrow extension name, a binding type tag. Recommended: every public entry of yggdryl-market and yggdryl-fix that reads text, a store or a wire calls its crate's install() itself; each moved test harness, bench and rustdoc example calls it through one support function the S4 script inserts; a docs block that parses a market name through the core shows yggdryl_market::install();, and docs/types/datatype.md says so. S0 records the choice and S1 pins it with the test-only kind.
Refusals. The register refuses: a byte, name or extension name claimed twice; a byte outside the Code range 0x6a..=0x7f or the Enum range 0xc0..=0xcf (datatype_kind.rs:140-178); a family's own id byte (0x6a, 0xc0: DataTypeKind::id, which the value stream already refuses, valuestream.rs:528-535); a byte a core leaf holds (0xc1, State); a retired byte, 0x75-0x77 (datatype_id.rs:192-194); a Text storage outside the Code range, or a Code8/Code16 storage outside the Enum range. Each refusal names the crate to add.
Seed. Until S4, the register seeds itself from the core's own kinds before its first read and before its first registration. S4 replaces the seed with yggdryl_market::install() and yggdryl_fix::install().
D8, one mechanism for market and media.
Recommended: one root file holding the keyed claim-once register, whose refusal names the crate to add, and the helper that gives a trait object its clone, equality, order and hash inside a derive-heavy enum. Not extension.rs, since arrow/extension.rs exists; for example plugin.rs. The tree's one process-wide registry today, the user functions (expression/user.rs:483-494, a locked map from name to a shared trait object), is the precedent.
What shares it. The market register (keyed by byte, name and extension name) lands on it in S1; the media register (keyed by MimeType, table-format name, catalog type word and URL scheme) in S2. LOGICAL_NAMES (a name to a DataType) registers on it too, as a third key, never a second registry.
What cannot be shared: the trait (a lazy store reader and writer, versus a value leaf); the key; a Serie variant. A medium needs none, because Serie::GenericMedia (serie.rs:672, declared by media_serie!(GenericMediaSerie, GenericMedia, ...) at media/mod.rs:567-570) already carries any medium type-erased.
D9, the cycles and protocol views (S3).
graph -> fix. graph names fix through MarketData::Fix(Box<FixMsg>) (graph/market_data.rs:44, 621-627, 745; the edges sit in graph/{arrow,element,kind,market_data}.rs). Recommended: yggdryl-market defines the trait that a message which splits into market leaves answers (walk and merge as itself, into_market_data, into_market_leaf, its stable hash); MarketData holds that trait and yggdryl-fix implements it. MarketKind::Fix (graph/kind.rs:31, 62, spelled fix) stays, naming that trait object. Refused unless pinned equal: deleting the variant and splitting at the FIX door, which changes how the walk merges a FIX message.
The FIX field view. The core generates FixField and FixFieldMut (protocol.rs:2078 over metadata.rs:156-163), while their 59 methods are inherent impls in fix/field.rs:108, 690, illegal across crates. Recommended: the core publishes a protocol-view builder; the FIX view and its FIX: keys move wholly to yggdryl-fix, and Field::as_fix leaves the core. The same builder serves IcebergField (iceberg/field.rs:39) in S6.
D10, the FIX vocabulary in the core (S3). Each piece moves if FIX alone uses it; otherwise it stays, with the reason written in DESIGN.md.
piece
disposition
the FIX Latest names in LOGICAL_NAMES (vocabulary.rs:155+)
yggdryl-fix registers them on D8's mechanism, and yggdryl-market registers mic and the code names. The core keeps non-market names only. First grep every reader of a FIX word outside fix/.
DateTime64::from_fix_text and from_fix_clock (datetime.rs:610, 637); Time32/Time64::from_fix_text (time.rs:476, 553); their grammar (temporal.rs:844-914)
Move to yggdryl-fix if the ISO part can be delegated to the public readers with no allocation per value, as decided by the FIX parse rows in allocations.rs and the fix_allocations bench. Otherwise keep them public as the type's FIX spellings, with that reason.
the FIX entry of for_each_well_known_protocol!; the FIX:* keys
Move with the view (D9).
Scheme::FIX (scheme.rs:27, 88, 169, 210, 420)
Moves if it names only the protocol; stays if the URI vocabulary routes on it.
MimeType::{FIX, FIXUL, FIXML, ULLINK}; the frame classifier (mime_type/line.rs:80-100, 756)
Stay, as routing vocabulary and the one bounded content read, as MimeType::PARQUET and media/magic.rs do. The medium they route to registers on S2's point.
parallel.rs
Stays: it is generic. Its AGENTS row is restated without FIX, and ordered is reached through D6.
logging targets
logging/facade.rs:72-76 treats targets outside yggdryl/yggdryl:: as foreign, and the docs pin logger names (docs/architecture.md:69-138). Use explicit target: names, or a facade mapping yggdryl_<crate>:: to yggdryl., so every name stays byte-identical. Pin it in rust/tests/logging/.
StringEnum::{CURRENCIES, COUNTRIES, MICS, SIDES, TIMESINFORCE, PREBUILT} (string.rs:2285-2410); DataType::CODES (:735-751)
Become registered listings that move with their codes. generate_fix_dictionary.py reads neither (:115 is a comment) and writes only rust/src/fix/constants.rs (:1834, 1916-1922): re-point its output path and comments, and AGENTS §2's COUNTRIES trigger follows the listing to its new owner.
D11, the binding boundary.
Two native modules would mean two cores. PyO3 refuses a foreign class (pyo3-0.29.2/src/type_object.rs:88-89 in the cargo registry). With napi, two builds of one binding crate share type tags, so each accepts the other copy's objects and casts them blindly, which is memory-unsafe (napi-3.12.1/src/bindgen_runtime/type_tag.rs:7-15). Every process global splits: SystemWarehouse, the user functions, the spill default, the logging tree, the HTTP runtime, the interned fields, the ISIN and FIX registries, the pools and the allocator.
Four options were weighed.
(a) A stable C ABI or PyCapsule exported by the core extension: a hand-written extern "C" vtable over opaque handles and unsafe the workspace denies; for data it collapses into (b).
(b) Two native modules exchanging Arrow C Data plus value-stream or pickle bytes: isinstance breaks, every global above is duplicated, market datatypes register only in the market copy (so yggdryl.DataType("isin") cannot resolve), and the native artifacts double against the PyPI storage limit 0.1.15 already hit (release.yml:776).
(c) One binding built with and without a market feature, the market copy a superset wheel at yggdryl/market/_native: the only option that shrinks the core artifact, but it carries the napi hazard, puts a second core per platform on PyPI and makes import order load-bearing.
(d) Split the Rust crates only: one native module per runtime links every crate and registers the market classes as today, and the market packages are pure language layers over it.
Recommended: option d. One core per process, every global single, market datatypes registering into the one parser from the module init, no FFI, no unsafe, no extra LTO links (the one-core-per-platform rule at release.yml:270-277 holds), no wheel growth. Refused: (a), (b) and (c), for the reasons above.
Option d splits the crate, not the shipped size. S0 reads the release sizes from metadata, with no download: the PyPI JSON API's per-file size (https://pypi.org/pypi/yggdryl/json) and npm view yggdryl dist.unpackedSize. If the sandbox cannot reach pypi.org or registry.npmjs.org, record the read as skipped by name and carry it as a question for the user.
Python namespace. yggdryl/__init__.py gains __path__ = __import__("pkgutil").extend_path(__path__, __name__). python/market/yggdryl/market/ has no yggdryl/__init__.py and is installed editable into python/.venv. Whether mypy --strict (ci.yml:388-395) merges the namespace is unverified: prove it in S5, and if it fails, ship yggdryl/market/ inside the core wheel and ask the user.
FIX ships inside the market packages unless S0 finds a reason not to; record which. The pure-Python build backend (hatchling, flit_core or uv_build) is S5's choice.
D12, names. crates.io: yggdryl-market, yggdryl-fix, yggdryl-avro, yggdryl-parquet, yggdryl-iceberg. PyPI: yggdryl-market (PEP 503 normalizes yggdryl.market to it), imported as yggdryl.market. npm: yggdryl-market is recommended, as one string preflight checks on all three registries; the user's spelling, yggdryl.market, is valid on npm and is the alternative. Record the choice, and put it to the user in the handoff before S5 publishes a name. Check availability read-only (cargo search, pip index versions, npm view); if the sandbox cannot reach crates.io, pypi.org or registry.npmjs.org, record each check as skipped by name and carry it as a question for the user.
D13, AGENTS.md and docs layout.
AGENTS.md: one root file with a section per crate. A nested AGENTS.md is not injected by every harness (unverified).
Docs: one mkdocs site.
Docs runner. Blocks already span crates (docs/graph/isin-registry.md:1238 and docs/graph/market-data.md:82 name FIX types; so do docs/types/enum/timeinforce.md:342 and docs/types/enum/pluginside.md:297), and a market dev-dependency on FIX would compile yggdryl-market twice. So the generated Rust target moves, in S4, into the one unpublished member that depends on every crate: cli/tests/docs_examples.rs, git-ignored, run as cargo test -p yggdryl-cli --test docs_examples with the runner's features. RUST_TARGET (scripts/check_docs_examples.py:46), run_rust (:207-213) and .gitignore:40 move with it, the blocks' CARGO_MANIFEST_DIR paths are re-anchored once, and S6 updates the feature list (:209).
D14, fixtures after S4.
Core tests that use codes as fixtures (about 47 files and 591 hits; allocations.rs 144, root/string.rs 72, xmla/dbtype.rs 48) move onto a test-only registered kind in rust/tests/support/. So do the core benches that use codes (types/datatype/value.rs, types/enums.rs, types/datatype/serie.rs:272, 382-383, types/field/value.rs:464, 472, types/datatype/ascii.rs:27, 148, arrow.rs:735, 847, 980), the core doctests (merge.rs:143-144, ascii.rs:253) and the core's doc-comment and doctest mentions of market items. List them with R="\b($K|MarketData|(Order|Quote|Execution|Book|Trade|Snapshot)Event|IdKey|IdType|IdSource|IsinRegistry)\b|graph::(book|market|iterator)|\bfix::|Fix[A-Z]" and git grep -nE "^\s*//.*($R)" -- rust/src $STAY: 90 lines in 20 files at 2ae975674, among them serie.rs 22, string.rs 16 (the CFI essay at :681-734), value/mod.rs 12, the doctests at graph/column.rs:26 and graph/element_column.rs:24, which import OrderEvent, logging/warning.rs:134 and logging/record.rs:136 (the module paths yggdryl::graph::book and yggdryl::fix::build), and datatype_kind.rs. It over-matches (txhash/value.rs:195's "Unit" is English; crate::graph::Event stays, D4), so read each line. Tests that pin market behaviour move to the market crate instead.
rust/tests/fix/ulbridge.log moves to rust/tests/support/ulbridge.log, the shared-fixture folder AGENTS names. The core's readers (benchmarks/text/line.rs:388, benchmarks/media/iceberg.rs:1899) keep reading it, and the FIX crate reads it at concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/support/ulbridge.log"). No corpus changes, so no bench row or page number moves.
D15, s3tables. A feature of yggdryl-iceberg that implies yggdryl/s3, not a crate. iceberg/table.rs:373-377, 455-457, 515-517 call s3tables::locate, create and open_or_create, so two crates would form a cycle. Confirm and record.
D16, Avro in the core. None. The core decodes no Avro: value streams and pickles use variant.rs. yggdryl-avro carries snap itself; today it rides on parquet (avro/container.rs:107-247). Confirm and record.
D17, iceberg/types.rs. It is in the default core as yggdryl::iceberg (lib.rs:79-81). The core keeps the Iceberg type-string spelling (parser.rs:1153) and compatibility::Target::Iceberg; the rest leaves in S6. Confirm in S2.
D18, S8a's prerequisites. See S8.
D19, the hashes: nothing moves.
What is at stake. Std Hash of DataType is hand-written over the private Shape enum's derived positions (datatype.rs:749-844), and stable_hash_of feeds it into XXH3 (hashing/stable.rs:23-27) for FixRegistry::stable_hash, FixMsg::stable_hash and the dictionary pin (rust/tests/fix/store.rs:3440, 17_114_512_833_162_386_024). Scalar's Hash writes value_rank first (scalar.rs:1204); the ranks are codes 18, State 27, MarketDataKind 28, Side 29, MarketDataType 30, TimeInForce 31, PluginSide 32 (scalar.rs:1497-1535); today's enum leaves are #[repr(u8/u16)] with a derived Hash whose discriminant is the code (enums.rs:204-207).
The fix. Each of the 22 kinds states today's rank and today's Shape position in its descriptor. Scalar's Ord and Hash read that rank, then exactly what today's variant feeds after it (a code's text, a member's write_u8/write_u16 of its code). impl Hash for DataType writes state.write_isize(position) for a market leaf. Shape keeps every other position: #[repr(isize)] with explicit discriminants equal to today's numbers, because an explicit discriminant on an enum with fields needs a primitive repr (E0732) and isize is the width a derived Hash writes today. A kind claimed later takes one reserved rank (codes 18, enums one past the last core rank) and one reserved Shape position, then orders and hashes by its byte; a core variant appended later takes a position after the reserved one.
Proof. The dictionary pin is unmoved in S1. Only if this plan is refused does it move, once, with its It last moved when sentence; before that, find every place a stable_hash_of output or an enum member's std hash (scalar.rs:1280-1285) is persisted (a table, the FIX store, a txhash id), and if any is, stop and ask.
What does not move in any case. The canonical XXH3 feeds (xxhash/scalar.rs:83-91, 348-361).
D20, typed market values.
Constraints. Value::from_scalar(&Scalar) -> Option<&Self> borrows a leaf inside a Scalar variant, which a registered value cannot satisfy without unsafe (value/mod.rs:78-90); D7's reason for refusing a fifth unsafe module refuses a reference cast here too. CodeValue and EnumValue carry per-type consts. DataTypeValue::id() takes no self. AGENTS asks for one sealed marker per datatype variant.
Decide: an owned from_scalar (a Str clone is inline below 24 bytes) or a borrowed one-pointer view; the marker TypedField takes for a registered kind. Restate the AGENTS rules that change.
D21, the media point's contract (designed in S0, built in S2).
What a medium registers. One trait per role, keyed on D8: a codec (read, row count, field, overwrite, native stream); a table format (Located's is-whole, overwrite-whole and clear, the write-session hooks, WriteCount::skip); a catalog factory (the type word and the scheme); a locator for Holder::from_url; a capability downcast; and the medium's own settings inside the options' one registered variant. S2's list says what each replaces.
The pushdown rides the options, not the trait. Every door hands the medium its RecordOptions whole, and the medium reads today's pushdown through the published section readers: projection by apply_columns, the filter by the where section with filter_phases and apply_arrow_expressions, partition pruning by partition_pairs, row bounds by max_row_size, row_offset and limit_arrow_reader; Bounds and Bound are published through D6. No pushdown argument rides the trait, so S8 changes the readers and the core defaults on MediaSerieValue (with_filter, with_select, with_row_range at media_serie.rs:261-298 are provided defaults no medium overrides), never the point.
Refused: a per-medium arm left in the core; a second register; a pushdown argument added to the trait now for S8, which would design S8 before its slice.
Contracts pinned before anything moves
S0 writes these pins through doors whose spelling survives S1 and S4: DataType::from_str, Field::from_str, serde JSON in and out, value-stream bytes in and out, Serie::from_arrow_array over an array that carries the extension, and stable_hash. They run green on today's tree and land as S0's own commit, with this prompt and DESIGN.md, pushed and CI read green before S1 starts, so S1 is proven against them.
Pin each of the following for each of the 22 kinds that become registered (the 17 codes, MarketDataKind, MarketDataType, Side, TimeInForce, PluginSide) and for State:
Byte and name. The DataTypeId byte (codes 0x6b-0x6f, 0x71-0x74, 0x78-0x7f; enums 0xc1-0xc6), as_str, and the order of DATA_TYPE_IDS and dataTypeIds.
Arrow. The extension name (yggdryl.<name>), its storage and its document, in both directions. An unknown yggdryl. name (yggdryl.nosuch) imports as its storage with the ARROW:extension:* keys kept (field.rs:2422-2445, 2570, 2616); after S4 a core-only test covers each market name the same way. The retired yggdryl.currency is refused (field.rs:2456-2464).
Serde. The tags of DataType, Field and Scalar (serde.rs:1206-1228, 1483-1505), and a code's text and a member's stored name (scalar.rs:438-451).
Value stream. The bytes of a value and of a datatype (valuestream.rs:408-418, with the placeholder refusal at :526-538), and the pickle bytes written through them.
Digest. The canonical feed (tag plus text; tag plus little-endian i32), as stable_hash values and row-digest values; and the std hash D19 keeps, through a door that feeds it to XXH3 (FixRegistry::stable_hash for datatypes, Python's Scalar.__hash__, python/src/scalar.rs:1607-1609, for values).
Order across kinds. Codes rank 18, ordered by (byte, text): today's code_key (scalar.rs:1439-1446). Each enum kind keeps its own rank (27-32) and orders by code within it. The rank is wire-visible where it orders an Arrow dictionary's values and any caller's sort (scalar.rs:1450-1455): pin the order of a dictionary's values across kinds.
Names. Display, the parser's names, and every LOGICAL_NAMES entry that resolves to a market kind (vocabulary.rs:121-154).
Cost gates. a_leaf_cell_read_allocates_nothing over code and enum columns (it reads only DataType::Country today, rust/tests/allocations.rs:6599; the typed_leaf_columns fixture at :6505-6537 also feeds a_record_row_costs_its_run_and_nothing_per_cell, :6555); the two size gates; and saved baselines: cargo bench -p yggdryl --bench types -- <code/enum filter> --quick --save-baseline s0, the same for --bench arrow, --bench fix -- --quick and --bench fix_allocations. A baseline lives in target/criterion and is gone in another sandbox, so also record each filter's measured numbers in DESIGN.md. If a later session finds the baseline missing, it rebuilds it from the S0 commit into this checkout's target/criterion before comparing: git worktree add ../s0 <S0 commit> && (C="$PWD/target/criterion"; cd ../s0 && CRITERION_HOME="$C" cargo bench -p yggdryl --bench <name> -- <filter> --quick --save-baseline s0). Each worktree has its own target/, and criterion 0.7.0 reads CRITERION_HOME first (criterion-0.7.0/src/lib.rs:137 in the cargo registry).
Where the pins live: a kind's own facts in rust/tests/root/<kind>.rs; the cross-kind order in rust/tests/root/scalar.rs until S1 creates its root file, when they move to rust/tests/root/market.rs. In S4 they move with their kinds. The binding facts sit beside the binding pins they extend: Scalar.__hash__ values and the pickle bytes in python/tests/test_scalar.py, the order of DATA_TYPE_IDS in python/tests/test_version.py (beside :314) and of dataTypeIds in node/tests/fields.test.js (beside :320). S0 builds the extension (maturin develop, Tooling) and the addon (npm run --prefix node build:debug) and runs those three files green before its commit, not only in CI.
S0, design (no edits under any src/)
Set up.
Install the Tooling. Run git fetch origin, create D1's branch from origin/main, and record HEAD.
Write this prompt, verbatim, to .handoff/next/MARKET_SPLIT_PROMPT.md. git status --short must then show that file alone.
Confirm cargo --version, $PY --version, and that $PY -c "import pandas, polars, tzdata, xxhash, requests" succeeds.
Re-count at HEAD, with the variables and the .handoff/ exclusion of The counts: the lines naming the 22 kinds in what stays in the core, git grep -nE "\b($K)\b" -- rust/src $STAY | wc -l (1,352 lines in 27 files at 2ae975674; -c lists them, and S1's Dispatch names the largest); the code_scalars!/enum_scalars! sites, git grep -nE '\b(code_scalars|enum_scalars)!' -- rust/src (23 lines in 10 files); the DataTypeId::<market> lines (D3's command: 243, plus its 15 outside rust/src and rust/tests); D6's items; the media sites S2's list names. Record each count in DESIGN.md with its command, as the baseline later slices compare against. A figure this prompt gives with its command that moved by more than the commits since 2ae975674 explain is a finding; an "about" figure is replaced by the recount, with no comparison.
Write .handoff/split/DESIGN.md. The crate map; U1-U7; D1-D21 each with its evidence and the alternative refused; the ledger; the risks (the D19 pins, the D11 namespace, S4's and S6's size, the instrument-registry change's rebase, the first publish of the new names); the slice table; S1's file sets per worker.
Spike D3 and D2's sizes on a scratch branch. Check whether DataTypeId(u8) with CamelCase associated consts compiles across rust/src once the matches get a _ arm, and whether Scalar and Serie stay at 48 and 40 bytes. Record the answers and delete the branch.
Pin the contracts and run them green on today's tree.
Review. One reviewer (opus where the harness offers subagents, else a separate foreground pass) reads DESIGN.md against AGENTS.md and the counts step 2 recorded. The foreground takes each finding, or records in DESIGN.md why not.
Commit the pins, this prompt and DESIGN.md as "Pin the market kinds' wire and cost contracts", push, open the draft PR, and read its CI green. Only then edit src/.
S1, the market extension point
The core (paths under rust/src/):
New root files. D8's mechanism, and market.rs holding MarketKind, MarketType, MarketScalar, MarketSerie, the register and its refusals (D2, D3, D7, D20).
The four root enums. DataType, Field (field_leaves!, field.rs:936-960, 1589-1654), Scalar and Serie lose their 22 per-kind variants and gain Market. State keeps its own (D4).
Their shadows collapse with them: Shape (datatype.rs:761-844, per D19), DataTypeRef and DataTypeWire (serde.rs:511-606, 840-931), and StructuralWire (scalar.rs:749-813).
Dispatch. Every dispatch site collapses to one Market arm. git grep -nE "\b($K)\b" -- rust/src $STAY prints 1,352 lines in 27 files at 2ae975674 - an upper bound, comments and the non-market Unit, Side and Country included - the largest scalar.rs 273, serie.rs 176, datatype.rs 139, serde.rs 132, datatype_id.rs 108, serie/value.rs 68, default.rs 61, value/canonical.rs 56, string.rs 47 and budget.rs 34. The code_scalars!/enum_scalars! patterns (scalar.rs:1358-1393; 23 uses in 10 files) become that arm's pattern; the 65 helper-call lines in what stays (git grep -nE '\b(is_code|code_width|enum_code|enum_name)\b' -- rust/src $STAY; 93 across rust/src) keep reading the family.
Ids. DataTypeId becomes the newtype, and the 243 DataTypeId::<market> lines take Kind::ID (D3).
Families. DataTypeKind::Code and DataTypeKind::Enum keep their ranges; the register owns which bytes in them are claimed.
The 22 kinds. Each becomes a MarketKind static in its own root file (ccy.rs ... pluginside.rs), claimed through the self-seeding register (D7). code_value! (code.rs:126-160) builds a descriptor; enum_leaf! (enums.rs:196-418) serves State alone, or folds into it.
Listings. The ones the parser and the bindings read answer the registered kinds: LOGICAL_NAMES (vocabulary.rs:116-154), DataType::CODES (string.rs:733-753), StringEnum::PREBUILT (string.rs:2403), arrow_extension_names() (datatype_id.rs:636-643), DataTypeId::all() (D3), and the shared-field table (typed.rs:330-356).
Arrow. One recognizer arm handles every registered name (field.rs:2446-2571, code.rs:309-330, enums.rs:519-529). The arrow-rs ExtensionType impls (arrow/extension.rs:54-122) stay one per kind, because the trait is Sized with an associated const NAME: &'static str (arrow-schema-59.2.0/src/extension/mod.rs:187, 207 in the cargo registry). Each takes NAME from a const on the kind (Isin::EXTENSION_NAME, which its MarketKind static also reads), never from DataTypeId::arrow_extension_name, and moves with its kind in S4 (a foreign trait on a local type).
Casts. CodeIngest and EnumIngest (cast.rs:1477-1487, 2617-2818) stay core kernels. The code kernel reads the descriptor's Text { width } once per array and calls its canonical-check pointer per row, the per-row cost today's field.dtype() match already pays (string.rs:618-667; the kernel is generic only over WIDTH, string.rs:513). The enum kernel reads the descriptor's member table. Certification (cast.rs:1404-1410) stays in the core; no budget or proof type is published.
Cell readers. One RunReading per kind, resolved at landing (serie/value.rs:156-185, 573-599, 686-728).
Tests (in rust/tests/ only):
Contracts. S0's pins stay unmoved.
The register runs in its own harness target, rust/tests/market_register.rs, which owns its process as spill_doors.rs does. A test-only code kind claims 0x70, the code family's one spare (datatype_kind.rs:150-156), and a test-only enum kind claims 0xc7; no other target registers a kind. Pin the claim; every refusal of D7; every intake door over them (parser, serde, value stream, Arrow in both directions, cast, digest); and the install rule D7 records.
Absent kinds. In --test root, an unclaimed byte (0xcf, which no kind claims) and an unregistered serde tag and parser name are refused, and the refusal names the missing registration, beside the lossless Arrow fallback. No listing pin runs in a process that claims; from S4, a harness that claims through a support install calls it first in every test, so its listings are one registry.
Allocations. Two rows: one reads a registered column's cells at two corpus sizes and allocates nothing; one runs a registered cast ingest.
Existing suites that named a deleted variant are re-spelled by script, from the compiler's list.
Bindings. Re-spell onto Market, with no change to any Python or JavaScript name:
Python: python/src/scalar.rs (41 lines; CODES at :669-675, as_py at :1970-1977, and :402-422, 763-784, 1917-1921); python/src/datatype.rs (:755, :1127, :2367, and the extension registration at :1120-1121); python/src/{fix,isin_registry}.rs, python/src/graph/book.rs; python/yggdryl/extension.py:234-252; the member tables at python/src/lib.rs:614-657; DATA_TYPE_IDS (lib.rs:431-434) over DataTypeId::all().
Node: node/src/value.rs, node/src/datatype.rs:93, 324, 1217, node/src/{country,mic}.rs, and dataTypeIds (node/src/enums/vocabulary.rs:26-27).
__hash__ (python/src/scalar.rs:1607-1609) keeps its value (D19).
Docs, skills, inventories. Change only what a public Rust spelling changed:
every page and skill that git grep -nE "(Scalar|DataType|Field|DataTypeId|Serie)::($K)\b" -- docs skills names (29 files and 375 lines at 2ae975674, docs/fix/capture.md and docs/types/text/string.md among them);
.api-inventory.txt;
in AGENTS.md: the Layout rows for code.rs, the code and enum files, string.rs, datatype_id.rs and value/; the Generic scalar list; the marker rule; a "registered kind" row in Common changes; the Strings sections that name DataType::Ccy and Scalar::Ccy;
the dictionary hash, only if D19's plan was refused;
DESIGN.md's ledger, committed with the slice.
Smoke while building.
cargo check --workspace --all-targets --all-features --keep-going --message-format=short first, so the bindings' sites are listed too; then cargo check -p yggdryl --all-targets.
cargo test -p yggdryl with --test market_register; --test root <kind>; --test root filtered to scalar, datatype, datatype_id, cast, enums and string; --test serie value, --test xxhash, --test fix store, --test allocations leaf_cell; and --features internals for the private pins.
cargo bench -p yggdryl --bench types -- <filter> --quick --baseline s0, and the same for --bench arrow, --bench fix and --bench fix_allocations. Slower is a design answer, not a re-pin.
Then run the chain under Checks, and make one commit saying what the tree is.
The later slices
Each later session re-counts its slice's figures at HEAD, writes its own section of DESIGN.md, and stops at its slice.
S2, the media extension point (D21; the eleven items below). The media are claimed in place, the core's parquet, iceberg and s3tables features kept: every gated site keeps its gate and each claim sits under the same gate. The features leave with their code, in S6.
Options. RecordOptions gains one registered variant. It holds the core-owned shared sections (the 14 fields record_options_fields! writes, media/options.rs:1217-1336) and the medium's own settings as a trait object, using D8's helper. IORecordOptions is : Sized (media/options.rs:142) and the host enum derives Eq, Ord and Hash (:1343), so the trait object needs those of its own. The Avro accessors (:1421-1542) and the Parquet ones (:1716-1857) move onto their media; the CSV accessors between them (:1561-1711) stay in the core.
Routing by MimeType. One lookup replaces for_mime_type (:1970), Media::open_as (media/mod.rs:117), Holder::into_media_base (holder/mod.rs:771-820), the outer-coding refusal (:747-753) and require_kind (media_serie.rs:430). An unregistered type is refused, naming the crate to add.
Leaf doors. The codec trait replaces iobase/transfer.rs:1171-1290 and iomedia.rs:1479-1490.
The Media enum. One type-erased variant serves the 8 match methods (media/mod.rs:200-305). The bindings pick their handle classes by MIME type (python/src/media/handles.rs:88-126).
Serie. The Parquet, Avro and IcebergTable variants (serie.rs:650-665) go onto GenericMedia: none of the 11 media_serie! invocations passes the optional native-read argument, and the scan state is already type-erased (media_serie.rs:14-21).
Table formats. A register replaces the 11 iceberg::located sites, covering Located (iceberg/mod.rs:146-318), the write-session hooks (transfer.rs:587-614, 879-886, 1058-1080) and WriteCount::skip. The ISIN registry store moves onto it now: it needs only is-whole, overwrite-whole and clear (isin_registry/store.rs:53-66, 400-403), so yggdryl-market reaches no Iceberg item at S4.
Warehouse. Catalog, Namespace and Table each get one registered variant; a catalog factory is keyed by the type word and the scheme (warehouse/catalog.rs:95-139); Holder::from_url gets a locator (holder/mod.rs:364-376); decide whether the core keeps the Iceberg folder names (warehouse/folder.rs:490-520); Site::Store is ungated to s3.
Errors. One external-error variant replaces Error::Iceberg (error.rs:161-172) and From<ParquetError> (parquet/mod.rs:2524).
Capability. A downcast hook on IOMedia replaces the three Parquet methods (iomedia.rs:369-409, forwarded in 8 files).
Pushdown. D21: a registered medium reads the options' sections exactly as a medium in the core does today; filter_phases (expression/mod.rs:1036) is published. apply_columns keeps its signature; S8b changes it.
Logging. D10's rule covers yggdryl.iceberg.table. D17 is confirmed here.
S3, the remaining seams, in place. D5's spelling; D6's market and FIX list; D9's trait and protocol-view builder, with the FIX view moved into fix/; D10's dispositions, with State's FIX tables moved into fix/ as free functions (D4). Proof: the dictionary hash and crate dump unmoved, cargo check --workspace --all-targets, and the whole run.
S4, yggdryl-market and yggdryl-fix.
Move. git mv the files, then run one compiler-driven script that rewrites each crate:: path to yggdryl::, yggdryl_market:: or crate::, by owner.
Paths the compiler cannot see. The same script rewrites every manifest-relative path (git grep -n CARGO_MANIFEST_DIR over the moved trees, cli/ and docs/: ../config/fix becomes ../../config/fix; 10 rustdoc examples in rust/src/fix such as batch.rs:541 and registry.rs:2972, about 20 test, bench and example files, the Rust blocks of 10 docs/fix pages) and every reader of a moved fixture (git grep -l for ulbridge.log per D14, equivalence.snapshot, seed.json) in Rust, Python, Node, the CLI and docs. These fail at run time, so the moved suites, doctests and benches are run, not only checked.
Tests. They mirror the new src/ trees, a crate's root files pinned by tests/root.rs. A test lives in the lowest crate that can name everything it uses: the FIX tests of rust/tests/graph/iterator.rs (:656, 1453), rust/tests/graph/market_data.rs (:503-536) and rust/tests/isin_registry/env.rs (:9, 74-80), scale_ulbridge.rs (keeping its iceberg gate), and every allocations/iobase_calls row reading a FIX capture go to rust/fix/tests/, each filed under the source file it pins and saying so in its //! line. Move the 32 root test files, tests/graph/, tests/isin_registry/, tests/fix/ with equivalence.snapshot, support/ulbridge.rs and support/allocations.rs, and about 44 allocations.rs rows and 3 iobase_calls.rs rows (:52, :127, :1637), byte-identical, into the new crates' cost targets. D14 re-fixtures the core. Each harness calls install() through the support function D7 names.
Benches. fix, graph and fix_allocations (rust/Cargo.toml:221-237), with their bench_profile.rs and allocation_measurement.rs support.
Examples. rust/examples/fix_capture.rs and fix_schema.rs fold into the docs or are deleted; AGENTS allows no examples/.
Generators. Re-point generate_country_currency.py:50, generate_mic_table.py:53, generate_fix_dictionary.py (D10's last row), check_isin_seed.py:60-62 and seed.rs:34's include_str!. Make generate_internals.py:32-33 and check_api_inventory.py:324 per crate.
CLI. cli/src/{fix,market,registry,quality,schema,diff,shell,warnings}.rs depend on the crates; main calls each install().
Bindings. Re-point them, module init calling install(), including python/src/scalar.rs:2830-2836, python/src/field.rs:22, 2435, 2902, 2946, python/src/serie.rs:55, node/src/text/codec.rs:2048-2057 and node/src/serie.rs:46.
Docs runner. D13's move to cli/tests/docs_examples.rs.
Manifests. The new crates per "Each new manifest"; smallvec (rust/Cargo.toml:137) leaves the core manifest if the core no longer names it.
Inventories, AGENTS.md and CI. The 78 market sections of .api-inventory.txt; in AGENTS.md, the crate sections (D13) and §2's generated-files rows; the -p lists in ci.yml (:43-44, 65, 77, 83, 95); members and default-members in Cargo.toml (:7, 10).
Packaging. cargo package --list, and $PY -m maturin sdist -m python/Cargo.toml (CI builds no sdist; only release.yml:200-208 does), so the nested path dependencies are proven before a rehearsal.
Local-only checks made stale: generate_country_currency.py --check and generate_mic_table.py --check (they fetch upstream lists; a content drift is reported as a finding, not fixed here; without network, report them skipped), check_isin_seed.py, and $PY -m unittest discover -s scripts/tests.
S5, the binding packages and the release.
Packages. python/market/ and node/market/, per D11 and D12.
Moves.
Python market tests to python/market/tests, mirroring python/market/yggdryl/market: test_fix.py (with the GIL pin), graph/, isin_registry, identifier, eusipa, country, mic, the five market enum tests (test_marketdatakind.py, test_marketdatatype.py, test_side.py, test_timeinforce.py, test_pluginside.py) and, out of test_state.py, only its from_fix_status/from_fix_msgtype cases (:63-70), State staying in the core (D4). The mixed python/yggdryl/enums/ (core enums plus the code bases, :32) and the stubs _native.pyi/__init__.pyi split.
Node market tests to node/market/tests: book.test.js, fix.test.js, fix/catalog.test.js, graph/, isin_registry, identifier, country, mic, and the five market enum tests with their .types.ts; state.test.js stays, Node having no FIX door on State. node/package.json's exports["./book"] and files (:20-24, 31-33) and node/scripts/local-binding-only.js follow.
cli/src/market.rs:58's include_bytes! of ../node/book/ moves with the display.
Peers. node/market/package.json declares "devDependencies": { "yggdryl": "file:.." } beside the exact peer, so the branch's addon satisfies it (npm 7+ installs peers from the registry otherwise); CI does the same.
Docs, generators, inventories. Market Python and JavaScript blocks in docs and skills are re-spelled to yggdryl.market and require('yggdryl-market'). The docs runner (check_docs_examples.py:50-52, 250-259) rewires yggdryl-market and yggdryl-market/book and installs python/market editable with --no-deps into python/.venv. scripts/build_docs_fix.js:35 requires fix from the new package. .api-bindings.txt and check_api_inventory.py gain python: yggdryl.market and javascript: yggdryl-market sections.
CI. The python, python-freethreaded and node jobs install and test the market packages.
Release. The sources job (release.yml:199) dry-runs every crate with one multi-package cargo publish --dry-run (MSRV 1.94, release jobs on stable). Publish in dependency order; preflight, half-out and report cover every name; the smoke step imports yggdryl.market. The user's repository configuration, before the merge: a PyPI pending trusted publisher; npm's first publish of a new package, which may need a token (unverified); CARGO_REGISTRY_TOKEN with publish-new scope (unverified). AGENTS §6's bump list gains the market manifests.
S6, the media crates (one commit; the reason is under The slices).
Each crate moves with its tests, benches, pages, exchange scripts and CI job: Avro with tests/avro, interop/avro.rs and the avro-interop job; Parquet with tests/parquet; Iceberg with tests/iceberg, tests/s3tables, interop/iceberg.rs, s3tables_handle.rs, support/s3tables.rs, medallion_ledger.rs, and the pyiceberg-interop, spark-interop and iceberg-msrv jobs.
Features. parquet, iceberg and s3tables leave the core with their code (about 220 sites in 34 files), with the optional dependencies they enable (parquet, snap, iceberg-official, uuid). The bindings and the CLI enable the crates instead: python/Cargo.toml:22-27, node/Cargo.toml:22-27, cli/Cargo.toml:41-53, stage_cli.py:125, check_docs_examples.py:209. Each binding's init and the CLI's main call the media crates' install().
The market and FIX crates' Iceberg-gated tests (scale_ulbridge.rs, tests/isin_registry/store.rs, and those over fix/schema.rs and fix/enrich.rs) take yggdryl-iceberg as a dev-dependency and call yggdryl_iceberg::install(); the crates' iceberg features are deleted.
Release. release.yml's preflight, half-out and report names, the sources dry run, the crates.io publish order (core, avro, parquet, iceberg, market, fix) and AGENTS §6 learn the three crates.
The bindings keep the media classes in the existing packages (U4).
S7, RecordOptions -> MediaOptions across the core and every crate, one sweep and one commit.
Scope. At 2ae975674, git grep -ow <word> -- . ':(exclude).handoff' | wc -l counts RecordOptions 1,302, IORecordOptions 288, record_options 894 and recordOptions 110, in 239 files (git grep -lwE 'RecordOptions|IORecordOptions|record_options|recordOptions' -- . ':(exclude).handoff' | wc -l). Re-count at HEAD across every crate under the same exclusion: from S0 on, this prompt and DESIGN.md spell these words.
from
to
RecordOptions (Rust type; Python and Node class)
MediaOptions
IORecordOptions
IOMediaOptions
record_options (method, function, field, local; Python property and keyword)
media_options
recordOptions (JavaScript)
mediaOptions
every compound (record_options_fields!, text_record_options, require_record_options, hashable_record_options, avro_record_options, CoreRecordOptions, core_record_options_from_value, PyRecordOptions, RecordOptionsLike, record_options_into_py, JsRecordOptions, RecordOptionsInput, readRecordOptions, resolvedRecordOptions, inferredRecordOptions, serieRecordOptions, _recordOptionsNative, the test names)
the same, with the word replaced
RecordOptions::for_media_type, for_mime_type; Target::record_options (expression/plan.rs:2234) and the 21 fn record_options overrides
MediaOptions::...; media_options
the prose noun "record options", where it names the type
"media options"
set_record_options does not exist; report it absent. The four published READMEs (README.md, rust/README.md, python/README.md, node/README.md) are in scope; node/index.js and node/index.d.ts are regenerated by build:debug and committed, never edited by the script.
Collisions, by hand first. require_record_options (avro/batch.rs:2467, ipc/mod.rs:823, parquet/mod.rs:2209, per medium) against MediaSerieValue::require_media_options (media_serie.rs:239, 478): read both sides; one fact gets one owner, two facts a name each for what they hold. internals::media_options is generate_internals.py's alias for media::options (lib.rs:510), not a name in one scope; check the regenerated re-export after the sweep. The script refuses to write a spelling that exists in the same scope.
On the wire. State in the commit and the docs: the pickle global RecordOptions._from_pickle (python/src/iomedia.rs:2571-2576) becomes MediaOptions._from_pickle, so a pickle written before S7 does not load after it, with no alias; the repr (:2563-2569) and the property error and warning texts (:1736-1738) change. Unchanged: the pickle state keys, stable_hash (it hashes the tags "parquet" and "avro", media/options.rs:1365-1370), every cost.
The script is .handoff/split/rename_media_options.py, committed with S7 and deleted at S9. It lexes code, comments and strings with each language's own lexing; splits identifiers at _ and at camel humps; uses exact anchors, each asserting its count; is idempotent, with a dry run first that writes .handoff/split/listings/. It scans and rewrites nothing under .handoff/ - not this prompt, not DESIGN.md, not the listings it writes there - so the instructions it runs under keep their spellings. Re-run it from the Pace check until clean; mypy --strict and tsc --noEmit give the binding lists.
Smoke. cargo test -p yggdryl --test media options, --test root iomedia, --test expression plan, and each media crate's options tests; --test iobase_calls and --test allocations whole in every crate that has them, counts unchanged and their diff the rename alone against the previous commit with the spellings mapped back; $PY -m pytest python/tests/test_iomedia.py -x -q; node --test node/tests/media/options.test.js; $PY scripts/check_api_inventory.py; $PY -m mkdocs build --strict --config-file mkdocs.yml.
Done when git grep -nIE 'RecordOptions|record_options|recordOptions|RECORD_OPTIONS' -- . ':(exclude).handoff' (no -w: the compound names are the point) and git grep -nIiE 'record[ -]options' -- . ':(exclude).handoff' print only what the listing keeps on purpose, and the inventories, AGENTS.md, docs and skills are renamed.
S8, the planned expression series (U6). Nothing of it is in the tree: SerieKind, push_plan, SelectSerie, FilterSerie, OrderSerie, LimitSerie, JoinSerie and Source::Serie have zero matches at 2ae975674 (git grep -nF <name> -- . ':(exclude).handoff', the exclusion needed once this prompt is committed), and apply_serie exists only as a crate-private key evaluator (expression/selector.rs:1708). Its design was settled outside the repository and is stated here whole; no other file describes it. Two commits, each proven alone: S8a and S8b. Each edits the crate that owns the code by then and is spelled in S7's names (MediaOptions, IOMediaOptions, media_options). The media crates implement their pushdown through the media point (D21), whose trait does not change.
Out of S8, planned in no file of the repository. If S8's design shows a dependency on one of these, record it in DESIGN.md and put it to the user:
the keyed holders: an item KeyScalar { key, value, rownum } and held and streamed WindowSerie/GroupSerie replacing today's KeySerie, KeySeries and StreamKeySerie, with window_by (adjacent runs, not shuffled) and group_by (shuffled to unique keys), the positional WindowSerie renamed RangeSerie, and stored batches passed through unless a row or byte bound is asked (Avro's alone lands in S8b, below);
the remaining Arrow doors: every Serie kind reading from and writing to an Arrow reader, batch and table (from_arrow_table, into_arrow_table, a lazy Serie::from_arrow_reader);
the ArrowCastOptions removal: the struct deleted, the engine taking one root Safety { Strict, Safe }, and a field's nullability deciding absence.
Where S8 needs a door those pieces would change, it uses today's: a lazy stream is a StreamChunkedSerie held as Serie::StreamChunked, a cast takes today's ArrowCastOptions, and the keyed kinds are today's.
S8a, D18: the prerequisites.
SerieKind, a new root file serie_kind.rs: the one kind vocabulary, one member per Serie variant family as the tree has it then - Run, Column, Chunked, Stream (native rows), StreamChunked, the keyed kinds as they stand (today Key, Keys, StreamKey), Media(Media) for every media variant, and from S8b Select, Filter, Order, Limit, Join - with as_str, Display and FromStr. Serie::kind() answers it; Python's Serie.kind getter (S8b) spells as_str; Node gains no getter (AGENTS §4). Pinned in rust/tests/root/serie_kind.rs.
Serie::from_holder(holder: impl Into<Holder>, options: Option<MediaOptions>) -> Result<Serie>, in media_serie.rs: the one door from a handle to the media kind of its media type, through the media register (D21) and the container-or-table decision open_container makes (iomedia.rs:23), never a third table beside them. It is the plan's resolution door and nothing else. IOMedia::read_serie stays the native read that MediaSerieValue::read_native calls; answering it through from_holder would loop (read_native -> read_serie -> from_holder -> read_native).
The media state reads its root lazily, into a OnceLock filled on the first field() or row read, so construction and push_plan cost no store call. This replaces with_reader's eager read_arrow_field (media_serie.rs:60).
into_held(self) -> arrow::Result<Serie>, as_held(&mut self) -> arrow::Result<&mut Self> and into_chunked(self) -> arrow::Result<ChunkedSerie> on Serie, beside is_held (serie.rs:3684): the drains. The name is not into_serie, which already means widen (SerieValue::into_serie), join (ChunkedSerie::into_serie) and slice.
apply_serie(&self, serie: Serie) -> Result<Serie>, the one evaluator.
On Term, Bound, Filter, Selector, BoundSelector, Plan and Expression (a sequence folds), and on the time_bucket and user functions through them.
On every protocol view that evaluates terms: TransformField::apply_serie (the fill of derived columns absent or at their default in every row); PartitionField::apply_serie, a one-line redirect to the transform view, which retires Python's binding-side scheme dispatch (python/src/field.rs:3303-3325); DigestField::apply_serie; and, on the seeded states Digester, Xxh32, Xxh64, Xxh3, Xxh128 and TxHasher, apply_serie(&self, serie, force), keeping force.
Field::apply_serie is serie.cast(self).
ArrowCastPlan::apply dispatches by kind, which is what keeps that redirect and the media seam's two casts lazy: today it lands its input through require_arrow_array (cast.rs:674, serie.rs:2841), which drains a stream. A held column is cast once; a chunked serie chunk by chunk, sharing every chunk the plan leaves exact; a stream lazily under the one plan (StreamChunkedSerie::cast's body, serie/arrow.rs:1699); a keyed kind item by item; a media kind into the declared field. The plan still compiles under today's ArrowCastOptions. apply_chunked (cast.rs:719) is deleted, and its callers redirect to the one apply with no public binding name changed: Python's ArrowCastPlan.apply (python/src/cast.rs:129) and Node's private _applyChunkedNative (node/src/cast.rs:60-63, wired at node/binding.js:3190-3227). Its tests (rust/tests/root/cast.rs, rust/tests/allocations.rs), pages (docs/types/cast.md, docs/types/chunked-serie.md), skills, .api-inventory.txt entry and AGENTS' cast.rs row are re-spelled. Pin that a chunked cast copies no chunk the plan leaves exact.
It binds once against the serie's root (StreamChunkedSerie::root_of, serie/arrow.rs:1922, for a stream) and dispatches by kind:
a held column: once, through the batch tier, over the columns the terms read alone (the key evaluator's narrow arm in expression/selector.rs, generalized);
a chunked serie: chunk by chunk, a filter answering a kept chunk as a slice of itself;
a stream: lazily batch by batch, the bound expression and one ArrowCastPlan compiled before the first pull (PlanCache where the schema can change); the result batches land Proven, so the stream above proves them no second time;
a keyed kind: item by item with the key columns in scope, folded into an item only where a clause reads a key path, a key-only term evaluated once per item;
a run or a native-row stream: the row tier, row by row, no batch laid out; a run names no root, so an unbound applier is refused by name;
a media kind: push_plan first, then the residual; a planned kind: its tree extended (both S8b). Until S8b, apply_serie over a media kind reads the media serie's rows (MediaSerieValue::read_native, media_serie.rs:248) and applies over them, and Plan::apply_serie evaluates its read sections - the early filter, the select, the late filter, the row bounds through WriteLimitState (media/options/limits.rs:24) - over the units of the serie it is given, building no node. S8b re-expresses both as the planned tree and push_plan.
The three kernel tiers in expression/eval.rs (row, batch, statistics) are unchanged, reached from apply_serie and the planned kinds alone.
The identity select and the always-true filter answer the input itself (Arc::ptr_eq on its buffers); the early returns (is_all, expression/selector.rs:1301, and the always-true check) stay before any wrap.
The crate-private BoundSelector::apply_serie(&Serie) (selector.rs:1708) becomes the public one, taking an owned Serie, and its internals forward (:2087) is deleted, since a caller now reaches it. apply_serie_window (:1740) and its forward (:2099) keep their names: the positional WindowSerie they cut is renamed RangeSerie only in the holder phase, which renames them with it. Record that in DESIGN.md.
The struct-null rule of apply_arrow_array (struct_rows, scattered, rebuilt_struct in expression/plan.rs) moves into apply_serie over a record serie with absent rows: present rows evaluated, absent rows scattered back.
The narrow landing. A transport batch entering an applier lands only the columns the bound terms read, under the root settled at bind (as KeyEngine does, selector.rs:1427; no schema import per call), those columns proven once. Every other column crosses as transport and is never read. The answer is assembled from the result columns and the untouched ones - a filter's mask applied to the caller's batch by filter_record_batch, a select from the narrow result plus the pass-through columns - and is Proven.
Why: wrapping the whole batch would prove every non-contract column and move UNNEST_BATCH_ALLOCATIONS = 12 and the map-key-lift constant in rust/tests/allocations.rs. Both stay unmoved, measured at the red step; a move is a defect of the landing, never a re-pin.
Behaviour pin: a batch whose column of a registered code kind (after S4, the test-only kind) holds an invalid value passes untouched through a filter on another column, and the same filter reading that column refuses it by row.
The redirects.
apply_arrow_batch(&batch) on Filter, Selector, BoundSelector, Bound, Expression, TransformField, PartitionField and the digest views: the narrow landing, apply_serie, the assembled batch.
apply_arrow_reader(reader) on Filter, Selector, BoundSelector, Bound, Plan, Expression and TransformField (PartitionField and the digest views have none, and gain none): the stream kind over the reader, apply_serie, into_arrow_reader; a reader whose plan and expression are the identity is handed back untouched.
apply_arrow_array(&array), and the crate-private Plan::apply_arrow_array: Serie::from_arrow_array, apply_serie, into_arrow_array.
apply_records(schema, records) on Filter, Selector and Expression: apply_serie over the native-row stream, the row tier per row.
apply_scalar on Filter, Selector, BoundSelector and FieldPath, and Bound::apply_scalar(row) (today's Bound::eval, expression/bind.rs:384, renamed): apply_serie over a one-row run, then scalar(0), pinned to allocate no batch.
Deleted: Bound::matches (bind.rs:396), evaluate, filter_mask, filter and filter_reader (expression/arrow.rs:160, 174, 195, 220); Python's Bound.evaluate_arrow_batch, filter_mask_arrow_batch, filter_arrow_batch and filter_arrow_reader; Node's _filterArrow*Native. Their crate callers move to the serie they hold: join.rs's key_arrays and pushdown_term to BoundSelector/Bound::apply_serie per chunk; xxhash/arrow.rs's level batch to the serie it was built from; iceberg/scan.rs's apply_predicates to Filter::apply_arrow_batch on the decoded batch (only the predicate columns land, inside the scan worker); media/merge.rs's key_columns to BoundSelector::apply_arrow_batch; the market crate's graph/arrow.rs:1643 (filter_mask) to apply_serie.
Unchanged: Field::apply_arrow_batch (the cast's transport face, reconcile_batch, no row proven), Field::apply_arrow_reader (the inner reader handed back under an identity plan) and Field::apply_arrow_schema.
Afterwards nothing in expression/, a media reader or a protocol view evaluates a term against a RecordBatch directly; the RecordBatch/BatchReader signatures stay and do their work through the serie. The market crate's graph/view.rs:245 and graph/serve.rs:1415 keep working as redirects and are re-spelled to apply_serie in S8a.
The media seam. IOMediaOptions::apply_serie(&self, serie: Serie, existing: Option<&Field>) -> Result<Serie> is the declared cast, then self.plan().read_sections().apply_serie(..) (expression/plan.rs:1449; the early, select and late phases split by the one owner expression::filter_phases), then the stored cast, over every serie kind and lazily on a stream.
It is the write session's Shaping (media/options.rs:1025) compiled per landed root: one chain where three stand at 2ae975674 - Shaping, IORecordOptions::apply_arrow_reader (media/options.rs:915-931) and prepare_arrow_write_deriving (iobase/transfer.rs:442-462).
apply_arrow_reader(reader, existing) and apply_arrow_batch(batch, existing) become redirects through it. apply_arrow_expressions (media/options.rs:491) and apply_stream (:512) are deleted, so the CSV, text and Avro leaves (iomedia.rs:1479-1490), a media serie's edited rows (media_serie.rs:96), http/request.rs, csv/media.rs and coding/mod.rs call the one door. opened_field (iomedia.rs:35) learns its schema through plan().field_from.
read_record_serie (iomedia.rs:1470) stays the native read, and read_scoped (iceberg/table.rs:2221) becomes what the Iceberg medium does with the sections push_plan set.
Row bounds stay limit_arrow_reader over WriteLimitState (media/options/limits.rs:24), the one owner LimitSerie reads too.
Shaping::apply(batch) keeps its transport signature, because the session pulls transport batches and lands nothing per batch. Each step is a redirect that lands narrow: the declared cast (reconcile_batch), Derivation::apply through TransformField::apply_arrow_batch, the early filter, the select, the late filter, the stored cast.
iobase_calls for every media read behind a where or select stays at today's count, measured at the red step. Plan::execute over a CSV leaf and over a Parquet leaf has no row at 2ae975674 (the file's record rows are mod records, rust/tests/iobase_calls.rs:831): write those two rows on the tree before S8a's first edit, run them green, then hold them unmoved through S8a and S8b.
S8a's pins. rust/tests/expression/{eval,filter,selector,plan,arrow,transform}.rs: every applier's apply_arrow_batch equal to apply_serie over the landed batch, byte for byte, on every kind; apply_scalar equal to the one-row apply_serie; the struct-null rule. allocations: apply_arrow_batch the narrow landing plus the kernels, a thousand batches through a stream a thousand times one, the identity select and the always-true filter zero. iobase_calls: a media serie's construction nothing, its into_arrow_reader its read_arrow_reader and nothing more. Bench: rust/benchmarks/expression.rs gains apply_serie/* per kind.
S8b: the planned kinds. Lazy verbs on Serie alone, taking self (a caller who keeps the source spells .clone(), a pointer bump):
verb
answers
select(self, by: impl IntoSelector) -> Result<Serie>
Serie::Select(Arc<SelectSerie>)
filter(self, by: impl IntoFilter) -> Result<Serie>
Serie::Filter(Arc<FilterSerie>)
order_by(self, by: impl IntoOrderings) -> Result<Serie>
Serie::Order(Arc<OrderSerie>)
limit(self, n: u64) -> Serie, offset(self, n: u64) -> Serie
one Serie::Limit(Arc<LimitSerie>)
join(self, other: impl Into<Serie>, by: impl IntoJoinKeys, how: JoinKind, options: &JoinOptions) -> Result<Serie>
Serie::Join(Arc<JoinSerie>)
The nodes. One root file per kind - select_serie.rs, filter_serie.rs, order_serie.rs, limit_serie.rs, join_serie.rs - each holding the node, its source() (sides() for a join) and clause accessor, and the impl Serie block of its own verb, so the mirrored test file pins the verb: SelectSerie { source: Serie, selector, root: Arc<Field> }, FilterSerie { source, filter, root }, OrderSerie { source, by: Vec<Ordering>, root }, LimitSerie { source, offset: u64, limit: Option<u64>, root }, JoinSerie { left, right, keys: JoinKeys, how: JoinKind, options: JoinOptions, root }. The limit node is not named Bounds, which expression::Bounds already holds (container statistics). The fold, Serie::plan() and the one pull live in serie.rs, never once per node. Serie stays 40 bytes (serie.rs:677), or the assertion moves with its reason.
into_sorted/as_sorted and into_sort_by/as_sort_by stay as the eager state verbs. AGENTS' Public vocabulary gains the row "a bare clause verb plans; the pull evaluates". set_plan keeps refusing $.order_by: the media options gain no order section.
A second limit or offset folds by sequence: offset(m) over {o, l} gives {o + m, l.saturating_sub(m)}, and limit(n) gives {o, min(l, n)}. So s.limit(10).offset(5) is five rows and s.offset(5).limit(10) ten; plan() prints limit l offset o.
join is the one join verb. join_with is deleted; its three bodies (serie/join.rs:68, 94, 126) become JoinSerie's evaluation, and serie.join(..)?.into_held()? is today's join_with. Node's joinWith (node/binding.js:1505, 2246) keeps its name over that, as a re-spelled existing door; the handoff puts that one spelling difference to the user.
Binding. A clause binds at construction where the source's root is in hand (a held column, a chunked serie, a stream, a keyed kind, a node over one of those), and a term that binds against no column is refused there, by path. Over a media serie, the clause is kept as parsed and binds at the first pull, so construction makes no store call. A node's root is its clause's answer (Selector::apply_field, Filter::apply_field, the join's output root), read without a row where the source's root is in hand. Before its pull a tree costs the nodes plus their binds, constant in rows, with zero store calls.
Evaluation.
A consuming pull (into_held, into_chunked, into_arrow_reader, into_stream, the write doors) takes the sole handle and holds nothing: the tree is evaluated as its units are pulled.
A row read (len, scalar, iter, memory_size, is_sorted, or a Scalar::Serie holding it, read) evaluates the tree once, into one OnceLock, under the spill bound - the cache a stream's held_leaf (serie.rs:3530) fills - so len then scalar(i) pulls a counting source once.
Until then is_held is false and JoinOptions::build treats the node as a stream, so an unevaluated node is never chosen as the build side on a size of zero. Two clones of a JoinSerie build once; pin that.
Pushdown. At the first pull, adjacent nodes fold into one Plan (filter before select, order and limit after, a join's keys into its probe side as join.rs's pushes_down/pushdown_term do). The source answers the crate-private Serie::push_plan(&self, plan) -> Result<(Serie, Plan)>: the source to read, plus the residual. push_plan never evaluates and mutates no shared source; it rewrites sections and answers. Per source kind:
A not-yet-read media serie takes select, where and limit/offset through IOMediaOptions::set_plan (media/options.rs:411), then planned (media_serie.rs:120): one default on MediaSerieValue, with no per-medium arm, reaching each media crate's serie as it reaches one in the core. Its residual is exactly order by and the joins: a computed filter or a built select is evaluated inside the read, never again by the node. with_filter, with_select and with_row_range are deleted (media_serie.rs:261-298), their test sites in rust/tests/root/media_serie.rs re-spelled to the planned verbs; with_key and key_values wait for the holder phase unless D18 says otherwise. An order by an Iceberg table's declared order already proves is a clone, by serie/order.rs's existing rule.
What each medium does with the sections. apply_columns (media/options.rs:465, today Option<Vec<String>> of root names) answers crate-private FieldPaths, and a medium that projects roots keeps reading roots. Parquet reads only the leaf column chunks select a.b, c[0].d names (projection_indices, arrow/mod.rs:712, fetch, parquet/mod.rs:1264, and ParallelRead, :1778, leaf-aware; the root-only schema.project replaced by a pruned struct schema) and prunes row groups by statistics (prune, parquet/mod.rs:1123, widened to leaf paths where the statistics are per leaf). Iceberg prunes by nested field id, manifest and file, and counts files_skipped. Avro brings one piece of the holder phase's pass-through into S8b: avro::read_batch_reader (avro/batch.rs:254) becomes AvroSerie's stream of its stored blocks, the header's schema naming the columns, projection and row bounds applied at the block (a block skipped whole under limit/offset). Its filter is not pruned at the block; it is evaluated inside the read through the media seam, so the node's residual stays order by and the joins. The rest of the pass-through stays with the holder phase. CSV and text stay decode-cut, a follow-up.
A held or edited media serie takes nothing: zero store calls, pinned.
A held or chunked column takes nothing and evaluates per unit through apply_serie; an identity select or an always-true filter answers with the unit itself.
A stream takes the plan lazily, batch by batch.
A keyed kind takes the conjuncts of a filter that name key paths alone, applied to the key before an item is built; the payload clauses run per item.
A JoinSerie pushes the first join's key filter into the probe side's push_plan (what the plan's Join does today through isin_term); the held side is built once.
The residual. OrderSerie evaluates through serie/order.rs (into_sort_by on a held or chunked source, StreamChunkedSerie::into_sort_by on a stream), so Plan's sorted_arrow_reader and order_indices go: one sort. LimitSerie stops at its bound: over a stream it stops pulling at the limit, and over a held or chunked source it reads only the chunks the offset and limit reach, never a filter of every row and a slice after; a limit of 10 over a 1000-batch counting stream pulls one batch. arrow::sliced_reader and Sliced (arrow/mod.rs:804, 818) are deleted. Every kind pins serie.clone().filter(f)?.select(s)?.into_held()? == s.apply_serie(f.apply_serie(serie)?)?. serie.plan() reads the tree back as the Plan it is, and serie.explain() prints it with what each node pushed and kept, on Serie alone.
The plan's source. Source::Serie(Serie) sits beside Target and Plan (expression/plan.rs:318), reached through Plan::read_from(impl Into<Source>) (:1128) and From<Serie> for Source; there is no Plan::from_serie. Source's derives become hand impls for that variant: it compares, orders and hashes by root and SerieKind, never by rows (a stream is never drained by Eq); static_root (:352, :1655) answers the serie's root, so a plan over a serie states its schema without a read; it prints <serie: {root name}, {kind}>, serializes as that pair, and deserializing one is refused at $.from. Plan::join takes (other: impl Into<Source>, by: impl IntoJoinKeys, how, options: &JoinOptions) and Plan::order_by takes impl IntoOrderings, the serie verbs' argument orders and intakes (today :1166 and :1205).
Resolution is unchanged as the one location door (Target::holder, :2134; write_holder, :2159). Once the registry lock is released, the result is wrapped in Serie::from_holder(holder, Some(target.media_options(&holder)?)), so every step after resolution speaks Serie.
execute and execute_in (:2326, 2336, today answering a BatchReader) build the tree over the source - source.filter(where)?.select(select)?.order_by(order)?.limit(..).offset(..), the joins as JoinSerie nodes - and answer a lazy Serie. A plan with a create or write verb consumes the tree's pull through the destination's write_serie (by verb, nothing landed) and answers its IOResult as one row: IOResult gains field() (struct<read_rows: uint64, written_rows: uint64, skipped_rows: uint64>), into_scalar and from_scalar, pinned in rust/tests/root/ioresult.rs.
Deleted: apply_in, shape_arrow_reader, narrowed_arrow_reader, sorted_arrow_reader, order_indices (expression/plan.rs:2462, 2487, 2587, 2626, 2641), empty_reader (:2865) and write_in's reader shape (:2760), the write verbs consuming the tree through write_serie. Plan::apply_arrow_reader is S8a's redirect.
Callers. xmla/service.rs:926-930 reads the returned serie's field() and rows into its rowset, and resolved passes Source::Serie through map_sources unchanged. The market and FIX crates' graph/serve.rs (2), graph/book.rs (1) and fix/market.rs (1) use the doors S8 deletes.
Bindings.
Python gains, on Serie, select, filter, order_by, limit, offset, join (today's join_with signature, python/src/serie.rs:1462-1490, under the one name), plan, explain(), into_held(), into_chunked() and the kind and is_held getters; ChunkedSerie, StreamChunkedSerie and StreamSerie reach them through Serie.from_(x). Every non-held kind exports __arrow_c_stream__ lazily and refuses __arrow_c_array__, into_arrow_array and into_numpy by name; Scalar.from_(serie) and into_scalar() drain a lazy kind through into_held first.
Python's Serie.from_holder(handle, options=None) is the one way a binding reaches the pushdown, pinned by Serie.from_holder(h).select("a") over a Parquet leaf reading only a's chunks. It needs a second handle on the binding's resource, which Holder::from_handle already gives (holder/mod.rs:502; AGENTS' holder/ row: an in-memory buffer and one HTTP answer are refused as Error::Unsupported). Serie.from_holder reaches the media kind through it as it stands, and S8b changes none of its arms.
Python's Plan.with_source(source) takes a serie and Plan.source answers it (no Plan.from_, which would collide with the inferring constructors). Plan.execute() and execute_in(warehouse) answer a Serie (a write plan the one-row IOResult record), run detached (py.detach), pinned in a child interpreter under a deadline as test_fix.py does. Plan.__reduce__ and into_json refuse a serie source naming $.from; equality and stable_hash go by root and kind, as the docstring says.
Node re-spells its existing doors only (AGENTS §4): plan.execute() answers a Serie, joinWith stays as above, and the deleted _filterArrow*Native go. No new Node door.
S8b's pins. rust/tests/root/{select_serie,filter_serie,order_serie,limit_serie,join_serie}.rs: the node's root read without a row; a term binding against no column refused at construction over a held source and at the first pull over a media source; the fold, with the limit/offset rule; plan() and explain() strings per source kind; the residual per source kind; the pull rule; the limit over the counting stream; chunks past a limit untouched; the equality above on every kind; join equal to the deleted join_with's rows on Serie, ChunkedSerie and StreamChunkedSerie. rust/tests/expression/plan.rs: read_from(serie) over every kind equal to the location form, execute answering the lazy serie and the IOResult row, Source::Serie equal by root and kind and refused on deserialization, order by through one sort. allocations: a five-node tree costs the nodes plus their binds and zero store calls. iobase_calls: push_plan on a media serie costs nothing; select a over a Parquet leaf reads only column a's chunks and select a.b only that leaf, the row asserting the requested ranges over a fixture larger than WHOLE_READ_BYTES (1 MiB) with column chunks more than FETCH_GAP_BYTES apart; a where on a column with statistics skips the row groups the bound excludes; an Iceberg table's files_skipped row; the two Plan::execute rows S8a wrote first, unmoved.
Docs. docs/types/serie.md gains "Planning a read" (the verbs, plan, explain, the pushdown table, the pull rule); docs/expression/ states the apply_serie rule once ("Applying an expression") with the door table beside it, and its plan pages are rewritten around the serie source; the skills' recipes follow; AGENTS.md's serie.rs and expression/ rows ("apply_serie first, everything else derived from it") and the "Serie is the collection" table gain the planned kinds.
Deferred: Parquet page pruning (the tree has no page index and no RowSelection under parquet/) and nested statistics beyond per-leaf row-group bounds; top-k for order_by followed by limit; CSV and text onto batch readers.
S9, the final sweep. READMEs (root, rust/, node/) and rust/OPTIMIZATION.md; the architecture, contributing, testing and benchmarks pages; the mkdocs nav (Codes, Enums, Graph and FIX sections, mkdocs.yml:164-279); skills/README.md:44-45, .claude-plugin/plugin.json and .claude-plugin/marketplace.json; the release rehearsal if S5's has not run; .handoff/split/ and both .handoff/next/MARKET_SPLIT_*.md files deleted; then the user's go bumps the version (D1) and merges.
Checks (every slice)
Background chain. One script, one log, a marker per step, launched once the slice's last cargo-locking phase has settled. In order:
cargo test -p yggdryl --all-targets --all-features --no-fail-fast.
cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings, then cargo clippy --workspace --all-features --all-targets -- -D warnings.
cargo test -p yggdryl --doc.
RUSTDOCFLAGS='-D warnings' cargo doc -p yggdryl --no-deps, then RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --all-features (ci.yml:78-81): S1 deletes variants doc comments link to, and S4 leaves links from kept core files to moved items (graph/element.rs:6, 314-315, 971, text/options.rs:607).
cargo test -p yggdryl-cli --all-targets --no-fail-fast (ci.yml:69-71; cli/tests/fix.rs and cli/tests/market.rs pin market spellings).
cargo check -p yggdryl --profile bench --benches --all-features (ci.yml:82-83).
§3's pre-push block: maturin develop (as under Tooling), $PY -m pytest python/tests, and $PY -m mypy --strict over §3's three paths.
§4's pre-push block: npm run --prefix node test:package:debug; git diff --exit-code -- node/index.js node/index.d.ts; cargo build --locked -p yggdryl-cli; npm test --prefix node; node scripts/build_docs_playground.js --check and node scripts/build_docs_fix.js --check.
$PY scripts/check_docs_examples.py --lang rust, then python, then javascript.
From S4, steps 1-6 take every crate's -p.
Foreground, meanwhile. cargo fmt --all, once, after the last worker returns; $PY scripts/check_api_inventory.py; $PY scripts/generate_internals.py --check; $PY -m mkdocs build --strict --config-file mkdocs.yml; the grep for test code under src/; git status --short.
Then commit, push and read the draft PR's CI. Push one slice and read its run to the end before pushing the next commit to the PR: a push cancels the PR's run in progress (ci.yml:23-25), and a slice whose run was cancelled is unproven. Handle a red run as §2's "A red run" says: read the log, reproduce narrowly, fix, smoke and push. Never re-run hoping for green, and never relax a check.
Sandbox effects. No host effect is known on Linux. A local failure counts as a sandbox effect only if the same test fails on 2ae975674 in the same sandbox (git worktree add ../base 2ae975674, then the one test there); otherwise it is real. If you run on Windows, four tests are known to fail on untouched code: the CRLF crate-dump test, truncating a mapped file, case-insensitive NTFS, and a WSAEACCES UDP bind. Name any such failure in the report; CI on Linux is the proof.
Done means (the first session)
.handoff/split/DESIGN.md is committed. D1-D21 are each decided, or deferred to a named slice, and the review has been answered.
S0's pins commit is pushed with its CI read green, and the pins are unmoved by S1; the one exception is a D19 move, made only if D19's plan was refused, once and accounted for.
S1 is one commit on the program branch, pushed, with the draft PR's CI read green.
No test code is under any src/. No AWS resource was touched. Nothing was published.
A later session's done is its slice's row under The slices, pushed with CI read green, and DESIGN.md's ledger updated.
Handoff
Write .handoff/next/MARKET_SPLIT_NEXT.md, commit it on its own after the slice's CI is green, and push it to the program branch. Use the AGENTS handoff keys:
Goal; Invariants;
State: the program branch, HEAD, the PR, the slices done, and any origin/main merge (D1);
Checks: each command with its exact result, and the skipped checks by name, a network read D11 or D12 could not make included;
Blockers;
Next: the next slice (after the first session, S2, the media extension point), with the exact first command.
Add two more sections:
The decision ledger: the rows of DESIGN.md's ledger decided or changed this session, as D# | decision | evidence | slice, and the path of the whole ledger.
Questions for the user:
the npm name (D12);
the artifact sizes, and whether option d's unchanged wheel size is acceptable (D11);
S6 as one commit, or the reversed order S6a-c if its diff is too large;
a persisted std hash, only if D19's plan was refused;
the go for S5's rehearsal and the repository configuration it needs;
any registry or network check that was skipped.
The session's final reply names the program branch and the PR, and asks the user to paste Program branch: <name> into every later session's prompt.
This prompt and .handoff/split/ stay on the program branch until S9 deletes them.
