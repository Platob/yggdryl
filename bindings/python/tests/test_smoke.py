"""Smoke test for the yggdryl Python binding scaffold."""

import yggdryl


def test_version():
    assert isinstance(yggdryl.version(), str)
    assert yggdryl.version()
