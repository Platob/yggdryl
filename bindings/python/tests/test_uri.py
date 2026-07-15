"""Tests for the ``yggdryl.uri`` URIs, URLs, and authorities."""

import pickle
from urllib.parse import urlparse

import pytest

import yggdryl.uri
from yggdryl.uri import Authority, Uri, Url, default_port


def test_module_surface():
    import yggdryl

    for cls in (Authority, Uri, Url):
        assert cls.__module__ == "yggdryl.uri"
        assert hasattr(yggdryl.uri, cls.__name__)


def test_parse_full_url_every_accessor():
    uri = Uri.parse("https://user:pw@example.com:8080/a/b.tar.gz?q=1#frag")
    assert uri.scheme == "https"
    assert uri.user == "user"
    assert uri.password == "pw"
    assert uri.host == "example.com"
    assert uri.port == 8080
    assert uri.path == "/a/b.tar.gz"
    assert uri.query == "q=1"
    assert uri.fragment == "frag"
    # Path accessors.
    assert uri.name == "b.tar.gz"
    assert uri.stem == "b.tar"
    assert uri.extension == "gz"
    assert uri.extensions == ["tar", "gz"]
    # The authority is exposed as its own value type.
    auth = uri.authority
    assert isinstance(auth, Authority)
    assert auth.host == "example.com"
    assert auth.port == 8080
    assert str(auth) == "user:pw@example.com:8080"
    # Canonical string round-trips through Display.
    assert str(uri) == "https://user:pw@example.com:8080/a/b.tar.gz?q=1#frag"


def test_windows_path_is_posix_normalized():
    uri = Uri.from_path(r"C:\Users\x\archive.tar.gz")
    assert uri.scheme is None
    assert uri.path == "C:/Users/x/archive.tar.gz"
    assert uri.name == "archive.tar.gz"
    assert uri.stem == "archive.tar"
    assert uri.extensions == ["tar", "gz"]
    # A parsed drive path normalizes identically (drive letter kept, no scheme).
    assert Uri.parse(r"C:\Users\x\archive.tar.gz") == uri


def test_dotfile_has_no_extension():
    uri = Uri.from_path("/home/me/.bashrc")
    assert uri.name == ".bashrc"
    assert uri.stem == ".bashrc"
    assert uri.extension is None
    assert uri.extensions == []


def test_mutators_builder_and_in_place():
    # Builder mutators return a new value and leave the original untouched.
    base = Uri.from_path("/p")
    built = base.with_scheme("https").with_host("h").with_port(443).with_query("a=1")
    assert str(built) == "https://h:443/p?a=1"
    assert base.scheme is None  # unchanged

    # In-place setters mutate the receiver.
    u = Uri.from_path("/p")
    u.set_scheme("ftp")
    u.set_host("h")
    u.set_port(21)
    u.set_user("me")
    u.set_fragment("frag")
    assert u.scheme == "ftp"
    assert u.host == "h"
    assert u.port == 21
    assert u.user == "me"
    assert u.fragment == "frag"


def test_bytes_and_pickle_round_trip():
    uri = Uri.parse("scheme://h/a/b.txt?q#f")
    assert Uri.deserialize_bytes(uri.serialize_bytes()) == uri
    assert pickle.loads(pickle.dumps(uri)) == uri
    assert uri.serialize_bytes() == b"scheme://h/a/b.txt?q#f"

    url = Url.parse("scheme://h/a/b.txt")
    assert Url.deserialize_bytes(url.serialize_bytes()) == url
    assert pickle.loads(pickle.dumps(url)) == url

    # Authority pickles through its components (no core byte codec).
    auth = Authority("h", user="u", password="p", port=42)
    assert pickle.loads(pickle.dumps(auth)) == auth
    assert Authority.from_host("h").host == "h"


def test_value_semantics_eq_hash():
    a = Uri.parse("https://h/p")
    b = Uri.parse("https://h/p")
    assert a == b
    assert hash(a) == hash(b)
    assert len({a, b}) == 1
    assert Uri.parse("https://h/p") != Uri.parse("https://h/other")


def test_uri_url_interchange():
    uri = Uri.parse("https://example.com/p")
    url = uri.to_url()
    assert isinstance(url, Url)
    assert url.scheme == "https"
    assert url.host == "example.com"
    # Uri.into_url is an alias of to_url; Url.from_uri is the reverse constructor.
    assert uri.into_url() == url
    assert Url.from_uri(uri) == url
    # A Url unwraps back to its Uri.
    assert url.as_uri() == uri
    assert url.into_uri() == uri
    # mailto: is a valid Url with a scheme but no host.
    mailto = Url.parse("mailto:person@example.com")
    assert mailto.scheme == "mailto"
    assert mailto.host is None
    assert mailto.path == "person@example.com"


def test_scheme_less_uri_is_not_a_url():
    rel = Uri.parse("/relative/path")
    assert rel.scheme is None
    with pytest.raises(ValueError, match="absolute"):
        rel.to_url()
    with pytest.raises(ValueError, match="absolute"):
        Url.parse("/relative/path")


