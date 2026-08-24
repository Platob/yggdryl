from __future__ import annotations

import copy
import json
import pickle
from typing import Any

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, MediaType, MimeType, Uri, Url


def test_field_infers_datatype_and_field_representations() -> None:
    field = Field("quantity", "decimal(18, 4)", nullable=False)
    arrow = pa.field("quantity", pa.decimal128(18, 4), nullable=False)

    assert Field("id", DataType("int64")).data_type == DataType("int64")
    assert field.into_arrow() == arrow
    assert Field.from_value(field) == field
    assert Field.from_value(arrow) == field
    assert Field.from_arrow(arrow) == field
    assert Field.from_value(str(field)) == field


def test_field_arrow_scalar_enforces_nullability_and_safe_casts() -> None:
    required = Field("quantity", "int8", nullable=False)
    nullable = Field("quantity", "int8")
    exact = pa.scalar(7, type=pa.int8())

    assert required.arrow_scalar(exact) is exact
    assert required.arrow_scalar(pa.scalar(7, type=pa.int64())).equals(exact)
    assert required.arrow_scalar(130, safe=False).as_py() == -126
    assert nullable.arrow_scalar(None).equals(pa.scalar(None, type=pa.int8()))

    with pytest.raises(ValueError, match="quantity.*not nullable"):
        required.arrow_scalar(None)
    with pytest.raises(ValueError, match="quantity.*not nullable"):
        required.arrow_scalar(pa.scalar(None, type=pa.int64()))
    with pytest.raises((pa.ArrowInvalid, OverflowError)):
        required.arrow_scalar(130)


def test_field_arrow_scalar_preserves_registered_extension_type() -> None:
    class ScalarExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(pa.int32(), "tests.field.scalar-extension")

        def __arrow_ext_serialize__(self) -> bytes:
            return b"v1"

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> ScalarExtension:
            assert storage_type == pa.int32()
            assert serialized == b"v1"
            return cls()

    extension = ScalarExtension()
    pa.register_extension_type(extension)
    try:
        field = Field.from_arrow(
            pa.field("value", extension, nullable=False)
        )
        exact = pa.scalar(3, type=extension)

        assert field.arrow_scalar(exact) is exact
        constructed = field.arrow_scalar(4)
        cast = field.arrow_scalar(pa.scalar(5, type=pa.int32()))
        assert constructed.type.equals(extension)
        assert constructed.as_py() == 4
        assert cast.type.equals(extension)
        assert cast.as_py() == 5
    finally:
        pa.unregister_extension_type(extension.extension_name)


def test_field_arrow_roundtrip_preserves_recursive_layout_flags() -> None:
    arrow = pa.field(
        "payload",
        pa.struct(
            [
                pa.field(
                    "lookup",
                    pa.map_(pa.string(), pa.int64(), keys_sorted=True),
                    False,
                ),
                pa.field(
                    "codes",
                    pa.dictionary(pa.int8(), pa.string(), ordered=True),
                    False,
                ),
            ]
        ),
        False,
    )

    projected = Field.from_arrow(arrow).into_arrow()
    assert projected.type.field("lookup").type.keys_sorted is True
    assert projected.type.field("codes").type.ordered is True


def test_field_http_metadata_is_canonical_typed_and_https_compatible() -> None:
    field = Field(
        "payload",
        "binary",
        nullable=False,
        metadata={
            "HTTPS:Content-Type": "Application/JSON; Charset=utf-8",
            "HTTP:Content-Encoding": " gzip,\tbr ",
            "HTTPS:Content-Length": "00042",
            "HTTPS:Location": "../relative",
        },
    )

    assert field.content_type == "Application/JSON; Charset=utf-8"
    assert field.content_encoding == " gzip,\tbr "
    assert field.content_length == 42
    assert field.mime_type == MimeType.JSON
    assert field.media_type == MediaType.from_parts(
        MimeType.JSON, [MimeType.GZIP, MimeType.BROTLI]
    )
    assert field.metadata.get("HTTPS:CONTENT-TYPE") == field.content_type
    assert field.get_property("https", "CONTENT-TYPE") == field.content_type
    assert dict(field.property_iter("https")) == {
        "content-encoding": " gzip,\tbr ",
        "content-length": "42",
        "content-type": "Application/JSON; Charset=utf-8",
        "location": "../relative",
    }
    assert list(field.metadata) == [
        "http:content-encoding",
        "http:content-length",
        "http:content-type",
        "http:location",
    ]
    with pytest.raises(ValueError):
        _ = field.http_location

    assert field.remove_property("https", "CONTENT-LENGTH") == "42"
    field.set_http_location("https://example.test/data")
    assert str(field.http_location) == "https://example.test/data"
    assert str(field.remove_http_location()) == "https://example.test/data"


