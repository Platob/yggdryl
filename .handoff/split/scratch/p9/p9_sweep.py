#!/usr/bin/env python3
"""P9 phase 0 (D42.8): the ISIN registry renamed onto the instrument, as a
pure rename - no behaviour, no pin moved.

`IsinRegistry` -> `Instruments`, `IsinEntry` -> `Instrument`, `IsinTable` ->
`InstrumentTable`, the module `isin_registry` -> `instrument` (a value, a
field, a keyword or a getter of that name -> `instruments`),
`with_isin_registry` -> `with_instruments`, the Node `isinRegistry` ->
`instruments`, `YGGDRYL_ISIN_REGISTRY_URI` -> `YGGDRYL_INSTRUMENTS_URI`, the
default folder `~/.config/yggdryl/isin/` -> `~/.config/yggdryl/instruments/`,
the seed `config/isin/instruments.json` -> `config/instruments/instruments.json`,
`scripts/check_isin_seed.py` -> `scripts/check_instruments_seed.py`, the docs
page `docs/graph/isin-registry.md` -> `docs/graph/instrument.md`, and every
file named for the registry moved by `git mv` onto the instrument's name.

Usage: python3 -I p9_sweep.py <tree root> [--check]

Three phases, every one validated before any file is written:

1. `PRE_EDITS`: the codec's crate-private snapshot door and field, today
   `instruments`, become `instrument_table` - the name `instruments` is the
   shared collection's after the rename - exact anchors, each asserted to
   occur exactly the count it states.
2. The sites: every line the D42.8 grep (plus the default folder) reports,
   each asserted to read exactly as the grep reported it (the anchor), every
   rule of `RULES` applied to it in order, and the line asserted to match the
   grep no more.
3. `MOVES`: `git mv` of every file named for the registry.

Idempotent: an applied pre-edit is recognised by its replacement, a swept
tree reports no site and a moved file is skipped. `--check` validates and
writes nothing, then exits non-zero unless the tree is already swept.
"""

import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

GREP = r"isin[-_ ]?registr|IsinEntry|IsinTable|check_isin_seed|config/isin"
# The default folder no D42.8 pattern names.
SITE_GREP = GREP + r"|yggdryl/isin/"
GREP_RE = re.compile(GREP, re.IGNORECASE)
SITE_RE = re.compile(SITE_GREP, re.IGNORECASE)

# (path, old, new, count): the codec's snapshot door and field.
PRE_EDITS = [
    ("rust/fix/src/codec.rs", "    instruments: Option<IsinTable>,\n", "    instrument_table: Option<IsinTable>,\n", 1),
    ("rust/fix/src/codec.rs", "            instruments: None,\n", "            instrument_table: None,\n", 1),
    ("rust/fix/src/codec.rs", "        self.instruments = None;\n", "        self.instrument_table = None;\n", 1),
    (
        "rust/fix/src/codec.rs",
        "    pub(super) fn instruments(&self) -> Option<Cow<'_, IsinTable>> {\n        if let Some(table) = &self.instruments {\n",
        "    pub(super) fn instrument_table(&self) -> Option<Cow<'_, IsinTable>> {\n        if let Some(table) = &self.instrument_table {\n",
        1,
    ),
    (
        "rust/fix/src/codec.rs",
        "        if codec.instruments.is_none() {\n            codec.instruments = self.instruments().map(Cow::into_owned);\n",
        "        if codec.instrument_table.is_none() {\n            codec.instrument_table = self.instrument_table().map(Cow::into_owned);\n",
        1,
    ),
    ("rust/fix/src/codec.rs", "            self.instruments().as_deref(),\n", "            self.instrument_table().as_deref(),\n", 1),
    (
        "rust/fix/src/messages.rs",
        "Source::Frames { codec, .. } => codec.instruments(),",
        "Source::Frames { codec, .. } => codec.instrument_table(),",
        1,
    ),
]

