"""The FIX boundary: the typed vocabulary, the registry, and the message.

Every answer here is the core's; what these check is the crossing - the key
coercion, the streams and Arrow twins, the catalog's component and group
doors, and the live reference metadata a field carries.
"""

from __future__ import annotations

import copy
import datetime as dt
import decimal
import io
import json
import logging
import pathlib
import pickle
import subprocess
import sys
from collections.abc import Iterator
from typing import Any, Iterable

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import (
    DataType,
    Field,
    Identifiers,
    IOBase,
    MimeType,
    Scalar,
    TextLine,
    TextOptions,
    Timezone,
    Url,
)
from yggdryl.fix import (
    ULBRIDGE_ROWHEADER,
    FixCapture,
    FixCodec,
    FixHeader,
    FixMessages,
    FixMsg,
    FixRegistry,
    MsgType,
    fix_crate_fields,
    fix_schema,
    fix_schema_carrying,
    fix_schema_tags,
)
from yggdryl import MarketDataKind, Side, State, TimeInForce, graph
from yggdryl.graph import BookEvent, MarketData, OrderEvent


# The one intake clock undated test bytes take, so a parse repeats.
CLOCK_batch = DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000)

def _kinds(ids: Identifiers) -> dict[str, str]:
    """Each type a set holds, its base key's value: the type's answer."""
    return {id.type: id.value for id in ids if id.src == "base"}


def _row_kinds(cell: list[tuple[str, str]]) -> dict[str, str]:
    """An identifier column's cell - a map from each key's text to its value - by type."""
    return {key: value for key, value in cell if ":" not in key}



def _fixed_batch(registry: FixRegistry, **pins: Any) -> FixCodec:
    """A codec whose undated messages all take ``CLOCK`` as their SendingTime.

    It reads every message type: the corpus below is a capture, and a capture
    holds the session traffic and the bridge rows stating no type that the
    :data:`DEFAULT_REFUSED_MSGTYPES` drop. A case about the refusals says so
    for itself.
    """
    pins.setdefault("exclude_msgtypes", [])
    pins.setdefault("threads", 1)
    return FixCodec(registry, default_sending_time=CLOCK_batch, **pins)


@pytest.fixture
def seed_batch(_seed_catalog: FixRegistry) -> FixRegistry:
    return copy.copy(_seed_catalog)


# A JSON document a bridge logs, which the codec does not read: one row, one
# `unknown` message.
DOCUMENT = (
    b'{"request":{"mbean":"com.ullink.ulbridge:type=*","type":"read"},'
    b'"value":{"com.ullink.ulbridge:name=A,type=Bridge":{"Name":"A"},'
    b'"com.ullink.ulbridge:name=B,type=Bridge":{"Name":"B"}},"status":200}'
)

# Every shape a real capture holds, the corpus the readers are tested on.
CAPTURE = [
    b"sending >> 8=FIX.4.2|9=176|35=D|11=ORDER-1|55=AAPL|54=1|10=203| << queued seq=1092",
    b"raw 8=FIX.4.4|9=224|35=8|17=E1|37=O9|31=12.75|32=50|10=118|",
    b"8=FIX.4.4|35=8|58=quoting #A=1 and #B=2|10=1|",
    b"sending >> 8=FIX.4.2|35=UL|#SYMBOL=TTF|#SIDE=1|10=044|",
    b"8=FIX.4.4|35=D|11=ORDER-1|SYMBOL=AAPL|SIDE=1|10=000",
    b"toBridge #ISINCODE=XX|#SYMBOL=TTF|#SIDE=1",
    b"ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1",
    b"<Order ClOrdID='XML-1'>body</Order>",
    b"Receiving XmlApi: <Execution ExecID='E1'></Execution>",
    b"no level printed by this plugin",
]

# The capture lines that carry a message: a line that opens no frame, states
# no bridge pair and carries no document carries nothing to read.
CARRYING = [CAPTURE[at] for at in (0, 1, 2, 3, 4, 5, 6, 7, 8)]

ORDER = b"8=FIX.4.4|35=D|11=A1|52=20240102-10:15:30|55=AAPL|VenueThing=7|9999=x|10=0|"
REPORT_batch = b"8=FIX.4.4|35=8|52=20240102-10:15:30|39=1|150=F|38=100|14=40|32=40|31=10.5|10=0|"


def _capture(lines: list[bytes], rows: int) -> pa.RecordBatchReader:
    """The capture as the batches a text reader hands the codec."""
    schema = pa.schema([pa.field("body", pa.binary(), nullable=False)])
    batches = [
        pa.record_batch([pa.array(lines[at : at + rows], pa.binary())], schema=schema)
        for at in range(0, len(lines), max(rows, 1))
    ]
    return pa.RecordBatchReader.from_batches(schema, batches)


def _wide() -> list[bytes]:
    """Two hundred wide orders, about 450 bytes each."""
    return [
        b"8=FIX.4.4|35=D|11=ORDER-%06d|58=%s|10=0|" % (index, b"x" * 400) for index in range(200)
    ]


def _one(codec: FixCodec, line: bytes) -> FixMsg:
    messages = codec.parse_line(line)
    message = next(messages)
    assert next(messages, None) is None, "one message"
    return message


def _column(table: pa.Table, name: str) -> list[Any]:
    return table.column(name).to_pylist()


def test_parse_lines_pulls_one_line_at_a_time_and_continues_past_a_refused_one(
    seed_batch: FixRegistry,
) -> None:
    pulled = 0

    def lines() -> Iterator[bytes]:
        nonlocal pulled
        for line in CARRYING:
            pulled += 1
            yield line

    stream = _fixed_batch(seed_batch).parse_lines(lines())
    assert isinstance(stream, FixMessages)
    assert iter(stream) is stream
    assert pulled == 0
    first = next(stream)
    assert pulled == 1
    assert first.by_tag(11).as_py() == "ORDER-1"
    assert len(list(stream)) == len(CARRYING) - 1
    assert pulled == len(CARRYING)
    assert next(stream, None) is None


def test_an_item_that_is_not_bytes_is_refused_where_it_is_met(seed_batch: FixRegistry) -> None:
    stream = _fixed_batch(seed_batch).parse_lines([CARRYING[0], 17])
    assert next(stream) is not None
    with pytest.raises(TypeError):
        next(stream)
    assert next(stream, None) is None


def test_parse_text_lines_pulls_one_line_at_a_time(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch, capture_names=["msgpluginid"])
    lines = [TextLine(index, line, ["ULB"]) for index, line in enumerate(CARRYING)]
    read = list(codec.parse_text_lines(lines))
    assert len(read) == len(CARRYING)
    assert all(held.capture().msgpluginid == "ULB" for held in read)
    # A line's own captures are facts about the capture, never entries.
    assert all(all(tag != MSGPLUGINID_TAG for tag, _, _, _ in held.entries()) for held in read)


def test_the_codec_answers_the_pins_it_was_given(seed_batch: FixRegistry) -> None:
    plain = FixCodec(seed_batch)
    assert plain.registry == seed_batch
    assert plain.default_sending_time is None
    assert plain.separator is None
    assert plain.payload_column == "body"
    assert plain.null_values == ["", "null", "<null>", "none", "n/a", "[n/a]"]
    assert plain.batch_byte_size == 128 * 1024 * 1024
    assert repr(plain).startswith("FixCodec(")
    with pytest.raises(TypeError):
        hash(plain)

    pinned = FixCodec(
        seed_batch,
        default_sending_time=CLOCK_batch,
        separator=124,
        payload_column="line",
        null_values=["<none>"],
        direction="R",
        batch_byte_size=1 << 20,
    )
    assert pinned.default_sending_time == CLOCK_batch
    assert pinned.separator == 124
    assert pinned.payload_column == "line"
    assert pinned.null_values == ["<none>"]
    assert pinned.direction == "R"
    assert pinned.batch_byte_size == 1 << 20
    assert copy.copy(pinned).registry == seed_batch

    # A stated absence produces no field at all.
    silent = _fixed_batch(seed_batch, null_values=["<none>"])
    assert next(silent.parse_line(b"8=FIX.4.4|35=D|55=<none>|10=0|")).get_by_tag(55) is None

    # A codec pins no version and no dialect: the dictionary is one namespace.
    with pytest.raises(TypeError):
        FixCodec(seed_batch, version="4.2")  # type: ignore[call-arg]
    with pytest.raises(TypeError):
        FixCodec(seed_batch, branch="cme")  # type: ignore[call-arg]


def test_threads_read_what_one_thread_reads_and_a_message_carries_its_rows_cells(
    seed_batch: FixRegistry,
) -> None:
    plain = FixCodec(seed_batch)
    one = _fixed_batch(seed_batch)
    four = _fixed_batch(seed_batch, threads=4)
    assert plain.threads >= 1
    assert one.threads == 1
    assert four.threads == 4
    assert _fixed_batch(seed_batch, threads=0).threads == 1

    # The line doors answer on four threads what they answer on one: the
    # same messages, in the same order.
    def stated(messages: Iterator[Any]) -> list[tuple[Scalar, bytes]]:
        return [(held.curruuid, held.into_bytes(ord("|"))) for held in messages]

    assert stated(four.parse_lines(CAPTURE)) == stated(one.parse_lines(CAPTURE))
    parsed = four.parse_text_arrow_reader(_capture(CAPTURE, 3)).read_all()
    assert parsed.equals(one.parse_text_arrow_reader(_capture(CAPTURE, 3)).read_all())
    assert stated(four.messages(parsed)) == stated(one.messages(parsed))

    # A message read back out of a row carries the row's own cells - the
    # body its line was cut from - and one parsed from bytes carries none.
    held = next(iter(one.messages(parsed)))
    assert dict(held.carried)["body"].as_py() == CAPTURE[0]
    assert next(one.parse_line(CAPTURE[0])).carried == []


def test_arrow_pool_pulls_one_input_batch_per_worker_ahead(seed_batch: FixRegistry) -> None:
    schema = pa.schema([pa.field("body", pa.binary(), nullable=False)])
    pulled = 0
    # The first row expands to two messages and the second to one. A worker
    # therefore keeps its input batch until its third output row is pulled.
    first = b"8=FIX.4.4|35=D|11=POOL-1|52=20240102-10:15:30|10=0|"
    second = b"8=FIX.4.4|35=D|11=POOL-2|52=20240102-10:15:30|10=0|"
    third = b"8=FIX.4.4|35=D|11=POOL-3|52=20240102-10:15:30|10=0|"

    def batches() -> Iterator[pa.RecordBatch]:
        nonlocal pulled
        for _ in range(12):
            pulled += 1
            yield pa.record_batch(
                [pa.array([first + second, third], pa.binary())], schema=schema
            )

    reader = FixCodec(
        seed_batch,
        default_sending_time=CLOCK_batch,
        exclude_msgtypes=[],
        threads=3,
        batch_row_size=1,
    ).parse_text_arrow_reader(pa.RecordBatchReader.from_batches(schema, batches()))
    for clordid in ("POOL-1", "POOL-2", "POOL-3"):
        batch = next(reader)
        assert batch.column(batch.schema.get_field_index("msgtype"))[0].as_py() == "D"
        assert batch.column(batch.schema.get_field_index("clordid"))[0].as_py() == clordid
        assert pulled == 3
    fourth = next(reader)
    assert fourth.column(fourth.schema.get_field_index("msgtype"))[0].as_py() == "D"
    assert pulled == 4


def test_the_schema_is_decided_before_the_first_row_is_read(seed_batch: FixRegistry) -> None:
    reader = _fixed_batch(seed_batch).parse_text_arrow_reader(_capture([], 1))
    names = reader.schema.names
    # The six identities every graph element states lead, then the event's
    # clocks; the capture's own column and the fixed columns follow - a table
    # is read by time and joined by identity.
    assert names[:7] == [
        "curruuid",
        "crossuuid",
        "crosscode",
        "currhashcode",
        "crosshashcode",
        "srcuuids",
        "currunix",
    ]
    assert "body" in names
    # A column is read by name rather than by position: the bands the row is
    # laid out in are the core's to order.
    for named in ("beginstring", "msgtype", "sendingtime", "symbol", "fixentries"):
        assert names.count(named) == 1, named
    # The one record closes the row, under the counter that counts it.
    # `fixentries` is a tag:name map that counts itself; no counter column (A8).
    assert names[-1] == "fixentries" and names.count("metadata") == 1
    assert reader.schema.field("msgtype").metadata[b"FIX:tag"] == b"35"
    # The identities cross as what a lake reads: a UUID and a 64-bit integer.
    assert reader.schema.field("curruuid").type == pa.uuid()
    assert reader.schema.field("currhashcode").type == pa.uint64()
    # And an empty capture yields no batch at all.
    assert reader.read_all().num_rows == 0


def test_a_capture_answers_one_row_per_message_not_one_per_line(seed_batch: FixRegistry) -> None:
    parsed = _fixed_batch(seed_batch).parse_text_arrow_reader(_capture(CAPTURE, len(CAPTURE))).read_all()
    assert parsed.num_rows == len(CARRYING), "one row a message"
    assert [bytes(body) for body in _column(parsed, "body")] == CARRYING
    msgtype = _column(parsed, "msgtype")
    assert msgtype[0] == "D", "a framed row states its type"
    # Every row settles its identity, so the non-null columns are filled.
    assert all(held is not None for held in _column(parsed, "curruuid"))
    assert all(held is not None for held in _column(parsed, "currhashcode"))
    # This ordinary message is fully projected; the residual record remains
    # present but empty rather than restaging projected facts.
    assert _column(parsed, "fixentries")[0] == []


def test_several_small_input_batches_accumulate_into_one_output_batch(seed_batch: FixRegistry) -> None:
    lines = _wide()
    whole = _fixed_batch(seed_batch).parse_text_arrow_reader(_capture(lines, 10)).read_all()
    assert whole.num_rows == 200
    assert len(whole.to_batches()) == 1, "one batch under the byte target"

    bounded = _fixed_batch(seed_batch, batch_byte_size=5 * 10 * 470).parse_text_arrow_reader(_capture(lines, 10))
    batches = list(bounded)
    assert 2 <= len(batches) < 20, len(batches)
    assert sum(batch.num_rows for batch in batches) == 200


def test_one_large_input_batch_splits_by_rows_in_proportion(seed_batch: FixRegistry) -> None:
    lines = _wide()
    target = 4096
    batches = list(
        _fixed_batch(seed_batch, batch_byte_size=target).parse_text_arrow_reader(_capture(lines, len(lines)))
    )
    assert len(batches) > 1
    assert sum(batch.num_rows for batch in batches) == 200, "the bound shapes batches"
    closed = batches[:-1]
    rows = closed[0].num_rows
    assert all(batch.num_rows == rows for batch in closed)

    # A target no row fits under closes a batch after every row.
    each = list(
        _fixed_batch(seed_batch, batch_byte_size=1).parse_text_arrow_reader(_capture(lines, len(lines)))
    )
    assert len(each) == 200
    assert all(batch.num_rows == 1 for batch in each)


def test_messages_to_batches_close_on_the_records_raw_bytes(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)
    schema = fix_schema(seed_batch)
    lines = _wide()
    one = list(codec.arrow_reader(schema, codec.parse_lines(lines)))
    assert len(one) == 1 and one[0].num_rows == 200

    bounded = _fixed_batch(seed_batch, batch_byte_size=10 * 450)
    many = list(bounded.arrow_reader(schema, codec.parse_lines(lines)))
    assert 1 < len(many) < 40, len(many)
    assert sum(batch.num_rows for batch in many) == 200

    each = list(_fixed_batch(seed_batch, batch_byte_size=1).arrow_reader(schema, codec.parse_lines(lines)))
    assert len(each) == 200


def test_messages_and_arrow_reader_invert_each_other(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch, null_values=[])
    schema = fix_schema(seed_batch)
    parsed = list(codec.parse_lines(CARRYING))
    again = list(codec.messages(codec.arrow_reader(schema, parsed)))
    assert len(again) == len(parsed)
    for held, message in zip(again, parsed):
        # Arrow reconstruction combines projected and residual content in
        # schema order; its row and stored identities are the contract.
        assert held.currhashcode == message.currhashcode
        assert held.curruuid == message.curruuid
        assert held.into_row(schema) == message.into_row(schema)
    # And the batches the second pass makes are the batches the first made.
    first = codec.arrow_reader(schema, parsed).read_all()
    second = codec.arrow_reader(schema, again).read_all()
    assert first.equals(second)


def test_book_arrow_reader_streams_lifted_market_data_books(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch, batch_row_size=1)
    snapshot = codec.parse_fix_line(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|"
        b"269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|"
    )
    update = codec.parse_fix_line(
        b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|"
        b"279=1|269=0|278=B1|270=101|271=11|"
        b"279=0|269=2|278=T1|270=101|271=2|10=0|"
    )

    reader = codec.book_arrow_reader([snapshot, update], snapshot_millis=0)
    assert isinstance(reader, pa.RecordBatchReader)
    # The one lifted `marketdata` schema every leaf is written under: the
    # kind, then every fact a column of its own, the book's entries, deltas,
    # a trade's executions and price levels nested - sixty-two columns, an
    # operation's `bookaction` and `bookposition` among them.
    assert reader.schema == MarketData.field().into_arrow_schema()
    names = reader.schema.names
    assert len(names) == 62
    assert names[0] == "curruuid"
    assert names.index("marketdatakind") == 15
    assert {"bookaction", "bookposition"} <= set(names)
    assert {"alive", "deltas", "executions", "bidlimits", "asklimits"} <= set(names)
    assert not {"bidside", "askside", "snapshotpartitions", "kind"} & set(names)
    assert "price" in names and "quantity" in names
    assert "px" not in names and "qty" not in names

    table = reader.read_all()
    rows = table.to_pylist()
    assert len(rows) == 2
    assert [row["marketdatakind"] for row in rows] == [MarketDataKind.BOOK] * 2
    assert [row["ticker"] for row in rows] == ["AAPL", "AAPL"]
    assert [row["price"] for row in rows] == [decimal.Decimal("101"), decimal.Decimal("101.5")]
    # The book states its best tradable levels as its own bid and ask.
    assert [row["bidpx"] for row in rows] == [decimal.Decimal("100"), decimal.Decimal("101")]
    assert [row["askpx"] for row in rows] == [decimal.Decimal("102"), decimal.Decimal("102")]
    # The full refresh is a snapshot input, so its book is whole: every entry
    # alive on it, each a quote filed under QUOT, and its levels best first,
    # each stating whether it trades. The update's book states its deltas
    # alone - no grid, no snapshot input - so it states no alive entry and no
    # level, and a book holds no execution: the trade entry (`269=2`) is an
    # execution no book folds, so the bid's change is its one delta.
    assert [row["alive"] is None for row in rows] == [False, True]
    assert rows[0]["alive"][0]["price"] == decimal.Decimal("100")
    assert rows[0]["alive"][0]["marketdatakind"] == MarketDataKind.QUOT
    assert [entry["side"] for entry in rows[0]["alive"]] == [Side.BUYS, Side.SELL]
    [bid] = rows[0]["bidlimits"]
    assert (bid["price"], bid["quantity"], bid["tradable"]) == (
        decimal.Decimal("100"),
        decimal.Decimal("10"),
        True,
    )
    assert [level["price"] for level in rows[0]["asklimits"]] == [decimal.Decimal("102")]
    assert (rows[1]["bidlimits"], rows[1]["asklimits"]) == (None, None)
    assert [row["executions"] for row in rows] == [None, None]
    [delta] = rows[1]["deltas"]
    assert (delta["price"], delta["side"], delta["bookaction"]) == (decimal.Decimal("101"), Side.BUYS, "1")
    # An operation row names its entry under `mdentryid`.
    assert _row_kinds(delta["identifiers"]) == {"mdentryid": "B1"}

    # The rows read back as the typed books they were written from, the
    # update's whole once rebuilt over the refresh's.
    books = [data.as_book_event() for data in MarketData.from_arrow_reader(table)]
    assert all(isinstance(book, BookEvent) for book in books)
    first, second = (book for book in books if book is not None)
    assert first.is_complete and not second.is_complete
    assert first.best_price(Side.BUYS) is not None
    assert first.best_price(Side.BUYS).as_py() == decimal.Decimal("100")
    assert second.best_price(Side.BUYS) is not None
    assert second.best_price(Side.BUYS).as_py() == decimal.Decimal("101")
    whole = second.with_previous(first)
    assert whole is not None and whole.is_complete
    live = whole.alive_on(Side.BUYS)[0].as_quote_event()
    assert live is not None and _kinds(live.identifiers) == {"mdentryid": "B1"}
    assert live.quantity is not None and live.quantity.as_py() == decimal.Decimal("11")


# One message of each dated leaf a capture expands into, in time order: an
# order, a quote, a fill, an initial trade report and an empty snapshot.
EVERY_KIND = [
    b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100|38=5|40=2|10=0|",
    b"8=FIX.4.4|35=S|52=20260921-10:00:01|117=Q1|55=AAPL|132=99|134=7|133=101|135=8|10=0|",
    b"8=FIX.4.4|35=8|52=20260921-10:00:02|17=E1|37=O1|55=AAPL|54=1|31=100|32=2|150=F|10=0|",
    b"8=FIX.4.4|35=AE|52=20260921-10:00:03|571=T1|487=0|55=AAPL|32=1|31=100|552=1|54=1"
    b"|1427=E2|1009=1|528=A|10=0|",
    b"8=FIX.4.4|35=W|52=20260921-10:00:04|55=AAPL|268=0|10=0|",
]

# An order stating four fields no typed column reads - OrdType(40),
# ExecInst(18), HandlInst(21) and DisplayQty(111) - beside the header, the
# trailer and its typed facts.
UNMAPPED_ORDER = (
    b"8=FIX.4.4|9=120|35=D|49=BUYER|56=VENUE|34=12|52=20260921-10:00:00|11=C1|1=ACC1|55=AAPL"
    b"|54=1|44=100.5|38=5|40=2|18=G|21=1|111=3|60=20260921-10:00:00|10=123|"
)


def test_market_data_answer_the_sorted_captures_leaves(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)
    unsorted = [
        codec.parse_fix_line(line)
        for line in (
            b"8=FIX.4.4|35=D|52=20260921-10:00:02|11=C2|55=AAPL|54=1|44=100|38=5|10=0|",
            b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|54=2|133=101|135=7|10=0|",
            b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=99|38=5|10=0|",
        )
    ]
    ordered = sorted(unsorted, key=lambda message: message.currunix)
    operations = codec.market_data(iter(unsorted))
    assert isinstance(operations, graph.MarketDataRowIterator)
    assert iter(operations) is operations
    leaves = list(operations)
    # The same leaves the message door answers over the sorted capture.
    assert leaves == [leaf for message in ordered for leaf in message.market_data()]
    # A sided message's cross code states its side (A17).
    assert [leaf.crosscode for leaf in leaves][1:] == ["10:1:C1", "10:1:C2"]
    assert next(operations, None) is None
    # The book door folds the unsorted capture in the order it reads it: an
    # operation dated before the book it would fold into is left out, with a
    # warning, so one book comes out; the sorted operations fold through the
    # stateful book, one book a leaf.
    assert codec.book_arrow_reader(unsorted).read_all().num_rows == 1
    assert len(list(graph.BookIterator(leaves))) == 3


def test_market_data_place_an_entry_clock_before_an_earlier_message(
    seed_batch: FixRegistry,
) -> None:
    codec = _fixed_batch(seed_batch)
    # The update's entry clock, 10:00:00.5, stands before the order sent at
    # 10:00:01, though the update itself was sent after it.
    capture = [
        codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=99|38=5|10=0|"),
        codec.parse_fix_line(
            b"8=FIX.4.4|35=X|52=20260921-10:00:02|55=AAPL|268=1|279=0|269=0|278=B1|270=100|271=10"
            b"|272=20260921|273=10:00:00.500|10=0|"
        ),
    ]
    leaves = list(codec.market_data(capture))
    assert [leaf.kind for leaf in leaves] == ["quote_event", "order_event"]
    assert len(list(graph.BookIterator(leaves))) == 2


