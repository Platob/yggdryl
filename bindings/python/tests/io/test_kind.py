"""Tests for the ``yggdryl.io`` ``IOKind`` enum.

Mirrors ``crates/yggdryl-core/src/io/kind.rs`` on the Python surface: the wire-stable
numeric values, the generic type-inferring ``parse`` (str name or int value), the
``exists`` predicate, and the canonical names.
"""

import pytest

import yggdryl.io
from yggdryl.io import IOKind

ALL_KINDS = [IOKind.Missing, IOKind.File, IOKind.Directory, IOKind.Heap]


def test_module_surface():
    assert IOKind.__module__ == "yggdryl.io"
    assert hasattr(yggdryl.io, "IOKind")


def test_wire_stable_values():
    assert IOKind.Missing == 0
    assert IOKind.File == 1
    assert IOKind.Directory == 2
    assert IOKind.Heap == 3
    assert int(IOKind.Directory) == 2
    assert [k.to_u8() for k in ALL_KINDS] == [0, 1, 2, 3]


def test_parse_dispatches_on_type():
    # str -> the core parse_str (canonical name or alias, case-insensitive).
    assert IOKind.parse("heap") == IOKind.Heap
    assert IOKind.parse("DIR") == IOKind.Directory
    assert IOKind.parse("directory") == IOKind.Directory
    assert IOKind.parse("missing") == IOKind.Missing
    # int -> the core from_u8.
    assert IOKind.parse(0) == IOKind.Missing
    assert IOKind.parse(2) == IOKind.Directory


def test_parse_round_trips_names_and_values():
    for kind in ALL_KINDS:
        assert IOKind.parse(kind.name()) == kind
        assert IOKind.parse(kind.to_u8()) == kind


def test_parse_errors_are_guided():
    with pytest.raises(ValueError, match="IOKind") as excinfo:
        IOKind.parse("bogus")
    assert "bogus" in str(excinfo.value)
    assert "directory" in str(excinfo.value)  # the accepted tokens are listed

    with pytest.raises(ValueError, match="IOKind"):
        IOKind.parse(9)  # outside 0..=3
    with pytest.raises(ValueError, match="IOKind"):
        IOKind.parse(None)  # neither str nor int


def test_parse_out_of_range_int_has_exact_core_text():
    # An int the u8 arm cannot hold still reads exactly like the core from_u8 error,
    # so Python and Node raise identically.
    for bad in (999, -1):
        with pytest.raises(ValueError) as excinfo:
            IOKind.parse(bad)
        message = str(excinfo.value)
        assert f'unknown IOKind "{bad}"' in message
        assert "expected one of 0 (missing), 1 (file), 2 (directory), 3 (heap)" in message


def test_exists():
    assert not IOKind.Missing.exists()
    assert IOKind.File.exists()
    assert IOKind.Directory.exists()
    assert IOKind.Heap.exists()


def test_names_and_str():
    assert IOKind.Directory.name() == "directory"
    assert str(IOKind.Missing) == "missing"
    assert [str(k) for k in ALL_KINDS] == ["missing", "file", "directory", "heap"]


def test_hashable_and_frozen():
    assert {IOKind.Heap, IOKind.Heap, IOKind.File} == {IOKind.Heap, IOKind.File}
    lookup = {IOKind.Heap: "h"}
    assert lookup[IOKind.parse(3)] == "h"  # equal values hash equal
