"""Plain text uses the ordinary record-media surface."""

from __future__ import annotations

import copy
import datetime
import gzip as stdlib_gzip
import os
import pathlib
import pickle

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, IOBase, RecordOptions, TextOptions, Timezone
from yggdryl.coding import gzip, zstd
from yggdryl.media import Text

ROWHEADER = r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)"
MTIME = datetime.datetime(2026, 8, 14, 12, 34, 56, 789_000, tzinfo=datetime.timezone.utc)


def text_options() -> TextOptions:
    return TextOptions()


def dated(target: pathlib.Path) -> pathlib.Path:
    """Pin the modification time the `mtime` column reads, so a test asserts an
    instant rather than whenever it happened to run."""
    stamp = int(MTIME.timestamp()) * 1_000_000_000 + MTIME.microsecond * 1_000
    os.utime(target, ns=(stamp, stamp))
    return target


def handle(tmp_path: pathlib.Path, data: bytes, name: str = "app.log") -> IOBase:
    target = tmp_path / name
    target.write_bytes(data)
    return IOBase(dated(target))


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
    options.lstrip = [r"^\s+"]
    options.rstrip = [r"\s+$"]
    options.linesep = r"\r\n"
    options.autotype = False
    options.timezone = "+02:00"
    options.start_rownum = -3
    options.parse_mtime = False
    options.batch_row_size = 7

    assert options.framing is True
    assert options.leading_fragment == "drop"
    assert options.max_record_byte_size == 4096
    assert options.rowheader == ROWHEADER
    assert options.lstrip == [r"^\s+"]
    assert options.rstrip == [r"\s+$"]
    assert options.linesep == b"\r\n"
    assert options.autotype is False
    assert options.timezone == Timezone("+02:00")
    assert options.start_rownum == -3
    assert options.parse_mtime is False
    assert options.batch_row_size == 7

    for rebuilt in (copy.copy(options), copy.deepcopy(options), pickle.loads(pickle.dumps(options))):
        assert rebuilt == options
        assert rebuilt.stable_hash() == options.stable_hash()

    constructor, [state] = options.__reduce__()
    for name in (
        "framing",
        "leading_fragment",
        "max_record_byte_size",
        "parse_mtime",
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
        options.lstrip = ["("]
    with pytest.raises(TypeError, match="not bool"):
        options.start_rownum = True
    with pytest.raises(OverflowError):
        options.start_rownum = 1 << 63

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
    assert state["parse_mtime"] is True
    for name in ("framing", "leading_fragment", "max_record_byte_size", "parse_mtime"):
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
    options.start_rownum = 10
    options.lstrip = [r"^\s+"]
    options.rstrip = [r"\s+$"]

    reader = source.read_arrow_reader(options=options)
    assert isinstance(reader, pa.RecordBatchReader)
    assert reader.schema.names == ["url", "rownum", "mtime", "body", "level", "id"]
    # A location is its own datatype over Utf8 storage, and it is nullable
    # because a handle that is nowhere has none to state.
    url_field = reader.schema.field("url")
    assert url_field.type == pa.string()
    assert url_field.nullable is True
    assert url_field.metadata[b"ARROW:extension:name"] == b"yggdryl.url"
    assert Field.from_arrow(url_field).dtype == DataType("url")
    assert reader.schema.field("rownum").type == pa.int64()
    assert reader.schema.field("mtime").type == pa.timestamp("ns", "UTC")
    assert reader.schema.field("body").type == pa.binary()
    assert reader.schema.field("level").type == pa.string()
    assert reader.schema.field("id").type == pa.int64()

    table = reader.read_all()
    assert table.column("rownum").to_pylist() == [10, 11, 12]
    assert table.column("mtime").to_pylist() == [MTIME, MTIME, MTIME]
    assert table.column("body").to_pylist() == [b"first", b"second", b"plain"]
    assert table.column("level").to_pylist() == ["INFO", "WARN", None]
    assert table.column("id").to_pylist() == [7, 9, None]
    # A located handle states the canonical URL of where it holds the bytes.
    located = str(source.url)
    assert located.startswith("file:///") and located.endswith("app.log")
    assert table.column("url").to_pylist() == [located] * 3

    assert list(source.read_records(options=options)) == [
        {
            "url": table.column("url")[0].as_py(),
            "rownum": 10,
            "mtime": MTIME,
            "body": b"first",
            "level": "INFO",
            "id": 7,
        },
        {
            "url": table.column("url")[1].as_py(),
            "rownum": 11,
            "mtime": MTIME,
            "body": b"second",
            "level": "WARN",
            "id": 9,
        },
        {
            "url": table.column("url")[2].as_py(),
            "rownum": 12,
            "mtime": MTIME,
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
    options.lstrip = [r"^left\s+"]
    options.rstrip = [r"\s+--$"]

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
    assert table.schema.names == ["url", "mtime", "body", "value"]
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


def test_mtime_dates_each_record_from_the_handle_that_holds_it(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"[INFO] id=7 first\n[WARN] id=9 second\n")
    options = text_options()
    options.rowheader = ROWHEADER
    options.start_rownum = 1

    # On by default, and pinned between the row number and the body.
    reader = source.read_arrow_reader(options=options)
    assert reader.schema.names == ["url", "rownum", "mtime", "body", "level", "id"]
    assert reader.schema.field("mtime") == pa.field(
        "mtime", pa.timestamp("ns", "UTC"), nullable=True
    )

    # No capture is spelled `mtime`, so the handle's own modification time is
    # read once and dates every row it answers with.
    assert reader.read_all().column("mtime").to_pylist() == [MTIME, MTIME]
    assert [row["mtime"] for row in source.read_records(options=options)] == [
        MTIME,
        MTIME,
    ]

    # A buffer was never written anywhere, so it has no such time to answer.
    buffered = IOBase.from_bytes(b"[INFO] id=7 first\n[WARN] id=9 second\n")
    table = buffered.read_arrow_reader(options=options).read_all()
    assert table.schema.names == ["url", "rownum", "mtime", "body", "level", "id"]
    assert table.column("mtime").to_pylist() == [None, None]
    # It is still somewhere - in memory - and that location is what it states.
    assert str(buffered.url).startswith("mem://")
    assert table.column("url").to_pylist() == [str(buffered.url)] * 2
    assert [row["mtime"] for row in buffered.read_records(options=options)] == [
        None,
        None,
    ]

    # Off, the column is gone and the rest of the field keeps its order.
    options.parse_mtime = False
    assert source.read_arrow_reader(options=options).schema.names == [
        "url",
        "rownum",
        "body",
        "level",
        "id",
    ]


def test_an_mtime_capture_owns_the_column_it_names(tmp_path: pathlib.Path) -> None:
    source = handle(
        tmp_path, b"2026-08-14T09:30:15 dated\nundated\n", "captured.log"
    )
    options = text_options()
    options.rowheader = r"^(?<mtime>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}) "
    options.timezone = Timezone.UTC
    captured = datetime.datetime(
        2026, 8, 14, 9, 30, 15, tzinfo=datetime.timezone.utc
    )

    # One column whichever answered: the capture fills the fixed column instead
    # of trailing beside it, and the line the header missed falls back to the
    # handle's own modification time.
    reader = source.read_arrow_reader(options=options)
    assert reader.schema.names == ["url", "mtime", "body"]
    assert reader.schema.field("mtime").type == pa.timestamp("ns", "UTC")
    assert reader.read_all().column("mtime").to_pylist() == [captured, MTIME]

    # Off, the name is an ordinary capture again: it trails the body, keeps the
    # resolution its own syntax declares, and is null wherever the header missed.
    options.parse_mtime = False
    reader = source.read_arrow_reader(options=options)
    assert reader.schema.names == ["url", "body", "mtime"]
    assert reader.schema.field("mtime").type == pa.timestamp("s", "UTC")
    assert reader.read_all().column("mtime").to_pylist() == [captured, None]


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

    # A `.log` name already composes to text; naming options replaces the
    # retained configuration rather than stacking a second wrapper, and the
    # handle it answers with is the one that carries them.
    assert isinstance(source, Text)
    configured = source.into_text(options)
    assert isinstance(configured, Text)
    [row] = list(configured.read_records())

    assert row["stamp"] == datetime.datetime(
        2026, 8, 29, 0, 0, 0, 434_958, tzinfo=datetime.timezone.utc
    )
    assert row["thread"] == "77-2f3e6ff7:9f4d2a08b1:128"
    assert row["module"] == "ModuleFailFastFilterChecker"
    assert row["level"] == "DEBUG"
    assert row["body"] == (
        b"Execution report (execId: 20260828180000369318, from session:"
    )
    # No capture is spelled `mtime`, so the file's own modification time dates
    # the row.
    assert row["mtime"] == MTIME


def test_generic_record_writes_encode_only_binary_body(tmp_path: pathlib.Path) -> None:
    target = IOBase(tmp_path / "out.txt")
    options = text_options()

    target.overwrite_records(({"body": value} for value in (b"one", b"two")), options=options)
    target.append_records([{"body": b"three"}], options=options)
    assert target.read_bytes() == b"one\ntwo\nthree\n"
    # The rows read back carry the modification time the writes just made.
    dated(tmp_path / "out.txt")
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
    dated(tmp_path / "rows.txt")
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
    base.start_rownum = 1
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
            "mtime",
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


def test_compressed_logs_stream_their_records_without_naming_the_coding(
    tmp_path: pathlib.Path,
) -> None:
    plain = b"[A] first\ncontinued\n[B] second\n"
    options = TextOptions()
    options.framing = True
    options.rowheader = r"^\[(?<kind>[A-Z])\] "
    options.batch_row_size = 1

    for name, encoded in (
        ("records.log.gz", gzip.dumps(plain)),
        ("records.log.zst", zstd.dumps(plain)),
    ):
        source = handle(tmp_path, encoded, name)
        reader = source.read_arrow_reader(options=options)
        assert reader.schema.names == ["url", "mtime", "body", "kind"]

        # The coding decodes as the batches are pulled, one record at a time.
        batches = list(reader)
        assert [batch.num_rows for batch in batches] == [1, 1]
        table = pa.Table.from_batches(batches)
        assert table.column("body").to_pylist() == [b"first\ncontinued", b"second"]
        assert table.column("kind").to_pylist() == ["A", "B"]

        # The property counts through the same decoded stream, under the
        # options the handle infers for itself: physical lines, unframed.
        assert source.row_size == 3

        # The decoded view reads the same records off the same bytes.
        coded = IOBase(tmp_path / name).into_coded()
        assert coded.read_arrow_reader(options=options).read_all().num_rows == 2


def test_an_empty_or_absent_compressed_log_keeps_its_schema(
    tmp_path: pathlib.Path,
) -> None:
    options = TextOptions()
    options.framing = True
    options.rowheader = r"^\[(?<kind>[A-Z])\] "

    for name, source in (
        ("empty.log.gz", handle(tmp_path, gzip.dumps(b""), "empty.log.gz")),
        ("absent.log.zst", IOBase(tmp_path / "absent.log.zst")),
    ):
        reader = source.read_arrow_reader(options=options)
        assert reader.schema.names == ["url", "mtime", "body", "kind"], name
        assert reader.read_all().num_rows == 0, name
        assert source.row_size == 0, name


def test_folders_decode_each_leaf_and_restart_row_numbers(tmp_path: pathlib.Path) -> None:
    root = tmp_path / "logs"
    root.mkdir()
    (root / "a.log").write_bytes(b"[INFO] id=1 from a\n")
    (root / "b.log.gz").write_bytes(
        stdlib_gzip.compress(b"[WARN] id=2 from b\n")
    )
    dated(root / "a.log")
    dated(root / "b.log.gz")
    options = text_options()
    options.rowheader = ROWHEADER
    options.start_rownum = 1
    options.lstrip = [r"^\s+"]

    rows = list(IOBase(root).read_records(options=options))
    assert [row["rownum"] for row in rows] == [1, 1]
    # Each leaf answers with its own modification time.
    assert [row["mtime"] for row in rows] == [MTIME, MTIME]
    assert [row["body"] for row in rows] == [b"from a", b"from b"]
    assert [row["id"] for row in rows] == [1, 2]
    assert [pathlib.PurePosixPath(row["url"]).name for row in rows] == [
        "a.log",
        "b.log.gz",
    ]
    # Each leaf states the canonical `file:` URL of where it was read from.
    assert [row["url"] for row in rows] == [
        (root / "a.log").as_uri(),
        (root / "b.log.gz").as_uri(),
    ]


def test_absence_and_zero_row_bounds_keep_the_regex_derived_schema(
    tmp_path: pathlib.Path,
) -> None:
    options = text_options()
    options.rowheader = ROWHEADER
    reader = IOBase(tmp_path / "missing.log").read_arrow_reader(options=options)

    assert reader.schema.names == ["url", "mtime", "body", "level", "id"]
    assert reader.schema.field("level").type == pa.string()
    assert reader.schema.field("id").type == pa.int64()
    assert reader.read_all().num_rows == 0

    options.max_row_size = 0
    reader = handle(tmp_path, b"[INFO] id=1 hidden\n").read_arrow_reader(options=options)
    assert reader.schema.names == ["url", "mtime", "body", "level", "id"]
    assert reader.read_all().num_rows == 0


def test_declared_text_field_uses_the_shared_projection_and_cast(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, b"[INFO] id=7 body\n")
    options = text_options()
    options.rowheader = ROWHEADER
    options.lstrip = [r"^\s+"]
    options.dtype = "struct<body: binary not null, id: int64>"

    field = source.read_arrow_field(options=options)
    assert field.dtype == DataType("struct<body: binary not null, id: int64>")
    assert list(source.read_records(options=options)) == [{"body": b"body", "id": 7}]


def test_a_nanosecond_modification_time_reaches_a_record_floored(
    tmp_path: pathlib.Path,
) -> None:
    # A filesystem dates a file to the nanosecond and `datetime` counts
    # microseconds, so the record path floors rather than refusing: the reading
    # a real file carries is worth more truncated than withheld.
    target = tmp_path / "odd.log"
    target.write_bytes(b"first\nsecond\n")
    stamp = int(MTIME.timestamp()) * 1_000_000_000 + MTIME.microsecond * 1_000 + 789
    os.utime(target, ns=(stamp, stamp))
    source = IOBase(target)

    # The batch path keeps every nanosecond of it, which is why the count and
    # not the `datetime` is what proves it.
    table = source.read_arrow_reader(options=TextOptions()).read_all()
    assert table.column("mtime").cast(pa.int64()).to_pylist() == [stamp, stamp]

    # The record path hands back the microsecond `datetime` holds.
    assert [row["mtime"] for row in source.read_records(options=TextOptions())] == [
        MTIME,
        MTIME,
    ]
