"""Canonical values and live views must not share Python hash semantics."""

from __future__ import annotations

import copy
import io
import pathlib
import pickle

import pytest

from yggdryl import (
    DataType,
    Expression,
    Field,
    IOBase,
    MediaType,
    MimeType,
    RecordOptions,
    Statement,
    Timezone,
    Uri,
    Scalar,
    _native,
)
from yggdryl.iceberg import Catalog, PartitionSpec, Table


def assert_unhashable(value: object) -> None:
    """Assert the type deliberately opts out instead of inheriting identity hash."""

    assert type(value).__hash__ is None
    with pytest.raises(TypeError, match="unhashable"):
        hash(value)


def test_canonical_values_remain_hashable_by_native_identity() -> None:
    values = [
        DataType("int64"),
        MimeType.JSON,
        Timezone.UTC,
        Scalar.from_py({"id": 1}),
        Expression("id + 1"),
        Statement("select id"),
        PartitionSpec.unpartitioned(),
    ]

    for value in values:
        assert isinstance(value.stable_hash(), int)
        assert hash(value) == hash(value)
        assert {value: "kept"}[value] == "kept"


def test_mutable_identity_wrappers_hash_lock_instead_of_becoming_unhashable() -> None:
    field = Field("id", "int64", nullable=False)
    media_type = MediaType("application/json")
    uri = Uri("https://example.com/data.json")
    record_options = RecordOptions("trades.arrows")

    for value in (field, media_type, uri, record_options):
        assert isinstance(value.stable_hash(), int)
        assert isinstance(hash(value), int)

    with pytest.raises(TypeError, match="hashed"):
        field.set_alias("event_id")
    with pytest.raises(TypeError, match="hashed"):
        media_type.push_encoding(MimeType.GZIP)
    with pytest.raises(TypeError, match="hashed"):
        uri.set_extension("parquet")
    with pytest.raises(TypeError, match="hashed"):
        record_options.safe = True


@pytest.mark.parametrize(
    ("media_type", "attribute", "value"),
    [
        ("trades.arrows", "field", Field("row", DataType.from_fields([]), False)),
        ("trades.arrows", "name", "records"),
        ("trades.arrows", "dtype", "struct<id: int64>"),
        ("trades.arrows", "metadata", {"owner": "tests"}),
        ("trades.arrows", "safe", True),
        ("trades.arrows", "batch_row_size", 32),
        ("trades.arrows", "commit_row_size", 64),
        ("trades.arrows", "max_row_size", 128),
        ("trades.arrows", "max_byte_size", 4096),
        ("trades.arrows", "level", 6),
        ("trades.arrows", "merge_by_names", ["id"]),
        ("trades.arrows", "select_by_names", ["id"]),
        ("trades.arrows", "filter_partitions", [("venue", "XNAS")]),
        ("trades.avro", "block_codec", "null"),
        ("trades.avro", "sync_marker", b"0123456789abcdef"),
        ("trades.parquet", "compression", "zstd(3)"),
        ("trades.parquet", "max_row_group_size", 128),
        ("trades.parquet", "key_value_metadata", [("owner", "tests")]),
    ],
)
def test_every_record_options_setter_rejects_mutation_after_hash(
    media_type: str,
    attribute: str,
    value: object,
) -> None:
    options = RecordOptions(media_type)
    hash(options)

    with pytest.raises(TypeError, match="hashed RecordOptions"):
        setattr(options, attribute, value)


@pytest.mark.parametrize(
    "media_type",
    ["trades.arrows", "trades.avro", "trades.parquet", MimeType.PLAIN_TEXT],
)
def test_record_options_value_protocols_preserve_each_variant(
    media_type: str | MimeType,
) -> None:
    options = RecordOptions(media_type)
    options.field = Field(
        "row",
        DataType.from_fields([Field("id", "int64", nullable=False)]),
        nullable=False,
    )
    options.name = "records"
    options.metadata = {"owner": "tests"}
    options.safe = True
    options.batch_row_size = 32
    options.commit_row_size = 64
    options.max_row_size = 128
    options.max_byte_size = 4096
    options.level = 6
    options.merge_by_names = ["id"]
    options.select_by_names = ["id"]
    options.filter_partitions = [("venue", "XNAS")]
    if options.block_codec is not None:
        options.block_codec = "null"
        options.sync_marker = b"0123456789abcdef"
    if options.compression is not None:
        options.compression = "zstd(3)"
        options.max_row_group_size = 128
        options.key_value_metadata = [("owner", "tests"), ("owner", "core")]

    represented = eval(
        repr(options),
        {"DataType": DataType, "Field": Field, "RecordOptions": RecordOptions},
    )
    restored = pickle.loads(pickle.dumps(options))
    copied = copy.copy(options)
    deep = copy.deepcopy(options)

    for rebuilt in (represented, restored, copied, deep):
        assert rebuilt == options
        assert rebuilt.stable_hash() == options.stable_hash()
        assert rebuilt <= options and rebuilt >= options

    assert options != object()
    assert {options: "kept"}[restored] == "kept"

    # The transient built-in hash lock never crosses a copy or pickle boundary.
    for rebuilt in (
        eval(
            repr(options),
            {"DataType": DataType, "Field": Field, "RecordOptions": RecordOptions},
        ),
        pickle.loads(pickle.dumps(options)),
        copy.copy(options),
        copy.deepcopy(options),
    ):
        rebuilt.safe = False
        assert rebuilt != options


def test_record_options_stable_hash_does_not_lock_mutation() -> None:
    options = RecordOptions("trades.arrows")
    before = options.stable_hash()

    options.safe = True

    assert options.stable_hash() != before


def test_operational_handles_views_and_iterators_are_explicitly_unhashable(
    tmp_path: pathlib.Path,
) -> None:
    handle = IOBase.from_bytes(b"one\ntwo\n")
    field = Field("row", DataType.from_fields([Field("id", "int64")]))
    catalog = Catalog(tmp_path)
    namespace = catalog.namespace("sales")
    text_path = tmp_path / "lines.txt"
    text_path.write_bytes(b"one\ntwo\n")

    operational = [
        handle,
        handle.cursor(),
        handle.pstream_bytes(batch_size=2),
        IOBase(text_path).read_lines(),
        IOBase(tmp_path).iterdir(),
        iter(Scalar.from_py([1, 2])),
        Scalar.from_py({"id": 1}).items(),
        iter(field.dtype),
        field.dtype.show_diffs(DataType.from_fields([])),
        iter(MediaType.from_parts(MimeType.JSON, [MimeType.GZIP])),
        iter(Uri("https://example.com/a/data.json")),
        field.metadata,
        iter(field.metadata),
        field.iceberg,
        field.iceberg.keys(),
        _native._codec_decode_iter(io.BytesIO(b"1 2"), "json"),
        catalog,
        namespace,
        catalog.namespaces,
        namespace.tables,
        catalog.namespaces.keys(),
        catalog.namespaces.values(),
        namespace.tables.values(),
    ]

    for value in operational:
        assert_unhashable(value)


def test_table_and_schema_update_are_live_and_unhashable(
    tmp_path: pathlib.Path,
) -> None:
    schema = Field(
        "row",
        DataType.from_fields([Field("id", "int64", nullable=False)]),
        nullable=False,
    )
    table = Table.create(IOBase(tmp_path), schema)

    assert_unhashable(table)
    assert_unhashable(table.update_schema())