def test_field_http_raw_vocabulary_validation_and_u64_boundaries() -> None:
    field = Field("payload", "binary")
    raw_values = {
        "accept": "application/json",
        "accept_encoding": "gzip, br",
        "accept_language": "en, fr;q=0.8",
        "accept_ranges": "bytes",
        "cache_control": "public, max-age=60",
        "content_disposition": 'attachment; filename="data.json"',
        "content_encoding": "unknown-coding",
        "content_language": "en",
        "content_location": "../data.json",
        "content_range": "bytes 0-9/10",
        "content_type": "application/json; charset=utf-8",
        "etag": '"revision-1"',
        "expires": "Sun, 16 Aug 2026 00:00:00 GMT",
        "last_modified": "Sat, 15 Aug 2026 00:00:00 GMT",
        "range": "bytes=0-9",
        "vary": "accept-encoding",
    }
    for name, value in raw_values.items():
        getattr(field, f"set_{name}")(value)
        assert getattr(field, name) == value
    for name, value in reversed(raw_values.items()):
        assert getattr(field, f"remove_{name}")() == value
        assert getattr(field, name) is None

    field.set_content_length(2**64 - 1)
    assert field.content_length == 2**64 - 1
    assert field.remove_content_length() == 2**64 - 1
    for invalid in [True, -1, 2**64, 1.5, "1"]:
        with pytest.raises((TypeError, OverflowError)):
            field.set_content_length(invalid)  # type: ignore[arg-type]
    for value in ["a\rb", "a\nb", "a\0b", "a\x7fb", "a\x1fb"]:
        with pytest.raises(ValueError):
            field.set_etag(value)
    field.set_etag("one\ttwo")
    assert field.etag == "one\ttwo"


def test_field_typed_http_media_updates_are_atomic() -> None:
    field = Field("payload", "binary")
    field.set_media_type(
        MediaType.from_parts(
            MimeType.CSV,
            [MimeType.GZIP, MimeType.COMPRESS, MimeType.ZSTD],
        )
    )
    assert field.content_type == "text/csv"
    assert field.content_encoding == "gzip, compress, zstd"
    before = field.into_json()

    with pytest.raises(ValueError):
        field.set_media_type(MediaType.from_parts(MimeType.JSON, [MimeType.BZIP2]))
    assert field.into_json() == before

    field.set_mime_type(MimeType.JSON)
    assert field.content_type == "application/json"
    assert field.content_encoding == "gzip, compress, zstd"
    assert field.remove_mime_type() == MimeType.JSON
    assert field.content_encoding == "gzip, compress, zstd"

    field.set_content_type("application/json")
    field.set_content_encoding("identity")
    malformed = field.into_json()
    with pytest.raises(ValueError):
        _ = field.media_type
    with pytest.raises(ValueError):
        field.remove_media_type()
    assert field.into_json() == malformed


def test_field_string_json_order_hash_repr_and_pickle_protocols() -> None:
    field = Field("symbol", pa.string(), nullable=False, metadata={"venue": "XPAR"})

    assert Field.from_str(str(field)) == field
    assert Field.from_json(field.into_json()) == field
    assert eval(repr(field), {"Field": Field}) == field
    assert copy.copy(field) == field
    assert pickle.loads(pickle.dumps(field)) == field
    assert hash(field) == hash(Field.from_value(field))
    assert field.stable_hash() == Field.from_value(field).stable_hash()
    other = Field("symbol", pa.string(), metadata={"venue": "XPAR"})
    assert field < other or other < field


def test_field_metadata_view_implements_the_mapping_protocol() -> None:
    field = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})
    peer = Field("symbol", "utf8", metadata={"venue": "XPAR"})
    before = field.stable_hash()

    # Item access on the field itself reaches a nested child; the metadata
    # mapping lives on the view, which is where a key means a key.
    assert len(field.metadata) == 1
    assert list(field.metadata) == ["venue"]
    assert "venue" in field.metadata
    assert field.metadata["venue"] == "XPAR"
    assert field.metadata.get("venue") == "XPAR"
    marker = object()
    assert field.metadata.get("missing", marker) is marker

    # A scalar field has no children, so subscripting it is a KeyError even
    # for a key its metadata carries.
    assert len(field) == 0
    with pytest.raises(KeyError):
        field["venue"]
    assert list(field.metadata.keys()) == ["venue"]
    assert list(field.metadata.values()) == ["XPAR"]
    assert list(field.metadata.items()) == [("venue", "XPAR")]
    assert field.metadata == peer.metadata
    assert field.metadata != object()
    with pytest.raises(TypeError):
        hash(field.metadata)

    field.metadata["currency"] = "EUR"
    assert field.metadata != peer.metadata
    peer.metadata["currency"] = "EUR"
    assert field.metadata == peer.metadata
    assert field.stable_hash() != before
    field.metadata.update({"source": "exchange"}, venue="XEUR")
    assert dict(field.metadata.items()) == {
        "currency": "EUR",
        "source": "exchange",
        "venue": "XEUR",
    }
    del field.metadata["source"]
    assert "source" not in field.metadata
    with pytest.raises(KeyError):
        del field.metadata["missing"]
    field.metadata.clear()
    assert len(field.metadata) == 0