def test_a_full_refresh_stating_no_entries_is_an_empty_snapshot(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)
    capture = [
        codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20260921-10:00:01|11=C1|55=AAPL|54=1|44=100|38=5|10=0|"),
        codec.parse_fix_line(b"8=FIX.4.4|35=W|52=20260921-10:00:02|55=AAPL|10=0|"),
        codec.parse_fix_line(b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|54=2|133=101|135=7|10=0|"),
    ]
    # Nothing a message states is refused: a W stating no NoMDEntries(268)
    # group clears its scope, with a warning, as the empty snapshot it is.
    operations = codec.market_data(capture)
    assert [leaf.kind for leaf in operations] == ["quote_event", "order_event", "snapshot_event"]
    assert codec.market_arrow_reader(capture).read_all().num_rows == 3


def test_a_python_failure_ends_the_capture_and_surfaces_at_its_end(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)

    def messages() -> Iterator[FixMsg]:
        yield codec.parse_fix_line(EVERY_KIND[0])
        raise RuntimeError("the source gave up")

    operations = codec.market_data(messages())
    assert next(operations).kind == "order_event"
    with pytest.raises(RuntimeError, match="the source gave up"):
        next(operations)
    assert next(operations, None) is None
    # The Arrow door carries it as the reader's own error, before any row.
    with pytest.raises(Exception, match="the source gave up"):
        codec.market_arrow_reader(messages()).read_all()
    # An item that is not a message is the pull's own failure.
    with pytest.raises(TypeError):
        list(codec.market_data([17]))
    with pytest.raises(TypeError):
        codec.market_data(5)


def test_market_arrow_reader_rows_state_every_kind_and_the_twin_agrees(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch, batch_row_size=2)
    capture = [codec.parse_fix_line(line) for line in EVERY_KIND]
    reader = codec.market_arrow_reader(capture)
    assert isinstance(reader, pa.RecordBatchReader)
    assert reader.schema == MarketData.field().into_arrow_schema()
    batches = list(reader)
    # A trade splits into sided messages at the parse (A12): the source
    # message a line reads as is no leaf of its own. A quote is one element
    # holding both its legs, never split, so the order, the two-sided quote,
    # the fill and the snapshot are the market data.
    assert [batch.num_rows for batch in batches] == [2, 2]
    table = pa.Table.from_batches(batches)
    assert table.column("marketdatakind").to_pylist() == [
        MarketDataKind.ORDR,
        MarketDataKind.QUOT,
        MarketDataKind.EXEC,
        MarketDataKind.BOOK,
    ]
    # The quote states its two legs and no side, under side 0.
    assert table.column("crosscode").to_pylist()[1] == "14:0:Q1"
    assert (table.column("bidpx").to_pylist()[1], table.column("askpx").to_pylist()[1]) == (
        decimal.Decimal("99"),
        decimal.Decimal("101"),
    )
    assert table.column("side").to_pylist()[1] == Side.UNKN
    direct = list(codec.market_data(capture))
    assert list(MarketData.from_arrow_reader(table)) == direct

    # The Arrow twin reads each FIX row as the message it holds.
    rows = codec.arrow_reader(fix_schema(seed_batch), capture)
    twin = codec.market_data_arrow_reader(rows)
    assert twin.schema == reader.schema
    assert list(MarketData.from_arrow_reader(twin)) == direct

    # A schema making no FIX row is refused before a row is read, as the
    # lifecycle twin refuses it.
    def foreign() -> pa.Table:
        return pa.Table.from_arrays([pa.array([1]), pa.array([2])], names=["a", "a"])

    with pytest.raises(ValueError) as refused:
        codec.market_data_arrow_reader(foreign())
    with pytest.raises(ValueError) as lifecycle:
        codec.lifecycle_arrow_reader(foreign())
    assert str(refused.value) == str(lifecycle.value)


def test_market_metadata_carries_what_no_typed_column_reads(seed_batch: FixRegistry) -> None:
    on = _fixed_batch(seed_batch)
    off = _fixed_batch(seed_batch, market_metadata=False)
    assert on.market_metadata is True
    assert off.market_metadata is False
    assert copy.copy(off).market_metadata is False
    message = on.parse_fix_line(UNMAPPED_ORDER)
    # The message itself holds no metadata: the map is computed per leaf.
    assert message.metadata == {}
    [stated] = on.market_data([message])
    [bare] = off.market_data([message])
    assert stated.metadata == {
        "execinst": "G",
        "handlinst": "1",
    }
    assert bare.metadata == {}
    # Account(1) is the leaf's `account` party id, with the metadata or without.
    for leaf in (stated, bare):
        assert _kinds(leaf.as_order_event().partyids) == {"account": "ACC1"}
    # The map is part of the leaf, so it is part of its identity.
    assert stated.curruuid != bare.curruuid
    assert stated.currhashcode != bare.currhashcode
    # The message door always carries it; the book door honours the switch.
    # With no grid the book states its deltas alone: the order is its one.
    assert message.market_data() == [stated]
    [book] = MarketData.from_arrow_reader(off.book_arrow_reader([message]))
    [live] = book.as_book_event().deltas  # type: ignore[union-attr]
    assert live.metadata == {}


def test_a_digital_asset_ticker_is_a_currency_no_iso_source_states(seed_batch: FixRegistry) -> None:
    """``ccy`` holds up to eight bytes; ``CurrencyCodeSource`` is stated only for an ISO 4217 code."""
    codec = _fixed_batch(seed_batch)

    def metadata(currency: bytes) -> dict[str, str]:
        line = (
            b"8=FIX.4.4|35=D|52=20240102-10:15:30|11=A1|55=AAPL|22=4|48=US0378331005|54=1|15="
            + currency
            + b"|38=100|44=10.5|60=20240102-10:15:31|10=0|"
        )
        message = codec.parse_fix_line(line)
        assert message.currency.as_py() == currency.decode()
        [leaf] = codec.market_data([message])
        return dict(leaf.metadata)

    assert metadata(b"USD")["currencycodesource"] == "6"
    # A ticker is a currency, up to the eight-byte bound, and ISO 4217 lists none.
    for ticker in (b"USDT", b"1INCH", b"BABYDOGE"):
        assert "currencycodesource" not in metadata(ticker)


def test_a_parse_places_each_message_among_the_messages_of_its_instant(seed_batch: FixRegistry) -> None:
    """A run of one instant counts its messages from zero in order; the next instant restarts."""
    order = b"8=FIX.4.4|35=D|52=20260102-10:15:30.000|11=A1|55=AAPL|54=1|38=100|10=0|"
    other = b"8=FIX.4.4|35=D|52=20260102-10:15:30.000|11=B1|55=MSFT|54=1|38=100|10=0|"
    later = b"8=FIX.4.4|35=D|52=20260102-10:15:31.000|11=C1|55=IBM|54=1|38=100|10=0|"
    for threads in (1, 4):
        codec = _fixed_batch(seed_batch, threads=threads)
        parsed = list(codec.parse_lines([order, other, order, later]))
        # By order: the order read again stands third at its instant, an
        # identity of its own.
        assert [held.seqnum for held in parsed] == [0, 1, 2, 0]
        assert parsed[0].curruuid != parsed[2].curruuid
        assert parsed[0].currhashcode == parsed[2].currhashcode
        # The place orders the identities of one millisecond.
        assert parsed[1].curruuid > parsed[0].curruuid
        # A place is where a message stands, never what it says.
        assert parsed[0].currhashcode != parsed[1].currhashcode


ULBRIDGE_LOG = pathlib.Path(__file__).resolve().parent.parent.parent / "rust" / "tests" / "fix" / "ulbridge.log"


def test_the_bridge_capture_reads_as_market_data_and_folds_into_books(
    seed_batch: FixRegistry,
) -> None:
    # The capture exactly as the bridge wrote it; the row header dates each
    # line by the clock written in front of it.
    options = TextOptions()
    options.rowheader = ULBRIDGE_ROWHEADER
    # The bridge writes Zurich's local time.
    options.timezone = "Europe/Zurich"
    options.start_rownum = 1
    source = IOBase.from_bytes(ULBRIDGE_LOG.read_bytes())
    source.media_type = Url("file:///ulbridge.log").media_type
    lines = list(source.read_text_lines(options=options))
    assert len(lines) == 144
    codec = _fixed_batch(seed_batch, capture_names=list(options.capture_names))
    messages = list(codec.parse_text_lines(lines))
    # Executions, two-sided quotes and trades split once, at the parse (A12,
    # A13): the trade capture's one side states no Side(54), so its
    # execution is of side UNKN.
    assert len(messages) == 151
    walked = list(codec.lifecycle(messages))
    # Three of the 42 deliveries are twins restating the live message they
    # repeat, under its identity: the window yields each identity once. The
    # capture's ten typeless rows state no sending time, so each is dated by
    # its own line: seven deliveries, none a twin.
    assert len(walked) == 39
    every = list(codec.with_dedup_window_ms(None).lifecycle(messages))
    assert len(every) == 42
    seen: set[object] = set()
    once = [held.curruuid for held in every if not (held.curruuid in seen or seen.add(held.curruuid))]
    assert once == [held.curruuid for held in walked]

    # Twenty-one deliveries are market data - eight fills and ten order
    # reports, a fill and a report of the twenty yielded once, and the NOVN
    # order's three steps: the venue's acknowledgement - an execution report
    # of no fill, its order's leaf - the restatement and the expiry - and
    # none is refused: the trade whose side states no Side(54) split into a
    # fill of side UNKN.
    operations = list(codec.market_data(walked))
    assert len(operations) == 21
    assert sorted(operation.kind for operation in operations) == ["execution_event"] * 8 + ["order_event"] * 13

    # Every order folds and every execution is pruned; the unpriced sell
    # order of 2454 rests at its side's unpriced level and leaves at the same
    # instant, so the last book holds nothing and states both as deltas. The
    # hash moved with the split fills, the sided cross codes,
    # the book's own bid and ask facts (A12, A17, A20), the metadata a
    # follower takes from its chain, the four-letter side code the
    # book's own unstated side feeds (`UNKN`), the time in force, an
    # enum member since it left the codes, feeding its code, and the
    # identifiers each delta digests as a source, a type and a value under
    # `identifiers`, `partyids` and `securityids`. It moved again with the
    # stored cross code (`3:0:2454`, kind and side before the base, which the
    # cross hash and every identity derive from) and the identifier maps
    # (keyed `src:type`, the bridge's own keys inferred into them). It moved
    # again when every type took its base key, spelled as the type alone -
    # the wire's identifiers digest under `base`, beside the base key a named
    # source fills - and when the bridge's `DETAILEDCFICODE` became a name of
    # `CFICode(461)`, folded into it: the one value `rust/tests/fix/ulbridge.rs`
    # pins for the same log. A book holds no execution - a fill moved its
    # book through its order's report - so a book stands at every instant an
    # order states, ten of them with the NOVN order's three steps, each its
    # deltas alone: with no grid and no snapshot input no book is whole. The
    # hash moved again when a book stopped holding executions, when a code's
    # first book came to state its deltas alone, and when a book came to be
    # keyed by its instrument's ISIN, holding it as its `isin`.
    books = list(graph.BookIterator(operations))
    assert len(books) == 10
    assert not any(book.is_complete for book in books)
    last = books[-1]
    assert last.ticker == "2454"
    assert (last.isincode, last.crosscode) == ("TW0002454006", "3:0:TW0002454006")
    assert last.alive == []
    assert [delta.price for delta in last.deltas] == [None, None]
    assert last.currhashcode == 9_341_042_899_919_963_577
    # Every book is keyed by the ISIN its inputs state, else their ticker:
    # the masked line's number keys its own book.
    assert sorted({book.crosscode for book in books}) == [
        "3:0:CH0012005267",
        "3:0:CH0012214059",
        "3:0:CH0012221716",
        "3:0:TW0001605004",
        "3:0:TW0002454006",
        "3:0:XX0000000001",
    ]

    # No leaf keys a typed fact - Account(1) is no typed fact since A1 - and
    # the fill line 105 carries states its bridge's own namespaced key, which
    # names a party: it is lifted into the leaf's `partyids` under the source
    # the key names (`tech:clientid`) and no longer stays in its metadata.
    for operation in operations:
        for typed in ("symbol", "side", "price", "orderqty", "clordid", "sendingtime"):
            assert typed not in operation.metadata
        assert "tech.clientid" not in operation.metadata
    assert any(
        getattr(operation.into_leaf(), "partyids", Identifiers()).get_from("tech:clientid") == "OMSX1"
        for operation in operations
    )


def test_lifecycled_two_sided_trade_streams_executions_and_no_book(
    seed_batch: FixRegistry,
) -> None:
    codec = _fixed_batch(seed_batch, batch_row_size=1)
    messages = list(
        codec.parse_line(
            b"8=FIX.4.4|35=AE|49=SELL|56=BUY|34=7|52=20260921-10:00:00|"
            b"571=T1|150=F|55=AAPL|32=10|31=101.25|60=20260921-10:00:00|552=2|"
            b"54=1|1427=BUY-EXEC|1009=4|37=BUY-ORDER|11=BUY-CLIENT|"
            b"54=2|1427=SELL-EXEC|1009=6|37=SELL-ORDER|11=SELL-CLIENT|10=0|"
        )
    )
    # The parse splits the trade once: the report keeps its own state beside
    # one filled execution per NoSides(552) occurrence, each sourced from it
    # (A12, A14).
    source, *fills = messages
    assert (source.state, source.side) == (State.TRADE, Side.UNKN)
    assert [(fill.state, fill.side) for fill in fills] == [
        (State.FILLED, Side.BUYS),
        (State.FILLED, Side.SELL),
    ]
    assert all(source.curruuid in fill.srcuuids for fill in fills)

    # A book holds no execution - a fill moves a book through its order's
    # report - so an instant only executions touched emits no book.
    assert codec.book_arrow_reader(codec.lifecycle(messages), snapshot_millis=0).read_all().num_rows == 0

    rows = codec.market_arrow_reader(codec.lifecycle(messages)).read_all().to_pylist()
    assert len(rows) == 2
    by_side = {execution["side"]: execution for execution in rows}
    assert set(by_side) == {Side.BUYS, Side.SELL}
    buy, sell = by_side[Side.BUYS], by_side[Side.SELL]
    # A trade-capture side states no price and no quantity: its last
    # executed price and quantity are lastpx and lastqty, and the two
    # nullable columns carry the null.
    assert (buy["price"], sell["price"]) == (None, None)
    assert (buy["quantity"], sell["quantity"]) == (None, None)
    assert (buy["lastpx"], sell["lastpx"]) == (
        decimal.Decimal("101.25"),
        decimal.Decimal("101.25"),
    )
    assert (buy["lastqty"], sell["lastqty"]) == (
        decimal.Decimal("4"),
        decimal.Decimal("6"),
    )
    assert all(
        execution["marketdatakind"] == MarketDataKind.EXEC
        and execution["ticker"] == "AAPL"
        and execution["currunix"] == pa.scalar(source.currunix, pa.timestamp("ns", "UTC")).as_py()
        for execution in by_side.values()
    )
    buy_ids, sell_ids = _row_kinds(buy["identifiers"]), _row_kinds(sell["identifiers"])
    assert buy_ids["execid"] == "BUY-EXEC"
    assert sell_ids["execid"] == "SELL-EXEC"
    assert buy_ids["orderid"] == "BUY-ORDER"
    assert sell_ids["orderid"] == "SELL-ORDER"
    assert buy_ids["clordid"] == "BUY-CLIENT"
    assert sell_ids["clordid"] == "SELL-CLIENT"
    assert buy["curruuid"] != sell["curruuid"]
    assert buy["crossuuid"] != sell["crossuuid"]
    # Each fill's stored cross code is its kind and its side, then its base.
    assert buy["crosscode"].startswith("8:1:") and sell["crosscode"].startswith("8:2:")


@pytest.mark.parametrize(
    ("body", "fills", "books"),
    [
        (
            b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|150=F|55=AAPL|"
            b"32=4|31=101.25|60=20260921-10:00:00|552=1|"
            b"1427=NO-SIDE|1009=4|37=ORDER-1|11=CLIENT-1|10=0|",
            [Side.UNKN],
            0,
        ),
        (
            b"8=FIX.4.4|35=AE|52=20260921-10:00:00|571=T1|150=F|55=AAPL|"
            b"32=0|31=101.25|60=20260921-10:00:00|552=0|10=0|",
            [],
            0,
        ),
    ],
)
def test_a_trade_side_stating_no_side_splits_into_a_fill_of_side_unknown(
    seed_batch: FixRegistry, body: bytes, fills: list[Side], books: int
) -> None:
    # The executions split once, at the parse (A12): a side stating no
    # Side(54) is still a fill, of side UNKN, said as a warning - never a
    # lost fill - and a trade stating no side at all splits into none. The
    # report itself is no leaf of a book, and a book holds no execution, so
    # neither emits a book.
    codec = _fixed_batch(seed_batch)
    trade, *split = codec.parse_line(body)
    assert (trade.state, trade.side) == (State.TRADE, Side.UNKN)
    assert [fill.side for fill in split] == fills
    rows = codec.book_arrow_reader([trade, *split], snapshot_millis=0).read_all()
    assert rows.num_rows == books


def test_messages_pull_from_the_reader_one_batch_at_a_time(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)
    source = codec.parse_text_arrow_reader(_capture(CARRYING, 3))
    stream = codec.messages(source)
    assert isinstance(stream, FixMessages)
    assert len(list(stream)) == len(CARRYING)


def test_a_python_failure_behind_a_batch_stream_arrives_with_the_batch(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)

    def messages() -> Iterator[FixMsg]:
        yield _one(codec, CARRYING[0])
        raise RuntimeError("the source gave up")

    reader = codec.arrow_reader(fix_schema(seed_batch), messages())
    # The failure travels as the reader's own error and arrives where the
    # batch it would have landed in does, carrying the native sentence.
    with pytest.raises(Exception, match="the source gave up"):
        reader.read_all()


def test_byte_in_byte_out_over_the_whole_corpus(seed_batch: FixRegistry) -> None:
    # The convention that drops a stated absence is deliberately not
    # byte-preserving, so it is turned off to measure the reader.
    codec = _fixed_batch(seed_batch, null_values=[], separator=124)
    sink = io.BytesIO()
    written = codec.write_arrow_reader(
        codec.parse_text_arrow_reader(_capture(CARRYING, len(CARRYING))), sink
    )
    assert written == len(CARRYING)
    back = sink.getvalue().split(b"\n")
    assert back.pop() == b""
    assert len(back) == len(CARRYING)


def test_a_batch_with_no_record_cannot_be_written(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)
    batch = codec.parse_text_arrow_reader(_capture(CARRYING, len(CARRYING))).read_all()
    sink = io.BytesIO()
    with pytest.raises(ValueError, match="fixentries"):
        codec.write_arrow_reader(batch.select(["symbol"]), sink)
    assert sink.getvalue() == b"", "refused before a row was read"

    class Refusing(io.RawIOBase):
        def write(self, _: Any) -> int:
            raise OSError("disk full")

    with pytest.raises(OSError, match="disk full"):
        codec.write_arrow_reader(batch, Refusing())


def test_a_row_reads_back_into_the_message_that_made_it(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)
    schema = fix_schema(seed_batch)
    parsed = _one(codec, ORDER)
    row = parsed.into_row(schema)

    held = FixMsg.from_row(schema, row, seed_batch)

    # Rebuilding combines projected fields with residual entries; semantic row
    # equality, rather than arrival entry order or wire spelling, is the contract.
    assert held.by_tag(11) == parsed.by_tag(11)
    assert held.by_tag(55) == parsed.by_tag(55)
    assert held.into_row(schema) == row
    assert held.currhashcode == parsed.currhashcode
    assert held.curruuid == parsed.curruuid
    assert held.crossuuid == parsed.crossuuid
    assert held.currunix == parsed.currunix
    assert held.header() == parsed.header()
    for tag in (11, 55):
        assert held.by_tag(tag) == parsed.by_tag(tag), tag
    # And it makes the row it came from, whole, without reading a clock.
    assert held.into_row(schema) == row
    # A recorded identity must not suppress derivation of projected market
    # facts when the row is reconstructed.
    sided = _one(codec, b"8=FIX.4.4|35=D|11=SIDE-1|55=AAPL|54=1|38=100|10=0|")
    assert sided.side is Side.BUYS
    restored = FixMsg.from_row(schema, sided.into_row(schema), seed_batch)
    assert restored.side is Side.BUYS
    assert restored.by_tag(54).as_py() is Side.BUYS

    # The process default is the registry when none is named.
    assert FixMsg.from_row(schema, row).registry is not None


def test_rows_prune_projected_scalars_and_complete_groups_from_residual_entries(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)
    schema = fix_schema(seed_batch)
    message = _one(
        codec,
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|453=1|448=BRK|447=D|452=1|9999=x|10=0|",
    )
    row = message.into_row(schema)
    # `fixentries` is a sorted map keyed `tag:name` (A8): what a column
    # projects is no entry of it, and a key no dictionary resolves lives in
    # the row's metadata under its own spelling rather than as an entry (A9).
    # `Parties(453)` is no column - the row's `partyids` states the party -
    # so the group is an entry, kept as sent.
    residual = row.as_py()[schema.index_of("fixentries")]
    assert not any(key.split(":")[0] == "55" for key in residual)
    assert "453:parties" in residual
    assert [str(id) for id in message.partyids] == ["executingfirm=BRK", "proprietary:executingfirm=BRK"]
    assert row.as_py()[schema.index_of("metadata")] == {"9999": "x"}
    rebuilt = FixMsg.from_row(schema, row, seed_batch)
    assert rebuilt.by_tag(55).as_py() == "AAPL"
    assert len(rebuilt.by_name("parties").as_py()) == 1
    assert rebuilt.get_by_tag(453) is None
    assert rebuilt.into_row(schema) == row


def test_a_bridges_code_aliases_land_in_securityids_and_leave_the_rows_metadata(
    seed_batch: FixRegistry,
) -> None:
    # A bridge's code-like keys each land in `securityids`: the crate's own
    # `ISINCODE` and `BLOOMBERGCODE` views, and the keyed `RICCODE`,
    # `OMS_CUSIPCODE` and `SEDOLCODE` an unmapped key is read by. The row's
    # `metadata` holds none of them - a keyed one rides `fixentries` under
    # `0:<key>`, a view its own column - and only a value a type refuses by
    # shape stays in `metadata`; the row read back restates the sets, the
    # digest and the identity.
    codec = _fixed_batch(seed_batch)
    schema = fix_schema(seed_batch)
    held = _one(
        codec,
        b"8=FIX.4.4|35=D|11=C1|ISINCODE=US0378331005|BLOOMBERGCODE=AAPL US Equity|"
        b"OMS_CUSIPCODE=037833100|RICCODE=AAPL.O|SEDOLCODE=2046251|#CUSIPCODE=03783310|10=0|",
    )
    assert _kinds(held.securityids) == {
        "bloomberg": "AAPL US Equity",
        "cusip": "037833100",
        "isin": "US0378331005",
        "ric": "AAPL.O",
        "sedol": "2046251",
    }
    assert held.securityids.get_from("oms:cusip") == "037833100"
    assert [field for field, _ in held.anomalies] == ["cusipcode"]

    row = held.into_row(schema)
    cells = row.as_py()
    assert cells[schema.index_of("metadata")] == {"cusipcode": "03783310"}
    residual = cells[schema.index_of("fixentries")]
    assert {key: value for key, value in residual.items() if key.startswith("0:")} == {
        "0:omscusipcode": "037833100",
        "0:riccode": "AAPL.O",
        "0:sedolcode": "2046251",
    }
    assert (cells[schema.index_of("isincode")], cells[schema.index_of("bloombergcode")]) == (
        "US0378331005",
        "AAPL US Equity",
    )
    again = FixMsg.from_row(schema, row, seed_batch)
    assert again.securityids == held.securityids
    assert (again.currhashcode, again.curruuid) == (held.currhashcode, held.curruuid)


def test_a_captures_own_columns_are_carried_and_never_become_facts(seed_batch: FixRegistry) -> None:
    """A message carries what the reader said about its line, and states none of it."""
    codec = _fixed_batch(seed_batch)
    # The object the line came out of is one of the capture's own columns: no
    # column of the fixed row states it, so a capture that knows it declares
    # it beside the body and the row number.
    capture = Field(
        "line",
        DataType.from_fields(
            [
                Field("url", "utf8"),
                Field("rownum", "int64"),
                Field("body", "binary"),
                Field("sourceurl", "url"),
            ]
        ),
        nullable=False,
    )
    schema = fix_schema_carrying(capture, fix_schema(seed_batch))
    # A carried column is nullable whatever the capture declared: only a pass
    # holding the source row can state one.
    assert schema.field("url").nullable
    parsed = _one(codec, ORDER)

    # A message parsed out of a line carries nothing: the capture's own
    # columns are null in its row, the one the crate tags among them.
    assert parsed.carried == []
    row = parsed.into_row(schema)
    held_row = row.as_py()
    for carrier in ("url", "rownum", "body", "sourceurl"):
        assert held_row[schema.index_of(carrier)] is None, carrier

    # A row a reader stated them on reads back carrying them, each under its
    # column's name: no child, no entry, nothing to answer by name, and the
    # content identity untouched.
    stated = list(held_row)
    stated[schema.index_of("url")] = "file:///capture.log"
    stated[schema.index_of("rownum")] = 42
    again = FixMsg.from_row(schema, stated, seed_batch)
    assert again.into_row(schema).as_py() == stated
    assert again.currhashcode == parsed.currhashcode
    assert {name: value.as_py() for name, value in again.carried} == {
        "url": "file:///capture.log",
        "rownum": 42,
    }
    for carrier in ("url", "rownum", "body"):
        assert again.field.index_of(carrier) is None, carrier
    # And writing one onto the message is refused rather than silently kept.
    with pytest.raises(ValueError):
        again.set("sourceurl", "file:///capture.log")

    # So a message states them again at their columns, and nowhere else.
    written = again.into_row(schema).as_py()
    assert written[schema.index_of("url")] == "file:///capture.log"
    assert written[schema.index_of("rownum")] == 42
    for carrier in ("body", "sourceurl"):
        assert written[schema.index_of(carrier)] is None, carrier
    assert "65048=" not in again.into_text("|")


def test_a_row_without_the_entries_column_keeps_projected_content(seed_batch: FixRegistry) -> None:
    codec = _fixed_batch(seed_batch)
    wide = fix_schema(seed_batch)
    narrow = Field(
        "fix",
        DataType.from_fields(
            [column for column in wide if column.name != "fixentries"]
        ),
        nullable=False,
    )
    parsed = _one(codec, ORDER)
    row = parsed.into_row(narrow)
    held = FixMsg.from_row(narrow, row, seed_batch)
    # A row without residual entries still reconstructs projected content.
    assert held.by_tag(55).as_py() == "AAPL"
    assert held.header() == parsed.header()
    assert held.currunix == parsed.currunix
    again = held.into_row(narrow).as_py()
    assert again[narrow.index_of("symbol")] == "AAPL"
    assert again[narrow.index_of("currunix")] == row.as_py()[narrow.index_of("currunix")]
    # A row that does not fit the schema is refused.
    with pytest.raises(ValueError):
        FixMsg.from_row(narrow, {"nosuchcolumn": 1}, seed_batch)


def test_a_group_counting_none_is_stated_and_a_list_read_back_empty_is_not(
    seed_batch: FixRegistry,
) -> None:
    """A group is its list alone, and one holding nothing is stated empty.

    ``802=0`` is the list holding nothing and re-emits; a column holds a group
    as null or as at least one occurrence, so the zero rides the residual
    record, and PyIceberg reads a null list of structs back as ``[]``, which
    states nothing: each row read back and settled again from its content is
    the message the parse wrote.
    """
    codec = _fixed_batch(seed_batch)
    party = b"8=FIX.4.4|35=D|49=S|56=T|34=7|11=A|55=AAPL|54=1|453=1|448=X|447=D|452=1|"
    absent = _one(codec, party + b"10=0|")
    counted = _one(codec, party + b"802=0|10=0|")
    assert counted.currhashcode != absent.currhashcode
    assert counted.digest() != absent.digest()
    assert "|452=1|802=0|" in counted.into_text("|")
    assert "802=" not in absent.into_text("|")

    # A row that does not record its content code is settled from what it
    # states; it holds the dictionary's `Parties(453)` group as a column, in
    # place of the `partyids` identifiers, as a narrower table may.
    wide = fix_schema(seed_batch)
    group_field = copy.copy(seed_batch.field_by_counter(453))
    group_field.set_nullable(True)
    columns = []
    for column in wide:
        if column.name == "fixentries":
            columns.append(group_field)
        if column.name not in ("currhashcode", "partyids"):
            columns.append(column)
    narrow = Field("fix", DataType.from_fields(columns), nullable=False)
    parties = narrow.index_of("parties")
    party_item = narrow.field_by_path("parties").dtype.field_at(0)
    subids = party_item.index_of("partysubids")
    assert party_item.index_of("nopartysubids") is None
    for message, stated in ((absent, False), (counted, True)):
        written = message.into_row(narrow).as_py()
        assert written[parties][0][subids] is None
        read_back = copy.deepcopy(written)
        read_back[parties][0][subids] = []
        for row in (written, read_back):
            held = FixMsg.from_row(narrow, row, seed_batch)
            assert held.currhashcode == message.currhashcode
            assert held.curruuid == message.curruuid
            assert ("|802=0|" in held.into_text("|")) is stated


def test_a_root_group_counting_none_is_stated_and_a_map_read_back_empty_is_not(
    seed_batch: FixRegistry,
) -> None:
    """The same rule at the root: ``453=0`` is stated, ``partyids = {}`` is not.

    A ``partyids`` identifier map read back as ``{}`` where the row held null is
    the group absent.
    """
    codec = _fixed_batch(seed_batch)
    order = b"8=FIX.4.4|35=D|49=S|56=T|34=7|11=A|55=AAPL|54=1|"
    absent = _one(codec, order + b"10=0|")
    counted = _one(codec, order + b"453=0|10=0|")
    assert "|453=0|" in counted.into_text("|")
    assert "453=" not in absent.into_text("|")

    wide = fix_schema(seed_batch)
    narrow = Field(
        "fix",
        DataType.from_fields([column for column in wide if column.name != "currhashcode"]),
        nullable=False,
    )
    partyids = narrow.index_of("partyids")
    for message, stated in ((absent, False), (counted, True)):
        written = message.into_row(narrow).as_py()
        read_back = copy.deepcopy(written)
        if read_back[partyids] is None:
            read_back[partyids] = {}
        for row in (written, read_back):
            held = FixMsg.from_row(narrow, row, seed_batch)
            assert held.currhashcode == message.currhashcode
            assert held.curruuid == message.curruuid
            assert ("|453=" in held.into_text("|")) is stated


def test_the_lifecycle_twin_walks_the_rows_a_batch_holds(seed_batch: FixRegistry) -> None:
    """``lifecycle`` over batches: the same schema in and out, nothing reparsed."""
    codec = _fixed_batch(seed_batch)
    schema = fix_schema(seed_batch)
    life = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=2|55=AAPL|60=20260102-10:15:31.000|10=0|",
    ]
    parsed = list(codec.parse_lines(life))
    table = codec.arrow_reader(schema, parsed).read_all()
    # A parse chains nothing, but places each message within its instant:
    # all four are undated, so they share the codec's default and count up.
    # The fill splits into its report and an execution of its own (A12).
    assert _column(table, "seqnum") == [None, 1, 2, 3]
    assert _column(table, "prevuuid") == [None, None, None, None]

    walked = codec.lifecycle_arrow_reader(table).read_all()
    assert walked.schema == table.schema, "the same schema in and out"
    # The order's later steps each keep their own place; the execution
    # shares the fill's instant with its report and stands after it.
    assert _column(walked, "seqnum") == [None, None, None, 1], "a later instant keeps its own place"
    assert _column(walked, "prevuuid")[0] is None and _column(walked, "prevuuid")[3] is None
    assert all(held is not None for held in _column(walked, "prevuuid")[1:3])
    assert _column(walked, "fixentries") == _column(table, "fixentries"), "the record is untouched"

    # The rows the twin wrote are the messages the stream walk answers.
    by_rows = list(codec.messages(walked))
    by_stream = list(codec.lifecycle(parsed))
    for held, message in zip(by_rows, by_stream):
        assert held.seqnum == message.seqnum
        assert held.prevuuid == message.prevuuid
        assert held.curruuid == message.curruuid
        assert held.crossuuid == message.crossuuid
        assert held.currhashcode == message.currhashcode


