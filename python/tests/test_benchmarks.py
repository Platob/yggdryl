"""Every benchmark script runs, and none is named after what it imports.

A script's own directory comes first on ``sys.path``, so a benchmark named
after a module resolves that name to itself. Both failures this guards against
had already happened and neither was visible: ``types.py`` shadowed the
standard library's ``types`` and broke seven sibling scripts outright, and
``xxhash.py`` shadowed the C package it measured against, so the comparison
silently ran yggdryl against yggdryl.
"""

from __future__ import annotations

import ast
import concurrent.futures
import pathlib
import subprocess
import sys

import pytest

BENCHMARKS = pathlib.Path(__file__).resolve().parent.parent / "benchmarks"
SCRIPTS = sorted(
    path
    for path in BENCHMARKS.rglob("*.py")
    if "__pycache__" not in path.parts and not path.name.startswith("_")
)


def _runs(script: pathlib.Path) -> tuple[pathlib.Path, int, str]:
    finished = subprocess.run(
        [sys.executable, str(script), "--help"],
        capture_output=True,
        text=True,
        timeout=180,
        check=False,
    )
    return script, finished.returncode, finished.stderr


def test_the_benchmark_directory_is_not_empty() -> None:
    # The two checks below pass vacuously if the glob ever stops matching.
    assert len(SCRIPTS) >= 10


def test_every_benchmark_script_runs() -> None:
    # One process each, because the failure being guarded against happens at
    # import time and only when the script's own directory leads `sys.path`.
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(_runs, SCRIPTS))

    broken = [
        f"{script.relative_to(BENCHMARKS)}: exit {code}\n{stderr.strip()[-500:]}"
        for script, code, stderr in results
        if code != 0
    ]
    assert not broken, "benchmark scripts that do not run:\n\n" + "\n\n".join(broken)


@pytest.mark.parametrize("script", SCRIPTS, ids=lambda path: path.stem)
def test_no_benchmark_shadows_a_module_it_imports(script: pathlib.Path) -> None:
    imported: set[str] = set()
    for node in ast.walk(ast.parse(script.read_text(encoding="utf-8"))):
        if isinstance(node, ast.Import):
            imported.update(alias.name.split(".")[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            imported.add(node.module.split(".")[0])

    assert script.stem not in imported, (
        f"{script.relative_to(BENCHMARKS)} imports {script.stem!r}, which is its "
        "own name: the script's directory leads `sys.path`, so it imports itself "
        "instead of the package it means to measure"
    )
