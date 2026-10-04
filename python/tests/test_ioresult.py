"""``python/src/ioresult.rs``: the rows one record write read, wrote and
skipped.

Mirrors ``rust/tests/root/ioresult.rs``: every number is the core's - the
constructor is ``IOResult::new``, ``+`` its ``Add``, ``str`` its ``Display``.
What each write door answers is pinned where the doors are, in
``test_iobase.py``.
"""

from __future__ import annotations

import copy
import inspect
import pickle

import pytest

from yggdryl import IOResult


class TestValue:
    def test_what_was_read_and_not_written_is_skipped(self) -> None:
        result = IOResult(10, 8)
        assert result.read_rows == 10
        assert result.written_rows == 8
        assert result.skipped_rows == 2
        assert not result.is_empty()
        assert IOResult(read_rows=10, written_rows=8) == result

    def test_a_write_of_more_rows_than_it_read_skips_none(self) -> None:
        # A `select` that unnests writes more rows than its source held, and
        # the skipped count never goes below zero.
        result = IOResult(2, 6)
        assert result.skipped_rows == 0
        assert (result.read_rows, result.written_rows) == (2, 6)

    def test_the_default_is_the_write_of_an_empty_source(self) -> None:
        # The defaults are in the signature: no row read, none written, and
        # the skipped rows derived from the two.
        defaults = {
            name: parameter.default
            for name, parameter in inspect.signature(IOResult).parameters.items()
        }
        assert defaults == {"read_rows": 0, "written_rows": 0, "skipped_rows": None}
        result = IOResult()
        assert (result.read_rows, result.written_rows, result.skipped_rows) == (0, 0, 0)
        assert result == IOResult(0, 0)
        assert result.is_empty()
        # A source whose every row was kept out was read: it is not empty.
        assert not IOResult(3, 0).is_empty()
        assert IOResult(3, 0).skipped_rows == 3

    def test_the_counts_are_read_only_and_never_negative(self) -> None:
        result = IOResult(1, 1)
        with pytest.raises(AttributeError):
            result.read_rows = 2  # type: ignore[misc]
        with pytest.raises(OverflowError):
            IOResult(-1, 0)

    def test_results_add_count_by_count(self) -> None:
        total = IOResult(10, 8) + IOResult(5, 5)
        assert total == IOResult(15, 13)
        total = total + IOResult(1, 0)
        assert (total.read_rows, total.written_rows, total.skipped_rows) == (16, 13, 3)
        # Summed from the empty result, as a write cut into commits answers.
        summed = sum([IOResult(1, 1), IOResult(2, 1), IOResult(4, 4)], IOResult())
        assert summed == IOResult(7, 6)
        # Each side's skipped rows are kept: a sum need not read as `new`.
        unnested = IOResult(2, 6) + IOResult(4, 0)
        assert (unnested.read_rows, unnested.written_rows, unnested.skipped_rows) == (6, 6, 4)
        assert unnested != IOResult(6, 6)
        with pytest.raises(TypeError):
            IOResult(1, 1) + 1  # type: ignore[operator]

    def test_a_result_displays_as_its_three_counts(self) -> None:
        result = IOResult(10, 8)
        assert str(result) == "read 10 rows, wrote 8, skipped 2"
        assert repr(result) == "IOResult(read_rows=10, written_rows=8, skipped_rows=2)"
        assert repr(IOResult()) == "IOResult(read_rows=0, written_rows=0, skipped_rows=0)"

    def test_the_three_counts_stated_rebuild_a_result_as_its_repr_spells_it(self) -> None:
        # A sum's skipped rows are its own, so the repr states all three and
        # the constructor takes them as they are.
        summed = IOResult(2, 6) + IOResult(4, 0)
        assert repr(summed) == "IOResult(read_rows=6, written_rows=6, skipped_rows=4)"
        assert IOResult(6, 6, 4) == summed
        assert IOResult(read_rows=6, written_rows=6, skipped_rows=4) == summed
        assert IOResult(6, 6, 4) != IOResult(6, 6)
        for result in (IOResult(), IOResult(10, 8), summed):
            assert eval(repr(result)) == result
        # Absent, the skipped rows are derived; `None` says the same.
        assert IOResult(10, 8, None) == IOResult(10, 8)


class TestIdentity:
    def test_equality_order_and_hash_read_the_three_counts(self) -> None:
        result = IOResult(10, 8)
        assert result == IOResult(10, 8)
        assert result != IOResult(10, 9)
        assert result != (10, 8, 2)
        # Ordered by the read rows, then the written, then the skipped.
        assert IOResult(1, 1) < IOResult(2, 0)
        assert IOResult(2, 0) < IOResult(2, 1) < IOResult(2, 2)
        # Two sums of one read and written count differ by their skipped.
        assert IOResult(2, 6) + IOResult(4, 0) > IOResult(6, 6)
        assert IOResult(2, 2) <= IOResult(2, 2) <= IOResult(3, 0)
        assert sorted([IOResult(3, 0), IOResult(1, 1), IOResult(2, 2)]) == [
            IOResult(1, 1),
            IOResult(2, 2),
            IOResult(3, 0),
        ]
        assert hash(result) == hash(IOResult(10, 8))
        assert len({result, IOResult(10, 8), IOResult(10, 9)}) == 2
        assert result.stable_hash() == IOResult(10, 8).stable_hash()
        assert result.stable_hash() != IOResult(10, 9).stable_hash()

    def test_pickle_and_copy_keep_the_three_counts_exactly(self) -> None:
        # A sum states skipped rows `IOResult(read, written)` would not.
        for result in (IOResult(), IOResult(10, 8), IOResult(2, 6) + IOResult(4, 0)):
            restored = pickle.loads(pickle.dumps(result))
            assert isinstance(restored, IOResult)
            assert restored == result
            assert restored.skipped_rows == result.skipped_rows
            assert restored.stable_hash() == result.stable_hash()
            assert copy.copy(result) == result
            assert copy.deepcopy(result) == result
        summed = IOResult(2, 6) + IOResult(4, 0)
        assert pickle.loads(pickle.dumps(summed)).skipped_rows == 4
