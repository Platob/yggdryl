"""Compile and run every example in the documentation.

The documentation contract says an example must be runnable, in every language
it is shown in. This extracts each fenced block under ``docs/`` and executes it:

* ``rust`` blocks become tests in one generated integration target and are
  compiled and run by cargo;
* ``python`` blocks run under the extension's virtual environment;
* ``javascript`` blocks run under node, with ``yggdryl`` resolved to the
  package in this repository.

A block that genuinely cannot stand alone is tagged ``ignore`` in the brace form
pymdownx.superfences reads (for example ``{ .rust .ignore }``), which is reported
rather than hidden. The comma form other tools accept is not one superfences
highlights, so a fence written that way is reported as a formatting failure
instead of shipping as an unhighlighted paragraph.

The scripting halves run one process per block, on a pool one process wide per
core; ``--jobs`` narrows it.

Usage:
    python scripts/check_docs_examples.py                 # every language
    python scripts/check_docs_examples.py --lang rust     # one language
    python scripts/check_docs_examples.py --jobs 4        # four at a time
    python scripts/check_docs_examples.py --keep          # keep generated files
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import textwrap
import threading
from typing import NamedTuple

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
RUST_TARGET = ROOT / "rust" / "tests" / "docs_examples.rs"
WORKERS = {"python": ROOT / "scripts" / "docs_worker.py", "javascript": ROOT / "scripts" / "docs_worker.js"}
PYTHON = ROOT / "python" / ".venv" / "Scripts" / "python.exe"
if not PYTHON.exists():
    PYTHON = ROOT / "python" / ".venv" / "bin" / "python"
NODE_BINDING = (ROOT / "node" / "binding.js").as_posix()
# Apache Arrow JS is a dependency of the package, so a reader who installed
# ``yggdryl`` can require it; a generated script in a temporary directory
# cannot, because Node resolves from the script's own folder.
NODE_ARROW = (ROOT / "node" / "node_modules" / "apache-arrow").as_posix()

# A fence is written the way pymdownx.superfences reads it: a bare language, or
# the brace form when a block also carries a flag for this script. Both spellings
# are extracted here; `INFO` is what decides whether a fence renders at all.
BLOCK = re.compile(
    r"```(?:\{ *\.(?P<braced>[a-z]+)(?P<attributes>[^}\n]*)\}|(?P<lang>[a-z]+)(?P<flags>[^\n]*))\n"
    r"(?P<code>.*?)```",
    re.DOTALL,
)
INFO = re.compile(r"[a-z]+|\{ *\.[a-z]+[^}\n]*\}")
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
        language = match.group("braced") or match.group("lang")
        if language not in LANGUAGES:
            continue
        flags = match.group("attributes") if match.group("braced") else match.group("flags")
        index = counters.get(language, 0)
        counters[language] = index + 1
        # A block inside a Material tab is indented four spaces; Python cares.
        code = textwrap.dedent(match.group("code"))
        section = ""
        for offset, title in sections:
            if offset > match.start():
                break
            section = title
        yield Block(index, language, flags.strip(" ,."), code, section)


def unhighlighted(page: pathlib.Path) -> list[str]:
    """Report fences on a page that pymdownx.superfences will not highlight.

    Superfences reads a bare language or the brace form; anything else - notably
    the ``lang,flag`` spelling other tools accept - falls through to plain
    Markdown, and the page then ships the backticks and the code as prose. That
    is invisible to a checker that only runs the code, so it is caught here.
    """
    problems: list[str] = []
    fenced = False
    for number, line in enumerate(page.read_text(encoding="utf-8").splitlines(), 1):
        stripped = line.strip()
        if not stripped.startswith("```"):
            continue
        # Only an opening fence carries an info string; a closing one is bare.
        if fenced:
            fenced = False
            continue
        fenced = True
        info = stripped[3:].strip()
        if info and not INFO.fullmatch(info):
            problems.append(
                f"{page.relative_to(ROOT)}:{number}: ```{info} is not a form "
                "pymdownx.superfences highlights - write it as "
                f"```{{ .{info.replace(',', ' .')} }}"
            )
    return problems


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
                # Never re-indented: Rust does not care, and a multi-line string
                # literal in an example does - adding a prefix would rewrite the
                # value the example asserts about.
                body = (
                    "#[test]\n"
                    "fn example() -> Result<(), Box<dyn std::error::Error>> {\n"
                    f"{code}\n"
                    "Ok(())\n"
                    "}\n"
                )

            functions.append(f"mod {label} {{\n{body}\n}}\n")

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
        ["cargo", "test", "--features", "parquet iceberg object", "--test", "docs_examples"],
        cwd=ROOT,
        check=False,
    )
    return result.returncode


class Pool:
    """Long-lived workers, each running one block at a time.

    A scripting-language block spends milliseconds on what it demonstrates and
    most of a second on the import behind it, so what a run costs is almost
    entirely how many times that import is paid. One process per block paid it
    908 times; a worker pays it once and then executes block after block in a
    fresh namespace, which is what the `docs_worker` scripts beside this one
    are. The process boundary is kept - a block that segfaults the extension
    takes down one worker, and the parent reports it and starts another.
    """

    def __init__(self, language: str, workspace: pathlib.Path, size: int) -> None:
        self.language = language
        self.workspace = workspace
        self.size = size
        if language == "python":
            self.command = [str(PYTHON), str(WORKERS["python"])]
            self.warm = ["pyarrow", "yggdryl"]
        else:
            self.command = ["node", str(WORKERS["javascript"])]
            self.warm = [NODE_BINDING, NODE_ARROW]

    def _spawn(self, slot: int) -> tuple[subprocess.Popen[str], pathlib.Path]:
        capture = self.workspace / f"capture-{self.language}-{slot}"
        capture.write_bytes(b"")
        # The Python worker points its own descriptors at the capture, so it
        # is named on the command line; the Node one is given the file as its
        # stderr, which is where a worker the runtime killed says why.
        arguments = [] if self.language == "javascript" else [str(capture)]
        with capture.open("wb") as sink:
            return (
                subprocess.Popen(  # noqa: S603 - a script of this repository
                    [*self.command, *arguments, *self.warm],
                    cwd=ROOT,
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=sink,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                    bufsize=1,
                ),
                capture,
            )

    def run(self, scripts: list[pathlib.Path]) -> list[str | None]:
        """Return one answer per script: ``None`` for a pass, else the detail."""
        answers: list[str | None] = [None] * len(scripts)
        queue = collections.deque(range(len(scripts)))
        lock = threading.Lock()

        def drain(slot: int) -> None:
            process, capture = self._spawn(slot)
            try:
                while True:
                    with lock:
                        if not queue:
                            return
                        index = queue.popleft()
                    answered, detail = self._ask(process, scripts[index])
                    if not answered:
                        # The worker died on this block - the capture is what
                        # it managed to say - so the next one starts fresh.
                        died = capture.read_text(encoding="utf-8", errors="replace").strip()
                        answers[index] = died or "the worker exited without answering"
                        process.kill()
                        process.wait()
                        process, capture = self._spawn(slot)
                    else:
                        answers[index] = detail
            finally:
                self._close(process)

        width = min(self.size, max(1, len(scripts)))
        threads = [threading.Thread(target=drain, args=(slot,)) for slot in range(width)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        return answers

    @staticmethod
    def _ask(process: subprocess.Popen[str], script: pathlib.Path) -> tuple[bool, str | None]:
        """Run one block.

        Returns whether the worker answered at all, and - when it did - the
        failure detail, or ``None`` for a block that passed. The two are
        separate because a worker that died and a block that passed both have
        nothing to report, and only one of them means run it again.
        """
        try:
            process.stdin.write(json.dumps({"path": str(script)}) + "\n")
            process.stdin.flush()
            line = process.stdout.readline()
        except (BrokenPipeError, OSError, ValueError):
            return False, None
        if not line.strip():
            return False, None
        answer = json.loads(line)
        return True, None if answer["ok"] else answer.get("detail", "")

    @staticmethod
    def _close(process: subprocess.Popen[str]) -> None:
        try:
            if process.stdin is not None:
                process.stdin.close()
            process.wait(timeout=10)
        except Exception:  # noqa: BLE001 - a worker that will not go is killed
            process.kill()


def run_scripts(pages, language: str, jobs: int) -> tuple[int, int, list[str]]:
    """Run every block of one scripting language, returning counts and failures.

    Each block writes its own file under the workspace and reads nothing
    another block writes, so the only shared state is the temporary directory
    and whatever an import left warm in the worker running it.
    """
    if language == "python" and not PYTHON.exists():
        return 0, 0, [f"{language}: no interpreter at {PYTHON}"]

    skipped = 0
    failures: list[str] = []

    with tempfile.TemporaryDirectory() as directory:
        workspace = pathlib.Path(directory)
        pending: list[tuple[pathlib.Path, Block, pathlib.Path]] = []
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
                else:
                    script = workspace / f"{label}.js"
                    rewired = block.code
                    for name, target in (
                        ("yggdryl", NODE_BINDING),
                        ("apache-arrow", NODE_ARROW),
                    ):
                        rewired = rewired.replace(f"'{name}'", f"'{target}'").replace(
                            f'"{name}"', f'"{target}"'
                        )
                    script.write_text(rewired, encoding="utf-8")
                pending.append((page, block, script))

        # Answers come back by index, so failures stay in page order however
        # the workers happen to finish.
        answers = Pool(language, workspace, jobs).run([script for _, _, script in pending])
        for (page, block, _), detail in zip(pending, answers):
            if detail is None:
                continue
            tail = detail.strip().splitlines()
            failures.append(
                f"{page.relative_to(ROOT)} {language} block {block.index}:\n      "
                + "\n      ".join(tail[-6:])
            )

    return len(pending), skipped, failures


def main() -> int:
    """Run every documentation example."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lang", choices=[*LANGUAGES, "all"], default="all")
    parser.add_argument("--keep", action="store_true", help="keep the generated Rust target")
    parser.add_argument(
        "--jobs",
        type=int,
        default=min(32, (os.cpu_count() or 4)),
        help="scripting-language blocks to run at once (default: one per core)",
    )
    arguments = parser.parse_args()

    pages = sorted(DOCS.rglob("*.md"))
    status = 0

    fences = [problem for page in pages for problem in unhighlighted(page)]
    print(f"fences: {len(fences)} that would not highlight, from {len(pages)} pages")
    for problem in fences:
        print(f"  {problem}")
    if fences:
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
        ran, skipped, failures = run_scripts(pages, language, max(1, arguments.jobs))
        print(
            f"{language}: {ran} examples run on {max(1, arguments.jobs)} workers, "
            f"{skipped} skipped, {len(failures)} failed"
        )
        for failure in failures:
            print(f"  {failure}")
        if failures:
            status |= 1

    return status


if __name__ == "__main__":
    sys.exit(main())
