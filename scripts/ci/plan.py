#!/usr/bin/env python3
"""Plan which CI jobs a change needs, and judge the run against that plan.

`.github/ci/rows.toml` says what every job of `.github/workflows/ci.yml`
reads; this reads it, the change and the ledger of rows already proven, and
answers the jobs to run. A push to `main` runs every job. A pull request runs
the rows its change touches that the ledger has not proven at the same
fingerprint, and the jobs those rows need - the core lanes, the wheel, the
addon - read from `needs:` in `ci.yml`; a path no row knows runs everything,
and a leaf crate's row runs what proves the leaf alone.

The same file then judges the run: `gate` fails unless every planned job
passed and every other job was skipped, which is what catches a planned job a
failure upstream skipped and a job an `if:` let run unplanned, and `record`
adds the rows this run proved to the ledger.

Usage:
    python3 scripts/ci/plan.py plan                       # in CI, from the event
    python3 scripts/ci/plan.py plan --base origin/main    # this branch as a pull request
    python3 scripts/ci/plan.py plan --paths node/src/a.rs # what a change of these paths runs
    python3 scripts/ci/plan.py targets yggdryl full rest  # one shard's cargo target arguments
    python3 scripts/ci/plan.py record                     # in the gate: NEEDS
    python3 scripts/ci/plan.py gate                       # in the gate: NEEDS

Standard library only, so the `changes` job and the gate need no install.
"""

from __future__ import annotations

import argparse
import dataclasses
import functools
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]
ROWS = ROOT / ".github" / "ci" / "rows.toml"
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
LEDGER = ROOT / ".ci-ledger" / "ledger.txt"
# The two jobs every plan has: the planner itself and the gate that judges it.
PLANNER = "changes"
GATE = "ci"
# A leaf's job: one matrix entry per lane and shard of every leaf that runs.
LEAVES_JOB = "leaves"
# The pseudo-row a row extends to read every leaf crate.
CRATES = "crates"
# Folders under `rust/` that are the core's own, never a leaf crate.
CORE_FOLDERS = frozenset({"src", "tests", "benchmarks", "examples", "target"})
# The members the workspace has besides its leaves.
MEMBERS = frozenset({"rust", "python", "node", "cli"})
FULL_LABEL = "ci:full"
# The core's two lanes, each a leaf's two lanes too: flags and the
# dependency cache the core lane of the same features saves.
LANES = {"default": ("", "stable-default"), "full": ("--all-features", "stable-full")}
# A cargo target as a shard names it, `kind:name`, and the flag cargo takes.
TARGET_FLAGS = {"test": "--test", "bench": "--bench"}


class PlanError(Exception):
    """The table, the workflow and the tree disagree; the message says where."""


@dataclasses.dataclass(frozen=True)
class Row:
    name: str
    extends: tuple[str, ...]
    paths: tuple[str, ...]
    jobs: tuple[str, ...]
    classifies: bool = True


@dataclasses.dataclass(frozen=True)
class Leaf:
    name: str
    package: str
    after: tuple[str, ...]
    msrv: str | None
    jobs: tuple[str, ...]


@dataclasses.dataclass
class Config:
    rows: dict[str, Row]
    inert: tuple[str, ...]
    leaves: dict[str, Leaf]
    leaf_jobs: tuple[str, ...]
    python_legs: tuple[str, ...]
    python_leaf_legs: tuple[str, ...]
    shards: dict[str, dict[str, dict[str, tuple[str, ...]]]]
    ledger_days: int
    ledger_lines: int

    def resolved(self, name: str) -> tuple[str, ...]:
        """Every pattern `name` reads: its own, its upstream rows', `workflow`'s."""
        seen: list[str] = []
        stack: list[str] = []

        def walk(row_name: str) -> None:
            if row_name in stack:
                raise PlanError(f"rows.toml: {' -> '.join(stack + [row_name])} is a cycle")
            if row_name == CRATES:
                for leaf in self.leaves:
                    walk(crate_row(leaf))
                return
            row = self.rows.get(row_name)
            if row is None:
                where = f"`{stack[-1]}` extends" if stack else "asked for"
                raise PlanError(f"rows.toml: {where} `{row_name}`, which is no row")
            stack.append(row_name)
            for upstream in row.extends:
                walk(upstream)
            stack.pop()
            for pattern in row.paths:
                if pattern not in seen:
                    seen.append(pattern)

        walk(name)
        if name != "workflow" and "workflow" in self.rows:
            for pattern in self.rows["workflow"].paths:
                if pattern not in seen:
                    seen.append(pattern)
        return tuple(seen)