# Applied in order to every site line, each replacing every occurrence on it.
RULES = [
    # Prose, by hand: the registry named in words.
    ("kinds, identifiers, ISIN registry and market graph", "kinds, identifiers, instruments and market graph"),
    ("market kinds, identifiers, ISIN registry,", "market kinds, identifiers, instruments,"),
    ("and the ISIN registry |", "and the instruments |"),
    ("identifiers, the ISIN registry and the market graph", "identifiers, the instruments and the market graph"),
    ("the identifiers, the ISIN registry and the market-data graph", "the identifiers, the instruments and the market-data graph"),
    ("isin registry get", "instruments get"),
    ("//! The ISIN registry: learning", "//! The instruments: learning"),
    ('"isin registry holder"', '"instruments holder"'),
    ('"isin registry",', '"instruments",'),
    ("/// An ISIN registry learns", "/// The instruments learn"),
    ("An ISIN an ISIN registry filled", "An ISIN the instruments filled"),
    ('"an isin registry read"', '"an instruments read"'),
    ("an ISIN registry or two columns", "the instruments or two columns"),
    ("an ISIN registry scores", "the instruments score"),
    ("an ISIN registry's classification", "an instrument's classification"),
    # The nav entry and the page.
    ("- IsinRegistry: graph/isin-registry.md", "- Instrument: graph/instrument.md"),
    ("isin-registry.md", "instrument.md"),
    # The test functions named for the registry.
    ("an_isin_registry_reads_and_learns_", "instruments_read_and_learn_"),
    ("an_isin_registry_reads_and_fills_", "instruments_read_and_fill_"),
    ("an_isin_registry_ticker_index_doubles_once_as_its_", "the_instruments_ticker_index_doubles_once_as_their_"),
    ("an_isin_registry_learns_", "instruments_learn_"),
    ("an_isin_registry_resolves_", "instruments_resolve_"),
    ("an_isin_registry_economic_scan_", "the_instruments_economic_scan_"),
    ("an_isin_registry_snapshot_stream_", "the_instruments_snapshot_stream_"),
    ("an_isin_registry_reloads_", "instruments_reload_"),
    ("an_isin_registry_reads_a_holder_", "instruments_read_a_holder_"),
    ("a_shared_isin_registry_carries_", "shared_instruments_carry_"),
    # Paths and scripts.
    ("config/isin/", "config/instruments/"),
    ("yggdryl/isin/", "yggdryl/instruments/"),
    ("check_isin_seed", "check_instruments_seed"),
    ("yggdryl-isin-registry-", "yggdryl-instruments-"),
    ("ISOLATED_ISIN_REGISTRY_TEST", "ISOLATED_INSTRUMENT_TEST"),
    ("YGGDRYL_ISIN_REGISTRY_URI", "YGGDRYL_INSTRUMENTS_URI"),
    # The module, wherever it is a path.
    ("isin_registry_as_table", "instruments_as_table"),
    ("isin_registry_learn_stating", "instruments_learn_stating"),
    ("with_isin_registry", "with_instruments"),
    ("test_isin_registry.py", "test_instrument.py"),
    ("isin_registry.test.js", "instrument.test.js"),
    ("isin_registry.types.ts", "instrument.types.ts"),
    ("isin_registry.py", "instrument.py"),
    ("isin_registry.rs", "instrument.rs"),
    ("isin_registry/", "instrument/"),
    ('/ "isin_registry" /', '/ "instrument" /'),
    ("isin_registry::", "instrument::"),
    ("mod isin_registry", "mod instrument"),
    ("crate::isin_registry", "crate::instrument"),
    ("yggdryl_market::isin_registry", "yggdryl_market::instrument"),
    ("internals::isin_registry", "internals::instrument"),
    ("as isin_registry_env", "as instrument_env"),
    ("as isin_registry;", "as instrument;"),
    ("graph_benches::isin_registry", "graph_benches::instrument"),
    ("graph/isin_registry", "graph/instrument"),
    ("from .isin_registry", "from .instrument"),
    ("yggdryl.isin_registry", "yggdryl.instrument"),
    ("--test isin_registry", "--test instrument"),
    ("--test root -- isin_registry", "--test root -- instrument"),
    ("--test allocations -- isin_registry", "--test allocations -- instruments"),
    ("graph::facts, isin_registry and idtype", "graph::facts, instrument and idtype"),
    # Every other `isin_registry` is a value: a field, a keyword, a getter, a variable.
    ("isin_registry", "instruments"),
    ("isinRegistry", "instruments"),
    # The types and the row.
    ("IsinRegistry", "Instruments"),
    ("IsinEntry", "Instrument"),
    ("IsinTable", "InstrumentTable"),
    ("isinregistry", "instrument"),
]

