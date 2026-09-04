from __future__ import annotations

import copy
import os
import pickle
from pathlib import PurePosixPath, PureWindowsPath
from typing import Any, Callable

import pytest

from yggdryl import MediaType, MimeType, Uri, Url, Urn


def test_uri_components_path_collection_and_value_protocols() -> None:
    value = Uri("https://example.com/archive/report.tar.gz?download=1#summary")

    assert value.scheme == "https"
    assert value.authority == "example.com"
    assert value.path == "/archive/report.tar.gz"
    assert value.query == "download=1"
    assert value.fragment == "summary"
    assert value.path_segments == ("archive", "report.tar.gz")
    assert tuple(value) == value.path_segments
    assert len(value) == 2
    assert value[0] == "archive"
    assert value[-1] == "report.tar.gz"
    assert "archive" in value
    assert value.file_name == "report.tar.gz"
    assert value.stem == "report.tar"
    assert value.extension == "gz"
    assert value.extensions == ("tar", "gz")

    assert Uri.from_str(str(value)) == value
    assert Uri.from_value(value) == value
    assert Uri.from_json(value.into_json()) == value
    assert eval(repr(value), {"Uri": Uri}) == value
    assert copy.copy(value) == value
    assert pickle.loads(pickle.dumps(value)) == value
    assert hash(value) == hash(Uri.from_str(str(value)))
    assert value.stable_hash() == Uri.from_str(str(value)).stable_hash()
    with pytest.raises(IndexError):
        _ = value[2]


def test_credentials_and_s3_location_are_parsed_by_the_core() -> None:
    credentials = Uri(
        "https://user:pass:word@[2001:db8::1]:8443/archive/data.parquet"
    )
    assert credentials.user == "user"
    assert credentials.password == "pass:word"
    assert credentials.hostname == "2001:db8::1"
    assert credentials.bucket is None
    assert credentials.region is None

    bucket = Uri("s3://market-data/year=2026/data.parquet")
    assert bucket.hostname is None
    assert bucket.bucket == "market-data"
    assert bucket.region is None

    endpoint = Url(
        "s3://market-data.s3.dualstack.eu-west-3.amazonaws.com/data.parquet"
    )
    assert endpoint.hostname == "market-data.s3.dualstack.eu-west-3.amazonaws.com"
    assert endpoint.bucket == "market-data"
    assert endpoint.region == "eu-west-3"

    compatible = Uri("s3://objects.example.io/archive/data.parquet")
    assert compatible.hostname == "objects.example.io"
    assert compatible.bucket == "archive"
    assert compatible.region is None


def test_uri_joinpath_and_division_use_the_core_path_resolver() -> None:
    base = Uri("https://example.com/a/b?q=1#rows")

    joined = base.joinpath("../c", PurePosixPath("data.parquet"))
    divided = base / "child"

    assert joined == Uri("https://example.com/a/c/data.parquet?q=1#rows")
    assert divided == Uri("https://example.com/a/b/child?q=1#rows")
    assert base.joinpath("/root") == Uri("https://example.com/root?q=1#rows")
    assert str(base) == "https://example.com/a/b?q=1#rows"

    keyed = {base: "source"}
    locked_join = base / "after-hash"
    locked_join.set_extension("json")
    assert locked_join == Uri("https://example.com/a/b/after-hash.json?q=1#rows")
    assert keyed[base] == "source"
    with pytest.raises(TypeError, match="hashed Uri is frozen"):
        base.set_stem("blocked")


