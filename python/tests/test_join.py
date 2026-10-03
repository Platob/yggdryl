"""``python/src/join.rs``: the facts beside a join's keys and kind, and the one
door every ``join_with`` reads its arguments through.

Mirrors the vocabulary and option cases of ``rust/tests/root/join.rs``: the
kinds read every word the core reads, the keys every spelling one ``Scalar``
takes, and ``options`` - ``JoinOptions()`` for ``None`` - takes each keyword
given on a copy, ``...`` being a keyword not given and ``None`` a value that
clears. What each kind answers is pinned where the verbs are, in
``test_serie.py`` (the held serie and the reader) and
``test_chunked_serie.py``.
"""

from __future__ import annotations

import copy
import inspect
import pathlib
import pickle

import pytest

from yggdryl import Field, JoinOptions, Serie, SpillOptions, StructSerie


def trades() -> Serie:
    """`(id, name)`: a key repeated twice, a null key, a key the right lacks."""
    return Serie.from_scalars(
        Field("trade", "struct<id: int64, name: utf8 not null>", nullable=False),
        [[1, "a"], [2, "b"], [2, "b2"], [None, "n"], [4, "d"]],
    )


def values() -> Serie:
    """`(id, value)`: a key repeated twice, a null key, a key the left lacks,
    the matched key last."""
    return Serie.from_scalars(
        Field("value", "struct<id: int64, value: int64 not null>", nullable=False),
        [[2, 20], [2, 21], [3, 30], [None, 99], [1, 10]],
    )


def names(serie: Serie) -> list[str]:
    assert isinstance(serie, StructSerie)
    return serie.names


class TestOptions:
    def test_the_options_default_as_the_core_says_and_each_keyword_moves_one_fact(
        self,
    ) -> None:
        options = JoinOptions()
        assert options.coalesce
        assert options.suffix == "_right"
        assert options.build is None
        assert options.prune
        assert options.spill is None
        assert options.pushdown_keys == 10_000
        # The signature states the core's defaults, which the options carry.
        for name, parameter in inspect.signature(JoinOptions).parameters.items():
            assert parameter.default == getattr(options, name), name
        moved = JoinOptions(
            coalesce=False,
            suffix="_r",
            build="LEFT",
            prune=False,
            spill=SpillOptions(SpillOptions.NEVER),
            pushdown_keys=7,
        )
        assert not moved.coalesce
        assert moved.suffix == "_r"
        assert moved.build == "left"
        assert not moved.prune
        assert moved.spill == SpillOptions(SpillOptions.NEVER)
        assert moved.pushdown_keys == 7
        assert moved != options
        assert JoinOptions(build="right").build == "right"

    def test_a_build_side_names_left_or_right_and_nothing_else(self) -> None:
        with pytest.raises(ValueError, match=r"\$\.build.*`left` or `right`.*middle"):
            JoinOptions(build="middle")
        with pytest.raises(OverflowError):
            JoinOptions(pushdown_keys=-1)

    def test_options_are_immutable_and_copy_pickle_hash_and_print_as_themselves(
        self, tmp_path: pathlib.Path
    ) -> None:
        for options in (
            JoinOptions(),
            JoinOptions(False, "_r", "right", False, SpillOptions(0, tmp_path), 3),
        ):
            assert pickle.loads(pickle.dumps(options)) == options
            assert copy.copy(options) == options
            assert copy.deepcopy(options) == options
            assert hash(copy.copy(options)) == hash(options)
        assert JoinOptions() == JoinOptions()
        assert JoinOptions() != JoinOptions(spill=SpillOptions())
        assert JoinOptions() != "inner"
        assert repr(JoinOptions(build="left")) == (
            'JoinOptions(coalesce=True, suffix="_right", build="left", prune=True, '
            "spill=None, pushdown_keys=10000)"
        )
        assert "spill=SpillOptions(byte_size=0)" in repr(JoinOptions(spill=SpillOptions(0)))
        with pytest.raises(AttributeError):
            JoinOptions().suffix = "_l"  # type: ignore[misc]


