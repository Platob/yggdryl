"""Compile and run every example in the documentation.

The documentation contract says an example must be runnable, in every language
it is shown in. This extracts each fenced block under ``docs/`` and executes it:

* ``rust`` blocks become tests in one generated integration target and are
  compiled and run by cargo;
* ``python`` blocks run under the extension's virtual environment;
* ``javascript`` blocks run under node, with ``@yggdryl/node`` resolved to the
  package in this repository.

A block that genuinely cannot stand alone is tagged ``ignore`` (for example
``rust,ignore``), which is reported rather than hidden.

The same blocks become the downloadable notebooks under ``docs/notebooks/``,
which this regenerates and then checks for drift on every run, whatever
``--lang`` selects.

Usage:
    python scripts/check_docs_examples.py                 # every language
    python scripts/check_docs_examples.py --lang rust     # one language
    python scripts/check_docs_examples.py --keep          # keep generated files
"""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys
import tempfile
import textwrap
from typing import NamedTuple

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
RUST_TARGET = ROOT / "rust" / "tests" / "docs_examples.rs"
PYTHON = ROOT / "python" / ".venv" / "Scripts" / "python.exe"
if not PYTHON.exists():
    PYTHON = ROOT / "python" / ".venv" / "bin" / "python"
NODE_BINDING = (ROOT / "node" / "binding.js").as_posix()
# Apache Arrow JS is a dependency of the package, so a reader who installed
# ``@yggdryl/node`` can require it; a generated script in a temporary directory
# cannot, because Node resolves from the script's own folder.
NODE_ARROW = (ROOT / "node" / "node_modules" / "apache-arrow").as_posix()

BLOCK = re.compile(r"```(?P<lang>[a-z]+)(?P<flags>[^\n]*)\n(?P<code>.*?)```", re.DOTALL)
HEADING = re.compile(r"^#{1,6} \S")
LANGUAGES = ("rust", "python", "javascript")


class Block(NamedTuple):
    """One fenced example, and where on its page it was written."""

    index: int
    language: str
    flags: str
    code: str
    section: str


def slug(path: pathlib.Path) -> str:
    """Return a safe identifier for a documentation page."""
    relative = path.relative_to(DOCS).with_suffix("")
    return re.sub(r"[^a-z0-9]+", "_", str(relative).replace("\\", "/").lower()).strip("_")


def headings(text: str) -> list[tuple[int, str]]:
    """Return the offset and text of every heading on a page, in page order."""
    found: list[tuple[int, str]] = []
    offset = 0
    fenced = False
    for line in text.splitlines(keepends=True):
        # Fences are tracked because a `#` inside one opens a comment rather than
        # a section, and an example is full of those.
        if line.strip().startswith("```"):
            fenced = not fenced
        elif not fenced and HEADING.match(line):
            found.append((offset, line.strip()))
        offset += len(line)
    return found


def blocks(page: pathlib.Path):
    """Yield every fenced example on a page, in the order it appears."""
    text = page.read_text(encoding="utf-8")
    sections = headings(text)
    counters: dict[str, int] = {}
    for match in BLOCK.finditer(text):
        language = match.group("lang")
        if language not in LANGUAGES:
            continue
        index = counters.get(language, 0)
        counters[language] = index + 1
        # A block inside a Material tab is indented four spaces; Python cares.
        code = textwrap.dedent(match.group("code"))
        section = ""
        for offset, title in sections:
            if offset > match.start():
                break
            section = title
        yield Block(index, language, match.group("flags").strip(", "), code, section)


def runnable(language: str, flags: str) -> bool:
    """Report whether a block is one this script executes."""
    # `no_run` is a rustdoc word for "compile only", and this target has no way
    # to compile a block without also running it.
    return "ignore" not in flags and not (language == "rust" and "no_run" in flags)


