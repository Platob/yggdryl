from __future__ import annotations

import copy
import os
import pickle
from collections.abc import Iterator
from pathlib import PurePosixPath, PureWindowsPath
from typing import Any, Callable

import pytest

from yggdryl import MediaType, MimeType, Uri, Url, Urn


def test_uri_components_path_collection_and_value_protocols() -> None:
    value = Uri("https://example.com/archive/report.tar.gz?download=1#summary")

    assert value.scheme == "https"
    assert value.authority == "example.com"
    assert value.path == "/archive/report.tar.gz"
    assert value.query() == "download=1"
    assert value.fragment() == "summary"
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

    # The key is the path below the bucket, as the path spells it.
    assert bucket.key == "year=2026/data.parquet"
    assert endpoint.key == "data.parquet"
    assert compatible.key == "data.parquet"
    assert Uri("s3://market-data/lake/").key == "lake/"
    assert Uri("s3://market-data/").key == ""
    assert Uri("https://example.com/data.parquet").key is None

    # A port, an IP literal, or `localhost` names an endpoint, never a bucket.
    local = Uri("s3://localhost:9000/market-data/lake/part.parquet")
    assert local.hostname == "localhost"
    assert local.bucket == "market-data"
    assert local.key == "lake/part.parquet"


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


def test_components_answer_raw_or_decoded_text() -> None:
    value = Url("https://example.com/a%20b/c%2Fd?q=a%26b#p%C3%A9")

    assert value.path == "/a%20b/c%2Fd"
    assert value.path_text() == "/a%20b/c%2Fd"
    assert value.query() == "q=a%26b"
    assert value.fragment() == "p%C3%A9"

    # Decoding answers with text, not with structure: the encoded slash stays
    # inside the segment that carried it.
    assert value.path_text(decode=True) == "/a b/c/d"
    assert value.path_segments == ("a%20b", "c%2Fd")
    assert value.query(decode=True) == "q=a&b"
    assert value.fragment(True) == "pé"

    urn = Urn("urn:example:weather?=op=map%20view#now")
    assert urn.query(decode=True) == "=op=map view"
    assert urn.path_text(decode=True) == "example:weather"

    # An escape that is not UTF-8 is refused rather than replaced.
    invalid = Uri("https://example.com/a%FF")
    assert invalid.path_text() == "/a%FF"
    with pytest.raises(ValueError, match="UTF-8"):
        invalid.path_text(decode=True)


def test_parameters_read_the_query_as_its_pairs() -> None:
    value = Url("https://example.com/t?symbol=AAPL&venue=XNAS&symbol=MSFT&flag")
    parameters = value.parameters()

    assert len(parameters) == 4
    assert bool(parameters)
    assert "symbol" in parameters
    assert "absent" not in parameters
    assert parameters["symbol"] == "AAPL"
    assert parameters.get_all("symbol") == ("AAPL", "MSFT")
    # A pair carrying no `=` names an empty value rather than nothing.
    assert parameters["flag"] == ""
    assert parameters.get("absent") is None
    assert parameters.get("absent", "fallback") == "fallback"
    assert list(parameters) == ["symbol", "venue", "symbol", "flag"]
    assert list(parameters.keys()) == ["symbol", "venue", "symbol", "flag"]
    assert list(parameters.values()) == ["AAPL", "XNAS", "MSFT", ""]
    assert list(parameters.items()) == [
        ("symbol", "AAPL"),
        ("venue", "XNAS"),
        ("symbol", "MSFT"),
        ("flag", ""),
    ]
    assert parameters.into_dict() == {"symbol": "AAPL", "venue": "XNAS", "flag": ""}
    assert repr(parameters).startswith("Parameters({")

    with pytest.raises(KeyError):
        parameters["absent"]

    assert Url("https://example.com/t").parameters().into_query() is None
    assert not Url("https://example.com/t?").parameters()


