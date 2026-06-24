"""Tests for the yggdryl Python extension.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import pytest

import yggdryl


def test_uri_components():
    uri = yggdryl.Uri("https://example.com/docs?page=2#intro")
    assert uri.scheme == "https"
    assert uri.authority == "example.com"
    assert uri.path == "/docs"
    assert uri.query == "page=2"
    assert uri.fragment == "intro"


def test_uri_without_authority():
    uri = yggdryl.Uri.parse("mailto:alice@example.com")
    assert uri.scheme == "mailto"
    assert uri.authority is None
    assert uri.path == "alice@example.com"


def test_uri_str_round_trip():
    text = "file:///etc/hosts"
    assert str(yggdryl.Uri(text)) == text


def test_uri_invalid_raises():
    with pytest.raises(ValueError):
        yggdryl.Uri("no-scheme/path")


def test_url_components():
    url = yggdryl.Url("https://user:pw@example.com:8443/api?v=1#top")
    assert url.scheme == "https"
    assert url.username == "user"
    assert url.password == "pw"
    assert url.host == "example.com"
    assert url.port == 8443
    assert url.path == "/api"
    assert url.query == "v=1"
    assert url.fragment == "top"
    assert url.authority == "user:pw@example.com:8443"


def test_url_ipv6():
    url = yggdryl.Url("http://[::1]:8080/status")
    assert url.host == "::1"
    assert url.port == 8080
    assert str(url) == "http://[::1]:8080/status"


def test_url_requires_authority():
    with pytest.raises(ValueError):
        yggdryl.Url("mailto:alice@example.com")


def test_equality_and_hash():
    a = yggdryl.Url("http://example.com")
    b = yggdryl.Url("http://example.com")
    assert a == b
    assert hash(a) == hash(b)
    assert len({a, b}) == 1


def test_repr():
    assert repr(yggdryl.Uri("urn:isbn:0451450523")) == "Uri('urn:isbn:0451450523')"


def test_version_components():
    v = yggdryl.Version.parse("1.4.2")
    assert (v.major, v.minor, v.patch) == (1, 4, 2)
    assert str(v) == "1.4.2"


def test_version_defaults_and_ctor():
    assert yggdryl.Version(2) == yggdryl.Version.parse("2")
    assert str(yggdryl.Version(2)) == "2.0.0"
    assert yggdryl.Version(1, 2, 3).patch == 3


def test_version_ordering():
    assert yggdryl.Version(1, 4, 2) < yggdryl.Version(1, 10, 0)
    assert yggdryl.Version(2, 0, 0) > yggdryl.Version(1, 99, 99)
    ordered = sorted([yggdryl.Version(1, 2, 0), yggdryl.Version(0, 9, 9)])
    assert ordered[0] == yggdryl.Version(0, 9, 9)


def test_version_invalid_raises():
    with pytest.raises(ValueError):
        yggdryl.Version.parse("1.x.0")


def test_module_version():
    assert isinstance(yggdryl.__version__, str)
