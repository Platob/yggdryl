"""Plain text uses the ordinary record-media surface."""

from __future__ import annotations

import copy
import datetime
import gzip as stdlib_gzip
import pathlib
import pickle

import pyarrow as pa
import pytest

from yggdryl import DataType, IOBase, RecordOptions, TextOptions, Timezone

ROWHEADER = r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)"


def text_options() -> TextOptions:
    return TextOptions()


def handle(tmp_path: pathlib.Path, data: bytes, name: str = "app.log") -> IOBase:
    target = tmp_path / name
    target.write_bytes(data)
    return IOBase(target)


def test_datatype_from_regex_is_the_shared_pre_read_schema_inference() -> None:
    inferred = DataType.from_regex(ROWHEADER)
    assert [field.name for field in inferred] == ["level", "id"]
    assert inferred["level"].dtype == DataType("utf8")
    assert inferred["id"].dtype == DataType("int64")
    assert all(field.nullable for field in inferred)

    text = DataType.from_regex(ROWHEADER, False)
    assert all(field.dtype == DataType("utf8") for field in text)
    with pytest.raises(ValueError, match="regular expression"):
        DataType.from_regex("(?<id>")


def test_text_options_are_flat_validated_values() -> None:
    options = text_options()
    assert options.framing is False
    assert options.leading_fragment == "keep"
    assert options.max_record_byte_size is None

    options.framing = True
    options.leading_fragment = "drop"
    options.max_record_byte_size = 4096
    options.rowheader = ROWHEADER
    options.lstrip = r"^\s+"
    options.rstrip = r"\s+$"
    options.linesep = r"\r\n"
    options.autotype = False
    options.timezone = "+02:00"
    options.with_rownum = -3
    options.batch_row_size = 7

    assert options.framing is True
    assert options.leading_fragment == "drop"
    assert options.max_record_byte_size == 4096
    assert options.rowheader == ROWHEADER
    assert options.lstrip == r"^\s+"
    assert options.rstrip == r"\s+$"
    assert options.linesep == b"\r\n"
    assert options.autotype is False
    assert options.timezone == Timezone("+02:00")
    assert options.with_rownum == -3
    assert options.batch_row_size == 7

    for rebuilt in (copy.copy(options), copy.deepcopy(options), pickle.loads(pickle.dumps(options))):
        assert rebuilt == options
        assert rebuilt.stable_hash() == options.stable_hash()

    constructor, [state] = options.__reduce__()
    for name in (
        "framing",
        "leading_fragment",
        "max_record_byte_size",
        "rowheader",
    ):
        incomplete = state.copy()
        incomplete.pop(name)
        with pytest.raises(ValueError, match=rf'missing "{name}"'):
            constructor(incomplete)

    with pytest.raises(
        ValueError, match="distinct from url, rownum, body, and dropped_byte_size"
    ):
        options.rowheader = r"(?<body>.+)"
    with pytest.raises(ValueError, match="expected one of keep, drop, error"):
        options.leading_fragment = "merge"
    assert options.leading_fragment == "drop"
    with pytest.raises(ValueError, match="valid byte regex"):
        options.lstrip = "("
    with pytest.raises(TypeError, match="not bool"):
        options.with_rownum = True
    with pytest.raises(OverflowError):
        options.with_rownum = 1 << 63

    arrow = RecordOptions("application/vnd.apache.arrow.stream")
    assert not hasattr(arrow, "autotype")
    with pytest.raises(ValueError, match="text"):
        arrow.timezone = Timezone.UTC

    generic_text = RecordOptions("text/plain")
    generic_text.timezone = Timezone.UTC
    assert generic_text.timezone == Timezone.UTC
    constructor, [state] = generic_text.__reduce__()
    assert state["framing"] is False
    assert state["leading_fragment"] == "keep"
    assert state["max_record_byte_size"] is None
    for name in ("framing", "leading_fragment", "max_record_byte_size"):
        incomplete = state.copy()
        incomplete.pop(name)
        with pytest.raises(ValueError, match=rf'missing "{name}"'):
            constructor(incomplete)