def test_metadata_bulk_paths_are_sorted_last_write_wins_and_scale_widely() -> None:
    entries = [(f"key_{index:05d}", str(index)) for index in reversed(range(4096))]
    entries.extend((("duplicate", "first"), ("duplicate", "second")))
    field = Field("wide", "utf8", metadata=entries)

    assert len(field.metadata) == 4097
    assert field.metadata["duplicate"] == "second"
    assert list(field.metadata) == sorted(field.metadata)

    field.metadata.update(
        (("duplicate", "iterable"), ("new_key", "iterable")),
        duplicate="keyword",
        new_key="keyword",
    )
    assert field.metadata["duplicate"] == "keyword"
    assert field.metadata["new_key"] == "keyword"

    before = dict(field.metadata.items())
    invalid = [(f"valid_{index:04d}", str(index)) for index in range(1024)]
    invalid.append(("", "rejected"))
    with pytest.raises(ValueError):
        field.metadata.update(invalid)
    assert dict(field.metadata.items()) == before


def test_invalid_inputs_are_reported_without_partial_metadata_mutation() -> None:
    with pytest.raises((TypeError, ValueError)):
        DataType(object())
    with pytest.raises(ValueError):
        DataType.from_str("not a datatype")

    field = Field("id", "int64", metadata={"valid": "value"})
    with pytest.raises((TypeError, ValueError)):
        field.metadata.update([("also_valid", "value"), ("", "invalid")])
    assert dict(field.metadata.items()) == {"valid": "value"}


def test_typed_names_location_and_protocol_properties_share_field_metadata() -> None:
    field = Field("price", "decimal(18, 6)", nullable=False)

    field.set_alias("close")
    field.set_catalog_name("analytics")
    field.set_schema_name("market")
    field.set_table_name("bars")
    field.set_location(Uri("s3://warehouse/bars/day=2026-08-15/data.parquet"))

    assert field.alias == "close"
    assert field.catalog_name == "analytics"
    assert field.schema_name == "market"
    assert field.table_name == "bars"
    assert field.location == Url("s3://warehouse/bars/day=2026-08-15/data.parquet")
    assert field.metadata["location"] == str(field.location)

    assert field.set_property("POSTGRES", "type", "numeric(18,6)") is None
    assert field.set_property("postgres", "column", "close") is None
    assert field.set_property("iceberg", "field-id", "7") is None
    assert field.set_property("fix", "tag", "44") is None
    assert field.set_property("field", "role", "measure") is None
    assert field.set_property("dtype", "logical", "price") is None
    assert field.get_property("postgres", "type") == "numeric(18,6)"
    assert field.has_property("postgres", "column")
    assert list(field.property_iter("postgres")) == [
        ("column", "close"),
        ("type", "numeric(18,6)"),
    ]
    assert field.metadata["postgres:type"] == "numeric(18,6)"

    assert field.set_property("postgres", "type", "decimal") == "numeric(18,6)"
    assert field.remove_property("postgres", "type") == "decimal"
    assert field.remove_property("postgres", "type") is None
    field.clear_properties("postgres")
    assert list(field.property_iter("postgres")) == []
    assert field.has_property("iceberg", "field-id")

    assert field.remove_alias() == "close"
    assert field.remove_catalog_name() == "analytics"
    assert field.remove_schema_name() == "market"
    assert field.remove_table_name() == "bars"
    assert field.remove_location() == Url(
        "s3://warehouse/bars/day=2026-08-15/data.parquet"
    )
    assert field.location is None


WELL_KNOWN_PROTOCOLS = (
    "http",
    "file",
    "urn",
    "postgres",
    "postgresql",
    "mysql",
    "arrow",
    "sql",
    "glue",
    "iceberg",
    "fix",
    "field",
    "dtype",
    "s3",
    "gs",
    "az",
    "spark",
    "polars",
    "pandas",
)


def test_protocol_view_implements_the_mapping_protocol_over_bare_names() -> None:
    field = Field("price", "float64", nullable=False, metadata={"venue": "XPAR"})
    view = field.iceberg

    assert not view
    assert len(view) == 0
    assert list(view) == []
    assert view.scheme == "iceberg"
    assert view.prefix == "iceberg"
    assert view.key("doc") == "iceberg:doc"

    view["doc"] = "closing price"
    view["field-id"] = "7"

    assert bool(view)
    assert len(view) == 2
    assert "doc" in view
    assert "iceberg:doc" not in view
    assert 7 not in view
    assert view["doc"] == "closing price"
    assert view.get("doc") == "closing price"
    marker = object()
    assert view.get("missing", marker) is marker
    assert view.get("missing") is None
    assert list(view) == ["doc", "field-id"]
    assert list(view.keys()) == ["doc", "field-id"]
    assert list(view.values()) == ["closing price", "7"]
    assert list(view.items()) == [("doc", "closing price"), ("field-id", "7")]
    assert dict(view) == {"doc": "closing price", "field-id": "7"}
    assert str(view) == '{"doc":"closing price","field-id":"7"}'
    assert repr(view) == (
        'ProtocolMetadata("iceberg", {"doc":"closing price","field-id":"7"})'
    )

    with pytest.raises(KeyError, match="iceberg:missing"):
        _ = view["missing"]
    with pytest.raises(KeyError, match="iceberg:missing"):
        del view["missing"]
    with pytest.raises(TypeError):
        view["doc"] = 3  # type: ignore[assignment]
    with pytest.raises(TypeError):
        _ = view[3]  # type: ignore[index]

    view.update({"doc": "close"}, snapshot="9")
    assert dict(view.items()) == {
        "doc": "close",
        "field-id": "7",
        "snapshot": "9",
    }
    before = dict(view.items())
    with pytest.raises(ValueError):
        view.update([("valid", "1"), ("", "rejected")])
    assert dict(view.items()) == before

    del view["snapshot"]
    assert "snapshot" not in view
    view.clear()
    assert not view
    assert dict(field.metadata.items()) == {"venue": "XPAR"}


