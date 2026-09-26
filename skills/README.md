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
| [`yggdryl`](yggdryl/SKILL.md) | install, the model (`DataType`, `Field`, `Scalar`, `Serie`, `IOBase`), cross-language conventions, which skill answers a task - load first |
| [`yggdryl-types`](yggdryl-types/SKILL.md) | datatype expressions, fields and metadata, the value door, every datatype family, record classes |
| [`yggdryl-arrow`](yggdryl-arrow/SKILL.md) | `Serie`, `ChunkedSerie`, `SerieReader`, `ArrowCastPlan`; pyarrow, pandas, polars, NumPy and Arrow JS in and out |
| [`yggdryl-storage`](yggdryl-storage/SKILL.md) | `IOBase` handles and bytes, local, ZIP and object-store backends, compression, charsets, call counts |
| [`yggdryl-uri`](yggdryl-uri/SKILL.md) | `Uri`, `Url`, `Urn`, `Arn`, paths, globs, hive partitions |
| [`yggdryl-records`](yggdryl-records/SKILL.md) | record reads and writes, `RecordOptions`, Arrow IPC, Parquet, Avro, text, Iceberg, partitions |
| [`yggdryl-documents`](yggdryl-documents/SKILL.md) | JSON, JSON Lines, YAML, TOML and XML over `Scalar` |
| [`yggdryl-expressions`](yggdryl-expressions/SKILL.md) | terms, filters, selectors, plans, evaluation and pushdown |
| [`yggdryl-hashing`](yggdryl-hashing/SKILL.md) | xxHash digests, stable hashes, row digests, TxHash |
| [`yggdryl-fix`](yggdryl-fix/SKILL.md) | FIX decode and encode, the registry and store, Arrow rows, captures, lifecycle, `ygg fix` |
| [`yggdryl-market-data`](yggdryl-market-data/SKILL.md) | orders, quotes, executions, trades, order books, market data views, replay |

Each skill is a `SKILL.md` - the decision table, the rules, the pitfalls -
and `references/rust.md`, `references/python.md` and
`references/javascript.md` with runnable recipes, so an agent reads only the
language it writes.

## Kept true

Every `rust`, `python` and `javascript` block here runs in CI beside the
documentation's own, through `scripts/check_docs_examples.py`: a skill that
teaches a method the package no longer has fails the build. A change to a
public surface updates the skill that teaches it in the same change.