def test_generic_records_have_optional_rownums_regex_types_and_binary_body(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(
        tmp_path,
        b"  [INFO] id=7 first  \r\n[WARN] id=9 second\nplain\r",
    )
    options = text_options()
    options.rowheader = ROWHEADER
    options.with_rownum = 10
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
    assert table.column("rownum").to_pylist() == [10, 11, 12]
    assert table.column("body").to_pylist() == [b"first", b"second", b"plain"]
    assert table.column("level").to_pylist() == ["INFO", "WARN", None]
    assert table.column("id").to_pylist() == [7, 9, None]
    assert all(url.endswith("app.log") for url in table.column("url").to_pylist())

    assert list(source.read_records(options=options)) == [
        {
            "url": table.column("url")[0].as_py(),
            "rownum": 10,
            "body": b"first",
            "level": "INFO",
            "id": 7,
        },
        {
            "url": table.column("url")[1].as_py(),
            "rownum": 11,
            "body": b"second",
            "level": "WARN",
            "id": 9,
        },
        {
            "url": table.column("url")[2].as_py(),
            "rownum": 12,
            "body": b"plain",
            "level": None,
            "id": None,
        },
    ]


def test_rowheader_removal_and_stripping_are_independent_edge_operations(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"left [INFO] id=7 right --\n")
    options = text_options()
    options.rowheader = ROWHEADER
    options.lstrip = r"^left\s+"
    options.rstrip = r"\s+--$"

    row = next(source.read_records(options=options))
    assert row["body"] == b"right"
    assert row["level"] == "INFO"
    assert row["id"] == 7


def test_capture_types_come_from_regex_before_any_row_is_read(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"1\n2\n", "values.txt")

    strings = text_options()
    strings.rowheader = r"(?<value>\d+)"
    strings.autotype = False
    table = source.read_arrow_reader(options=strings).read_all()
    assert table.schema.field("value").type == pa.string()
    assert table.column("value").to_pylist() == ["1", "2"]

    typed = text_options()
    typed.rowheader = r"(?<value>\d+)"
    missing = IOBase(tmp_path / "missing.log")
    assert missing.read_arrow_field(options=typed)["value"].dtype == DataType("int64")
    reader = source.read_arrow_reader(options=typed)
    assert reader.schema.field("value").type == pa.int64()
    assert reader.read_all().column("value").to_pylist() == [1, 2]

    broad = text_options()
    broad.rowheader = r"(?<value>\S+)"
    table = handle(tmp_path, b"1\nword\n", "broad.txt").read_arrow_reader(
        options=broad
    ).read_all()
    assert table.schema.names == ["url", "body", "value"]
    assert table.column("value").to_pylist() == ["1", "word"]


def test_timezone_is_used_only_for_autotyped_offset_free_timestamps(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"2024-02-01T00:00:00 event\n")
    options = text_options()
    options.rowheader = (
        r"(?<stamp>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})"
    )
    options.timezone = "+02:00"

    table = source.read_arrow_reader(options=options).read_all()
    dtype = table.schema.field("stamp").type
    assert pa.types.is_timestamp(dtype)
    assert dtype.tz == "+02:00"
    value = table.column("stamp")[0].as_py()
    assert value.utcoffset() == datetime.timedelta(hours=2)
    assert value.replace(tzinfo=None) == datetime.datetime(2024, 2, 1)


def test_retained_text_options_parse_the_real_execution_row(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(
        tmp_path,
        b"2026-08-29 00:00:00.434_958 "
        b"[77-2f3e6ff7:9f4d2a08b1:128] "
        b"[ModuleFailFastFilterChecker] (DEBUG) Execution report "
        b"(execId: 20260828180000369318, from session:\n",
        "execution.log",
    )
    options = TextOptions()
    options.rowheader = (
        r"^(?<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}_\d{3}) "
        r"\[(?<thread>[^]]+)\] \[(?<module>[^]]+)\] \((?<level>[A-Z]+)\) "
    )
    options.timezone = Timezone.UTC

    assert source.into_text(options) is source
    assert source.into_text() is source
    [row] = list(source.read_records())

    assert row["stamp"] == datetime.datetime(
        2026, 8, 29, 0, 0, 0, 434_958, tzinfo=datetime.timezone.utc
    )
    assert row["thread"] == "77-2f3e6ff7:9f4d2a08b1:128"
    assert row["module"] == "ModuleFailFastFilterChecker"
    assert row["level"] == "DEBUG"
    assert row["body"] == (
        b"Execution report (execId: 20260828180000369318, from session:"
    )


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


def test_framing_normalizes_terminators_and_reports_record_caps(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(
        tmp_path, b"leading\r\n[A] abc\ndef\r\n[B] xyz\r", "framed.log"
    )
    base = TextOptions()
    base.framing = True
    base.leading_fragment = "drop"
    base.rowheader = r"^\[(?<kind>[A-Z])\] "
    base.with_rownum = 1
    base.batch_row_size = 1

    for limit, expected_bodies, expected_dropped in (
        (7, [b"abc\ndef", b"xyz"], [None, None]),
        (6, [b"abc\nde", b"xyz"], [1, None]),
        (0, [b"", b""], [7, 3]),
    ):
        options = copy.copy(base)
        options.max_record_byte_size = limit
        reader = source.read_arrow_reader(options=options)
        assert reader.schema.names == [
            "url",
            "rownum",
            "body",
            "dropped_byte_size",
            "kind",
        ]
        batches = list(reader)
        assert [batch.num_rows for batch in batches] == [1, 1]
        table = pa.Table.from_batches(batches)
        assert table.column("body").to_pylist() == expected_bodies
        assert table.column("dropped_byte_size").to_pylist() == expected_dropped
        assert table.column("rownum").to_pylist() == [2, 4]
        assert table.column("kind").to_pylist() == ["A", "B"]

    rejected = copy.copy(base)
    rejected.leading_fragment = "error"
    with pytest.raises(ValueError, match="leading physical line"):
        source.read_arrow_reader(options=rejected).read_all()


def test_folders_decode_each_leaf_and_restart_row_numbers(tmp_path: pathlib.Path) -> None:
    root = tmp_path / "logs"
    root.mkdir()
    (root / "a.log").write_bytes(b"[INFO] id=1 from a\n")
    (root / "b.log.gz").write_bytes(
        stdlib_gzip.compress(b"[WARN] id=2 from b\n")
    )
    options = text_options()
    options.rowheader = ROWHEADER
    options.with_rownum = 1
    options.lstrip = r"^\s+"

    rows = list(IOBase(root).read_records(options=options))
    assert [row["rownum"] for row in rows] == [1, 1]
    assert [row["body"] for row in rows] == [b"from a", b"from b"]
    assert [row["id"] for row in rows] == [1, 2]
    assert [pathlib.PurePosixPath(row["url"]).name for row in rows] == [
        "a.log",
        "b.log.gz",
    ]


def test_absence_and_zero_row_bounds_keep_the_regex_derived_schema(
    tmp_path: pathlib.Path,
) -> None:
    options = text_options()
    options.rowheader = ROWHEADER
    reader = IOBase(tmp_path / "missing.log").read_arrow_reader(options=options)

    assert reader.schema.names == ["url", "body", "level", "id"]
    assert reader.schema.field("level").type == pa.string()
    assert reader.schema.field("id").type == pa.int64()
    assert reader.read_all().num_rows == 0

    options.max_row_size = 0
    reader = handle(tmp_path, b"[INFO] id=1 hidden\n").read_arrow_reader(options=options)
    assert reader.schema.names == ["url", "body", "level", "id"]
    assert reader.read_all().num_rows == 0


def test_declared_text_field_uses_the_shared_projection_and_cast(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"[INFO] id=7 body\n")
    options = text_options()
    options.rowheader = ROWHEADER
    options.lstrip = r"^\s+"
    options.dtype = "struct<body: binary not null, id: int64>"

    field = source.read_arrow_field(options=options)
    assert field.dtype == DataType("struct<body: binary not null, id: int64>")
    assert list(source.read_records(options=options)) == [{"body": b"body", "id": 7}]