def test_protocol_view_is_a_live_window_on_the_field_it_came_from() -> None:
    field = Field("price", "float64", nullable=False)
    view = field.iceberg
    other = field.iceberg

    view["doc"] = "closing price"
    assert field.get_property("iceberg", "doc") == "closing price"
    assert field.metadata["iceberg:doc"] == "closing price"
    assert other["doc"] == "closing price"
    assert view == other

    other["doc"] = "close"
    assert view["doc"] == "close"

    field.set_property("iceberg", "field-id", "7")
    assert view["field-id"] == "7"
    assert len(view) == 2

    field.clear_properties("iceberg")
    assert not view
    assert view == field.postgres

    unrelated = Field("other", "int64")
    unrelated.iceberg["doc"] = "close"
    assert view != unrelated.iceberg
    with pytest.raises(TypeError):
        hash(view)


def test_protocol_view_named_accessors_cover_every_well_known_protocol() -> None:
    field = Field("price", "float64", nullable=False)

    for protocol in WELL_KNOWN_PROTOCOLS:
        view = getattr(field, protocol)
        assert view.scheme == protocol
        assert view.prefix == protocol
        assert view.key("doc") == f"{protocol}:doc"
        view["doc"] = protocol

    assert [name.split(":", 1)[0] for name in field.metadata] == sorted(
        WELL_KNOWN_PROTOCOLS
    )
    for protocol in WELL_KNOWN_PROTOCOLS:
        assert field.get_property(protocol, "doc") == protocol
        assert field.metadata[f"{protocol}:doc"] == protocol
        assert field.protocol(protocol.upper())["doc"] == protocol

    with pytest.raises(ValueError):
        field.protocol("1invalid")


def test_protocol_view_http_covers_https_and_ignores_header_case() -> None:
    field = Field(
        "payload",
        "binary",
        nullable=False,
        metadata={"HTTPS:Content-Type": "application/json"},
    )
    http = field.http
    https = field.protocol("HTTPS")

    assert https.scheme == "https"
    assert http.prefix == https.prefix == "http"
    assert http["Content-Type"] == "application/json"
    assert https["content-type"] == "application/json"
    assert "CONTENT-TYPE" in http
    assert list(http) == ["content-type"]
    assert http == https

    https["Content-Encoding"] = "gzip"
    assert field.metadata["http:content-encoding"] == "gzip"
    assert http["content-encoding"] == "gzip"
    assert field.content_encoding == "gzip"

    del http["CONTENT-ENCODING"]
    assert field.content_encoding is None
    assert not hasattr(field, "https")


def test_protocol_view_refuses_writes_to_a_generated_field_class() -> None:
    row_type = Field.from_arrow_schema(
        pa.schema([pa.field("id", pa.int64(), nullable=False)])
    ).into_dataclass(name="FrozenViewField")
    frozen = row_type.field()
    child = frozen.data_type[0]
    view = child.iceberg

    assert view.get("doc") is None
    assert len(view) == 0
    with pytest.raises(TypeError, match="read-only"):
        view["doc"] = "identifier"
    with pytest.raises(TypeError, match="read-only"):
        view.update({"doc": "identifier"})
    with pytest.raises(TypeError, match="read-only"):
        del view["doc"]
    with pytest.raises(TypeError, match="read-only"):
        view.clear()
    with pytest.raises(TypeError, match="read-only"):
        child.set_partition(True)
    for restored in (copy.copy(frozen), pickle.loads(pickle.dumps(frozen))):
        with pytest.raises(TypeError, match="read-only"):
            restored.metadata["owner"] = "tests"
    assert list(view.items()) == []
    assert not child.is_partition


def test_partition_fields_are_marked_reported_and_split_on_a_struct_root() -> None:
    root = Field(
        "row",
        DataType.from_fields(
            [
                Field("year", "int32", nullable=False),
                Field("month", "int32", nullable=False),
                Field("price", "float64", nullable=False),
            ]
        ),
        nullable=False,
    )

    assert not root.has_partition_fields
    assert root.partition_fields == []
    assert root.partition_field_names == []
    assert root.partition_field_len == 0
    assert root.without_partition_fields() == root

    marked = root.with_partition_fields(["year", "month"])

    assert marked.has_partition_fields
    assert marked.partition_field_names == ["year", "month"]
    assert marked.partition_field_len == 2
    assert [child.name for child in marked.partition_fields] == ["year", "month"]
    assert all(isinstance(child, Field) for child in marked.partition_fields)
    assert all(child.is_partition for child in marked.partition_fields)
    assert marked.partition_fields[0].field["partition"] == "true"
    assert not root.has_partition_fields

    assert marked.only_partition_fields().partition_field_names == ["year", "month"]
    assert [child.name for child in marked.only_partition_fields().data_type] == [
        "year",
        "month",
    ]
    assert [child.name for child in marked.without_partition_fields().data_type] == [
        "price"
    ]

    year = Field("year", "int32", nullable=False)
    assert not year.is_partition
    year.set_partition(True)
    assert year.is_partition
    assert year.metadata["field:partition"] == "true"
    assert year.field["partition"] == "true"
    year.set_partition(False)
    assert not year.is_partition
    assert year == Field("year", "int32", nullable=False)

    with pytest.raises(TypeError):
        year.set_partition(1)  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="struct root"):
        year.only_partition_fields()
    with pytest.raises(ValueError, match="struct root"):
        year.without_partition_fields()


