"""The Python extension mirrors the core `version()` — the minimal end-to-end example."""

import yggdryl


def test_version_is_a_nonempty_string():
    v = yggdryl.version()
    assert isinstance(v, str)
    assert v != ""


def test_version_matches_the_package():
    # The core reports its Cargo version; it matches the packaged 0.1.x line.
    assert yggdryl.version().startswith("0.1.")
