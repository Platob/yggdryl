"""`python/yggdryl/instrument.py` and `python/src/instrument.rs`: the
instruments, one element per instrument keyed by its cross code - a real
ISIN, or the `class:body` its CFI class and characteristics spell - holding
every identifier and listing it is known by, learned from and filled into
market data, bound to the store it is loaded from and committed back to,
redirected to the core."""

from __future__ import annotations

import copy
import datetime
import logging
import math
import os
import pathlib
import pickle
import re
import uuid

import pyarrow as pa
import pytest

from yggdryl import DataType, Eusipa, Field, Identifier, IOResult, Instruments, graph
from yggdryl.holder import LocalFile
from yggdryl.fix import FixCodec, FixMsg, FixRegistry
from yggdryl.instrument import Resolution

SEED = pathlib.Path(__file__).resolve().parents[2] / "config" / "fix"
HOLCIM = "CH0012214059"
APPLE = "US0378331005"
NOVARTIS = "CH0012005267"
DIAGEO = "GB0002374006"
SAP = "DE0007164600"
HSBC = "GB0005405286"
MICROSOFT = "US5949181045"
# A Eurex-shaped number no agency assigned: what an option is stated under
# before its body is known.
EUREX = "DE0000000009"
OPTION = "OC:US0378331005:2026-12-18:200"
# The digests the D42.2 pins state: the code's XXH3-64 and the identity it
# widens to, until every cross identity moves to the XXH3-128.
APPLE_CROSSHASH = 0x27E376388C8738FD
EURUSD_MINT = "QYLTVIRYHNX5"


@pytest.fixture(scope="module")
def codec() -> FixCodec:
    return FixCodec(FixRegistry.from_handle(SEED))


def stated(codec: FixCodec) -> FixMsg:
    """A message stating Holcim's ISIN, its RIC, its CFI code, its ticker, its market and its currency."""
    return codec.parse_fix_line(
        b"8=FIX.4.4|35=D|11=A|22=4|48=" + HOLCIM.encode() + b"|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|"
    )


def listing(miccode: str | None, ticker: str | None = None, currency: str | None = None, **codes: str) -> dict[str, object]:
    """One listing as a row states it."""
    return {"miccode": miccode, "ticker": ticker, "currency": currency, "codes": codes or None}


IDENTITY = ("uuid", "crossuuid", "hashcode", "crosshashcode", "updunix", "firstunix", "lastunix")


def facts(row: dict[str, object] | None) -> dict[str, object]:
    """The facts a row states: its non-null cells but the identity and the stamps."""
    assert row is not None
    return {key: value for key, value in row.items() if value is not None and key not in IDENTITY}


def test_an_instrument_is_learned_from_a_message_and_fills_a_later_one_named_by_its_ticker(codec: FixCodec) -> None:
    held = Instruments()
    assert not held and len(held) == 0 and not held.is_dirty
    message = stated(codec)
    assert message.instcode == HOLCIM, "a stated real ISIN is the code from the parse"
    assert held.learn(message)
    assert held.is_dirty
    row = held.get(HOLCIM)
    assert row is not None
    # The three stamps are the message's instant, which the undated line
    # takes from the clock: present, and left out of the facts compared.
    assert all(row[stamp] is not None for stamp in ("updunix", "firstunix", "lastunix"))
    assert facts(row) == {
        "crosscode": HOLCIM,
        "placeholder": False,
        "isin": HOLCIM,
        "cficode": "ESVUFR",
        # The Valor its own ISIN embeds is an instrument code; the RIC is the
        # listing's, on the market the message names.
        "securityids": {"cfi": "ESVUFR", "isin": HOLCIM, "valor": "1221405"},
        "listings": [listing("XSWX", "HOLN", "CHF", ric="HOLN.S")],
    }
    assert row["uuid"] == row["crossuuid"], "an instrument is its own identity"
    assert row["countrycode"] is None, "the prefix already says the country"
    assert held.get_by_ticker("HOLN", "XSWX") == row and held.get_by_ticker("HOLN", "XLON") is None
    later = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|")
    assert (later.isincode, later.instcode) == (None, None)
    assert held.fill(later)
    assert later.isincode == HOLCIM and later.securityids.is_derived("isin")
    assert later.instcode == HOLCIM, "the instrument's cross code"
    assert later.securityids.get("ric") == "HOLN.S" and later.securityids.is_derived("ric")
    assert later.cficode is not None and later.cficode.as_py() == "ESVUFR" and later.ticker == "HOLN"
    assert later.currency.as_py() == "CHF"
    assert not held.fill(later), "nothing left to fill"
    wire = later.into_bytes(ord("|"))
    assert b"461=" not in wire and b"15=" not in wire and b"48=" not in wire, "never the wire"
    # A RIC is a listing code and, with no ISIN stated, one of the lookup
    # codes: it names the one instrument holding it, its ISIN derived.
    by_ric = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=C|22=5|48=HOLN.S|10=0|")
    assert held.fill(by_ric) and by_ric.isincode == HOLCIM and by_ric.securityids.is_derived("isin")
    assert by_ric.instcode == HOLCIM


def test_one_isin_on_two_markets_is_two_listings_and_every_learn_moves_lastunix(codec: FixCodec) -> None:
    held = Instruments()
    utc = datetime.timezone.utc
    first, second, later = (datetime.datetime(2026, 1, 2, hour, tzinfo=utc) for hour in (9, 10, 11))

    def at(instant: datetime.datetime, line: bytes) -> FixMsg:
        return FixCodec(codec.registry, default_sending_time=instant).parse_fix_line(line)

    isin = b"|22=4|48=" + HOLCIM.encode()
    assert held.learn(at(first, b"8=FIX.4.4|35=D|11=A" + isin + b"|55=HOLN|207=XSWX|15=CHF|10=0|"))
    assert held.learn(at(second, b"8=FIX.4.4|35=D|11=B" + isin + b"|55=HOLNL|207=XLON|15=GBP|10=0|"))
    assert (len(held), held.rows) == (1, 1), "one instrument - one row - two listings"
    listings = held.listings(HOLCIM)
    assert listings == [listing("XLON", "HOLNL", "GBP"), listing("XSWX", "HOLN", "CHF")], "in MIC order"
    row = held.get(HOLCIM)
    assert row is not None and row["listings"] == listings, "nested in the instrument's row"
    assert held.get_listing(HOLCIM, "XSWX") == listings[1] and held.get_listing(HOLCIM, "XLON") == listings[0]
    assert held.get_listing(HOLCIM, "XNAS") is None and held.get_listing(APPLE, "XSWX") is None
    assert held.listings(APPLE) == []
    with pytest.raises(ValueError):
        held.get_listing(HOLCIM, "TOOLONG")
    assert held.get_by_ticker("HOLNL", "XLON") == row and held.get_by_ticker("HOLN", "XLON") is None
    # `lastunix` is the latest instant a statement met the instrument, and
    # `updunix` the instant a fact last moved.
    assert row["lastunix"] == second
    stamp = row["updunix"]
    # A later message stating the ISIN alone teaches no fact, yet moves
    # `lastunix` and leaves `updunix` where it was.
    assert held.learn(at(later, b"8=FIX.4.4|35=D|11=C" + isin + b"|10=0|")), "the instant moved"
    row = held.get(HOLCIM)
    assert row is not None and (row["lastunix"], row["updunix"]) == (later, stamp)
    assert not held.learn(at(first, b"8=FIX.4.4|35=D|11=D" + isin + b"|10=0|")), "an earlier instant keeps the later"
    removed = held.remove_listing(HOLCIM, "XLON")
    assert removed == listings[0]
    assert held.listings(HOLCIM) == listings[1:] and held.remove_listing(HOLCIM, "XLON") is None
    assert held.remove_listing(HOLCIM, "XSWX") == listings[1]
    assert len(held) == 1, "the instrument stays without a listing"


