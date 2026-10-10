#!/usr/bin/env python3
"""P5R: re-target the parked P5 patch (D37, bf5921bf4 over P4 4e46b5ab7) onto
the tree after S4 and P9, rewriting file paths only - never a hunk's text.

Run from the repository root, read-only on the tree (`git ls-tree`/`git show`
of HEAD and plain reads of the working tree):

    python3 -I .handoff/split/scratch/p5r/path_map.py [--repo .] [--out DIR]

Writes, under DIR (default: this script's folder):

- `p5_on_s4p9.patch` - every section whose file maps to ONE destination, its
  `index` line kept, so `git apply -3` merges it from P4's blob (present in
  the object store: 4e46b5ab7 is in history);
- `p5_on_s4p9.routed.patch` - the sections of a file S4 split across crates
  (the core harness, `lib.rs`, `implementer.rs`, `graph/mod.rs`, the graph
  tests the market and FIX crates both hold, `allocations.rs`): each hunk
  routed to the one candidate whose text holds its old side, the `index` line
  dropped (P4's whole-file blob is no base for a third of it), for
  `git apply --reject`;
- `p5_on_s4p9.unrouted.patch` - the hunks no candidate holds, by hand;
- `path_map_report.md` - every path mapped, dropped, routed or unrouted, each
  destination checked against `git ls-tree` of HEAD and the working tree, the
  hunks whose old side the destination no longer holds verbatim, and the old
  names the added lines still spell.

With `--preview TMP` it also merges each mapped section as `git apply -3`
would - P4's blob as the base, the parked section applied to it as theirs,
the destination as it stands (HEAD or P9's working tree) as ours - by
`git merge-file -p` in TMP (outside the repository), and reports each
file's conflict count: the conflicts to expect before P7 and P8 add theirs.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
PATCH = HERE.parent / "p5_on_p4.patch"

# The core keeps the event vocabulary (S4): these graph files never move.
CORE_GRAPH = {"element.rs", "column.rs", "element_column.rs"}

# What S4 split across crates, by hand or by item: every candidate a hunk
# may land in, the core's own first.
SPLIT = {
    "rust/src/lib.rs": ["rust/src/lib.rs", "rust/market/src/lib.rs", "rust/fix/src/lib.rs"],
    "rust/src/implementer.rs": ["rust/src/implementer.rs", "rust/market/src/implementer.rs"],
    "rust/src/graph/mod.rs": ["rust/src/graph/mod.rs", "rust/market/src/graph/mod.rs"],
    "rust/tests/graph.rs": [
        "rust/tests/graph.rs",
        "rust/market/tests/graph.rs",
        "rust/fix/tests/graph.rs",
    ],
    "rust/tests/allocations.rs": [
        "rust/tests/allocations.rs",
        "rust/market/tests/allocations.rs",
        "rust/fix/tests/allocations.rs",
    ],
}

# P9 (D42.8): the registry's files under the Instrument's names.
P9 = [
    ("rust/market/src/isin_registry.rs", "rust/market/src/instrument.rs"),
    ("rust/market/src/isin_registry/", "rust/market/src/instrument/"),
    ("rust/market/tests/root/isin_registry.rs", "rust/market/tests/root/instrument.rs"),
    ("rust/market/tests/isin_registry.rs", "rust/market/tests/instrument.rs"),
    ("rust/market/tests/isin_registry/", "rust/market/tests/instrument/"),
    ("rust/market/benchmarks/graph/isin_registry.rs", "rust/market/benchmarks/graph/instrument.rs"),
    ("rust/fix/tests/isin_registry.rs", "rust/fix/tests/instrument.rs"),
    ("rust/fix/tests/isin_registry/", "rust/fix/tests/instrument/"),
    ("python/src/isin_registry.rs", "python/src/instrument.rs"),
    ("python/yggdryl/isin_registry.py", "python/yggdryl/instrument.py"),
    ("python/tests/test_isin_registry.py", "python/tests/test_instrument.py"),
    ("node/src/isin_registry.rs", "node/src/instrument.rs"),
    ("node/tests/isin_registry.test.js", "node/tests/instrument.test.js"),
    ("node/tests/isin_registry.types.ts", "node/tests/instrument.types.ts"),
    ("docs/graph/isin-registry.md", "docs/graph/instrument.md"),
    ("config/isin/instruments.json", "config/instruments/instruments.json"),
    ("scripts/check_isin_seed.py", "scripts/check_instruments_seed.py"),
]

# Spellings the added lines must not carry onto the target tree: P7's
# renames (D40.4; `detailedcficode` and the other dictionary words excluded
# by the leading-letter rule), P9's (D42.8), D38's and the deleted surfaces.
OLD_NAMES = [
    ("P7", r"(?<![A-Za-z0-9])_?(?:get_|set_)?(isincode|cficode|miccode|bloombergcode|figicode|forexcode|eusipacode|countrycode)\b"),
    ("P7", r"\b(ISINCODE|MICCODE|FIGICODE|FOREXCODE|BLOOMBERGCODE)_TAG_NAME\b"),
    ("P7", r"\bMarketColumn::(IsinCode|CfiCode|MicCode)\b"),
    ("P7", r"\b(instuuid)\b"),
    ("P9", r"\b(IsinRegistry|IsinEntry|IsinTable|isin_registry|with_isin_registry|isinRegistry|YGGDRYL_ISIN_REGISTRY_URI)\b"),
    ("D38", r"\b(currunix|recdunix|curruuid|currhashcode)\b"),
    ("deleted", r"\b(FixLifted|BookService)\b"),
    ("deleted", r"(MarketData::Fix)\b"),
    ("debug", r"(\"SCRATCH wire)"),
]


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True, capture_output=True, text=True
    ).stdout


def single(path: str) -> str | None:
    """The one destination of `path` after S4, or None to drop it."""
    if path.startswith(".handoff/"):
        return None  # this lane writes its own DESIGN.md record
    if path.startswith("rust/examples/"):
        return None  # S4 deleted rust/examples/
    m = re.fullmatch(r"rust/src/graph/(.+)", path)
    if m:
        return path if m.group(1) in CORE_GRAPH else f"rust/market/src/graph/{m.group(1)}"
    m = re.fullmatch(r"rust/src/fix/(.+)", path)
    if m:
        return "rust/fix/src/lib.rs" if m.group(1) == "mod.rs" else f"rust/fix/src/{m.group(1)}"
    if path == "rust/src/isin_registry.rs":
        return "rust/market/src/isin_registry.rs"
    m = re.fullmatch(r"rust/src/isin_registry/(.+)", path)
    if m:
        return f"rust/market/src/isin_registry/{m.group(1)}"
    if path == "rust/tests/fix.rs":
        return "rust/fix/tests/root.rs"
    m = re.fullmatch(r"rust/tests/fix/(.+)", path)
    if m:
        return "rust/fix/tests/root/lib.rs" if m.group(1) == "mod_.rs" else f"rust/fix/tests/root/{m.group(1)}"
    m = re.fullmatch(r"rust/tests/isin_registry(\.rs|/.+)", path)
    if m:
        return f"rust/market/tests/isin_registry{m.group(1)}"
    m = re.fullmatch(r"rust/tests/graph/(.+)", path)
    if m:
        return path if m.group(1) in CORE_GRAPH else f"rust/market/tests/graph/{m.group(1)}"
    if path == "rust/benchmarks/graph.rs":
        return "rust/market/benchmarks/graph.rs"
    m = re.fullmatch(r"rust/benchmarks/graph/(.+)", path)
    if m:
        return f"rust/market/benchmarks/graph/{m.group(1)}"
    if path == "rust/benchmarks/fix.rs":
        return "rust/fix/benchmarks/fix.rs"
    m = re.fullmatch(r"rust/benchmarks/fix/(.+)", path)
    if m:
        return f"rust/fix/benchmarks/fix/{m.group(1)}"
    return path


def after_p9(path: str) -> str:
    for old, new in P9:
        if path == old or (old.endswith("/") and path.startswith(old)):
            return new + path[len(old):]
    return path


class Tree:
    """HEAD by `git ls-tree`, the working tree by plain reads (P9 in flight)."""

    def __init__(self, repo: Path):
        self.repo = repo
        self.head = set(git(repo, "ls-tree", "-r", "--name-only", "HEAD").splitlines())
        self.cache: dict[str, list[str] | None] = {}

    def where(self, path: str) -> str:
        at_head = path in self.head
        in_work = (self.repo / path).is_file()
        if at_head and in_work:
            return "HEAD"
        if in_work:
            return "P9 (working tree, not at HEAD)"
        if at_head:
            return "HEAD, gone from the working tree (P9 moves it)"
        return "missing"

    def lines(self, path: str) -> list[str] | None:
        if path not in self.cache:
            file = self.repo / path
            if file.is_file():
                text = file.read_text(encoding="utf-8", errors="replace")
            elif path in self.head:
                text = git(self.repo, "show", f"HEAD:{path}")
            else:
                text = None
            self.cache[path] = None if text is None else text.split("\n")
        return self.cache[path]


# S4 re-spelled every path into its crate; matching reads past that.
PREFIX = re.compile(
    r"\b(?:yggdryl_market::implementer::|yggdryl_fix::implementer::|yggdryl::implementer::"
    r"|yggdryl_market::|yggdryl_fix::|yggdryl::|crate::|super::)"
)


def norm(line: str) -> str:
    return " ".join(PREFIX.sub("", line).split())


def old_side(hunk: list[str]) -> list[str]:
    return [l[1:] for l in hunk[1:] if l[:1] in (" ", "-")]


def held_verbatim(hunk: list[str], lines: list[str] | None) -> bool:
    """Whether the hunk's old side stands contiguous in the target as is."""
    old = old_side(hunk)
    if lines is None:
        return not old
    if not old:
        return True
    first, n = old[0], len(old)
    return any(
        lines[i] == first and lines[i : i + n] == old for i in range(len(lines) - n + 1)
    )