def crate_row(leaf: str) -> str:
    return f"crate-{leaf}"


def load_config(path: pathlib.Path = ROWS) -> Config:
    with path.open("rb") as handle:
        data = tomllib.load(handle)
    rows: dict[str, Row] = {}
    for name, body in data.get("rows", {}).items():
        unknown = set(body) - {"extends", "paths", "jobs", "classifies"}
        if unknown:
            raise PlanError(f"rows.toml: row `{name}` has unknown keys {sorted(unknown)}")
        rows[name] = Row(
            name=name,
            extends=tuple(body.get("extends", ())),
            paths=tuple(body.get("paths", ())),
            jobs=tuple(body.get("jobs", ())),
            classifies=bool(body.get("classifies", True)),
        )
    leaves: dict[str, Leaf] = {}
    for name, body in data.get("leaves", {}).items():
        unknown = set(body) - {"package", "after", "msrv", "jobs"}
        if unknown or "package" not in body:
            raise PlanError(f"rows.toml: leaf `{name}` needs `package` and takes `after`, `msrv` and `jobs`")
        leaves[name] = Leaf(
            name, body["package"], tuple(body.get("after", ())), body.get("msrv"), tuple(body.get("jobs", ()))
        )
    leaf_jobs = tuple(data.get("leaf", {}).get("jobs", ()))
    for name, leaf in leaves.items():
        row = crate_row(name)
        if row in rows:
            raise PlanError(f"rows.toml: `{row}` is the leaf `{name}`'s row and cannot be written")
        for upstream in leaf.after:
            if upstream not in leaves:
                raise PlanError(f"rows.toml: leaf `{name}` is after `{upstream}`, which is no leaf")
        rows[row] = Row(
            name=row,
            extends=("corelib",) + tuple(crate_row(upstream) for upstream in leaf.after),
            paths=(f"rust/{name}/**", "config/**"),
            jobs=tuple(dict.fromkeys((LEAVES_JOB, *leaf_jobs, *leaf.jobs))),
        )
    shards: dict[str, dict[str, dict[str, tuple[str, ...]]]] = {}
    for package, lanes in data.get("shards", {}).items():
        shards[package] = {}
        for lane, named in lanes.items():
            if lane not in LANES:
                raise PlanError(f"rows.toml: shards of `{package}` name lane `{lane}`; lanes are {sorted(LANES)}")
            if "rest" in named:
                raise PlanError(f"rows.toml: `rest` is every target no shard of `{package}` names; it is not written")
            shards[package][lane] = {shard: tuple(targets) for shard, targets in named.items()}
    python = data.get("python", {})
    legs = tuple(python.get("legs", ()))
    leaf_legs = tuple(python.get("leaf", legs))
    if not legs or not set(leaf_legs) <= set(legs) or not leaf_legs:
        raise PlanError("rows.toml: `[python]` names its `legs` and a `leaf` subset of them")
    ledger = data.get("ledger", {})
    return Config(
        rows=rows,
        inert=tuple(data.get("inert", ())),
        leaves=leaves,
        leaf_jobs=leaf_jobs,
        python_legs=legs,
        python_leaf_legs=leaf_legs,
        shards=shards,
        ledger_days=int(ledger.get("days", 7)),
        ledger_lines=int(ledger.get("lines", 4000)),
    )


# -- the workflow, read for what the plan must agree with --------------------

JOB_HEADER = re.compile(r"^  ([A-Za-z0-9_-]+):\s*(?:#.*)?$")
NEEDS_LINE = re.compile(r"^    needs:\s*(.*?)\s*(?:#.*)?$")


@dataclasses.dataclass(frozen=True)
class Job:
    name: str
    needs: tuple[str, ...]
    body: str


