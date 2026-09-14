"""How much a user-defined function costs beside the grammar's own terms.

Run after ``maturin develop`` with::

    python benchmarks/udf.py --rows 200000 --repeat 5

Three spellings of one projection over the same batch are timed: the grammar
term ``size * 2``, a row-wise Python function registered with
``@user_defined_function``, and the same function registered ``vectorized``,
which takes the whole column as a pyarrow array. The row-wise path measures
the one interpreter attachment per batch and one scalar crossing per value;
the vectorized path measures one crossing per column. A filter over the same
function is timed the same way, because a predicate runs the same evaluators.
"""

from __future__ import annotations

import argparse
import statistics
import timeit

import pyarrow as pa
import pyarrow.compute as pc

from yggdryl import Filter, Selector
from yggdryl.expression import user_defined_filter, user_defined_function


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--rows", type=int, default=100_000, help="rows in the batch")
    parser.add_argument("--repeat", type=int, default=3, help="timed repetitions")
    return parser.parse_args()


def batch(rows: int) -> pa.RecordBatch:
    return pa.record_batch({"size": pa.array(range(rows), pa.int64())})


def timed(label: str, run, repeat: int) -> None:
    samples = [timeit.timeit(run, number=1) for _ in range(repeat)]
    print(f"{label:<40} {statistics.median(samples) * 1e3:10.2f} ms")


def main() -> None:
    args = parse_args()
    rows = batch(args.rows)

    @user_defined_function(namespace="bench")
    def double(value: int) -> int:
        return value * 2

    @user_defined_function(namespace="bench", name="double_vectorized", vectorized=True)
    def double_vectorized(value: int) -> int:
        return pc.multiply(value, 2)

    @user_defined_filter(namespace="bench")
    def even(value: int) -> bool:
        return value % 2 == 0

    projections = {
        "select size * 2": Selector("size * 2 as doubled"),
        "select bench.double(size) row-wise": Selector("bench.double(size) as doubled"),
        "select bench.double_vectorized(size)": Selector("bench.double_vectorized(size) as doubled"),
    }
    for label, selector in projections.items():
        timed(label, lambda selector=selector: selector.apply_arrow_batch(rows), args.repeat)
    filters = {
        "where size % 2 = 0": Filter("size % 2 = 0"),
        "where bench.even(size) row-wise": Filter("bench.even(size)"),
    }
    for label, predicate in filters.items():
        timed(label, lambda predicate=predicate: predicate.apply_arrow_batch(rows), args.repeat)
    for function in (double, double_vectorized, even):
        function.unregister()


if __name__ == "__main__":
    main()