def score(hunk: list[str], lines: list[str] | None) -> float:
    if lines is None:
        return 0.0
    have = {norm(l) for l in lines}
    old = [norm(l) for l in old_side(hunk)]
    old = [l for l in old if l and l not in ("{", "}", "};", ");", "//!", "///", "//")]
    if not old:
        return 0.0
    # The removed lines weigh double: they must be there to be removed.
    removed = {norm(l[1:]) for l in hunk[1:] if l.startswith("-")}
    total = sum(2 if l in removed else 1 for l in old)
    found = sum((2 if l in removed else 1) for l in old if l in have)
    return found / total


P4 = "4e46b5ab7"


def preview(repo: Path, tmp: Path, old: str, header: list[str], hunks: list[list[str]], ours: list[str] | None) -> int | str:
    """The conflict count of P4 -> section merged into `ours`, or why none."""
    if ours is None:
        return "destination missing"
    work = tmp / "preview"
    if work.exists():
        for f in sorted(work.rglob("*"), reverse=True):
            f.unlink() if f.is_file() else f.rmdir()
    (work / Path(old).parent).mkdir(parents=True, exist_ok=True)
    base = git(repo, "show", f"{P4}:{old}")
    (work / old).write_text(base, encoding="utf-8")
    (work / "base").write_text(base, encoding="utf-8")
    (work / "ours").write_text("\n".join(ours), encoding="utf-8")
    (work / "section.patch").write_text("\n".join(header + [l for h in hunks for l in h]) + "\n", encoding="utf-8")
    applied = subprocess.run(["git", "apply", "section.patch"], cwd=work, capture_output=True, text=True)
    if applied.returncode:
        return "section does not apply to P4's blob"
    merged = subprocess.run(
        ["git", "merge-file", "-p", "ours", "base", old], cwd=work, capture_output=True, text=True
    )
    if merged.returncode < 0:
        return f"merge-file failed: {merged.stderr.strip()[:80]}"
    return sum(1 for l in merged.stdout.split("\n") if l.startswith("<<<<<<< "))