def test_with_partition_fields_reports_a_column_the_root_does_not_have() -> None:
    root = Field(
        "row",
        DataType.from_fields([Field("year", "int32", nullable=False)]),
        nullable=False,
    )

    with pytest.raises(ValueError, match=r'a column of "row" to partition on'):
        root.with_partition_fields(["year", "missing"])
    assert not root.has_partition_fields
    with pytest.raises(TypeError):
        # A bare string is a column name spelled as an iterable of letters,
        # which the boundary refuses rather than partitioning on "y".
        root.with_partition_fields("year")


def test_typed_int32_field_id_is_canonical_atomic_and_arrow_compatible() -> None:
    imported = Field.from_arrow(
        pa.field(
            "value",
            pa.int64(),
            metadata={b"PARQUET:field_id": b"+00017"},
        )
    )
    assert imported.parquet_field_id == 17
    assert imported.metadata["PARQUET:field_id"] == "17"
    assert imported.into_arrow().metadata[b"PARQUET:field_id"] == b"17"

    field = Field("value", "int64")
    assert field.parquet_field_id is None
    for value in (-(2**31), 2**31 - 1):
        field.set_parquet_field_id(value)
        assert field.parquet_field_id == value
        assert field.metadata["PARQUET:field_id"] == str(value)
    assert field.remove_parquet_field_id() == 2**31 - 1
    assert field.remove_parquet_field_id() is None

    field.set_parquet_field_id(7)
    for invalid in (True, 7.0, 2**31, -(2**31) - 1):
        with pytest.raises((TypeError, OverflowError)):
            field.set_parquet_field_id(invalid)  # type: ignore[arg-type]
        assert field.parquet_field_id == 7
    with pytest.raises(ValueError):
        field["PARQUET:field_id"] = "not-an-int32"
    assert field.parquet_field_id == 7
    with pytest.raises(ValueError):
        Field(
            "invalid",
            "int64",
            metadata={"PARQUET:field_id": "2147483648"},
        )

    row_type = Field.from_arrow_schema(
        pa.schema([imported.into_arrow()])
    ).into_dataclass(name="IdentifiedField")
    child = row_type.field().data_type[0]
    assert child.parquet_field_id == 17
    assert row_type.field().into_arrow_schema().field(0).metadata[
        b"PARQUET:field_id"
    ] == b"17"
    with pytest.raises(TypeError, match="read-only"):
        child.set_parquet_field_id(18)
    with pytest.raises(TypeError, match="read-only"):
        child.remove_parquet_field_id()


def test_typed_metadata_validation_is_atomic_and_arrow_compatible() -> None:
    field = Field("id", "int64", metadata={"source": "feed"})
    before = field.into_arrow()

    with pytest.raises(ValueError):
        field.set_alias("")
    with pytest.raises(ValueError):
        field.set_property("postgres", "", "integer")
    with pytest.raises(ValueError):
        field.set_property("1invalid", "type", "integer")
    with pytest.raises(ValueError):
        field["location"] = "urn:isbn:9780131103627"

    assert field.into_arrow() == before
    assert field.set_property("postgres", "default", "") is None
    assert field.get_property("postgres", "default") == ""
    assert field.remove_property("postgres", "default") == ""
    field.set_table_name("events")
    field.set_property("arrow", "extension:name", "example.event")
    arrow = field.into_arrow()
    assert arrow.metadata[b"table_name"] == b"events"
    assert arrow.metadata[b"arrow:extension:name"] == b"example.event"
    imported = Field.from_arrow(arrow)
    assert imported.table_name == "events"
    assert imported.get_property("arrow", "extension:name") == "example.event"


def test_dictionary_options_are_owned_and_validated_by_core() -> None:
    field = Field.from_str(
        'field("codes",dictionary(int16,utf8),nullable=true,'
        'dictionary_id=42,dictionary_is_ordered=true,metadata={})'
    )

    assert field.dictionary_id == 42
    assert field.dictionary_is_ordered is True
    field.set_dictionary_options(7, False)
    assert field.dictionary_id == 7
    assert field.dictionary_is_ordered is False
    assert Field.from_str(str(field)) == field

    keyed = {field: "stable"}
    with pytest.raises(TypeError, match="hashed Field is frozen"):
        field.set_dictionary_options(9, True)
    assert keyed[field] == "stable"
    copied = copy.copy(field)
    copied.set_dictionary_options(9, True)
    assert copied.dictionary_id == 9

    with pytest.raises(ValueError):
        Field("plain", "int64").set_dictionary_options(1, True)

    wide = Field.from_str(
        'field("wide",dictionary(int16,utf8),nullable=true,'
        'dictionary_id=9007199254740993,metadata={})'
    )
    assert wide.dictionary_id == 9_007_199_254_740_993
    assert json.loads(wide.into_json())["dictionary_id"] == "9007199254740993"