def load_workflow(path: pathlib.Path = WORKFLOW) -> dict[str, Job]:
    """The jobs of `ci.yml` and their `needs:`, read line by line.

    The standard library reads no YAML, and the planner runs before anything
    is installed, so `ci.yml` writes every `needs:` as one name or a flow
    list - `[a, b]`, which may run over several lines - and this reads
    exactly that.
    """
    jobs: dict[str, tuple[list[str], list[str]]] = {}
    current: str | None = None
    inside = False
    lines = path.read_text(encoding="utf-8").splitlines()
    # A flow list continued over lines is joined onto its `needs:` line.
    joined: list[tuple[int, str]] = []
    pending: tuple[int, str] | None = None
    for number, line in enumerate(lines, start=1):
        if pending is not None:
            pending = (pending[0], pending[1] + " " + line.strip())
            if "]" in line:
                joined.append(pending)
                pending = None
            continue
        if NEEDS_LINE.match(line) and "[" in line and "]" not in line:
            pending = (number, line)
            continue
        joined.append((number, line))
    if pending is not None:
        raise PlanError(f"ci.yml:{pending[0]}: a `needs:` list is never closed")
    for number, line in joined:
        if line.startswith("jobs:"):
            inside = True
            continue
        if not inside:
            continue
        if line and not line[0].isspace() and not line.startswith("#"):
            inside = False
            current = None
            continue
        header = JOB_HEADER.match(line)
        if header:
            current = header.group(1)
            jobs[current] = ([], [])
            continue
        if current is None:
            continue
        jobs[current][1].append(line)
        needs = NEEDS_LINE.match(line)
        if needs:
            value = needs.group(1)
            if value.startswith("[") and value.endswith("]"):
                names = [name.strip() for name in value[1:-1].split(",") if name.strip()]
            elif re.fullmatch(r"[A-Za-z0-9_-]+", value):
                names = [value]
            else:
                raise PlanError(f"ci.yml:{number}: `{current}` writes `needs:` as a block; write it inline")
            jobs[current][0].extend(names)
    return {name: Job(name, tuple(needs), "\n".join(body)) for name, (needs, body) in jobs.items()}


def producers(job: str, workflow: dict[str, Job]) -> tuple[str, ...]:
    """The jobs `job` needs: every one but the planner is a job whose work it uses."""
    return tuple(need for need in workflow[job].needs if need != PLANNER)


def closure(jobs: set[str], workflow: dict[str, Job]) -> set[str]:
    planned = set(jobs)
    pending = list(jobs)
    while pending:
        job = pending.pop()
        for need in producers(job, workflow):
            if need not in planned:
                planned.add(need)
                pending.append(need)
    return planned


def check(config: Config, workflow: dict[str, Job], root: pathlib.Path = ROOT) -> list[str]:
    """Every way the table, `ci.yml` and the tree can disagree, each a line."""
    problems: list[str] = []
    for name in config.rows:
        try:
            config.resolved(name)
        except PlanError as error:
            problems.append(str(error))
    for pattern in [p for row in config.rows.values() for p in row.paths] + list(config.inert):
        try:
            glob_regex(pattern)
        except PlanError as error:
            problems.append(str(error))

    for job in (PLANNER, GATE):
        if job not in workflow:
            problems.append(f"ci.yml has no `{job}` job")
    others = sorted(set(workflow) - {PLANNER, GATE})
    for row in config.rows.values():
        for job in row.jobs:
            if job not in workflow:
                problems.append(f"rows.toml: row `{row.name}` runs `{job}`, which ci.yml has no job of")
    for job in workflow.values():
        for need in job.needs:
            if need not in workflow:
                problems.append(f"ci.yml: `{job.name}` needs `{need}`, which is no job")
    if problems:
        return problems

    proven = closure({job for row in config.rows.values() for job in row.jobs}, workflow)
    for job in others:
        if job not in proven and job != LEAVES_JOB:
            problems.append(f"ci.yml: no row of rows.toml runs `{job}`, and no job a row runs needs it")
    gate_needs = set(workflow[GATE].needs)
    if gate_needs != set(others) | {PLANNER}:
        missing = sorted((set(others) | {PLANNER}) - gate_needs)
        extra = sorted(gate_needs - set(others) - {PLANNER})
        problems.append(f"ci.yml: `{GATE}` must need every job; missing {missing}, unknown {extra}")
    for job in others:
        if PLANNER not in workflow[job].needs:
            problems.append(f"ci.yml: `{job}` does not need `{PLANNER}`")
        if f"' {job} '" not in workflow[job].body:
            problems.append(f"ci.yml: `{job}`'s `if:` does not ask the plan for `' {job} '`")

    # A job needs only what it reuses, and the plan carries every need of a
    # planned job, so no `if:` asks for a need's result: a need that failed or
    # was cancelled skips the job, and the gate names it as planned and skipped.
    for job in others:
        if re.search(r"needs\.[A-Za-z0-9_-]+\.result", workflow[job].body) or "always()" in workflow[job].body:
            problems.append(f"ci.yml: `{job}` asks for a need's result; a planned job's needs are planned")
    for job in config.leaf_jobs:
        if job not in workflow:
            problems.append(f"rows.toml: `[leaf]` runs `{job}`, which ci.yml has no job of")

    problems.extend(check_leaves(config, root))
    return problems


