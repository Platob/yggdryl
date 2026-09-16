"""The bridge's own row header, as the extension hands it over."""

from __future__ import annotations

import pathlib

import pytest

from yggdryl import IOBase, TextOptions, ULBRIDGE_ROWHEADER
from yggdryl.fix import FixCodec, FixRegistry

#: Two lines of a bridge log: one whose bracket holds the whole message
#: context, and one holding nothing but the thread.
LINES = (
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] "
    "[Broker_DarkPool_TradeCapture] (DEBUG) "
    "Sending : 8=FIX.4.4|35=D|11=ORD-1|55=AAPL|10=000|\n"
    "2026-08-14 06:46:30.416 [15261] [OMS_X1_TradeCapture] (DEBUG) "
    "Sending : 8=FIX.4.4|35=0|49=CLI|56=OMS|10=159|\n"
)


def options() -> TextOptions:
    held = TextOptions()
    held.rowheader = ULBRIDGE_ROWHEADER
    held.timezone = "UTC"
    held.start_rownum = 1
    return held


@pytest.fixture
def capture(tmp_path: pathlib.Path) -> IOBase:
    target = tmp_path / "bridge.log"
    target.write_text(LINES, encoding="utf-8")
    handle = IOBase.from_uri(target.as_uri())
    yield handle
    handle.close()


def test_the_constant_is_the_expression_the_bridge_writes() -> None:
    """A caller reads the header here rather than spelling it again."""
    assert isinstance(ULBRIDGE_ROWHEADER, str)
    assert ULBRIDGE_ROWHEADER.startswith(r"^(?P<timestamp>")
    assert ULBRIDGE_ROWHEADER.endswith(r"\((?P<level>[A-Z]+)\) ")


def test_every_capture_is_named_for_the_field_it_fills() -> None:
    """The names are the contract: a capture lands because it is called what
    the field is called, so nothing maps a spelling onto a tag."""
    assert options().capture_names == (
        "timestamp",
        "threadId",
        "bridgesessionid",
        "msgctxid",
        "msgseqnum",
        "pluginid",
        "level",
    )
    # Typed from the pattern, before a byte is read.
    captures = options().source_field().into_arrow_schema()
    assert str(captures.field("msgseqnum").type) == "int64"


def test_the_header_frames_a_line_and_its_captures_fill_the_row(capture: IOBase) -> None:
    """The whole point of naming the constant: one read, and the bracket's
    parts are columns of the message the line carried."""
    held = options()
    codec = FixCodec(FixRegistry(), capture_names=held.capture_names)
    rows = list(codec.parse_text_lines(capture.read_text_lines(options=held)))

    assert len(rows) == 2
    # The bracket's own first part is the session instance, never what the
    # message says about itself.
    assert rows[0].by_name("bridgesessionid").as_py() == "e7254b22"
    assert rows[0].by_name("msgctxid").as_py() == "9f015ee861"
    assert rows[0].by_name("pluginid").as_py() == "Broker_DarkPool_TradeCapture"
    assert rows[0].get_by_name("sendersessionid") is None


def test_a_bracket_holding_only_its_thread_still_frames(capture: IOBase) -> None:
    """The three bracketed parts are optional as a whole, so a line carrying
    only its thread leaves them null rather than failing the row."""
    held = options()
    codec = FixCodec(FixRegistry(), capture_names=held.capture_names)
    rows = list(codec.parse_text_lines(capture.read_text_lines(options=held)))

    assert rows[1].get_by_name("bridgesessionid") is None
    assert rows[1].get_by_name("msgctxid") is None
    assert rows[1].by_name("pluginid").as_py() == "OMS_X1_TradeCapture"