class TestDoor:
    """The kind, the keys and the options every ``join_with`` reads, here
    through ``Serie.join_with``."""

    def test_every_kind_reads_its_words(self) -> None:
        for spelling, canonical in (
            ("inner", "inner"),
            ("INNER JOIN", "inner"),
            ("left outer", "left"),
            ("Left Outer Join", "left"),
            ("right", "right"),
            ("outer", "full"),
            ("full outer join", "full"),
            ("semi", "semi"),
            ("anti", "anti"),
        ):
            assert trades().join_with(values(), "id", spelling, build="right") == (
                trades().join_with(values(), "id", canonical, build="right")
            ), spelling
        with pytest.raises(ValueError, match=r"\$\.how.*cross"):
            trades().join_with(values(), "id", "cross")

    def test_the_keys_read_every_spelling_one_scalar_takes(self) -> None:
        expected = trades().join_with(values(), "id", build="right")
        assert len(expected) == 5
        for by in ("id", ["id"], "id = id", [["id", "id"]], {"id": "id"}, ("id",)):
            assert trades().join_with(values(), by, build="right") == expected, by
        # Two keys, each a column of both sides; an absent key matches
        # nothing.
        both = trades().join_with(trades(), "id, name", build="right")
        assert names(both) == ["id", "name"]
        assert len(both) == 4

    def test_a_key_list_that_names_nothing_or_is_no_key_is_refused_by_name(self) -> None:
        with pytest.raises(ValueError, match="empty key list"):
            trades().join_with(values(), [])
        with pytest.raises(ValueError, match="join key list"):
            trades().join_with(values(), 5)  # type: ignore[arg-type]
        with pytest.raises(ValueError, match="venue"):
            trades().join_with(values(), "venue")
        with pytest.raises(ValueError, match="an equality"):
            trades().join_with(values(), "id < id")

    def test_a_keyword_is_set_on_a_copy_of_the_options_and_wins(self) -> None:
        options = JoinOptions(coalesce=False)
        kept = trades().join_with(values(), "id", options=options, build="right")
        assert names(kept) == ["id", "name", "id_right", "value"]
        suffixed = trades().join_with(values(), "id", "inner", options, suffix="_r")
        assert names(suffixed) == ["id", "name", "id_r", "value"]
        coalesced = trades().join_with(values(), "id", options=options, coalesce=True)
        assert names(coalesced) == ["id", "name", "value"]
        # The options object is as it was.
        assert options == JoinOptions(coalesce=False)

    def test_none_clears_a_stated_build_side_and_a_stated_bound(
        self, tmp_path: pathlib.Path
    ) -> None:
        # The trades weigh less, so they are built and the values probe: the
        # rows come in the values' order unless the right side is built.
        right = JoinOptions(build="right")
        assert [row["value"] for row in trades().join_with(values(), "id", options=right).as_py()] == [
            10,
            20,
            21,
            20,
            21,
        ]
        picked = trades().join_with(values(), "id", options=right, build=None)
        assert [row["value"] for row in picked.as_py()] == [20, 20, 21, 21, 10]
        # Every output batch settles under the stated bound: under a folder
        # that is not there it is refused naming the folder, and `None`
        # settles under the process default instead.
        missing = JoinOptions(spill=SpillOptions(0, tmp_path / "missing"))
        with pytest.raises(ValueError, match="missing"):
            trades().join_with(values(), "id", options=missing)
        settled = trades().join_with(values(), "id", options=missing, spill=None)
        assert not settled.is_spilled()
        spilled = trades().join_with(values(), "id", spill=SpillOptions(0))
        assert spilled.is_spilled() and spilled == settled

    def test_a_keyword_of_the_wrong_kind_is_refused(self) -> None:
        with pytest.raises(TypeError):
            trades().join_with(values(), "id", suffix=1)  # type: ignore[arg-type]
        with pytest.raises(TypeError):
            trades().join_with(values(), "id", spill=0)  # type: ignore[arg-type]
        with pytest.raises(TypeError):
            trades().join_with(values(), "id", options="left")  # type: ignore[arg-type]
        with pytest.raises(TypeError):
            trades().join_with(values(), "id", tier=1)  # type: ignore[call-arg]