def test_windows_and_unc_paths_are_normalized_by_core() -> None:
    path = PureWindowsPath(r"C:\Users\Ada Lovelace\report.parquet")
    drive = Uri.from_path(path)
    unc = Uri.from_path(r"\\server\share\prices\ticks.csv")

    assert Uri(path) == drive
    assert Url.from_path(path).into_uri() == drive
    assert str(drive) == "file:///C:/Users/Ada%20Lovelace/report.parquet"
    assert drive.scheme == "file"
    assert drive.authority == ""
    assert drive.path == "/C:/Users/Ada%20Lovelace/report.parquet"
    assert "\\" not in str(drive)
    assert drive.file_name == "report.parquet"
    assert drive.extension == "parquet"
    assert drive.into_path() == "C:/Users/Ada Lovelace/report.parquet"
    assert os.fspath(drive) == drive.into_path()
    file_url = Url.from_path(path)
    assert file_url.into_path() == drive.into_path()
    assert os.fspath(file_url) == drive.into_path()

    assert str(unc) == "file://server/share/prices/ticks.csv"
    assert unc.scheme == "file"
    assert unc.authority == "server"
    assert unc.path == "/share/prices/ticks.csv"
    assert "\\" not in str(unc)
    assert unc.into_path() == "//server/share/prices/ticks.csv"

    unicode_unc = Uri("file://caf%C3%A9/share/market%20data%25.csv")
    assert unicode_unc.into_path() == "//café/share/market data%.csv"
    assert Uri.from_path(unicode_unc.into_path()) == unicode_unc

    with pytest.raises(ValueError):
        Uri("https://example.com/data").into_path()
    with pytest.raises(ValueError):
        os.fspath(Uri("https://example.com/data"))
    assert Uri("file:///tmp/A%20B.csv").into_path() == "/tmp/A B.csv"
    assert Uri("file://server").into_path() == "//server/"
    with pytest.raises(ValueError):
        Uri("file:///tmp/a%2Fb.csv").into_path()
    with pytest.raises(ValueError):
        Uri("file:///tmp/a%5Cb.csv").into_path()
    with pytest.raises(ValueError):
        Uri("file:///tmp/data.csv?download=1").into_path()
    with pytest.raises(ValueError):
        Uri("file:///tmp/data.csv#row-1").into_path()
    with pytest.raises(ValueError):
        Uri("file:").into_path()
    with pytest.raises(ValueError):
        Uri("file:///tmp/%FF.csv").into_path()
    with pytest.raises(ValueError):
        Uri("file://user%40host/share/data.csv").into_path()
    with pytest.raises(ValueError):
        Uri("file:///%43%3A/data.csv").into_path()
    with pytest.raises(ValueError):
        Uri("file:///tmp/%C2%85/data.csv").into_path()


def test_url_converts_through_uri_without_binding_side_parsing() -> None:
    uri = Uri("https://example.com/a/data.json?raw=true")
    value = Url.from_uri(uri)

    assert Url(uri) == value
    assert Url.from_value(str(value)) == value
    assert Uri(value) == uri
    assert value.into_uri() == uri
    assert uri.into_url() == value
    assert value.scheme == "https"
    assert value.authority == "example.com"
    assert value.path_segments == ("a", "data.json")
    assert value.file_name == "data.json"
    assert value.stem == "data"
    assert value.extension == "json"
    assert Url("https://example.com/a/") / "data.json" == Url(
        "https://example.com/a/data.json"
    )
    assert Url.from_json(value.into_json()) == value
    assert eval(repr(value), {"Url": Url}) == value
    assert copy.copy(value) == value
    assert pickle.loads(pickle.dumps(value)) == value
    assert hash(value) == hash(Url.from_str(str(value)))
    assert value.stable_hash() == Url.from_str(str(value)).stable_hash()
    other = Url("https://example.com/b/data.json")
    assert value < other or other < value

    with pytest.raises(ValueError, match="URL"):
        Url(Urn("urn:isbn:9780131103627"))


def test_urn_components_and_uri_conversion() -> None:
    uri = Uri("urn:isbn:9780141036144")
    value = Urn.from_uri(uri)

    assert Urn(uri) == value
    assert Urn.from_value(str(value)) == value
    assert Uri(value) == uri
    assert value.into_uri() == uri
    assert uri.into_urn() == value
    assert value.scheme == "urn"
    assert value.authority == ""
    assert value.namespace == "isbn"
    assert value.namespace_specific == "9780141036144"
    assert Urn.from_json(value.into_json()) == value
    assert eval(repr(value), {"Urn": Urn}) == value
    assert copy.copy(value) == value
    assert pickle.loads(pickle.dumps(value)) == value
    assert hash(value) == hash(Urn.from_str(str(value)))
    assert value.stable_hash() == Urn.from_str(str(value)).stable_hash()
    other = Urn("urn:isbn:9780131103627")
    assert value < other or other < value

    with pytest.raises(ValueError):
        Urn.from_uri(Uri("https://example.com/resource"))
    with pytest.raises(ValueError, match="URN"):
        Urn(Url("https://example.com/resource"))
    with pytest.raises(ValueError):
        uri.into_url()


