"""Tests for the ``yggdryl.io`` ``IOMode`` enum.

Mirrors ``crates/yggdryl-core/src/io/mode.rs`` on the Python surface: the wire-stable
numeric values, the generic type-inferring ``parse`` (str name or int value), the
``is_readable`` / ``is_writable`` predicates, and the canonical names.
"""

import pytest

import yggdryl.io
from yggdryl.io import IOMode

ALL_MODES = [IOMode.Read, IOMode.Write, IOMode.ReadWrite, IOMode.Append, IOMode.Overwrite]


def test_module_surface():
    assert IOMode.__module__ == "yggdryl.io"
    assert hasattr(yggdryl.io, "IOMode")


def test_wire_stable_values():
    assert IOMode.Read == 1
    assert IOMode.Write == 2
    assert IOMode.ReadWrite == 3
    assert IOMode.Append == 4
    assert IOMode.Overwrite == 5
    assert int(IOMode.Append) == 4
    assert [m.to_u8() for m in ALL_MODES] == [1, 2, 3, 4, 5]


def test_parse_dispatches_on_type():
    # str -> the core parse_str (canonical name or short alias, case-insensitive).
    assert IOMode.parse("rw") == IOMode.ReadWrite
    assert IOMode.parse("READ") == IOMode.Read
    assert IOMode.parse("+") == IOMode.ReadWrite
    assert IOMode.parse("truncate") == IOMode.Overwrite
    # int -> the core from_u8.
    assert IOMode.parse(4) == IOMode.Append
    assert IOMode.parse(1) == IOMode.Read


def test_parse_round_trips_names_and_values():
    for mode in ALL_MODES:
        assert IOMode.parse(mode.name()) == mode
        assert IOMode.parse(mode.to_u8()) == mode


def test_parse_errors_are_guided():
    with pytest.raises(ValueError, match="IOMode") as excinfo:
        IOMode.parse("bogus")
    assert "bogus" in str(excinfo.value)
    assert "read_write" in str(excinfo.value)  # the accepted tokens are listed

    with pytest.raises(ValueError, match="IOMode"):
        IOMode.parse(9)  # outside 1..=5
    with pytest.raises(ValueError, match="IOMode"):
        IOMode.parse(None)  # neither str nor int


def test_parse_out_of_range_int_has_exact_core_text():
    # An int the u8 arm cannot hold still reads exactly like the core from_u8 error,
    # so Python and Node raise identically.
    for bad in (999, -1):
        with pytest.raises(ValueError) as excinfo:
            IOMode.parse(bad)
        message = str(excinfo.value)
        assert f'unknown IOMode "{bad}"' in message
        assert (
            "expected one of 1 (read), 2 (write), 3 (read_write), 4 (append), "
            "5 (overwrite)" in message
        )


def test_is_readable_and_is_writable():
    assert IOMode.Read.is_readable()
    assert IOMode.ReadWrite.is_readable()
    assert not IOMode.Write.is_readable()
    assert not IOMode.Append.is_readable()
    assert not IOMode.Overwrite.is_readable()

    assert not IOMode.Read.is_writable()
    for mode in (IOMode.Write, IOMode.ReadWrite, IOMode.Append, IOMode.Overwrite):
        assert mode.is_writable()


def test_names_and_str():
    assert IOMode.ReadWrite.name() == "read_write"
    assert str(IOMode.Overwrite) == "overwrite"
    assert [str(m) for m in ALL_MODES] == [
        "read",
        "write",
        "read_write",
        "append",
        "overwrite",
    ]


def test_hashable_and_frozen():
    assert {IOMode.Read, IOMode.Read, IOMode.Write} == {IOMode.Read, IOMode.Write}
    lookup = {IOMode.Append: "a"}
    assert lookup[IOMode.parse("a")] == "a"  # equal values hash equal
