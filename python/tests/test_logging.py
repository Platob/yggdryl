"""`python/src/logging.rs` and `yggdryl/logging.py`: the core's logging tree
hosted by `logging` - what it reports, under which names, at which levels -
and the file handler writing through a location."""

from __future__ import annotations

import json
import logging
import pathlib
import subprocess
import sys
import textwrap

import pyarrow as pa
import pytest

from yggdryl import IOBase, Url
from yggdryl.iceberg import IcebergTable, assign_field_ids
from yggdryl.logging import (
    Deduplicate,
    FileHandler,
    TerminalFormatter,
    TerminalHandler,
    deduplicate,
    is_deduplicating,
)

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ]
)


def _round_trip(root: pathlib.Path, rows: int) -> int:
    """Create a table, append `rows` rows, and read them all back."""
    table = IcebergTable.create(IOBase(root), assign_field_ids(SCHEMA), ["venue"])
    table.append(
        pa.record_batch(
            {"id": list(range(rows)), "venue": ["XNAS"] * rows},
            schema=SCHEMA,
        )
    )
    return sum(batch.num_rows for batch in table.scan())


@pytest.fixture
def native_records(caplog: pytest.LogCaptureFixture) -> pytest.LogCaptureFixture:
    """Capture at debug: `setLevel` drops the native side's cached levels too."""
    caplog.set_level(logging.DEBUG)
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
    # The tree is the process's `log` backend, so every crate in the build could
    # reach `logging`. Anything that is not this project passes only at warning
    # and above.
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
    assert "yggdryl.iceberg.table" in names


def test_a_data_warning_reaches_logging_once_and_then_is_counted() -> None:
    # What a parse passes over it says as a warning, deduplicated for the
    # whole process: the first occurrence in full, then a count at each
    # tenfold. A fresh interpreter reads the table from its first entry.
    script = textwrap.dedent(
        """
        import json
        import logging

        records = []

        class Collect(logging.Handler):
            def emit(self, record):
                records.append((record.name, record.levelname, record.getMessage()))

        logging.getLogger("yggdryl").addHandler(Collect())
        from yggdryl.fix import FixCodec, FixRegistry

        codec = FixCodec(FixRegistry())
        for _ in range(10):
            message = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=bad|10=0|")
            assert [field for field, _ in message.anomalies] == ["sendingtime"]
        print(json.dumps(records))
        """
    )
    ran = subprocess.run(
        [sys.executable, "-c", script], capture_output=True, text=True, timeout=120, check=False
    )
    assert ran.returncode == 0, ran.stderr
    records = [
        (name, level, text)
        for name, level, text in json.loads(ran.stdout)
        if "FIX clock left unstated" in text
    ]
    assert [(name, level) for name, level, _ in records] == [
        ("yggdryl.fix.build", "WARNING"),
        ("yggdryl.fix.build", "WARNING"),
    ]
    first, counted = (text for _, _, text in records)
    assert "(sendingtime)" in first and '"bad"' in first
    assert "later occurrences are counted rather than repeated" in first
    assert counted.endswith("seen 10 times")


def _created(root: pathlib.Path) -> None:
    IcebergTable.create(IOBase(root), assign_field_ids(SCHEMA))