def test_filename_mutators_are_atomic_and_copies_are_independent() -> None:
    value = Uri("https://example.com/archive/report.tar.gz?q=1#part")
    clone = copy.copy(value)

    value.set_stem("renamed")
    assert str(value) == "https://example.com/archive/renamed.gz?q=1#part"
    value.set_extension("zst")
    value.set_extensions(extension for extension in ("csv", "gz"))
    assert str(value) == "https://example.com/archive/renamed.csv.gz?q=1#part"
    assert value.remove_extension() is True
    assert value.extensions == ("csv",)
    assert value.clear_extensions() is True
    assert value.extension is None
    assert str(clone) == "https://example.com/archive/report.tar.gz?q=1#part"

    unchanged = str(value)
    for invalid in ("", "bad/name", "bad?name", "bad#name", "bad%"):
        with pytest.raises(ValueError):
            value.set_file_name(invalid)
        assert str(value) == unchanged
    with pytest.raises(ValueError):
        value.set_extensions(["json", "bad/name"])
    assert str(value) == unchanged
    with pytest.raises(TypeError, match="iterable of strings"):
        value.set_extensions("json")
    assert str(value) == unchanged

    authority_only = Url("https://example.com?q=1#part")
    authority_only.set_file_name("data.json")
    assert str(authority_only) == "https://example.com/data.json?q=1#part"

    urn = Urn("urn:example:reports/data.csv?=raw#rows")
    urn.set_file_name("renamed.json")
    assert str(urn) == "urn:example:reports/renamed.json?=raw#rows"
    assert urn.namespace == "example"
    assert urn.file_name == "renamed.json"
    assert urn.stem == "renamed"


@pytest.mark.parametrize(
    ("value", "mutate"),
    [
        (Uri("https://example.com/data.json"), lambda value: value.set_stem("other")),
        (Url("https://example.com/data.json"), lambda value: value.set_extension("csv")),
        (Urn("urn:example:data.json"), lambda value: value.clear_extensions()),
    ],
)
def test_hash_locks_only_the_hashed_uri_wrapper(
    value: Any, mutate: Callable[[Any], object]
) -> None:
    original = str(value)
    stable_only = copy.copy(value)
    stable_only.stable_hash()
    mutate(stable_only)
    assert str(stable_only) != original

    keyed = {value: "stable"}

    with pytest.raises(TypeError, match="hashed .* is frozen"):
        mutate(value)
    assert str(value) == original
    assert keyed[value] == "stable"

    copied = copy.copy(value)
    mutate(copied)
    assert str(copied) != original

    restored = pickle.loads(pickle.dumps(value))
    mutate(restored)
    assert str(restored) != original


def test_uri_mime_and_media_inference_uses_native_suffix_tables() -> None:
    value = Uri("https://example.com/report.csv.gz.zst?q=1#part")

    assert value.mime_type == MimeType("application/zstd")
    assert value.media_type.base == MimeType("text/csv")
    assert value.media_type.encodings == (
        MimeType("application/gzip"),
        MimeType("application/zstd"),
    )

    value.set_mime_type("application/json")
    assert str(value) == "https://example.com/report.csv.gz.json?q=1#part"

    media = MediaType.from_parts(
        MimeType("text/csv"),
        [MimeType("application/gzip"), MimeType("application/zstd")],
    )
    value.set_media_type(media)
    assert str(value) == "https://example.com/report.csv.gz.zst?q=1#part"

    unchanged = str(value)
    with pytest.raises(ValueError, match="preferred filename extension"):
        value.set_mime_type("application/vnd.example")
    assert str(value) == unchanged
    custom_media = MediaType.from_parts(
        MimeType("application/vnd.example"), [MimeType("application/gzip")]
    )
    with pytest.raises(ValueError, match="preferred filename extension"):
        value.set_media_type(custom_media)
    assert str(value) == unchanged

    urn = Urn("urn:example:reports/data.json")
    urn.set_media_type("text/csv;encodings=application/gzip")
    assert str(urn) == "urn:example:reports/data.csv.gz"
    assert urn.namespace == "example"
