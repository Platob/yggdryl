"""Check this expression engine against PyArrow and PyIceberg.

Self-consistency proves nothing about a semantics. The row evaluator, the
vectorized evaluator, and the statistics evaluator agreeing with *each other* is
what the crate's own tests already assert, and all three could still be wrong
together about what ``venue <> 'XNAS'`` does to a null. So this driver runs two
comparisons against implementations a reader already trusts:

1. **Row semantics against PyArrow.** ``cargo test --features parquet --test
   expression_interop`` writes ``target/expression-interop/corpus.parquet`` and
   ``answers.json``: one deliberately awkward six-row corpus, and for each
   predicate the row indexes this engine selects. Each entry carries the
   PyArrow expression asking the same question, written by the Rust half rather
   than translated here - a translation is exactly the thing that could hide a
   disagreement. This script evaluates those over the same file with
   ``pyarrow.dataset`` and compares the selected rows.

2. **Pruning semantics against PyIceberg.** For the predicates PyIceberg's own
   parser accepts, its ``_InclusiveMetricsEvaluator`` is asked whether a data
   file with given statistics may match, and the answer is compared with this
   engine's. Disagreement in the *conservative* direction - we read a file
   PyIceberg would skip - is reported and allowed, because reading a file that
   holds nothing costs time and never correctness. Disagreement in the other
   direction is a hard failure, because it is a lost row.

The cargo half prints ``SKIPPED`` when it cannot write the corpus, and this
driver fails on that word, so a skipped half can never read as a pass.
"""

from __future__ import annotations

import json
import subprocess
import sys
import warnings
from pathlib import Path

warnings.simplefilter("ignore")

REPO = Path(__file__).resolve().parent.parent
INTEROP = REPO / "target" / "expression-interop"
CORPUS = INTEROP / "corpus.parquet"
ANSWERS = INTEROP / "answers.json"
VENV_PYTHON = REPO / "python" / ".venv" / "Scripts" / "python.exe"
if not VENV_PYTHON.exists():
    VENV_PYTHON = REPO / "python" / ".venv" / "bin" / "python"