def test_enrich_learns_then_fills(codec: FixCodec) -> None:
    held = Instruments()
    assert held.enrich(stated(codec))
    assert len(held) == 1


def test_a_row_merges_by_the_update_rule() -> None:
    held = Instruments()
    assert held.merge({"isin": HOLCIM, "listings": [listing("XSWX", "HOLN", ric="HOLN.S")], "updunix": 10})
    assert not held.merge({"isin": HOLCIM, "listings": [listing("XSWX", "HOLN", ric="HOLN.S")]}), "nothing new moves nothing"
    assert held.merge({"isin": HOLCIM, "listings": [listing("XSWX", ric="HOLN.VX")], "updunix": 5}), "a valid value replaces whatever the time"
    assert held.get_listing(HOLCIM, "XSWX") == listing("XSWX", "HOLN", "CHF", ric="HOLN.VX")
    row = held.get(HOLCIM)
    assert row is not None and row["updunix"] is not None
    assert not held.merge({"isin": HOLCIM, "securityids": {"cusip": "037833101"}}), "a typo under a checked code moves nothing"
    assert held.merge({"isin": HOLCIM, "securityids": {"cusip": "037833100"}, "countrycode": "LI"})
    row = held.get(HOLCIM)
    assert row is not None and (row["securityids"]["cusip"], row["countrycode"]) == ("037833100", "LI")
    # A second market is a second listing, carrying its own facts.
    assert held.merge({"isin": HOLCIM, "listings": [listing("XLON", "HOLNL")]}), "a second market, a second listing"
    assert (len(held), held.rows) == (1, 1), "one row, its listings nested"
    assert [(each["miccode"], each["ticker"]) for each in held.listings(HOLCIM)] == [("XLON", "HOLNL"), ("XSWX", "HOLN")]
    with pytest.raises(ValueError, match="crosscode"):
        held.merge({"listings": [listing("XSWX", ric="HOLN.S")]})
    with pytest.raises(ValueError, match="ric"):
        held.merge({"isin": HOLCIM, "ric": "HOLN.S"})
    removed = held.remove(HOLCIM)
    assert removed is not None and removed["crosscode"] == HOLCIM and len(removed["listings"]) == 2
    assert held.remove(HOLCIM) is None and held.rows == len(held) == 0
    held.merge({"isin": HOLCIM})
    held.clear()
    assert len(held) == 0


def test_metadata_holds_the_complementary_facts_and_every_source_keeps_its_identifier() -> None:
    held = Instruments()
    assert held.merge({"isin": HOLCIM, "metadata": {"issuer": "Holcim Ltd"}, "securityids": {"ullink:isin": HOLCIM}})
    row = held.get(HOLCIM)
    assert row is not None and row["metadata"] == {"issuer": "Holcim Ltd"}
    assert row["securityids"]["ullink:isin"] == HOLCIM and row["securityids"]["isin"] == HOLCIM
    hashcode, crossuuid = row["hashcode"], row["crossuuid"]
    assert not held.merge({"isin": HOLCIM, "metadata": {"issuer": "Holcim Ltd"}}), "an equal value moves nothing"
    assert held.merge({"isin": HOLCIM, "metadata": {"issuer": "Holcim AG", "sector": "materials"}}), "replaced and filled"
    row = held.get(HOLCIM)
    assert row is not None and row["metadata"] == {"issuer": "Holcim AG", "sector": "materials"}
    assert row["hashcode"] != hashcode and row["crossuuid"] == crossuuid, "a fact, never the key"
    with pytest.raises(ValueError, match="metadata"):
        held.merge({"isin": HOLCIM, "metadata": {"issuer": "x" * 129}})
    # A golden file's column no field reads lands in each row's metadata
    # under its name: a text cell as it is, any other as its JSON text.
    stored = held.into_arrow_reader().read_all()
    golden = stored.append_column("desk", pa.array(["equities"])).append_column("lot", pa.array([100], pa.int32()))
    read = Instruments.from_arrow_reader(golden).get(HOLCIM)
    assert read is not None and read["metadata"] == {"desk": "equities", "issuer": "Holcim AG", "lot": "100", "sector": "materials"}


