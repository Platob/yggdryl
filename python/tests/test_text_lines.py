"""The text-line surface: readers described by configuration, not by code."""

from __future__ import annotations

import pathlib

import pytest

from yggdryl import DataType, IOBase, field_from_pattern, yaml

PATTERN = r"^(?<stamp>\S+) \[(?<level>[A-Z]+)\]"

LOG = (
    "2024-02-01T10:00:00 [ERROR] boom\n"
    "\tat Handler.invoke(Handler.java:42)\n"
    "2024-02-01T10:00:01 [INFO] fine\n"
)


def handle(tmp_path: pathlib.Path, text: str, name: str = "app.log") -> IOBase:
    target = tmp_path / name
    # Keep line terminators byte-exact on Windows as well as POSIX: these
    # tests exercise CR, LF, and CRLF independently, so host newline
    # translation would change the fixture before yggdryl sees it.
    target.write_bytes(text.encode())
    return IOBase(target)


def test_records_read_as_text_and_group_by_the_pattern(tmp_path: pathlib.Path) -> None:
    records = list(handle(tmp_path, LOG).read_lines(PATTERN))
    assert len(records) == 2
    # The stack trace joined its entry rather than becoming a record.
    assert "Handler.java" in records[0]


def test_the_terminator_is_flexible_unset_and_exact_when_pinned(
    tmp_path: pathlib.Path,
) -> None:
    mixed = "lf\ncrlf\r\ncr\rlast"
    assert list(handle(tmp_path, mixed).read_lines()) == ["lf", "crlf", "cr", "last"]

    # Pinned, a lone `\n` is content rather than a break.
    assert list(handle(tmp_path, mixed).read_lines(linesep=r"\r\n")) == [
        "lf\ncrlf",
        "cr\rlast",
    ]


def test_writing_is_deterministic_and_round_trips(tmp_path: pathlib.Path) -> None:
    target = IOBase(tmp_path / "out.log")
    # An iterable, never a list the binding materializes first.
    target.write_lines(f"row-{index}" for index in range(1_000))
    target.append_lines(["tail"])

    assert target.read_bytes().endswith(b"row-999\ntail\n")
    records = list(target.read_lines())
    assert len(records) == 1_001
    assert records[-1] == "tail"

    # A pinned terminator is written verbatim and read back exactly.
    pinned = IOBase(tmp_path / "crlf.log")
    pinned.write_lines(["one", "two"], linesep=r"\r\n")
    assert pinned.read_bytes() == b"one\r\ntwo\r\n"
    assert list(pinned.read_lines(linesep=r"\r\n")) == ["one", "two"]


def test_a_reader_is_fully_described_by_a_configuration_document(
    tmp_path: pathlib.Path,
) -> None:
    # No Rust, no Python callbacks, no per-row Python: a document is the reader.
    document = """
pattern: '^(?<stamp>\\S+) \\[(?<level>[A-Z]+)\\]'
byte_size: 1048576
batch_size: 4096
rstrip: ascii
timestamp_capture: stamp
capture_types:
  level: utf8
custom_fields:
  source: gateway
"""
    options = yaml.loads(document)

    # The schema answers from the document alone, with no resource in sight -
    # so the table exists before the first log line does.
    schema = field_from_pattern(options=options)
    assert schema.name == "row"
    assert schema["level"].data_type == DataType("utf8")
    assert schema["source"].data_type == DataType("utf8")

    reader = handle(tmp_path, LOG).read_arrow_lines(options=options)
    # The reader emits exactly the schema the builder answered from the
    # document, so the table can be created before any resource exists.
    assert reader.schema.names == [field.name for field in schema]
    table = reader.read_all()
    assert table.num_rows == 2
    assert table.column("level").to_pylist() == ["ERROR", "INFO"]
    assert table.column("source").to_pylist() == ["gateway", "gateway"]
    assert table.column("message").to_pylist()[1] == "fine"


def test_keywords_refine_a_document_and_both_validate_the_same_way(
    tmp_path: pathlib.Path,
) -> None:
    reader = handle(tmp_path, LOG).read_arrow_lines(PATTERN, batch_size=1)
    assert [batch.num_rows for batch in reader] == [1, 1]

    with pytest.raises(ValueError, match="a known option"):
        handle(tmp_path, LOG).read_arrow_lines(options={"batch-size": 1})
    with pytest.raises(ValueError, match="registry knows"):
        handle(tmp_path, LOG).read_arrow_lines(PATTERN, timezone="Not/AZone")


def test_log_mode_needs_no_expression_anywhere(tmp_path: pathlib.Path) -> None:
    table = handle(tmp_path, LOG).read_arrow_lines(logs=True).read_all()
    assert table.num_rows == 2
    # The fixed, always-emitted token columns.
    assert table.column("level").to_pylist() == ["ERROR", "INFO"]
    assert table.column("logger").to_pylist() == [None, None]
    # And the schema is answerable from the options alone.
    assert field_from_pattern(logs=True)["level"].data_type == DataType("utf8")


def test_both_batch_bounds_apply_and_the_first_to_trip_wins(
    tmp_path: pathlib.Path,
) -> None:
    text = "".join(f"2024-02-01T10:00:00 [INFO] row {index}\n" for index in range(50))
    source = handle(tmp_path, text)

    by_rows = source.read_arrow_lines(PATTERN, batch_size=10)
    assert [batch.num_rows for batch in by_rows] == [10] * 5

    # `byte_size` counts decoded *input* bytes, not Arrow buffer memory.
    by_bytes = source.read_arrow_lines(PATTERN, byte_size=100)
    counts = [batch.num_rows for batch in by_bytes]
    assert sum(counts) == 50
    assert max(counts) < 10


def test_a_zone_makes_unix_a_real_instant_and_unset_changes_nothing(
    tmp_path: pathlib.Path,
) -> None:
    source = handle(tmp_path, "2024-02-01T00:00:00 [INFO] x\n")
    naive = source.read_arrow_lines(PATTERN).read_all().column("unix").to_pylist()
    zoned = (
        source.read_arrow_lines(PATTERN, timezone="+02:00")
        .read_all()
        .column("unix")
        .to_pylist()
    )
    assert naive == [1_706_745_600_000_000_000]
    assert zoned == [naive[0] - 2 * 3_600 * 1_000_000_000]


def test_a_folder_of_mixed_codings_reads_uniformly(tmp_path: pathlib.Path) -> None:
    import gzip as stdlib_gzip

    root = tmp_path / "logs"
    root.mkdir()
    (root / "a.log").write_text("2024-02-01T10:00:00 [INFO] from a\n")
    (root / "b.log.gz").write_bytes(
        stdlib_gzip.compress(b"2024-02-01T11:00:00 [WARN] from b\n")
    )

    table = IOBase(root).read_arrow_lines(PATTERN).read_all()
    # Each leaf decoded by its own media type; nothing named a codec.
    assert table.column("message").to_pylist() == ["from a", "from b"]
    # `rownum` restarts per leaf, and each row names its own resource.
    assert table.column("rownum").to_pylist() == [1, 1]
    assert len({url for url in table.column("url").to_pylist()}) == 2


def test_an_absent_resource_reads_as_empty_with_the_schema_still_answered(
    tmp_path: pathlib.Path,
) -> None:
    reader = IOBase(tmp_path / "never.log").read_arrow_lines(PATTERN)
    assert reader.schema.field("url").name == "url"
    assert reader.read_all().num_rows == 0