def check_leaves(config: Config, root: pathlib.Path) -> list[str]:
    problems = []
    rust = root / "rust"
    crates = sorted(
        entry.name
        for entry in rust.iterdir()
        if entry.is_dir() and entry.name not in CORE_FOLDERS and (entry / "Cargo.toml").exists()
    ) if rust.is_dir() else []
    for name in crates:
        if name not in config.leaves:
            problems.append(f"rust/{name}/Cargo.toml has no line under [leaves] in rows.toml")
    for name in config.leaves:
        if name not in crates:
            problems.append(f"rows.toml: leaf `{name}` has no rust/{name}/Cargo.toml")
    manifest = root / "Cargo.toml"
    if manifest.exists():
        with manifest.open("rb") as handle:
            members = tomllib.load(handle).get("workspace", {}).get("members", [])
        for member in members:
            leaf = member.removeprefix("rust/") if member.startswith("rust/") else None
            if member not in MEMBERS and (leaf is None or leaf not in config.leaves):
                problems.append(f"Cargo.toml: member `{member}` is neither a binding nor a leaf rows.toml lists")
    return problems


# -- paths ---------------------------------------------------------------------


@functools.lru_cache(maxsize=None)
def glob_regex(pattern: str) -> re.Pattern[str]:
    """`pattern` as git's glob magic reads it: `*` in one segment, `**/` across many."""
    if any(character in pattern for character in "[]\\"):
        raise PlanError(f"rows.toml: `{pattern}` uses a character class or an escape; spell it plainly")
    if pattern.startswith("/") or pattern.endswith("/"):
        raise PlanError(f"rows.toml: `{pattern}` is written from the root with no leading or trailing `/`")
    out = []
    index = 0
    while index < len(pattern):
        if pattern.startswith("**/", index) and (index == 0 or pattern[index - 1] == "/"):
            out.append("(?:.*/)?")
            index += 3
        elif pattern.startswith("/**", index) and index + 3 == len(pattern):
            out.append("/.*")
            index += 3
        elif pattern.startswith("**", index):
            raise PlanError(f"rows.toml: `{pattern}` uses `**` other than as `**/` or a trailing `/**`")
        elif pattern[index] == "*":
            out.append("[^/]*")
            index += 1
        elif pattern[index] == "?":
            out.append("[^/]")
            index += 1
        else:
            out.append(re.escape(pattern[index]))
            index += 1
    return re.compile("".join(out) + r"\Z")


def matches(path: str, patterns: tuple[str, ...]) -> bool:
    return any(glob_regex(pattern).match(path) for pattern in patterns)


def unclassified(paths: list[str], config: Config) -> list[str]:
    """The paths no row claims: a row claims its own paths, where it `classifies`."""
    known = tuple(
        pattern for row in config.rows.values() if row.classifies for pattern in row.paths
    ) + config.inert
    return [path for path in paths if not matches(path, known)]


def git(*arguments: str) -> bytes:
    return subprocess.run(["git", *arguments], cwd=ROOT, check=True, capture_output=True).stdout


def changed_paths(base: str | None) -> list[str]:
    """The pull request's change: its merge commit against the base it merged onto."""
    revisions = [f"{base}...HEAD"] if base else ["HEAD^1", "HEAD"]
    output = git("diff", "--name-only", "--no-renames", "-z", *revisions)
    return [path for path in output.decode().split("\0") if path]


def fingerprint(name: str, patterns: tuple[str, ...], salt: str) -> str:
    """The row's inputs: every blob its patterns name, the patterns, the toolchain, the image."""
    digest = hashlib.sha256()
    digest.update(f"v1\0{salt}\0{name}\0".encode())
    digest.update("\0".join(patterns).encode())
    digest.update(git("ls-files", "-s", "-z", "--", *(f":(glob){pattern}" for pattern in patterns)))
    return digest.hexdigest()[:20]


