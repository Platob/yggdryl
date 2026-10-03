"""The core's logging tree, hosted by Python's `logging`.

The native core logs through Rust's `log` facade, and importing the package
makes the core's own logging tree that facade's backend with `logging` as its
host: a record raised in `yggdryl::iceberg::table` is handled by
`logging.getLogger("yggdryl.iceberg.table")` - its level, its filters, its
handlers, and its ancestors' up to the root - exactly as a record logged in
Python is. `logging.getLogger("yggdryl")` is the one switch for the whole
package.

The core asks `logging` once which level each logger handles and keeps the
answer until `logging` drops its own level cache - `setLevel`,
`logging.disable`, `basicConfig` and `dictConfig` all do - so a level changed
at any time applies to the next record, and a record no logger handles costs
the native side a few atomic reads and never takes the interpreter. A crate the
build depends on reaches `logging` only at `WARNING` and above: enabling debug
narrates this package and nothing else.

Repeated records are counted by the core's hash of the logger, the level and
the message, in a lock-free table: `deduplicate("yggdryl")` drops the core's
own repeats where they are logged, before they reach `logging` - a repeat
never takes the interpreter - and `Deduplicate` is the same counting as a
`logging.Filter` for any record. Either says a record the first time, then
again at its 10th, 100th, 1000th occurrence as `message (seen N times)`.

`TerminalHandler` is a `logging.StreamHandler` writing the core's terminal
format - `2026-10-03 14:05:09,123 • INFO     [MainThread] trades.feed open:42 ›
opened 3 venues`, the level's glyph and name in its colour - coloured where its
stream is a colour terminal (`NO_COLOR`, `FORCE_COLOR`, `CLICOLOR_FORCE` and
`TERM=dumb` honoured) and plain anywhere else:
`logging.basicConfig(level=logging.INFO, handlers=[TerminalHandler()])` is the
whole setup, and code only logs.

`FileHandler` is a `logging.Handler` writing through any location the package
reads - a path, a `Url`, an `IOBase`, an object in a bucket - with one append
per publish: at once by default, or once `capacity` bytes are held or a record
at `flush_level` arrives. `logging.shutdown`, which Python runs at exit,
publishes what it holds.
"""

from __future__ import annotations

import logging as _logging
import os
from typing import TYPE_CHECKING, TextIO

from ._native import (
    LogFile,
    LogRepeats,
    log_deduplicate,
    log_is_color_enabled,
    log_is_deduplicating,
    log_terminal,
)

if TYPE_CHECKING:
    from ._native import IOBase, Url

__all__ = [
    "Deduplicate",
    "FileHandler",
    "TerminalFormatter",
    "TerminalHandler",
    "deduplicate",
    "is_deduplicating",
]


def _name(logger: str | _logging.Logger) -> str:
    return logger.name if isinstance(logger, _logging.Logger) else logger


def deduplicate(logger: str | _logging.Logger, enabled: bool | None = True) -> None:
    """States whether the core drops repeated records logged on `logger` and below.

    `logger` is a name or a `logging.Logger`; `enabled` is `True` to drop
    repeats where the core logs them, `False` to pass every record, `None` to
    take the nearest ancestor's choice again. Only records the native core
    logs are counted here; a record logged in Python is counted by a
    `Deduplicate` filter.
    """
    log_deduplicate(_name(logger), enabled)


def is_deduplicating(logger: str | _logging.Logger) -> bool:
    """Whether the core drops repeated records logged on `logger`."""
    return log_is_deduplicating(_name(logger))


class Deduplicate(_logging.Filter):
    """Says a record the first time, then at each tenfold repeat with its count.

    Records are counted by the core's hash of the logger name, the level and
    the formatted message, in a table of this filter's own: the 10th, 100th
    and 1000th occurrence pass as `message (seen N times)` and every other
    repeat is dropped. `name` scopes the filter as `logging.Filter`'s does,
    except that a record outside it passes uncounted rather than dropped.
    """

    def __init__(self, name: str = "") -> None:
        super().__init__(name)
        self._repeats = LogRepeats()

    def filter(self, record: _logging.LogRecord) -> bool:
        if not super().filter(record):
            return True
        # A record another `Deduplicate` already re-spelled is counted by the
        # message it was logged with, so filters in a chain agree.
        message = record.__dict__.get(_SAID)
        if message is None:
            try:
                message = record.getMessage()
            except Exception:  # noqa: BLE001 - a malformed call is the handler's to report
                # Passed on uncounted: the handler's `handleError` says what
                # is wrong with it, and the code that logged never sees it.
                return True
        said = self._repeats.said(record.name, record.levelno, message)
        if said is None:
            return False
        if said != message:
            record.__dict__.setdefault(_SAID, message)
            record.msg = said
            record.args = None
        return True


