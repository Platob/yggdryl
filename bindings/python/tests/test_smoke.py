"""Smoke test for the yggdryl Python binding."""

import yggdryl
from yggdryl import core


def test_core_version():
    assert isinstance(core.version(), str)
    assert core.version()


def test_core_hello(capfd):
    assert core.hello() is None
    out, _ = capfd.readouterr()
    assert "Hello from yggdryl" in out
