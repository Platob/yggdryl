#!/usr/bin/env python3
"""S6d: move the object-store backend into `yggdryl-s3` at `rust/s3/`.

Usage:
    python3 -I s6_s3_move.py <tree> [--no-git-lock] [--residue <file>]

Run it on a clean tree - the program branch with D36's register in place
(P3), alone or in the S6 batch after the Avro and Parquet moves and before
the Iceberg one - and it leaves the tree uncommitted: `git status` shows the
moves as renames and every rewrite as a modification, for the lane manager's
compiler loop (`cargo check --workspace --all-targets --all-features
--keep-going`). It runs no cargo command. A second run on a tree it already
moved stops at step 0 and changes nothing.

The core does not build between this move and the Iceberg one: the core's
`s3tables/` (Iceberg's, D15) opens a table's store through
`yggdryl_s3::located_with`, which the core cannot depend on; the Iceberg move
takes `s3tables/` into `yggdryl-iceberg`, whose `s3tables` feature depends on
`yggdryl-s3` (D36.6).

What it does, in order (`--residue` gets every site it could not rewrite
mechanically, `file:line` each):

 1. test split - before the moves, every core test item that builds an
                 object store's handle - a `yggdryl::s3` path, a name
                 imported from it, `yggdryl::internals::s3_*`, a positive
                 `feature = "s3"` gate - moves into `rust/s3/tests/` at the
                 same relative path with the helpers it names (D39), the
                 harness written beside it; `FakeS3` stays under
                 `rust/tests/support/`, `#[path]`-included. What builds an
                 Iceberg table over the store stays for the Iceberg move:
                 `accounting::iceberg` of `rust/tests/s3/mod_.rs` is kept in
                 the core at that path with its fixtures (D36.7), and the
                 core's Iceberg suites and bench keep their object-store
                 modules, their `s3` gates re-keyed to `s3tables`.
 2. moves      - `git mv`: `rust/src/s3/` to `rust/s3/src/` (`mod.rs` the
                 crate root `lib.rs`), `rust/tests/s3{,.rs}` to
                 `rust/s3/tests/`, `rust/tests/interop/s3/` to
                 `rust/s3/tests/interop/s3/`, `rust/benchmarks/holder/s3/`
                 to `rust/s3/benchmarks/s3/`.
 3. crate      - `rust/s3/src/lib.rs`: the doc, `#![deny(unsafe_code)]`,
                 `install()` claiming `S3_BACKEND` through
                 `yggdryl::holder::claim_backend` as `yggdryl-s3`, idempotent
                 (D7); the session's crate-private doors and two associated
                 ones called through `yggdryl::implementer`, the two core
                 modules the client imported whole flattened to the names it
                 reads, links to what the core keeps private turned to code.
 4. core       - `pub mod s3` and the register's seed gone (no backend is the
                 core's own any more); the 23 `feature = "s3"` sites of
                 `aws/`, `auth/`, `http/` and `xml/` re-keyed to the feature
                 of the module that holds them, or deleted (D36.5); the 58
                 crate-private items the backend reaches published in
                 `yggdryl::implementer` (functions forwarded, types raised in
                 their private modules and re-exported, associated items
                 forwarded taking their receiver first); `s3tables/`
                 re-spelled onto `yggdryl_s3`; the logging facade's `CRATES`
                 gains `yggdryl_s3`; the `s3` feature, `md-5` and the S3
                 bench baseline (`object_store`, `futures`) leave the core.
 5. paths      - every `crate::` path of the moved sources by owner, every
                 `yggdryl::s3` path and `use` tree in tests, benches,
                 bindings, docs and skills, `yggdryl::internals::s3_*`, file
                 paths in every text file, `#[path]` re-anchored.
 6. siblings   - a leaf crate's test or bench reaching the backend (the
                 Parquet crate's, in the batch) stays, its `s3` gates
                 resolved, the crate a dev-dependency and installed, its
                 `s3` feature forward gone and its `[leaves]` line after `s3`.
 7. install    - every test of the crate opens with
                 `crate::install::installed()`, the bench `main` installs, a
                 rustdoc example and a page's Rust block reaching the backend
                 install, the bindings' init and the CLI's `main` install.
 8. manifests  - `rust/s3/Cargo.toml` and its README, the workspace members
                 and `[workspace.dependencies]`, the core's, the bindings',
                 the CLI's.
 9. tooling    - `generate_internals.py` and `check_api_inventory.py` per
                 crate where S4 has not made them so, the generator run.
10. CI         - the `s3` leaf line with its three exchanges, the `iceberg`
                 line after `s3`, `x-s3`/`x-azure`/`x-gcs` reading the crate,
                 the planner's leaf grammar and exchange tests, the exchange
                 lane's build step, the three drivers' cargo lines, the docs
                 runner's features.
11. docs       - the holder page's object-store and register sections, the
                 architecture, contributing, testing and benchmark pages, the
                 skills, AGENTS.md, the READMEs, `.api-inventory.txt`.
12. residue    - what is left for the compiler loop, by file and line.
"""

from __future__ import annotations

import argparse
import ast
import collections
import json
import os
import pathlib
import posixpath
import re
import subprocess
import sys

SCRATCH = pathlib.Path(
    os.environ.get(
        "S",
        "/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad",
    )
)
sys.path.insert(0, str(SCRATCH))
import s4_move as s4  # noqa: E402  (S4's lexer, use trees, rewriter, lock)

CORE, S3 = "core", "s3"
PACKAGE = "yggdryl-s3"
CRATE = "yggdryl_s3"
CRATE_DIR = "rust/s3"

RESIDUE: list[str] = []
DONE: list[str] = []


def residue(where: str, what: str) -> None:
    RESIDUE.append(f"- `{where}`: {what}")


def done(what: str) -> None:
    DONE.append(what)
    print(f"[s6-s3] {what}", flush=True)


# S4's helpers report into this script's residue; its rewriter learns the crate.
s4.residue = residue
s4.CRATE_NAME[S3] = CRATE
s4.PACKAGE[S3] = PACKAGE

read, write, git, tracked = s4.read, s4.write, s4.git, s4.tracked
code_only, top_items, inline_body = s4.code_only, s4.top_items, s4.inline_body
parse_use, render_use, UseParseError = s4.parse_use, s4.render_use, s4.UseParseError
line_of = s4.line_of
