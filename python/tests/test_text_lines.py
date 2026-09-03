"""Plain text uses the ordinary record-media surface."""

from __future__ import annotations

import copy
import datetime
import gzip as stdlib_gzip
import pathlib
import pickle

import pyarrow as pa
import pytest

from yggdryl import DataType, IOBase, RecordOptions, Timezone

HEADER = r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)"


def text_options() -> RecordOptions:
    return RecordOptions("text/plain")


def handle(tmp_path: pathlib.Path, data: bytes, name: str = "app.log") -> IOBase:
    target = tmp_path / name
    target.write_bytes(data)
    return IOBase(target)


def test_text_options_are_flat_validated_values() -> None:
    options = text_options()
    options.header = HEADER
    options.lstrip = r"^\s+"
    options.rstrip = r"\s+$"
    options.linesep = r"\r\n"
    options.autotype = False
    options.timezone = "+02:00"
    options.batch_row_size = 7

    assert options.header == HEADER
    assert options.lstrip == r"^\s+"
    assert options.rstrip == r"\s+$"
    assert options.linesep == b"\r\n"
    assert options.autotype is False
    assert options.timezone == Timezone("+02:00")
    assert options.batch_row_size == 7

    for rebuilt in (copy.copy(options), copy.deepcopy(options), pickle.loads(pickle.dumps(options))):
        assert rebuilt == options
        assert rebuilt.stable_hash() == options.stable_hash()

    with pytest.raises(ValueError, match="distinct from url, rownum, and body"):
        options.header = r"(?<body>.+)"
    with pytest.raises(ValueError, match="valid byte regex"):
        options.lstrip = "("

    arrow = RecordOptions("application/vnd.apache.arrow.stream")
    assert arrow.autotype is None
    with pytest.raises(ValueError, match="text"):
        arrow.autotype = True