def test_format_answers_the_rows_one_message_field_holds(seed_batch: FixRegistry) -> None:
    """The verb a consumer reads by: messages in, rows under a field out."""
    codec = _fixed_batch(seed_batch)
    target = fix_schema(seed_batch)

    messages = list(codec.parse_line(ORDER))
    rows = codec.format_messages(messages, target)
    assert len(rows) == 1
    held = rows[0].as_py()
    assert held[target.index_of("symbol")] == "AAPL"
    # The keys no dictionary resolves are the row's metadata (A9).
    assert held[target.index_of("metadata")]

    # The Arrow twin answers the same row, deciding its schema before a row
    # is read.
    source = codec.arrow_reader(fix_schema(seed_batch), messages)
    formatted = codec.format_arrow_reader(source, target)
    assert [field.name for field in formatted.schema] == [column.name for column in target]
    batched = formatted.read_all()
    assert batched.num_rows == 1
    assert batched.column("symbol").to_pylist() == ["AAPL"]


def test_a_column_a_narrow_row_dropped_is_lifted_out_of_the_record(seed_batch: FixRegistry) -> None:
    """Formatting a narrow row into a wider field reads the record."""
    codec = _fixed_batch(seed_batch)
    wide = fix_schema(seed_batch)
    keep = (
        "beginstring",
        "msgtype",
        "sendingtime",
        "currunix",
        "creaunix",
        "currhashcode",
        "crosshashcode",
        "curruuid",
        "crossuuid",
        "fixentries",
    )
    narrow = Field(
        "fix",
        DataType.from_fields([wide[wide.index_of(name)] for name in keep]),
        nullable=False,
    )
    assert narrow.index_of("symbol") is None

    parsed = _one(codec, ORDER)
    stored = parsed.into_row(narrow)
    held = FixMsg.from_row(narrow, stored, seed_batch)
    row = codec.format_messages([held], wide).pop().as_py()
    assert row[wide.index_of("symbol")] == "AAPL"


def _field_catalog(name: str, tag: int, dtype: str = "utf8") -> Field:
    value = Field(name, dtype)
    value.fix.tag = tag
    return value


def _message(name: str, code: str, members: list[Field] | None = None) -> Field:
    value = Field(name, DataType.from_fields(members or []), nullable=False)
    value.fix.msgtype = code
    return value


def _catalog(members: Iterable[Field] = ()) -> FixRegistry:
    """`Party` referencing `PartyID`, restated by a group and a message."""
    registry = FixRegistry.from_fields([_field_catalog("NoPartyIDs", 453, "int32"), _field_catalog("PartyID", 448)])
    member = registry.field(448)
    member.fix.field_ref = "PartyID"
    registry.insert(Field("Party", DataType.from_fields([member, *members]), nullable=False))
    component = registry.field_by_name("Party")
    group = yggdryl.serie("Parties", component)
    group.fix.counter = 453
    group.fix.component = "Party"
    registry.insert(group)
    group = registry.field_by_name("Parties")
    group.fix.group = "Parties"
    # A group is its list alone: the counter is the dictionary's field and
    # no member of the message beside it.
    registry.insert(_message("NewOrderSingle", "D", [group]))
    return registry


def test_a_definition_is_filed_by_the_shape_it_has() -> None:
    """One `insert`: a Struct is a component, a Serie or a Map a group."""
    registry = _catalog()

    component = registry.field_by_name("Party")
    assert component.dtype.is_nested
    assert [member.name for member in component] == ["PartyID"]

    group = registry.field_by_name("Parties")
    assert group.fix.counter == 453
    assert group.fix.component == "party"
    # A group is reached by the counter it opens as well as by its name, and
    # the counter itself is still the scalar field it is.
    assert registry.field_by_counter(453).name == "Parties"
    assert registry.get_field_by_counter(9999) is None
    assert registry.field_by_tag(453).dtype == DataType("int32")

    # A message is a component carrying `FIX:msgtype`, so the same door
    # reaches it and the message-type view names it.
    message = registry.field_by_name("NewOrderSingle")
    assert message.fix.msgtype == "D"
    assert registry.msgtype("D").name == "NewOrderSingle"
    # The path grammar walks through the group into its member.
    assert registry.field_by_path("NewOrderSingle.Parties.PartyID").fix.tag == 448

    # A definition answers the tag door too, under the identity the catalog
    # derived for it - never under a tag a caller could claim.
    tag = component.fix.tag
    assert tag is not None
    assert registry.get_field_by_tag(tag).name == "Party"


def test_update_merges_a_definition_and_remove_keeps_a_referenced_one() -> None:
    registry = _catalog()
    before = registry.into_json()

    # A definition another one references stays, and answers `None`.
    for name in ("PartyID", "Party", "Parties"):
        assert registry.remove(name) is None
    assert registry.into_json() == before

    # `update` merges into the definition the folded name reaches, and every
    # reference to it sees the change without holding a copy.
    component = registry.field_by_name("Party")
    component.fix.description = "Reviewed"
    registry.update(component)
    assert registry.field_by_name("Party").fix.description == "Reviewed"
    assert registry.field_by_path("NewOrderSingle.Parties.PartyID") is not None
    assert registry.field_by_name("parties").fix.counter == 453

    # A definition nothing holds is refused by `update`.
    with pytest.raises(ValueError):
        registry.update(Field("Nope", DataType.from_fields([]), nullable=False))

    # Removed in reference order, each answers the definition it took out.
    for name in ("NewOrderSingle", "Parties", "Party", "PartyID", "NoPartyIDs"):
        assert registry.remove(name) is not None, name
        assert registry.get_field_by_name(name) is None, name
        assert registry.remove(name) is None, name

    # Only what every registry is built with is left.
    assert len(registry) == len(FixRegistry())
    assert [registry.field(tag).name for tag in (52, 60)] == ["sendingtime", "transacttime"]
    assert registry.field_by_counter(METADATA_TAG).name == "metadata"


def test_a_code_set_is_the_dictionarys_and_a_snapshot_preserves_every_definition() -> None:
    registry = _catalog()
    # The members are stated first: a dictionary refuses a field naming a
    # vocabulary nothing states.
    registry.set_codeset("partyidcodeset", [{"value": "B", "name": "Broker"}])
    value = registry.field(448)
    value.fix.codeset = "partyidcodeset"
    # Membership is metadata like any other: it travels with the field.
    value.fix.branches = ["Pending"]
    registry.update(value)
    # Every reference reaches the one field, and the field names the one set.
    member = registry.field_by_path("NewOrderSingle.Parties.PartyID")
    assert member.fix.codeset == "partyidcodeset"
    assert member.metadata["FIX:codeset"] == "partyidcodeset"
    codes = registry.codeset_of(member)
    assert codes is not None and [code["name"] for code in codes] == ["Broker"]
    assert registry.dialects() == ["pending"]

    document = json.loads(registry.into_json())
    # The vocabularies are the fourth key, and they lead the document: a
    # field names the set it reads by, so a reader holds the sets before it
    # meets a field naming one.
    assert set(document) == {"codesets", "fields", "components", "groups"}
    assert [held["name"] for held in document["codesets"]] == [
        "marketdatatypecodeset",
        "msgcatcodeset",
        "partyidcodeset",
        "statecodeset",
    ]
    with pytest.raises(TypeError):
        hash(registry)

    for restored in (
        FixRegistry.from_json(registry.into_json()),
        copy.copy(registry),
        copy.deepcopy(registry),
        pickle.loads(pickle.dumps(registry)),
    ):
        assert restored == registry
        assert restored.stable_hash() == registry.stable_hash()
        assert restored.dialects() == ["pending"]
        assert restored.field(448).fix.has_branch("PENDING")
        assert restored.field_by_name("Parties").fix.counter == 453
        assert restored.msgtype("D").name == "NewOrderSingle"

    changed = copy.copy(registry)
    message = changed.field_by_name("NewOrderSingle")
    message.fix.description = "Different message definition"
    changed.update(message)
    assert changed != registry
    assert changed.stable_hash() != registry.stable_hash()


def test_a_store_round_trips_every_definition(tmp_path: Any) -> None:
    registry = _catalog()
    registry.set_codeset("partyidcodeset", [{"value": "B", "name": "Broker"}])
    member = registry.field(448)
    member.fix.codeset = "partyidcodeset"
    registry.update(member)
    root = tmp_path / "catalog"
    registry.write_into(root)

    # The definitions land in the folders their shapes name, beside the
    # crate's own dump of the fixed row; a vocabulary is the fourth folder,
    # filed under the name the field that reads by it states.
    assert (root / "components" / "Party.json").exists()
    assert (root / "groups" / "Parties.json").exists()
    assert (root / "components" / "fixmsg.json").exists()
    assert (root / "codesets" / "partyidcodeset.json").exists()

    reloaded = FixRegistry.from_handle(root)
    assert reloaded == registry
    assert reloaded.field_by_path("NewOrderSingle.Parties.PartyID").fix.tag == 448
    assert [code["name"] for code in reloaded.codeset("partyidcodeset")] == ["Broker"]
    assert reloaded.msgtype("D").get_group_by_tag(453).name == "Parties"


def test_a_message_type_is_an_immutable_view_that_pins_its_registry() -> None:
    registry = _catalog()
    singleton = registry.msgtype("D")
    assert isinstance(singleton, MsgType)
    assert singleton.name == "NewOrderSingle"
    assert singleton.value == "D"
    assert str(singleton) == "D"
    assert repr(singleton) == 'MsgType("NewOrderSingle", "D")'
    assert singleton.field.dtype == registry.field_by_name("NewOrderSingle").dtype
    assert singleton.get_group_by_tag(453).name == "Parties"
    assert singleton.get_group_by_tag(9999) is None

    # The view is read-only and pins the registry while it lives.
    with pytest.raises((AttributeError, TypeError, ValueError)):
        singleton.field.set_name("Changed")
    with pytest.raises(ValueError, match="shared"):
        registry.update(registry.field_by_name("NewOrderSingle"))

    for restored in (
        copy.copy(singleton),
        copy.deepcopy(singleton),
        pickle.loads(pickle.dumps(singleton)),
    ):
        assert restored == singleton
        assert hash(restored) == hash(singleton)
        assert restored.stable_hash() == singleton.stable_hash()

    with pytest.raises(TypeError):
        MsgType()


def test_a_wire_code_reaches_the_message_it_names() -> None:
    registry = FixRegistry()
    registry.insert(_message("D", "X"))
    registry.insert(_message("NewOrderSingle", "D"))
    registry.insert(_message("BridgeReport", "P Report Ack"))

    # A new registry seeds no message type of its own.
    assert FixRegistry().get_msgtype("D") is None
    assert registry.msgtype("D").name == "NewOrderSingle"
    assert registry.msgtype("bridgereport").value == "P Report Ack"
    assert registry.get_msgtype("p report ack") is None
    assert registry.msgtype("neworder_single").value == "D"

    # Ordering is the definition's own, so a sort is the core's.
    left, right = registry.msgtype("D"), registry.msgtype("bridgereport")
    assert (left < right) != (right < left)
    assert sorted([right, left]) in ([left, right], [right, left])
    with pytest.raises(TypeError):
        left < "ZZ"


def test_message_singleton_identifiers_are_compiled_once() -> None:
    client, order = _field_catalog("clordid", 11), _field_catalog("orderid", 37)
    numeric = _field_catalog("numericid", 9001, "int64")
    declaration = _message("order", "D", [client, order, numeric])
    declaration.fix.identifiers = ["9001", "37", "11"]
    registry = FixRegistry.from_fields([client, order, numeric])
    registry.insert(declaration)

    row = Field(
        "row",
        DataType.from_fields([_field_catalog("venueorder", 37), client, numeric]),
        nullable=False,
    )
    numeric_value = 9_007_199_254_740_993
    message = FixMsg(row, ["O-1", "C-1", numeric_value], registry)
    singleton = registry.msgtype("D")
    selected = singleton.identifier_values(message)
    assert [(field.name, value.as_py()) for field, value in selected] == [
        ("clordid", "C-1"),
        ("orderid", "O-1"),
        ("numericid", numeric_value),
    ]
    assert selected[2][1].dtype == DataType("int64")
    assert type(selected[2][1].as_py()) is int
    with pytest.raises(TypeError, match="read-only"):
        selected[0][0].set_name("changed")

    absent = FixMsg(row, [None, "C-1", None], registry)
    assert [field.name for field, _ in singleton.identifier_values(absent)] == ["clordid"]
    with pytest.raises(TypeError):
        singleton.identifier_values("not a message")  # type: ignore[arg-type]


def test_identifier_declarations_merge_whole() -> None:
    client, order = _field_catalog("clordid", 11), _field_catalog("orderid", 37)
    registry = FixRegistry.from_fields([client, order])
    for member in (client, order):
        member.fix.field_ref = member.name
    stored = _message("order", "D", [client, order])
    stored.fix.identifiers = ["clordid"]
    registry.insert(stored)

    incoming = _message("order", "D", [order, client])
    incoming.fix.identifiers = ["37", "11"]
    assert incoming.fix.identifiers == ["orderid", "clordid"]
    # An incoming declaration replaces the previous one whole.
    registry.update(incoming)
    held = registry.field_by_name("order").fix.identifiers
    assert sorted(held) == ["clordid", "orderid"]

    restored = FixRegistry.from_json(registry.into_json())
    assert restored == registry
    assert restored.field_by_name("order").fix.identifiers == held

    malformed = copy.copy(incoming)
    malformed.metadata["FIX:identifiers"] = "clordid,,orderid"
    before = registry.into_json()
    with pytest.raises(ValueError, match="FIX:identifiers"):
        registry.update(malformed)
    assert registry.into_json() == before


def test_merge_with_folds_definitions_and_unions_their_membership() -> None:
    target, source = _catalog(), _catalog()
    # One name, two statements of it: the fold is the dictionary's to make,
    # and the field only ever carries the name.
    for registry, code, name in ((target, "B", "Broker"), (source, "C", "Client")):
        registry.set_codeset("partyidcodeset", [{"value": code, "name": name}])
        member = registry.field(448)
        member.fix.codeset = "partyidcodeset"
        registry.update(member)
    message = source.field_by_name("NewOrderSingle")
    message.set_name("IncomingOrder")
    message.fix.msgtype = "I"
    source.insert(message)
    before_source = source.into_json()

    # The counts are over the fields, which both dictionaries already hold.
    report = target.merge_with(source)
    assert report["sources"] == 1
    assert report["added"] == 0
    assert report["merged"] >= 2
    assert report["dropped"] == []
    for path in ("PartyID", "Party.PartyID", "Parties.PartyID", "NewOrderSingle.Parties.PartyID"):
        member = target.field_by_path(path)
        assert member.fix.codeset == "partyidcodeset", path
        codes = target.codeset_of(member)
        assert codes is not None, path
        assert {item["value"]: item["name"] for item in codes} == {"B": "Broker", "C": "Client"}, path
    assert target.msgtype("I").get_group_by_tag(453).name == "Parties"
    assert source.into_json() == before_source, "the source is untouched"

    restored = FixRegistry.from_json(target.into_json())
    assert restored == target
    assert restored.stable_hash() == target.stable_hash()


def test_a_json_row_is_one_unknown_message_keeping_its_source_columns() -> None:
    registry = FixRegistry()
    # A document states no message type, so this codec reads the untyped row
    # the default refusals drop.
    codec = FixCodec(registry, exclude_msgtypes=[])
    raw = json.dumps({"request": {"type": "read"}, "status": 200}).encode()

    messages = codec.parse_line(raw)
    assert isinstance(messages, FixMessages)
    assert iter(messages) is messages
    values = list(messages)
    assert len(values) == 1
    assert values[0].field.name == "unknown"
    assert values[0].entries() == []
    assert values[0].get_by_name("status") is None
    assert next(messages, None) is None

    # One byte a batch is a batch a row, and a document row keeps the
    # columns it arrived with.
    capture = pa.table(
        {"url": ["capture.log"], "rownum": [17], "body": pa.array([raw], type=pa.binary())}
    )
    output = (
        FixCodec(registry, batch_byte_size=1, exclude_msgtypes=[])
        .parse_text_arrow_reader(capture)
        .read_all()
    )
    assert output.num_rows == 1
    assert output.column("url").to_pylist() == ["capture.log"]
    assert output.column("rownum").to_pylist() == [17]

    # Any JSON object is the same row through every door.
    stranger = b'{"a":1}'
    assert next(codec.parse_line(stranger)).field.name == "unknown"
    assert next(codec.parse_text_line(TextLine(17, stranger))).field.name == "unknown"


def _numeric_group_registry(scoped: bool) -> FixRegistry:
    """A venue's counted group under tags 6000..6002, stamped as the venue's."""
    registry = FixRegistry.from_fields(
        [_field_catalog("MsgType", 35), _field_catalog("Symbol", 55), _field_catalog("CheckSum", 10)]
    )

    def venue_field(name: str, tag: int, dtype: str) -> Field:
        value = Field(name, dtype)
        value.fix.tag = tag
        value.fix.branches = ["alpha"]
        return value

    counter = venue_field("NoAlphaRows", 6000, "int32")
    member = venue_field("AlphaID", 6001, "int32")
    tail = venue_field("AlphaValue", 6002, "int32")
    registry.add_fields([counter, member, tail])
    component = Field("AlphaRowsEntry", DataType.from_fields([member]), nullable=False)
    component.fix.branches = ["alpha"]
    registry.insert(component)
    held = yggdryl.serie("AlphaRows", component)
    held.fix.branches = ["alpha"]
    held.fix.counter = 6000
    held.fix.component = component.name
    registry.insert(held)
    if scoped:
        message = _message("AlphaMessage", "X", [held, tail])
        message.fix.branches = ["alpha"]
        registry.insert(message)
    return registry


@pytest.mark.parametrize("scoped", [False, True])
def test_numeric_groups_resolve_through_the_one_namespace(scoped: bool) -> None:
    """Membership is provenance: a venue's counted group needs no pin."""
    registry = _numeric_group_registry(scoped)
    assert registry.dialects() == ["alpha"]
    assert registry.field_by_counter(6000).name == "AlphaRows"
    assert registry.field_by_tag(6001).fix.has_branch("alpha")

    codec = FixCodec(registry)
    wire = b"35=X|6000=1|6001=42|6002=7|55=AAPL|10=0|"
    message = codec.parse_fix_line(wire)
    assert len(message.by_name("AlphaRows").as_py()) == 1
    assert message.get_by_name("NoAlphaRows") is None
    assert message.by_path("AlphaRows[0].AlphaID").as_py() == 42
    assert message.by_name("AlphaValue").as_py() == 7
    assert message.by_tag(55).as_py() == "AAPL"
    # The header the parse read the frame at leads the wire, and the row
    # follows it as the line stated it.
    assert message.into_bytes(ord("|")).endswith(b"6000=1|6001=42|6002=7|55=AAPL|10=0|")
    # A message root the codec builds is not a dictionary member.
    assert message.field.fix.branches == []

    # A counter no group declares is kept under its own spelling.
    loose = codec.parse_fix_line(b"6100=1|6101=42|55=AAPL|")
    assert loose.get_by_name("AlphaRows") is None
    assert loose.by_name("6100").as_py() == "1"
    assert loose.by_name("6101").as_py() == "42"


def test_the_crate_map_group_is_a_group_a_message_may_reference(tmp_path: Any) -> None:
    registry = FixRegistry()
    mapping = registry.field_by_counter(METADATA_TAG)
    assert mapping.name == "metadata"
    mapping.fix.group = "metadata"
    registry.insert(_message("bridged", "ID", [mapping]))

    snapshot = json.loads(registry.into_json())
    assert "metadata" in [group["name"] for group in snapshot["groups"]]
    assert FixRegistry.from_json(registry.into_json()) == registry

    location = tmp_path / "metadata-reference"
    registry.write_into(location)
    assert (location / "groups" / "metadata.json").exists()
    assert FixRegistry.from_handle(location) == registry

    # The group's number before A11 renumbered the crate's tags (65051)
    # reaches nothing: no alias keeps an old number.
    with pytest.raises(KeyError):
        registry.field_by_counter(65051)

REPO = pathlib.Path(__file__).resolve().parent.parent.parent
SEED = REPO / "config" / "fix"

# What the crate itself adds beside the specification: 49 definitions in tag
# order from 65001 - the element's identities, the event's clocks, the market
# and operation facts in the fixed row's band order, then the message's own
# and the capture's own - 48 scalar facts and one Map group.
# ISIN, Forex, Bloomberg, FIGI and MIC are crate columns, views of the
# message's security identifiers; a bridge's originating plugin and
# conversation are columns too, while the identifiers it states under its own
# keys are inferred from them and stay content; the option's strike price is
# the dictionary's StrikePrice(202); CFI remains FIX's standard tag 461, as
# do prices and quantities.
CRATED = 49
# What ``FixRegistry()`` holds: the crate fields a message states - a derived
# column is the fixed row's, never filed - SendingTime (52) and TransactTime
# (60), and the Map group. ``len`` counts groups; iteration walks the 32
# scalars alone.
SEEDED = 33
SEEDED_SCALARS = 32

# The one intake clock undated test bytes take, so a parse repeats; replay
# never consults now.
CLOCK_NS = 1_704_190_530_000_000_000
CLOCK = DataType('datetime64(ns,"UTC")').scalar(CLOCK_NS)
CLOCK_INSTANT = dt.datetime(2024, 1, 2, 10, 15, 30, tzinfo=dt.timezone.utc)

# The crate's own tags the message answers as typed facts, renumbered
# contiguously from 65001 in the fixed row's band order (A11).
UNIX_TAG = 65007
EXECUNIX_TAG = 65023
RECDUNIX_TAG = 65009
CREAUNIX_TAG = 65008
PREVUNIX_TAG = 65011
SNAPUNIX_TAG = 65012
EXPRUNIX_TAG = 65010
CURRUUID_TAG = 65001
CROSSUUID_TAG = 65002
CROSSCODE_TAG = 65003
HASHCODE_TAG = 65004
CROSSHASHCODE_TAG = 65005
PREVUUID_TAG = 65013
SEQNUM_TAG = 65014
SRCUUIDS_TAG = 65006
MSGCAT_TAG = 65016
MSGPLUGINID_TAG = 65040
MSGCTXID_TAG = 65042
MSGSESSIONID_TAG = 65043
MSGSESSEVENTID_TAG = 65044
ISINCODE_TAG = 65021
FOREXCODE_TAG = 65046
BLOOMBERGCODE_TAG = 65047
FIGICODE_TAG = 65048
MICCODE_TAG = 65022
STATE_TAG = 65015
METADATA_TAG = 65035
SOURCEURL_TAG = 65049
MSGORIGINATOR_TAG = 65041
CONVERSATIONID_TAG = 65045


def _fixed(registry: FixRegistry, **pins: Any) -> FixCodec:
    """A codec whose undated messages all take ``CLOCK`` as their SendingTime."""
    return FixCodec(registry, default_sending_time=CLOCK, **pins)


# One Ullink CBlock in the shape a production file has: a vocabulary of a
# specification tag and a venue one, and a grammar binding whose root the
# registry form answers and the vocabulary form drops.
CBLOCK = """<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration version="1.2" fix-version="4.4" targetcompid="BLPFIX" sendercompid="OURDESK">
	<vocabulary>
		<vocabulary-tag name="55" alt="Symbol" type="string">
			<description>Ticker symbol.</description>
		</vocabulary-tag>
		<vocabulary-tag name="10001" alt="ExcludedDealers" type="string" />
	</vocabulary>
	<grammar-binding type="7">
		<grammar>
			<tag-constraint name="55" part="body" required="true" />
		</grammar>
	</grammar-binding>
</cplugin-configuration>
"""


def _field(
    name: str,
    dtype: str,
    tag: int,
    *,
    branches: Iterable[str] = (),
    tags: Iterable[int] = (),
    names: Iterable[str] = (),
    description: str | None = None,
    nullable: bool = True,
) -> Field:
    """One FIX field, written through the protocol view alone."""
    field = Field(name, dtype, nullable=nullable)
    field.fix.tag = tag
    if branches:
        field.fix.branches = branches
    if tags:
        field.fix.tags = tags
    if names:
        field.fix.names = names
    if description is not None:
        field.fix.description = description
    return field


def _declared(registry: FixRegistry) -> list[str]:
    """The scalar names one source added, past what every dictionary holds.

    A `CBlock` reads in as a whole dictionary now, so what the file declared
    is what the crate's own definitions and the two seeded clocks are not.
    Sorted, because a registry answers in identity order and a file declares
    in its own.
    """
    seeded = {field.name for field in FixRegistry()}
    return sorted(field.name for field in registry if field.name not in seeded)


@pytest.fixture(scope="module")
def _seed_catalog() -> FixRegistry:
    """The dictionary the repository tracks at ``config/fix``."""
    return FixRegistry.from_handle(SEED)


@pytest.fixture
def seed(_seed_catalog: FixRegistry) -> FixRegistry:
    return copy.copy(_seed_catalog)


