"""The static enum vocabularies: canonical spellings, unpacked from the core."""

from __future__ import annotations

from yggdryl import DataType, enums


def test_every_vocabulary_is_a_nonempty_tuple_of_strings() -> None:
    for listing in (
        enums.DATA_TYPE_IDS,
        enums.DATA_TYPE_KINDS,
        enums.TIME_UNITS,
        enums.UNION_MODES,
        enums.WRITE_MODES,
        enums.CODECS,
        enums.IO_KINDS,
        enums.COMPATIBILITY_SCHEMES,
    ):
        assert isinstance(listing, tuple)
        assert listing
        assert all(isinstance(value, str) and value for value in listing)


def test_the_spellings_are_the_ones_the_parsers_accept() -> None:
    # Every listed id parses as a datatype or is a parameterized family the
    # grammar spells with arguments; the simple ones round-trip exactly.
    assert "int64" in enums.DATA_TYPE_IDS
    assert DataType("int64").id == "int64"
    assert "integer" in enums.DATA_TYPE_KINDS
    assert enums.UNION_MODES == ("sparse", "dense")
    assert enums.WRITE_MODES == ("overwrite", "append", "merge")
    assert "us" in enums.TIME_UNITS
    assert "gzip" in enums.CODECS
    assert "file" in enums.IO_KINDS
    assert "arrow" in enums.COMPATIBILITY_SCHEMES


def test_the_level_scale_names_its_points() -> None:
    assert enums.LEVELS["none"] == 0
    assert enums.LEVELS["fast"] == 1
    assert enums.LEVELS["default"] == 6
    assert enums.LEVELS["best"] == 9
