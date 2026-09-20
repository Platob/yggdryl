"""The market boundary: the five products, their doors, their rows and the book reader.

Every answer here is the core's; what these check is the crossing - that a
product's facts cross exactly as ``MarketEventData`` crosses them and the
readings its trait provides cross as properties and methods, that a door
pulls a Python iterable one message at a time and answers a stream, that
the Arrow twins answer ``pyarrow`` readers of the same rows, that a row
round-trips through ``from_row``, that ``Symbol`` and ``BookIterator``
answer what the core answers over products from a door and from rows, that
the refusing writers raise ``ValueError`` with the core's sentence, and
that the bridge's own capture joins: every product's ``srcuuids`` are
among its messages' ``curruuid``.
"""

from __future__ import annotations

import copy
import datetime as dt
import decimal
import pathlib
import pickle
from typing import Any, Iterable

import pyarrow as pa
import pytest

from yggdryl import ArrowScalar, DataType, Field, IOBase, Scalar, TextOptions, Timezone
from yggdryl.fix import FixCodec, FixMessages, FixMsg, FixRegistry, MarketEventData, fix_schema
from yggdryl.market import (
    Book,
    BookIterator,
    Books,
    Execution,
    Executions,
    Order,
    Orders,
    Quote,
    Quotes,
    Statements,
    Symbol,
    Trade,
    Trades,
)

REPO = pathlib.Path(__file__).resolve().parent.parent.parent.parent
SEED = REPO / "config" / "fix"
# The capture, exactly as the bridge wrote it.
CAPTURE = REPO / "rust" / "tests" / "fix" / "ulbridge.log"

# The one intake clock undated test bytes take, so a parse repeats; replay
# never consults now.
CLOCK_NS = 1_704_190_530_000_000_000
CLOCK = DataType('datetime64(ns,"UTC")').scalar(CLOCK_NS)

# The bridge's own row header, as the core's `ULBRIDGE_ROWHEADER` spells it.
ULBRIDGE_ROWHEADER = (
    r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) "
    r"\[(?P<msgthreadid>[1-9]\d*)"
    r"(?:-(?P<msgsessionid>[0-9a-f]{8}):(?P<msgctxid>[0-9a-f]{10}):(?P<msgseqnum>\d+))?\] "
    r"\[(?P<msgpluginid>[^\]]+)\] \((?P<level>[A-Z]+)\) "
)

# One second per grid step, which is the whole capture.
STEP = 1_000_000_000

# The sixteen event columns every product's row opens with, in row order.
EVENT_COLUMNS = [
    "currunix",
    "creaunix",
    "expirunix",
    "prevunix",
    "snapunix",
    "curruuid",
    "crossuuid",
    "crosscode",
    "currhashcode",
    "crosshashcode",
    "prevuuid",
    "seqnum",
    "parentuuids",
    "srcuuids",
    "identifiers",
    "state",
]
CODES = ["isincode", "cusipcode", "sedolcode", "bloombergcode", "cficode", "miccode"]
EVENT_FACTS = EVENT_COLUMNS

# One order's life: the placement under the client's identifier, the
# acknowledgement under the venue's, a partial fill and the fill naming the
# venue's alone.
LIFE = [
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|59=0|15=USD|52=20260102-10:15:30.250|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|52=20260102-10:15:30.500|10=0|",
    b"8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|6=10.5|55=AAPL|54=1|52=20260102-10:15:31.100|10=0|",
    b"8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.5|32=60|6=10.5|55=AAPL|54=1|52=20260102-10:15:33.100|10=0|",
]

# Three reports against one order: an acknowledgement that fills nothing,
# then two fills at the venue.
REPORTS = [
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|15=USD|52=20260102-10:15:30.500|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|30=XNAS|55=AAPL|54=1|15=USD|880=M1|52=20260102-10:15:31.100|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.6|32=60|30=XNAS|55=AAPL|54=1|15=USD|880=M2|52=20260102-10:15:33.100|10=0|",
]

# The two sides' reports of one match, with dates and parties, and a trade
# capture report of another.
MATCHES = [
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|31=10.5|32=40|55=AAPL|54=1|15=USD|75=20260102|64=20260105|880=M1|453=2|448=FIRM|447=D|452=1|448=CLI-9|447=D|452=3|60=20260102-10:15:31.100|52=20260102-10:15:31.100|10=0|",
    b"8=FIX.4.4|35=8|11=B7|37=O2|17=E9|150=F|39=2|31=10.5|32=40|55=AAPL|54=2|15=USD|75=20260102|880=M1|60=20260102-10:15:31.200|52=20260102-10:15:31.200|10=0|",
    b"8=FIX.4.4|35=AE|571=T1|1003=TR1|31=11|32=5|55=AAPL|54=1|15=USD|75=20260102|60=20260102-10:15:40.000|52=20260102-10:15:40.000|10=0|",
]

# One quote, its update, its cancel, and a one-sided quote of its own.
QUOTES = [
    b"8=FIX.4.4|35=S|117=Q1|131=R1|55=AAPL|132=10.4|134=100|133=10.6|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:30.250|10=0|",
    b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=100|133=10.55|135=150|15=USD|62=20260102-10:16:00.000|52=20260102-10:15:31.250|10=0|",
    b"8=FIX.4.4|35=Z|117=Q1|298=1|52=20260102-10:15:32.250|10=0|",
    b"8=FIX.4.4|35=S|117=Q2|55=AAPL|54=1|132=10.3|134=50|15=USD|52=20260102-10:15:33.250|10=0|",
]

# The makers of two instruments' ladders across three grid seconds: three
# bids and an ask resting in AAPL, a two-sided quote joining them, the first
# bid filled away, and one bid in MSFT.
MAKERS = [
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.100|10=0|",
    b"8=FIX.4.4|35=D|11=A2|55=AAPL|54=1|38=50|44=10.5|15=USD|52=20260102-10:15:30.200|10=0|",
    b"8=FIX.4.4|35=D|11=A3|55=AAPL|54=1|38=70|44=10.4|15=USD|52=20260102-10:15:30.300|10=0|",
    b"8=FIX.4.4|35=D|11=S1|55=AAPL|54=2|38=80|44=10.7|15=USD|52=20260102-10:15:30.400|10=0|",
    b"8=FIX.4.4|35=S|117=Q1|55=AAPL|132=10.45|134=30|133=10.6|135=40|15=USD|52=20260102-10:15:31.100|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|15=USD|52=20260102-10:15:32.100|10=0|",
    b"8=FIX.4.4|35=D|11=M1|55=MSFT|54=1|38=10|44=400|15=USD|52=20260102-10:15:32.500|10=0|",
]


