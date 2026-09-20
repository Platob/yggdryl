"""The market's products: what the market did, read out of what a venue said.

A message is what a venue said; a product is what the market did. Five
products, each a value of the core with its own row and its own identity,
not a view over a message and not a second model of any protocol: an
:class:`Order` is one order's life, a :class:`Quote` a price stated at an
instant, an :class:`Execution` one fill, a :class:`Trade` the settled
transaction an execution reports, and a :class:`Book` the ladder for one
instrument at one instant. Each is a graph event exactly as a
:class:`~yggdryl.fix.FixMsg` is, so each answers the sixteen event facts
:class:`~yggdryl.fix.MarketEventData` answers - ``curruuid``, ``crossuuid``,
``crosscode``, ``currhashcode``, ``crosshashcode``, ``identifiers``,
``parentuuids``, ``srcuuids``, ``currunix``, ``state``, ``seqnum``,
``creaunix``, ``expirunix``, ``prevunix``, ``prevuuid``, ``snapunix`` - as
properties and as ``event()``, crossed the same way: a ``uuid`` ``Scalar``
for an identity, an ``int`` of nanoseconds since the epoch, UTC, for an
instant, a decimal ``Scalar`` for a number, a code ``Scalar`` for a
currency, a side, a state or an instrument identifier. A product's
``srcuuids`` are the identities of the messages it was read from, so a
product's table joins the message table on ``srcuuids`` to ``curruuid`` with
no mapping, and a walked product's ``prevuuid`` and ``parentuuids`` join its
own table. Every product answers ``is_alive``: whether its state can still
change and it is not past its expiration.

Each product publishes exactly the market facts its row holds after the
sixteen, and beside them what those facts imply, read on every call: a
bare noun is a property, a reading with an argument a method. An order:
``px``, ``avgpx``, ``qty``, ``cumqty``, ``leavesqty``, ``side``,
``currency``, ``unit``, ``tif``, ``tradable``, ``symbolticker``, the six
instrument codes ``isincode``, ``cusipcode``, ``sedolcode``,
``bloombergcode``, ``cficode`` and ``miccode``, its own ``stoppx`` and
``ordtype`` (FIX's ``OrdType(40)`` as spelled), and the implied ``pricing``
(``"market"``, ``"limit"``, ``"stop"``, ``"stoplimit"`` or ``None``),
``remaining``, ``filled``, ``filled_ratio``, ``is_resting`` and
``notional``. An execution: ``px``, ``qty``, ``side``, ``currency``,
``unit``, ``symbolticker``, the six codes, and ``notional``,
``is_partial``, ``completes``. A trade: the same market facts, its own
``tradedate`` and ``settldate`` as ``datetime.date`` and ``parties`` as
``(role, id, source)`` tuples, and ``notional``, ``settlement_days``,
``party_by_role(role)``. A quote: the two lanes ``bidpx``, ``bidqty``,
``bidcurrency``, ``bidunit``, ``askpx``, ``askqty``, ``askcurrency``,
``askunit``, ``symbolticker``, the six codes, and ``bid`` and ``ask`` as
``(px, qty)`` where quoted above zero, ``lane(side)``, ``is_two_sided``,
``mid``, ``spread``. A book is a market event whose ``px`` is the mid (zero
where none), ``qty`` the size resting on both ladders, ``bidpx``,
``bidqty``, ``askpx``, ``askqty`` the tops, ``lastpx``, ``lastqty``,
``avgpx``, ``cumqty`` the prints since the chain began and ``tradable`` the
makers' stated fact; then ``currency``, ``unit``, ``symbolticker``, the six
codes, its own ``depth``, ``updates``, ``bids`` and ``asks`` as ``(px, qty,
count)`` tuples best first, and the ladders' readings ``best_bid``,
``best_ask``, ``level(side, index)``, ``is_two_sided``, ``is_locked``,
``is_crossed``, ``mid``, ``spread``, ``spread_bps``, ``microprice``,
``imbalance``, ``imbalance_to_depth(levels)``, ``bid_size``, ``ask_size``,
``bid_count`` and ``ask_count``. ``into_row`` answers the product as one
row of ``field()`` - ``Book.field(depth)``, refusing a depth of zero - and
``from_row`` reads one back, canonicalized under the field first. A
product is a frozen snapshot: it compares by every fact, hashes by the code
the facts digest to, and copies as itself.

A :class:`Symbol` is the instrument a statement is about, as the one text
every book of it is keyed by: ``Symbol(text)`` the caller's own spelling,
blank being ``Symbol.GLOBAL``, and ``Symbol.of(product)`` the rule a
statement is keyed by - its ISIN, else its ticker, else its CUSIP, SEDOL
or Bloomberg identifier, else the global symbol. A book's ``crosscode`` is
the symbol it was read under.

The doors are on :class:`~yggdryl.fix.FixCodec`: ``orders``, ``executions``,
``trades`` and ``quotes`` read a stream of messages, lazily, as a stream of
one product - the lifecycle first, so a statement carries what its
message's chain folded forward, a message logged at two hops folded into
one statement naming both, the statements of one product then chained
exactly as messages are; ``statements`` reads every statement the messages
make in instant order, each an ``Order``, a ``Quote`` or an ``Execution``;
and ``books(messages, depth, snapshot_ns=0)`` reads the statements into a
:class:`BookIterator`, one book per symbol per instant to ``depth`` levels
a side, or per grid step of ``snapshot_ns`` nanoseconds at its closing
state, refusing a depth of zero and a negative step. :class:`Orders`,
:class:`Executions`, :class:`Trades`, :class:`Quotes`, :class:`Statements`
and :class:`Books` are the streams those doors answer, one shape each as
:class:`~yggdryl.fix.FixMessages` is. Each product door has an Arrow twin
over batches of message rows - ``orders_arrow_reader`` and the other four
- answering batches of product rows under the product's ``field()``.
:class:`BookIterator` is the same reader over any iterable of products,
arriving in instant order, from a door or read back from rows. Writing
back, :meth:`~yggdryl.fix.FixMsg.from_order` states an order as a new order
single and :meth:`~yggdryl.fix.FixMsg.from_execution` an execution as an
execution report; ``from_quote``, ``from_trade`` and ``from_book`` always
refuse, because no one message states a quote, a match or a ladder and the
crate does not guess.
"""

from __future__ import annotations

from ._native import (
    Book,
    BookIterator,
    Books,
    Execution,
    Executions,
    Order,
    Orders,
    Quote,
    Quotes,
    Scalar,
    Statements,
    Symbol,
    Trade,
    Trades,
)

# One level of a book's ladder as Python reads it: the price, the size
# resting there and how many orders and quote lanes rest there.
MarketLevel = tuple[Scalar, Scalar, int]
# One party to a trade: its role, its identifier and the scheme that issued
# it, each as the report spells it.
MarketParty = tuple[str, str, str]

__all__ = [
    "Book",
    "BookIterator",
    "Books",
    "Execution",
    "Executions",
    "MarketLevel",
    "MarketParty",
    "Order",
    "Orders",
    "Quote",
    "Quotes",
    "Statements",
    "Symbol",
    "Trade",
    "Trades",
]