MOVES = [
    ("config/isin/instruments.json", "config/instruments/instruments.json"),
    ("docs/graph/isin-registry.md", "docs/graph/instrument.md"),
    ("node/src/isin_registry.rs", "node/src/instrument.rs"),
    ("node/tests/isin_registry.test.js", "node/tests/instrument.test.js"),
    ("node/tests/isin_registry.types.ts", "node/tests/instrument.types.ts"),
    ("python/src/isin_registry.rs", "python/src/instrument.rs"),
    ("python/tests/test_isin_registry.py", "python/tests/test_instrument.py"),
    ("python/yggdryl/isin_registry.py", "python/yggdryl/instrument.py"),
    ("rust/fix/tests/isin_registry.rs", "rust/fix/tests/instrument.rs"),
    ("rust/fix/tests/isin_registry/env.rs", "rust/fix/tests/instrument/env.rs"),
    ("rust/market/benchmarks/graph/isin_registry.rs", "rust/market/benchmarks/graph/instrument.rs"),
    ("rust/market/src/isin_registry.rs", "rust/market/src/instrument.rs"),
    ("rust/market/src/isin_registry/env.rs", "rust/market/src/instrument/env.rs"),
    ("rust/market/src/isin_registry/seed.json", "rust/market/src/instrument/seed.json"),
    ("rust/market/src/isin_registry/seed.rs", "rust/market/src/instrument/seed.rs"),
    ("rust/market/src/isin_registry/store.rs", "rust/market/src/instrument/store.rs"),
    ("rust/market/tests/isin_registry.rs", "rust/market/tests/instrument.rs"),
    ("rust/market/tests/isin_registry/env.rs", "rust/market/tests/instrument/env.rs"),
    ("rust/market/tests/isin_registry/seed.rs", "rust/market/tests/instrument/seed.rs"),
    ("rust/market/tests/isin_registry/store.rs", "rust/market/tests/instrument/store.rs"),
    ("rust/market/tests/root/isin_registry.rs", "rust/market/tests/root/instrument.rs"),
    ("scripts/check_isin_seed.py", "scripts/check_instruments_seed.py"),
]


def swept(text):
    """`text` with every rule applied: a pre-edit as a swept tree spells it."""
    for old, new in RULES:
        text = text.replace(old, new)
    return text


def git(root, *args):
    return subprocess.run(["git", "-C", str(root), *args], check=False, capture_output=True, text=True)


def sites(root, pattern):
    """`file -> [(line number, text)]` the grep reports, `.handoff/` excluded."""
    found = git(root, "grep", "-n", "-I", "-i", "-E", pattern, "--", ".", ":!.handoff")
    if found.returncode not in (0, 1):
        sys.exit(found.stderr)
    by_file = {}
    for row in found.stdout.splitlines():
        path, number, text = row.split(":", 2)
        by_file.setdefault(path, []).append((int(number), text))
    return by_file


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    root = Path(sys.argv[1]).resolve()
    check = "--check" in sys.argv[2:]
    errors = []
    writes = {}

    # 1. The pre-edits.
    pre_count = 0
    for path, old, new, count in PRE_EDITS:
        text = writes.get(path) or (root / path).read_text(encoding="utf-8")
        if max(text.count(new), text.count(swept(new))) >= count:
            continue
        if text.count(old) == count:
            writes[path] = text.replace(old, new)
            pre_count += count
        else:
            errors.append(f"{path}: pre-edit anchor {old!r} occurs {text.count(old)} times, expected {count}")

    # 2. The sites.
    rules = Counter()
    files = 0
    lines = 0
    for path, rows in sites(root, SITE_GREP).items():
        original = (root / path).read_text(encoding="utf-8").split("\n")
        text = writes.get(path) or "\n".join(original)
        body = text.split("\n")
        for number, reported in rows:
            if original[number - 1] != reported:
                errors.append(f"{path}:{number}: the line reads {original[number - 1]!r}, the grep reported {reported!r}")
                continue
            line = body[number - 1]
            for old, new in RULES:
                hits = line.count(old)
                if hits:
                    line = line.replace(old, new)
                    rules[old] += hits
            if SITE_RE.search(line):
                errors.append(f"{path}:{number}: no rule reaches {SITE_RE.search(line).group(0)!r} in {line!r}")
            body[number - 1] = line
            lines += 1
        writes[path] = "\n".join(body)
        files += 1

    # 3. The moves.
    moves = []
    for old, new in MOVES:
        if (root / new).exists() and not (root / old).exists():
            continue
        if not (root / old).exists():
            errors.append(f"{old}: nothing to move")
        elif (root / new).exists():
            errors.append(f"{new}: already exists beside {old}")
        else:
            moves.append((old, new))

    if errors:
        sys.exit("\n".join(errors))
    print(f"pre-edits: {pre_count}; sites: {lines} lines in {files} files; moves: {len(moves)}")
    for old, hits in rules.most_common():
        print(f"  {hits:5} {old}")
    if check:
        left = sum(len(rows) for rows in sites(root, GREP).values())
        if pre_count or lines or moves or left:
            sys.exit(f"not swept: {pre_count} pre-edits, {lines} site lines, {len(moves)} moves, {left} grep lines")
        print("swept")
        return
    for path, text in writes.items():
        target = root / path
        if target.read_text(encoding="utf-8") != text:
            target.write_text(text, encoding="utf-8")
    for old, new in moves:
        (root / new).parent.mkdir(parents=True, exist_ok=True)
        moved = git(root, "mv", old, new)
        if moved.returncode:
            sys.exit(moved.stderr)
    left = sites(root, GREP)
    if left:
        sys.exit(f"grep not empty after the sweep: {sorted(left)}")


if __name__ == "__main__":
    main()