# -- the ledger ------------------------------------------------------------------


def today() -> int:
    return int(time.time() // 86400)


def read_ledger(path: pathlib.Path = LEDGER) -> list[tuple[str, str, int]]:
    if not path.exists():
        return []
    entries = []
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[2].isdigit():
            entries.append((parts[0], parts[1], int(parts[2])))
    return entries


def write_ledger(entries: list[tuple[str, str, int]], keep: int, path: pathlib.Path = LEDGER) -> None:
    newest: dict[tuple[str, str], int] = {}
    for row, fp, day in entries:
        newest[(row, fp)] = max(day, newest.get((row, fp), day))
    kept = sorted(newest.items(), key=lambda item: (-item[1], item[0]))[:keep]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(f"{row} {fp} {day}\n" for (row, fp), day in kept), encoding="utf-8")


# -- the plan --------------------------------------------------------------------


@dataclasses.dataclass
class Plan:
    full: str | None
    rows: dict[str, str]
    jobs: set[str]
    leaves: list[dict[str, str]]
    proof: dict[str, dict[str, object]]
    unclassified: list[str]
    skipped_by_ledger: list[str]
    python_legs: list[str]

    def outputs(self, config: Config) -> dict[str, str]:
        core = config.shards.get("yggdryl", {})
        return {
            "jobs": " " + " ".join(sorted(self.jobs)) + " ",
            "core_shards": json.dumps({lane: [*core.get(lane, {}), "rest"] for lane in LANES}),
            "leaves": json.dumps(self.leaves),
            "python_legs": json.dumps(self.python_legs),
            "proof": json.dumps(self.proof, sort_keys=True),
        }


def msrv_toolchain(root: pathlib.Path = ROOT) -> str:
    with (root / "Cargo.toml").open("rb") as handle:
        version = tomllib.load(handle)["workspace"]["package"]["rust-version"]
    return version if version.count(".") == 2 else f"{version}.0"


def leaf_entries(config: Config, leaves: list[str], root: pathlib.Path = ROOT) -> list[dict[str, str]]:
    entries = []
    for name in leaves:
        leaf = config.leaves[name]
        # One dependency cache per leaf, saved on `main` by its full lane's
        # `rest` shard and restored by both lanes: the full lane compiles a
        # superset of what the default lane links, and one key per leaf keeps
        # seven leaves inside the repository's cache budget.
        for lane, (flags, _) in LANES.items():
            named = config.shards.get(leaf.package, {}).get(lane, {})
            for shard in [*named, "rest"]:
                entries.append({
                    "name": name, "package": leaf.package, "lane": lane, "shard": shard,
                    "flags": flags, "cache": f"leaf-{name}", "toolchain": "",
                    "save": "true" if (lane, shard) == ("full", "rest") else "false",
                })
        if leaf.msrv is not None:
            entries.append({
                "name": name, "package": leaf.package, "lane": "msrv", "shard": "rest",
                "flags": leaf.msrv, "cache": "", "toolchain": msrv_toolchain(root), "save": "false",
            })
    return entries


def make_plan(
    config: Config,
    workflow: dict[str, Job],
    *,
    pull_request: bool,
    changed: list[str],
    labels: list[str],
    salt: str,
    ledger: list[tuple[str, str, int]],
    day: int,
    fingerprints: bool = True,
) -> Plan:
    full = None
    stray: list[str] = []
    if not pull_request:
        full = "not a pull request"
    elif FULL_LABEL in labels:
        full = f"the `{FULL_LABEL}` label"
    else:
        stray = unclassified(changed, config)
        if stray:
            full = f"{len(stray)} path(s) no row knows"

    proven = {(row, fp) for row, fp, when in ledger if day - when < config.ledger_days}
    running: dict[str, str] = {}
    from_ledger: list[str] = []
    for name, row in config.rows.items():
        if not row.jobs:
            continue
        patterns = config.resolved(name)
        if full is None and not any(matches(path, patterns) for path in changed):
            continue
        fp = fingerprint(name, patterns, salt) if pull_request and fingerprints else ""
        if full is None and (name, fp) in proven:
            from_ledger.append(name)
            continue
        running[name] = fp

    jobs = closure({job for name in running for job in config.rows[name].jobs}, workflow)
    leaves = sorted(name for name in config.leaves if crate_row(name) in running)
    # Every leg where a row of the `python` job's own runs it; where only a
    # leaf crate's row does, the leaf's legs.
    leaf_rows = {crate_row(name) for name in config.leaves}
    all_legs = any(
        "python" in closure(set(config.rows[name].jobs), workflow) for name in running if name not in leaf_rows
    )
    legs = list(config.python_legs if all_legs else config.python_leaf_legs)
    proof = {}
    if pull_request and fingerprints:
        proof = {
            name: {"fp": fp, "jobs": sorted(closure(set(config.rows[name].jobs), workflow))}
            for name, fp in running.items()
        }
    return Plan(full, running, jobs, leaf_entries(config, leaves), proof, stray, sorted(from_ledger), legs)