def test_parameters_write_through_to_the_url() -> None:
    value = Url("https://example.com/t?symbol=AAPL&venue=XNAS&symbol=MSFT#part")
    parameters = value.parameters()

    # Setting keeps the key's position and drops the pairs it replaced.
    parameters["symbol"] = "TSLA"
    assert str(value) == "https://example.com/t?symbol=TSLA&venue=XNAS#part"

    parameters.append("symbol", "NVDA")
    assert parameters.get_all("symbol") == ("TSLA", "NVDA")

    del parameters["symbol"]
    assert str(value) == "https://example.com/t?venue=XNAS#part"
    with pytest.raises(KeyError):
        del parameters["symbol"]

    parameters.update({"region": "eu"}, depth="10")
    assert parameters.into_query() == "venue=XNAS&region=eu&depth=10"
    assert parameters.pop("region") == "eu"
    assert parameters.pop("region", None) is None
    with pytest.raises(KeyError):
        parameters.pop("region")
    assert parameters.setdefault("depth", "50") == "10"
    assert parameters.setdefault("side", "buy") == "buy"

    parameters.clear()
    assert str(value) == "https://example.com/t#part"
    assert len(parameters) == 0

    # The view is live rather than a snapshot: the URL changing is visible.
    value.set_query("symbol=NVDA")
    assert parameters["symbol"] == "NVDA"
    assert value.query() == "symbol=NVDA"


def test_parameters_decode_and_encode_when_asked() -> None:
    value = Url("https://example.com/t?as%20of=2026-01-02&note=a%26b")

    raw = value.parameters()
    assert not raw.decode
    assert raw["as%20of"] == "2026-01-02"
    assert raw["note"] == "a%26b"

    decoded = value.parameters(decode=True)
    assert decoded.decode
    assert decoded["as of"] == "2026-01-02"
    assert decoded["note"] == "a&b"
    assert decoded == {"as of": "2026-01-02", "note": "a&b"}
    assert decoded != raw

    # A decoding view encodes what the query syntax cannot carry; a raw view
    # refuses it instead of writing a query that would not parse back.
    decoded["side"] = "buy & sell"
    assert str(value).endswith("&side=buy%20%26%20sell")
    assert value.parameters(decode=True)["side"] == "buy & sell"

    with pytest.raises(ValueError):
        raw["side"] = "buy & sell"


def test_a_hashed_url_refuses_a_parameter_write() -> None:
    value = Url("https://example.com/t?symbol=AAPL")
    parameters = value.parameters()
    hash(value)

    with pytest.raises(TypeError, match="frozen"):
        parameters["symbol"] = "MSFT"
    assert value.query() == "symbol=AAPL"


def test_a_lookup_that_changes_nothing_writes_nothing() -> None:
    """The mapping methods that only read must not rewrite the query.

    A write-back respells what it parsed - ``flag`` becomes ``flag=``, an empty
    pair disappears - and a frozen value refuses it, so a ``dict``-shaped read
    would both change the URL and raise where a ``dict`` would not.
    """
    value = Url("https://example.com/t?flag&a=1&&b=%7Ez")
    stored = str(value)
    parameters = value.parameters()

    assert parameters.pop("absent", None) is None
    assert parameters.setdefault("a", "9") == "1"
    parameters.update()
    assert str(value) == stored

    with pytest.raises(KeyError):
        parameters.pop("absent")
    assert str(value) == stored

    empty = Url("https://example.com/t")
    empty.parameters().clear()
    assert str(empty) == "https://example.com/t"

    # The same reads answer on a hashed value, which refuses every write.
    frozen = Url("https://example.com/t?a=1")
    hash(frozen)
    assert frozen.parameters().pop("absent", "default") == "default"
    assert frozen.parameters().setdefault("a", "9") == "1"
    frozen.parameters().update()
    with pytest.raises(TypeError, match="frozen"):
        frozen.parameters()["a"] = "2"


def test_parameters_compare_and_update_the_way_a_mapping_does() -> None:
    import collections.abc

    class Reading(collections.abc.Mapping[str, str]):
        """A mapping that is not a ``dict``, which ``update`` must still take."""

        def __init__(self, values: dict[str, str]) -> None:
            self._values = values

        def __getitem__(self, key: str) -> str:
            return self._values[key]

        def __iter__(self) -> Iterator[str]:
            return iter(self._values)

        def __len__(self) -> int:
            return len(self._values)

    value = Url("https://example.com/t")
    value.parameters().update(Reading({"symbol": "AAPL"}))
    value.parameters().update([("venue", "XNAS")])
    assert value.query() == "symbol=AAPL&venue=XNAS"
    assert value.parameters() == {"symbol": "AAPL", "venue": "XNAS"}

    # A query that names one key twice is no dict, whatever the dict holds.
    repeated = Url("https://example.com/t?a=1&a=1").parameters()
    assert repeated != {"a": "1"}
    assert repeated != {"a": "1", "b": "2"}
    assert repeated == Url("https://example.com/t?a=1&a=1").parameters()