def test_generic_records_have_base_columns_adaptive_captures_and_binary_body(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(
        tmp_path,
        b"  [INFO] id=7 first  \r\n[WARN] id=9 second\nplain\r",
    )
    options = text_options()
    options.header = HEADER
    options.lstrip = r"^\s+"
    options.rstrip = r"\s+$"

    reader = source.read_arrow_reader(options=options)
    assert isinstance(reader, pa.RecordBatchReader)
    assert reader.schema.names == ["url", "rownum", "body", "level", "id"]
    assert reader.schema.field("url").type == pa.string()
    assert reader.schema.field("rownum").type == pa.int64()
    assert reader.schema.field("body").type == pa.binary()
    assert reader.schema.field("level").type == pa.string()
    assert reader.schema.field("id").type == pa.int64()

    table = reader.read_all()
    assert table.column("rownum").to_pylist() == [1, 2, 3]
    assert table.column("body").to_pylist() == [b"first", b"second", b"plain"]
    assert table.column("level").to_pylist() == ["INFO", "WARN", None]
    assert table.column("id").to_pylist() == [7, 9, None]
    assert all(url.endswith("app.log") for url in table.column("url").to_pylist())

    assert list(source.read_records(options=options)) == [
        {
            "url": table.column("url")[0].as_py(),
            "rownum": 1,
            "body": b"first",
            "level": "INFO",
            "id": 7,
        },
        {
            "url": table.column("url")[1].as_py(),
            "rownum": 2,
            "body": b"second",
            "level": "WARN",
            "id": 9,
        },
        {
            "url": table.column("url")[2].as_py(),
            "rownum": 3,
            "body": b"plain",
            "level": None,
            "id": None,
        },
    ]


def test_header_removal_and_stripping_are_independent_edge_operations(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"left [INFO] id=7 right --\n")
    options = text_options()
    options.header = HEADER
    options.lstrip = r"^left\s+"
    options.rstrip = r"\s+--$"

    row = next(source.read_records(options=options))
    assert row["body"] == b"right"
    assert row["level"] == "INFO"
    assert row["id"] == 7


def test_autotype_can_be_disabled_and_fixes_types_after_the_first_batch(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"1\nword\n", "values.txt")

    strings = text_options()
    strings.header = r"(?<value>\S+)"
    strings.autotype = False
    table = source.read_arrow_reader(options=strings).read_all()
    assert table.schema.field("value").type == pa.string()
    assert table.column("value").to_pylist() == ["1", "word"]

    adaptive = text_options()
    adaptive.header = r"(?<value>\S+)"
    adaptive.batch_row_size = 1
    reader = source.read_arrow_reader(options=adaptive)
    assert reader.schema.field("value").type == pa.int64()
    assert reader.read_next_batch().column("value").to_pylist() == [1]
    with pytest.raises(ValueError, match="inferred datatype int64"):
        reader.read_next_batch()


def test_timezone_is_used_only_for_autotyped_offset_free_timestamps(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"2024-02-01T00:00:00 event\n")
    options = text_options()
    options.header = r"(?<stamp>\S+)"
    options.timezone = "+02:00"

    table = source.read_arrow_reader(options=options).read_all()
    dtype = table.schema.field("stamp").type
    assert pa.types.is_timestamp(dtype)
    assert dtype.tz == "+02:00"
    value = table.column("stamp")[0].as_py()
    assert value.utcoffset() == datetime.timedelta(hours=2)
    assert value.replace(tzinfo=None) == datetime.datetime(2024, 2, 1)


def test_generic_record_writes_encode_only_binary_body(tmp_path: pathlib.Path) -> None:
    target = IOBase(tmp_path / "out.txt")
    options = text_options()

    target.overwrite_records(({"body": value} for value in (b"one", b"two")), options=options)
    target.append_records([{"body": b"three"}], options=options)
    assert target.read_bytes() == b"one\ntwo\nthree\n"
    assert [row["body"] for row in target.read_records(options=options)] == [
        b"one",
        b"two",
        b"three",
    ]

    with pytest.raises(ValueError, match="without its record terminator"):
        target.append_records([{"body": b"bad\nline"}], options=options)


def test_pinned_line_separator_round_trips_through_generic_records(
    tmp_path: pathlib.Path,
) -> None:
    target = IOBase(tmp_path / "rows.txt")
    options = text_options()
    options.linesep = r"\r\n"

    target.overwrite_records([{"body": b"one"}, {"body": b"two"}], options=options)
    assert target.read_bytes() == b"one\r\ntwo\r\n"
    assert [row["body"] for row in target.read_records(options=options)] == [
        b"one",
        b"two",
    ]


def test_folders_decode_each_leaf_and_restart_row_numbers(tmp_path: pathlib.Path) -> None:
    root = tmp_path / "logs"
    root.mkdir()
    (root / "a.log").write_bytes(b"[INFO] id=1 from a\n")
    (root / "b.log.gz").write_bytes(
        stdlib_gzip.compress(b"[WARN] id=2 from b\n")
    )
    options = text_options()
    options.header = HEADER
    options.lstrip = r"^\s+"

    rows = list(IOBase(root).read_records(options=options))
    assert [row["rownum"] for row in rows] == [1, 1]
    assert [row["body"] for row in rows] == [b"from a", b"from b"]
    assert [row["id"] for row in rows] == [1, 2]
    assert [pathlib.PurePosixPath(row["url"]).name for row in rows] == [
        "a.log",
        "b.log.gz",
    ]


def test_absence_and_zero_row_bounds_keep_an_adaptive_schema(
    tmp_path: pathlib.Path,
) -> None:
    options = text_options()
    options.header = HEADER
    reader = IOBase(tmp_path / "missing.log").read_arrow_reader(options=options)

    assert reader.schema.names == ["url", "rownum", "body", "level", "id"]
    assert reader.schema.field("level").type == pa.string()
    assert reader.schema.field("id").type == pa.string()
    assert reader.read_all().num_rows == 0

    options.max_row_size = 0
    reader = handle(tmp_path, b"[INFO] id=1 hidden\n").read_arrow_reader(options=options)
    assert reader.schema.names == ["url", "rownum", "body", "level", "id"]
    assert reader.read_all().num_rows == 0


def test_declared_text_field_uses_the_shared_projection_and_cast(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"[INFO] id=7 body\n")
    options = text_options()
    options.header = HEADER
    options.lstrip = r"^\s+"
    options.dtype = "struct<body: binary not null, id: int64>"

    field = source.read_arrow_field(options=options)
    assert field.dtype == DataType("struct<body: binary not null, id: int64>")
    assert list(source.read_records(options=options)) == [{"body": b"body", "id": 7}]