def parse(text: str):
    """Sections: (old path, new path, header lines, hunks)."""
    out = []
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    i = 0
    while i < len(lines):
        if not lines[i].startswith("diff --git "):
            raise SystemExit(f"unexpected line {i + 1}: {lines[i][:80]!r}")
        m = re.fullmatch(r"diff --git a/(\S+) b/(\S+)", lines[i])
        if not m:
            raise SystemExit(f"unparsed header at {i + 1}: {lines[i]!r}")
        header, hunks = [lines[i]], []
        i += 1
        while i < len(lines) and not lines[i].startswith(("@@ ", "diff --git ")):
            header.append(lines[i])
            i += 1
        while i < len(lines) and lines[i].startswith("@@ "):
            hunk = [lines[i]]
            i += 1
            while i < len(lines) and not lines[i].startswith(("@@ ", "diff --git ")):
                hunk.append(lines[i])
                i += 1
            hunks.append(hunk)
        out.append((m.group(1), m.group(2), header, hunks))
    return out


def retarget(header: list[str], old: str, new: str, dest: str, keep_index: bool) -> list[str]:
    """The header with `old`/`new` written as `dest`; one anchor each, asserted."""
    out = []
    for line in header:
        if line.startswith("diff --git "):
            assert line == f"diff --git a/{old} b/{new}", line
            out.append(f"diff --git a/{dest} b/{dest}")
        elif line.startswith("--- "):
            assert line in (f"--- a/{old}", "--- /dev/null"), line
            out.append(line if line == "--- /dev/null" else f"--- a/{dest}")
        elif line.startswith("+++ "):
            assert line in (f"+++ b/{new}", "+++ /dev/null"), line
            out.append(line if line == "+++ /dev/null" else f"+++ b/{dest}")
        elif line.startswith("index ") and not keep_index:
            continue
        else:
            out.append(line)
    return out


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default=".")
    ap.add_argument("--patch", default=str(PATCH))
    ap.add_argument("--out", default=str(HERE))
    ap.add_argument("--preview", help="a scratch folder outside the repository")
    args = ap.parse_args(argv)
    repo, out = Path(args.repo).resolve(), Path(args.out)
    tree = Tree(repo)
    sections = parse(Path(args.patch).read_text(encoding="utf-8"))

    main_patch: list[str] = []
    routed: dict[str, list[tuple[str, list[str], list[str]]]] = defaultdict(list)
    unrouted: list[tuple[str, str, list[str], list[str], str]] = []
    mapped, dropped, verbatim = [], [], {}
    residue: dict[str, Counter] = defaultdict(Counter)
    conflicts: dict[str, int | str] = {}

    for old, new, header, hunks in sections:
        assert old == new, f"a rename in the patch: {old} -> {new}"
        kind = "new" if "--- /dev/null" in header else "deleted" if "+++ /dev/null" in header else "modified"
        for hunk in hunks:
            added = [l[1:] for l in hunk[1:] if l.startswith("+")]
            for tag, pattern in OLD_NAMES:
                for l in added:
                    for m in re.finditer(pattern, l):
                        residue[old][f"{tag}: {m.group(1)}"] += 1
        if old in SPLIT:
            for hunk in hunks:
                scores = sorted(
                    ((score(hunk, tree.lines(c)), c) for c in SPLIT[old]), reverse=True
                )
                best, second = scores[0], scores[1] if len(scores) > 1 else (0.0, "")
                if best[0] >= 0.6 and best[0] - second[0] >= 0.15:
                    routed[best[1]].append((old, header, hunk))
                else:
                    why = ", ".join(f"{c} {s:.2f}" for s, c in scores)
                    unrouted.append((old, kind, header, hunk, why))
            continue
        dest = single(old)
        if dest is None:
            dropped.append((old, kind, len(hunks)))
            continue
        dest = after_p9(dest)
        # A market graph test S4 split between the market and FIX crates:
        # route its hunks like the split files.
        fix_half = dest.replace("rust/market/tests/graph/", "rust/fix/tests/graph/")
        if dest.startswith("rust/market/tests/graph/") and kind == "modified" and tree.lines(fix_half) is not None:
            for hunk in hunks:
                scores = sorted(((score(hunk, tree.lines(c)), c) for c in (dest, fix_half)), reverse=True)
                (s1, c1), (s2, _) = scores
                if s1 >= 0.6 and s1 - s2 >= 0.15:
                    routed[c1].append((old, header, hunk))
                else:
                    why = ", ".join(f"{c} {s:.2f}" for s, c in scores)
                    unrouted.append((old, kind, header, hunk, why))
            continue
        if args.preview and kind == "modified":
            conflicts[dest] = preview(repo, Path(args.preview), old, header, hunks, tree.lines(dest))
        main_patch += retarget(header, old, new, dest, keep_index=True)
        for hunk in hunks:
            main_patch += hunk
        target = None if kind == "new" else tree.lines(dest)
        held = sum(held_verbatim(h, target) for h in hunks) if kind == "modified" else len(hunks)
        verbatim[dest] = (held, len(hunks))
        mapped.append((old, dest, kind, len(hunks), tree.where(dest) if kind != "new" else (
            "new (absent, as expected)" if tree.where(dest) == "missing" else f"new, but present: {tree.where(dest)}"
        )))

    routed_patch: list[str] = []
    for dest in sorted(routed):
        items = routed[dest]
        old, header = items[0][0], items[0][1]
        routed_patch += retarget(header, old, old, dest, keep_index=False)
        for _, _, hunk in items:
            routed_patch += hunk
    unrouted_patch: list[str] = []
    for old, kind, header, hunk, _ in unrouted:
        unrouted_patch += retarget(header, old, old, old, keep_index=False)
        unrouted_patch += hunk

    out.mkdir(parents=True, exist_ok=True)
    for name, body in (
        ("p5_on_s4p9.patch", main_patch),
        ("p5_on_s4p9.routed.patch", routed_patch),
        ("p5_on_s4p9.unrouted.patch", unrouted_patch),
    ):
        (out / name).write_text("\n".join(body) + ("\n" if body else ""), encoding="utf-8")

    r = ["# P5R path map report", ""]
    r.append(
        f"Source `{Path(args.patch).name}`: {len(sections)} sections. Mapped {len(mapped)} "
        f"(`p5_on_s4p9.patch`), routed {sum(len(v) for v in routed.values())} hunks into "
        f"{len(routed)} files (`p5_on_s4p9.routed.patch`), unrouted {len(unrouted)} hunks "
        f"(`p5_on_s4p9.unrouted.patch`), dropped {len(dropped)} sections."
    )
    r += ["", "Destinations are checked against `git ls-tree -r HEAD` and the working tree (P9 in flight).", ""]
    r += ["## Mapped (one destination, `git apply -3`)", "", "| Patch path | Destination | Kind | Hunks | Old side verbatim | Destination is |", "| --- | --- | --- | --- | --- | --- |"]
    for old, dest, kind, n, where in mapped:
        held, total = verbatim[dest]
        shown = dest if dest != old else "(unchanged)"
        r.append(f"| `{old}` | `{shown}` | {kind} | {n} | {held}/{total} | {where} |")
    if args.preview:
        r += ["", "## Merge preview (`git apply -3`'s merge, against the tree as it stands)", ""]
        clean = [d for d, c in conflicts.items() if c == 0]
        r.append(f"{len(clean)} of {len(conflicts)} modified files merge clean; the rest:")
        r += ["", "| Destination | Conflicts |", "| --- | --- |"]
        r += [f"| `{d}` | {c} |" for d, c in sorted(conflicts.items()) if c != 0]
    r += ["", "## Routed by hunk (S4 split the file; `git apply --reject`)", "", "| Patch path | Destination | Hunks | Old side verbatim | Destination is |", "| --- | --- | --- | --- | --- |"]
    for dest in sorted(routed):
        items = routed[dest]
        held = sum(held_verbatim(h, tree.lines(dest)) for _, _, h in items)
        r.append(f"| `{items[0][0]}` | `{dest}` | {len(items)} | {held}/{len(items)} | {tree.where(dest)} |")
    r += ["", "## Unrouted hunks (no candidate holds the old side; by hand)", ""]
    if not unrouted:
        r.append("None.")
    for old, kind, _, hunk, why in unrouted:
        r.append(f"- `{old}` `{hunk[0]}` - scores: {why}")
    r += ["", "## Dropped", ""]
    for old, kind, n in dropped:
        reason = "this lane writes its own record" if old.startswith(".handoff/") else "S4 deleted `rust/examples/`"
        r.append(f"- `{old}` ({kind}, {n} hunks): {reason}")
    r += ["", "## Not mapped: paths missing from both HEAD and the working tree", ""]
    missing = [(o, d) for o, d, k, _, w in mapped if k != "new" and w == "missing"]
    r += [f"- `{o}` -> `{d}`" for o, d in missing] or ["None."]
    r += ["", "## Old names in the added lines (re-spell after apply; never write back)", "", "| Patch path | Names |", "| --- | --- |"]
    for old in sorted(residue):
        names = ", ".join(f"{k} x{v}" for k, v in sorted(residue[old].items()))
        r.append(f"| `{old}` | {names} |")
    (out / "path_map_report.md").write_text("\n".join(r) + "\n", encoding="utf-8")
    print(r[2])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