class _Collect(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.records: list[logging.LogRecord] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.records.append(record)

    def messages(self) -> list[str]:
        taken = [record.getMessage() for record in self.records]
        self.records.clear()
        return taken


@pytest.fixture
def package() -> object:
    """The package's own logger with a collector, restored afterwards."""
    logger = logging.getLogger("yggdryl")
    collect = _Collect()
    level = logger.level
    logger.addHandler(collect)
    yield collect
    logger.removeHandler(collect)
    logger.setLevel(level)
    logging.disable(logging.NOTSET)


def test_a_level_changed_after_records_flowed_applies_to_the_next_record(
    tmp_path: pathlib.Path, package: _Collect
) -> None:
    logger = logging.getLogger("yggdryl")
    logger.setLevel(logging.WARNING)
    _created(tmp_path / "quiet")
    assert package.messages() == []

    # No refresh call: `setLevel` drops `logging`'s level cache, and the
    # native side's with it.
    logger.setLevel(logging.INFO)
    _created(tmp_path / "loud")
    assert any(message.startswith("created iceberg table at") for message in package.messages())

    logging.getLogger("yggdryl.iceberg.table").setLevel(logging.ERROR)
    try:
        _created(tmp_path / "child")
        assert package.messages() == [], "the nearer logger's level wins"
    finally:
        logging.getLogger("yggdryl.iceberg.table").setLevel(logging.NOTSET)


def test_logging_disable_holds_back_the_native_side_too(
    tmp_path: pathlib.Path, package: _Collect
) -> None:
    logging.getLogger("yggdryl").setLevel(logging.INFO)
    logging.disable(logging.INFO)
    _created(tmp_path / "disabled")
    assert package.messages() == []
    logging.disable(logging.NOTSET)
    _created(tmp_path / "enabled")
    assert package.messages()


def test_a_native_record_is_made_by_the_logger_it_reaches(
    tmp_path: pathlib.Path, package: _Collect
) -> None:
    logging.getLogger("yggdryl").setLevel(logging.INFO)
    _created(tmp_path / "made")
    [record] = [record for record in package.records if record.name == "yggdryl.iceberg.table"]
    assert record.levelno == logging.INFO
    assert record.pathname.endswith("table.rs"), record.pathname
    assert record.lineno > 0
    assert record.funcName == "(unknown function)"


def test_the_level_cache_hook_keeps_logging_s_own_cache(package: _Collect) -> None:
    # The native side wraps `Manager._clear_cache`; `logging`'s own cache must
    # still be dropped by it, or `isEnabledFor` would answer from before.
    logger = logging.getLogger("yggdryl.hooked")
    logger.setLevel(logging.ERROR)
    assert not logger.isEnabledFor(logging.INFO)
    logger.setLevel(logging.DEBUG)
    assert logger.isEnabledFor(logging.INFO)
    logger.setLevel(logging.NOTSET)


def test_a_file_handler_publishes_python_and_native_records_through_a_location(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "logs" / "app.log"
    handler = FileHandler(path, capacity=1 << 16)
    handler.setFormatter(logging.Formatter("%(levelname)s %(name)s %(message)s"))
    logger = logging.getLogger("yggdryl")
    logger.addHandler(handler)
    logger.setLevel(logging.INFO)
    try:
        logging.getLogger("yggdryl.app").warning("from python")
        _created(tmp_path / "logged")
        assert not path.exists(), "held under the capacity"
        handler.flush()
        lines = path.read_text(encoding="utf-8").splitlines()
        assert lines[0] == "WARNING yggdryl.app from python"
        assert lines[1].startswith("INFO yggdryl.iceberg.table created iceberg table at")
    finally:
        logger.removeHandler(handler)
        logger.setLevel(logging.NOTSET)
        handler.close()
    assert handler.url == str(Url.from_path(path))
    assert (handler.mode, handler.capacity, handler.flush_level) == ("append", 1 << 16, logging.ERROR)
    assert repr(handler).startswith("<FileHandler file://")


def test_a_file_handler_publishes_each_record_by_default_and_at_its_flush_level(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "each.log"
    path.write_text("kept\n", encoding="utf-8")
    eager = FileHandler(str(path))
    eager.setFormatter(logging.Formatter("%(message)s"))
    record = logging.LogRecord("trades", logging.INFO, __file__, 1, "opened", None, None)
    eager.handle(record)
    assert path.read_text(encoding="utf-8") == "kept\nopened\n"
    eager.close()

    held = FileHandler(IOBase(path), mode="overwrite", capacity=1 << 16, flush_level="WARNING")
    held.setFormatter(logging.Formatter("%(message)s"))
    held.handle(record)
    assert path.read_text(encoding="utf-8") == "kept\nopened\n"
    held.handle(logging.LogRecord("trades", logging.WARNING, __file__, 1, "late", None, None))
    assert path.read_text(encoding="utf-8") == "opened\nlate\n"
    held.close()


def test_a_file_handler_refuses_what_it_cannot_write_with() -> None:
    with pytest.raises(ValueError, match="expected append or overwrite"):
        FileHandler("logs/never.log", mode="merge")
    with pytest.raises(ValueError, match="mode"):
        FileHandler("logs/never.log", mode="a")
    with pytest.raises(ValueError, match="log level"):
        FileHandler("logs/never.log", flush_level="LOUD")


def test_logging_shutdown_publishes_what_a_handler_held(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "shutdown.log"
    script = textwrap.dedent(
        f"""
        import logging
        from yggdryl.logging import FileHandler

        handler = FileHandler({str(path)!r}, capacity=1 << 20)
        handler.setFormatter(logging.Formatter("%(message)s"))
        logging.getLogger("trades").addHandler(handler)
        logging.getLogger("trades").warning("held until exit")
        """
    )
    ran = subprocess.run(
        [sys.executable, "-c", script], capture_output=True, text=True, timeout=120, check=False
    )
    assert ran.returncode == 0, ran.stderr
    assert path.read_text(encoding="utf-8") == "held until exit\n"


def test_the_bridge_names_no_refresh_call() -> None:
    import yggdryl

    assert not hasattr(yggdryl, "refresh_logging")
    assert "logging" in yggdryl.__all__
    assert yggdryl.logging.FileHandler is FileHandler


def test_the_core_drops_its_own_repeats_before_they_reach_logging(
    tmp_path: pathlib.Path, package: _Collect
) -> None:
    logging.getLogger("yggdryl").setLevel(logging.INFO)
    assert not is_deduplicating("yggdryl.iceberg")
    deduplicate(logging.getLogger("yggdryl.iceberg.table"))
    try:
        assert is_deduplicating("yggdryl.iceberg.table")
        assert not is_deduplicating("yggdryl.iceberg")
        table = IcebergTable.create(IOBase(tmp_path / "repeated"), assign_field_ids(SCHEMA))
        package.messages()
        for _ in range(10):
            assert sum(batch.num_rows for batch in table.scan()) == 0
        said = [message for message in package.messages() if "iceberg scan" in message]
        assert len(said) == 2, said
        assert said[1].endswith("(seen 10 times)"), said
    finally:
        deduplicate("yggdryl.iceberg.table", None)
    assert not is_deduplicating("yggdryl.iceberg.table")


def test_a_deduplicate_filter_counts_python_records_by_the_core_s_hash() -> None:
    logger = logging.getLogger("tests.deduplicate")
    collect = _Collect()
    collect.addFilter(Deduplicate())
    logger.addHandler(collect)
    logger.propagate = False
    try:
        for _ in range(100):
            logger.warning("late %s", "fill")
        logger.warning("late fills")
        logger.error("late fill")
        assert collect.messages() == [
            "late fill",
            "late fill (seen 10 times)",
            "late fill (seen 100 times)",
            "late fills",
            "late fill",
        ]
    finally:
        logger.removeHandler(collect)
        logger.propagate = True


def test_a_deduplicate_filter_passes_what_its_name_does_not_scope() -> None:
    scoped = Deduplicate("tests.scoped")
    outside = logging.LogRecord("tests.other", logging.INFO, __file__, 1, "tick", None, None)
    assert all(scoped.filter(outside) for _ in range(3))
    inside = logging.LogRecord("tests.scoped.feed", logging.INFO, __file__, 1, "tick", None, None)
    assert [scoped.filter(inside) for _ in range(3)] == [True, False, False]


def test_a_record_level_past_the_tree_s_range_is_never_refused(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "loud.log"
    handler = FileHandler(path)
    handler.setFormatter(logging.Formatter("%(message)s"))
    record = logging.LogRecord("tests.loud", 300, __file__, 1, "beyond", None, None)
    assert Deduplicate().filter(record)
    handler.handle(record)
    handler.close()
    assert path.read_text(encoding="utf-8") == "beyond\n"


def test_a_refused_file_handler_leaves_nothing_for_shutdown(tmp_path: pathlib.Path) -> None:
    script = textwrap.dedent(
        f"""
        import logging
        from yggdryl.logging import FileHandler

        try:
            FileHandler({str(tmp_path / "never.log")!r}, mode="merge")
        except ValueError:
            pass
        logging.shutdown()
        print("shut down")
        """
    )
    ran = subprocess.run(
        [sys.executable, "-c", script], capture_output=True, text=True, timeout=120, check=False
    )
    assert ran.returncode == 0, ran.stderr
    assert ran.stdout.strip() == "shut down"
    assert "Error" not in ran.stderr, ran.stderr


def test_a_file_handler_takes_an_iobase_whole(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "taken.log"
    handle = IOBase(path)
    handler = FileHandler(handle)
    handler.setFormatter(logging.Formatter("%(message)s"))
    with pytest.raises(Exception, match="consumed|taken|empty"):
        handle.read_bytes()
    handler.handle(logging.LogRecord("tests.taken", logging.INFO, __file__, 1, "kept", None, None))
    handler.close()
    assert path.read_text(encoding="utf-8") == "kept\n"


def _logged_here(logger: logging.Logger) -> int:
    logger.warning("crossed\nat 101.5")
    return sys._getframe().f_lineno - 1


def test_a_terminal_handler_spells_time_level_thread_caller_and_message(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import io

    # The colour rule reads these; the test asks what a plain shell decides.
    for name in ("FORCE_COLOR", "CLICOLOR_FORCE", "NO_COLOR"):
        monkeypatch.delenv(name, raising=False)

    stream = io.StringIO()
    handler = TerminalHandler(stream)
    logger = logging.getLogger("tests.terminal")
    logger.addHandler(handler)
    logger.propagate = False
    try:
        line = _logged_here(logger)
    finally:
        logger.removeHandler(handler)
        logger.propagate = True
    first, second = stream.getvalue().splitlines()
    assert "\x1b" not in first, "a StringIO is no colour terminal"
    expected = f" ! WARNING  [MainThread] tests.terminal _logged_here:{line} › crossed"
    assert first.endswith(expected), first
    assert len(first) - len(first.lstrip("0123456789-:, ")) == len("2026-10-03 14:05:09,123 ")
    assert second == " " * (len(first) - len("crossed")) + "at 101.5"


def test_a_terminal_formatter_colours_and_carries_the_exception() -> None:
    formatter = TerminalFormatter(color=True)
    try:
        raise ValueError("bad fill")
    except ValueError:
        record = logging.LogRecord(
            "tests.terminal", logging.ERROR, __file__, 7, "rejected", None, sys.exc_info()
        )
    spelled = formatter.format(record)
    assert spelled.startswith("\x1b[2m"), spelled
    assert "\x1b[31m✗ ERROR   \x1b[0m" in spelled
    assert "ValueError: bad fill" in spelled
    plain = TerminalFormatter().format(record)
    assert "\x1b" not in plain
    assert plain.splitlines()[1].startswith(" " * plain.splitlines()[0].index("rejected"))


def test_a_file_handler_stating_no_formatter_writes_the_terminal_line(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "terminal.log"
    handler = FileHandler(path)
    record = logging.LogRecord("tests.plain", logging.ERROR, "/src/feed.py", 12, "rejected", None, None)
    record.funcName = "submit"
    handler.handle(record)
    handler.close()
    line = path.read_text(encoding="utf-8")
    assert line.endswith(" ✗ ERROR    [MainThread] tests.plain submit:12 › rejected\n"), line
    assert "\x1b" not in line


def test_the_terminal_line_dates_a_record_at_pythons_own_milliseconds() -> None:
    record = logging.LogRecord("tests.clock", logging.INFO, "/src/feed.py", 3, "opened", None, None)
    # 2023-11-14 22:13:20.123 UTC, where `created * 1e9` as a float lands a
    # nanosecond short of the millisecond.
    record.created = 1_700_000_000.123
    record.msecs = 123.0
    assert int(record.created * 1_000_000_000) % 1_000_000_000 < 123_000_000
    assert TerminalFormatter().format(record).startswith("2023-11-14 22:13:20,123 • INFO "), record


def test_a_malformed_call_under_deduplicate_never_raises_into_the_caller(
    capsys: pytest.CaptureFixture[str],
) -> None:
    logger = logging.getLogger("tests.dedup.malformed")
    logger.propagate = False
    handler = logging.StreamHandler()
    logger.addHandler(handler)
    logger.addFilter(Deduplicate())
    try:
        logger.warning("%d items", "x")
    finally:
        logger.removeHandler(handler)
    assert "--- Logging error ---" in capsys.readouterr().err, "the handler reports it"


def test_deduplicate_filters_on_two_handlers_count_the_message_it_was_logged_with() -> None:
    logger = logging.getLogger("tests.dedup.handlers")
    logger.propagate = False

    class Keep(logging.Handler):
        def __init__(self) -> None:
            super().__init__()
            self.said: list[str] = []
            self.addFilter(Deduplicate())

        def emit(self, record: logging.LogRecord) -> None:
            self.said.append(record.getMessage())

    # The first handler's filter re-spells the 10th record before the second
    # handler's filter sees it; both still count "late fill".
    first, second = Keep(), Keep()
    logger.addHandler(first)
    logger.addHandler(second)
    try:
        for _ in range(100):
            logger.warning("late fill")
    finally:
        logger.removeHandler(first)
        logger.removeHandler(second)
    expected = ["late fill", "late fill (seen 10 times)", "late fill (seen 100 times)"]
    assert first.said == expected
    assert second.said == expected


def test_a_refused_file_handler_leaves_the_callers_handle_as_it_was(tmp_path: pathlib.Path) -> None:
    handle = IOBase(tmp_path / "kept.log")
    handle.write_bytes(b"kept\n")
    for refused in (
        {"mode": "merge"},
        {"flush_level": "LOUD"},
        {"level": "BOGUS"},
    ):
        with pytest.raises(ValueError):
            FileHandler(handle, **refused)  # type: ignore[arg-type]
        assert handle.read_bytes() == b"kept\n", refused


def test_the_terminal_line_names_a_level_as_python_registered_it() -> None:
    logging.addLevelName(25, "NOTICE")
    record = logging.LogRecord("tests.notice", 25, "/src/feed.py", 3, "noted", None, None)
    assert " NOTICE   [" in TerminalFormatter().format(record)
    trace = logging.LogRecord("tests.notice", 5, "/src/feed.py", 3, "traced", None, None)
    assert " Level 5  [" in TerminalFormatter().format(trace), "Python's name, not the core's TRACE"