def test_the_minted_number_keys_an_fx_pair_no_agency_numbers(codec: FixCodec) -> None:
    assert Instruments.mint("IF:EUR/USD") == EURUSD_MINT
    assert Instruments.mint(OPTION) != Instruments.mint("IF:EUR/USD") and Instruments.mint(OPTION).startswith("QY")
    # A hand-built EUR/USD order: the parse writes the pair's code and its
    # number from the message alone, the lifecycle creates the instrument.
    held = Instruments()
    shared = FixCodec(codec.registry, instruments=held)
    parsed = shared.parse_fix_line(b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:01|11=1|55=EUR/USD|54=1|38=1000000|44=1.0850|10=0|")
    assert (parsed.instcode, parsed.isincode) == ("IF:EUR/USD", EURUSD_MINT)
    [walked] = list(shared.lifecycle([parsed]))
    assert (walked.instcode, walked.isincode) == ("IF:EUR/USD", EURUSD_MINT)
    pair = held.get("IF:EUR/USD")
    assert pair is not None and held.get(EURUSD_MINT) == pair, "the number leads to it"
    assert (pair["crosscode"], pair["isin"], pair["forexcode"], pair["currency"]) == ("IF:EUR/USD", EURUSD_MINT, "EUR/USD", "USD")
    assert pair["securityids"]["yggdryl:isin"] == EURUSD_MINT and pair["countrycode"] is None
    # Another spelling of the pair is the same instrument.
    list(shared.lifecycle([shared.parse_fix_line(b"8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:02|11=2|55=EURUSD CURNCY|54=2|38=500000|10=0|")]))
    assert len(held) == 1
    # A ticker-only security has no instrument and no code.
    [ticker] = list(shared.lifecycle([shared.parse_fix_line(b"8=FIX.4.4|35=D|49=S|56=T|34=3|52=20260102-10:15:03|11=3|55=AAPL|54=1|38=100|10=0|")]))
    assert ticker.instcode is None and len(held) == 1


def test_a_body_rows_own_minted_number_merges_back_without_a_warning(caplog: pytest.LogCaptureFixture) -> None:
    held = Instruments()
    # A statement of facts alone: the element columns and `placeholder` are
    # derived from them, never stated.
    assert held.merge({"cficode": "IFXXXP", "forexcode": "EUR/USD"})
    pair = held.get("IF:EUR/USD")
    assert pair is not None and pair["uuid"] == pair["crossuuid"] != str(uuid.UUID(int=0))
    assert (pair["securityids"]["yggdryl:isin"], pair["isin"], pair["placeholder"]) == (EURUSD_MINT, EURUSD_MINT, False)
    assert held.merge({"cficode": "OCXXXX", "underlying": APPLE, "characteristics": {"expiry": "2026-12-18", "strikepx": "200"}})
    option = held.get(OPTION)
    assert option is not None and option["characteristics"]["expiry"] == "2026-12-18", "nested records by name"
    assert option["listings"] is None and option["isin"] == Instruments.mint(OPTION)
    # A row read back holds the number this crate minted for it: the
    # instrument's own derivation, merged back silently.
    with caplog.at_level(logging.WARNING):
        assert not held.merge(pair), "its own row stated again moves nothing"
        assert not held.merge(option)
    dropped = [record.getMessage() for record in caplog.records if "instrument value dropped" in record.getMessage()]
    assert dropped == [], dropped
    assert held.get("IF:EUR/USD") == pair and held.get(OPTION) == option


def test_the_code_and_the_identity_are_the_pinned_ones() -> None:
    held = Instruments.seeded()
    apple = held.get(APPLE)
    assert apple is not None and apple["crosscode"] == APPLE, "a security keyed by its bare real ISIN"
    assert apple["crosshashcode"] == APPLE_CROSSHASH
    # The identity is the code's XXH3-64 widened as a version 8 UUID - its
    # version nibble and its variant bits over the low half.
    widened = (0x8 << 76) | (0b10 << 62) | (APPLE_CROSSHASH & ((1 << 62) - 1))
    assert apple["uuid"] == apple["crossuuid"] == str(uuid.UUID(int=widened)) == "00000000-0000-8000-a7e3-76388c8738fd"
    assert held.get_by_uuid(apple["uuid"]) == apple
    assert held.get_by_uuid(uuid.UUID(apple["uuid"])) == apple, "a uuid.UUID or its text"
    assert held.get_by_uuid(uuid.uuid4()) is None
    with pytest.raises(ValueError):
        held.get_by_uuid("not a uuid")


def test_a_placeholder_is_rekeyed_by_its_body_and_keeps_its_old_code() -> None:
    held = Instruments()
    assert held.merge({"isin": APPLE, "listings": [listing("XNAS", "AAPL")]})
    assert held.merge({"isin": EUREX, "cficode": "OCXXXX", "listings": [listing("XEUR", "ODAX")]})
    placeholder = held.get(EUREX)
    assert placeholder is not None and placeholder["placeholder"] and placeholder["crosscode"] == EUREX
    body = {"isin": EUREX, "cficode": "OCXXXX", "underlying": APPLE, "characteristics": {"expiry": "2026-12-18", "strikepx": "200"}}
    assert held.merge(body)
    assert len(held) == 2, "re-keyed, not added"
    rekeyed = held.get(OPTION)
    assert rekeyed is not None and held.get(EUREX) == rekeyed, "the old code leads to it"
    assert not rekeyed["placeholder"] and rekeyed["aliascodes"] == [EUREX]
    assert (rekeyed["isin"], rekeyed["underlying"]) == (EUREX, APPLE), "the number stays a fact"
    assert rekeyed["characteristics"]["expiry"] == "2026-12-18"
    assert "yggdryl:isin" not in rekeyed["securityids"], "a number stated, none minted"
    assert rekeyed["listings"] == [listing("XEUR", "ODAX", "EUR")], "the listing stays"
    assert held.get_by_uuid(rekeyed["uuid"]) == rekeyed and rekeyed["uuid"] != placeholder["uuid"]
    assert held.get_by_uuid(placeholder["uuid"]) == rekeyed, "a uuid written before the re-key resolves too"
    # The multiplier is a fact, never a key byte.
    assert held.merge({**body, "characteristics": {"expiry": "2026-12-18", "strikepx": "200.0", "multiplier": "100"}})
    assert held.get(OPTION)["crosscode"] == OPTION and len(held) == 2


def test_every_market_leaf_reads_the_instcode_a_fill_writes(codec: FixCodec) -> None:
    held = Instruments.seeded()
    message = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|22=1|48=037833100|10=0|")
    assert message.instcode is None
    assert held.fill(message) and message.instcode == APPLE and message.isincode == APPLE
    leaf = graph.MarketData(message)
    assert leaf.instcode == APPLE
    order = graph.OrderEvent(1)
    assert order.instcode is None
    rows = graph.MarketData.arrow_reader([message]).read_all()
    assert rows.column("instcode").to_pylist() == [APPLE]


def test_a_walk_learns_the_underlying_a_message_names_and_learn_never_does(codec: FixCodec) -> None:
    line = b"8=FIX.4.4|35=D|11=W|22=4|48=" + NOVARTIS.encode() + b"|55=NOVN|207=XSWX|711=1|311=HOLN|309=" + HOLCIM.encode() + b"|305=4|10=0|"
    held = Instruments()
    shared = FixCodec(codec.registry, instruments=held)
    list(shared.lifecycle([shared.parse_fix_line(line)]))
    row = held.get(NOVARTIS)
    assert row is not None and row["underlying"] is None, "an underlying no instrument keys names none"
    list(shared.lifecycle([shared.parse_fix_line(b"8=FIX.4.4|35=D|11=H|22=4|48=" + HOLCIM.encode() + b"|10=0|")]))
    walked = list(shared.lifecycle([shared.parse_fix_line(line)]))
    row = held.get(NOVARTIS)
    assert row is not None and row["underlying"] == HOLCIM, "the lifecycle learns the underlying's code"
    assert HOLCIM not in str(walked[0].securityids), "lifted nowhere"
    assert b"309=" + HOLCIM.encode() in walked[0].into_bytes(ord("|")), "the wire is the parse's"
    fresh = Instruments()
    assert fresh.learn(codec.parse_fix_line(line))
    assert fresh.get(NOVARTIS)["underlying"] is None, "the bindings' learn reads no underlying"


def test_the_product_category_is_an_instrument_fact_merged_by_the_update_rule() -> None:
    # The EUSIPA product category crosses as its number - an `int` `Eusipa`
    # reads - typed `int32`, right after the legs.
    field = Instruments.field()
    assert field.index_of("eusipacode") == field.index_of("legs") + 1
    assert str(field["eusipacode"].dtype) == "int32"
    held = Instruments()
    assert held.merge({"isin": HOLCIM, "listings": [listing("XSWX", "HOLN")]})
    assert held.get(HOLCIM)["eusipacode"] is None
    assert held.merge({"isin": HOLCIM, "eusipacode": 2300}), "a category fills"
    assert held.get(HOLCIM)["eusipacode"] == 2300
    assert Eusipa(held.get(HOLCIM)["eusipacode"]).name == "Constant Leverage Certificate"
    assert not held.merge({"isin": HOLCIM, "eusipacode": 2300}), "the same category moves nothing"
    assert not held.merge({"isin": HOLCIM}), "a statement of none moves nothing"
    assert held.merge({"isin": HOLCIM, "eusipacode": 2301}), "a category neither map lists is a category"
    assert held.merge({"isin": HOLCIM, "eusipacode": 1260, "updunix": 1}), "whatever the time"
    assert not held.merge({"isin": HOLCIM, "eusipacode": 3100}), "a number of no category's shape is dropped"
    assert held.get(HOLCIM)["eusipacode"] == 1260


def test_a_walk_learns_the_product_category_a_bridge_key_states_and_lifts_it_nowhere(codec: FixCodec) -> None:
    held = Instruments()
    shared = FixCodec(codec.registry, instruments=held)
    line = b"8=FIX.4.4|35=D|11=W|22=4|48=" + NOVARTIS.encode() + b"|EUSIPACode=2300|10=0|"
    [walked] = list(shared.lifecycle([shared.parse_fix_line(line)]))
    row = held.get(NOVARTIS)
    assert row is not None and row["eusipacode"] == 2300, "the lifecycle learns the category"
    assert "2300" not in str(walked.securityids) and "2300" not in str(walked.identifiers), "lifted nowhere"
    assert b"|eusipacode=2300|" in walked.into_bytes(ord("|")).lower(), "the entry stays on the wire"
    # Either map's name, a namespace before it, replaces it.
    other = b"8=FIX.4.4|35=D|11=X|22=4|48=" + NOVARTIS.encode() + b"|OMS_SSPACategory=1260|10=0|"
    list(shared.lifecycle([shared.parse_fix_line(other)]))
    assert held.get(NOVARTIS)["eusipacode"] == 1260
    # Two categories state none, and `learn` alone reads none: the reading is the lifecycle's.
    both = b"8=FIX.4.4|35=D|11=Y|22=4|48=" + NOVARTIS.encode() + b"|EUSIPACode=2300|SSPACategory=2205|10=0|"
    list(shared.lifecycle([shared.parse_fix_line(both)]))
    assert held.get(NOVARTIS)["eusipacode"] == 1260
    fresh = Instruments()
    assert fresh.learn(codec.parse_fix_line(b"8=FIX.4.4|35=D|11=Z|22=4|48=" + NOVARTIS.encode() + b"|55=MINI|207=XSWX|EUSIPACode=2300|10=0|"))
    assert fresh.get(NOVARTIS)["eusipacode"] is None


def test_a_store_written_under_a_row_without_the_cross_code_is_refused_by_name(tmp_path: pathlib.Path) -> None:
    older = pa.table({"isin": [HOLCIM], "miccode": ["XSWX"], "ric": ["HOLN.S"]})
    target = tmp_path / "instruments.arrow"
    LocalFile(target).overwrite_arrow_reader(pa.RecordBatchReader.from_batches(older.schema, older.to_batches()))
    with pytest.raises(ValueError, match="crosscode"):
        Instruments.from_url(target)


def test_the_pipelines_iceberg_table_holds_the_instruments_and_a_pre_instrument_table_is_laid_out_afresh(
    tmp_path: pathlib.Path,
) -> None:
    from tests import medallion
    from yggdryl.iceberg import IcebergCatalog

    silver = IcebergCatalog.open_or_create("silver", tmp_path / "silver")
    # A lake that ran before the instruments: a table of the old row.
    namespace = silver.namespaces.open_or_create(medallion.NAMESPACE)
    older = Field("older", DataType.from_fields([Field("isin", "string", nullable=False), Field("miccode", "string")]), nullable=False)
    namespace.tables.open_or_create("instruments", older)
    table, held = medallion.instruments(silver)
    assert "crosscode" in table.field(), "dropped and created afresh"
    assert held.merge({"isin": NOVARTIS, "underlying": HOLCIM})
    seed = Instruments.seeded()
    assert len(held) == len(seed) and held.commit().written_rows == len(held), "one row per instrument"
    stored = silver.table("record_keeping.instruments")
    assert stored.row_size() == len(held)
    reloaded = Instruments.from_url(stored.url)
    assert reloaded.get(NOVARTIS)["underlying"] == HOLCIM
    assert reloaded.get(HSBC) == held.get(HSBC), "the listings cross nested"


def test_a_ticker_leads_back_to_its_instrument_on_the_same_market() -> None:
    held = Instruments()
    assert held.merge({"isin": HOLCIM, "listings": [listing("XSWX", "HOLN", ric="HOLN.S")]})

    def code(ticker: str, market: str | None = None) -> str | None:
        row = held.get_by_ticker(ticker, market)
        return None if row is None else str(row["crosscode"])

    assert code("HOLN") == code("HOLN", "XSWX") == HOLCIM
    assert code("HOLN", "XXXX") == HOLCIM, "XXXX states no market"
    assert code("HOLN", "XLON") is None and code("ABBN") is None
    with pytest.raises(ValueError):
        held.get_by_ticker("HOLN", "TOOLONG")
    # Two instruments listing one ticker on two markets: a market resolves
    # to its listing, none resolves to neither.
    assert held.merge({"isin": NOVARTIS, "listings": [listing("XLON", "HOLN")]})
    assert (code("HOLN", "XLON"), code("HOLN", "XSWX")) == (NOVARTIS, HOLCIM)
    assert code("HOLN") is None, "ambiguous"
    assert held.remove(NOVARTIS) is not None
    assert code("HOLN") == HOLCIM
    held.clear()
    assert code("HOLN") is None


def test_a_new_instrument_past_the_bound_is_refused_by_merge() -> None:
    held = Instruments(max_instruments=1)
    assert held.max_instruments == 1
    assert held.merge({"isin": HOLCIM})
    with pytest.raises(ValueError, match="1"):
        held.merge({"isin": APPLE})
    assert repr(held) == "Instruments(len=1, max_instruments=1, dirty=True)"


def test_a_full_collection_warns_once_through_logging_and_learns_on(codec: FixCodec, caplog: pytest.LogCaptureFixture) -> None:
    # The warning crosses into Python's `logging` from the core, with no
    # GIL held while the collection's lock is: the call returns.
    held = Instruments(max_instruments=1)
    assert held.merge({"isin": APPLE})
    with caplog.at_level(logging.WARNING):
        assert not held.learn(stated(codec))
        assert not held.learn(stated(codec))
    assert len(held) == 1
    assert sum("instruments full" in record.getMessage() for record in caplog.records) == 1


def test_an_instruments_round_trips_through_a_holder(tmp_path: pathlib.Path) -> None:
    held = Instruments()
    held.merge({"isin": HOLCIM, "cficode": "ESVUFR", "listings": [listing("XSWX", ric="HOLN.S")], "updunix": 7})
    held.merge({"isin": APPLE, "listings": [listing("XNAS", ric="AAPL.OQ")]})
    snapshot = held.into_arrow_reader()
    held.clear()
    table = snapshot.read_all()
    assert table.num_rows == 2, "the stream is a snapshot a later write does not move"
    assert table.column("crosscode").to_pylist() == [HOLCIM, APPLE], "in cross code order"
    assert table.schema.names == [child.name for child in Instruments.field()]
    assert len(table.schema.names) == 25
    target = tmp_path / "instruments.arrow"
    LocalFile(target).overwrite_arrow_reader(Instruments.from_arrow_reader(table).into_arrow_reader())
    loaded = Instruments.from_url(target)
    assert len(loaded) == 2 and loaded.get(HOLCIM) == Instruments.from_arrow_reader(table).get(HOLCIM)
    assert loaded.extend_from_handle(LocalFile(target)) == 2
    assert loaded.extend_from_handle(target) == 2
    assert len(Instruments.from_url(tmp_path / "missing.arrow")) == 0, "a missing store is empty"
    with pytest.raises(TypeError, match="property"):
        Instruments.from_url(target, media_type=1)


def test_an_instruments_commits_to_its_store_only_where_it_moved(tmp_path: pathlib.Path) -> None:
    # A trailing separator is what makes a location that is not there yet
    # a folder rather than a leaf.
    store = str(tmp_path / "instruments") + os.sep
    held = Instruments.from_url(store, max_instruments=8)
    assert len(held) == 0 and not held.is_dirty and held.max_instruments == 8
    assert not (tmp_path / "instruments").exists(), "nothing is laid out before a commit"
    assert held.commit() == IOResult(0, 0), "a clean collection writes nothing"
    assert held.merge({"isin": HOLCIM, "listings": [listing("XSWX", ric="HOLN.S")]})
    assert held.is_dirty
    result = held.commit()
    assert isinstance(result, IOResult) and result.written_rows == 1
    assert not held.is_dirty
    assert (tmp_path / "instruments" / "part-0.arrows").is_file()
    assert held.commit().written_rows == 0, "a second commit writes nothing"
    reloaded = Instruments.from_url(store)
    assert reloaded.get(HOLCIM) == held.get(HOLCIM) and not reloaded.is_dirty
    # Emptied: cleared on the next commit.
    held.clear()
    held.commit()
    assert len(Instruments.from_url(store)) == 0
    # An unbound collection has nowhere to commit.
    with pytest.raises(ValueError, match="holder"):
        Instruments().commit()


def test_a_codec_shares_the_callers_instruments_with_every_lifecycle_and_every_parse(codec: FixCodec) -> None:
    held = Instruments()
    shared = FixCodec(codec.registry, instruments=held)
    assert shared.instruments == held and codec.instruments is None
    assert shared.with_dedup_window_ms(None).instruments == held, "a derived codec keeps it"
    list(shared.lifecycle([stated(shared)]))
    assert len(held) == 1 and held.get(HOLCIM) is not None
    list(codec.lifecycle([stated(codec)]))
    assert len(held) == 1, "a codec without one learns into its own"
    # A parse through the sharing codec fills derived identifiers from the
    # table, learns nothing, and leaves the identity the bare parse gives -
    # the two read under one pinned clock, so an undated line dates alike.
    pinned = datetime.datetime(2026, 1, 2, 10, 15, 30, tzinfo=datetime.timezone.utc)
    line = b"8=FIX.4.4|35=D|11=P|55=HOLN|207=XSWX|10=0|"
    filled = FixCodec(codec.registry, instruments=held, default_sending_time=pinned).parse_fix_line(line)
    bare = FixCodec(codec.registry, default_sending_time=pinned).parse_fix_line(line)
    assert filled.isincode == HOLCIM and filled.securityids.is_derived("isin")
    assert filled.securityids.get("ric") == "HOLN.S"
    assert bare.isincode is None
    assert (filled.uuid, filled.hashcode, filled.into_bytes(ord("|"))) == (
        bare.uuid,
        bare.hashcode,
        bare.into_bytes(ord("|")),
    )
    assert filled.cficode is None, "a parse fills identifiers only"
    assert not held.learn(filled), "nothing a parse derived is learned back"


def test_the_process_instruments_are_the_sealed_store_and_the_codec_the_environment_names_shares_them() -> None:
    # The suite's conftest points `YGGDRYL_INSTRUMENTS_URI` at a session
    # folder before anything resolves the default.
    default = Instruments.from_env()
    assert default == Instruments.from_env(), "resolved once"
    assert not default.is_dirty
    apple = default.get(APPLE)
    assert apple is not None and apple["listings"][0]["ticker"] == "AAPL", "the store is laid over the seed"
    codec = FixCodec.from_env()
    assert codec.instruments == default
    own = Instruments()
    assert FixCodec.from_env(instruments=own).instruments == own, "a stated pin stands"
    assert FixCodec(FixRegistry.from_env()).instruments is None, "a codec built by hand attaches none"
    with pytest.raises(ValueError, match="already resolved"):
        Instruments.install_env(Instruments())
    assert "YGGDRYL_INSTRUMENTS_URI" in os.environ, "the suite seals the default"


def test_the_seed_holds_the_common_instruments_clean_and_bound_to_nothing() -> None:
    seeded = Instruments.seeded()
    assert (len(seeded), seeded.rows) == (208, 208) and not seeded.is_dirty
    assert [each["miccode"] for each in seeded.listings(HSBC)] == ["XHKG", "XLON"], "HSBC lists on two markets"
    assert len(Instruments()) == 0, "a collection built by hand holds none of it"
    apple = seeded.get(APPLE)
    assert apple is not None
    assert (apple["listings"], apple["fisn"]) == ([listing("XNAS", "AAPL", "USD")], "APPLE INC/SH SH")
    assert apple["securityids"]["cusip"] == "037833100", "the CUSIP its ISIN embeds"
    assert seeded.get_by_ticker("AAPL", "XNAS") == apple
    assert Instruments.seeded() != seeded, "each call is a collection of its own"
    with pytest.raises(ValueError, match="unbound"):
        seeded.commit()


def test_a_store_bound_seeded_is_laid_over_the_seed(tmp_path: pathlib.Path) -> None:
    target = tmp_path / "instruments.arrows"
    store = Instruments.from_url(target)
    assert store.merge({"isin": APPLE, "listings": [listing("XNAS", "AAPL", "CHF")]})
    bae = "GB0002634946"
    assert store.merge({"isin": bae, "listings": [listing("XLON")]})
    store.commit()
    seed = Instruments.seeded()
    assert seed.get(bae) is None
    held = Instruments.seeded_from_url(target, max_instruments=1024)
    assert len(held) == len(seed) + 1 and not held.is_dirty and held.max_instruments == 1024
    apple = held.get(APPLE)
    assert apple is not None and apple["listings"][0]["currency"] == "CHF", "the store's value wins"
    seeded = seed.get(APPLE)
    assert seeded is not None and apple["fisn"] == seeded["fisn"] == "APPLE INC/SH SH", "the seed's fact stands"
    assert held.get(bae) == store.get(bae), "an instrument only the store holds"
    assert held.commit() == IOResult(0, 0), "clean after the load"
    assert len(Instruments.from_url(target)) == 2, "unseeded: the store's rows alone"
    assert held.merge({"isin": HOLCIM, "listings": [listing("XSWX", ric="HOLN.S")]})
    assert held.commit().written_rows == len(held), "the seed's instruments with the store's"
    assert len(Instruments.from_url(target)) == len(held)
    assert len(Instruments.seeded_from_url(LocalFile(target))) == len(held), "a handle names the store too"
    with pytest.raises(TypeError, match="properties"):
        Instruments.seeded_from_url(LocalFile(target), media_type="x")
    first = Instruments.seeded_from_url(str(tmp_path / "instruments") + os.sep)
    assert len(first) == len(seed) and not first.is_dirty, "a first run: the seed bound to the store"
    assert first.commit() == IOResult(0, 0) and not (tmp_path / "instruments").exists()


def test_the_row_declares_the_code_partition_and_the_code_order() -> None:
    field = Instruments.field()
    assert field.metadata["PARTITION:by"] == '["truncate(crosscode, 2)"]'
    assert field.metadata["SORT:by"] == '["crosscode"]', "one row per instrument"
    names = [child.name for child in field]
    assert names[:8] == ["uuid", "crossuuid", "crosscode", "hashcode", "crosshashcode", "srcuuids", "aliascodes", "placeholder"]
    assert names[-4:] == ["metadata", "updunix", "firstunix", "lastunix"]


def test_the_short_name_merges_by_the_update_rule() -> None:
    held = Instruments()
    assert held.merge({"isin": HOLCIM, "fisn": "HOLCIM LTD/SH"})
    assert held.get(HOLCIM)["fisn"] == "HOLCIM LTD/SH"
    assert held.merge({"isin": HOLCIM, "fisn": "HOLCIM AG/SH"}), "a stated value replaces one that differs"
    assert held.get(HOLCIM)["fisn"] == "HOLCIM AG/SH"


def test_a_row_carries_the_defaults_its_facts_imply_where_it_states_none() -> None:
    held = Instruments()
    bae = "GB0002634946"
    assert held.merge({"isin": bae, "listings": [listing("XLON")]})
    row = held.get(bae)
    assert row is not None
    # The SEDOL a GB '00' ISIN embeds is a listing code of the London
    # listing, and the currency the listing market's country's.
    assert row["listings"] == [listing("XLON", None, "GBP", sedol="0263494")]
    # A stated value is never replaced by a default.
    assert held.merge({"isin": bae, "listings": [listing("XLON", currency="USD")]})
    assert held.get_listing(bae, "XLON")["currency"] == "USD"


def test_an_instruments_is_equal_only_to_itself_and_never_hashed_or_pickled() -> None:
    held = Instruments()
    assert held == held and held != Instruments()
    with pytest.raises(TypeError):
        hash(held)
    with pytest.raises(TypeError):
        pickle.dumps(held)
    with pytest.raises(TypeError):
        copy.deepcopy(held)


def test_the_first_learn_sets_firstunix_and_only_an_earlier_event_moves_it(codec: FixCodec) -> None:
    utc = datetime.timezone.utc
    early, first, later = (datetime.datetime(2026, 1, 2, hour, tzinfo=utc) for hour in (8, 9, 11))

    def met(instant: datetime.datetime) -> FixMsg:
        line = b"8=FIX.4.4|35=D|11=A|22=4|48=" + HOLCIM.encode() + b"|10=0|"
        return FixCodec(codec.registry, default_sending_time=instant).parse_fix_line(line)

    held = Instruments()
    assert held.field().index_of("firstunix") == held.field().index_of("updunix") + 1
    assert held.field().index_of("lastunix") == held.field().index_of("updunix") + 2
    assert held.learn(met(first))
    row = held.get(HOLCIM)
    assert row is not None and (row["updunix"], row["firstunix"], row["lastunix"]) == (first, first, first)
    assert held.learn(met(later)), "a later event moves lastunix"
    row = held.get(HOLCIM)
    assert row is not None and (row["firstunix"], row["lastunix"]) == (first, later), "not firstunix"
    assert held.learn(met(early)), "replayed out of order"
    row = held.get(HOLCIM)
    assert row is not None
    assert (row["updunix"], row["firstunix"], row["lastunix"]) == (first, early, later), "updunix moves with a fact alone"
    clean = Instruments.from_arrow_reader(held.into_arrow_reader())
    assert not clean.learn(met(first)) and not clean.is_dirty, "between the two: nothing moves"


def _held(*rows: dict[str, object]) -> Instruments:
    held = Instruments()
    for row in rows:
        assert held.merge(row)
    return held


def _order(*codes: tuple[str, str], **facts: object) -> graph.OrderEvent:
    return graph.OrderEvent(1, securityids=[Identifier(kind, value) for kind, value in codes], **facts)


def test_the_cascade_takes_the_isin_then_a_code_then_the_ticker_on_its_market() -> None:
    held = _held(
        {"isin": APPLE, "listings": [listing("XNAS", "AAPL")]},
        {"isin": DIAGEO, "listings": [listing("XLON", "DGE")]},
        {"isin": SAP, "listings": [listing("XETR")]},
    )
    # The ISIN wins over a CUSIP naming Apple.
    by_isin = held.resolve(_order(("isin", SAP), ("cusip", "037833100")))
    assert isinstance(by_isin, Resolution) and by_isin and by_isin.matched
    assert (by_isin.tier, by_isin.kind, by_isin.derived, by_isin.listing) == ("isin", None, False, True)
    assert by_isin.entry is not None and by_isin.entry["crosscode"] == SAP
    assert (by_isin.unmatched, by_isin.codes, by_isin.stated, by_isin.similarity) == (None, None, None, None)
    # A CUSIP wins over a ticker naming Diageo; Apple is not listed on XLON.
    by_code = held.resolve(_order(("cusip", "037833100"), ticker="DGE", miccode="XLON"))
    assert (by_code.tier, by_code.kind, by_code.derived, by_code.listing) == ("code", "cusip", True, False)
    assert by_code.entry is not None and by_code.entry["crosscode"] == APPLE
    # The ticker on its market last.
    by_ticker = held.resolve(_order(ticker="DGE", miccode="XLON"))
    assert (by_ticker.tier, by_ticker.derived, by_ticker.listing) == ("symbology", True, True)
    assert by_ticker.entry is not None and by_ticker.entry["crosscode"] == DIAGEO
    # A FIX message and a `MarketData` resolve as any leaf does.
    assert held.resolve(graph.MarketData(_order(("isin", SAP)))) == by_isin
    with pytest.raises(TypeError, match="market leaf"):
        held.resolve("US0378331005")
    # The public lookups.
    apple = held.get_by_code("cusip", "037833100")
    assert apple is not None and apple["crosscode"] == APPLE
    diageo = held.get_by_code("SEDOL", "0237400")
    assert diageo is not None and diageo["crosscode"] == DIAGEO, "the SEDOL its ISIN embeds, any spelling of its type"
    assert held.get_by_code("cusip", "not a cusip") is None
    assert held.get_by_code("isoccy", "USD") is None, "no lookup code"
    assert "isoccy" not in Instruments.LOOKUP_CODES and Instruments.LOOKUP_CODES[:3] == ("cusip", "sedol", "wkn")
    # Nothing stated: no key; a ticker nobody lists: no candidate.
    nothing = held.resolve(_order())
    assert not nothing and nothing.unmatched == "NoKey"
    assert (nothing.entry, nothing.tier, nothing.derived, nothing.listing) == (None, None, None, None)
    assert held.resolve(_order(ticker="ZZZZ")).unmatched == "NoCandidate"


def test_the_cross_code_an_fx_element_spells_resolves_it(codec: FixCodec) -> None:
    held = Instruments()
    assert held.learn(codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|55=EUR/USD|54=1|38=1000000|10=0|"))
    pair = held.resolve(codec.parse_fix_line(b"8=FIX.4.4|35=D|11=B|55=EUR/USD|54=2|38=1000|10=0|"))
    assert pair.matched and pair.entry is not None and pair.entry["crosscode"] == "IF:EUR/USD"
    assert (pair.tier, pair.derived) == ("crosscode", True), "the code the pair and its class spell"
    unknown = held.resolve(codec.parse_fix_line(b"8=FIX.4.4|35=D|11=C|55=GBP/USD|54=1|38=1000|10=0|"))
    assert (unknown.matched, unknown.unmatched, unknown.stated) == (False, "UnknownCode", "IF:GBP/USD")


def test_an_unknown_stated_isin_ends_the_cascade_and_fills_nothing(codec: FixCodec) -> None:
    held = _held({"isin": APPLE, "listings": [listing("XNAS")]})
    unknown = held.resolve(_order(("isin", NOVARTIS), ("cusip", "037833100")))
    assert (unknown.matched, unknown.unmatched, unknown.stated) == (False, "UnknownIsin", NOVARTIS)
    assert unknown.code is None and unknown.held is None
    message = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|22=4|48=" + NOVARTIS.encode() + b"|10=0|")
    assert held.resolve(message) == unknown
    assert not held.fill(message) and message.isincode == NOVARTIS and message.cficode is None


def test_a_code_two_instruments_hold_is_ambiguous_and_stops_the_cascade() -> None:
    held = _held(
        {"isin": APPLE, "securityids": {"common": "C-1"}},
        {"isin": SAP, "securityids": {"common": "C-1"}},
        {"isin": DIAGEO, "listings": [listing("XLON", "DGE")]},
    )
    ambiguous = held.resolve(_order(("common", "C-1"), ticker="DGE", miccode="XLON"))
    assert (ambiguous.matched, ambiguous.unmatched, ambiguous.tier, ambiguous.kind) == (False, "Ambiguous", "code", "common")
    assert ambiguous.codes == [SAP, APPLE], "in code order"
    assert held.get_by_code("common", "C-1") is None
    # One instrument on two markets is one answer, never an ambiguity.
    hsbc = Instruments.seeded().resolve(_order(("sedol", "0540528")))
    assert (hsbc.tier, hsbc.kind, hsbc.derived) == ("code", "sedol", True)
    assert hsbc.entry is not None and hsbc.entry["crosscode"] == HSBC


def test_the_economic_tier_matches_a_similar_short_name_in_the_same_currency() -> None:
    def named(name: str, **facts: object) -> graph.OrderEvent:
        return _order(("fisn", name), **{"currency": "USD", **facts})

    def held_of(name: str, cfi: str) -> Instruments:
        return _held({"isin": APPLE, "listings": [listing("XNAS")], "fisn": name, "cficode": cfi})

    similar = held_of("APPLE INC./SH", "ESVUFR").resolve(named("APPLE INC/SH"))
    assert (similar.tier, similar.derived, similar.listing) == ("economic", True, True)
    assert similar.similarity is not None and math.isclose(similar.similarity, 12 / 13)
    assert similar.entry is not None and similar.entry["crosscode"] == APPLE

    plain = held_of("APPLE INC/SH", "ESVUFR")
    below = plain.resolve(named("APPLE INC/SH USD"))
    assert (below.unmatched, below.best, below.code) == ("BelowThreshold", 0.75, APPLE)

    conflict = held_of("APPLE INC/SH", "DBFTFR").resolve(named("APPLE INC/SH", cficode="ESVUFR"))
    assert (conflict.unmatched, conflict.stated, conflict.held, conflict.code) == ("CfiConflict", "E", "D", APPLE)
    assert plain.resolve(named("APPLE INC/SH", cficode="XXXXXX")).tier == "economic", "unclassified conflicts with nothing"
    assert plain.resolve(named("APPLE INC/SH", currency="EUR")).unmatched == "NoCandidate", "another currency"

    issued = _held({"isin": APPLE, "listings": [listing("XNAS")], "fisn": "APPLE INC/SH", "origccy": "USD"})
    origin = issued.resolve(named("APPLE INC/SH", origccy="EUR"))
    assert (origin.unmatched, origin.stated, origin.held, origin.code) == ("CurrencyConflict", "EUR", "USD", APPLE)

    twins = held_of("APPLE INC/SH", "ESVUFR")
    assert twins.merge({"isin": MICROSOFT, "listings": [listing("XNYS")], "fisn": "APPLE INC/SH"})
    tied = twins.resolve(named("APPLE INC/SH"))
    assert (tied.unmatched, tied.tier, tied.similarity, tied.codes) == ("Ambiguous", "economic", 1.0, [APPLE, MICROSOFT])

    assert plain.resolve(_order(ticker="ZZZZ", currency="USD")).unmatched == "NoCandidate", "no short name"
    assert plain.resolve(named("APPLE INC/SH", currency="XXX")).unmatched == "NoCandidate", "XXX states no currency"


def test_a_fill_takes_the_economic_match_only_where_it_is_enabled(codec: FixCodec) -> None:
    held = _held({"isin": APPLE, "listings": [listing("XNAS")], "fisn": "APPLE INC./SH"})
    assert not held.is_economic_match
    assert held.economic_threshold == Instruments.DEFAULT_ECONOMIC_THRESHOLD == 0.85

    def named() -> FixMsg:
        return codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|2737=APPLE INC/SH|15=USD|10=0|")

    off = named()
    assert held.resolve(off).tier == "economic", "resolve weighs it always"
    assert not held.fill(off) and off.isincode is None, "a judgement no fill takes unasked"
    held.set_economic_match(True)
    assert held.is_economic_match
    on = named()
    assert held.fill(on) and on.isincode == APPLE and on.securityids.is_derived("isin")
    assert on.instcode == APPLE
    held.set_economic_threshold(0.95)
    assert not held.fill(named()), "a stricter threshold refuses what it took"
    for refused in (0.0, 1.5, math.nan, -0.5):
        spelled = "NaN" if math.isnan(refused) else f"{refused:g}"
        with pytest.raises(ValueError, match=re.escape(spelled)):
            held.set_economic_threshold(refused)
    assert held.economic_threshold == 0.95, "a refusal moves nothing"
    held.set_economic_threshold(1.0)
    held.clear()
    assert (held.economic_threshold, held.is_economic_match) == (1.0, True), "the collection's, never the store's"


def test_the_origin_currency_is_stated_or_filled_and_reads_the_currency_where_unheld(codec: FixCodec) -> None:
    euro = graph.OrderEvent(1, currency="EUR")
    assert euro.origccy is None and euro.origin_currency.as_py() == "EUR", "defaulted by currency, never stored"
    issued = graph.OrderEvent(1, currency="EUR", origccy="USD")
    assert issued.origccy is not None and issued.origccy.as_py() == "USD"
    assert (issued.origin_currency.as_py(), issued.currency.as_py()) == ("USD", "EUR")
    assert graph.MarketData(issued).origin_currency.as_py() == "USD"
    assert graph.OrderEvent(1).origin_currency.as_py() == "XXX"
    # A row's cell is the held value and null otherwise.
    rows = graph.MarketData.arrow_reader([euro, issued]).read_all()
    assert rows.column("origccy").to_pylist() == [None, "USD"]

    # The instrument's stated origin currency fills an element stating none,
    # and never one that does; nothing derives it into a column.
    held = _held({"isin": APPLE, "listings": [listing("XNAS")], "origccy": "USD"}, {"isin": SAP, "listings": [listing("XETR")]})
    assert Instruments.field().index_of("origccy") == Instruments.field().index_of("currency") + 1
    sap = held.get(SAP)
    assert sap is not None and sap["listings"][0]["currency"] == "EUR" and sap["origccy"] is None
    line = b"8=FIX.4.4|35=D|11=A|22=4|48=" + APPLE.encode() + b"|15=EUR|10=0|"
    unstated = codec.parse_fix_line(line)
    assert unstated.origccy is None and unstated.origin_currency.as_py() == "EUR"
    assert held.fill(unstated)
    assert unstated.origccy is not None and unstated.origccy.as_py() == "USD"
    assert unstated.origin_currency.as_py() == "USD" and unstated.currency.as_py() == "EUR"
    own = codec.parse_fix_line(line)
    own.set("origccy", "CHF")
    held.fill(own)
    assert own.origccy is not None and own.origccy.as_py() == "CHF", "a statement stands"


def test_a_resolution_is_equal_by_value_and_never_hashed() -> None:
    held = _held({"isin": APPLE, "listings": [listing("XNAS")]})
    first, second = (held.resolve(_order(("isin", APPLE))) for _ in range(2))
    assert first == second and first != held.resolve(_order())
    assert repr(first) == "Resolution(matched=True, crosscode='US0378331005', tier='isin', derived=False, listing=True)"
    assert repr(held.resolve(_order(("isin", NOVARTIS)))) == (
        "Resolution(matched=False, unmatched='UnknownIsin', stated='CH0012005267')"
    )
    with pytest.raises(TypeError):
        hash(first)
    with pytest.raises(TypeError):
        Resolution()  # type: ignore[call-arg]