def rust_target(pages) -> tuple[int, list[str]]:
    """Generate the Rust integration target and return its size."""
    functions: list[str] = []
    skipped: list[str] = []

    for page in pages:
        name = slug(page)
        for block in blocks(page):
            if block.language != "rust":
                continue
            code = block.code
            label = f"{name}_{block.index}"
            if not runnable("rust", block.flags):
                skipped.append(f"{page.relative_to(ROOT)} rust block {block.index} ({block.flags})")
                continue

            if "fn main" in code:
                body = code.replace(
                    "fn main() -> Result<(), Box<dyn std::error::Error>> {",
                    "#[test]\nfn example() -> Result<(), Box<dyn std::error::Error>> {",
                    1,
                ).replace("fn main() {", "#[test]\nfn example() {", 1)
            else:
                indented = "\n".join(
                    f"    {line}" if line.strip() else line for line in code.splitlines()
                )
                body = (
                    "#[test]\n"
                    "fn example() -> Result<(), Box<dyn std::error::Error>> {\n"
                    f"{indented}\n"
                    "    Ok(())\n"
                    "}\n"
                )

            scoped = "\n".join(
                f"    {line}" if line.strip() else line for line in body.splitlines()
            )
            functions.append(f"mod {label} {{\n{scoped}\n}}\n")

    header = (
        "//! Generated by scripts/check_docs_examples.py - do not edit.\n"
        "//!\n"
        "//! Every Rust example in the documentation, compiled and run.\n\n"
        "#![allow(unused_imports, unused_variables, clippy::all)]\n\n"
    )
    RUST_TARGET.write_text(header + "\n".join(functions), encoding="utf-8")
    return len(functions), skipped


def run_rust() -> int:
    result = subprocess.run(
        ["cargo", "test", "--features", "parquet iceberg", "--test", "docs_examples"],
        cwd=ROOT,
        check=False,
    )
    return result.returncode


def run_scripts(pages, language: str) -> tuple[int, int, list[str]]:
    """Run every block of one scripting language, returning counts and failures."""
    if language == "python" and not PYTHON.exists():
        return 0, 0, [f"{language}: no interpreter at {PYTHON}"]

    ran = 0
    skipped = 0
    failures: list[str] = []

    with tempfile.TemporaryDirectory() as directory:
        workspace = pathlib.Path(directory)
        for page in pages:
            for block in blocks(page):
                if block.language != language:
                    continue
                if not runnable(language, block.flags):
                    skipped += 1
                    continue

                label = f"{slug(page)}_{block.index}"
                if language == "python":
                    script = workspace / f"{label}.py"
                    script.write_text(block.code, encoding="utf-8")
                    command = [str(PYTHON), str(script)]
                else:
                    script = workspace / f"{label}.js"
                    rewired = block.code
                    for name, target in (
                        ("@yggdryl/node", NODE_BINDING),
                        ("apache-arrow", NODE_ARROW),
                    ):
                        rewired = rewired.replace(f"'{name}'", f"'{target}'").replace(
                            f'"{name}"', f'"{target}"'
                        )
                    script.write_text(rewired, encoding="utf-8")
                    command = ["node", str(script)]

                ran += 1
                result = subprocess.run(
                    command, cwd=ROOT, check=False, capture_output=True, text=True
                )
                if result.returncode != 0:
                    tail = (result.stderr or result.stdout).strip().splitlines()
                    detail = "\n      ".join(tail[-6:])
                    failures.append(
                        f"{page.relative_to(ROOT)} {language} block {block.index}:\n      {detail}"
                    )

    return ran, skipped, failures


def run_notebooks(pages) -> tuple[int, list[str]]:
    """Regenerate the downloadable notebooks and prove the generator settled."""
    # The builder reads its blocks from this module, so it is imported here
    # rather than beside the others: at module scope the two would be a cycle.
    from build_docs_notebooks import build

    count, problems = build(pages)
    # A generator that does not settle in one pass turns every documentation run
    # into a diff, so the second pass has to find nothing left to do.
    _, drift = build(pages, write=False)
    return count, [*problems, *drift]


def main() -> int:
    """Run every documentation example, and rebuild what is generated from them."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lang", choices=[*LANGUAGES, "all"], default="all")
    parser.add_argument("--keep", action="store_true", help="keep the generated Rust target")
    arguments = parser.parse_args()

    pages = sorted(DOCS.rglob("*.md"))
    status = 0

    count, problems = run_notebooks(pages)
    print(f"notebooks: {count} generated from {len(pages)} pages, {len(problems)} unresolved")
    for problem in problems:
        print(f"  {problem}")
    if problems:
        status |= 1

    if arguments.lang in ("rust", "all"):
        count, skipped = rust_target(pages)
        print(f"rust: {count} example tests from {len(pages)} pages")
        for entry in skipped:
            print(f"  skipped: {entry}")
        if count:
            status |= run_rust()
        if not arguments.keep:
            RUST_TARGET.unlink(missing_ok=True)

    for language in ("python", "javascript"):
        if arguments.lang not in (language, "all"):
            continue
        ran, skipped, failures = run_scripts(pages, language)
        print(f"{language}: {ran} examples run, {skipped} skipped, {len(failures)} failed")
        for failure in failures:
            print(f"  {failure}")
        if failures:
            status |= 1

    return status


if __name__ == "__main__":
    sys.exit(main())