class TestGenericCast:
    """One generic cast, the kind inferred and kept."""

    @staticmethod
    def _root() -> Any:
        return Field("row", DataType("struct<id: int64, symbol: string>"), False)

    def test_every_pyarrow_kind_comes_back_as_itself(self) -> None:
        import pyarrow as pa

        root = self._root()
        batch = pa.table(
            {"id": pa.array([1, 2], pa.int32()), "symbol": ["AAPL", "MSFT"]}
        ).to_batches()[0]

        # A batch, a table, and a reader keep their kinds; the int32 widens.
        cast_batch = root.cast_arrow(batch)
        assert isinstance(cast_batch, pa.RecordBatch)
        assert cast_batch.schema.field("id").type == pa.int64()
        assert root.cast_arrow_batch(batch).equals(cast_batch)

        table = pa.Table.from_batches([batch])
        cast_table = root.cast_arrow(table)
        assert isinstance(cast_table, pa.Table)

        reader = root.cast_arrow(table.to_reader())
        assert isinstance(reader, pa.RecordBatchReader)
        assert reader.read_all().num_rows == 2

        # An array casts as an array, a scalar as a scalar - and `cast` is
        # the same dispatch with plain Python values allowed too.
        price = Field("price", DataType("int64"), False)
        assert price.cast_arrow(pa.array([5], pa.int32())).type == pa.int64()
        assert price.cast_arrow_scalar(pa.scalar(5, pa.int32())).as_py() == 5
        assert price.cast(5).as_py() == 5

    def test_polars_frames_stay_polars_and_lazy_stays_lazy(self) -> None:
        pl = pytest.importorskip("polars")

        root = self._root()
        frame = pl.DataFrame({"id": [1, 2], "symbol": ["AAPL", "MSFT"]})

        cast = root.cast_arrow(frame)
        assert isinstance(cast, pl.DataFrame)
        assert cast.schema["id"] == pl.Int64

        # The lazy frame answers schema questions without collecting, and the
        # cast is deferred until the caller collects.
        lazy = root.cast_arrow(frame.lazy())
        assert isinstance(lazy, pl.LazyFrame)
        collected = lazy.collect()
        assert collected.schema["id"] == pl.Int64
        assert collected.height == 2

    def test_pandas_frames_cross_through_arrow_and_back(self) -> None:
        pd = pytest.importorskip("pandas")

        root = self._root()
        frame = pd.DataFrame({"id": [1, 2], "symbol": ["AAPL", "MSFT"]})
        cast = root.cast_arrow(frame)
        assert isinstance(cast, pd.DataFrame)
        assert list(cast.columns) == ["id", "symbol"]


def test_item_access_on_a_schema_node_reaches_a_nested_child() -> None:
    """Subscripting a `Field` or a `DataType` descends the schema.

    One semantic across both classes: before, `field["level"]` was a metadata
    lookup while `data_type["level"]` was a child, so a caller walking one
    object graph got two unrelated things from identical syntax.
    """
    line = Field(
        "line",
        DataType.from_fields([Field("price", "float64"), Field("qty", "int64")]),
        nullable=False,
    )
    order = Field(
        "order",
        DataType.from_fields([Field("id", "int64", nullable=False), line]),
        nullable=False,
    )

    # By name, on both classes, with the same answer.
    assert order["id"].data_type == DataType("int64")
    assert order.data_type["id"].data_type == DataType("int64")

    # By position, negatives counting from the end.
    assert order[0].name == "id"
    assert order[-1].name == "line"
    assert order.data_type[-1].name == "line"

    # Chained subscripts are the nesting story - no dotted path form.
    assert order["line"]["price"].data_type == DataType("float64")
    assert order["line"]["qty"].data_type == DataType("int64")

    # `len`, `in`, and iteration speak children on both classes.
    assert len(order) == 2
    assert len(order.data_type) == 2
    assert "line" in order
    assert "line" in order.data_type
    assert [child.name for child in order] == ["id", "line"]
    assert [child.name for child in order.data_type] == ["id", "line"]

    # Absence and the wrong key type report the same way on both.
    for node in (order, order.data_type):
        with pytest.raises(KeyError):
            node["absent"]
        with pytest.raises(IndexError):
            node[5]
        with pytest.raises(TypeError):
            node[object()]


def test_a_non_nested_datatype_subscripts_to_a_clear_error() -> None:
    scalar = Field("price", "float64")

    assert len(scalar) == 0
    with pytest.raises(KeyError):
        scalar["anything"]
    with pytest.raises(IndexError):
        scalar[0]
    with pytest.raises(KeyError):
        scalar.data_type["anything"]