def summarize(plan: Plan, changed: list[str]) -> str:
    lines = ["## CI plan", ""]
    if plan.full:
        lines.append(f"Every job runs: {plan.full}.")
    else:
        lines.append(f"{len(changed)} changed path(s); {len(plan.rows)} row(s) run.")
    for path in plan.unclassified[:20]:
        lines.append(f"- unclassified: `{path}`")
    if plan.skipped_by_ledger:
        lines.append("")
        lines.append("Proven at this fingerprint by an earlier green run: " + ", ".join(f"`{r}`" for r in plan.skipped_by_ledger))
    lines.append("")
    lines.append("| job | runs |")
    lines.append("| --- | --- |")
    return "\n".join(lines) + "\n"


def write_outputs(values: dict[str, str]) -> None:
    target = os.environ.get("GITHUB_OUTPUT")
    if not target:
        for key, value in values.items():
            print(f"{key}={value}")
        return
    with open(target, "a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def command_plan(arguments: argparse.Namespace) -> int:
    config = load_config()
    workflow = load_workflow()
    problems = check(config, workflow)
    if problems:
        for problem in problems:
            print(f"::error::{problem}")
        return 1
    event = os.environ.get("GITHUB_EVENT_NAME", "")
    pull_request = event == "pull_request" or arguments.base is not None or arguments.paths is not None
    labels = json.loads(os.environ.get("LABELS") or "null") or []
    if arguments.paths is not None:
        changed = list(arguments.paths)
    elif pull_request:
        changed = changed_paths(arguments.base)
    else:
        changed = []
    salt = "\0".join([
        os.environ.get("TOOLCHAIN", ""),
        os.environ.get("ImageOS", ""),
        os.environ.get("ImageVersion", ""),
    ])
    plan = make_plan(
        config, workflow,
        pull_request=pull_request, changed=changed, labels=labels, salt=salt,
        ledger=read_ledger(), day=today(),
    )
    write_outputs(plan.outputs(config))
    every = sorted(set(workflow) - {PLANNER, GATE})
    table = summarize(plan, changed) + "".join(
        f"| `{job}` | {'yes' if job in plan.jobs else '-'} |\n" for job in every
    )
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(table)
    print(table)
    return 0


# -- shards ----------------------------------------------------------------------


def package_targets(package: str) -> list[dict[str, object]]:
    output = subprocess.run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT, check=True, capture_output=True,
    ).stdout
    for entry in json.loads(output)["packages"]:
        if entry["name"] == package:
            return entry["targets"]
    raise PlanError(f"the workspace has no package `{package}`")


def shard_arguments(config: Config, package: str, lane: str, shard: str, targets: list[dict[str, object]]) -> list[str]:
    """The cargo target flags of one shard: a named one's targets, or `rest` - every other."""
    named = config.shards.get(package, {}).get(lane, {})
    present = {(kind, str(target["name"])) for target in targets for kind in target["kind"]}
    claimed: dict[tuple[str, str], str] = {}
    for name, spelled in named.items():
        for entry in spelled:
            kind, _, target = entry.partition(":")
            if kind not in TARGET_FLAGS or not target:
                raise PlanError(f"rows.toml: shard `{name}` of `{package}` names `{entry}`; write `test:<name>` or `bench:<name>`")
            if (kind, target) not in present:
                raise PlanError(f"rows.toml: shard `{name}` of `{package}` names `{entry}`, which `{package}` has no target of")
            if (kind, target) in claimed:
                raise PlanError(f"rows.toml: `{entry}` is in shards `{claimed[(kind, target)]}` and `{name}` of `{package}`")
            claimed[(kind, target)] = name
    if shard != "rest":
        if shard not in named:
            raise PlanError(f"rows.toml: `{package}` has no `{lane}` shard `{shard}`")
        flags = []
        for entry in named[shard]:
            kind, _, target = entry.partition(":")
            flags += [TARGET_FLAGS[kind], target]
        return flags
    # `--all-targets` is `--lib --bins --tests --benches --examples`; the
    # plural flags keep cargo's own reading of the targets no shard takes.
    kinds = {kind for target in targets for kind in target["kind"]}
    flags = []
    if kinds & {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}:
        flags.append("--lib")
    if "bin" in kinds:
        flags.append("--bins")
    if "example" in kinds:
        flags.append("--examples")
    for target in sorted(targets, key=lambda target: str(target["name"])):
        for kind in target["kind"]:
            if kind == "test" and target.get("test", True) and (kind, target["name"]) not in claimed:
                flags += ["--test", str(target["name"])]
            if kind == "bench" and (kind, target["name"]) not in claimed:
                flags += ["--bench", str(target["name"])]
    return flags


