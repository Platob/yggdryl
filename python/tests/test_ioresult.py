"""``python/src/ioresult.rs``: the rows one record write read, wrote and
skipped, every number the core's. What each write door answers is pinned in
``test_iobase.py``.
"""

from __future__ import annotations

import copy
import inspect
import pickle

import pytest

from yggdryl import IOResult

# A sum keeps each side's skipped rows, which `IOResult(6, 6)` would not derive.
SUMMED = IOResult(2, 6) + IOResult(4, 0)


@pytest.mark.parametrize(
    ("result", "counts"),
    [
        (IOResult(10, 8), (10, 8, 2)),
        (IOResult(read_rows=10, written_rows=8, skipped_rows=None), (10, 8, 2)),
        # A `select` that unnests writes more than it read: none skipped.
        (IOResult(2, 6), (2, 6, 0)),
        # Every row kept out: read, so not empty.
        (IOResult(3, 0), (3, 0, 3)),
        (IOResult(), (0, 0, 0)),
        (IOResult(6, 6, 4), (6, 6, 4)),
        (SUMMED, (6, 6, 4)),
    ],
    ids=["read-and-written", "keywords", "unnested", "all-skipped", "default", "stated", "sum"],
)
def test_the_three_counts_round_trip(result: IOResult, counts: tuple[int, int, int]) -> None:
    assert (result.read_rows, result.written_rows, result.skipped_rows) == counts
    assert result.is_empty() == (counts == (0, 0, 0))
    assert repr(result) == "IOResult(read_rows={}, written_rows={}, skipped_rows={})".format(*counts)
    assert eval(repr(result)) == result
    assert pickle.loads(pickle.dumps(result)) == result
    assert copy.copy(result) == result and copy.deepcopy(result) == result


def test_the_defaults_are_in_the_signature() -> None:
    defaults = {
        name: parameter.default for name, parameter in inspect.signature(IOResult).parameters.items()
    }
    assert defaults == {"read_rows": 0, "written_rows": 0, "skipped_rows": None}


def test_the_counts_are_read_only_and_never_negative() -> None:
    with pytest.raises(AttributeError):
        IOResult(1, 1).read_rows = 2  # type: ignore[misc]
    with pytest.raises(OverflowError):
        IOResult(-1, 0)


def test_results_add_count_by_count() -> None:
    assert IOResult(10, 8) + IOResult(5, 5) + IOResult(1, 0) == IOResult(16, 13)
    assert sum([IOResult(1, 1), IOResult(2, 1), IOResult(4, 4)], IOResult()) == IOResult(7, 6)
    assert SUMMED != IOResult(6, 6)
    with pytest.raises(TypeError):
        IOResult(1, 1) + 1  # type: ignore[operator]


def test_a_result_displays_as_its_three_counts() -> None:
    assert str(IOResult(10, 8)) == "read 10 rows, wrote 8, skipped 2"


def test_equality_order_and_hash_read_the_three_counts() -> None:
    result = IOResult(10, 8)
    assert result != IOResult(10, 9)
    assert result != (10, 8, 2)
    # Ordered by the read rows, then the written, then the skipped.
    assert IOResult(1, 1) < IOResult(2, 0) < IOResult(2, 1) < IOResult(2, 2)
    assert IOResult(2, 2) <= IOResult(2, 2) <= IOResult(3, 0)
    assert SUMMED > IOResult(6, 6)
    assert sorted([IOResult(3, 0), IOResult(1, 1), IOResult(2, 2)]) == [
        IOResult(1, 1),
        IOResult(2, 2),
        IOResult(3, 0),
    ]
    assert len({result, IOResult(10, 8), IOResult(10, 9)}) == 2
    assert result.stable_hash() == IOResult(10, 8).stable_hash() != IOResult(10, 9).stable_hash()