def test_protocol_view_carries_the_typed_fix_vocabulary() -> None:
    field = Field("OrderQty", "decimal128(20, 8)")
    field.fix.tag = 38
    field.fix.tags = [1088]
    field.fix.names = ["Qty", "Quantity"]
    field.fix.description = "Quantity ordered."

    assert field.fix.tag == 38
    assert field.fix.tags == [1088]
    assert field.fix.names == ["Qty", "Quantity"]
    assert field.fix.description == "Quantity ordered."
    # Ordinary namespaced text, in the one metadata map: a list is the
    # compact JSON array it is.
    assert field.metadata["FIX:names"] == '["Qty","Quantity"]'
    assert field.metadata["FIX:tags"] == "[1088]"
    assert field.fix["tag"] == "38"
    # Three, not four: a description is a fact about the column rather than a
    # FIX fact, so it lives on the generic key every catalog reads.
    assert field.metadata["description"] == "Quantity ordered."
    assert "FIX:description" not in field.metadata
    assert len(field.fix) == 3

    # An empty list removes the property; `del` removes any of them.
    field.fix.tags = []
    assert field.fix.tags == []
    assert "tags" not in field.fix
    field.fix.names = ()
    assert field.fix.names == []
    del field.fix["tag"]
    assert field.fix.tag is None

    absent = Field("Symbol", "utf8")
    assert absent.fix.tag is None
    assert absent.fix.tags == []
    assert absent.fix.names == []
    assert absent.fix.description is None


def test_typed_vocabulary_is_only_on_the_fix_view() -> None:
    field = Field("Symbol", "utf8")
    field.fix.tag = 55

    for view, scheme in ((field.http, "http"), (field.iceberg, "iceberg")):
        with pytest.raises(TypeError, match=scheme):
            view.tag
        with pytest.raises(TypeError, match=scheme):
            view.names
        with pytest.raises(TypeError, match=scheme):
            view.branches
        with pytest.raises(TypeError, match=scheme):
            view.id
        with pytest.raises(TypeError, match=scheme):
            view.tag = 55
        with pytest.raises(TypeError, match=scheme):
            view.add_branch("cme")
    # The mapping protocol still works on every view, including this one.
    assert field.protocol("fix")["tag"] == "55"


def test_a_field_carries_no_derivation_rule() -> None:
    """The fills are the crate's native code: no field states a rule (A7)."""
    field = Field("leavesqty", "float64")
    field.fix.tag = 151
    assert not hasattr(field.fix, "derivation")


def test_direction_rules_cross_as_a_list() -> None:
    """One record per code of the set, the patterns decoded, on tag 385."""
    field = Field("MsgDirection", "utf8")
    field.fix.tag = 385
    assert field.fix.directions == []

    rules = [
        {"code": "S", "patterns": ["(?i)^TX\\b"]},
        {"code": "R", "patterns": ["(?i)^RX\\b"]},
    ]
    field.fix.directions = rules
    assert field.fix.directions == rules
    assert json.loads(field.metadata["FIX:directions"]) == rules

    # A codec compiles the rules of the dictionary it is built over, once,
    # and the line door fills the header's direction from them.
    registry = FixRegistry()
    registry.insert(field)
    codec = _fixed(registry)
    assert next(codec.parse_line(b"TX 8=FIX.4.4|35=D|10=0|")).header().msgdirection == "S"
    assert next(codec.parse_line(b"RX 8=FIX.4.4|35=D|10=0|")).header().msgdirection == "R"
    assert next(codec.parse_line(b"sending >> 8=FIX.4.4|35=D|10=0|")).header().msgdirection is None

    # A pattern the regex crate refuses is refused whole, the field unchanged.
    with pytest.raises(ValueError, match="valid byte regex"):
        field.fix.directions = [{"code": "S", "patterns": ["("]}]
    assert field.fix.directions == rules
    field.fix.directions = []
    assert "FIX:directions" not in field.metadata


def test_identifier_map_keys_cross_as_a_list(seed: FixRegistry) -> None:
    """One record per key a field names a message by, and the registry's table."""
    field = Field("OrderID", "utf8")
    field.fix.tag = 37
    assert field.fix.idmap == []

    sources = [{"map": "identifiers", "key": "orderid", "follow": True, "role": None}]
    field.fix.idmap = [{"map": "identifiers", "key": "orderid", "follow": True}]
    assert field.fix.idmap == sources
    assert field.metadata["FIX:idmap"] == '[{"map":"identifiers","key":"orderid","follow":true}]'
    # A key is read as the identifier type word it folds to, and stored as it.
    field.fix.idmap = [{"map": "identifiers", "key": "OrderID", "follow": True}]
    assert field.fix.idmap == sources

    # A follow flag belongs to an alternate identifier alone, and a refused
    # list leaves the field as it was.
    # `identifiers` is the one map left since A1 deleted the accounts and users.
    with pytest.raises(ValueError, match='expected "map" to be identifiers'):
        field.fix.idmap = [{"map": "accountids", "key": "orderid", "follow": True}]
    with pytest.raises(ValueError, match="identifier type"):
        field.fix.idmap = [{"map": "identifiers", "key": "order id!"}]
    assert field.fix.idmap == sources
    # A stored document states the folded word itself: an upper-case key, or
    # an alias of a type, is refused where it is read.
    other = Field("OrderID", "utf8")
    other.fix.tag = 37
    for key in ("ORDERID", "isinnumber"):
        other.set_metadata({"FIX:idmap": '[{"map":"identifiers","key":"%s"}]' % key})
        with pytest.raises(ValueError, match='expected "key"'):
            other.fix.idmap
    field.fix.idmap = []
    assert "FIX:idmap" not in field.metadata

    # The committed dictionary compiles every field's once: OrderID follows,
    # and PartyID feeds no FIX:idmap entry - a party is an account.
    table = seed.idmap_sources()
    assert {"tag": 37, "map": "identifiers", "key": "orderid", "follow": True, "role": None} in table
    assert [row for row in table if row["tag"] == 448] == []
    # The parents an identifier's chain gives it are stated by the field that
    # states it: ClOrdID(11)'s one parent is the type OrigClOrdID(41) names.
    assert seed.get_field_by_tag(11).fix.parents == ["origclordid"]
    assert seed.get_field_by_tag(11).metadata["FIX:parents"] == '["origclordid"]'
    assert {"base": "clordid", "parents": ["origclordid"]} in seed.parent_sources()

    # A bridge's own keys are no field of the dictionary: each is read for the
    # identifier name it ends with, its source the rest of the key, and lands
    # in the set its type belongs to - an account or a user a party, anything
    # else an identifier - beside the identifiers the fields state.
    codec = _fixed(seed)
    message = next(
        codec.parse_line(
            b"8=FIX.4.4|35=8|17=E1|37=O1|VENUE_Account=ACC1|VENUE_UserID=trader1|"
            b"FIRM.ParentOrderID=P1|SHOP_ClOrdID=U1|10=0|"
        )
    )
    # A parent stated without its base is what it came from: `firm:orderid`
    # is filled from `firm:parentorderid`, under the parent's own source; a
    # named source fills its type's base key where the wire stated none.
    assert [str(id) for id in message.identifiers] == [
        "clordid=U1",
        "execid=E1",
        "firm:orderid=P1",
        "firm:parentorderid=P1",
        "orderid=O1",
        "parentorderid=P1",
        "shop:clordid=U1",
    ]
    assert message.identifiers.get_from("firm:parentorderid") == "P1"
    assert [str(id) for id in message.partyids] == [
        "account=ACC1",
        "userid=trader1",
        "venue:account=ACC1",
        "venue:userid=trader1",
    ]
    assert not message.securityids


def test_a_bridge_line_names_the_plugin_and_conversation_on_the_capture(
    seed: FixRegistry,
) -> None:
    """Provenance the capture holds and the fixed row states, never content."""
    codec = _fixed(seed)
    frame = b"8=FIX.4.4|35=8|17=E1|37=O1|10=0|"
    bare = next(codec.parse_line(frame))
    named = next(
        codec.parse_line(
            b"Execution report from PLUGIN_A type trade {conversationId: c-1} " + frame
        )
    )
    assert named.capture().msgoriginator == "PLUGIN_A"
    assert named.capture().conversationid == "c-1"
    assert named.by_tag(MSGORIGINATOR_TAG).as_py() == "PLUGIN_A"
    assert named.by_tag(CONVERSATIONID_TAG).as_py() == "c-1"
    assert bare.capture().msgoriginator is None
    assert bare.capture().conversationid is None
    assert named.into_bytes(124) == bare.into_bytes(124)
    assert named.currhashcode == bare.currhashcode
    assert named.capture() != bare.capture()

    schema = fix_schema(seed)
    row = named.into_row(schema)
    assert row.as_py()[schema.index_of("msgoriginator")] == "PLUGIN_A"
    assert row.as_py()[schema.index_of("conversationid")] == "c-1"
    assert FixMsg.from_row(schema, row, seed).capture() == named.capture()


def test_tag_and_identifier_keys_refuse_bool_and_narrowing(seed: FixRegistry) -> None:
    field = Field("Symbol", "utf8")
    with pytest.raises(TypeError, match="not bool"):
        field.fix.tag = True
    with pytest.raises(OverflowError):
        field.fix.tag = 2**31

    with pytest.raises(TypeError, match="not bool"):
        seed.get_field_by_tag(True)
    with pytest.raises(TypeError, match="not bool"):
        seed.field_by_id(True)
    with pytest.raises(OverflowError):
        seed.get_field_by_id(2**31)
    # A name and a path take one argument: there is no dictionary to pin.
    with pytest.raises(TypeError):
        seed.get_field_by_name("Symbol", "")  # type: ignore[call-arg]


def test_id_is_the_tag_under_the_name_and_never_stored() -> None:
    field = _field("Symbol", "utf8", 55)
    assert field.fix.id == _field("symbol", "utf8", 55).fix.id
    assert field.fix.id != _field("Symbol", "utf8", 56).fix.id
    assert "FIX:id" not in field.metadata
    assert Field("Symbol", "utf8").fix.id is None


def test_registry_resolves_every_key_the_way_the_core_does(seed: FixRegistry) -> None:
    # The store's fields, its components and its groups, and the crate's own
    # beside them: a store writes those too, and a loaded dictionary holds
    # the crate's definition rather than the document's.
    assert bool(seed)
    assert len(seed) > len(list(seed)), "len counts the definitions, iteration walks scalars"

    assert seed.field_by_tag(55).name == "symbol"
    assert seed.get_field_by_tag(55) == seed.field_by_tag(55)
    symbol_id = seed.field_by_tag(55).fix.id
    assert symbol_id is not None
    assert seed.field_by_id(symbol_id).name == "symbol"
    # Only the crate's own `state` is typed as the ranked lifecycle
    # vocabulary; the protocol's own code sets keep their base type, and
    # tag 54 is the one FIX field typed as the value it holds.
    assert seed.field_by_tag(54).dtype == DataType("side")
    assert seed.field_by_tag(15).dtype == DataType("ccy")
    assert seed.field_by_tag(39).dtype == DataType("utf8")
    assert seed.field_by_tag(40).dtype == DataType("utf8")

    # An alternate tag is not an identity: the one id is the canonical tag's.
    alternate = _field("exectype", "utf8", 150)
    alternate.fix.tags = [20]
    aliased = FixRegistry.from_fields([alternate])
    assert aliased.field_by_tag(20).name == "exectype"
    assert aliased.get_field_by_id(_field("exectype", "utf8", 20).fix.id) is None

    # A name answers the canonical spelling whatever case it was asked in.
    assert seed.field_by_name("SYMBOL").name == "symbol"
    named = FixRegistry.from_fields([_field("symbol", "utf8", 55, names=["ticker"])])
    assert named.field_by_name("ticker").name == "symbol"

    # One namespace: a component and a group answer the name doors too, and
    # a group answers the counter its occurrences are counted by.
    assert seed.field_by_name("parties").fix.counter == 453
    assert seed.field_by_name("party").dtype.is_nested
    assert seed.field_by_counter(453).name == "parties"
    assert seed.get_field_by_counter(9999) is None
    assert seed.field_by_tag(453).dtype == DataType("int32")

    # A path reaches a repeating group and one of its members.
    assert seed.field_by_path("NoPartyIDs").fix.tag == 453
    assert seed.field_by_path("parties.partyid").fix.tag == 448
    assert seed.get_field_by_path("parties.partyid.partyid") is None

    # The generic pair answers exactly what the specialized one does.
    for key in (55, "Symbol", "nopartyids", "parties.partyid"):
        assert seed.get_field(key) == seed[key]
        assert seed.field(key) == seed[key]
        assert key in seed
    assert 9999 not in seed
    assert seed.get_field(9999) is None
    assert seed.get(9999, "fallback") == "fallback"


def test_registry_absence_is_a_key_error_carrying_the_core_message(seed: FixRegistry) -> None:
    with pytest.raises(KeyError) as by_tag:
        seed.field_by_tag(9999)
    assert by_tag.value.args[0] == 'expected a fix field at "tag 9999", got nothing'

    absent_id = _field("TradeID", "utf8", 5001).fix.id
    assert absent_id is not None
    with pytest.raises(KeyError):
        seed.field_by_id(absent_id)
    with pytest.raises(KeyError):
        seed.field_by_name("Nope")
    with pytest.raises(KeyError):
        seed.field_by_counter(9999)
    with pytest.raises(KeyError):
        seed.msgtype("nope")


def test_registry_iterates_lazily_in_ascending_identifier_order() -> None:
    registry = FixRegistry.from_fields(
        [
            _field("symbol", "utf8", 55),
            _field("TradeID", "utf8", 5001, branches=["cme"]),
            _field("Price", "decimal128(20, 8)", 44),
            _field("Account", "utf8", 1),
        ]
    )
    # Tag-major, then by identifier. The seeded SendingTime (52) and
    # TransactTime (60) walk among the dictionary's own tags, and the crate's
    # own fields close every walk, above any tag a test claims.
    tags = [field.fix.tag for field in registry]
    assert tags[:5] == [1, 44, 52, 55, 60]
    assert tags == sorted(tags)
    assert len(tags) == 4 + SEEDED_SCALARS

    walk = iter(registry)
    assert next(walk).name == "Account"
    # An unfinished walk shares the registry, so a mutation refuses until it
    # is dropped rather than moving the fields under the cursor.
    with pytest.raises(ValueError, match="shared with a message"):
        registry.remove(1)
    del walk
    assert registry.remove(1) is not None


def test_a_new_registry_holds_the_crate_and_the_two_seeded_clocks() -> None:
    registry = FixRegistry()
    assert len(registry) == SEEDED
    assert len(list(registry)) == SEEDED_SCALARS
    assert registry.dialects() == []
    assert repr(registry) == f"FixRegistry({SEEDED} fields)"

    # The seeds are ordinary definitions, typed as the clock the event reads.
    for tag, name, display in ((52, "sendingtime", "SendingTime"), (60, "transacttime", "TransactTime")):
        seeded = registry.field_by_tag(tag)
        assert (seeded.name, seeded.display) == (name, display)
        assert seeded.dtype == DataType('datetime64(ns,"UTC")')

    # The crate's Map group is a group: filed by its shape, reached by name
    # and by its own tag as a counter, never by the scalar tag door.
    for name, tag in (("metadata", METADATA_TAG),):
        assert registry.field_by_name(name).dtype.is_nested, name
        assert registry.field_by_counter(tag).name == name, name
        assert registry.get_field_by_tag(tag) is None, name

    # Retired crate spellings reach nothing. Prices and quantities remain
    # standard FIX fields; state, categories and normalized identifiers have
    # their one current crate column.
    for retired in (
        "updatedat",
        "createdat",
        "msghash",
        "msgphash",
        "altids",
        "code",
        "version",
        "px",
        "qty",
        "tradable",
        "symbolticker",
        "cusipcode",
        "sedolcode",
        # The bridge's own keys and the strike price: the first arrive as
        # unmapped entries read for the identifier they name, the last is
        # `StrikePrice(202)`.
        "strikepx",
        "omsdealeraccount",
        "omsuserid",
        "parentorderid",
        "parentclordid",
        "omsdealerparentorderid",
        "exchangeclientorderid",
        "transversalkey",
        "ultraderclordid",
        "omsinstrumentid",
        "ullinkinstrumentid",
    ):
        assert registry.get_field_by_name(retired) is None, retired


def test_the_crate_fields_declare_their_own_protocols() -> None:
    """Each column says what it derives from and what it holds, on the field."""
    fields = {field.name: field for field in fix_crate_fields()}
    assert len(fields) == CRATED
    # In tag order, one contiguous block from 65001 in the fixed row's band
    # order: 48 scalar identity, clock, category, market, operation,
    # normalized-identifier and bridge-provenance facts plus the one Map. ISIN,
    # Forex, Bloomberg, FIGI and MIC are crate columns; CFI keeps FIX's
    # standard tag 461. Price, quantity and the option's strike price remain
    # their standard FIX fields.
    assert list(fields) == [
        "curruuid",
        "crossuuid",
        "crosscode",
        "currhashcode",
        "crosshashcode",
        "srcuuids",
        "currunix",
        "creaunix",
        "recdunix",
        "exprunix",
        "prevunix",
        "snapunix",
        "prevuuid",
        "seqnum",
        "state",
        "marketdatakind",
        "marketdatatype",
        "hiddenqty",
        "unit",
        "securityids",
        "isincode",
        "miccode",
        "execunix",
        "prevpx",
        "prevqty",
        "spotrate",
        "forwardpoints",
        "bidqty",
        "bidccy",
        "askpx",
        "askqty",
        "askccy",
        "fxrates",
        "ticker",
        "metadata",
        "ordqty",
        "tradable",
        "identifiers",
        "partyids",
        "msgpluginid",
        "msgoriginator",
        "msgctxid",
        "msgsessionid",
        "msgsesseventid",
        "conversationid",
        "forexcode",
        "bloombergcode",
        "figicode",
        "sourceurl",
    ]
    tags = [field.fix.tag for field in fields.values()]
    assert tags == sorted(tags)
    assert tags[0] == CURRUUID_TAG and tags[-1] == SOURCEURL_TAG
    assert tags == list(range(65001, 65050))
    assert all(field.fix.branches == [] for field in fields.values())
    assert all(field.description is not None for field in fields.values())

    # The columns every message settles are non-null; every other one is
    # nullable, because a message that carried nothing there answers null.
    assert [name for name, field in fields.items() if not field.nullable] == [
        "curruuid",
        "crossuuid",
        "currhashcode",
        "crosshashcode",
        "currunix",
        "creaunix",
    ]

    # The clocks are instants in UTC, to the nanosecond; the identities are
    # what a lake reads as a UUID and a 64-bit integer; the facts a row
    # derives are typed as the thing they hold.
    for name in (
        "currunix",
        "prevunix",
        "creaunix",
        "snapunix",
        "exprunix",
        "execunix",
        "recdunix",
    ):
        assert fields[name].dtype == DataType('datetime64(ns,"UTC")'), name
    assert fields["state"].dtype == DataType("state")
    assert fields["marketdatakind"].dtype == DataType("marketdatakind")
    assert fields["marketdatatype"].dtype == DataType("marketdatatype")
    assert fields["forexcode"].dtype == DataType("forex")
    assert "strikepx" not in fields and "strikeprice" not in fields, "StrikePrice(202) is the dictionary's"
    for name, dtype in (
        ("isincode", "isin"),
        ("bloombergcode", "bbg"),
        ("miccode", "mic"),
    ):
        assert fields[name].dtype == DataType(dtype), name
    for name in ("curruuid", "crossuuid", "prevuuid"):
        assert fields[name].dtype == DataType("uuid"), name
    for name in ("currhashcode", "crosshashcode", "seqnum"):
        assert fields[name].dtype == DataType("uint64"), name
    assert fields["sourceurl"].dtype == DataType("url")
    assert fields["crosscode"].dtype == DataType("utf8")
    # The session event a bridge delivered the message as is the text its
    # four values join to.
    assert fields["msgsesseventid"].dtype == DataType("utf8")
    assert "nofixentries" not in fields
    # How a layout is cut is the target's: no partition column.
    assert all(not field.is_partition for field in fields.values())
    # The Map carries what a bridge said under its own namespaces.
    for name in ("metadata",):
        assert fields[name].into_arrow().type.equals(
            pa.map_(pa.string(), pa.string(), keys_sorted=True)
        ), name


def test_registry_takes_every_storage_location(tmp_path: pathlib.Path) -> None:
    # Every spelling of one folder reads the one dictionary it holds: a small
    # one written for the purpose, since the spelling is the claim.
    written = FixRegistry()
    written.insert(_field("Venue", "utf8", 39999))
    absolute = (tmp_path / "dictionary").resolve()
    written.write_into(absolute)
    for location in (absolute, str(absolute), absolute.as_uri(), Url(absolute), IOBase(absolute)):
        assert FixRegistry.from_handle(location) == written

    # A folder that is not there loads as a new registry and is not created.
    missing = tmp_path / "missing"
    assert FixRegistry.from_handle(missing) == FixRegistry()
    assert not missing.exists()


def test_a_malformed_native_field_shard_is_located(tmp_path: pathlib.Path) -> None:
    root = tmp_path / "invalid"
    (root / "fields").mkdir(parents=True)
    (root / "fields" / "000000000.json").write_text("not JSON", encoding="utf-8")
    with pytest.raises(ValueError, match="000000000.json"):
        FixRegistry.from_handle(root)


def test_a_store_writes_the_whole_row_and_reads_its_own_dump_back(tmp_path: pathlib.Path) -> None:
    """Shards are nine digits wide, and the crate dumps its own row beside them."""
    registry = FixRegistry.from_fields(
        [_field("MsgType", "utf8", 35), _field("TradeID", "utf8", 5001, branches=["cme"])]
    )
    root = tmp_path / "dictionary"
    registry.write_into(root)

    # One shard arithmetic for every field, nine digits with leading zeros:
    # tag 35 lands in shard 0, tag 5001 in shard 50, the crate's own in 650.
    assert sorted(path.relative_to(root).as_posix() for path in root.rglob("*.json")) == [
        "codesets/marketdatatypecodeset.json",
        "codesets/msgcatcodeset.json",
        "codesets/statecodeset.json",
        "components/fixmsg.json",
        "fields/000000000.json",
        "fields/000000050.json",
        "fields/000000650.json",
        "groups/metadata.json",
    ]
    assert not (root / "branches.json").exists()

    reloaded = FixRegistry.from_handle(root)
    assert reloaded == registry
    assert reloaded.field_by_name("tradeid").fix.branches == ["cme"]
    assert reloaded.dialects() == ["cme"]


def test_registry_insert_update_and_remove_over_fields() -> None:
    registry = FixRegistry.from_fields(
        [
            _field("symbol", "utf8", 55, names=["Ticker"]),
            _field("Price", "decimal128(20, 8)", 44, names=["Px"]),
        ]
    )
    assert len(registry) == 2 + SEEDED
    assert registry.insert(_field("Side", "utf8", 54)) is None
    assert registry.field_by_tag(54).name == "Side"

    # A key another field holds is refused, naming both; nothing changes.
    with pytest.raises(ValueError, match="held by symbol"):
        registry.insert(_field("SymbolSfx", "utf8", 65, names=["ticker"]))
    assert len(registry) == 3 + SEEDED

    # A merge concatenates the two list properties, incoming first.
    registry.update(_field("SYMBOL", "utf8", 55, tags=[65], names=["Sym"]))
    merged = registry.field_by_tag(65)
    assert merged.name == "symbol"
    assert merged.fix.names == ["Sym", "Ticker"]
    with pytest.raises(ValueError):
        registry.update(_field("symbol", "large_utf8", 55))
    assert registry.field_by_tag(55).dtype == DataType("utf8")

    removed = registry.remove("sym")
    assert removed is not None and removed.name == "symbol"
    assert registry.remove(9999) is None

    # `remove` reads an int as a tag, so a field sharing its tag with another
    # leaves only through the identifier `remove_by_id` reads as one.
    registry.insert(_field("Held", "utf8", 77, names=["Beside"]))
    registry.insert(_field("Beside", "utf8", 78, tags=[77]))
    beside = registry.field_by_name("Beside")
    assert registry.remove_by_id(beside.fix.id) is not None
    assert registry.field_by_tag(77).name == "Held"
    assert registry.remove_by_id(beside.fix.id) is None

    # A field with no tag cannot enter at all.
    with pytest.raises(ValueError, match="FIX:tag"):
        registry.insert(Field("Untagged", "utf8"))


def test_registry_add_field_answers_whether_the_field_arrived_or_folded() -> None:
    """`add_field` is the one-field verb `add_fields` folds through."""
    registry = FixRegistry.from_fields(
        [
            _field("Symbol", "utf8", 55, tags=[65], names=["Ticker"], description="stored"),
            _field("Price", "float64", 44),
        ]
    )

    incoming = _field(
        "symbol", "utf8", 9001, tags=[66], names=["Sym", "TICKER"], description="incoming"
    )
    assert registry.add_field(incoming) is False
    stored = registry.field_by_tag(55)
    assert stored.name == "Symbol"
    assert stored.fix.tags == [65, 66, 9001]
    assert stored.fix.names == ["Ticker", "Sym"]
    assert stored.description == "incoming"
    for key in (9001, 66, 65, "sym", "TICKER"):
        assert registry.field(key).name == "Symbol", key

    before = registry.into_json()
    assert registry.add_field(incoming) is False
    assert registry.into_json() == before
    assert registry.add_field(_field("Text", "utf8", 58)) is True

    # A datatype that disagrees with the stored field is refused atomically.
    before = registry.into_json()
    with pytest.raises(ValueError, match="utf8"):
        registry.add_field(_field("SYMBOL", "int32", 9002))
    assert registry.into_json() == before
    # One of this crate's own is every dictionary's already.
    assert registry.add_field(fix_crate_fields()[0]) is False
    assert registry.into_json() == before


def _rejection(declared: str) -> str:
    """One CBlock declaring tag 532 as ``declared`` and binding it in ``r``."""
    return f"""<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
  <vocabulary>
    <vocabulary-tag name="35" alt="MsgType" type="string" />
    <vocabulary-tag name="532" alt="MassCancelRejectReason" type="{declared}" />
    <vocabulary-tag name="58" alt="Text" type="string" />
  </vocabulary>
  <grammar-binding type="r"><grammar>
    <tag-constraint name="532" />
    <tag-constraint name="58" />
  </grammar></grammar-binding>
</cplugin-configuration>"""


def test_a_glob_of_cblocks_passes_over_a_tag_typed_two_ways(tmp_path: pathlib.Path) -> None:
    folder = tmp_path / "cblocks"
    folder.mkdir()
    (folder / "bloomberg_fix44_dropcopy.cfb").write_text(_rejection("integer"))
    (folder / "axessiq_fix44.cfb").write_text(_rejection("boolean"))
    (folder / "tradeweb_fix44.cfb").write_text(_rejection("string"))
    (folder / "broken_fix44.cfb").write_text("<cplugin-configuration><vocabulary>")
    (folder / "notes.txt").write_text("not a dictionary")
    # A folder holds the .cfb files directly inside it, and a glob what it
    # matches: both fold the same files into the same dictionary.
    globbed = FixRegistry()
    globbed.add_cfb_files(folder / "*fix44*.cfb")
    dictionary = FixRegistry()
    report = dictionary.add_cfb_files(folder)
    assert dictionary.into_json() == globbed.into_json()

    # Every file folds but the one that is no CBlock, which is left out and
    # named; the first in URL order is held. An integer contradicts a flag
    # and is named; text is every FIX datatype on the wire, so it restates
    # the flag and folds under it.
    assert report["sources"] == 3
    [failed] = report["failed"]
    assert failed["source"].endswith("broken_fix44.cfb")
    assert "vocabulary" in failed["reason"]
    assert report["restated"] == 1
    [dropped] = report["dropped"]
    assert dropped["source"].endswith("bloomberg_fix44_dropcopy.cfb")
    assert dropped["incoming"].fix.tag == 532
    assert dropped["incoming"].fix.branches == ["bloomberg_fix44_dropcopy"]
    assert "boolean" in dropped["reason"] and "int32" in dropped["reason"]
    held = dictionary.field_by_tag(532)
    assert held.dtype == DataType("boolean")
    assert held.fix.branches == ["axessiq_fix44", "tradeweb_fix44"]
    assert dictionary.get_msgtype("r") is not None