def test_child_assignment_replaces_by_position_and_appends_by_unknown_name() -> None:
    row = Field(
        "row",
        DataType.from_fields([Field("id", "int64", nullable=False)]),
        nullable=False,
    )

    # An unknown name appends - dict-like, and how a schema is built up.
    row["venue"] = Field("venue", "utf8")
    assert [child.name for child in row] == ["id", "venue"]

    # A known name replaces in place, keeping its position.
    row["id"] = Field("id", "utf8", nullable=False)
    assert [child.name for child in row] == ["id", "venue"]
    assert row["id"].data_type == DataType("utf8")

    # A position replaces only, and never grows the node silently.
    row[1] = Field("venue", "large_utf8")
    assert row["venue"].data_type == DataType("large_utf8")
    with pytest.raises(IndexError):
        row[7] = Field("late", "int64")

    # Deleting by either form closes the gap.
    del row[0]
    assert [child.name for child in row] == ["venue"]
    row["extra"] = Field("extra", "int64")
    del row["venue"]
    assert [child.name for child in row] == ["extra"]
    with pytest.raises(KeyError):
        del row["gone"]


def test_hash_locks_all_field_equality_state() -> None:
    row = Field(
        "row",
        DataType.from_fields([Field("id", "int64", nullable=False)]),
        nullable=False,
    )
    keyed = {row: "stable"}

    with pytest.raises(TypeError, match="hashed Field is frozen"):
        row.metadata["owner"] = "tests"
    with pytest.raises(TypeError, match="hashed Field is frozen"):
        row["venue"] = Field("venue", "utf8")
    with pytest.raises(TypeError, match="hashed Field is frozen"):
        del row["id"]
    assert keyed[row] == "stable"

    copied = copy.copy(row)
    copied.metadata["owner"] = "tests"
    copied["venue"] = Field("venue", "utf8")
    assert copied.metadata["owner"] == "tests"
    assert "venue" in copied
    assert "venue" not in row

    restored = pickle.loads(pickle.dumps(row))
    restored.metadata["owner"] = "tests"
    restored["venue"] = Field("venue", "utf8")
    assert "venue" in restored


def test_a_datatype_is_a_read_only_child_collection() -> None:
    row = DataType.from_fields([Field("id", "int64", nullable=False)])

    # Reading is shared with `Field`; writing belongs on `Field`, which owns
    # the cache-aware mutation and stays hashable because a `DataType` does.
    with pytest.raises(TypeError):
        row["venue"] = Field("venue", "utf8")
    with pytest.raises(TypeError):
        del row["id"]


def test_metadata_is_not_reachable_by_subscript_but_is_through_the_view() -> None:
    row = Field(
        "row",
        DataType.from_fields([Field("id", "int64", nullable=False)]),
        nullable=False,
        metadata={"owner": "tests"},
    )

    # The old spelling reaches a child - or reports that there is none.
    with pytest.raises(KeyError):
        row["owner"]

    # The view is where a key means a key, and it is live in both directions.
    assert row.metadata["owner"] == "tests"
    assert dict(row.metadata.items()) == {"owner": "tests"}
    row.metadata["venue"] = "XPAR"
    assert row.metadata["venue"] == "XPAR"
    row.metadata.update({"owner": "core"})
    assert row.metadata["owner"] == "core"
    del row.metadata["venue"]
    assert "venue" not in row.metadata


def test_subscripting_a_schema_node_reaches_a_nested_child() -> None:
    line = Field(
        "line",
        DataType.from_fields(
            [Field("price", "float64", nullable=False), Field("qty", "int64")]
        ),
        nullable=False,
    )
    order = Field(
        "order",
        DataType.from_fields(
            [Field("id", "int64", nullable=False), line, Field("tags", "list(tag: utf8)")]
        ),
        nullable=False,
        metadata={"owner": "trading"},
    )

    # By name and by position, on the Field and on the DataType alike - one
    # shared semantic, so a caller walking the graph never gets two answers.
    assert order["id"].data_type == DataType("int64")
    assert order[0].name == "id"
    assert order[-1].name == "tags"
    assert order.data_type["id"].data_type == DataType("int64")
    assert order.data_type[1].name == "line"

    # Chained descent, including through a List item.
    assert order["line"]["price"].data_type == DataType("float64")
    assert order["tags"][0].name == "tag"

    # Children answer len, iteration, and membership on both classes.
    assert len(order) == 3
    assert len(order.data_type) == 3
    assert [child.name for child in order] == ["id", "line", "tags"]
    assert [child.name for child in order.data_type] == ["id", "line", "tags"]
    assert "line" in order
    assert "owner" not in order

    # Metadata is not reachable by subscript any more, and is still reachable
    # through its view and the named accessor.
    with pytest.raises(KeyError):
        order["owner"]
    assert order.metadata["owner"] == "trading"
    assert order.metadata.get("owner") == "trading"