def test_the_pair_views_are_read_more_than_once() -> None:
    parameters = Url("https://example.com/t?a=1&b=2&a=3").parameters()

    keys = parameters.keys()
    assert list(keys) == ["a", "b", "a"]
    assert list(keys) == ["a", "b", "a"]
    assert len(keys) == 3
    assert parameters.values() == ("1", "2", "3")
    assert parameters.items() == (("a", "1"), ("b", "2"), ("a", "3"))
    assert list(parameters) == ["a", "b", "a"]


def test_the_scheme_and_the_authority_answer_what_they_alone_decide() -> None:
    value = Uri("https://reader:pa:ss@example.com:8443/a")

    # The port written in the authority and the port the scheme implies are
    # two different answers, so they have two names.
    assert value.port == 8443
    assert value.default_port == 443
    assert value.host_port == "example.com:8443"
    assert value.user == "reader"
    assert value.password == "pa:ss"
    assert value.is_storage()
    assert value.has_authority

    assert Uri("postgres://host/db").default_port == 5432
    assert Uri("mysql://host/db").default_port == 3306
    assert Uri("s3://bucket/key").default_port is None
    assert not Uri("s3://bucket/key").port
    assert not Uri("urn:isbn:9780131103627").is_storage()

    # An IPv6 host keeps its brackets out of the port.
    assert Uri("https://[2001:db8::1]:9000/a").port == 9000
    assert Uri("https://[2001:db8::1]/a").port is None

    # `mailto:` writes no authority marker; `file:` writes an empty one.
    assert not Uri("mailto:desk@example.com").has_authority
    assert Uri("file:///lake").has_authority
    assert Url("file:///lake").default_port is None


def test_a_store_location_names_its_endpoint_and_its_addressing() -> None:
    virtual = Uri("s3://bucket.s3.us-east-1.amazonaws.com/key")
    assert virtual.is_virtual_hosted()
    assert virtual.store_endpoint == "s3.us-east-1.amazonaws.com"
    assert virtual.bucket == "bucket"
    assert virtual.region == "us-east-1"

    plain = Url("s3://bucket/key")
    assert not plain.is_virtual_hosted()
    assert plain.store_endpoint is None
    assert plain.bucket == "bucket"

    google = Url("gs://bucket/key")
    assert google.bucket == "bucket"
    assert google.key == "key"

    azure = Uri("abfss://lake@trades.dfs.core.windows.net/part")
    assert azure.bucket == "lake"
    assert azure.account == "trades"
    assert azure.key == "part"

    assert Uri("https://example.com/a").store_endpoint is None
    assert not Uri("https://example.com/a").is_virtual_hosted()


def test_a_uri_is_built_from_its_parts_and_validated_as_a_whole() -> None:
    value = Uri.from_parts("https", "example.com", "/a/b", "raw=true", "top")
    assert str(value) == "https://example.com/a/b?raw=true#top"
    assert value.validate() is None

    assert str(Uri.from_parts("mailto", "", "desk@example.com")) == "mailto:desk@example.com"
    assert Uri.from_parts("https", "example.com", "/a").query() is None

    # A path after an authority is empty or slash-rooted.
    with pytest.raises(ValueError):
        Uri.from_parts("https", "example.com", "a/b")
    with pytest.raises(ValueError):
        Uri.from_parts("not a scheme", "", "/a")


