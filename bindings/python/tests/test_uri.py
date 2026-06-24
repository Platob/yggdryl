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
    uri = yggdryl.Uri.from_str("mailto:alice@example.com")
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
    v = yggdryl.Version.from_str("1.4.2")
    assert (v.major, v.minor, v.patch) == (1, 4, 2)
    assert str(v) == "1.4.2"


def test_version_defaults_and_ctor():
    assert yggdryl.Version(2) == yggdryl.Version.from_str("2")
    assert str(yggdryl.Version(2)) == "2.0.0"
    assert yggdryl.Version(1, 2, 3).patch == 3


def test_version_ordering():
    assert yggdryl.Version(1, 4, 2) < yggdryl.Version(1, 10, 0)
    assert yggdryl.Version(2, 0, 0) > yggdryl.Version(1, 99, 99)
    ordered = sorted([yggdryl.Version(1, 2, 0), yggdryl.Version(0, 9, 9)])
    assert ordered[0] == yggdryl.Version(0, 9, 9)


def test_version_invalid_raises():
    with pytest.raises(ValueError):
        yggdryl.Version.from_str("1.x.0")


def test_safe_flag():
    # Invalid scheme: rejected when safe, accepted when not.
    with pytest.raises(ValueError):
        yggdryl.Uri("1http:x")
    assert yggdryl.Uri("1http:x", safe=False).scheme == "1http"
    # Lenient version parse.
    assert str(yggdryl.Version.from_str("1.2.3.4", safe=False)) == "1.2.3"


def test_from_mapping():
    uri = yggdryl.Uri.from_mapping({"scheme": "https", "authority": "example.com", "path": "/x"})
    assert str(uri) == "https://example.com/x"
    url = yggdryl.Url.from_mapping({"scheme": "https", "host": "h", "port": "8443"})
    assert url.host == "h" and url.port == 8443
    assert yggdryl.Version.from_mapping({"major": "1", "minor": "4"}) == yggdryl.Version(1, 4)


def test_percent_encoding():
    assert yggdryl.percent_encode("a b/c") == "a%20b%2Fc"
    assert yggdryl.percent_decode("a%20b%2Fc") == "a b/c"
    with pytest.raises(ValueError):
        yggdryl.percent_decode("%zz")


def test_from_parts():
    url = yggdryl.Url.from_parts("https", "example.com", port=8443, username="user", password="pw", path="/api")
    assert str(url) == "https://user:pw@example.com:8443/api"
    uri = yggdryl.Uri.from_parts("mailto", path="alice@example.com")
    assert str(uri) == "mailto:alice@example.com"


def test_functional_copy_and_with():
    base = yggdryl.Url("https://example.com/api")
    secured = base.with_port(8443).with_username("user")
    assert str(secured) == "https://user@example.com:8443/api"
    # original untouched
    assert str(base) == "https://example.com/api"
    assert str(yggdryl.Version(1, 0, 0).with_minor(4)) == "1.4.0"
    assert yggdryl.Uri("https://h/a").copy() == yggdryl.Uri("https://h/a")


def test_params_and_add_param():
    url = yggdryl.Url("https://h/p?a=1&a=2&b=hi")
    params = url.params()
    assert params["a"] == ["1", "2"]
    assert params["b"] == ["hi"]
    # add_param adds or replaces, multi-value aware
    updated = url.add_param("a", ["x"]).add_param("c", ["1", "2"])
    assert updated.params()["a"] == ["x"]
    assert updated.params()["c"] == ["1", "2"]
    # with_params percent-encodes
    built = yggdryl.Uri("https://h/p").with_params({"q": ["a b"]})
    assert built.query == "q=a%20b"


def test_copy_overrides():
    url = yggdryl.Url("https://example.com/api")
    assert str(url.copy(port=8443)) == "https://example.com:8443/api"
    assert str(url.copy()) == "https://example.com/api"


def test_url_to_uri():
    url = yggdryl.Url("https://user@h:8443/p?x=1")
    uri = url.to_uri()
    assert isinstance(uri, yggdryl.Uri)
    assert uri.authority == "user@h:8443"


def test_module_version():
    assert isinstance(yggdryl.__version__, str)