def test_child_access_raises_the_exact_types_on_both_classes() -> None:
    row = Field(
        "row",
        DataType.from_fields([Field("id", "int64", nullable=False)]),
        nullable=False,
    )

    for node in (row, row.data_type):
        with pytest.raises(KeyError):
            node["absent"]
        with pytest.raises(IndexError):
            node[3]
        with pytest.raises(IndexError):
            node[-2]
        with pytest.raises(TypeError):
            node[object()]

    # A non-nested datatype subscripts to a clear error rather than a crash.
    with pytest.raises(KeyError):
        DataType("int64")["anything"]
    with pytest.raises(IndexError):
        DataType("int64")[0]


def test_child_mutation_replaces_by_position_and_appends_by_unknown_name() -> None:
    row = Field(
        "row",
        DataType.from_fields(
            [Field("id", "int64", nullable=False), Field("venue", "utf8", nullable=False)]
        ),
        nullable=False,
    )

    # A known name replaces in place, keeping its position.
    row["id"] = Field("id", "utf8", nullable=False)
    assert len(row) == 2
    assert row[0].name == "id"
    assert row["id"].data_type == DataType("utf8")

    # An unknown name appends - dict-like, and how a schema gets built up.
    row["price"] = Field("price", "float64")
    assert len(row) == 3
    assert row[2].name == "price"

    # A position replaces only; past the end is an IndexError, never a grow.
    row[1] = Field("venue", "int32", nullable=False)
    assert len(row) == 3
    assert row["venue"].data_type == DataType("int32")
    with pytest.raises(IndexError):
        row[9] = Field("late", "int64")
    assert len(row) == 3

    # Deleting closes the gap, by either key form.
    del row["id"]
    assert row[0].name == "venue"
    del row[0]
    assert [child.name for child in row] == ["price"]
    with pytest.raises(KeyError):
        del row["absent"]

    # A DataType is a read-only child collection: reading is shared, writing
    # belongs on the Field that owns the cache-aware mutation.
    with pytest.raises(TypeError):
        row.data_type["price"] = Field("price", "int64")
    with pytest.raises(TypeError):
        del row.data_type["price"]


def test_the_three_formats_share_one_structural_model() -> None:
    field = Field(
        "order",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field(
                "line",
                DataType.from_fields([Field("price", "float64", nullable=False)]),
                nullable=True,
            ),
        ]),
        nullable=False,
        metadata={"owner": "trading"},
    )

    # One `dict` model, three writers over it - so the three agree by
    # construction rather than by three sets of tests.
    assert Field.from_dict(field.into_dict()) == field
    assert Field.from_json(field.into_json()) == field
    assert Field.from_yaml(field.into_yaml()) == field
    assert Field.from_toml(field.into_toml()) == field

    assert DataType.from_dict(field.data_type.into_dict()) == field.data_type
    assert DataType.from_yaml(field.data_type.into_yaml()) == field.data_type
    assert DataType.from_toml(field.data_type.into_toml()) == field.data_type

    # The mapping is a plain dict a caller can build and edit.
    shape = field.into_dict()
    assert isinstance(shape, dict)
    assert shape["name"] == "order"
    assert shape["data_type"]["type"] == "struct"
    # Unset optional attributes are absent rather than null.
    assert "dictionary_id" not in shape


def test_indent_lays_out_bytes_without_changing_meaning() -> None:
    field = Field(
        "row",
        DataType.from_fields([Field("id", "int64", nullable=False)]),
        nullable=False,
    )

    # JSON: compact by default, `json.dumps(indent=n)` on request.
    assert "\n" not in field.into_json()
    assert field.into_json(indent=2).startswith('{\n  "name": "row",')
    assert field.into_json(indent=4).startswith('{\n    "name": "row",')

    # YAML: block style at two spaces by default, flow style only on request.
    assert field.into_yaml().startswith("name: row\ndata_type:\n  type: struct")
    assert field.into_yaml(indent=4).startswith("name: row\ndata_type:\n    type: struct")
    assert field.into_yaml(indent=None).startswith("{name: row,")

    # Round-trip and idempotence, per format per setting.
    for dump, parse in (
        (field.into_json, Field.from_json),
        (field.into_yaml, Field.from_yaml),
        (field.into_toml, Field.from_toml),
    ):
        for indent in (None, 2, 4):
            text = dump(indent=indent)
            assert parse(text) == field
            assert dump(indent=indent) == text


def test_str_and_repr_are_unchanged_and_pretty_is_the_readable_form() -> None:
    field = Field(
        "order",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field(
                "line",
                DataType.from_fields([Field("price", "float64", nullable=False)]),
                nullable=True,
            ),
        ]),
        nullable=False,
    )

    # `repr` stays the eval-round-trip form Python expects.
    assert repr(field).startswith("Field.from_str(")
    assert eval(repr(field)) == field  # noqa: S307 - the point of `repr`
    assert Field.from_str(str(field)) == field
    assert "\n" not in str(field)

    # `pretty` is the readable form: one fact per line, one indent per level.
    assert field.pretty() == (
        "order: struct[2], required\n"
        "  id: int64, required\n"
        "  line: struct[1], nullable\n"
        "    price: float64, required"
    )
    assert field.data_type.pretty().startswith("struct[2]")
    # Stable across runs.
    assert field.pretty() == field.pretty()