def command_targets(arguments: argparse.Namespace) -> int:
    config = load_config()
    flags = shard_arguments(config, arguments.package, arguments.lane, arguments.shard, package_targets(arguments.package))
    if not flags:
        raise PlanError(f"`{arguments.package}` {arguments.lane} shard `{arguments.shard}` selects no target")
    print("\n".join(flags))
    return 0


# -- the gate --------------------------------------------------------------------


def needs_from_environment() -> dict[str, dict[str, object]]:
    return json.loads(os.environ.get("NEEDS") or "{}")


def verdict(needs: dict[str, dict[str, object]]) -> list[str]:
    """Each job whose result is not the one the plan expects, as a line."""
    planner = needs.get(PLANNER, {})
    if planner.get("result") != "success":
        return [f"{PLANNER}: {planner.get('result', 'absent')}, expected success - nothing was planned"]
    planned = set(str(planner.get("outputs", {}).get("jobs", "")).split())
    wrong = []
    for job, state in sorted(needs.items()):
        if job == PLANNER:
            continue
        expected = "success" if job in planned else "skipped"
        if state.get("result") != expected:
            wrong.append(f"{job}: {state.get('result')}, expected {expected}")
    for job in sorted(planned - set(needs)):
        wrong.append(f"{job}: planned, but the gate does not need it")
    return wrong


def command_gate(_: argparse.Namespace) -> int:
    needs = needs_from_environment()
    wrong = verdict(needs)
    rows = "".join(f"| `{job}` | {state.get('result')} |\n" for job, state in sorted(needs.items()))
    report = "## CI result\n\n| job | result |\n| --- | --- |\n" + rows
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(report + "".join(f"\n- {line}" for line in wrong) + "\n")
    print(report)
    for line in wrong:
        print(f"::error::{line}")
    return 1 if wrong else 0


def command_record(_: argparse.Namespace) -> int:
    needs = needs_from_environment()
    planner = needs.get(PLANNER, {})
    proof = json.loads(str(planner.get("outputs", {}).get("proof") or "{}"))
    day = today()
    entries = read_ledger()
    added = []
    for row, info in sorted(proof.items()):
        if all(needs.get(job, {}).get("result") == "success" for job in info["jobs"]):
            entries.append((row, str(info["fp"]), day))
            added.append(row)
    if added:
        write_ledger(entries, load_config().ledger_lines)
    write_outputs({"added": str(len(added))})
    print(f"proved {len(added)} row(s): {', '.join(added) or 'none'}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    plan = commands.add_parser("plan", help="the jobs this change runs")
    plan.add_argument("--base", help="plan this branch as a pull request onto BASE")
    plan.add_argument("--paths", nargs="*", help="plan a change of exactly these paths")
    plan.set_defaults(run=command_plan)
    targets = commands.add_parser("targets", help="one shard's cargo target arguments, one per line")
    targets.add_argument("package")
    targets.add_argument("lane", choices=sorted(LANES))
    targets.add_argument("shard")
    targets.set_defaults(run=command_targets)
    commands.add_parser("gate", help="judge the run against the plan").set_defaults(run=command_gate)
    commands.add_parser("record", help="add the rows this run proved to the ledger").set_defaults(run=command_record)
    arguments = parser.parse_args()
    try:
        return arguments.run(arguments)
    except PlanError as error:
        print(f"::error::{error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