def _fixed(registry: FixRegistry, **pins: Any) -> FixCodec:
    """A codec whose undated messages all take ``CLOCK`` as their SendingTime."""
    return FixCodec(registry, default_sending_time=CLOCK, **pins)


def _decimal(text: str) -> Scalar:
    """One of the market's numbers as the exact decimal the products answer."""
    return DataType("decimal128(38, 18)").scalar(decimal.Decimal(text))


def _reader(codec: FixCodec, lines: Iterable[bytes]) -> pa.RecordBatchReader:
    """The lines as one stream of batches of FIX rows, the shape every Arrow door reads."""
    return codec.arrow_reader(fix_schema(codec.registry), codec.parse_lines(lines))


def _rows(reader: pa.RecordBatchReader) -> list[Scalar]:
    """Every row a reader answers, crossed back as the native rows they are."""
    rows: list[Scalar] = []
    for batch in reader:
        rows.extend(ArrowScalar.from_(batch).into_scalar())
    return rows


def _uuids(values: Iterable[Scalar]) -> set[str]:
    return {held.as_py() for held in values}


@pytest.fixture(scope="module")
def _seed_catalog() -> FixRegistry:
    """The dictionary the repository tracks at ``config/fix``."""
    return FixRegistry.from_handle(SEED)


@pytest.fixture
def seed(_seed_catalog: FixRegistry) -> FixRegistry:
    return copy.copy(_seed_catalog)


def _assert_event_parity(product: Any, event: MarketEventData) -> None:
    """A product's own properties answer what its ``event()`` answers."""
    assert isinstance(event, MarketEventData)
    for name in EVENT_FACTS:
        assert getattr(product, name) == getattr(event, name), name
    assert isinstance(product.curruuid, Scalar) and product.curruuid.as_py() is not None
    assert isinstance(product.currhashcode, int) and isinstance(product.seqnum, int)
    assert isinstance(product.identifiers, dict)
    assert all(isinstance(held, Scalar) for held in product.parentuuids + product.srcuuids)


def _assert_protocol(product: Any, cls: type) -> None:
    """Frozen snapshot: equality by value, hash by the code, copies as itself."""
    assert isinstance(product, cls)
    assert isinstance(product.is_alive, bool)
    assert product == product
    assert not (product != product)
    assert product.__eq__(object()) is NotImplemented
    assert hash(product) == hash(copy.copy(product))
    assert copy.copy(product) == product
    assert copy.deepcopy(product) == product
    assert isinstance(copy.copy(product), cls)
    assert repr(product).startswith(f"{cls.__name__}(")
    assert "crosscode=" in repr(product)
    with pytest.raises(AttributeError):
        product.currunix = 0


def _assert_round_trip(product: Any, cls: type, field: Field) -> None:
    """A row read back is the same product by row, identity and hash.

    By row, as the core's own tests pin it: a walked statement's event
    holds facts its row does not publish, so the value read back is the
    row's product, not the walk's.
    """
    row = product.into_row()
    assert isinstance(row, Scalar)
    again = cls.from_row(field, row)
    assert isinstance(again, cls)
    assert again.into_row() == row
    assert again.curruuid == product.curruuid
    assert again.currhashcode == product.currhashcode
    assert hash(again) == hash(product)
    # A mapping of names and a sequence in the field's order read the same.
    named = dict(zip([child.name for child in field], row.as_py()))
    assert cls.from_row(field, named).into_row() == row
    assert cls.from_row(field, row.as_py()).into_row() == row
    # A row that does not fit the field is refused where it is met.
    with pytest.raises(ValueError):
        cls.from_row(field, {"currunix": "not an instant"})


# --------------------------------------------------------------------------
# Orders


