"""Smoke test for the yggdryl Python binding."""

import yggdryl
from yggdryl import core
from yggdryl.core import Whence


def test_core_version():
    assert isinstance(core.version(), str)
    assert core.version()


def test_core_hello(capfd):
    assert core.hello() is None
    out, _ = capfd.readouterr()
    assert "Hello from yggdryl" in out


def test_core_whence():
    assert core.Whence is Whence
    assert int(Whence.Start) == 0 and int(Whence.End) == 2
    assert Whence.Start != Whence.End
    # hashable → usable as a dict key
    assert {Whence.Start: 1}[Whence.Start] == 1