def test_a_cblock_at_another_precision_folds_under_the_stored_datatype(
    tmp_path: pathlib.Path,
) -> None:
    # A stem with a space crosses as a percent escape and names the dialect
    # decoded.
    path = tmp_path / "Morgan Stanley.cfb"
    path.write_text(
        """<?xml version="1.0"?>
<cplugin-configuration fix-version="4.4">
  <vocabulary>
    <vocabulary-tag name="44" alt="Price" type="float" />
  </vocabulary>
</cplugin-configuration>""",
        encoding="utf-8",
    )
    dictionary = FixRegistry.from_fields([_field("Price", "decimal128(38, 18)", 44)])
    report = dictionary.add_cfb_file(path)

    # A CBlock's `float` names a family rather than a width: the stored
    # decimal is the finer statement of one number, so the tag merges under
    # it and is counted as restated rather than passed over.
    assert report["restated"] == 1
    assert report["added"] == 0
    assert report["merged"] == 3, "the price and the two seeded clocks"
    assert report["dropped"] == []
    held = dictionary.field_by_tag(44)
    assert held.dtype == DataType("decimal128(38, 18)")
    assert held.fix.branches == ["morgan stanley"]


def test_a_cblock_reads_in_whole_and_stamps_its_dialect(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "bloomberg.cfb"
    path.write_text(CBLOCK, encoding="utf-8")

    # One door: the file read whole is the dictionary its vocabulary states -
    # code sets and all - and the message roots its grammar bindings describe.
    registry, roots = FixRegistry.from_cfb_file(path, dialect="Bloomberg")
    assert _declared(registry) == ["excludeddealers", "symbol"]
    symbol = registry.field_by_tag(55)
    assert symbol.name == "symbol"
    assert symbol.description == "Ticker symbol."
    assert symbol.fix.branches == ["bloomberg"]

    assert [root.name for root in roots] == ["7"]
    message = registry.get_msgtype("7")
    assert message is not None and message.value == "7"
    assert message.field.fix.branches == ["bloomberg"]
    assert registry.dialects() == ["bloomberg"]

    codec = _fixed(registry)
    stated = next(codec.parse_line(b"8=FIX.4.4|35=D|10001=DEALER-A|10=0|"))
    assert stated.by_name("ExcludedDealers").as_py() == "DEALER-A"
    assert stated.by_tag(10001).as_py() == "DEALER-A"
    excluded = next(codec.parse_line(b"8=FIX.4.4|35=D|10001=NONE|10=0|"))
    assert excluded.get_by_name("ExcludedDealers") is None
    assert excluded.get_by_tag(10001) is None
    # Every location the registry takes reads the same file.
    for location in (path, str(path), path.as_uri(), Url(path), IOBase(path)):
        read, _ = FixRegistry.from_cfb_file(location, "bloomberg")
        assert _declared(read) == ["excludeddealers", "symbol"], location

    unstamped, _ = FixRegistry.from_cfb_file(path)
    assert unstamped.dialects() == []

    # The vocabulary folds into a dictionary that already exists.
    dictionary = FixRegistry.from_fields([_field("symbol", "utf8", 55)])
    report = dictionary.add_cfb_file(path, "bloomberg")
    assert report["added"] == 1, "excludeddealers is the one definition nothing held"
    assert report["merged"] >= 1, "symbol is the dictionary's own, stamped by the fold"
    assert dictionary.field_by_tag(55).fix.branches == ["bloomberg"]
    assert dictionary.field_by_name("excludeddealers").fix.branches == ["bloomberg"]

    # A stem or a dialect that carries a comma is refused rather than stored.
    with pytest.raises(ValueError, match="FIX:branches"):
        FixRegistry.from_cfb_file(path, "b,loomberg")


def test_a_cblock_warns_about_the_declaration_it_dropped(
    tmp_path: pathlib.Path, caplog: pytest.LogCaptureFixture
) -> None:
    def warned(path: pathlib.Path) -> list[str]:
        caplog.clear()
        FixRegistry.from_cfb_file(path, "bloomberg")
        return [
            record.getMessage()
            for record in caplog.records
            if record.name.startswith("yggdryl") and record.levelno == logging.WARNING
        ]

    caplog.set_level(logging.WARNING)

    # A tag named by no decimal is no tag: the warning says what was wrong,
    # where the file says it, which dialect's file it is and what the reader
    # did about it.
    broken = tmp_path / "bloomberg.cfb"
    broken.write_text(
        CBLOCK.replace('name="55" alt="Symbol"', 'name="Symbol" alt="Symbol"'),
        encoding="utf-8",
    )
    warnings = warned(broken)
    assert warnings, "the native reader reported nothing"
    [dropped] = [warning for warning in warnings if "the declaration is dropped" in warning]
    assert "bloomberg.cfb [bloomberg] " in dropped
    assert "invalid cfb expression at byte" in dropped
    assert "line " in dropped
    assert 'expected a decimal tag, got "Symbol"' in dropped
    assert dropped.endswith("; the declaration is dropped")

    # The declaration went; every other declaration the file made stands,
    # and the message's constraint on tag 55 still says the tag is on the
    # wire, so it declares the tag as text named by its digits - which the
    # warning over the constraint says too.
    [declared] = [warning for warning in warnings if "named by its digits" in warning]
    assert "tag 55" in declared
    stripped, _ = FixRegistry.from_cfb_file(broken)
    assert _declared(stripped) == ["55", "excludeddealers"]
    assert stripped.field_by_tag(55).name == "55"

    # A type word nothing reads keeps the tag, typed as the text it is on
    # the wire, and says so.
    unknown = tmp_path / "widget.cfb"
    unknown.write_text(
        CBLOCK.replace('name="55" alt="Symbol" type="string"', 'name="55" alt="Symbol" type="widget"'),
        encoding="utf-8",
    )
    [typed] = warned(unknown)
    assert "widget.cfb [bloomberg] " in typed
    assert '"widget"' in typed and "typed string" in typed
    kept, _ = FixRegistry.from_cfb_file(unknown)
    assert _declared(kept) == ["excludeddealers", "symbol"]
    assert kept.field_by_tag(55).dtype == DataType("utf8")

    # A document that stops with an element open is refused, in one sentence
    # the dialect does not change: what the reader stopped on is the file's.
    truncated = tmp_path / "truncated.cfb"
    truncated.write_text(CBLOCK.replace("</vocabulary>", ""), encoding="utf-8")
    with pytest.raises(ValueError) as refused:
        FixRegistry.from_cfb_file(truncated, "bloomberg")
    with pytest.raises(ValueError) as also:
        FixRegistry.from_cfb_file(truncated)
    assert str(also.value) == str(refused.value)
    assert "vocabulary" in str(refused.value)


def test_a_code_set_is_named_once_and_every_field_reads_by_that_name() -> None:
    """The dictionary owns the vocabulary; a field only states its name."""
    registry = FixRegistry()
    registry.set_codeset(
        "sidecodeset",
        [
            {"value": "1", "name": "Buy", "aliases": ["Bought"]},
            {"value": "2", "name": "Sell", "description": "the short side"},
        ],
    )
    assert registry.codeset_names() == [
        "marketdatatypecodeset",
        "msgcatcodeset",
        "sidecodeset",
        "statecodeset",
    ]
    assert registry.codeset("sidecodeset") == [
        {
            "value": "1",
            "name": "Buy",
            "description": None,
            "aliases": ["Bought"],
            "group": None,
        },
        {
            "value": "2",
            "name": "Sell",
            "description": "the short side",
            "aliases": [],
            "group": None,
        },
    ]
    # The name folds the way every name folds, and absence is a `KeyError`.
    assert registry.get_codeset("SideCodeSet") == registry.codeset("sidecodeset")
    assert registry.get_codeset("nosuchcodeset") is None
    with pytest.raises(KeyError, match="nosuchcodeset"):
        registry.codeset("nosuchcodeset")

    # A set is stated before a field points at it: a dictionary refuses a
    # field naming a vocabulary nothing states.
    side = _field("Side", "utf8", 54)
    side.fix.codeset = "nosuchcodeset"
    with pytest.raises(ValueError, match="nosuchcodeset"):
        registry.insert(side)
    assert registry.get_field_by_tag(54) is None

    # The field carries the name and nothing else - `FIX:codeset` is one word.
    side.fix.codeset = "sidecodeset"
    assert side.metadata["FIX:codeset"] == "sidecodeset"
    registry.insert(side)

    # A second field reading by the same set reads the one set: resolution
    # runs through the dictionary, never through a copy on the field.
    other = _field("SideOfMarket", "utf8", 9054)
    other.fix.codeset = "sidecodeset"
    registry.insert(other)
    held = registry.field_by_tag(54)
    assert held.fix.codeset == "sidecodeset"
    assert registry.codeset_of(held) == registry.codeset("sidecodeset")
    assert registry.codeset_of(registry.field_by_tag(9054)) == registry.codeset_of(held)
    assert registry.codeset_of(_field("Symbol", "utf8", 55)) is None

    # Restating the set restates what both fields read by, at once.
    registry.set_codeset("sidecodeset", [{"value": "1", "name": "Bought"}])
    assert [code["name"] for code in registry.codeset("sidecodeset")] == ["Bought"]
    assert registry.codeset_of(registry.field_by_tag(9054)) == registry.codeset(
        "sidecodeset"
    )

    # A set a held field still reads by is not taken away, by either door.
    for taking in (
        lambda: registry.remove_codeset("sidecodeset"),
        lambda: registry.set_codeset("sidecodeset", []),
    ):
        with pytest.raises(ValueError, match="sidecodeset"):
            taking()
    assert registry.codeset_names() == [
        "marketdatatypecodeset",
        "msgcatcodeset",
        "sidecodeset",
        "statecodeset",
    ]

    # Removed once nothing reads by it, answering the members it held. The
    # reference is dropped by `insert`, which replaces: a merge keeps the
    # vocabulary the stored field already read by.
    for tag in (54, 9054):
        moved = registry.field_by_tag(tag)
        moved.fix.codeset = None
        assert moved.fix.codeset is None
        assert "FIX:codeset" not in moved.metadata
        registry.insert(moved)
    taken = registry.remove_codeset("sidecodeset")
    assert taken is not None and [code["value"] for code in taken] == ["1"]
    assert registry.codeset_names() == [
        "marketdatatypecodeset",
        "msgcatcodeset",
        "statecodeset",
    ]
    assert registry.remove_codeset("sidecodeset") is None


def test_a_code_set_merge_keeps_what_the_dictionary_already_held() -> None:
    """A fold widens a vocabulary; it never narrows one."""
    registry = FixRegistry()
    registry.set_codeset(
        "lastqtycodeset",
        [
            {"value": "5", "name": "HeldOnly"},
            {"value": "1", "name": "Shared", "description": "the held reading"},
        ],
    )
    registry.merge_codeset(
        "lastqtycodeset",
        [
            {"value": "9", "name": "FoldedOnly"},
            {"value": "1", "name": "Folded", "description": "the folded reading"},
        ],
    )
    folded = registry.codeset("lastqtycodeset")

    # Keyed by wire value: a value only one side stated is kept, and the
    # reading the dictionary already held wins the one they share.
    assert [code["value"] for code in folded] == ["5", "1", "9"]
    shared = folded[1]
    assert shared["name"] == "Shared"
    assert shared["description"] == "the held reading"
    # What the fold would otherwise have dropped is kept as a spelling.
    assert shared["aliases"] == ["Folded"]

    # Two dictionaries fold the same way, and a field keeps the name it
    # already reads by: the members are the dictionary's to widen.
    source = FixRegistry()
    source.set_codeset("lastqtycodeset", [{"value": "7", "name": "SourceOnly"}])
    lastqty = _field("LastQty", "utf8", 32)
    lastqty.fix.codeset = "lastqtycodeset"
    source.insert(lastqty)
    registry.insert(lastqty)
    registry.merge_with(source)

    assert registry.field_by_tag(32).fix.codeset == "lastqtycodeset"
    assert [code["value"] for code in registry.codeset("lastqtycodeset")] == [
        "5",
        "1",
        "9",
        "7",
    ]

    # A set no dictionary held yet arrives whole.
    registry.merge_codeset("newcodeset", [{"value": "A", "name": "Arrived"}])
    assert registry.codeset_names() == [
        "lastqtycodeset",
        "marketdatatypecodeset",
        "msgcatcodeset",
        "newcodeset",
        "statecodeset",
    ]


def test_registry_mutation_refuses_while_something_shares_it(seed: FixRegistry) -> None:
    root = Field("row", DataType.from_fields([seed.field_by_tag(55)]), nullable=False)
    message = FixMsg(root, {"symbol": "AAPL"}, seed)

    for mutation in (
        lambda: seed.insert(_field("Side", "utf8", 54)),
        lambda: seed.update(_field("symbol", "utf8", 55)),
        lambda: seed.remove(55),
    ):
        with pytest.raises(ValueError, match="shared with a message"):
            mutation()
    assert message.registry == seed

    # The registry a message shares is still readable, and a copy is writable.
    fresh = copy.copy(seed)
    fresh.insert(_field("LocalValue", "utf8", 9999))
    assert fresh.remove(9999) is not None

    # A message type view shares it too.
    registry = FixRegistry.from_fields([_field("MsgType", "utf8", 35)])
    order = Field("NewOrderSingle", DataType.from_fields([]), nullable=False)
    order.fix.msgtype = "D"
    registry.insert(order)
    view = registry.msgtype("D")
    with pytest.raises(ValueError, match="shared"):
        registry.insert(_field("Symbol", "utf8", 55))
    del view
    assert registry.insert(_field("Symbol", "utf8", 55)) is None


def _order(seed: FixRegistry) -> Field:
    """A root carrying a group, a tag no dictionary explains, and its clock.

    A message built by hand reads UTC now for a SendingTime it does not
    state, so the root states one and two builds of it are one message.
    """
    return Field(
        "NewOrderSingle",
        DataType.from_fields(
            [
                seed.field_by_tag(55),
                seed.field_by_tag(38),
                seed.field_by_name("Parties"),
                Field("9999", "utf8"),
                seed.field_by_tag(52),
            ]
        ),
        nullable=False,
    )


ORDER_VALUE: dict[str, Any] = {
    "symbol": "AAPL",
    "orderqty": decimal.Decimal("100"),
    "parties": [{"partyid": "BROKER", "partyidsource": "D", "partyrole": 1}],
    "9999": "custom",
    "sendingtime": CLOCK,
}


def test_message_resolves_through_the_registry_it_carries(seed: FixRegistry) -> None:
    root = _order(seed)
    message = FixMsg(root, ORDER_VALUE, seed)

    # A child stating a typed fact fills the holder that owns it and leaves
    # the row, so the root keeps only what the message states of its own.
    assert [child.name for child in message.field] == [
        "symbol",
        "parties",
        "9999",
    ]
    assert message.registry == seed
    assert len(message) == 3
    symbol_id = seed.field_by_tag(55).fix.id
    assert symbol_id is not None
    assert message.by_tag(55).as_py() == "AAPL"
    assert message.by_id(symbol_id).as_py() == "AAPL"
    assert message.by_name("symbol").as_py() == "AAPL"
    assert message.by_tag(38).as_py() == 100.0
    assert message.by_path("parties[0].partyid").as_py() == "BROKER"
    # An unknown tag is retained under its rendered name, never dropped.
    assert message.by_tag(9999).as_py() == "custom"
    # An identifier is exact: a field the dictionary does not hold misses.
    absent_id = _field("TradeID", "utf8", 5001).fix.id
    assert absent_id is not None
    assert message.get_by_id(absent_id) is None

    assert message[55] == message.by_tag(55)
    assert message["symbol"] == message.by_tag(55)
    assert message.get(55) == message.by_tag(55)
    assert message.get(1234) is None
    assert message.get(1234, "fallback") == "fallback"
    assert message.get_by_name("nope") is None
    assert message.get_by_path("Parties.PartyID") is None

    with pytest.raises(KeyError) as by_tag:
        message.by_tag(1234)
    assert by_tag.value.args[0] == 'expected a fix value at "tag 1234", got nothing'
    with pytest.raises(KeyError) as by_path:
        message.by_path("Parties.PartyID")
    assert "path Parties.PartyID" in by_path.value.args[0]
    with pytest.raises(TypeError, match="not bool"):
        message[True]
    with pytest.raises(TypeError):
        message.by_id("55")  # type: ignore[arg-type]
    with pytest.raises(OverflowError):
        message.get_by_id(2**31)

    # The mapping input became the ordered row the message declares.
    pairs = [(name, value) for name, value in message]
    assert [name for name, _ in pairs] == [child.name for child in message.field]
    assert pairs[0][0] == "symbol" and pairs[0][1].as_py() == "AAPL"

    # A native Scalar names the same content under the message's own root.
    # The typed facts are not in it, so the rebuild settles its own clocks
    # and states the content the first one states.
    rebuilt = FixMsg(message.field, message.value, seed)
    assert rebuilt.field == message.field
    assert rebuilt.value == message.value
    assert rebuilt.entries() == message.entries()


def test_a_field_maps_its_wire_values_onto_times_in_force() -> None:
    """``FIX:timeinforce``: the member each wire value of a field stands for."""
    registry = FixRegistry()
    # TimeInForce(59) reads through the crate's own table with no mapping,
    # a venue's own value as the catch-all; a field that states no time in
    # force reads as none.
    assert registry.timeinforce_of(59, "3") is TimeInForce.IOC
    assert registry.timeinforce_of(59, "Z") is TimeInForce.OTHER
    assert registry.timeinforce_of(54, "1") is None

    field = Field("VenueTif", "utf8")
    field.fix.tag = 20059
    assert field.fix.timeinforces == []
    field.fix.timeinforces = [("G", TimeInForce.GTC), ("I", "ioc"), ("F", 5)]
    assert field.fix.timeinforces == [
        ("G", TimeInForce.GTC),
        ("I", TimeInForce.IOC),
        ("F", TimeInForce.FOK),
    ]
    with pytest.raises(ValueError):
        field.fix.timeinforces = [("G", "not a time in force")]
    with pytest.raises(ValueError):
        field.fix.timeinforces = [("G", 1), ("G", 2)]
    assert len(field.fix.timeinforces) == 3
    registry.insert(field)
    assert registry.timeinforce_of(20059, "G") is TimeInForce.GTC
    assert (20059, "I", TimeInForce.IOC) in registry.timeinforce_sources()

    # A message reads its time in force through either field, as the member.
    codec = _fixed(registry)
    head = b"8=FIX.4.4|35=D|49=S|56=T|34=7|52=20240102-10:15:30|11=A1|55=AAPL|54=1|38=100|44=10.5|"
    assert codec.parse_fix_line(head + b"59=3|10=0|").timeinforce is TimeInForce.IOC
    assert codec.parse_fix_line(head + b"20059=G|10=0|").timeinforce is TimeInForce.GTC

    field.fix.timeinforces = []
    assert field.fix.timeinforces == []


def test_a_message_holds_its_typed_facts_beside_its_row(seed: FixRegistry) -> None:
    """The three holders and the two extras, each answering its own facts."""
    codec = _fixed(seed)
    wire = (
        b"8=FIX.4.4|35=D|49=SENDER|56=TARGET|34=7|52=20240102-10:15:30|"
        b"11=A1|55=AAPL|54=1|15=USD|38=100|44=10.5|31=10.25|32=40|"
        b"6=10.3|14=40|151=60|140=9.75|58=note|60=20240102-10:15:31|10=0|"
    )
    message = codec.parse_fix_line(wire)

    header = message.header()
    assert isinstance(header, FixHeader)
    assert header.beginstring == "FIX.4.4"
    assert header.msgtype == "D"
    assert header.sendercompid == "SENDER"
    assert header.targetcompid == "TARGET"
    assert header.msgseqnum == 7
    assert header.sendingtime == CLOCK_NS
    assert header.stated_sendingtime
    assert header.possdupflag is None
    assert header.msgdirection is None
    assert header == message.header()
    assert hash(header) == hash(message.header())
    assert copy.copy(header) == header

    capture = message.capture()
    assert isinstance(capture, FixCapture)
    # What the line said about its capture, and nothing the reader said: the
    # object it came out of, and whatever else the reader carried, are the
    # capture's own columns, on the row - and nothing records a capture
    # clock.
    assert not hasattr(capture, "sourceurl")
    assert not hasattr(capture, "recordedat")
    assert capture.msgpluginid is None
    assert capture.msgctxid is None
    assert capture.msgsessionid is None
    assert capture.msgsesseventid is None

    # The message answers every fact the graph vocabulary states as its own
    # property, and expands to the one typed leaf it is.
    event = message
    [operation] = message.market_data()
    assert isinstance(operation, MarketData)
    assert operation.kind == "order_event"
    leaf = operation.as_order_event()
    assert isinstance(leaf, OrderEvent)
    # The transaction stands one second from the sending clock, which is
    # exactly the codec's default delay, so the two are the one event said
    # twice and the more exact saying of it dates the message.
    assert event.currunix == CLOCK_NS + 1_000_000_000
    assert event.creaunix == event.currunix
    assert event.execunix is None
    assert event.recdunix == CLOCK_NS
    assert not hasattr(event, "refrecdunix")
    assert not hasattr(event, "marketoperationid")
    assert event.msgcat is MarketDataKind.ORDR
    assert event.strikeprice is None and not hasattr(event, "strikepx")
    assert event.price.as_py() == 10.5
    # OrderQty(38) is the ordered quantity; `quantity` moves with leavesqty.
    assert event.ordqty is not None and event.ordqty.as_py() == 100
    assert event.quantity.as_py() == 60
    assert event.lastpx is not None and event.lastpx.as_py() == decimal.Decimal("10.25")
    assert event.lastqty is not None and event.lastqty.as_py() == 40
    assert event.avgpx is not None and event.avgpx.as_py() == decimal.Decimal("10.3")
    assert event.cumqty is not None and event.cumqty.as_py() == 40
    assert event.leavesqty is not None and event.leavesqty.as_py() == 60
    assert event.prevpx is not None and event.prevpx.as_py() == decimal.Decimal("9.75")
    assert event.prevqty is None
    assert event.timeinforce is TimeInForce.DAY
    assert event.tradable is None
    assert event.ticker == "AAPL"
    assert not hasattr(event, "px")
    assert not hasattr(event, "qty")
    assert not hasattr(event, "symbolticker")
    assert event.currency.as_py() == "USD"
    assert event.unit == ""
    assert event.side is Side.BUYS
    # A market message's stored cross code is its kind, its side, its base.
    assert event.crosscode == "10:1:A1"
    # The names the order goes by, each under the field that stated it.
    assert _kinds(event.identifiers) == {"clordid": "A1"}
    # The parties are the Parties groups and the Account(1) the message
    # names, and this one names neither.
    assert not event.partyids and not hasattr(event, "userids")
    # An order states no instrument code, no bridge metadata and no FX legs.
    assert not event.securityids
    assert event.cficode is None and event.miccode is None
    assert event.metadata == {}
    assert event.spotrate is None and event.forwardpoints is None
    # A buyer's price, quantity and currency are its side's bid facts; the
    # ask side is stated by nobody, and no rate is stated.
    assert not hasattr(event, "bid") and not hasattr(event, "ask")
    assert event.bidpx == event.price and event.bidqty == event.quantity
    assert event.bidccy == event.currency
    assert event.askpx is None and event.askqty is None and event.askccy is None
    assert event.fxrates == {}
    assert event.seqnum == 0
    assert event.prevuuid is None and event.prevunix is None and event.snapunix is None
    # A new order stating no status asks for a new order.
    assert event.state is State.PENDING_NEW
    assert message.isincode is None
    assert message.market_data() == [operation]

    # The leaf the message expands to states the facts the message states.
    assert leaf.currunix == message.currunix
    assert leaf.crossuuid == message.crossuuid
    assert leaf.crosscode == message.crosscode
    assert (leaf.price, leaf.quantity, leaf.side) == (message.price, message.quantity, message.side)
    assert leaf.identifiers == message.identifiers and leaf.bidpx == message.bidpx
    assert message.crosscode == event.crosscode
    assert message.currhashcode == event.currhashcode
    assert message.crosshashcode == event.crosshashcode
    assert message.identifiers == event.identifiers
    assert message.securityids == event.securityids
    assert message.unit == event.unit
    assert message.ticker == event.ticker
    assert message.timeinforce == event.timeinforce
    assert message.tradable == event.tradable
    assert message.fxrates == event.fxrates
    assert message.spotrate == event.spotrate and message.forwardpoints == event.forwardpoints
    assert message.metadata == event.metadata
    assert message.msgcat == event.msgcat == 10
    assert message.srcuuids == event.srcuuids == []
    assert message.state == event.state
    assert message.seqnum == event.seqnum
    assert message.prevuuid is None
    assert message.price == event.price
    assert message.quantity == event.quantity
    assert not hasattr(message, "px")
    assert not hasattr(message, "qty")
    assert message.side == event.side
    assert message.currency == event.currency
    # The identities are uuid scalars, the codes uint64.
    assert message.curruuid.dtype == DataType("uuid")
    assert isinstance(message.currhashcode, int) and message.currhashcode != 0
    # The cross identity derives from the cross code, so it is not the own one.
    assert message.crossuuid != message.curruuid
    assert message.crosshashcode != 0

    # Tag 58 and a bridge's namespaced keys are the two extras.
    assert message.text == "note"
    assert message.metadata == {}
    bridged = codec.parse_ullink_line(b"|#SYMBOL=TTF|#TECH.CLIENTID=MCFP2|")
    assert bridged.metadata == {"tech.clientid": "MCFP2"}
    assert bridged.text is None

    # A typed tag answers the holder, typed as its column is; every other
    # key reaches the row.
    assert message.by_tag(35).as_py() == "D"
    assert message.by_tag(52) == CLOCK
    assert message.by_tag(54).as_py() is Side.BUYS
    assert message.by_tag(58).as_py() == "note"
    assert message.by_tag(HASHCODE_TAG).as_py() == message.currhashcode
    assert message.by_tag(CURRUUID_TAG) == message.curruuid
    # The instant is the transaction, one second from the sending clock and
    # so inside the codec's default delay.
    assert message.by_tag(UNIX_TAG).as_py() == dt.datetime.fromtimestamp(
        (CLOCK_NS + 1_000_000_000) / 1e9, dt.timezone.utc
    )
    # The cross code is stored as `{kind}:{side}:{base}`.
    assert message.by_tag(CROSSCODE_TAG).as_py() == "10:1:A1"
    assert message.by_name("crosscode").as_py() == "10:1:A1"
    assert message.get_by_tag(SNAPUNIX_TAG) is None
    # None of them is a row child: the content row holds the rest, the side
    # and the currency among it, and the day order the dictionary derived.
    assert [child.name for child in message.field] == [
            "symbol",
            "side",
            "currency",
            "prevclosepx",
            "transacttime",
        "timeinforce",
        "settlcurrency",
        "currencycodesource",
    ]


def test_a_message_settles_its_identity_from_what_it_states(seed: FixRegistry) -> None:
    """The cross code, the hash code and the two identities, settled once."""
    codec = _fixed(seed)
    order = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|52=20240102-10:15:30|10=0|")

    # The cross code is the first stated of the cross tags, stored under its
    # kind and side (`10:0:A1`: an order stating no side), and the cross
    # identity and the cross hash code follow the stored code.
    assert order.crosscode == "10:0:A1"
    replaced = codec.parse_fix_line(
        b"8=FIX.4.4|35=G|41=A1|11=A2|55=AAPL|52=20240102-10:15:30|10=0|"
    )
    # 37, then 11, then 41: a replace states its own ClOrdID first.
    assert replaced.crosscode == "10:0:A2"
    reported = codec.parse_fix_line(
        b"8=FIX.4.4|35=8|37=O1|11=A1|55=AAPL|52=20240102-10:15:30|10=0|"
    )
    assert reported.crosscode == "10:0:O1"

    # A message naming no cross code takes its own identity as the cross one.
    plain = codec.parse_fix_line(b"8=FIX.4.4|35=0|52=20240102-10:15:30|10=0|")
    assert plain.crosscode == ""
    assert plain.crosshashcode == 0
    assert plain.crossuuid == plain.curruuid

    # Two reads of one line are one message, identities included.
    again = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|52=20240102-10:15:30|10=0|")
    assert again == order
    assert again.curruuid == order.curruuid
    assert again.currhashcode == order.currhashcode
    assert again.digest() == order.digest()

    # A write settles it again: the content moves, so the hash code and the
    # own identity move with it while the cross identity stands.
    written = copy.copy(order)
    written.set(55, "MSFT")
    assert written.currhashcode != order.currhashcode
    assert written.curruuid != order.curruuid
    assert written.crossuuid == order.crossuuid
    # Writing the cross code moves the cross identity too.
    # A code is stored under the message's kind and side whatever it names.
    written.set("crosscode", "X-1")
    assert written.crosscode == "10:0:X-1"
    assert written.crossuuid != order.crossuuid
    written.set("crosscode", "8:2:X-1")
    assert written.crosscode == "10:0:X-1", "a prefix of another kind or side is replaced"


def test_the_bridge_session_event_is_captured_but_never_the_crosscode_or_content(
    seed: FixRegistry,
) -> None:
    """FIX names the chain; a complete bridge bracket names its delivery."""
    codec = _fixed(seed)
    message = codec.parse_ullink_line(
        b"MSGTYPE=8|#ORDERID=ORDER-1|#CLORDID=CLIENT-1|#MSGSESSIONID=SESSION-1|"
        b"#MSGCTXID=CONTEXT-1|#MSGSEQNUM=7|#SYMBOL=n/A|#VENUEOWNTHING=n/A|"
    )

    # The message type, session, context and sequence joined as they are
    # stated, on the capture: delivery provenance, never a name the message
    # goes by.
    assert message.crosscode == "10:0:ORDER-1"
    assert message.capture().msgsesseventid == "8:SESSION-1:CONTEXT-1:7"
    assert message.get_by_tag(MSGSESSEVENTID_TAG).as_py() == "8:SESSION-1:CONTEXT-1:7"
    assert _kinds(message.identifiers) == {"clordid": "CLIENT-1", "orderid": "ORDER-1"}
    assert message.get_by_tag(55) is None
    assert message.get_by_name("venueownthing") is None
    content_hash = message.currhashcode
    content_uuid = message.curruuid

    # Every write settles it again, and none of it is content.
    message.set("msgsessionid", "SESSION-2")
    assert message.capture().msgsessionid == "SESSION-2"
    assert message.capture().msgsesseventid == "8:SESSION-2:CONTEXT-1:7"
    assert _kinds(message.identifiers) == {"clordid": "CLIENT-1", "orderid": "ORDER-1"}
    assert message.currhashcode == content_hash
    assert message.curruuid == content_uuid

    # A missing part unsays it rather than leaving a stale key behind.
    message.set("msgctxid", None)
    assert message.capture().msgctxid is None
    assert message.capture().msgsesseventid is None
    assert message.currhashcode == content_hash

    message.set("msgctxid", "CONTEXT-2")
    assert message.capture().msgsesseventid == "8:SESSION-2:CONTEXT-2:7"
    assert message.currhashcode == content_hash

    # It is a column of the fixed row, and a row read back states it again.
    schema = fix_schema(seed)
    row = message.into_row(schema)
    assert row.as_py()[schema.index_of("msgsesseventid")] == "8:SESSION-2:CONTEXT-2:7"
    rebuilt = FixMsg.from_row(schema, row, seed)
    assert rebuilt.crosscode == message.crosscode
    assert rebuilt.identifiers == message.identifiers
    assert rebuilt.capture() == message.capture()
    assert rebuilt.into_row(schema) == row


def test_a_lines_session_event_joins_its_four_values_as_stated(seed: FixRegistry) -> None:
    """The bridge's session and context captures, the type and the sequence."""
    codec = _fixed(seed, capture_names=["msgsessionid", "msgctxid"])
    line = TextLine(0, b"8=FIX.4.4|35=8|34=1094|10=0|", ["e7256476", "9effef3e6a"])
    (message,) = list(codec.parse_text_line(line))

    capture = message.capture()
    assert capture.msgsessionid == "e7256476"
    assert capture.msgctxid == "9effef3e6a"
    # Joined by `:` with nothing in front of a part, and held by the capture
    # rather than among the names the message goes by.
    assert capture.msgsesseventid == "8:e7256476:9effef3e6a:1094"
    assert message.by_tag(MSGSESSEVENTID_TAG).as_py() == "8:e7256476:9effef3e6a:1094"
    assert not message.identifiers.contains_kind("msgsesseventid")

    # A part missing is no session event at all.
    for body, captures in (
        (b"8=FIX.4.4|35=8|10=0|", ["e7256476", "9effef3e6a"]),
        (b"8=FIX.4.4|35=8|34=1094|10=0|", ["e7256476", None]),
    ):
        (partial,) = list(codec.parse_text_line(TextLine(0, body, captures)))
        assert partial.capture().msgsesseventid is None, body


def test_a_write_reaches_the_holder_or_the_row_by_the_key_it_resolves(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    order = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|52=20240102-10:15:30|10=0|")
    message = copy.copy(order)

    # A row key types the value through the dictionary's field and keeps the
    # child where it stands; an absent one is appended.
    at = message.field.index_of("symbol")
    message.set(55, "MSFT")
    assert message.field.index_of("symbol") == at
    assert message.by_tag(55).as_py() == "MSFT"
    message.set(1, "ACC-1")
    assert message.by_tag(1).as_py() == "ACC-1"
    assert [child.name for child in message.field][-1] == "account"

    # A typed key writes its holder and never the row. `OrderQty(38)` and
    # `Price(44)` are two: the numbers the message lifted, and what it is
    # *about* is read off them.
    before = len(message)
    message.set(38, decimal.Decimal("100"))
    assert message.quantity.as_py() == 100
    assert message.by_tag(38).as_py() == decimal.Decimal("100")
    assert len(message) == before
    message.set(44, "12.5")
    assert message.price.as_py() == decimal.Decimal("12.5")
    assert len(message) == before
    # The side is an ordinary child, so writing it grows the row once.
    message.set(54, "2")
    assert message.side is Side.SELL
    assert message.by_tag(54).as_py() is Side.SELL
    message.set(58, "note")
    assert message.text == "note"
    message.set("metadata", {"tech.clientid": "A"})
    assert message.metadata == {"tech.clientid": "A"}
    # The identifier maps are read off the fields that state them: a write
    # to `ClOrdID(11)` restates `altids`, and there is no map key to write.
    assert _kinds(message.identifiers) == {"clordid": "A1"}
    message.set(11, "C-2")
    assert _kinds(message.identifiers) == {"clordid": "C-2"}
    with pytest.raises(KeyError):
        message.set("identifiers", {"clordid": "C-2"})
    message.set(11, "A1")
    assert _kinds(message.identifiers) == {"clordid": "A1"}

    # `None` clears a typed fact and is stored as a stated null in the row.
    assert message.remove(54) is not None
    assert message.side is Side.UNKN
    assert message.remove(54) is None
    message.set(58, None)
    assert message.text is None
    message.set(55, None)
    held = message.get_by_tag(55)
    assert held is not None and held.is_null()

    # A name the dictionary does not know still reaches a child spelled that
    # way, and a bare unknown tag appends a text child named by its decimal.
    message.set(9999, "custom")
    assert message.by_tag(9999).as_py() == "custom"
    assert [child.name for child in message.field][-1] == "9999"
    with pytest.raises(KeyError, match="nosuchfield"):
        message.set("nosuchfield", "y")

    # A removal answers the value and the other tags still reach theirs.
    count = len(message)
    assert message.remove(9999) == Scalar.from_("custom")
    assert len(message) == count - 1
    assert message.remove("nosuchfield") is None
    assert message.by_tag(11).as_py() == "A1"

    # A hashed message is frozen; a copy takes writes again.
    hashed = copy.copy(order)
    held_set = {hashed}
    with pytest.raises(TypeError, match="hashed FixMsg is frozen"):
        hashed.set(55, "MSFT")
    with pytest.raises(TypeError, match="hashed FixMsg is frozen"):
        hashed.remove(55)
    assert hashed in held_set
    written = copy.copy(hashed)
    written.set(55, "MSFT")
    assert written.by_tag(55).as_py() == "MSFT"


def test_the_entries_are_the_row_read_as_a_tree(seed: FixRegistry) -> None:
    """One tuple per stated child, a group's occurrences nested under it."""
    codec = _fixed(seed)
    wire = b"8=FIX.4.4|35=D|11=A1|55=AAPL|453=1|448=BRK|447=D|452=1|9999=x|10=0|"
    message = codec.parse_fix_line(wire)

    assert message.entries() == [
        (55, "symbol", "AAPL", []),
        (
            453,
            "parties",
            "1",
            [
                (
                    0,
                    "party",
                    None,
                    [
                        (448, "partyid", "BRK", []),
                        (447, "partyidsource", "D", []),
                        (452, "partyrole", "1", []),
                    ],
                )
            ],
        ),
        (0, "9999", "x", []),
        (59, "timeinforce", "0", []),
    ]
    # The group's length is the group entry's own value, filed under its
    # counter's tag, and no counter child stands beside it.
    assert [tag for tag, _, _, _ in message.entries()].count(453) == 1
    # A typed fact is not an entry: the holders answer those, the lifted
    # `ClOrdID(11)` and the trailer's own `CheckSum(10)` included.
    assert all(tag not in (8, 10, 11, 35, 52) for tag, _, _, _ in message.entries())

    # The wire puts the header and the lifted fields in front of them, the
    # trailer behind, and a derived value the dictionary licensed rides with
    # the row.
    assert message.into_bytes(124) == (
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|453=1|448=BRK|447=D|452=1|9999=x|59=0|10=0|"
    )
    assert message.into_text("|") == message.into_bytes(124).decode()
    assert len(message.digest()) == 16
    assert message.digest() == copy.copy(message).digest()

    # A key no dictionary explains keeps tag 0 and its own spelling, and the
    # wire re-emits it as it arrived.
    unresolved = _fixed(seed, exclude_msgtypes=[]).parse_pairs(
        [("999999", "one"), ("OwnThing", "two")]
    )
    assert unresolved.entries() == [(0, "999999", "one", []), (0, "ownthing", "two", [])]
    assert unresolved.by_tag(999_999).as_py() == "one"
    assert unresolved.get_by_name("OwnThing") is not None
    # The beginstring the parse read the frame at heads the wire whatever
    # the frame itself stated.
    assert unresolved.into_bytes(124).endswith(b"999999=one|ownthing=two|")


def test_message_is_hashable_copyable_and_picklable(seed: FixRegistry) -> None:
    root = _order(seed)
    message = FixMsg(root, ORDER_VALUE, seed)
    same = FixMsg(root, ORDER_VALUE, seed)

    assert message == same
    assert hash(message) == hash(same)
    assert message.stable_hash() == same.stable_hash()
    assert len({message, same}) == 1
    assert message != FixMsg(root, {**ORDER_VALUE, "symbol": "MSFT"}, seed)
    assert message != object()

    assert copy.copy(message) == message
    assert copy.deepcopy(message) == message
    restored = pickle.loads(pickle.dumps(message))
    assert restored == message
    assert restored.registry == seed
    assert restored.header() == message.header()
    assert (restored.currunix, restored.creaunix, restored.recdunix, restored.side) == (
        message.currunix,
        message.creaunix,
        message.recdunix,
        message.side,
    )
    assert restored.by_path("parties[0].partyid").as_py() == "BROKER"

    assert repr(message) == 'FixMsg("NewOrderSingle", 3 values)'


def test_message_refuses_a_value_its_field_refuses(seed: FixRegistry) -> None:
    root = Field("row", DataType.from_fields([seed.field_by_tag(55)]), nullable=False)
    with pytest.raises(ValueError, match="symbol"):
        FixMsg(root, {"symbol": [1]}, seed)
    with pytest.raises(ValueError):
        FixMsg(Field("scalar", "utf8"), {"symbol": "AAPL"}, seed)


INSTALL_SCRIPT = """
import pathlib
import sys

from yggdryl import DataType, Field
from yggdryl.fix import FixMsg, FixRegistry

seed = FixRegistry.from_handle(pathlib.Path(sys.argv[1]))
FixRegistry.install_env(seed)
assert FixRegistry.from_env() == seed
assert FixRegistry.from_env().field_by_tag(55).name == "symbol"

root = Field(
    "row",
    DataType.from_fields([FixRegistry.from_env().field_by_tag(55)]),
    nullable=False,
)
assert FixMsg(root, {"symbol": "AAPL"}).registry == seed

try:
    FixRegistry.install_env(FixRegistry())
except ValueError as error:
    assert "already resolved" in str(error), error
else:
    raise AssertionError("installing twice must fail")
print("ok")
"""


def test_install_env_wins_before_the_default_resolves() -> None:
    """Process-wide state, so it is driven in a process of its own."""
    result = subprocess.run(
        [sys.executable, "-c", INSTALL_SCRIPT, str(SEED)],
        capture_output=True,
        text=True,
        cwd=REPO,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip().endswith("ok")


def test_message_links_the_process_default_when_none_is_named() -> None:
    default = FixRegistry.from_env()
    assert isinstance(default, FixRegistry)
    assert default == FixRegistry.from_env()

    root = Field("row", DataType.from_fields([_field("symbol", "utf8", 55)]), nullable=False)
    assert FixMsg(root, {"symbol": "AAPL"}).registry == default
    explicit = FixRegistry.from_fields([_field("symbol", "utf8", 55)])
    assert FixMsg(root, {"symbol": "AAPL"}, explicit).registry == explicit


def test_reader_parses_every_frame_shape_the_core_reads(seed: FixRegistry) -> None:
    """One reader, five entry points, and each is the core's own."""
    # A bridge row and a FIXML order state no message type, so this reader is
    # told to read the untyped row the default refusals drop.
    reader = _fixed(seed, exclude_msgtypes=[])

    framed = next(reader.parse_line(b"sending >> 8=FIX.4.4|35=D|55=AAPL|10=0|"))
    assert framed.by_tag(55).as_py() == "AAPL"
    assert next(reader.parse_line(b"8=FIX.4.4|35=D|55=AAPL|10=0|")).by_tag(55).as_py() == "AAPL"
    assert reader.parse_fix_line(b"8=FIX.4.4\x0135=D\x0155=AAPL\x0110=0\x01").by_tag(55).as_py() == "AAPL"
    assert reader.parse_pairs([("55", "AAPL")]).by_tag(55).as_py() == "AAPL"
    assert reader.parse_fixml_line(b"<Order ClOrdID='XML-1'/>").by_tag(11).as_py() == "XML-1"

    # A bridge frame, byte for byte: `#`-prefixed name keys, one occurrence
    # whose value packs its members behind the two control bytes ULLINK uses.
    bridge = reader.parse_ullink_line(
        b"|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=1"
        b"|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|"
    )
    inferred = next(
        reader.parse_line(
            b"|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=1"
            b"|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|"
        )
    )
    assert inferred == bridge
    assert bridge.by_tag(55).as_py() == "TTF"
    assert bridge.price.as_py() == 41.25
    assert bridge.ordqty.as_py() == 1200
    assert bridge.quantity is None
    assert bridge.side is Side.BUYS
    assert bridge.by_path("parties[0].partyid").as_py() == "BUYSIDE"

    # A line carrying two frames is two messages; a sentence is none.
    two = list(reader.parse_line(b"8=FIX.4.4|35=D|11=A|10=001|8=FIX.4.4|35=8|37=O|10=002|"))
    assert [held.header().msgtype for held in two] == ["D", "8"]
    assert list(reader.parse_line(b"no level printed by this plugin")) == []

    # A JSON document is one unknown message stating nothing at all.
    (document,) = list(reader.parse_line(b'{"a":1}'))
    assert MimeType.infer_bytes(b'{"a":1}') == MimeType.JSON
    assert FixCodec.infer_msgtype_bytes(b'{"a":1}') is None
    assert document.field.name == "unknown"
    assert document.entries() == []
    assert document.header().msgtype == ""


def test_the_default_sending_time_is_the_clock_undated_intake_takes(seed: FixRegistry) -> None:
    """The pin that makes a parse of undated bytes repeat."""
    registry = FixRegistry()
    assert FixCodec(registry).default_sending_time is None
    assert FixCodec(registry, default_sending_time=None).default_sending_time is None
    # A `Heartbeat` is what the default refuses, and this case is about the
    # clock an undated message takes, so this codec reads every type.
    codec = FixCodec(registry, default_sending_time=CLOCK, exclude_msgtypes=[])
    assert codec.default_sending_time == CLOCK
    # An aware UTC `datetime` is read once into the same nanosecond clock.
    assert FixCodec(registry, default_sending_time=CLOCK_INSTANT).default_sending_time == CLOCK

    wire = b"8=FIX.4.4|35=0|10=0|"
    first = next(codec.parse_line(wire))
    second = next(codec.parse_line(wire))
    assert first == second
    assert first.currhashcode == second.currhashcode
    assert first.digest() == second.digest()
    assert first.header().sendingtime == CLOCK_NS
    assert not first.header().stated_sendingtime
    assert first.currunix == CLOCK_NS
    assert first.creaunix == CLOCK_NS
    # A settled clock is not the message's own, so the wire does not state it.
    assert first.into_bytes(ord("|")) == wire

    # A message's own clocks precede the pin: SendingTime is the stated one,
    # and it is the reference the event is dated against. This transaction
    # stands nearly two seconds in front of it, outside the codec's default
    # one-second delay, so the sending clock dates the event and the creation.
    stated = next(
        codec.parse_line(
            b"8=FIX.4.4|35=0|52=19700101-00:00:02.123456789|60=19700101-00:00:03.987654321|10=0|"
        )
    )
    assert stated.by_tag(52) == DataType('datetime64(ns,"UTC")').scalar(2_123_456_789)
    assert stated.header().sendingtime == 2_123_456_789
    assert stated.header().stated_sendingtime
    assert stated.currunix == 2_123_456_789
    assert stated.creaunix == 2_123_456_789
    assert stated.by_tag(60) == DataType('datetime64(ns,"UTC")').scalar(3_987_654_321)
    # `OrigSendingTime(122)` dates nothing at the parse, and a `TransactTime`
    # stating only a day dates nothing at all: the sending clock stands for
    # a resent message and a day-only transaction alike.
    dictionary = _fixed(seed, exclude_msgtypes=[])
    resent = next(dictionary.parse_line(b"8=FIX.4.4|35=0|122=19700101-00:00:01|10=0|"))
    assert resent.creaunix == CLOCK_NS
    assert resent.by_tag(122) == DataType('datetime64(ns,"UTC")').scalar(1_000_000_000)
    day = next(dictionary.parse_line(b"8=FIX.4.4|35=D|11=A|60=20260814|10=0|"))
    assert day.currunix == CLOCK_NS

    # The pin is exact: another unit, a naive clock or text is refused.
    for refused in (DataType('datetime64(us,"UTC")').scalar(0), DataType("datetime64(ns)").scalar(0), "1970-01-01T00:00:00Z"):
        with pytest.raises(ValueError, match="default_sending_time"):
            FixCodec(registry, default_sending_time=refused)


# A row header dating each line by an `mtime` capture, the line's own clock.
DATED = r"^(?P<mtime>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) "
RECORDED = "2024-03-05 10:15:30.250"
RECORDED_NS = 1_709_633_730_250_000_000


def _dated_line(body: bytes) -> TextLine:
    """``body`` as a line the header dated at ``RECORDED``, in UTC."""
    options = TextOptions()
    options.rowheader = DATED
    options.timezone = Timezone.UTC
    return TextLine(0, body, [RECORDED], options)


def test_a_lines_own_clock_dates_a_message_stating_no_sending_time(seed: FixRegistry) -> None:
    """The line was recorded as its message went by: nearer the send than any pin."""
    line = _dated_line(b"8=FIX.4.4|35=8|10=0|")
    assert line.currunix == RECORDED_NS
    schema = fix_schema(seed)

    # Unpinned and pinned alike, the line's clock is the sending clock an
    # undated frame on it is read against - ahead of the default and of now -
    # and so the instant, the creation and the recording.
    for codec in (FixCodec(seed), _fixed(seed)):
        (message,) = list(codec.parse_text_line(line))
        assert message.header().sendingtime == RECORDED_NS
        assert message.currunix == RECORDED_NS
        assert message.creaunix == RECORDED_NS
        assert message.recdunix == RECORDED_NS
        # Supplied, never stated: neither the wire nor the row's own column
        # says what the frame did not.
        assert not message.header().stated_sendingtime
        assert b"52=" not in message.into_bytes(ord("|"))
        assert message.into_row(schema).as_py()[schema.index_of("sendingtime")] is None

    # A frame stating its own SendingTime keeps it; the line still says when
    # it was recorded.
    stated = _dated_line(b"8=FIX.4.4|35=8|52=20240102-10:15:30|10=0|")
    (message,) = list(FixCodec(seed).parse_text_line(stated))
    assert message.header().sendingtime == CLOCK_NS
    assert message.header().stated_sendingtime
    assert message.currunix == CLOCK_NS
    assert message.recdunix == RECORDED_NS

    # The Arrow door reads the same clock off a row's `currunix` cell.
    source = pa.table(
        {
            "currunix": pa.array([RECORDED_NS], pa.timestamp("ns", tz="UTC")),
            "body": pa.array([b"8=FIX.4.4|35=8|10=0|"], pa.binary()),
        }
    )
    parsed = _fixed(seed).parse_text_arrow_reader(source).read_all()
    assert parsed.column("currunix").cast(pa.int64()).to_pylist() == [RECORDED_NS]
    assert parsed.column("recdunix").cast(pa.int64()).to_pylist() == [RECORDED_NS]
    assert parsed.column("sendingtime").to_pylist() == [None]

    # A raw-byte door holds no line, so the same frame there takes the pin.
    assert _fixed(seed).parse_fix_line(b"8=FIX.4.4|35=8|10=0|").currunix == CLOCK_NS


def test_a_quote_states_its_bid_and_offer_and_holds_both_legs(seed: FixRegistry) -> None:
    """A quote's bid and offer are facts of their own, its two legs one message (A20)."""
    codec = _fixed(seed)

    # A bid alone states the bid and no side: no side is invented from it.
    bid = codec.parse_fix_line(b"8=FIX.4.4|35=S|117=Q1|55=AAPL|15=USD|132=101.5|134=200|10=0|")
    assert bid.side is Side.UNKN and bid.price is None
    assert bid.bidpx is not None and bid.bidpx.as_py() == decimal.Decimal("101.5")
    assert bid.bidqty is not None and bid.bidqty.as_py() == 200
    assert bid.bidccy is not None and bid.bidccy.as_py() == "USD"
    assert bid.askpx is None and bid.askqty is None and bid.askccy is None
    assert bid.crosscode == "14:0:Q1"

    # An offer alone states the ask.
    offer = codec.parse_fix_line(b"8=FIX.4.4|35=S|117=Q2|55=AAPL|133=102|135=50|10=0|")
    assert offer.side is Side.UNKN
    assert offer.askpx is not None and offer.askpx.as_py() == 102
    assert offer.askqty is not None and offer.askqty.as_py() == 50

    # Both legs are one message: the parse never splits a quote, which states
    # no side, no price of its own and its two legs under side 0.
    [quote] = codec.parse_line(b"8=FIX.4.4|35=S|117=Q3|55=AAPL|132=101|133=102|10=0|")
    assert quote.side is Side.UNKN and quote.price is None
    assert quote.crosscode == "14:0:Q3"
    assert quote.bidpx is not None and quote.bidpx.as_py() == 101
    assert quote.askpx is not None and quote.askpx.as_py() == 102
    # Its book rests it on both sides as one entry, one delta; with no grid
    # that book states its delta alone, whole over the empty book of its key.
    [book] = [data.as_book_event() for data in MarketData.from_arrow_reader(codec.book_arrow_reader([quote]))]
    assert book is not None and not book.is_complete
    assert [delta.crosscode for delta in book.deltas] == ["14:0:Q3"]
    assert (book.bidpx, book.askpx) == (quote.bidpx, quote.askpx)
    whole = book.with_previous(BookEvent.keyed(book.currunix, "AAPL"))
    assert whole is not None
    assert [entry.crosscode for entry in whole.alive] == ["14:0:Q3"]
    assert whole.alive_on(Side.BUYS) == whole.alive_on(Side.SELL) == whole.alive

    # A stated side is a tag: the code states side 0, and the one leg stated
    # is the one side the quote rests on.
    [stated] = codec.parse_line(b"8=FIX.4.4|35=S|117=Q4|55=AAPL|54=2|132=101|10=0|")
    assert stated.side is Side.SELL and stated.crosscode == "14:0:Q4"


def test_an_execution_report_stating_no_execution_clock_executed_at_its_instant(
    seed: FixRegistry,
) -> None:
    """Intake dates the execution a report states, rather than a later walk."""
    codec = _fixed(seed)

    # A fill stating no ExecutionTimestamp, no execution TrdRegTimestamp and
    # no TransactTime executed when it happened.
    fill = codec.parse_fix_line(b"8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|10=0|")
    assert fill.currunix == CLOCK_NS
    assert fill.execunix == CLOCK_NS
    assert fill.prevuuid is None
    # The row states it, and a row read back keeps it.
    schema = fix_schema(seed)
    row = fill.into_row(schema)
    assert row.as_py()[schema.index_of("execunix")] == CLOCK_INSTANT
    assert FixMsg.from_row(schema, row, seed).execunix == CLOCK_NS
    # On a dated line, that instant is the line's clock.
    # A fill splits into its report and one execution sourced from it (A12);
    # both executed at the line's clock.
    dated, execution = list(codec.parse_text_line(_dated_line(b"8=FIX.4.4|35=8|150=F|39=2|10=0|")))
    assert dated.execunix == execution.execunix == RECORDED_NS
    # Read from the line and from the report it split out of.
    assert len(execution.srcuuids) == 2 and dated.curruuid in execution.srcuuids

    # A trade's own TransactTime is its execution clock.
    traded = codec.parse_fix_line(
        b"8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=2|60=20240102-10:15:30.5|10=0|"
    )
    assert traded.execunix == CLOCK_NS + 500_000_000

    # An acknowledgement and an order report no execution.
    acknowledged = codec.parse_fix_line(b"8=FIX.4.4|35=8|37=O1|17=E0|150=0|39=0|10=0|")
    assert acknowledged.execunix is None
    order = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|10=0|")
    assert order.execunix is None


def test_a_parse_fills_what_the_line_implied_and_leaves_the_wire_alone(seed: FixRegistry) -> None:
    """The rules run inside the parse: there is no enriching step of its own."""
    reader = _fixed(seed)

    # A `SecurityID` an ISIN's check digit closes has stated its source, and
    # under that source the ISIN the trait answers and the country.
    line = b"8=FIX.4.4|35=D|11=A|48=US0378331005|10=0|"
    filled = next(reader.parse_line(line))
    assert filled.by_tag(22).as_py() == "4"
    # A US ISIN embeds the CUSIP, derived beside it under its own source,
    # which fills the CUSIP's base key.
    assert _kinds(filled.securityids) == {"cusip": "037833100", "isin": "US0378331005"}
    assert [str(id) for id in filled.securityids] == [
        "cusip=037833100",
        "derived:cusip=037833100",
        "isin=US0378331005",
    ]
    assert filled.securityids.is_derived("cusip") and not filled.securityids.is_derived("isin")
    assert filled.by_tag(470).as_py() == "US"
    # An order stating no time in force is a day order.
    assert filled.by_tag(59).as_py() == "0"
    # The wire is the message as it now stands: the header, the lifted
    # `ClOrdID`, what the line carried with the pairs the dictionary derived
    # from it, then the trailer.
    assert filled.into_bytes(ord("|")) == (
        b"8=FIX.4.4|35=D|11=A|48=US0378331005|22=4|59=0|470=US|10=0|"
    )

    # A value no standard closes answers nothing rather than a guess.
    opaque = next(reader.parse_line(b"8=FIX.4.4|35=D|11=A|48=HIGH_TOUCH|10=0|"))
    assert opaque.get_by_tag(22) is None
    assert _kinds(opaque.securityids) == {}

    # The names a message goes by are read off the fields that state them -
    # `OrderID(37)`, `ClOrdID(11)`, `ExecID(17)` - each under its key.
    report = reader.parse_fix_line(b"8=FIX.4.4|35=8|37=O-01|11=C-001|17=E-09|10=0|")
    assert _kinds(report.identifiers) == {"clordid": "C-001", "execid": "E-09", "orderid": "O-01"}
    # The fields state them whatever the message type, known or not.
    unknown = reader.parse_fix_line(b"8=FIX.4.4|35=ZZ|11=C-1|10=0|")
    assert _kinds(unknown.identifiers) == {"clordid": "C-1"}
    assert unknown.header().msgtype == "ZZ"
    # Each is keyed by its type alone - a FIX field states the base key, the
    # type the word its name folds to - and displays as such.
    assert [str(id) for id in report.identifiers] == [
        "clordid=C-001",
        "execid=E-09",
        "orderid=O-01",
    ]
    assert report.identifiers.get_from("orderid") == "O-01"
    assert report.identifiers.get_from("fix:orderid") == "O-01", "the standard's word reads as the base"


def test_party_ids_are_typed_by_role_and_sourced_by_their_id_source(seed: FixRegistry) -> None:
    """``Parties`` states ``partyids``: the role's name is the type, the source's the source."""
    reader = _fixed(seed)
    message = reader.parse_fix_line(
        b"8=FIX.4.4|35=D|11=C-1|41=C-0|1=ACC9|453=2|448=BRK|447=D|452=1|448=CL|447=N|452=3|10=0|"
    )
    assert [str(id) for id in message.partyids] == [
        "account=ACC9",
        "clientid=CL",
        "executingfirm=BRK",
        "legalentityidentifier:clientid=CL",
        "proprietary:executingfirm=BRK",
    ]
    assert message.partyids.get("executingfirm") == "BRK"
    assert message.partyids.get_from("proprietary:executingfirm") == "BRK"
    # `OrigClOrdID(41)` is the `origclordid` of the `clordid` it replaced: its
    # own key beside the `clordid`'s, no lineage held on the identifier.
    assert message.identifiers.get("clordid") == "C-1"
    assert message.identifiers.get_from("origclordid") == "C-0"
    assert [id.key for id in message.identifiers] == ["clordid", "origclordid"]
    # A party stating no role is typed `party`, a bare unknown role code is
    # `partyrole<code>`, and a source stating none is the base source.
    bare = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=C-2|453=2|448=P1|452=9999|448=P2|10=0|")
    assert [str(id) for id in bare.partyids] == ["party=P2", "partyrole9999=P1"]


def test_an_unmapped_key_naming_an_identifier_lands_in_the_set_its_type_belongs_to(seed: FixRegistry) -> None:
    """A key no dictionary resolves is read for the identifier name it ends with."""
    reader = _fixed(seed)
    message = reader.parse_fix_line(
        b"8=FIX.4.4|35=D|11=C1|VENUE_ClOrdID=X1|VENUE_UserID=U1|venue.InstrumentID=dbi;X|"
        b"UnderlyingISIN=CH0012214059|TransversalKey=K1|10=0|"
    )
    # An identifier the key's source and type name, a party for a role, a
    # security for a security type: each in its own map, keyed `src:type`.
    # A named source fills its type's base key only where nothing states it:
    # the wire's `ClOrdID` answers `clordid`.
    assert [str(id) for id in message.identifiers] == ["clordid=C1", "venue:clordid=X1"]
    assert [str(id) for id in message.partyids] == ["userid=U1", "venue:userid=U1"]
    assert [str(id) for id in message.securityids] == ["instrumentid=dbi;X", "venue:instrumentid=dbi;X"]
    # Another instrument's security (`underlyingisin`) and a name that is no
    # identifier's (`transversalkey`) are lifted nowhere.
    assert message.isincode is None
    sets = (message.identifiers, message.partyids, message.securityids)
    assert all(id.value not in ("K1", "CH0012214059") for ids in sets for id in ids)
    # The entries stay on the wire as they arrived, folded.
    wire = message.into_text("|")
    assert "venueclordid=X1" in wire and "underlyingisin=CH0012214059" in wire and "transversalkey=K1" in wire
    # A whole security name is that type from the base source, its value checked.
    isin = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=C1|security_isin=US0378331005|10=0|")
    assert isin.securityids.get_from("isin") == "US0378331005"
    # The market leaf lifts the same keys and keeps the rest in its metadata.
    [leaf] = message.market_data()
    order = leaf.as_order_event()
    assert order is not None
    assert order.identifiers.get_from("venue:clordid") == "X1"
    assert order.partyids.get_from("venue:userid") == "U1"
    assert order.securityids.get_from("venue:instrumentid") == "dbi;X"
    assert order.metadata == {"transversalkey": "K1", "underlyingisin": "CH0012214059"}


def test_a_message_reads_its_strike_price_off_the_dictionary_field(seed: FixRegistry) -> None:
    """``strikeprice`` is ``StrikePrice(202)``, read each call; no crate column holds it."""
    reader = _fixed(seed)
    stated = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|202=12.5|10=0|")
    assert stated.strikeprice is not None
    assert stated.strikeprice.as_py() == decimal.Decimal("12.5")
    assert stated.strikeprice == stated.by_tag(202) == stated.by_name("strikeprice")
    assert not hasattr(stated, "strikepx")
    assert reader.parse_fix_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|10=0|").strikeprice is None
    # A write to the field moves the answer, and `None` clears it.
    stated.set(202, "13")
    assert stated.strikeprice is not None and stated.strikeprice.as_py() == decimal.Decimal("13")
    stated.set(202, None)
    assert stated.strikeprice is None
    # The fixed row's instrument band carries the dictionary's own column.
    schema = fix_schema(seed)
    assert schema.index_of("strikepx") is None
    column = schema.field_by_path("strikeprice")
    assert column.fix.tag == 202 and column.dtype == DataType("decimal128(38, 18)")
    row = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|202=12.5|10=0|").into_row(schema).as_py()
    assert row[schema.index_of("strikeprice")] == decimal.Decimal("12.5")


def test_a_field_states_the_parents_of_the_identifier_it_names(seed: FixRegistry) -> None:
    """``FIX:parents``: the identifier types of an identifier's parents, nearest first."""
    field = Field("OrderID", "utf8")
    field.fix.tag = 37
    assert field.fix.parents == []
    field.fix.parents = ["ParentOrderID", "orig order id"]
    assert field.fix.parents == ["parentorderid", "origorderid"]
    assert field.metadata["FIX:parents"] == '["parentorderid","origorderid"]'
    # A spelling no identifier type folds from, or a type twice, leaves it as it was.
    for refused in (["café"], ["orderid", "orderid"], [""]):
        with pytest.raises(ValueError, match="FIX:parents"):
            field.fix.parents = refused
    assert field.fix.parents == ["parentorderid", "origorderid"]
    field.fix.parents = []
    assert "FIX:parents" not in field.metadata

    # The committed dictionary states `clordid`'s one parent, and the registry
    # answers any base: the stated list, else its name's (`orderid` has two).
    assert seed.parents_of("clordid") == ["origclordid"]
    assert seed.parents_of("orderid") == ["parentorderid", "origorderid"]
    assert seed.parents_of("tradeid") == ["origtradeid"]
    assert seed.parents_of("parentorderid") == [], "a parent type has none"
    assert seed.parent_of("origclordid") == ("clordid", 0)
    assert seed.parent_of("parentorderid") == ("orderid", 0)
    assert seed.parent_of("origorderid") == ("orderid", 1)
    assert seed.parent_of("origtradeid") == ("tradeid", 0)
    assert seed.parent_of("clordid") is None and seed.parent_of("originalorderid") is None
    with pytest.raises(ValueError, match="identifier"):
        seed.parents_of("café")

    # A registry's fields name each other's parents as they arrive.
    def declared(name: str, tag: int) -> Field:
        held = Field(name, "utf8")
        held.fix.tag = tag
        return held

    venue = FixRegistry.from_fields([declared("OrderID", 37), declared("OrigOrderID", 9001)])
    assert venue.get_field_by_tag(37).fix.parents == ["origorderid"]
    assert {"base": "orderid", "parents": ["origorderid"]} in venue.parent_sources()
    venue.insert(declared("ParentOrderID", 9002))
    assert venue.get_field_by_tag(37).fix.parents == ["parentorderid", "origorderid"]
    assert venue.parent_of("origorderid") == ("orderid", 1)


REPORT = (
    b"8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|"
    b"14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|"
)


def test_a_parse_restates_deprecated_fields_to_their_latest_aliases(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    latest = next(codec.parse_line(REPORT))

    # ExecType PartiallyFilled is restated as Trade; Rule80A A is an agency
    # order; ExecBroker and ClientID are two parties, in tag order, the
    # group's length its count.
    assert latest.by_tag(150).as_py() == "F"
    assert latest.by_tag(528).as_py() == "A"
    assert len(latest.by_name("parties").as_py()) == 2
    assert latest.by_path("parties[0].partyid").as_py() == "BRKR"
    assert latest.by_path("parties[1].partyid").as_py() == "CLIENT1"
    # The fill under its newest spelling, reachable by the old one too.
    assert latest.by_tag(32).as_py() == 100.0
    assert latest.by_name("LastShares").as_py() == 100.0
    # `LastQty` is the quantity the event last traded, so it is a typed fact
    # and no longer a child of the content row.
    assert [name for name, _ in latest].count("lastqty") == 0
    assert latest.lastqty is not None and latest.lastqty.as_py() == 100
    # The state the event reached, as the crate's lifecycle-sorted member.
    assert latest.state is State.PARTIALLY_FILLED
    # And the rules the dictionary licenses: one fill's average is its price.
    assert latest.by_tag(6).as_py() == 10.5
    # The version it was read at is the header's, not a column.
    assert latest.header().beginstring == "FIX.4.2"


# The messages of one order's life: the order, its acknowledgement, a partial
# fill, a replace whose new identifier names the old one, its acknowledgement
# and the fill under the new identifier alone.
LIFE = [
    b"8=FIX.4.4|35=D|52=20260102-10:15:30.000|11=A1|55=AAPL|207=XNAS|15=USD|54=1|38=100|44=12.5|10=0|",
    b"8=FIX.4.4|35=8|52=20260102-10:15:30.250|11=A1|37=O1|17=E1|150=0|39=0|55=AAPL|207=XNAS|15=USD|38=100|14=0|151=100|10=0|",
    b"8=FIX.4.4|35=8|52=20260102-10:15:31.000|11=A1|37=O1|17=E2|150=F|39=1|55=AAPL|207=XNAS|15=USD|38=100|14=50|151=50|32=50|31=12.5|10=0|",
]


def test_a_message_read_from_a_line_states_the_line_as_its_one_source(seed: FixRegistry) -> None:
    """Provenance: the line an event was parsed out of, never the chain."""
    codec = _fixed(seed)
    lines = [TextLine(at, body) for at, body in enumerate(LIFE)]
    # A line is an event of its own, and the message read from it names it.
    [message] = list(codec.parse_text_line(lines[0]))
    assert message.srcuuids == [lines[0].curruuid]
    assert message.srcuuids == [lines[0].curruuid]
    # The source is no part of the code: the same bytes are the same message.
    [raw] = list(codec.parse_lines(LIFE[:1]))
    assert raw.srcuuids == []
    assert raw.curruuid == message.curruuid
    assert raw.currhashcode == message.currhashcode
    # A walk links the chain by predecessor and leaves every source its own.
    parsed = list(codec.parse_text_lines(lines))
    walked = list(codec.lifecycle(parsed))
    assert [held.srcuuids for held in walked[:3]] == [[line.curruuid] for line in lines]
    assert walked[2].prevuuid == walked[1].curruuid
    # The fill's execution was read from the line and from the report it
    # split out of, as the parse identified it (A12), and starts a chain of
    # its own.
    execution = walked[3]
    assert execution.state is State.FILLED and execution.prevuuid is None
    assert {held.as_py() for held in execution.srcuuids} == {
        lines[2].curruuid.as_py(),
        parsed[2].curruuid.as_py(),
    }


def test_a_message_split_off_one_of_three_twins_names_its_own_report(seed: FixRegistry) -> None:
    """Twins at one instant: each split execution names the report it came from."""
    codec = _fixed(seed)
    # Three identical fill reports share one identity before their places;
    # each execution the parse splits off names its own report, never an
    # earlier twin.
    fill = b"8=FIX.4.4|35=8|52=20260102-10:15:30.000|37=ORD-1|17=E-1|150=F|39=2|54=1|55=AAPL|32=5|31=10|10=0|"
    parsed = list(codec.parse_lines([fill, fill, fill]))
    assert [message.seqnum for message in parsed] == [0, 1, 2, 3, 4, 5]
    assert len({parsed[at].curruuid for at in (0, 2, 4)}) == 3
    for report, split in ((0, 1), (2, 3), (4, 5)):
        assert parsed[split].srcuuids == [parsed[report].curruuid]


def test_the_lifecycle_states_each_message_as_the_one_it_follows(seed: FixRegistry) -> None:
    """The one walk, over any iterable, lazily."""
    codec = _fixed(seed)
    parsed = list(codec.parse_lines(LIFE))
    # A parse chains nothing: every message stands alone, except the split
    # pair, which shares the fill's instant and takes the next place in it.
    assert [held.seqnum for held in parsed] == [0, 0, 0, 1]
    assert all(held.prevuuid is None for held in parsed)

    stream = codec.lifecycle(parsed)
    assert isinstance(stream, FixMessages)
    walked = list(stream)
    # The partial fill splits into its report and one execution (A12).
    assert len(walked) == len(LIFE) + 1

    # One chain: the acknowledgement and the report follow the order under
    # the cross identity its client identifier and side derive (A17); the
    # execution walks a chain of its own.
    order_chain, execution = walked[:3], walked[3]
    assert len({held.crossuuid.as_py() for held in order_chain}) == 1
    assert execution.crossuuid not in {held.crossuuid for held in order_chain}
    # Each later step in the order chain is a later instant, so it keeps its
    # own place; the execution shares the fill's instant and stands after it.
    assert [held.seqnum for held in walked] == [0, 0, 0, 1]
    assert walked[0].prevuuid is None and execution.prevuuid is None
    for earlier, later in zip(order_chain, order_chain[1:]):
        assert later.prevuuid == earlier.curruuid
        assert later.prevunix == earlier.currunix
    # The lifecycle's own creation instant is carried forward.
    assert {held.creaunix for held in order_chain} == {walked[0].creaunix}
    # The state moves with the messages, and an execution is filled (A14).
    assert [held.state for held in walked] == [
        State.PENDING_NEW,
        State.NEW,
        State.PARTIALLY_FILLED,
        State.FILLED,
    ]

    # The walk states what a message follows and restates no content but
    # what it lacks: a message that named no side takes the side of the order
    # it follows into its content - as it takes a missing order link - so its
    # row states the side its cross code carries. Every other entry is the
    # parse's own, and a message that stated its side is untouched.
    side = (54, "side", walked[0].side.fix_code, [])
    for before, after in zip(parsed, walked):
        if any(entry[0] == 54 for entry in before.entries()):
            assert after.entries() == before.entries()
            assert after.value == before.value
        else:
            added = [entry for entry in after.entries() if entry not in before.entries()]
            assert added in ([], [side]), added
            assert [entry for entry in after.entries() if entry != side] == before.entries()

    # A second walk over a walked stream changes nothing.
    assert list(codec.lifecycle(walked)) == walked

    # The walk sorts by instant, so a stream that arrived out of order is
    # chained in the order the messages happened in.
    reordered = list(codec.lifecycle([parsed[2], parsed[0], parsed[1]]))
    assert [held.currunix for held in reordered] == sorted(held.currunix for held in reordered)
    # Sorted, each is again a later instant than its predecessor, so each
    # keeps its own place.
    assert [held.seqnum for held in reordered] == [0, 0, 0]

    # A message naming no chain has an identity and no predecessor. A
    # `Heartbeat` is what the default refuses, so this reader is told to
    # read it.
    session = _fixed(seed, exclude_msgtypes=[])
    heartbeat = next(session.parse_line(b"8=FIX.4.4|35=0|34=7|52=20260102-10:15:30.000|10=0|"))
    (held,) = list(session.lifecycle(item for item in [heartbeat]))
    assert held.crosscode == ""
    assert held.crossuuid == held.curruuid
    assert held.prevuuid is None

    # An item that is not a message is refused where it is met.
    mixed = session.lifecycle([heartbeat, b"8=FIX.4.4|35=0|10=0|"])
    assert next(mixed) == held
    with pytest.raises(TypeError):
        next(mixed)
    assert next(mixed, None) is None


def test_party_id_source_alias_redirects_to_its_standard_code(seed: FixRegistry) -> None:
    message = _fixed(seed).parse_fix_line(
        b"8=FIX.4.4|35=D|11=P|453=1|448=BRK|447=proprietary/customcode|452=1|10=0|"
    )
    assert b"447=D" in message.into_bytes(124)


def test_figi_redirects_through_datatype_sources_and_the_fixed_row(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    schema = fix_schema(seed)
    for line in (
        b"8=FIX.4.4|35=D|11=P|22=S|48=BBG000BLNQ16|10=0|",
        b"8=FIX.4.4|35=D|11=A|454=1|455=BBG000BLNQ16|456=S|10=0|",
    ):
        message = codec.parse_fix_line(line)
        assert _kinds(message.securityids) == {"figi": "BBG000BLNQ16"}
        assert message.securityids == message.securityids
        assert FixMsg.from_row(schema, message.into_row(schema), seed).into_row(schema) == message.into_row(schema)
    bloomberg = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=B|22=A|48=AAPL US Equity|10=0|")
    assert _kinds(bloomberg.securityids) == {"bloomberg": "AAPL US Equity"}
    direct = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=X|10=0|")
    direct.set(FIGICODE_TAG, "BBG000BLNQ16")
    assert _kinds(direct.securityids) == {"figi": "BBG000BLNQ16"}


def test_a_security_source_reads_by_its_name_and_an_unknown_one_is_kept(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    by_code = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|22=4|48=US0378331005|10=0|")
    for name in (b"ISIN number", b"isin"):
        by_name = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|22=" + name + b"|48=US0378331005|10=0|")
        assert _kinds(by_name.securityids) == _kinds(by_code.securityids)
        # A derivation reads the name as the code: the ISIN's country.
        assert by_name.get(470) == by_code.get(470)
    private = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|22=100|48=HOUSE-1|10=0|")
    assert private.securityids.get_from("100") == "HOUSE-1"
    refused = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|22=House/Key|48=HK-1|10=0|")
    assert refused.securityids.get_from("housekey") is None
    assert "securityid" in [field for field, _ in refused.anomalies]


def test_official_time_delay_bounds_which_clock_dates_the_message(seed: FixRegistry) -> None:
    """Python forwards the parse's official-clock reading and its one pin."""
    assert FixCodec(seed).official_time_delay_ms == 1_000
    assert FixCodec(seed, official_time_delay_ms=None).official_time_delay_ms == 1_000
    assert FixCodec(seed, official_time_delay_ms=0).official_time_delay_ms == 0
    assert FixCodec(seed, official_time_delay_ms=-1).official_time_delay_ms == -1

    codec = FixCodec(seed, exclude_msgtypes=[])
    sending = 1_787_308_200_415_000_000
    # A transaction half a second in front of the sending clock is the same
    # event said twice, so the more exact saying of it dates the message.
    near = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|11=A|10=0|"
    )
    assert near.currunix == 1_787_308_199_900_000_000
    assert near.creaunix == near.currunix
    # Five seconds out is a different event of the session's day.
    apart = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:55|11=A|10=0|"
    )
    assert apart.currunix == sending

    # A message stating no transaction is dated by the regulatory stamp its
    # `TrdRegTimestampType(770)` says is about the event; the nearer stamp is
    # when the report reached a repository, which is not that.
    stamped = codec.parse_fix_line(
        b"8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=2|"
        b"769=20260821-10:30:00.400|770=23|769=20260821-10:29:59.900|770=1|10=0|"
    )
    assert stamped.currunix == 1_787_308_199_900_000_000
    # With only the unranked stamp, the one clock every message carries keeps it.
    unranked = codec.parse_fix_line(
        b"8=FIX.4.4|35=AE|52=20260821-10:30:00.415|768=1|"
        b"769=20260821-10:30:00.400|770=23|10=0|"
    )
    assert unranked.currunix == sending

    # The pin is the caller's to widen and to close.
    wide = FixCodec(seed, exclude_msgtypes=[], official_time_delay_ms=10_000)
    assert wide.parse_fix_line(
        b"8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:55|11=A|10=0|"
    ).currunix == 1_787_308_195_000_000_000
    shut = FixCodec(seed, exclude_msgtypes=[], official_time_delay_ms=0)
    assert shut.parse_fix_line(
        b"8=FIX.4.4|35=D|52=20260821-10:30:00.415|60=20260821-10:29:59.900|11=A|10=0|"
    ).currunix == sending


def test_lifecycle_learns_a_later_missing_instrument_code(seed: FixRegistry) -> None:
    """The core's contract: a later message lacking a code learns it in event order."""
    codec = _fixed(seed)
    later = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:31|11=LATER|isincode=US0378331005|10=0|"
    )
    earlier = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=EARLIER|isincode=US0378331005|bloombergcode=AAPL US Equity|10=0|"
    )
    learned = list(codec.lifecycle([later, earlier]))
    assert [held.header().msgseqnum for held in learned] == [1, 2]
    assert learned[1].securityids.get("bloomberg") == "AAPL US Equity"
    assert not later.securityids.contains_kind("bloomberg")
    schema = fix_schema(seed)
    rebuilt = FixMsg.from_row(schema, learned[1].into_row(schema), seed)
    assert rebuilt.securityids.get("bloomberg") == "AAPL US Equity"


# Two chains in instant order across four epoch hours: the second order's
# acknowledgement and fill each land hours after it.
HOURS = [
    *LIFE,
    b"8=FIX.4.4|35=D|52=20260102-11:40:00.000|11=B1|55=AAPL|207=XNAS|15=USD|54=2|38=10|44=13|10=0|",
    b"8=FIX.4.4|35=8|52=20260102-12:05:00.000|11=B1|37=O2|17=E3|150=0|39=0|55=AAPL|207=XNAS|15=USD|38=10|14=0|151=10|10=0|",
    b"8=FIX.4.4|35=8|52=20260102-14:30:00.000|11=B1|37=O2|17=E4|150=F|39=2|55=AAPL|207=XNAS|15=USD|38=10|14=10|151=0|32=10|31=13|10=0|",
]


def test_a_sorted_lifecycle_walks_one_hour_at_a_time_as_the_whole_capture(seed: FixRegistry) -> None:
    """A capture in instant order walks alike held one hour at a time."""
    codec = _fixed(seed)
    messages = sorted(codec.parse_lines(HOURS), key=lambda held: held.currunix)

    def walk(codec: FixCodec) -> list[tuple[Any, int, Any]]:
        return [(held.curruuid, held.seqnum, held.prevuuid) for held in codec.lifecycle(messages)]

    whole = walk(codec)
    # Each fill splits into its report and one execution (A12).
    assert len(whole) == len(HOURS) + 2
    # Every step across the hours is a later instant and keeps its own
    # place; only a fill's split pair ever shares one.
    assert max(seqnum for _, seqnum, _ in whole) == 1
    assert walk(codec.with_sorted_lifecycle(True)) == whole
    assert walk(_fixed(seed, sorted_lifecycle=True)) == whole


def test_the_lifecycle_hands_the_expirations_of_an_instant_over_first(seed: FixRegistry) -> None:
    """Expirations sharing a deadline take its first places; what arrives there follows."""
    codec = _fixed(seed)
    messages = codec.parse_lines(
        [
            b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=E1|55=AAPL|54=1|38=100|10=0|",
            b"8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:31|126=20260102-10:15:32|11=E2|55=AAPL|54=1|38=100|10=0|",
            b"8=FIX.4.4|35=D|49=S|56=T|34=3|52=20260102-10:15:32|11=S1|55=IBM|54=1|38=100|10=0|",
        ]
    )
    walked = list(codec.lifecycle(messages))
    deadline = [held for held in walked if held.currunix == walked[-1].currunix]
    assert [(held.state, held.seqnum) for held in deadline] == [
        (State.EXPIRED, 0),
        (State.EXPIRED, 1),
        (State.PENDING_NEW, 2),
    ]
    # What the walk made keeps its place, so the walk answers itself.
    again = list(codec.lifecycle(walked))
    assert [(held.curruuid, held.seqnum) for held in again] == [(held.curruuid, held.seqnum) for held in walked]


def test_the_lifecycle_yields_an_identity_once_within_its_dedup_window(seed: FixRegistry) -> None:
    """The window is the core's pin: one minute unless given, None clearing it."""
    codec = _fixed(seed)
    assert codec.dedup_window_ms == 60_000
    assert FixCodec(seed).dedup_window_ms == 60_000
    assert FixCodec(seed, dedup_window_ms=5_000).dedup_window_ms == 5_000
    assert FixCodec(seed, dedup_window_ms=None).dedup_window_ms is None
    assert FixCodec(seed, dedup_window_ms=0).dedup_window_ms is None
    assert FixCodec(seed, dedup_window_ms=-1).dedup_window_ms is None
    assert FixCodec.from_env(dedup_window_ms=None).dedup_window_ms is None
    # Set on a codec in hand, it keeps every other setting.
    wide = codec.with_sorted_lifecycle(True).with_dedup_window_ms(7_200_000)
    assert (wide.dedup_window_ms, wide.sorted_lifecycle) == (7_200_000, True)
    assert wide.exclude_msgtypes == codec.exclude_msgtypes
    assert wide.with_dedup_window_ms(None).dedup_window_ms is None

    def order(sequence: int, clordid: str) -> FixMsg:
        return codec.parse_fix_line(
            f"8=FIX.4.4|35=D|49=S|56=T|34={sequence}|52=20260102-10:15:30|11={clordid}|55=AAPL|54=1|38=100|10=0|".encode()
        )

    # The order delivered again under another sequence restates the live
    # order, under its identity, after another order rather than beside it.
    messages = [order(1, "A1"), order(2, "B1"), order(3, "A1")]
    every = list(codec.with_dedup_window_ms(None).lifecycle(messages))
    assert len(every) == 3
    assert every[2].curruuid == every[0].curruuid
    once = list(codec.lifecycle(messages))
    assert [held.curruuid for held in once] == [held.curruuid for held in every[:2]]


def test_lifecycle_redirects_categories_snapshots_expiry_dedup_and_learning(seed: FixRegistry) -> None:
    """Python forwards the settled lifecycle contract without changing sources."""
    intrinsic = FixRegistry()
    with pytest.raises(ValueError, match="fixed MsgCat operation identifiers"):
        intrinsic.set_codeset("msgcatcodeset", [])
    with pytest.raises(ValueError, match="fixed MsgCat operation identifiers"):
        intrinsic.merge_codeset("msgcatcodeset", [{"value": "99", "name": "ORDR"}])
    with pytest.raises(ValueError, match="fixed MsgCat operation identifiers"):
        intrinsic.remove_codeset("msgcatcodeset")

    assert FixCodec(seed).snapshot_ns is None
    assert FixCodec(seed, snapshot_ns=None).snapshot_ns is None
    assert FixCodec(seed, snapshot_ns=0).snapshot_ns is None
    assert FixCodec(seed, snapshot_ns=-1).snapshot_ns is None

    codec = _fixed(seed)
    snapshot_codec = _fixed(seed, snapshot_ns=1_000_000_000)
    assert snapshot_codec.snapshot_ns == 1_000_000_000
    # A grid set on a codec in hand keeps every other setting of it.
    regridded = codec.with_snapshot_ns(3_600_000_000_000)
    assert regridded.snapshot_ns == 3_600_000_000_000
    assert regridded.default_sending_time == codec.default_sending_time
    assert regridded.exclude_msgtypes == codec.exclude_msgtypes
    assert codec.snapshot_ns is None
    assert snapshot_codec.with_snapshot_ns(None).snapshot_ns is None
    # The sorted pin is off by default, set by keyword or on a codec in hand,
    # and keeps every other setting.
    assert codec.sorted_lifecycle is False
    assert FixCodec(seed, sorted_lifecycle=True).sorted_lifecycle is True
    assert FixCodec.from_env(sorted_lifecycle=True).sorted_lifecycle is True
    sorted_codec = snapshot_codec.with_sorted_lifecycle(True)
    assert sorted_codec.sorted_lifecycle is True
    assert sorted_codec.snapshot_ns == snapshot_codec.snapshot_ns
    assert sorted_codec.default_sending_time == snapshot_codec.default_sending_time
    assert snapshot_codec.sorted_lifecycle is False
    assert sorted_codec.with_sorted_lifecycle(False).sorted_lifecycle is False
    assert seed.msgtype("D").msgcat is MarketDataKind.ORDR

    original = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=7|52=20260102-10:15:30|11=REPLAY-1|55=AAPL|10=0|"
    )
    replay = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=7|43=Y|52=20260102-10:15:31|122=20260102-10:15:30|11=REPLAY-1|55=AAPL|10=0|"
    )
    distinct = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=8|52=20260102-10:15:32|11=REPLAY-1|55=AAPL|10=0|"
    )
    assert original.msgcat is MarketDataKind.ORDR and original.msgcat == 10
    deduplicated = list(codec.lifecycle([original, copy.copy(original), replay, distinct]))
    assert [held.header().msgseqnum for held in deduplicated] == [7, 8]
    # The twin merges into the live message; `distinct`, a later instant,
    # keeps its own place too.
    assert [held.seqnum for held in deduplicated] == [0, 0]

    later = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:31|11=LATER|isincode=US0378331005|10=0|"
    )
    earlier = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=EARLIER|isincode=US0378331005|bloombergcode=AAPL US Equity|10=0|"
    )
    # The walk orders by event time and leaves its inputs alone; what a
    # later message learns from an earlier one is its own test below.
    assert not later.securityids.contains_kind("bloomberg")
    learned = list(codec.lifecycle([later, earlier]))
    assert [held.header().msgseqnum for held in learned] == [1, 2]
    assert _kinds(learned[0].securityids) == {
        "bloomberg": "AAPL US Equity",
        "cusip": "037833100",
        "isin": "US0378331005",
    }
    assert not later.securityids.contains_kind("bloomberg")

    expiring = snapshot_codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=EXP-1|55=AAPL|10=0|"
    )
    entries = expiring.entries()
    deadline = expiring.exprunix
    assert deadline is not None
    walked = list(snapshot_codec.lifecycle([expiring]))
    assert expiring.entries() == entries
    assert expiring.snapunix is None
    expired = next(
        held
        for held in walked
        if held.snapunix is None and held.currunix == deadline
    )
    assert expired.entries() == entries
    snapshots = [held for held in walked if held.snapunix is not None]
    assert snapshots
    assert expired.state is State.EXPIRED
    [live] = [held for held in walked if held.snapunix is None and held.currunix < deadline]
    # A view is the live message as of its tick: dated at it, so its identity
    # is the one that tick derives, while its content, its place and its cross
    # element are the live message's, and its snapshot instant the one the
    # live message was stated at.
    assert [held.currunix for held in snapshots] == [
        live.currunix,
        live.currunix + 1_000_000_000,
    ]
    for held in snapshots:
        assert held.snapunix == live.currunix
        snapunix = held.currunix
        assert snapunix < deadline
        assert held.currhashcode == live.currhashcode
        assert (held.seqnum, held.prevuuid, held.crossuuid) == (
            live.seqnum,
            live.prevuuid,
            live.crossuuid,
        )
        assert (held.curruuid == live.curruuid) == (snapunix == live.currunix)