#: The attribute a re-spelled record keeps the message it was logged with in.
_SAID = "_yggdryl_message"


def _checked_level(level: int | str) -> None:
    """Refuses a level as `logging.Handler` would, before anything is built."""
    if isinstance(level, str):
        if not isinstance(_logging.getLevelName(level), int):
            raise ValueError(f"Unknown level: {level!r}")
    elif not isinstance(level, int):
        raise TypeError(f"Level not an integer or a valid string: {level!r}")


class FileHandler(_logging.Handler):
    """Writes each record as one line through a location's handle.

    `location` is a path, a `Url` or an `IOBase`; an `IOBase` is taken whole -
    its store options and coding kept - and answers nothing after.
    `mode` is `append` - keep what the location holds - or `overwrite` -
    replace it with the first publish. A record is spelled by the handler's
    formatter - the core's terminal format, plain, when none is set - and
    published with one append: at once when `capacity` is `0`,
    otherwise once the held lines reach `capacity` bytes or a record at or
    above `flush_level` arrives. A publish the location refuses goes to
    `handleError`, as any handler's failure does.
    """

    def __init__(
        self,
        location: str | os.PathLike[str] | Url | IOBase,
        mode: str = "append",
        capacity: int = 0,
        flush_level: int | str = _logging.ERROR,
        level: int | str = _logging.NOTSET,
    ) -> None:
        # Every argument is checked before the native half takes the
        # location, and the native half is built before `logging` registers
        # the handler: a refusal leaves the caller's handle as it was and no
        # half-built handler for `logging.shutdown` to find.
        _checked_level(level)
        self._file = LogFile(location, mode, capacity, flush_level)
        super().__init__(level)

    @property
    def url(self) -> str | None:
        """The location written to."""
        return self._file.url

    @property
    def mode(self) -> str:
        """`append` or `overwrite`."""
        return self._file.mode

    @property
    def capacity(self) -> int:
        """The bytes held back before a publish."""
        return self._file.capacity

    @property
    def flush_level(self) -> int:
        """The level that publishes what is held at once."""
        return self._file.flush_level

    def format(self, record: _logging.LogRecord) -> str:
        """Spells `record` with the handler's formatter, the core's terminal
        format - plain - when none is set."""
        return (self.formatter or _PLAIN_TERMINAL).format(record)

    def emit(self, record: _logging.LogRecord) -> None:
        try:
            self._file.write(self.format(record), record.levelno)
        except Exception:
            self.handleError(record)

    def flush(self) -> None:
        self._file.flush()

    def close(self) -> None:
        try:
            self._file.close()
        finally:
            super().close()

    def __repr__(self) -> str:
        level = _logging.getLevelName(self.level)
        return f"<{type(self).__name__} {self.url} ({level})>"


#: What Python's `findCaller` says of a record that names no function.
_UNKNOWN_FUNCTION = "(unknown function)"


class TerminalFormatter(_logging.Formatter):
    """Spells a record in the core's terminal format, coloured when `color`.

    The timestamp, the level's glyph and name, the thread, the logger, the
    call site as `funcName:lineno` and the message, its later lines starting
    under its first; an exception or a stack a record carries is part of the
    message. Rendered natively, one call per record.
    """

    def __init__(self, color: bool = False) -> None:
        super().__init__()
        self.color = color

    def format(self, record: _logging.LogRecord) -> str:
        message = record.getMessage()
        if record.exc_info and not record.exc_text:
            record.exc_text = self.formatException(record.exc_info)
        if record.exc_text:
            message = f"{message}\n{record.exc_text}"
        if record.stack_info:
            message = f"{message}\n{self.formatStack(record.stack_info)}"
        function = record.funcName
        return log_terminal(
            record.name,
            record.levelno,
            record.levelname,
            message,
            # Whole seconds and Python's own milliseconds: `created` scaled
            # as a float loses the last digits, so `,123` would read `,122`.
            int(record.created) * 1_000_000_000 + int(record.msecs) * 1_000_000,
            record.pathname,
            record.lineno,
            None if function in (None, _UNKNOWN_FUNCTION) else function,
            record.threadName,
            self.color,
        )


class TerminalHandler(_logging.StreamHandler):  # type: ignore[type-arg]
    """A `logging.StreamHandler` writing the core's terminal format.

    `stream` is standard error unless given; `color` is decided from it by
    the core's rule unless stated.
    """

    def __init__(self, stream: TextIO | None = None, color: bool | None = None) -> None:
        super().__init__(stream)
        if color is None:
            isatty = getattr(self.stream, "isatty", None)
            color = log_is_color_enabled(bool(isatty and isatty()))
        self.setFormatter(TerminalFormatter(color))


#: What a `FileHandler` stating no formatter spells its records with.
_PLAIN_TERMINAL = TerminalFormatter()
