"""Tests that every value/data type survives ``pickle`` and ``copy``.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import copy
import pickle

import yggdryl


def _round_trips(obj):
    """A value type round-trips through pickle and deepcopy, comparing by equality."""
    assert pickle.loads(pickle.dumps(obj)) == obj
    assert copy.deepcopy(obj) == obj


def test_value_types_pickle_and_copy():
    _round_trips(yggdryl.Version(1, 4, 2))
    _round_trips(yggdryl.Uri("https://example.com/docs?page=2#intro"))
    _round_trips(yggdryl.Url("https://user:pw@example.com:8443/api?v=1#t"))
    _round_trips(yggdryl.MimeType("image/png"))
    # An unknown but well-formed MIME (``Other``) round-trips verbatim.
    _round_trips(yggdryl.MimeType("application/x-made-up"))
    _round_trips(yggdryl.MediaType.from_path("data.csv.gz"))
    _round_trips(yggdryl.Compression("gzip"))


def test_bytesio_pickle_preserves_bytes():
    buffer = yggdryl.BytesIO(b"hello world")
    restored = pickle.loads(pickle.dumps(buffer))
    assert restored.getvalue() == b"hello world"
    assert copy.deepcopy(buffer).getvalue() == b"hello world"


def test_localpath_pickle_preserves_location(tmp_path):
    target = tmp_path / "data.bin"
    target.write_bytes(b"abc")
    handle = yggdryl.LocalPath(str(target))
    restored = pickle.loads(pickle.dumps(handle))
    assert restored.location == str(target)
    assert restored.getvalue() == b"abc"


def test_iostats_pickle_round_trips():
    stats = yggdryl.IoStats(size=42, kind="file", content_type="text/csv", etag="abc")
    restored = pickle.loads(pickle.dumps(stats))
    assert restored.size == 42
    assert restored.kind == "file"
    assert restored.content_type == "text/csv"
    assert restored.etag == "abc"


def test_http_response_pickle_round_trips():
    response = yggdryl.HttpResponse(
        200,
        "https://example.com/",
        [("content-type", "text/plain")],
        b"body",
        http_version="HTTP/2",
    )
    restored = pickle.loads(pickle.dumps(response))
    assert restored.status == 200
    assert restored.url == "https://example.com/"
    assert restored.content == b"body"
    assert restored.header("content-type") == "text/plain"
    # The negotiated protocol version round-trips too (default is "HTTP/1.1").
    assert restored.http_version == "HTTP/2"


def test_module_level_http_verbs_exist():
    # The shared-session convenience verbs are exposed at module level.
    for name in ("get", "head", "delete", "post", "put", "patch", "request"):
        assert callable(getattr(yggdryl, name))
