#!/usr/bin/env python3
"""P7 / D40.4: the registered-code columns, holders and readers drop their
`code` suffix, one spelling each, over the tree P9 (D42) leaves.

Targets the POST-P9 tree: the names below are D42's final ones - the market
holder `instcode` beside `isincode`, `cficode`, `miccode`; the FIX crate tags
65_022 `isincode`, 65_023 `miccode`, 65_049 `forexcode`, 65_050
`bloombergcode`, 65_051 `figicode`; the `instrument` row's `cficode`,
`countrycode`, `forexcode`, `eusipacode` and its nested `listings`'
`miccode` (`rust/market/src/{instrument,listing}.rs`). Run it with `--check`
first, once P9 has landed: what it lists as REVIEW and RESIDUE is the residue
this sweep does not decide - every other site is the mechanical rename.

| Old | New |
| --- | --- |
| `isincode` (`get_isincode`, `fix_message_isincode`, ...) | `isin` |
| `cficode` (`get_cficode`, `set_cficode`, `try_with_cficode`, ...) | `cfi` |
| `miccode` (`get_miccode`, `set_miccode`, `Listing::miccode`, ...) | `mic` |
| `bloombergcode` | `bbg` |
| `figicode` | `figi` |
| `forexcode` | `forex` |
| `eusipacode` (`with_eusipacode`, ...) | `eusipa` |
| `countrycode` (`with_countrycode`, the field, the column) | `country` |
| `countrycode(` called bare - `Instrument::countrycode()` | `stated_country(` (`country()` is the derived one) |
| `MarketColumn::{IsinCode, CfiCode, MicCode}` | `{Isin, Cfi, Mic}` (D40.2 then moves them to `LiftedColumn`) |
| `ISINCODE_TAG_NAME`, `MICCODE_`, `FIGICODE_`, `FOREXCODE_`, `BLOOMBERGCODE_` | `ISIN_TAG_NAME`, `MIC_`, `FIGI_`, `FOREX_`, `BBG_` (and the bare `_TAG` forms) |
| the `instrument` row's indexes `CFICODE`, `FOREXCODE`, `COUNTRYCODE`, `EUSIPACODE` (`NAMES[..]`, `const`) | `CFI`, `FOREX`, `COUNTRY`, `EUSIPA` |
| the fixed row's import aliases `FIGICODE`, `FOREXCODE`, `BLOOMBERGCODE` (`rust/fix/src/schema.rs`) | `FIGI`, `FOREX`, `BBG` |
| displays `"ISIN Code"`, `"CFI Code"`, `"MIC Code"`, `"Forex Code"`, `"Bloomberg Code"`, `"FIGI Code"`, `"Country Code"`, `"EUSIPA Code"` | `"ISIN"`, `"CFI"`, `"MIC"`, `"Forex"`, `"BBG"`, `"FIGI"`, `"Country"`, `"EUSIPA"` |

A lower-case name is matched where no letter or digit precedes it (`_` may:
`get_isincode`), so the dictionary's own words - `detailedcficode`,
`legcficode`, `underlyingcficode`, `isocountrycode`, `legisincode` - keep
their spelling; upper-case and CamelCase words are wire and dictionary
spellings (`ISINCODE`, `CFICode`, `LegCFICode`, `ISINCode`, `EUSIPACode`,
`OMS_FIGICODE`, `CFICODE_TAG` - 461) and are never edited. `instcode`,
`crosscode`, `hashcode` and `crosshashcode` hold none of the names.

Not edited, listed as REVIEW (D40's interpretation 6, the intake rule and
the hash rule):
- a digest feed's name, `feed("cficode", ..)`, `feed("miccode", ..)`
  (`rust/market/src/graph/market.rs`): the name is hashed, and D40 moves no
  market row's `hashcode`; the instrument's feeds read `NAMES[..]`, which the
  sweep renames, so every `Instrument::hashcode` moves once at P7 (the
  instruments are rebuilt from bronze at P7 anyway, D40.5);
- a name written as a wire key, `|isincode=US..|`, `|eusipacode=2300|`: a
  bridge's spelling or a crate column stated by name on the wire - the latter
  (`rust/fix/tests/root/batch.rs`, `python/tests/test_fix.py`,
  `node/tests/fix.test.js`) re-spelled by hand where the test means the
  crate's column;
- a quoted or backticked `cficode` under the FIX crate, its tests, benches,
  pages and skill: on the FIX row it is `CFICode(461)`'s registry name
  (D40.1), on a `marketdata` row the market name - one decision per site;
- `eusipacode` within three lines of an `sspa` word: the six EUSIPA intake
  spellings (`eusipa`, `eusipacode`, `eusipacategory`, `sspa`, `sspacode`,
  `sspacategory`) read a foreign column and keep their spelling;
- every match in `INTAKE_FILES`: the `IdType` word tables and
  `Identifier::from_key` read `isincode`, `cficode`, `forexcode`, ... as
  intake spellings of the types; a doc line there naming a column is
  re-spelled by hand;
- the CamelCase display spellings in the FIX crate's column docs and pages
  (`displayed \\`ForexCode\\``), which follow the display rename once the
  dump is written.

RESIDUE is what the broad net `(isin|cfi|mic|bloomberg|figi|forex|country|
eusipa)_?code` still finds in a swept line that no rule renamed and no
foreign spelling explains; SHADOW flags a renamed `let` binding with the new
name already bound within forty lines (the compiler does not see a shadow).

Usage:
  python3 p7_sweep.py <tree root> --check [--manifest PATH]
  python3 p7_sweep.py <tree root> --apply PATH

`--check` writes nothing in the tree: it prints the sites per file, the
REVIEW, RESIDUE and SHADOW lists, and with `--manifest` writes every site -
path, line, the line as it is, the line as it becomes - to PATH. `--apply
PATH` recomputes the sweep and refuses unless it answers exactly the sites
PATH holds, each site's anchor - its whole original line - found at its
line and occurring in its file exactly as many times as the manifest lists
it; then it writes every file, or none. Idempotent: a swept tree has no site.
The kept region (the dictionary-hash rustdoc in
`rust/fix/tests/root/store.rs`, whose sentences are history) is never read.
Files are git's tracked and untracked-not-ignored set, so the sweep sees P9's
new files before they are committed.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

# The lower-case names, longest first so no name is read inside another.
LOWER = {
    "bloombergcode": "bbg",
    "countrycode": "country",
    "eusipacode": "eusipa",
    "forexcode": "forex",
    "isincode": "isin",
    "figicode": "figi",
    "cficode": "cfi",
    "miccode": "mic",
}
NAMES = "|".join(LOWER)

DISPLAYS = {
    '"ISIN Code"': '"ISIN"',
    '"CFI Code"': '"CFI"',
    '"MIC Code"': '"MIC"',
    '"Forex Code"': '"Forex"',
    '"Bloomberg Code"': '"BBG"',
    '"FIGI Code"': '"FIGI"',
    '"Country Code"': '"Country"',
    '"EUSIPA Code"': '"EUSIPA"',
}
CONSTS = {"ISIN": "ISIN", "MIC": "MIC", "FIGI": "FIGI", "FOREX": "FOREX", "BLOOMBERG": "BBG"}

# (name, pattern, replacement[, path]): applied in order to each line outside
# a kept region, a rule naming a path to that file alone; a replacement is a
# string or a function of the match.
RULES = [
    # `Instrument::countrycode()` - the stated country - beside `country()`.
    (
        "stated_country",
        re.compile(r"(?<![A-Za-z0-9_])countrycode(?=\()"),
        "stated_country",
    ),
    (
        "lower",
        re.compile(r"(?<![A-Za-z0-9])(" + NAMES + r")"),
        lambda match: LOWER[match.group(1)],
    ),
    (
        "variant",
        re.compile(r"(?<![A-Za-z0-9])(Isin|Cfi|Mic)Code(?![A-Za-z0-9])"),
        lambda match: match.group(1),
    ),
    (
        "const",
        re.compile(r"(?<![A-Za-z0-9_])(ISIN|MIC|FIGI|FOREX|BLOOMBERG)CODE_TAG(_NAME)?(?![A-Za-z0-9])"),
        lambda match: CONSTS[match.group(1)] + "_TAG" + (match.group(2) or ""),
    ),
    (
        "display",
        re.compile("|".join(re.escape(old) for old in DISPLAYS)),
        lambda match: DISPLAYS[match.group(0)],
    ),
    # The fixed row's import aliases of three crate tags.
    (
        "alias",
        re.compile(r"(?<![A-Za-z0-9_])(FIGI|FOREX|BLOOMBERG)CODE(?![A-Za-z0-9_])"),
        lambda match: CONSTS[match.group(1)],
        "rust/fix/src/schema.rs",
    ),
    # The instrument row's column indexes.
    (
        "index",
        re.compile(r"(?<=\[)(CFI|FOREX|COUNTRY|EUSIPA)CODE(?=\])|(?<=const )(CFI|FOREX|COUNTRY|EUSIPA)CODE(?=:)"),
        lambda match: match.group(1) or match.group(2),
        "rust/market/src/instrument.rs",
    ),
]

# Where every match is an intake spelling or a dictionary name, so nothing is
# edited and every match is REVIEW.
INTAKE_FILES = {
    "rust/market/src/idtype.rs",
    "rust/market/src/identifier.rs",
    "rust/market/tests/root/idtype.rs",
    "rust/market/tests/root/identifier.rs",
    "python/src/identifier.rs",
    "python/tests/test_identifier.py",
    "node/src/identifier.rs",
    "node/tests/identifier.test.js",
    "docs/graph/identifier.md",
    "scripts/generate_fix_dictionary.py",
}

# Where a quoted `cficode` may be the FIX row's 461 column (D40 item 6).
FIX_SCOPE = (
    "rust/fix/",
    "python/tests/test_fix.py",
    "python/tests/medallion.py",
    "python/benchmarks/fix",
    "node/tests/fix.test.js",
    "node/benchmarks/fix",
    "docs/fix/",
    "docs/graph/schemas.md",
    "skills/yggdryl-fix/",
)
QUOTED_CFI = re.compile(r"""(?<=["'`])cficode(?=["'`])""")
# The EUSIPA intake list: `eusipacode` within two lines of its siblings.
SSPA = re.compile(r"(?<![a-z])(sspacode|sspacategory|eusipacategory)(?![a-z])")
EUSIPA = re.compile(r"(?<![A-Za-z0-9])eusipacode")
# A name written as a wire key - `|eusipacode=2300|`, `#isincode=` - is the
# bridge's spelling, lower-cased by a reader, never ours.
WIRE = re.compile(r"(?<=[|#\x01])(" + NAMES + r")(?==)")
# A digest feed's name is hashed: `staged.feed("cficode", ..)` keeps its
# spelling, or every market row's `hashcode` and `uuid` move (D40 "What does
# not move": the hash feeds of every leaf but the book's).
FEED = re.compile(r'(?<=feed\(")(' + NAMES + r')(?=")')
DISPLAY_CAMEL = re.compile(r"(?<![A-Za-z0-9_])(ForexCode|ISINCode|MICCode|FIGICode|BloombergCode)(?![A-Za-z0-9])")
DISPLAY_SCOPE = ("rust/fix/src/crated.rs", "rust/fix/tests/root/crated.rs", "docs/fix/capture.md", "docs/graph/schemas.md")

# The broad net, and the spellings it may meet that are not ours: a match on
# a line no REVIEW note covers is RESIDUE.
NET = re.compile(r"(?i)(isin|cfi|mic|bloomberg|figi|forex|country|eusipa)code")
FOREIGN = re.compile(
    r"[A-Za-z0-9](?i:isin|cfi|mic|bloomberg|figi|forex|country|eusipa)_?(?i:code)"  # a longer word
    r"|(?<![a-z])(ISIN|CFI|MIC|BLOOMBERG|FIGI|FOREX|COUNTRY|EUSIPA)CODE"  # wire, `CFICODE_TAG`
    r"|(?<![A-Za-z0-9])(CFI|ISIN|MIC|FIGI|EUSIPA)Code"  # FIX and bridge CamelCase
    r"|(?<![A-Za-z0-9])(Forex|Bloomberg|Country)Code"
)

EXCLUDED_PREFIXES = (
    ".git/",
    ".handoff/",
    "target/",
    "config/fix/",
    "docs/assets/",
    "node/node_modules/",
    "python/.venv",
)
EXCLUDED_PATHS = {
    "node/index.js",
    "node/index.d.ts",
    "rust/fix/src/constants.rs",
    "rust/fix/tests/root/equivalence.snapshot",
}
EXCLUDED_SUFFIXES = (".log", ".zst", ".so", ".pyc", ".parquet", ".avro", ".xlsx", ".zip", ".png")

KEPT_REGIONS = {
    "rust/fix/tests/root/store.rs": (
        "    /// The committed dictionary's hash, pinned as a literal.\n",
        "    fn the_committed_dictionary_hashes_to_one_pinned_value() {\n",
    ),
}
# A binding of a name: `let`, a closure's `|name|`, a parameter `(name:`.
BINDING = "(?:let (?:mut )?|\\||[(,]\\s*)({})\\s*[:=|,)]"


def excluded(path):
    return (
        path in EXCLUDED_PATHS
        or path.startswith(EXCLUDED_PREFIXES)
        or path.endswith(EXCLUDED_SUFFIXES)
    )


def tree_files(root):
    listing = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        check=True,
        capture_output=True,
    ).stdout
    names = {name for name in listing.decode("utf-8").split("\0") if name}
    return sorted(name for name in names if not excluded(name) and (root / name).is_file())


def read_text(path):
    data = path.read_bytes()
    if b"\0" in data:
        return None
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return None


def kept_lines(path, lines, errors):
    """The 0-based line numbers the sweep never reads."""
    region = KEPT_REGIONS.get(path)
    if region is None:
        return set()
    start, end = region
    if lines.count(start) != 1 or lines.count(end) != 1:
        errors.append(f"{path}: the kept region's anchors are not each found once")
        return set()
    first, last = lines.index(start), lines.index(end)
    if last < first:
        errors.append(f"{path}: the kept region ends before it starts")
        return set()
    return set(range(first, last))


def sweep_line(path, lines, index):
    """The line rewritten, the rules that moved it, and its REVIEW notes."""
    line = lines[index]
    review = []
    protected = []  # spans no rule may edit
    if path in INTAKE_FILES:
        if any(rule[1].search(line) for rule in RULES):
            review.append("intake file")
        return line, [], review
    for match in WIRE.finditer(line):
        protected.append(match.span())
        review.append("wire key")
    for match in FEED.finditer(line):
        protected.append(match.span())
        review.append("digest feed name, kept")
    if path.startswith(FIX_SCOPE):
        for match in QUOTED_CFI.finditer(line):
            protected.append(match.span())
            review.append("quoted cficode in FIX scope (D40 item 6)")
    if EUSIPA.search(line):
        window = "".join(lines[max(0, index - 2) : index + 3])
        if SSPA.search(window):
            for match in EUSIPA.finditer(line):
                protected.append(match.span())
                review.append("eusipacode beside the EUSIPA intake spellings")
    if path in DISPLAY_SCOPE and DISPLAY_CAMEL.search(line):
        review.append("CamelCase display spelling")

    moved = []
    # Protected spans are swapped for placeholders no rule can match.
    held = {}
    for number, (start, end) in enumerate(sorted(protected, reverse=True)):
        token = f"\x00{number}\x00"
        held[token] = line[start:end]
        line = line[:start] + token + line[end:]
    for name, pattern, replacement, *scope in RULES:
        if scope and path != scope[0]:
            continue
        line, count = pattern.subn(replacement, line)
        if count:
            moved.append(name)
    for token, text in held.items():
        line = line.replace(token, text)
    return line, moved, review


def residue(line):
    """What the net still finds in `line` that no foreign spelling explains."""
    found = []
    for match in NET.finditer(line):
        start = max(0, match.start() - 1)
        window = line[start : match.end()]
        if FOREIGN.search(window) or FOREIGN.match(line, match.start()):
            continue
        found.append(match.group(0))
    return found


def sweep(root):
    errors, sites, reviews, residues, shadows = [], [], [], [], []
    texts = {}
    for path in tree_files(root):
        text = read_text(root / path)
        if text is None:
            continue
        lines = text.splitlines(keepends=True)
        kept = kept_lines(path, lines, errors)
        new_lines = list(lines)
        for index, line in enumerate(lines):
            if index in kept:
                continue
            new, moved, review = sweep_line(path, lines, index)
            for note in review:
                reviews.append((path, index + 1, note, line.rstrip("\n")))
            if new != line:
                sites.append(
                    {"path": path, "line": index + 1, "old": line, "new": new, "rules": moved}
                )
                new_lines[index] = new
            if path not in INTAKE_FILES and not review:
                for word in residue(new):
                    residues.append((path, index + 1, word, new.rstrip("\n")))
        texts[path] = "".join(new_lines)
        # SHADOW: a renamed binding whose new name is bound nearby already.
        for site in (s for s in sites if s["path"] == path):
            for old, new in LOWER.items():
                if not re.search(BINDING.format(old), site["old"]):
                    continue
                low, high = max(0, site["line"] - 41), site["line"] + 40
                window = [l for n, l in enumerate(lines[low:high], low + 1) if n != site["line"]]
                if any(re.search(BINDING.format(new), l) for l in window):
                    shadows.append((path, site["line"], f"{old} -> {new}", site["old"].rstrip("\n")))
    return texts, sites, reviews, residues, shadows, errors


def report(sites, reviews, residues, shadows):
    per_file = {}
    for site in sites:
        per_file[site["path"]] = per_file.get(site["path"], 0) + 1
    print(f"SITES {len(sites)} lines in {len(per_file)} files")
    for path, count in sorted(per_file.items()):
        print(f"  {count:5d} {path}")
    for title, rows in (("REVIEW", reviews), ("RESIDUE", residues), ("SHADOW", shadows)):
        print(f"{title} {len(rows)}")
        for path, line, note, text in rows:
            print(f"  {path}:{line}: {note}: {text.strip()[:140]}")


def main(argv):
    args = [arg for arg in argv[1:] if not arg.startswith("--")]
    flags = [arg for arg in argv[1:] if arg.startswith("--")]
    if not args or not flags or flags[0] not in ("--check", "--apply"):
        sys.exit(__doc__)
    root = Path(args[0]).resolve()
    texts, sites, reviews, residues, shadows, errors = sweep(root)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        sys.exit(f"{len(errors)} refusal(s); nothing written")

    if flags[0] == "--check":
        report(sites, reviews, residues, shadows)
        if "--manifest" in flags:
            out = Path(args[1])
            out.write_text(json.dumps(sites, indent=1) + "\n")
            print(f"manifest: {out} ({len(sites)} sites)")
        return

    manifest = json.loads(Path(args[1]).read_text())
    key = lambda site: (site["path"], site["line"], site["old"], site["new"])
    if sorted(map(key, sites)) != sorted(map(key, manifest)):
        planned, now = set(map(key, manifest)), set(map(key, sites))
        for site in sorted(planned - now)[:20]:
            print(f"planned, not found: {site[0]}:{site[1]}", file=sys.stderr)
        for site in sorted(now - planned)[:20]:
            print(f"found, not planned: {site[0]}:{site[1]}", file=sys.stderr)
        sys.exit("the tree no longer answers the manifest; run --check again; nothing written")
    refusals = []
    by_path = {}
    for site in manifest:
        by_path.setdefault(site["path"], []).append(site)
    for path, rows in by_path.items():
        lines = (root / path).read_text().splitlines(keepends=True)
        for site in rows:
            if lines[site["line"] - 1] != site["old"]:
                refusals.append(f"{path}:{site['line']}: the anchor is not at its line")
            expected = sum(1 for row in rows if row["old"] == site["old"])
            found = sum(1 for line in lines if line == site["old"])
            if found != expected:
                refusals.append(
                    f"{path}:{site['line']}: anchor occurs {found} times, the manifest lists {expected}"
                )
    if refusals:
        for refusal in sorted(set(refusals)):
            print(refusal, file=sys.stderr)
        sys.exit(f"{len(set(refusals))} refusal(s); nothing written")
    for path in by_path:
        (root / path).write_bytes(texts[path].encode("utf-8"))
    print(f"changed {len(by_path)} file(s), {len(manifest)} line(s)")


if __name__ == "__main__":
    main(sys.argv)