def test_default_port_module_function():
    assert default_port("https") == 443
    assert default_port("HTTPS") == 443  # scheme is case-insensitive
    assert default_port("ws") == 80
    assert default_port("postgres") == 5432
    assert default_port("s3") is None  # no registered default
    assert yggdryl.uri.default_port("ssh") == 22  # reachable on the namespace too


def test_port_or_default_falls_back_to_scheme():
    implicit = Uri.parse("https://example.com/p")
    assert implicit.port is None
    assert implicit.default_port == 443
    assert implicit.port_or_default == 443

    explicit = Uri.parse("https://example.com:8443/p")
    assert explicit.port == 8443
    assert explicit.port_or_default == 8443  # explicit wins over the default

    # Scheme-less / no-default schemes report None.
    assert Uri.parse("//h/p").port_or_default is None
    assert Uri.parse("s3://bucket/key").port_or_default is None

    # Url mirrors it (scheme always present).
    assert Url.parse("wss://h/socket").port_or_default == 443


def test_default_port_does_not_mutate_the_uri():
    uri = Uri.parse("https://example.com/p")
    assert uri.port_or_default == 443
    assert str(uri) == "https://example.com/p"  # no ":443" was written in
    assert uri.serialize_bytes() == b"https://example.com/p"
    # An implicit-port URI stays distinct from one written with the default port.
    assert uri != Uri.parse("https://example.com:443/p")


def test_ipv6_host_detection_and_unbracketing():
    uri = Uri.parse("http://[2001:db8::1]:8080/p")
    assert uri.host == "[2001:db8::1]"  # stored bracketed
    assert uri.host_is_ipv6
    assert uri.host_unbracketed == "2001:db8::1"  # bare address to dial
    assert uri.port_or_default == 8080

    plain = Uri.parse("http://example.com/p")
    assert not plain.host_is_ipv6
    assert plain.host_unbracketed == "example.com"

    # No authority -> None / False.
    mailto = Uri.parse("mailto:a@b.com")
    assert not mailto.host_is_ipv6
    assert mailto.host_unbracketed is None

    # Authority value type exposes the same pair.
    auth = Authority.from_host("[::1]")
    assert auth.host_is_ipv6
    assert auth.host_unbracketed == "::1"

    # Url mirrors it.
    url = Url.parse("https://[::1]/status")
    assert url.host_is_ipv6
    assert url.host_unbracketed == "::1"


def test_joinpath_combines_paths_correctly():
    base = Uri.parse("https://api.example.com/v1")
    assert str(base.joinpath("users")) == "https://api.example.com/v1/users"
    # Chains; a trailing slash on the base is not doubled.
    assert base.joinpath("users").joinpath("42").path == "/v1/users/42"
    assert Uri.from_path("/v1/").joinpath("users").path == "/v1/users"
    # Multi-segment in one call.
    assert Uri.from_path("/v1").joinpath("users/42").path == "/v1/users/42"
    # An absolute segment resets the path; query/fragment are kept.
    assert str(Uri.parse("https://h/a?x=1#f").joinpath("/b")) == "https://h/b?x=1#f"
    # A relative segment under an authority stays rooted (does not fuse into the host).
    assert Uri.parse("https://h").joinpath("p").path == "/p"
    # Encoded like set_path.
    assert Uri.from_path("/v1").joinpath("a b").path == "/v1/a%20b"
    # Url.joinpath keeps the scheme.
    assert Url.parse("https://h/v1").joinpath("x").scheme == "https"


def test_merge_with_overlays_present_components():
    base = Uri.parse("https://prod.example.com/v1?trace=1")
    # A patch with only an authority swaps the host, keeping scheme/path/query.
    assert (
        str(base.merge_with(Uri.parse("//staging.example.com")))
        == "https://staging.example.com/v1?trace=1"
    )
    # Merging a default (empty) URI is an identity copy.
    assert base.merge_with(Uri.parse("")) == base
    # Authority merges at the component level.
    a = Authority("db", user="svc", password="secret", port=5432)
    assert str(a.merge_with(Authority.from_host("replica"))) == "svc:secret@replica:5432"


def test_copy_is_independent():
    base = Uri.parse("https://h/a?q#f")
    dup = base.copy()
    assert dup == base
    dup.set_path("/b")  # mutating the copy leaves the original untouched
    assert base.path == "/a"
    assert dup.path == "/b"
    assert Authority.from_host("h").copy() == Authority.from_host("h")


def test_stdlib_copy_module_via_dunders():
    import copy

    uri = Uri.parse("https://user@h:8080/a?q=1#f")
    assert copy.copy(uri) == uri and copy.deepcopy(uri) == uri
    url = Url.parse("s3://bucket/key")
    assert copy.copy(url) == url and copy.deepcopy(url) == url
    auth = Authority.from_host("h").with_port(443)
    assert copy.copy(auth) == auth and copy.deepcopy(auth) == auth
    # The copy is independent.
    dup = copy.copy(uri)
    dup.set_path("/b")
    assert uri.path == "/a"