def test_a_uri_navigates_the_path_it_addresses() -> None:
    value = Uri("https://example.com/a/b/../c.json?raw=true")

    # `parts` is what the path reaches; `path_segments` is what it says.
    assert value.parts == ("a", "c.json")
    assert value.path_segments == ("a", "b", "..", "c.json")
    assert Url("file:///a/./b/../c").parts == ("a", "c")

    # Scheme, authority, and query survive the walk to the root.
    assert str(value.parent) == "https://example.com/a?raw=true"
    assert [str(parent) for parent in value.parents] == [
        "https://example.com/a?raw=true",
        "https://example.com/?raw=true",
    ]

    # A location at the root is its own parent, which is what `pathlib` does.
    root = Uri("https://example.com/")
    assert str(root.parent) == str(root)
    assert root.parents == ()


def test_a_glob_url_splits_into_the_root_a_listing_starts_from() -> None:
    pattern = Url("file:///lake/trades/year=2024/**/*.parquet")
    assert pattern.is_glob()
    assert pattern.is_recursive_glob()

    root, rest = pattern.glob_parts()
    assert str(root) == "file:///lake/trades/year=2024"
    assert rest == "**/*.parquet"

    # A location that is not a glob is its own root with no pattern.
    plain = Url("file:///lake/trades/part-0.parquet")
    assert not plain.is_glob()
    assert not plain.is_recursive_glob()
    assert plain.glob_parts() == (plain, None)

    assert Url.is_pattern("part-*")
    assert Url.is_pattern("[ab]c")
    assert not Url.is_pattern("part-0.parquet")


def test_a_pattern_matches_anchored_at_the_root_it_was_split_from() -> None:
    root = Url("file:///lake/trades")
    leaf = Url("file:///lake/trades/year=2024/part-0.parquet")

    assert leaf.full_match_under(root, "**/*.parquet")
    assert leaf.full_match_under("file:///lake/trades", "year=2024/*.parquet")
    # Anchored at the root, so a bare name does not reach two levels down.
    assert not leaf.full_match_under(root, "*.parquet")
    # A location outside the root never matches.
    assert not leaf.full_match_under("file:///elsewhere", "**/*.parquet")

    # The same pattern read from `glob_parts` matches what it was split from.
    pattern_root, pattern = Url("file:///lake/trades/**/*.parquet").glob_parts()
    assert pattern is not None
    assert leaf.full_match_under(pattern_root, pattern)


def test_hive_partitions_are_read_relative_to_the_address_they_hang_under() -> None:
    leaf = Url("file:///lake/year=2024/month=01/part-0.parquet")

    assert leaf.is_partitioned()
    assert leaf.partitions == (("year", "2024"), ("month", "01"))
    assert leaf.partitions_under("file:///lake") == (("year", "2024"), ("month", "01"))
    # A directory in the table's own address is not a partition of it.
    assert leaf.partitions_under("file:///lake/year=2024") == (("month", "01"),)
    assert leaf.partitions_under("file:///elsewhere") == ()

    assert str(Url("file:///lake").with_partition("year", "2024")) == "file:///lake/year=2024"
    assert not Url("file:///lake/part-0.parquet").is_partitioned()


def test_a_url_says_whether_it_is_local_and_what_the_local_entry_is(
    tmp_path: Any,
) -> None:
    directory = Url.from_path(os.fspath(tmp_path))
    assert directory.is_local()
    assert str(directory.local_mime_type) == str(MimeType.DIRECTORY)

    leaf = directory / "trades.parquet"
    assert str(leaf.local_mime_type) == "application/vnd.apache.parquet"

    remote = Url("https://example.com/trades.parquet")
    assert not remote.is_local()
    assert str(remote.local_mime_type) == str(remote.mime_type)


def test_joining_a_path_like_value_reads_its_own_separators() -> None:
    base = Url("file:///lake")

    # A `str` is one URL path component, joined as written.
    assert str(base.joinpath("a", "b")) == "file:///lake/a/b"
    assert str(base / "a/b") == "file:///lake/a/b"

    # An `os.PathLike` is an operating system path, read component by
    # component, so `.` and `..` resolve the way the core resolves them.
    assert str(base / PurePosixPath("a/b")) == "file:///lake/a/b"
    assert str(base / PurePosixPath("a/./b/../c")) == "file:///lake/a/c"
    assert str(base.joinpath(PurePosixPath("a"), PurePosixPath("b"))) == "file:///lake/a/b"