def test_the_fixed_row_is_named_by_fold_and_never_shifts(seed: FixRegistry) -> None:
    """The one shape a whole capture lands in."""
    schema = fix_schema(seed, "FixMessage")
    columns = [child.name for child in schema]
    assert schema.name == "FixMessage"
    # Every tag the listing names is some column's identity, and a group's
    # counter names the group beside it, so there are columns the listing
    # does not repeat.
    held = {child.fix.tag for child in schema} | {child.fix.counter for child in schema}
    assert set(fix_schema_tags()) <= held
    assert len(columns) >= len(fix_schema_tags())

    # The six identities every element states open the row, the event's
    # clocks follow - a table is read by time - and the one arrival record
    # closes it.
    assert columns[:7] == [
        "curruuid",
        "crossuuid",
        "crosscode",
        "currhashcode",
        "crosshashcode",
        "srcuuids",
        "currunix",
    ]
    # `fixentries` is a tag:name map that counts itself; no counter column (A8).
    assert columns[-1] == "fixentries" and columns.count("metadata") == 1
    assert columns.count("msgdirection") == 1

    # A column is found by the name the dictionary spells, never by digits.
    assert schema.index_of("msgtype") is not None
    assert schema.index_of("35") is None
    # The tag stays the identity: each column carries its field's.
    at = schema.index_of("msgtype")
    assert at is not None
    assert columns[at - 1 : at + 3] == ["beginstring", "msgtype", "msgseqnum", "sendercompid"]
    assert fix_schema_tags()[at] == 35
    assert schema[at].fix.tag == 35

    # The columns every message settles are the non-null ones; which band
    # each falls in is the core's to order.
    assert {child.name for child in schema if not child.nullable} == {
        "currunix",
        "creaunix",
        "currhashcode",
        "crosshashcode",
        "curruuid",
        "crossuuid",
        "beginstring",
    }
    # The facts a row derives are typed as the thing they hold.
    assert schema[schema.index_of("securityexchange")].dtype == DataType("mic")
    assert schema[schema.index_of("price")].dtype == DataType("decimal128(38, 18)")
    assert schema[schema.index_of("curruuid")].dtype == DataType("uuid")
    assert schema[schema.index_of("metadata")].fix.counter == METADATA_TAG

    # The session event a bridge delivered the message as closes the session
    # band it is joined from.
    at = schema.index_of("msgsessionid")
    assert at is not None
    assert columns[at + 1] == "msgsesseventid"
    assert schema[at + 1].fix.tag == MSGSESSEVENTID_TAG

    # The retired columns are gone rather than renamed.
    for retired in (
        "updatedat",
        "createdat",
        "msghash",
        "msgphash",
        "code",
        "version",
        "refrecdunix",
        "altids",
        "secaltids",
        "parties",
        "cusipcode",
        "sedolcode",
    ):
        assert schema.index_of(retired) is None, retired

    reader = _fixed(seed)
    wire = b"8=FIX.4.4|35=D|11=A|52=20240102-10:15:30|9999=x|VenueOwnThing=y|10=0|"
    message = next(reader.parse_line(wire))
    row = message.into_row(schema).as_py()
    assert len(row) == len(columns)
    assert row[schema.index_of("beginstring")] == "FIX.4.4"
    assert row[schema.index_of("msgtype")] == "D"
    assert row[schema.index_of("marketdatakind")] == 10
    assert row[schema.index_of("clordid")] == "A"
    assert row[schema.index_of("currunix")] == CLOCK_INSTANT
    assert row[schema.index_of("creaunix")] == CLOCK_INSTANT
    assert row[schema.index_of("sendingtime")] == CLOCK_INSTANT
    assert row[schema.index_of("crosscode")] == "10:0:A"
    assert row[schema.index_of("currhashcode")] == message.currhashcode
    assert str(row[schema.index_of("curruuid")]) == message.curruuid.as_py()
    # A read is not a snapshot, and a fact the message gave nothing for is
    # null rather than a shift.
    assert row[schema.index_of("snapunix")] is None
    assert row[schema.index_of("ordstatus")] is None
    assert row[schema.index_of("prevunix")] is None
    assert row[schema.index_of("msgsessionid")] is None
    assert schema.index_of("nofixentries") is None

    # The residual map holds only names no projected column represents, and
    # only names a dictionary resolves (A8): `ClOrdID(11)` is lifted and
    # `TimeInForce(59)` is projected, so neither reaches it, and the two
    # unknown names are the row's metadata under their own spelling (A9).
    assert row[schema.index_of("fixentries")] == {}
    assert sorted(dict(row[schema.index_of("metadata")])) == ["9999", "venueownthing"]
    assert FixMsg.from_row(schema, row, seed).into_row(schema).as_py() == row


