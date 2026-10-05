# Yggdryl agent skills

Skills that teach a coding agent to use the `yggdryl` package - the Rust
crate, the Python wheel and the npm package - the way its core is built to be
used: which door answers a task, what keeps a read streamed and a cast
compiled once, and the spellings an agent gets wrong when it guesses.

They are for code that *uses* the package. Changing the package itself is
governed by [`AGENTS.md`](../AGENTS.md).

## Install

Claude Code, as a plugin from this repository's marketplace:

```bash
claude plugin marketplace add Platob/yggdryl
claude plugin install yggdryl@yggdryl
```

Inside a session the same is `/plugin marketplace add Platob/yggdryl`, then
`/plugin install yggdryl@yggdryl`. The plugin tracks the repository, so an
update brings the skills written for the current release.

Any other agent that reads Agent Skills: copy the folders you want into its
skills directory (for Claude Code without the plugin, `.claude/skills/` in a
project or `~/.claude/skills/`), or point the agent at `skills/yggdryl/SKILL.md`
and let it follow the links. Every skill is plain Markdown with a `name` and
`description` front matter.

## The set

| Skill | Teaches |
| --- | --- |
| [`yggdryl`](yggdryl/SKILL.md) | install and features, the model (`DataType`, `Field`, `Scalar`, `Serie`, `IOBase`), cross-language conventions, which skill answers a task - load first |
| [`yggdryl-types`](yggdryl-types/SKILL.md) | datatype expressions, fields and metadata, the value door, every datatype family (geospatial, `variant`, `version`, `interval` and run-end included), codes and enums with their FIX wire values, record classes |
| [`yggdryl-arrow`](yggdryl-arrow/SKILL.md) | `Serie`, `ChunkedSerie`, `SerieReader`, `ArrowCastPlan`; pyarrow, pandas, polars, NumPy and Arrow JS in and out; sorting, windows of equal keys, joins, spilling, byte sizes |
| [`yggdryl-storage`](yggdryl-storage/SKILL.md) | `IOBase` handles and bytes, local, Arrow filesystems, ZIP and S3 / GCS / Azure backends with the AWS identity chain, the HTTP(S) client and serving a handle (`http.Server`), page caches, compression, charsets, call counts |
| [`yggdryl-warehouse`](yggdryl-warehouse/SKILL.md) | catalogs, namespaces and tables: `Warehouse`, `SystemWarehouse`, memory, folder, Iceberg and Amazon S3 Tables catalogs, `MediaTable`, dotted paths, properties, the views, the XML for Analysis provider (`yggdryl xmla serve`) |
| [`yggdryl-uri`](yggdryl-uri/SKILL.md) | `Uri`, `Url`, `Urn`, `Arn`, paths, globs, hive partitions |
| [`yggdryl-records`](yggdryl-records/SKILL.md) | record reads and writes, `RecordOptions`, Arrow IPC, Parquet, Avro, CSV, Excel, XML for Analysis rowsets, text lines, Iceberg (refs, expiry, compaction, inspection), partitions and what an overwrite replaces |
| [`yggdryl-documents`](yggdryl-documents/SKILL.md) | JSON, JSON Lines, YAML, TOML and XML over `Scalar`; namespace-aware XML and SOAP 1.1 envelopes (Rust) |
| [`yggdryl-expressions`](yggdryl-expressions/SKILL.md) | terms, filters, selectors, plans and their joins, `time_bucket`, evaluation and pushdown |
| [`yggdryl-hashing`](yggdryl-hashing/SKILL.md) | xxHash digests of bytes, handles and values, stable hashes, row-digest holders, TxHash and UUIDv7 keys |
| [`yggdryl-fix`](yggdryl-fix/SKILL.md) | FIX decode and encode, the registry and store, Arrow rows and serie faces, captures, lifecycle, books, `yggdryl fix` |
| [`yggdryl-market-data`](yggdryl-market-data/SKILL.md) | orders, quotes, executions, trades, order books and their deltas as rows, candles, the instrument registry, market data views, the book display (`yggdryl market serve`, `BookService`) |
| [`yggdryl-logging`](yggdryl-logging/SKILL.md) | the logger tree and levels, the terminal line, `FileHandler` through any handle, formatters, deduplication, the `log` facade, Python's `logging` hosting the core, the JavaScript namespace, shutdown, where the core's warnings land |

Each skill is a `SKILL.md` - the decision table, the rules, the pitfalls -
and `references/rust.md`, `references/python.md` and
`references/javascript.md` with runnable recipes, so an agent reads only the
language it writes; a layer with a vocabulary of its own adds one topic
reference (`spellings.md`, `cast-rules.md`, `backends.md`, `formats.md`,
`grammar.md`, `cli.md`).

## Kept true

Every `rust`, `python` and `javascript` block here runs in CI beside the
documentation's own, through `scripts/check_docs_examples.py`: a skill that
teaches a method the package no longer has fails the build. A change to a
public surface updates the skill that teaches it in the same change.
