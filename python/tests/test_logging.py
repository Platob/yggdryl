"""What the native core reports to `logging`, and what it deliberately does not."""

from __future__ import annotations

import logging
import pathlib

import pyarrow as pa
import pytest

from yggdryl import IOBase, refresh_logging
from yggdryl.media.iceberg import Table, assign_field_ids

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ]
)


def _round_trip(root: pathlib.Path, rows: int) -> int:
    """Create a table, append `rows` rows, and read them all back."""
    table = Table.create(IOBase(root), assign_field_ids(SCHEMA), ["venue"])
    table.append(
        pa.record_batch(
            {"id": list(range(rows)), "venue": ["XNAS"] * rows},
            schema=SCHEMA,
        )
    )
    return sum(batch.num_rows for batch in table.scan())


@pytest.fixture
def native_records(caplog: pytest.LogCaptureFixture) -> pytest.LogCaptureFixture:
    """Capture at debug, with the level cache dropped so the change applies."""
    caplog.set_level(logging.DEBUG)
    # The bridge caches each Python logger's effective level, so a level set
    # after import reaches it only through this call - which is the whole
    # reason the function is exported.
    refresh_logging()
    return caplog


def test_an_operation_reports_what_it_did_and_what_it_is_doing(
    tmp_path: pathlib.Path, native_records: pytest.LogCaptureFixture
) -> None:
    assert _round_trip(tmp_path / "trades", 3) == 3

    mine = [record for record in native_records.records if record.name.startswith("yggdryl")]
    assert mine, "the native core reported nothing"

    done = {record.getMessage() for record in mine if record.levelno == logging.INFO}
    doing = {record.getMessage() for record in mine if record.levelno == logging.DEBUG}

    # Info is a thing done, and carries the numbers a monitor watches.
    assert any(message.startswith("created iceberg table at") for message in done)
    assert any("wrote 3 rows as" in message for message in done)
    assert any(message.startswith("committed iceberg metadata version") for message in done)
    assert any("data files to open" in message for message in done)
    # Debug is a thing starting.
    assert any(message.startswith("writing an iceberg append snapshot") for message in doing)
    assert any(message.startswith("planning iceberg scan of") for message in doing)


def test_the_record_count_does_not_follow_the_row_count(
    tmp_path: pathlib.Path, native_records: pytest.LogCaptureFixture
) -> None:
    # A commit is the unit worth watching, so ten times the rows is the same
    # handful of records: nothing is reported per row, per batch, or per file.
    assert _round_trip(tmp_path / "small", 5) == 5
    small = len([record for record in native_records.records if record.name.startswith("yggdryl")])

    native_records.clear()
    assert _round_trip(tmp_path / "large", 5_000) == 5_000
    large = len([record for record in native_records.records if record.name.startswith("yggdryl")])

    assert small == large


def test_only_this_project_reports_below_a_warning(
    tmp_path: pathlib.Path, native_records: pytest.LogCaptureFixture
) -> None:
    # The bridge is global, so every crate in the build could reach `logging`.
    # Anything that is not this project passes only at warning and above.
    assert _round_trip(tmp_path / "quiet", 3) == 3

    foreign = [
        record
        for record in native_records.records
        if not record.name.startswith(("yggdryl", "tests"))
        and record.levelno < logging.WARNING
    ]
    assert foreign == [], f"a dependency narrated its work: {[r.name for r in foreign]}"


def test_the_records_hang_off_the_packages_own_logger(
    tmp_path: pathlib.Path, native_records: pytest.LogCaptureFixture
) -> None:
    assert _round_trip(tmp_path / "named", 3) == 3

    names = {record.name for record in native_records.records if record.name.startswith("yggdryl")}
    assert names, "the native core reported nothing"
    # Silencing the package silences all of it, because the Rust module path is
    # the logger name and `yggdryl` is its root.
    assert all(name == "yggdryl" or name.startswith("yggdryl.") for name in names)
    assert "yggdryl.media.iceberg.table" in names