def test_a_captures_own_columns_follow_the_shared_ones(seed: FixRegistry) -> None:
    """Where a line was read from is what a monitor orders and joins on."""
    carrier = Field(
        "line",
        DataType.from_fields([Field("url", DataType("utf8")), Field("body", DataType("binary"))]),
        nullable=False,
    )
    plain = fix_schema(seed, "FixMessage")
    carried = fix_schema_carrying(carrier, plain)

    after = plain.index_of("partyids") + 1
    assert [child.name for child in carried][0] == "curruuid"
    assert [child.name for child in carried][after : after + 2] == ["url", "body"]
    assert len(carried) == len(plain) + 2
    assert carried.index_of("msgtype") == plain.index_of("msgtype") + 2

    # A column no tag names is the capture's, so a row answers null there.
    message = next(_fixed(seed).parse_line(b"8=FIX.4.4|35=D|10=0|"))
    row = message.into_row(carried).as_py()
    assert row[after] is None
    assert row[carried.index_of("msgtype")] == "D"

    # A capture column is led nullable however the carrier declared it,
    # because only a pass holding the source row can say where a line came
    # from: a required one is carried as nullable and the projection answers
    # null there rather than refusing.
    strict = fix_schema_carrying(
        Field(
            "line",
            DataType.from_fields([Field("url", DataType("utf8"), nullable=False)]),
            nullable=False,
        ),
        plain,
    )
    assert strict[after].nullable
    assert message.into_row(strict).as_py()[after] is None

    # A capture column whose folded name a FIX column takes is not carried in
    # front: `msgSessionId` and `msgsessionid` are one name.
    named = Field(
        "line",
        DataType.from_fields(
            [
                Field("url", DataType("utf8"), nullable=False),
                Field("msgSessionId", DataType("utf8")),
                Field("body", DataType("binary"), nullable=False),
            ]
        ),
        nullable=False,
    )
    folded = fix_schema_carrying(named, plain)
    names = [child.name for child in folded]
    after = plain.index_of("partyids") + 1
    assert names[after : after + 2] == ["url", "body"]
    assert "msgSessionId" not in names
    assert names.count("msgsessionid") == 1
    assert len(folded) == len(plain) + 2