def test_an_order_is_its_placement_and_every_report_against_it(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    parsed = list(codec.parse_lines(LIFE))
    stream = codec.orders(parsed)
    assert isinstance(stream, Orders)
    assert iter(stream) is stream
    with pytest.raises(TypeError):
        hash(stream)
    orders = list(stream)
    assert len(orders) == 4
    assert next(stream, None) is None

    # Every statement names the message it was read from, and nothing else.
    chained = list(codec.lifecycle(parsed))
    for order, message in zip(orders, chained):
        assert order.srcuuids == [message.curruuid]
        assert order.currunix == message.currunix

    # The chain is the order's, under the identifier it opened with.
    placement, ack, partial, fill = orders
    assert placement.crosscode == "A1"
    assert {held.crosscode for held in orders} == {"A1"}
    assert {held.crossuuid.as_py() for held in orders} == {placement.crossuuid.as_py()}
    assert [held.seqnum for held in orders] == [0, 1, 2, 3]
    assert placement.prevuuid is None
    assert ack.prevuuid == placement.curruuid
    assert partial.prevuuid == ack.curruuid
    assert fill.prevuuid == partial.curruuid
    assert fill.prevunix == partial.currunix
    assert fill.parentuuids == [placement.curruuid, ack.curruuid, partial.curruuid]
    assert {held.creaunix for held in orders} == {placement.creaunix}
    assert fill.identifiers == {"clordid": "A1", "orderid": "O1"}

    # The market facts, crossed as the event crosses them.
    assert fill.px == _decimal("10.5")
    assert fill.avgpx == _decimal("10.5")
    assert fill.qty == _decimal("100")
    assert fill.cumqty == _decimal("100")
    assert fill.leavesqty == _decimal("0")
    assert fill.side.as_py() == "BUY"
    assert fill.currency.as_py() == "USD"
    assert fill.unit == ""
    assert fill.tif == "0"
    assert fill.tradable is None
    assert fill.symbolticker == "AAPL"
    assert all(getattr(fill, code) is None for code in CODES)
    assert fill.stoppx is None
    assert fill.ordtype is None, "the placement states no OrdType(40)"
    assert fill.state.as_py() == "80FILLED"
    assert placement.state.as_py() == "00UNKNOWN"
    assert fill.expirunix is None and fill.snapunix is None

    # What the stated facts imply, read as properties: a limit stated
    # alone is a limit order, what is left and what filled floor at zero,
    # and a filled order rests nothing.
    assert [held.pricing for held in orders] == ["limit"] * 4
    assert [held.remaining for held in orders] == [
        _decimal("100"),
        _decimal("100"),
        _decimal("60"),
        _decimal("0"),
    ]
    assert [held.filled for held in orders] == [
        _decimal("0"),
        _decimal("0"),
        _decimal("40"),
        _decimal("100"),
    ]
    assert partial.filled_ratio == _decimal("0.4") and fill.filled_ratio == _decimal("1")
    assert [held.is_resting for held in orders] == [True, True, True, False]
    assert [held.is_alive for held in orders] == [True, True, True, False]
    assert fill.notional == _decimal("1050"), "px * qty"
    assert isinstance(fill.remaining, Scalar) and isinstance(fill.filled, Scalar)

    for order in orders:
        _assert_event_parity(order, order.event())
        _assert_protocol(order, Order)
    assert placement != fill
    assert hash(placement) != hash(fill)

    # The row: the sixteen, the market columns, then the order's own.
    field = Order.field()
    assert isinstance(field, Field)
    assert field.name == "order" and not field.nullable
    assert [child.name for child in field] == EVENT_COLUMNS + [
        "px",
        "avgpx",
        "qty",
        "cumqty",
        "leavesqty",
        "side",
        "currency",
        "unit",
        "tif",
        "tradable",
        "symbolticker",
        *CODES,
        "stoppx",
        "ordtype",
    ]
    for order in orders:
        _assert_round_trip(order, Order, field)
    row = fill.into_row().as_py()
    assert row[field.index_of("crosscode")] == "A1"
    assert row[field.index_of("stoppx")] is None
    assert row[field.index_of("ordtype")] is None
    # A row stating a type reads it back, and the pricing follows the type.
    stated = dict(zip([child.name for child in field], row))
    stated["ordtype"] = "1"
    market = Order.from_row(field, stated)
    assert market.ordtype == "1" and market.pricing == "market"
    assert not market.is_resting, "a market order rests on no ladder"
    stated["ordtype"] = "P"
    assert Order.from_row(field, stated).pricing is None, "a pegged order prices some other way"


def test_a_stream_pulls_one_message_at_a_time_and_raises_where_it_fails(
    seed: FixRegistry,
) -> None:
    codec = _fixed(seed)
    parsed = list(codec.parse_lines(LIFE))
    pulled = 0

    def messages() -> Iterable[FixMsg]:
        nonlocal pulled
        for message in parsed:
            pulled += 1
            yield message

    stream = codec.orders(messages())
    # The walk sorts by instant, so the door pulls the iterable through
    # before it answers a product; the pull is the door's, one item at a
    # time, and nothing is collected on the Python side.
    assert pulled == len(parsed)
    first = next(stream)
    assert isinstance(first, Order)
    assert len(list(stream)) == len(parsed) - 1
    assert pulled == len(parsed)

    # An item that is not a message is refused where it is met, as itself,
    # and ends the stream.
    mixed = codec.orders([parsed[0], b"8=FIX.4.4|35=D|10=0|"])
    assert isinstance(next(mixed), Order)
    with pytest.raises(TypeError):
        next(mixed)
    assert next(mixed, None) is None

    # A failure of the iterable itself raises as itself.
    def failing() -> Iterable[FixMsg]:
        yield parsed[0]
        raise RuntimeError("the source broke")

    broken = codec.executions(failing())
    with pytest.raises(RuntimeError, match="the source broke"):
        list(broken)

    # Something that is not iterable is refused before anything is pulled.
    with pytest.raises(TypeError):
        codec.trades(17)  # type: ignore[arg-type]


def test_an_order_states_back_as_a_new_order_single(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    placement, *_ = list(codec.orders(codec.parse_lines(LIFE)))
    message = FixMsg.from_order(codec, placement)
    assert isinstance(message, FixMsg)
    assert message.header().msgtype == "D"
    assert message.by_tag(11).as_py() == "A1"
    assert message.by_tag(40).as_py() == "2", "a limit order"
    assert message.px == placement.px
    assert message.qty == placement.qty
    assert message.srcuuids == [placement.curruuid]
    # What the order stated is what the message reads back as.
    [again] = list(codec.orders([message]))
    assert (again.qty, again.side.as_py()) == (placement.qty, "BUY")
    # A codec and an order are what the door takes.
    with pytest.raises(TypeError):
        FixMsg.from_order(codec, message)  # type: ignore[arg-type]


# --------------------------------------------------------------------------
# Executions


def test_only_a_report_stating_a_traded_quantity_is_an_execution(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    stream = codec.executions(codec.parse_lines(REPORTS))
    assert isinstance(stream, Executions)
    executions = list(stream)
    assert len(executions) == 2, "the acknowledgement fills nothing"
    first, second = executions
    assert first.px == _decimal("10.5") and first.qty == _decimal("40")
    assert second.px == _decimal("10.6") and second.qty == _decimal("60")
    assert first.side.as_py() == "BUY"
    assert first.currency.as_py() == "USD"
    assert first.unit == ""
    assert first.symbolticker == "AAPL"
    assert first.miccode is not None and first.miccode.as_py() == "XNAS"
    assert all(getattr(first, code) is None for code in CODES[:-1])
    assert first.identifiers == {"clordid": "A1", "execid": "E1", "orderid": "O1"}
    # Both fills are in the chain of the order they fill.
    assert first.crosscode == second.crosscode
    assert second.prevuuid == first.curruuid
    assert [held.seqnum for held in executions] == [0, 1]
    # Where the fill left its order, read off the state it reports.
    assert first.is_partial and not first.completes
    assert second.completes and not second.is_partial
    assert first.is_alive and not second.is_alive
    assert first.notional == _decimal("420") and second.notional == _decimal("636")
    for execution in executions:
        _assert_event_parity(execution, execution.event())
        _assert_protocol(execution, Execution)

    field = Execution.field()
    assert field.name == "execution"
    assert [child.name for child in field] == EVENT_COLUMNS + [
        "px",
        "qty",
        "side",
        "currency",
        "unit",
        "symbolticker",
        *CODES,
    ]
    for execution in executions:
        _assert_round_trip(execution, Execution, field)

    # An execution states back as the report it is.
    message = FixMsg.from_execution(codec, first)
    assert message.header().msgtype == "8"
    assert message.by_tag(150).as_py() == "F"
    assert message.by_tag(17).as_py() == "E1"
    assert message.srcuuids == [first.curruuid]
    [again] = list(codec.executions([message]))
    assert (again.px, again.qty) == (first.px, first.qty)


# --------------------------------------------------------------------------
# Trades


def test_a_trade_is_the_matched_quantity_with_its_parties_and_dates(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    stream = codec.trades(codec.parse_lines(MATCHES))
    assert isinstance(stream, Trades)
    trades = list(stream)
    assert len(trades) == 3
    buy, sell, captured = trades
    # The two sides' reports of one match share the chain the venue matched
    # them by.
    assert buy.crosscode == sell.crosscode == "M1"
    assert sell.prevuuid == buy.curruuid
    assert captured.crosscode != "M1"
    assert buy.px == _decimal("10.5") and buy.qty == _decimal("40")
    assert buy.side.as_py() == "BUY" and sell.side.as_py() == "SELL"
    assert captured.px == _decimal("11") and captured.qty == _decimal("5")
    # Dates cross as `datetime.date`, the day count read as the day it names.
    assert buy.tradedate == dt.date(2026, 1, 2)
    assert buy.settldate == dt.date(2026, 1, 5)
    assert sell.settldate is None
    assert captured.tradedate == dt.date(2026, 1, 2)
    # Parties as (role, id, source), in the order the report states them.
    assert buy.parties == [("1", "FIRM", "D"), ("3", "CLI-9", "D")]
    assert sell.parties == []
    assert all(isinstance(party, tuple) and len(party) == 3 for party in buy.parties)
    # The party of one role, and what the clocks imply.
    assert buy.party_by_role("1") == ("1", "FIRM", "D")
    assert buy.party_by_role("3") == ("3", "CLI-9", "D")
    assert buy.party_by_role("9") is None and sell.party_by_role("1") is None
    assert buy.settlement_days == 3, "T+3"
    assert sell.settlement_days is None, "no settlement date stated"
    assert buy.notional == _decimal("420") and captured.notional == _decimal("55")
    for trade in trades:
        _assert_event_parity(trade, trade.event())
        _assert_protocol(trade, Trade)

    field = Trade.field()
    assert field.name == "trade"
    assert [child.name for child in field] == EVENT_COLUMNS + [
        "px",
        "qty",
        "side",
        "currency",
        "unit",
        "symbolticker",
        *CODES,
        "tradedate",
        "settldate",
        "parties",
    ]
    for trade in trades:
        _assert_round_trip(trade, Trade, field)
    row = buy.into_row().as_py()
    assert row[field.index_of("tradedate")] == dt.date(2026, 1, 2)
    # A raw row states a struct cell positionally; the Arrow twin names it.
    assert row[field.index_of("parties")] == [["1", "FIRM", "D"], ["3", "CLI-9", "D"]]

    # No one message states a match, so the crate does not guess.
    with pytest.raises(ValueError, match="no one message states a match"):
        FixMsg.from_trade(codec, buy)


# --------------------------------------------------------------------------
# Quotes


def test_a_quote_is_its_statement_its_updates_and_its_cancel(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    stream = codec.quotes(codec.parse_lines(QUOTES))
    assert isinstance(stream, Quotes)
    quotes = list(stream)
    assert len(quotes) == 4
    stated, updated, cancelled, one_sided = quotes
    assert [held.crosscode for held in quotes] == ["Q1", "Q1", "Q1", "Q2"]
    assert [held.seqnum for held in quotes] == [0, 1, 2, 0]
    assert updated.prevuuid == stated.curruuid
    assert stated.bidpx == _decimal("10.4") and stated.bidqty == _decimal("100")
    assert stated.askpx == _decimal("10.6") and stated.askqty == _decimal("150")
    assert updated.bidpx == _decimal("10.45") and updated.askpx == _decimal("10.55")
    assert stated.bidcurrency is not None and stated.bidcurrency.as_py() == "USD"
    assert stated.askcurrency is not None and stated.askcurrency.as_py() == "USD"
    assert stated.bidunit is None and stated.askunit is None
    assert stated.symbolticker == "AAPL"
    assert all(getattr(stated, code) is None for code in CODES)
    assert stated.expirunix == 1_767_348_960_000_000_000
    assert cancelled.state.as_py() == "90CANCELED"
    # A one-sided quote fills one lane and leaves the other None.
    assert one_sided.bidpx == _decimal("10.3")
    assert one_sided.askpx is None and one_sided.askqty is None
    assert one_sided.askcurrency is None
    # The lanes as lanes, and what they imply.
    assert stated.bid == (_decimal("10.4"), _decimal("100"))
    assert stated.ask == (_decimal("10.6"), _decimal("150"))
    assert stated.is_two_sided and not one_sided.is_two_sided
    assert stated.mid == _decimal("10.5") and stated.spread == _decimal("0.2")
    assert updated.spread == _decimal("0.1")
    assert one_sided.ask is None and one_sided.mid is None and one_sided.spread is None
    assert cancelled.bid is None and cancelled.ask is None
    assert not cancelled.is_alive and stated.is_alive
    # A lane by side, under any spelling Side.read reads; a cross takes none.
    assert stated.lane("Buy") == stated.bid
    assert stated.lane("1") == stated.bid
    assert stated.lane("SELL") == stated.ask
    assert stated.lane("H") == stated.ask, "sell undisclosed is still a sell"
    assert stated.lane("Cross") is None, "a cross is both sides at once"
    assert one_sided.lane("2") is None
    with pytest.raises(ValueError, match="side"):
        stated.lane("nope")
    for quote in quotes:
        _assert_event_parity(quote, quote.event())
        _assert_protocol(quote, Quote)

    field = Quote.field()
    assert field.name == "quote"
    assert [child.name for child in field] == EVENT_COLUMNS + [
        "bidpx",
        "bidqty",
        "bidcurrency",
        "bidunit",
        "askpx",
        "askqty",
        "askcurrency",
        "askunit",
        "symbolticker",
        *CODES,
    ]
    for quote in quotes:
        _assert_round_trip(quote, Quote, field)

    with pytest.raises(ValueError, match="no one message states which of them"):
        FixMsg.from_quote(codec, stated)


# --------------------------------------------------------------------------
# Books


def test_a_book_is_the_ladder_at_one_instant_to_a_declared_depth(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    stream = codec.books(codec.parse_lines(MAKERS), 5)
    assert isinstance(stream, Books)
    books = list(stream)
    # One book per symbol per instant touched: six of AAPL, one of MSFT,
    # none stamped with a grid.
    assert len(books) == 7
    first, second, third, fourth, quoted, filled, msft = books
    assert [held.crosscode for held in books] == ["AAPL"] * 6 + ["MSFT"]
    assert [held.symbolticker for held in books] == ["AAPL"] * 6 + ["MSFT"]
    assert all(held.depth == 5 for held in books)
    assert all(held.snapunix is None for held in books)
    assert first.currunix == 1_767_348_930_100_000_000
    assert first.identifiers == {}, "a book goes by its symbol, which is its chain"
    # Levels cross as (px, qty, count), best first, summed per price.
    assert first.bids == [(_decimal("10.5"), _decimal("100"), 1)]
    assert first.asks == []
    assert second.bids == [(_decimal("10.5"), _decimal("150"), 2)]
    assert third.bids == [
        (_decimal("10.5"), _decimal("150"), 2),
        (_decimal("10.4"), _decimal("70"), 1),
    ]
    assert fourth.asks == [(_decimal("10.7"), _decimal("80"), 1)]
    assert quoted.bids == [
        (_decimal("10.5"), _decimal("150"), 2),
        (_decimal("10.45"), _decimal("30"), 1),
        (_decimal("10.4"), _decimal("70"), 1),
    ]
    assert quoted.asks == [
        (_decimal("10.6"), _decimal("40"), 1),
        (_decimal("10.7"), _decimal("80"), 1),
    ]
    assert all(isinstance(level, tuple) and len(level) == 3 for level in quoted.bids)
    # The fill took the first bid away, and printed against the book.
    assert filled.bids[0] == (_decimal("10.5"), _decimal("50"), 1)
    assert msft.bids == [(_decimal("400"), _decimal("10"), 1)]

    # A book is a market event: the tops are its lanes, the mid its px,
    # what rests on both ladders its qty, the prints its last and volume.
    assert first.bidpx == _decimal("10.5") and first.bidqty == _decimal("100")
    assert first.askpx is None and first.askqty is None
    assert first.px == _decimal("0"), "no mid on a one-sided book"
    assert first.mid is None and first.spread is None
    assert fourth.px == _decimal("10.6") and fourth.mid == _decimal("10.6")
    assert fourth.qty == _decimal("300"), "resting on both ladders"
    assert first.lastpx is None and first.lastqty is None
    assert first.avgpx is None and first.cumqty is None
    assert filled.lastpx == _decimal("10.5") and filled.lastqty == _decimal("100")
    assert filled.avgpx == _decimal("10.5") and filled.cumqty == _decimal("100")
    assert filled.updates == 7, "five makers, the order's fill and the print"
    assert [held.updates for held in books] == [1, 2, 3, 4, 5, 7, 1]
    assert first.tradable is None
    assert first.currency.as_py() == "USD" and first.unit == ""
    assert all(getattr(first, code) is None for code in CODES)
    assert first.state.as_py() == "00UNKNOWN", "a book has no lifecycle of its own"

    # What the ladders imply, read on every call.
    assert quoted.best_bid == (_decimal("10.5"), _decimal("150"), 2)
    assert quoted.best_ask == (_decimal("10.6"), _decimal("40"), 1)
    assert first.best_ask is None
    assert quoted.level("Buy", 1) == (_decimal("10.45"), _decimal("30"), 1)
    assert quoted.level("2", 0) == quoted.best_ask
    assert quoted.level("Sell", 5) is None, "past the ladder"
    assert quoted.level("H", 0) == quoted.level("Sell", 0), "sell undisclosed takes the ask"
    assert quoted.level("Cross", 0) is None, "a cross takes no lane"
    with pytest.raises(ValueError, match="side"):
        quoted.level("nope", 0)
    assert quoted.is_two_sided and not first.is_two_sided
    assert not quoted.is_locked and not quoted.is_crossed
    assert quoted.spread == _decimal("0.1")
    assert quoted.spread_bps == _decimal("94.786729857819905213")
    assert quoted.microprice == _decimal("10.578947368421052631")
    assert quoted.imbalance == _decimal("0.578947368421052631")
    assert quoted.imbalance_to_depth(1) == quoted.imbalance
    assert quoted.imbalance_to_depth(5) == _decimal("0.351351351351351351")
    assert quoted.imbalance_to_depth(0) is None
    assert first.imbalance == _decimal("1"), "a missing top counts as no size"
    assert first.spread_bps is None and first.microprice is None
    assert quoted.bid_size == _decimal("250") and quoted.ask_size == _decimal("120")
    assert quoted.bid_count == 4 and quoted.ask_count == 2
    assert first.ask_size == _decimal("0") and first.ask_count == 0

    # A book's sources are the statements applied since the book before
    # it, and what rested any maker it retired.
    chained = list(codec.lifecycle(codec.parse_lines(MAKERS)))
    assert quoted.srcuuids == [chained[4].curruuid]
    assert _uuids(filled.srcuuids) == _uuids([chained[0].curruuid, chained[5].curruuid])
    assert [len(held.srcuuids) for held in books] == [1, 1, 1, 1, 1, 2, 1]
    # The chain is the symbol's, flat: each book follows the one before
    # and descends from it alone; another instrument is another chain.
    assert [held.seqnum for held in books] == [0, 1, 2, 3, 4, 5, 0]
    assert second.prevuuid == first.curruuid
    assert filled.parentuuids == [quoted.curruuid]
    assert msft.prevuuid is None
    assert msft.crossuuid != first.crossuuid
    for book in books:
        _assert_event_parity(book, book.event())
        _assert_protocol(book, Book)

    field = Book.field(5)
    assert field.name == "book"
    assert [child.name for child in field] == EVENT_COLUMNS + [
        "lastpx",
        "lastqty",
        "avgpx",
        "cumqty",
        "tradable",
        "currency",
        "unit",
        "symbolticker",
        *CODES,
        "bids",
        "asks",
        "updates",
    ]
    assert field.index_of("bids") == 30
    assert field.index_of("asks") == 31
    assert field.index_of("updates") == 32
    assert Book.field(2) != field
    for book in books:
        _assert_round_trip(book, Book, field)
    row = quoted.into_row().as_py()
    assert len(row[field.index_of("bids")]) == 5, "a fixed-size list of depth levels"
    assert row[field.index_of("bids")][3] is None
    assert row[field.index_of("updates")] == 5
    again = Book.from_row(field, row)
    assert again.px == quoted.px, "the mid is re-derived from the ladders"
    assert again.updates == 5 and again.microprice == quoted.microprice

    with pytest.raises(ValueError, match="no one message states a ladder"):
        FixMsg.from_book(codec, first)


def test_a_grid_reads_one_book_per_symbol_per_step_at_its_closing_state(
    seed: FixRegistry,
) -> None:
    codec = _fixed(seed)
    books = list(codec.books(codec.parse_lines(MAKERS), 5, STEP))
    # Three steps of AAPL, one of MSFT, in the order the steps closed.
    assert len(books) == 4
    first, second, third, msft = books
    assert [held.crosscode for held in books] == ["AAPL", "AAPL", "AAPL", "MSFT"]
    # Every book is the closing state of a step, stamped with it and dated
    # at the last instant that moved the symbol.
    assert [held.snapunix for held in books] == [
        1_767_348_930_000_000_000,
        1_767_348_931_000_000_000,
        1_767_348_932_000_000_000,
        1_767_348_932_000_000_000,
    ]
    assert first.currunix == 1_767_348_930_400_000_000
    assert first.bids == [
        (_decimal("10.5"), _decimal("150"), 2),
        (_decimal("10.4"), _decimal("70"), 1),
    ]
    assert first.asks == [(_decimal("10.7"), _decimal("80"), 1)]
    assert first.updates == 4
    assert len(first.srcuuids) == 4, "every statement applied in the step"
    assert second.bids == [
        (_decimal("10.5"), _decimal("150"), 2),
        (_decimal("10.45"), _decimal("30"), 1),
        (_decimal("10.4"), _decimal("70"), 1),
    ]
    assert third.bids[0] == (_decimal("10.5"), _decimal("50"), 1)
    assert third.lastpx == _decimal("10.5")
    # Chained per symbol across steps, flat.
    assert [held.seqnum for held in books] == [0, 1, 2, 0]
    assert second.prevuuid == first.curruuid
    assert third.parentuuids == [second.curruuid]
    assert msft.bids == [(_decimal("400"), _decimal("10"), 1)]
    for book in books:
        _assert_round_trip(book, Book, Book.field(5))


def test_a_book_refuses_no_depth_and_a_negative_step_before_a_message_is_pulled(
    seed: FixRegistry,
) -> None:
    codec = _fixed(seed)
    pulled = 0

    def messages() -> Iterable[FixMsg]:
        nonlocal pulled
        for message in codec.parse_lines(MAKERS):
            pulled += 1
            yield message

    with pytest.raises(ValueError, match="depth"):
        codec.books(messages(), 0, STEP)
    with pytest.raises(ValueError, match="depth"):
        codec.books(messages(), 0)
    with pytest.raises(ValueError, match="snapshot_ns"):
        codec.books(messages(), 5, -1)
    assert pulled == 0
    # A step of zero is no grid: one book per instant, the default.
    assert [held.snapunix for held in codec.books(messages(), 5, 0)] == [None] * 7
    with pytest.raises(ValueError, match="depth"):
        codec.books_arrow_reader(_reader(codec, MAKERS), 0, STEP)
    with pytest.raises(ValueError, match="snapshot_ns"):
        codec.books_arrow_reader(_reader(codec, MAKERS), 5, -1)
    with pytest.raises(ValueError, match="depth"):
        Book.field(0)
    # A depth that is not a count is refused by the boundary.
    with pytest.raises((OverflowError, TypeError)):
        codec.books(messages(), -1, STEP)


# --------------------------------------------------------------------------
# Symbols


def test_a_symbol_is_the_text_a_book_is_keyed_by(seed: FixRegistry) -> None:
    aapl = Symbol("AAPL")
    assert aapl.text == "AAPL" and str(aapl) == "AAPL"
    assert repr(aapl) == 'Symbol("AAPL")'
    assert not aapl.is_global
    assert Symbol(" AAPL ") == aapl, "trimmed"
    # Blank text is the global symbol, the default of no instrument.
    assert Symbol.GLOBAL.text == "GLOBAL" and Symbol.GLOBAL.is_global
    assert Symbol("") == Symbol.GLOBAL and Symbol("   ") == Symbol.GLOBAL
    assert Symbol("GLOBAL") == Symbol.GLOBAL
    assert isinstance(Symbol.GLOBAL, Symbol)
    # Immutable: equality, order and hash over the text, copies and pickles.
    assert aapl != Symbol("MSFT")
    assert aapl.__eq__("AAPL") is NotImplemented
    assert aapl < Symbol("MSFT") and Symbol("MSFT") > aapl
    assert aapl <= Symbol("AAPL") and aapl >= Symbol("AAPL")
    assert sorted([Symbol("MSFT"), Symbol.GLOBAL, aapl]) == [aapl, Symbol.GLOBAL, Symbol("MSFT")]
    with pytest.raises(TypeError):
        aapl < "MSFT"  # type: ignore[operator]
    assert hash(aapl) == hash(Symbol("AAPL")) and hash(aapl) != hash(Symbol("MSFT"))
    assert len({aapl, Symbol("AAPL"), Symbol.GLOBAL}) == 2
    assert copy.copy(aapl) == aapl and copy.deepcopy(aapl) == aapl
    assert pickle.loads(pickle.dumps(aapl)) == aapl
    with pytest.raises(AttributeError):
        aapl.text = "MSFT"  # type: ignore[misc]
    with pytest.raises(TypeError):
        Symbol()  # type: ignore[call-arg]

    # The rule a statement is keyed by: the ISIN, else the ticker, else
    # the other codes, else the global symbol; and a book's own symbol.
    codec = _fixed(seed)
    [order] = list(codec.orders(codec.parse_lines(MAKERS[:1])))
    assert Symbol.of(order) == aapl
    [quote] = list(codec.quotes(codec.parse_lines(MAKERS[4:5])))
    assert Symbol.of(quote) == aapl
    execution, *_ = list(codec.executions(codec.parse_lines(REPORTS)))
    assert Symbol.of(execution) == aapl
    trade, *_ = list(codec.trades(codec.parse_lines(MATCHES)))
    assert Symbol.of(trade) == aapl
    book, *_ = list(codec.books(codec.parse_lines(MAKERS), 5))
    assert Symbol.of(book) == aapl and book.crosscode == aapl.text
    field = Order.field()
    row = dict(zip([child.name for child in field], order.into_row().as_py()))
    row["isincode"] = "US0378331005"
    assert Symbol.of(Order.from_row(field, row)) == Symbol("US0378331005"), "the ISIN leads"
    row["isincode"] = None
    row["symbolticker"] = None
    assert Symbol.of(Order.from_row(field, row)) == Symbol.GLOBAL, "nothing named"
    with pytest.raises(TypeError, match="Symbol.of takes"):
        Symbol.of("AAPL")  # type: ignore[arg-type]


# --------------------------------------------------------------------------
# Statements and the book reader


def test_the_statements_door_answers_every_statement_in_instant_order(
    seed: FixRegistry,
) -> None:
    codec = _fixed(seed)
    parsed = list(codec.parse_lines(MAKERS))
    stream = codec.statements(parsed)
    assert isinstance(stream, Statements)
    assert iter(stream) is stream
    with pytest.raises(TypeError):
        hash(stream)
    statements = list(stream)
    assert next(stream, None) is None
    # Five placements, the quote, and the report as the order statement
    # then the fill it makes, in instant order, the orders of one instant
    # first; no trade, since the fill is the print.
    assert [type(held).__name__ for held in statements] == [
        "Order",
        "Order",
        "Order",
        "Order",
        "Quote",
        "Order",
        "Execution",
        "Order",
    ]
    instants = [held.currunix for held in statements]
    assert instants == sorted(instants)
    assert [held.crosscode for held in statements] == [
        "A1",
        "A2",
        "A3",
        "S1",
        "Q1",
        "A1",
        "A1",
        "M1",
    ]
    fill = statements[6]
    assert isinstance(fill, Execution)
    assert fill.crossuuid == statements[5].crossuuid, "a fill is in its order's chain"
    assert fill.qty == _decimal("100")
    # Each statement is the product its own door answers.
    assert [held for held in statements if isinstance(held, Order)] == list(
        codec.orders(parsed)
    )
    assert [held for held in statements if isinstance(held, Quote)] == list(
        codec.quotes(parsed)
    )
    assert [held for held in statements if isinstance(held, Execution)] == list(
        codec.executions(parsed)
    )
    # Pulled one message at a time, refusing what is not a message.
    mixed = codec.statements([parsed[0], b"8=FIX.4.4|35=D|10=0|"])
    assert isinstance(next(mixed), Order)
    with pytest.raises(TypeError):
        next(mixed)
    with pytest.raises(TypeError):
        codec.statements(17)  # type: ignore[arg-type]


def test_the_book_reader_reads_products_from_a_door_and_from_rows(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    parsed = list(codec.parse_lines(MAKERS))
    expected = [held.into_row() for held in codec.books(parsed, 5)]
    assert len(expected) == 7

    # Over the statements a door answers: what the books door answers.
    reader = BookIterator(codec.statements(parsed), 5)
    assert isinstance(reader, BookIterator)
    assert iter(reader) is reader
    with pytest.raises(TypeError):
        hash(reader)
    assert reader.depth == 5 and reader.snapshot_ns == 0 and reader.symbol is None
    books = list(reader)
    assert all(isinstance(held, Book) for held in books)
    assert [held.into_row() for held in books] == expected
    assert next(reader, None) is None

    # Over a grid: the closing state of each step.
    grid = BookIterator(codec.statements(parsed), 5, snapshot_ns=STEP)
    assert grid.snapshot_ns == STEP
    assert [held.into_row() for held in grid] == [
        held.into_row() for held in codec.books(parsed, 5, STEP)
    ]

    # Over products read back from rows, pulled one item at a time: the
    # same books, since a statement is what its row states.
    statements = list(codec.statements(parsed))
    order_field, quote_field, fill_field = Order.field(), Quote.field(), Execution.field()
    pulled = 0

    def from_rows() -> Iterable[Order | Quote | Execution | Trade]:
        nonlocal pulled
        for held in statements:
            pulled += 1
            if isinstance(held, Order):
                yield Order.from_row(order_field, held.into_row())
            elif isinstance(held, Quote):
                yield Quote.from_row(quote_field, held.into_row())
            else:
                yield Execution.from_row(fill_field, held.into_row())

    reader = BookIterator(from_rows(), 5)
    assert pulled == 0, "nothing is pulled before a book is asked for"
    first = next(reader)
    assert first.crosscode == "AAPL"
    assert pulled == 2, "the first instant closes when the second opens"
    assert [first.into_row(), *(held.into_row() for held in reader)] == expected
    assert pulled == len(statements)
    # A trade prints as an execution does.
    trades = list(codec.trades(codec.parse_lines(MATCHES)))
    # One book per instant the prints touched; the last carries them all.
    *_, printed = list(BookIterator(trades, 1))
    assert printed.cumqty == _decimal("85") and printed.lastpx == _decimal("11")
    assert printed.bids == [] and printed.updates == 3

    # One symbol for the whole stream: the global book.
    reader = BookIterator(statements, 5, symbol=Symbol.GLOBAL)
    assert reader.symbol == Symbol.GLOBAL
    books = list(reader)
    assert [held.crosscode for held in books] == ["GLOBAL"] * 7
    assert books[-1].bid_count == 4, "MSFT rests on the one ladder"
    assert [held.seqnum for held in books] == list(range(7))
    named = BookIterator(statements, 5, symbol=Symbol("MINE"))
    assert named.symbol == Symbol("MINE")
    assert {held.crosscode for held in named} == {"MINE"}


def test_the_book_reader_refuses_by_name_and_raises_where_it_is_met(
    seed: FixRegistry,
) -> None:
    codec = _fixed(seed)
    statements = list(codec.statements(codec.parse_lines(MAKERS)))
    pulled = 0

    def source() -> Iterable[Order | Quote | Execution | Trade]:
        nonlocal pulled
        for held in statements:
            pulled += 1
            yield held

    # The declarations are refused before an item is pulled.
    with pytest.raises(ValueError, match="depth"):
        BookIterator(source(), 0)
    with pytest.raises(ValueError, match="snapshot_ns"):
        BookIterator(source(), 5, snapshot_ns=-1)
    assert pulled == 0
    with pytest.raises((OverflowError, TypeError)):
        BookIterator(source(), -1)
    with pytest.raises(TypeError):
        BookIterator(source(), 5, symbol="AAPL")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        BookIterator(17, 5)  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        BookIterator(source(), 5, STEP)  # type: ignore[misc]

    # An item that is not a product raises TypeError as itself and ends
    # the stream, after the books of the instant already open.
    mixed = BookIterator([statements[0], b"not a product"], 5)
    assert next(mixed).crosscode == "AAPL"
    with pytest.raises(TypeError, match="not bytes"):
        next(mixed)
    assert next(mixed, None) is None
    # A failure of the iterable raises as itself, likewise.
    def failing() -> Iterable[Order | Quote | Execution | Trade]:
        yield statements[0]
        raise RuntimeError("the source broke")

    broken = BookIterator(failing(), 5)
    assert isinstance(next(broken), Book)
    with pytest.raises(RuntimeError, match="the source broke"):
        next(broken)

    # The statements arrive in instant order: one before the open instant
    # is refused naming currunix, moves nothing, and the stream goes on.
    rewound = BookIterator([statements[1], statements[0], statements[2]], 5)
    with pytest.raises(ValueError, match="currunix"):
        next(rewound)
    books = list(rewound)
    assert [held.currunix for held in books] == [statements[1].currunix, statements[2].currunix]
    assert books[0].bids == [(_decimal("10.5"), _decimal("50"), 1)]


# --------------------------------------------------------------------------
# Arrow twins


def test_every_arrow_door_agrees_with_its_message_door(seed: FixRegistry) -> None:
    codec = _fixed(seed)
    cases: list[tuple[Any, Any, list[bytes], Field]] = [
        (codec.orders_arrow_reader, codec.orders, LIFE, Order.field()),
        (codec.executions_arrow_reader, codec.executions, REPORTS, Execution.field()),
        (codec.trades_arrow_reader, codec.trades, MATCHES, Trade.field()),
        (codec.quotes_arrow_reader, codec.quotes, QUOTES, Quote.field()),
    ]
    for arrow_door, door, lines, field in cases:
        reader = arrow_door(_reader(codec, lines))
        assert isinstance(reader, pa.RecordBatchReader)
        assert reader.schema.names == [child.name for child in field]
        rows = _rows(reader)
        products = list(door(codec.messages(_reader(codec, lines))))
        assert len(rows) == len(products) > 0
        # Row for row: the batch is the product's own row.
        assert rows == [product.into_row() for product in products]
        assert [row.as_py()[field.index_of("curruuid")] for row in rows] == [
            product.curruuid.as_py() for product in products
        ]
        # A table and a batch cross the same door.
        table = _reader(codec, lines).read_all()
        assert _rows(arrow_door(table)) == rows
        assert _rows(arrow_door(table.to_batches()[0])) == rows

    reader = codec.books_arrow_reader(_reader(codec, MAKERS), 5, STEP)
    assert isinstance(reader, pa.RecordBatchReader)
    assert reader.schema.names == [child.name for child in Book.field(5)]
    rows = _rows(reader)
    books = list(codec.books(codec.messages(_reader(codec, MAKERS)), 5, STEP))
    assert len(rows) == len(books) == 4
    assert rows == [book.into_row() for book in books]
    # The default is one book per instant, on both doors.
    assert _rows(codec.books_arrow_reader(_reader(codec, MAKERS), 5)) == [
        book.into_row() for book in codec.books(codec.messages(_reader(codec, MAKERS)), 5)
    ]
    assert len(_rows(codec.books_arrow_reader(_reader(codec, MAKERS), 5))) == 7
    # A row read off the batch is the product again, under the batch's
    # own schema.
    batch = codec.books_arrow_reader(_reader(codec, MAKERS), 5, STEP).read_next_batch()
    root = Field.from_arrow_schema(batch.schema)
    again = Book.from_row(root, rows[1])
    assert again.into_row() == books[1].into_row()
    assert again.bids == books[1].bids

    # A source that is not a stream of batches is refused at the boundary.
    with pytest.raises(TypeError):
        codec.orders_arrow_reader(17)  # type: ignore[arg-type]


# --------------------------------------------------------------------------
# The bridge's own capture


def _reading() -> TextOptions:
    options = TextOptions()
    options.rowheader = ULBRIDGE_ROWHEADER
    options.timezone = Timezone.UTC
    options.start_rownum = 1
    return options


def test_every_product_joins_the_capture_on_srcuuids_to_curruuid(seed: FixRegistry) -> None:
    """The three hops: a line is a message's source, a message a product's."""
    options = _reading()
    codec = _fixed(seed, capture_names=list(options.capture_names))
    source = IOBase(CAPTURE)
    lines = list(source.read_text_lines(options=options))
    assert len(lines) == 144
    line_identities = _uuids(line.curruuid for line in lines)

    messages = list(codec.lifecycle(codec.parse_text_lines(lines)))
    assert len(messages) == 79
    assert len(_uuids(held.crossuuid for held in messages)) == 11
    for message in messages:
        assert len(message.srcuuids) == 1
        assert message.srcuuids[0].as_py() in line_identities
    known = _uuids(held.curruuid for held in messages)

    def parsed() -> FixMessages:
        return codec.parse_text_lines(source.read_text_lines(options=options))

    def joins(products: list[Any], pinned: tuple[int, int]) -> None:
        for product in products:
            assert product.srcuuids, "a product names what it was read from"
            for held in product.srcuuids:
                assert held.as_py() in known, "a source that is no message"
        assert len(products) <= len(messages), "products multiply no messages"
        identities = _uuids(held.curruuid for held in products)
        assert len(identities) <= len(known), "products multiply no identities"
        assert (len(products), len(identities)) == pinned

    orders = list(codec.orders(parsed()))
    joins(orders, (18, 18))
    order_identities = _uuids(held.curruuid for held in orders)
    for order in orders:
        if order.prevuuid is not None:
            assert order.prevuuid.as_py() in order_identities
        for parent in order.parentuuids:
            assert parent.as_py() in order_identities

    executions = list(codec.executions(parsed()))
    joins(executions, (14, 14))
    assert len(executions) < len(orders)

    trades = list(codec.trades(parsed()))
    joins(trades, (19, 19))
    assert len(trades) >= len(executions)

    quotes = list(codec.quotes(parsed()))
    joins(quotes, (0, 0))

    books = list(codec.books(parsed(), 5, STEP))
    joins(books, (10, 10))
    for book in books:
        assert book.snapunix is not None
        assert len(book.bids) <= 5 and len(book.asks) <= 5
    # The book reader over the statements door reads the same books.
    assert [held.into_row() for held in BookIterator(codec.statements(parsed()), 5, snapshot_ns=STEP)] == [
        held.into_row() for held in books
    ]

    # Each Arrow door is its message door over the rows the capture parsed
    # into, row for row.
    def reader() -> pa.RecordBatchReader:
        return codec.parse_text_arrow_reader(source.read_arrow_reader(options=options))

    def products_of(door: Any, *args: Any) -> list[Scalar]:
        return [held.into_row() for held in door(codec.messages(reader()), *args)]

    assert _rows(codec.orders_arrow_reader(reader())) == products_of(codec.orders)
    assert _rows(codec.executions_arrow_reader(reader())) == products_of(codec.executions)
    assert _rows(codec.trades_arrow_reader(reader())) == products_of(codec.trades)
    assert _rows(codec.quotes_arrow_reader(reader())) == products_of(codec.quotes)
    assert _rows(codec.books_arrow_reader(reader(), 5, STEP)) == products_of(
        codec.books, 5, STEP
    )