def test_with_authority_and_authority_builders():
    authority = Authority.from_host("db.internal").with_user("svc").with_port(5432)
    built = (
        Uri.from_path("").with_scheme("postgres").with_authority(authority).with_path("/app")
    )
    assert str(built) == "postgres://svc@db.internal:5432/app"
    # Dropping the authority.
    assert Uri.parse("https://user@h:8080/p").with_authority(None).authority is None
    # Authority builders chain and clear via None.
    a = Authority.from_host("h").with_user("u").with_password("p").with_port(80)
    assert str(a) == "u:p@h:80"
    assert str(a.with_user(None).with_password(None)) == "h:80"


def test_out_of_range_port_is_guided_value_error():
    with pytest.raises(ValueError, match="0..=65535"):
        Uri.parse("https://h:99999/")


def test_bad_scheme_and_non_utf8_are_guided():
    with pytest.raises(ValueError, match="scheme"):
        Uri.parse("://host")
    with pytest.raises(ValueError, match="UTF-8"):
        Uri.deserialize_bytes(b"\xff\xfe")


@pytest.mark.parametrize(
    "raw",
    [
        "https://user@example.com:8080/a/b?q=1#f",
        "ftp://host.example.org/dir/file.txt",
    ],
)
def test_parity_with_urllib(raw):
    # Sanity: yggdryl agrees with the stdlib on the core components.
    ours = Uri.parse(raw)
    theirs = urlparse(raw)
    assert ours.scheme == theirs.scheme
    assert ours.host == theirs.hostname
    assert ours.port == theirs.port
    assert ours.path == theirs.path


def test_query_param_crud():
    uri = Uri.parse("http://h/p?a=1&b=2&a=3")
    # Read
    assert uri.query_param("a") == "1"  # first occurrence wins
    assert uri.query_param("missing") is None
    assert uri.query_param_all("a") == ["1", "3"]
    assert uri.query_params() == [("a", "1"), ("b", "2"), ("a", "3")]
    assert dict(uri.query_params()) == {"a": "3", "b": "2"}  # map view (last dup wins)
    assert uri.has_query_param("b")
    assert not uri.has_query_param("z")

    # Update (map semantics) then create (absent -> appended)
    uri.set_query_param("a", "9")
    assert uri.query == "a=9&b=2"
    uri.set_query_param("c", "7")
    assert uri.query == "a=9&b=2&c=7"

    # Delete
    assert uri.remove_query_param("a") is True
    assert uri.query == "b=2&c=7"
    assert uri.remove_query_param("z") is False

    # Builder variants return fresh values
    built = Uri.parse("http://h/p").with_query_param("x", "1").with_query_param("y", "2")
    assert str(built) == "http://h/p?x=1&y=2"
    assert str(built.without_query_param("x")) == "http://h/p?y=2"


def test_query_param_on_url_and_edges():
    url = Url.parse("https://h/p?flag&a=")
    assert url.query_param("flag") == ""  # bare key -> empty value
    assert url.query_param("a") == ""  # explicit empty value
    assert url.has_query_param("flag")
    url.set_query_param("flag", "on")
    assert url.query_param("flag") == "on"
    assert Uri.parse("http://h/p?t=a=b").query_param("t") == "a=b"  # value keeps inner '='


def test_query_params_bulk_and_normalize():
    uri = Uri.parse("http://h/p?a=1&b=2&a=3")
    uri.set_query_params([("a", "9"), ("c", "7")])  # bulk update in one pass
    assert uri.query == "a=9&b=2&c=7"
    uri.set_query_params(list({"z": "1"}.items()))  # apply a dict via items()
    assert uri.query_param("z") == "1"

    messy = Uri.parse("http://h/p?c=3&a=1&&b=2")
    messy.normalize_query()  # sort by key + drop empty tokens
    assert messy.query == "a=1&b=2&c=3"

    built = Uri.parse("http://h/p?b=2").with_query_params([("a", "1")]).with_normalized_query()
    assert str(built) == "http://h/p?a=1&b=2"


def test_query_param_encoding():
    uri = Uri.parse("http://h/p")
    uri.set_query_param("q", "a b&c")
    assert uri.query == "q=a%20b%26c"                          # stored encoded
    assert uri.query_param("q") == "a b&c"                     # decoded by default
    assert uri.query_param("q", encoded=True) == "a%20b%26c"   # raw stored form

    for value in ["plain", "a b", "100%", "x&y=z", "café", ""]:
        u = Uri.parse("http://h/p").with_query_param("k", value)
        assert u.query_param("k") == value                     # set -> get round-trips

    u = Uri.parse("http://h/p").with_query_param("n", "a b").with_query_param("t", "x&y")
    assert dict(u.query_params()) == {"n": "a b", "t": "x&y"}           # decoded map view
    assert dict(u.query_params(encoded=True)) == {"n": "a%20b", "t": "x%26y"}

    assert Uri.parse("http://h").with_path("/a b").path == "/a%20b"     # component encoded