def test_a_rows_own_columns_feed_the_message(seed: FixRegistry) -> None:
    """A capture's clock is context; its other columns fill what they name."""
    clock = dt.datetime(2026, 1, 2, 10, 15, 30, 500000, tzinfo=dt.timezone.utc)
    source = pa.table(
        {
            "timestamp": pa.array([clock, None], pa.timestamp("us", tz="UTC")),
            "msgsessionid": pa.array(["e7254b22", None], pa.utf8()),
            "msgseqnum": pa.array([4507, None], pa.int64()),
            "body": pa.array(
                [
                    b"8=FIX.4.4|35=D|55=AAPL|10=0|",
                    b"8=FIX.4.4|35=0|34=696|52=20260102-09:29:59.250|10=0|",
                ],
                pa.binary(),
            ),
        }
    )
    # The second line is a `Heartbeat`, which the default refuses, and this
    # case is about what a capture's own columns fill: read every type.
    parsed = _fixed(seed, exclude_msgtypes=[]).parse_text_arrow_reader(source).read_all()
    names = parsed.schema.names

    # A capture column whose folded name a fixed column takes fills that
    # column instead of riding beside the shared ones; no fixed column is
    # named `timestamp`, so the capture's clock is carried as context.
    assert names[0] == "curruuid"
    assert names.index("body") == names.index("timestamp") + 1
    assert names.index("timestamp") > names.index("partyids")
    for once in ("timestamp", "msgsessionid", "msgseqnum"):
        assert names.count(once) == 1, once
    # `fixentries` is a tag:name map that counts itself; no counter column (A8).
    assert names[-1] == "fixentries" and names.count("metadata") == 1

    # The capture's clock stamps nothing: the wire's own clock settles the
    # message, else - with no `currunix` column dating the row's line - the
    # codec's default SendingTime.
    instant = dt.datetime(2026, 1, 2, 9, 29, 59, 250000, tzinfo=dt.timezone.utc)
    assert parsed.column("timestamp").to_pylist() == [clock, None]
    assert parsed.column("currunix").to_pylist() == [CLOCK_INSTANT, instant]
    # Only a stated SendingTime lands in its column: the first row settled
    # on the codec's default and states none of its own.
    assert parsed.column("sendingtime").to_pylist() == [None, instant]
    assert parsed.column("snapunix").to_pylist() == [None, None]
    # A column spelled for a field fills it where the frame stated none.
    assert parsed.column("msgsessionid").to_pylist() == ["e7254b22", None]
    assert parsed.column("msgseqnum").to_pylist() == [4507, 696]
    # `Symbol(55)` and derived `TimeInForce(59)` are projected columns, so
    # neither is staged in the residual record.
    assert [[entry["tag"] for entry in row] for row in parsed.column("fixentries").to_pylist()] == [
        [],
        [],
    ]


def test_a_rows_msgpluginid_fills_its_field_and_selects_nothing(seed: FixRegistry) -> None:
    """A row's `msgpluginid` fills the capture's own fact; one namespace."""
    seed.insert(_field("VenueTag", "utf8", 5001, branches=["venue"]))
    codec = _fixed(seed)
    body = b"MSGTYPE=D|CLORDID=A|VENUETAG=dark"
    spellings = ["venue", "OMS_X1_TradeCapture", None]
    source = pa.table(
        {
            "msgpluginid": pa.array(spellings, pa.utf8()),
            "body": pa.array([body] * len(spellings), pa.binary()),
        }
    )
    parsed = codec.parse_text_arrow_reader(source).read_all()
    assert parsed.schema.names[0] == "curruuid"
    assert parsed.schema.names.count("body") == 1
    assert parsed.schema.names.count("msgpluginid") == 1
    assert parsed.column("msgpluginid").to_pylist() == spellings
    # The plugin selects nothing: the venue key remains residual because the
    # fixed schema has no column for it; derived TimeInForce is projected.
    for row in parsed.column("fixentries").to_pylist():
        assert [key for key, _ in row] == ["5001:VenueTag"]

    # One line read alone answers exactly what the batch did, and the plugin
    # is a fact about the capture rather than an entry.
    lined = _fixed(seed, capture_names=["msgpluginid"])
    for spelled in spellings:
        message = next(lined.parse_text_line(TextLine(0, body, [spelled])))
        assert message.by_tag(5001).as_py() == "dark", spelled
        assert message.capture().msgpluginid == spelled, spelled
        assert all(tag != MSGPLUGINID_TAG for tag, _, _, _ in message.entries()), spelled

    # A stated `msgdirection` column is the row's direction, read once and
    # carried by the header.
    # The second row is a bridge frame stating no type, which the default
    # refuses, and this case is about the direction column.
    spoken = (
        _fixed(seed, capture_names=["msgpluginid"], exclude_msgtypes=[])
        .parse_text_arrow_reader(
            pa.table(
                {
                    "msgdirection": pa.array(["Send", "R"], pa.utf8()),
                    "body": pa.array([b"MSGTYPE=D|CLORDID=A", b"|#SYMBOL=TTF|"], pa.binary()),
                }
            )
        )
        .read_all()
    )
    assert spoken.schema.names.count("msgdirection") == 1
    assert spoken.column("msgdirection").to_pylist() == ["S", "R"]


PROPERTIES = [
    ("counter", 453, "453"),
    ("component", "Party", "party"),
    ("field_ref", "PartyID", "partyid"),
    ("group", "Parties", "parties"),
    ("msgtype", "P Report Ack", "P Report Ack"),
    ("msgcat", "ORDR", "ORDR"),
]


@pytest.mark.parametrize("property_name,value,stored", PROPERTIES)
def test_reference_properties_share_metadata_and_remove_with_none(
    property_name: str, value: object, stored: str
) -> None:
    field = Field("probe", "utf8")
    view = field.fix
    key = f"FIX:{'field' if property_name == 'field_ref' else property_name}"
    assert getattr(view, property_name) is None

    setattr(view, property_name, value)
    assert field.metadata[key] == stored
    expected = int(stored) if property_name == "counter" else stored
    assert getattr(field.fix, property_name) == expected
    assert Field.from_arrow(field.into_arrow()).metadata[key] == stored

    setattr(view, property_name, None)
    assert getattr(view, property_name) is None
    assert key not in field.metadata
    setattr(view, property_name, None)

    field.metadata[key] = stored
    assert getattr(view, property_name) == expected
    del view["field" if property_name == "field_ref" else property_name]
    assert getattr(view, property_name) is None
    assert key not in field.metadata


@pytest.mark.parametrize("property_name", ["component", "field_ref", "group"])
@pytest.mark.parametrize("invalid", ["", "..", "../party", "Party ID"])
def test_invalid_catalog_reference_preserves_the_complete_field(
    property_name: str, invalid: str
) -> None:
    field = Field("probe", "utf8", metadata={"source": "test"})
    setattr(field.fix, property_name, "Party")
    before = field.into_json()
    with pytest.raises(ValueError, match="catalog name"):
        setattr(field.fix, property_name, invalid)
    assert field.into_json() == before


# A counter is a positive tag: 0 is what an unresolved arrival records, so it
# is refused with a negative (``rust/tests/fix/entry.rs``).
@pytest.mark.parametrize(
    "invalid,error",
    [(True, TypeError), (1.5, TypeError), (2**31, OverflowError), (-1, ValueError), (0, ValueError)],
)
def test_counter_refuses_invalid_python_integers_atomically(
    invalid: object, error: type[Exception]
) -> None:
    field = Field("probe", "utf8")
    field.fix.counter = 453
    before = field.into_json()
    with pytest.raises(error):
        field.fix.counter = invalid
    assert field.into_json() == before


@pytest.mark.parametrize("invalid", ["", "D\x01", "D\n"])
def test_message_code_refuses_control_text_without_changing_metadata(invalid: str) -> None:
    field = Field("probe", "utf8")
    field.fix.msgtype = "BridgeReport"
    before = field.into_json()
    with pytest.raises(ValueError, match="message-code"):
        field.fix.msgtype = invalid
    assert field.into_json() == before


def test_message_category_refuses_invalid_text_without_changing_metadata() -> None:
    field = Field("probe", "utf8")
    field.fix.msgcat = "ORDR"
    before = field.into_json()
    with pytest.raises(ValueError):
        field.fix.msgcat = "order"
    assert field.into_json() == before


@pytest.mark.parametrize("property_name,value,stored", PROPERTIES)
def test_reference_properties_refuse_other_protocols_and_frozen_fields(
    property_name: str, value: object, stored: str
) -> None:
    field = Field("probe", "utf8")
    with pytest.raises(TypeError, match="fix property"):
        getattr(field.iceberg, property_name)
    with pytest.raises(TypeError, match="fix property"):
        setattr(field.iceberg, property_name, value)

    setattr(field.fix, property_name, value)
    root = Field("row", DataType.from_fields([field]), nullable=False)
    row_type = root.into_dataclass(name=f"FrozenFix{property_name}")
    frozen = row_type.into_field().dtype[0]
    before = frozen.into_json()
    for replacement in (value, None):
        with pytest.raises(TypeError, match="read-only"):
            setattr(frozen.fix, property_name, replacement)
        assert frozen.into_json() == before


def test_identifiers_resolve_python_iterables_into_canonical_member_order() -> None:
    client = Field("clordid", "utf8")
    client.fix.tag = 11
    client.fix.names = ["ClientOrder"]
    order = Field("orderid", "utf8")
    order.fix.tag = 37
    component = Field("order", DataType.from_fields([client, order]), nullable=False)
    view = component.fix
    assert view.identifiers == []
    view.identifiers = (name for name in ["37", "ClientOrder"])
    assert view.identifiers == ["clordid", "orderid"]
    assert component.metadata["FIX:identifiers"] == "clordid,orderid"
    assert Field.from_arrow(component.into_arrow()).fix.identifiers == view.identifiers
    view.identifiers = ("ORDERID",)
    assert view.identifiers == ["orderid"]
    view.identifiers = []
    assert view.identifiers == []
    assert "FIX:identifiers" not in component.metadata


@pytest.mark.parametrize(
    "invalid",
    [[""], ["absent"], ["nested"], ["nested.child"], ["clordid,orderid"], ["clordid", "11"]],
)
def test_identifier_refusals_leave_the_complete_field_unchanged(invalid: list[str]) -> None:
    client = Field("clordid", "utf8", metadata={"FIX:tag": "11"})
    nested = Field("nested", DataType.from_fields([Field("child", "utf8")]))
    component = Field("order", DataType.from_fields([client, nested]), nullable=False)
    component.fix.identifiers = ["clordid"]
    before = component.into_json()
    with pytest.raises(ValueError, match="order.fix:identifiers"):
        component.fix.identifiers = invalid
    assert component.into_json() == before


def test_identifiers_refuse_nontext_other_protocols_and_readonly_declarations() -> None:
    component = Field("order", DataType.from_fields([Field("clordid", "utf8")]), nullable=False)
    component.fix.identifiers = ["clordid"]
    before = component.into_json()
    for invalid in ("clordid", 11, ["clordid", 11]):
        with pytest.raises(TypeError):
            component.fix.identifiers = invalid  # type: ignore[assignment]
        assert component.into_json() == before

    def interrupted() -> Iterator[str]:
        yield "clordid"
        raise RuntimeError("identifier input failed")

    with pytest.raises(RuntimeError, match="identifier input failed"):
        component.fix.identifiers = interrupted()
    assert component.into_json() == before
    with pytest.raises(TypeError, match="fix property"):
        component.iceberg.identifiers
    with pytest.raises(TypeError, match="fix property"):
        component.iceberg.identifiers = ["clordid"]
    assert component.into_json() == before
    frozen = component.into_dataclass(name="IdentifierOrder").into_field()
    before = frozen.into_json()
    with pytest.raises(TypeError, match="read-only"):
        frozen.fix.identifiers = []
    assert frozen.into_json() == before


def test_the_bridge_row_header_is_the_crates_own_text_and_names_its_captures() -> None:
    """The constant crosses whole, so a caller never respells the expression."""
    options = yggdryl.TextOptions()
    options.rowheader = ULBRIDGE_ROWHEADER
    captures = options.source_field()
    names = [child.name for child in captures]
    assert names[-6:] == [
        "msgthreadid",
        "msgsessionid",
        "msgctxid",
        "msgseqnum",
        "msgpluginid",
        "loglevel",
    ]
    assert str(captures.field("msgseqnum").dtype) == "int64"
    # The clock is `mtime`, consumed into each line's `currunix`, so it leads
    # no column of its own.
    assert "mtime" not in names and "timestamp" not in names


def test_the_bridge_row_header_dates_each_line_by_its_own_clock(tmp_path: pathlib.Path) -> None:
    """A file's modification time dates no line the header matched."""
    capture = tmp_path / "ulbridge.log"
    capture.write_bytes(ULBRIDGE_LOG.read_bytes())
    options = TextOptions()
    options.rowheader = ULBRIDGE_ROWHEADER
    options.timezone = "UTC"
    lines = list(IOBase(capture).read_text_lines(options=options))
    assert len(lines) == 144
    # `2026-08-14 14:46:39.769` in front of the first line, read in UTC.
    assert lines[0].currunix == 1_786_718_799_769_000_000
    assert lines[0].mtime == lines[0].currunix
    # One instant per clock the bridge wrote, none of them the file's.
    clocks = {line.split(" [", 1)[0] for line in ULBRIDGE_LOG.read_text(encoding="utf-8", errors="replace").splitlines() if line}
    dated = {line.currunix for line in lines}
    assert len(clocks) == 40
    assert len(dated) == len(clocks)
    assert int(capture.stat().st_mtime_ns) not in dated
    # A point or a comma opens the fraction, and a clock may state none.
    loose = IOBase.from_bytes(
        b"2026-08-14 14:46:39.769 [7] [P] (INFO) a\n"
        b"2026-08-14 14:46:39,769_123 [7] [P] (INFO) b\n"
        b"2026-08-14 14:46:40 [7] [P] (WARN) c\n"
    )
    assert [line.currunix for line in loose.read_text_lines(options=options)] == [
        1_786_718_799_769_000_000,
        1_786_718_799_769_123_000,
        1_786_718_800_000_000_000,
    ]