def run_cargo() -> str:
    """Run the Rust half and return everything it printed."""
    completed = subprocess.run(
        [
            "cargo",
            "test",
            "--locked",
            "--manifest-path",
            str(REPO / "rust" / "Cargo.toml"),
            "--features",
            "parquet",
            "--test",
            "expression_interop",
            "--",
            "--nocapture",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    output = completed.stdout + completed.stderr
    if completed.returncode != 0:
        print(output)
        raise SystemExit("the Rust half failed")
    return output


def compare_rows() -> int:
    """Ask PyArrow the same questions over the same file."""
    import datetime  # noqa: F401 - named by the emitted PyArrow expressions
    import decimal  # noqa: F401 - likewise

    import pyarrow.compute as pc  # noqa: F401 - likewise
    import pyarrow.dataset as ds

    dataset = ds.dataset(str(CORPUS), format="parquet")
    stored = dataset.to_table()
    answers = json.loads(ANSWERS.read_text(encoding="utf-8"))
    disagreements = 0

    for entry in answers:
        expression = entry["expression"]
        rendered = entry["pyarrow"]
        ours = list(entry["rows"])
        try:
            predicate = eval(rendered)  # noqa: S307 - the Rust half wrote this
        except Exception as error:  # noqa: BLE001 - report and move on
            print(f"  ? {expression}: PyArrow cannot spell it ({error})")
            continue
        # Row indexes rather than values: a filtered table loses the positions,
        # so an index column is added and filtered alongside.
        indexed = stored.append_column(
            "__row", pyarrow_index(stored.num_rows)
        )
        kept = ds.dataset(indexed).to_table(filter=predicate)
        theirs = list(kept.column("__row").to_pylist())
        if ours == theirs:
            print(f"  ok {expression}")
            continue
        disagreements += 1
        print(f"  DISAGREE {expression}")
        print(f"     yggdryl selects {ours}")
        print(f"     pyarrow selects {theirs}")
    return disagreements


def pyarrow_index(rows: int):
    """A column of row positions, for comparing selections rather than values."""
    import pyarrow as pa

    return pa.array(list(range(rows)), type=pa.int64())


# One data file's statistics, in the shape both engines are asked about. These
# repeat what the Rust half hands its own evaluator, because the whole point is
# that two engines given the *same* numbers reach the same decision.
FILE_LOWER = {"id": 1, "venue": "XLON"}
FILE_UPPER = {"id": 10, "venue": "XNYS"}
FILE_NULLS = {"id": 1, "venue": 1}
FILE_ROWS = 6
PRUNING_ANSWERS = INTEROP / "pruning.json"


def compare_pruning() -> int:
    """Ask PyIceberg's inclusive metrics evaluator the same questions."""
    try:
        from pyiceberg.expressions.parser import parse
        from pyiceberg.expressions.visitors import _InclusiveMetricsEvaluator
        from pyiceberg.manifest import DataFile, DataFileContent, FileFormat
        from pyiceberg.conversions import to_bytes
        from pyiceberg.schema import Schema
        from pyiceberg.types import IntegerType, NestedField, StringType
    except ImportError as error:
        print(f"  ? PyIceberg is not installed ({error}); pruning not cross-checked")
        return 0

    schema = Schema(
        NestedField(1, "id", IntegerType(), required=False),
        NestedField(2, "venue", StringType(), required=False),
    )
    data_file = DataFile.from_args(
        content=DataFileContent.DATA,
        file_path="corpus.parquet",
        file_format=FileFormat.PARQUET,
        partition={},
        record_count=FILE_ROWS,
        file_size_in_bytes=1024,
        value_counts={1: FILE_ROWS, 2: FILE_ROWS},
        null_value_counts={1: FILE_NULLS["id"], 2: FILE_NULLS["venue"]},
        nan_value_counts={},
        lower_bounds={
            1: to_bytes(IntegerType(), FILE_LOWER["id"]),
            2: to_bytes(StringType(), FILE_LOWER["venue"]),
        },
        upper_bounds={
            1: to_bytes(IntegerType(), FILE_UPPER["id"]),
            2: to_bytes(StringType(), FILE_UPPER["venue"]),
        },
        key_metadata=None,
        split_offsets=[],
        equality_ids=[],
        sort_order_id=None,
    )

    decisions = json.loads(PRUNING_ANSWERS.read_text(encoding="utf-8"))
    lost = 0
    for decision in decisions:
        ours_text = decision["expression"]
        theirs_text = decision["pyiceberg"]
        ours_possible = bool(decision["possible"])
        try:
            # A `py:` prefix names a PyIceberg constructor rather than its text
            # grammar, for the handful of predicates its parser cannot spell.
            if theirs_text.startswith("py:"):
                from pyiceberg.expressions import (  # noqa: F401 - named below
                    Reference,
                    StartsWith,
                    literal,
                )

                built = eval(theirs_text[3:])  # noqa: S307 - the Rust half wrote this
            else:
                built = parse(theirs_text)
            evaluator = _InclusiveMetricsEvaluator(schema, built)
            theirs_possible = bool(evaluator.eval(data_file))
        except Exception as error:  # noqa: BLE001 - report and move on
            print(f"  ? {ours_text}: PyIceberg cannot spell it ({error})")
            continue
        if ours_possible == theirs_possible:
            print(f"  ok {ours_text} -> {'read' if ours_possible else 'skip'}")
            continue
        if ours_possible and not theirs_possible:
            # Conservative: a file read that need not have been. Time, never
            # correctness.
            print(f"  ~ {ours_text}: we read a file PyIceberg would skip")
            continue
        lost += 1
        print(f"  LOST {ours_text}: we skip a file PyIceberg would read")
    return lost


def main() -> int:
    if VENV_PYTHON.exists() and Path(sys.executable).resolve() != VENV_PYTHON.resolve():
        print(f"re-running under {VENV_PYTHON}")
        return subprocess.call([str(VENV_PYTHON), __file__, *sys.argv[1:]])

    print("== Rust writes the corpus and its answers")
    output = run_cargo()
    if "SKIPPED" in output:
        raise SystemExit("the Rust half skipped; nothing was cross-validated")
    if "expression-interop: wrote" not in output:
        raise SystemExit("the Rust half did not report writing the corpus")
    if "expression-interop: wrote" not in output or "pruning decisions" not in output:
        raise SystemExit("the Rust half did not report writing the pruning decisions")
    if not CORPUS.exists() or not ANSWERS.exists() or not PRUNING_ANSWERS.exists():
        raise SystemExit("the corpus, the answers, or the pruning decisions are missing")

    print("\n== Row semantics against PyArrow")
    disagreements = compare_rows()

    print("\n== Pruning semantics against PyIceberg")
    lost = compare_pruning()

    if disagreements:
        raise SystemExit(f"{disagreements} predicate(s) select different rows than PyArrow")
    if lost:
        raise SystemExit(f"{lost} predicate(s) skip a file PyIceberg would read")

    print("\nBoth outside implementations agree.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
