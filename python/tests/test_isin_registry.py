"""`python/yggdryl/isin_registry.py` and `python/src/isin_registry.rs`: the
instrument registry, one row per ISIN and market of every fact it is known by,
learned from and filled into FIX messages, bound to the store it is loaded from and
committed back to, redirected to the core."""

from __future__ import annotations

import copy
import datetime
import logging
import math
import os
import pathlib
import pickle
import re

import pyarrow as pa
import pytest

from yggdryl import Eusipa, Identifier, IOResult, IsinRegistry, graph
from yggdryl.holder import LocalFile
from yggdryl.fix import FixCodec, FixMsg, FixRegistry
from yggdryl.isin_registry import Resolution

SEED = pathlib.Path(__file__).resolve().parents[2] / "config" / "fix"
HOLCIM = "CH0012214059"
APPLE = "US0378331005"
NOVARTIS = "CH0012005267"
DIAGEO = "GB0002374006"
SAP = "DE0007164600"
HSBC = "GB0005405286"
MICROSOFT = "US5949181045"


@pytest.fixture(scope="module")
def codec() -> FixCodec:
    return FixCodec(FixRegistry.from_handle(SEED))


def stated(codec: FixCodec) -> object:
    """A message stating Holcim's ISIN, its RIC, its CFI code, its ticker, its market and its currency."""
    return codec.parse_fix_line(
        b"8=FIX.4.4|35=D|11=A|22=4|48=" + HOLCIM.encode() + b"|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|"
    )


def test_a_registry_learns_a_message_and_fills_a_later_one_named_by_its_ticker(codec: FixCodec) -> None:
    registry = IsinRegistry()
    assert not registry and len(registry) == 0 and not registry.is_dirty
    assert registry.learn(stated(codec))
    assert registry.is_dirty
    row = registry.get(HOLCIM)
    assert row is not None
    # The three stamps are the message's instant, which the undated line
    # takes from the clock: present, and left out of the facts compared.
    stamps = ("updunix", "firstunix", "lastunix")
    assert all(row[stamp] is not None for stamp in stamps)
    assert {key: value for key, value in row.items() if value is not None and key not in stamps} == {
        "cficode": "ESVUFR",
        "currency": "CHF",
        "isin": HOLCIM,
        "miccode": "XSWX",
        "ric": "HOLN.S",
        "ticker": "HOLN",
        "valor": "1221405",
    }
    # The row's Valor is the one its own ISIN embeds, filled at the fold as a
    # default every row of a Swiss ISIN carries - not the message's derived
    # identifier, which a learn still never reads.
    assert row["countrycode"] is None, "the prefix already says the country"
    assert registry.get_by_ticker("HOLN", "XSWX") == row and registry.get_by_ticker("HOLN", "XLON") is None
    later = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|")
    assert later.isincode is None
    assert registry.fill(later)
    assert later.isincode == HOLCIM and later.securityids.is_derived("isin")
    assert later.securityids.get("ric") == "HOLN.S" and later.securityids.is_derived("ric")
    assert later.cficode is not None and later.cficode.as_py() == "ESVUFR" and later.ticker == "HOLN"
    assert later.currency.as_py() == "CHF"
    assert not registry.fill(later), "nothing left to fill"
    wire = later.into_bytes(ord("|"))
    assert b"461=" not in wire and b"15=" not in wire and b"48=" not in wire, "never the wire"
    # A RIC is a listing code and, with no ISIN stated, one of the lookup
    # codes (`IsinRegistry.LOOKUP_CODES`, since local codes became keys): it
    # names the one instrument holding it, its ISIN derived.
    by_ric = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=C|22=5|48=HOLN.S|10=0|")
    assert registry.fill(by_ric) and by_ric.isincode == HOLCIM and by_ric.securityids.is_derived("isin")


def test_one_isin_on_two_markets_is_two_listings_and_every_learn_moves_lastunix(codec: FixCodec) -> None:
    registry = IsinRegistry()
    utc = datetime.timezone.utc
    first, second, later = (datetime.datetime(2026, 1, 2, hour, tzinfo=utc) for hour in (9, 10, 11))

    def at(instant: datetime.datetime, line: bytes) -> object:
        return FixCodec(codec.registry, default_sending_time=instant).parse_fix_line(line)

    isin = b"|22=4|48=" + HOLCIM.encode()
    assert registry.learn(at(first, b"8=FIX.4.4|35=D|11=A" + isin + b"|55=HOLN|207=XSWX|15=CHF|10=0|"))
    assert registry.learn(at(second, b"8=FIX.4.4|35=D|11=B" + isin + b"|55=HOLNL|207=XLON|15=GBP|10=0|"))
    assert (len(registry), registry.rows) == (1, 2), "one instrument, two listings"
    listings = registry.listings(HOLCIM)
    assert [(row["miccode"], row["ticker"], row["currency"]) for row in listings] == [
        ("XLON", "HOLNL", "GBP"),
        ("XSWX", "HOLN", "CHF"),
    ], "in MIC order, each its own listing facts"
    assert registry.get(HOLCIM) == listings[0], "the first listing in MIC order"
    assert registry.get_listing(HOLCIM, "XSWX") == listings[1] and registry.get_listing(HOLCIM, "XLON") == listings[0]
    assert registry.get_listing(HOLCIM, "XNAS") is None and registry.get_listing(APPLE, "XSWX") is None
    assert registry.listings(APPLE) == []
    with pytest.raises(ValueError):
        registry.get_listing(HOLCIM, "TOOLONG")
    assert registry.get_by_ticker("HOLNL", "XLON") == listings[0] and registry.get_by_ticker("HOLN", "XLON") is None
    # `lastunix` is an instrument fact - every listing holds the latest
    # instant - and `updunix` the instant a fact last moved.
    assert {row["lastunix"] for row in listings} == {second}
    stamps = {row["miccode"]: row["updunix"] for row in listings}
    # A later message stating the ISIN alone teaches no fact, yet moves
    # `lastunix` on every listing and leaves `updunix` where it was.
    assert registry.learn(at(later, b"8=FIX.4.4|35=D|11=C" + isin + b"|10=0|")), "the instant moved"
    assert registry.is_dirty
    relisted = registry.listings(HOLCIM)
    assert [row["lastunix"] for row in relisted] == [later, later]
    assert {row["miccode"]: row["updunix"] for row in relisted} == stamps, "no fact moved"
    assert not registry.learn(at(first, b"8=FIX.4.4|35=D|11=D" + isin + b"|10=0|")), "an earlier instant keeps the later"
    removed = registry.remove_listing(HOLCIM, "XLON")
    assert removed is not None and removed["miccode"] == "XLON"
    assert (len(registry), registry.rows) == (1, 1) and registry.remove_listing(HOLCIM, "XLON") is None
    assert registry.remove_listing(HOLCIM, "XSWX") is not None
    assert (len(registry), registry.rows) == (0, 0), "the instrument goes with its last listing"


def test_enrich_learns_then_fills(codec: FixCodec) -> None:
    registry = IsinRegistry()
    assert registry.enrich(stated(codec))
    assert len(registry) == 1


def test_a_row_merges_by_the_update_rule(codec: FixCodec) -> None:
    registry = IsinRegistry()
    assert registry.merge({"isin": HOLCIM, "ric": "HOLN.S", "updunix": 10})
    assert not registry.merge({"isin": HOLCIM, "ric": "HOLN.S"}), "a row stating nothing new moves nothing"
    assert registry.merge({"isin": HOLCIM, "ric": "HOLN.VX", "updunix": 5}), "a valid value replaces whatever the time"
    row = registry.get(HOLCIM)
    assert row is not None and row["ric"] == "HOLN.VX" and row["updunix"] is not None
    assert not registry.merge({"isin": HOLCIM, "cusip": "037833101"}), "a typo under a checked code moves nothing"
    assert registry.merge({"isin": HOLCIM, "cusip": "037833100", "countrycode": "LI", "currency": "CHF"})
    row = registry.get(HOLCIM)
    assert row is not None and (row["cusip"], row["countrycode"], row["currency"]) == ("037833100", "LI", "CHF")
    assert row["underlyingisin"] is None
    assert registry.merge({"isin": HOLCIM, "underlyingisin": APPLE}), "an instrument fact fills"
    assert not registry.merge({"isin": HOLCIM, "underlyingisin": HOLCIM}), "the row's own ISIN states nothing"
    assert not registry.merge({"isin": HOLCIM, "underlyingisin": "US0378331006"}), "a typo is dropped"
    # The ISIN's one row states no market yet, so the first market a
    # statement names becomes that row's; a second market is a second
    # listing row, carrying the instrument facts beside its own.
    assert registry.merge({"isin": HOLCIM, "miccode": "XLON", "ticker": "HOLNL"}), "the unlisted row takes its market"
    assert (len(registry), registry.rows) == (1, 1)
    assert registry.merge({"isin": HOLCIM, "miccode": "XSWX", "ticker": "HOLN"}), "a second market, a second listing"
    assert (len(registry), registry.rows) == (1, 2)
    assert [(row["miccode"], row["ticker"], row["underlyingisin"]) for row in registry.listings(HOLCIM)] == [
        ("XLON", "HOLNL", APPLE),
        ("XSWX", "HOLN", APPLE),
    ], "an instrument fact every listing carries"
    with pytest.raises(ValueError, match="isin"):
        registry.merge({"ric": "HOLN.S"})
    removed = registry.remove(HOLCIM)
    assert [(row["isin"], row["miccode"]) for row in removed] == [(HOLCIM, "XLON"), (HOLCIM, "XSWX")], "every listing, in MIC order"
    assert registry.remove(HOLCIM) == [] and registry.rows == 0
    registry.merge({"isin": HOLCIM})
    registry.clear()
    assert len(registry) == 0


def test_a_walk_learns_the_underlying_a_message_names_and_learn_never_does(codec: FixCodec) -> None:
    line = b"8=FIX.4.4|35=D|11=W|22=4|48=CH0012005267|55=NOVN|207=XSWX|711=1|311=HOLN|309=" + HOLCIM.encode() + b"|305=4|10=0|"
    registry = IsinRegistry()
    shared = FixCodec(codec.registry, isin_registry=registry)
    walked = list(shared.lifecycle([shared.parse_fix_line(line)]))
    row = registry.get("CH0012005267")
    assert row is not None and row["underlyingisin"] == HOLCIM, "the lifecycle learns the underlying"
    assert HOLCIM not in str(walked[0].securityids), "lifted nowhere"
    assert b"309=" + HOLCIM.encode() in walked[0].into_bytes(ord("|")), "the wire is the parse's"
    fresh = IsinRegistry()
    assert fresh.learn(codec.parse_fix_line(line))
    assert fresh.get("CH0012005267")["underlyingisin"] is None, "the bindings' learn reads no underlying"


def test_the_product_category_is_an_instrument_fact_merged_by_the_update_rule(codec: FixCodec) -> None:
    # The EUSIPA product category crosses as its number - an `int` `Eusipa`
    # reads - typed `int32`, right after the underlying.
    field = IsinRegistry.field()
    assert field.index_of("eusipacode") == field.index_of("underlyingisin") + 1
    assert str(field["eusipacode"].dtype) == "int32"
    registry = IsinRegistry()
    assert registry.merge({"isin": HOLCIM, "miccode": "XSWX", "ticker": "HOLN"})
    row = registry.get(HOLCIM)
    assert row is not None and row["eusipacode"] is None
    assert registry.merge({"isin": HOLCIM, "eusipacode": 2300}), "a category fills"
    row = registry.get(HOLCIM)
    assert row is not None and row["eusipacode"] == 2300
    assert Eusipa(row["eusipacode"]).name == "Constant Leverage Certificate"
    assert not registry.merge({"isin": HOLCIM, "eusipacode": 2300}), "the same category moves nothing"
    assert not registry.merge({"isin": HOLCIM}), "a statement of none moves nothing"
    assert registry.merge({"isin": HOLCIM, "eusipacode": 2301}), "a category neither map lists is a category"
    assert registry.merge({"isin": HOLCIM, "eusipacode": "1260", "updunix": 1}), "text of the number, whatever the time"
    assert not registry.merge({"isin": HOLCIM, "eusipacode": 3100}), "a number of no category's shape is dropped"
    assert registry.merge({"isin": HOLCIM, "miccode": "XLON", "ticker": "HOLNL"}), "a second listing"
    assert [(row["miccode"], row["eusipacode"]) for row in registry.listings(HOLCIM)] == [
        ("XLON", 1260),
        ("XSWX", 1260),
    ], "an instrument fact the new listing carries too"
    # A golden file states it under either map's name, as a number or its text.
    for name in ("EUSIPA_Code", "eusipa", "EUSIPACategory", "SSPA", "sspa_code", "SSPACategory"):
        for cells in ([2300, 3100, None], ["2300", "3100", ""]):
            golden = IsinRegistry.from_arrow_reader(pa.table({"ISIN": [HOLCIM, APPLE, "CH0012005267"], name: cells}))
            assert [golden.get(key)["eusipacode"] for key in (HOLCIM, APPLE, "CH0012005267")] == [2300, None, None], name


def test_a_walk_learns_the_product_category_a_bridge_key_states_and_lifts_it_nowhere(codec: FixCodec) -> None:
    mini = "CH0012005267"
    registry = IsinRegistry()
    shared = FixCodec(codec.registry, isin_registry=registry)
    line = b"8=FIX.4.4|35=D|11=W|22=4|48=" + mini.encode() + b"|EUSIPACode=2300|10=0|"
    [walked] = list(shared.lifecycle([shared.parse_fix_line(line)]))
    row = registry.get(mini)
    assert row is not None and row["eusipacode"] == 2300, "the lifecycle learns the category"
    assert "2300" not in str(walked.securityids) and "2300" not in str(walked.identifiers), "lifted nowhere"
    assert b"|eusipacode=2300|" in walked.into_bytes(ord("|")).lower(), "the entry stays on the wire"
    # Either map's name, a namespace before it, replaces it.
    other = b"8=FIX.4.4|35=D|11=X|22=4|48=" + mini.encode() + b"|OMS_SSPACategory=1260|10=0|"
    list(shared.lifecycle([shared.parse_fix_line(other)]))
    assert registry.get(mini)["eusipacode"] == 1260
    # Two categories state none, and `learn` alone reads none: the reading is the lifecycle's.
    both = b"8=FIX.4.4|35=D|11=Y|22=4|48=" + mini.encode() + b"|EUSIPACode=2300|SSPACategory=2205|10=0|"
    list(shared.lifecycle([shared.parse_fix_line(both)]))
    assert registry.get(mini)["eusipacode"] == 1260
    fresh = IsinRegistry()
    assert fresh.learn(codec.parse_fix_line(b"8=FIX.4.4|35=D|11=Z|22=4|48=" + mini.encode() + b"|55=MINI|207=XSWX|EUSIPACode=2300|10=0|"))
    assert fresh.get(mini)["eusipacode"] is None


def test_a_store_written_before_the_product_category_loads_it_null(tmp_path: pathlib.Path) -> None:
    registry = IsinRegistry()
    registry.merge({"isin": HOLCIM, "ric": "HOLN.S"})
    older = registry.into_arrow_reader().read_all().drop_columns(["eusipacode"])
    # Forty-six columns since `firstunix` and `origccy` joined the row.
    assert len(older.schema.names) == 45, "the forty-six columns but the product category"
    target = tmp_path / "instruments.arrow"
    LocalFile(target).overwrite_arrow_reader(pa.RecordBatchReader.from_batches(older.schema, older.to_batches()))
    loaded = IsinRegistry.from_url(target)
    row = loaded.get(HOLCIM)
    assert row is not None and row["ric"] == "HOLN.S" and row["eusipacode"] is None
    assert not loaded.is_dirty


def test_the_underlying_crosses_the_pipelines_iceberg_table(tmp_path: pathlib.Path) -> None:
    from tests import medallion
    from yggdryl.iceberg import IcebergCatalog

    silver = IcebergCatalog.open_or_create("silver", tmp_path / "silver")
    registry = medallion.instruments(silver)
    assert registry.merge({"isin": "CH0012005267", "underlyingisin": HOLCIM})
    seed = IsinRegistry.seeded()
    assert len(registry) == len(seed) and registry.commit().written_rows == registry.rows == seed.rows, "the seed's listing rows, one moved"
    stored = silver.table("record_keeping.instruments")
    field = stored.field()
    assert field.index_of("underlyingisin") == field.index_of("forexcode") + 1
    reloaded = IsinRegistry.from_url(stored.url)
    assert reloaded.get("CH0012005267")["underlyingisin"] == HOLCIM


def test_a_ticker_leads_back_to_its_isin_on_the_same_market() -> None:
    # Through the inverse index, gated by the market: the one row listing the
    # ticker whose market is the one asked, or whose market or the one asked
    # is unstated; two rows answering is ambiguous, and answers none.
    novartis = "CH0012005267"
    registry = IsinRegistry()
    assert registry.merge({"isin": HOLCIM, "ric": "HOLN.S", "ticker": "HOLN", "miccode": "XSWX"})

    def isin(ticker: str, market: str | None = None) -> str | None:
        row = registry.get_by_ticker(ticker, market)
        return None if row is None else str(row["isin"])

    assert isin("HOLN") == isin("HOLN", "XSWX") == HOLCIM
    assert isin("HOLN", "XXXX") == HOLCIM, "XXXX states no market"
    assert isin("HOLN", "XLON") is None and isin("ABBN") is None
    with pytest.raises(ValueError):
        registry.get_by_ticker("HOLN", "TOOLONG")
    # Two rows listing one ticker on two markets: a market resolves to its
    # listing, none resolves to neither.
    assert registry.merge({"isin": novartis, "ticker": "HOLN", "miccode": "XLON"})
    assert (isin("HOLN", "XLON"), isin("HOLN", "XSWX")) == (novartis, HOLCIM)
    assert isin("HOLN") is None, "ambiguous"
    assert [row["miccode"] for row in registry.remove(novartis)] == ["XLON"]
    assert isin("HOLN") == HOLCIM
    registry.clear()
    assert isin("HOLN") is None


def test_a_new_isin_past_the_bound_is_refused_by_merge() -> None:
    registry = IsinRegistry(max_instruments=1)
    assert registry.max_instruments == 1
    assert registry.merge({"isin": HOLCIM})
    with pytest.raises(ValueError, match="1"):
        registry.merge({"isin": APPLE})
    assert repr(registry) == "IsinRegistry(len=1, max_instruments=1, dirty=True)"


def test_a_full_registry_warns_once_through_logging_and_learns_on(codec: FixCodec, caplog: pytest.LogCaptureFixture) -> None:
    # The warning crosses into Python's `logging` from the core, with no
    # GIL held while the registry's lock is: the call returns.
    registry = IsinRegistry(max_instruments=1)
    assert registry.merge({"isin": APPLE})
    with caplog.at_level(logging.WARNING):
        assert not registry.learn(stated(codec))
        assert not registry.learn(stated(codec))
    assert len(registry) == 1
    assert sum("instrument registry full" in record.getMessage() for record in caplog.records) == 1


def test_a_golden_file_loads_by_any_spelling_of_its_columns() -> None:
    golden = pa.table(
        {
            "ISIN": [HOLCIM, APPLE],
            "RIC": ["HOLN.S", "AAPL.OQ"],
            "BloombergSymbol": ["HOLN SW Equity", "AAPL US Equity"],
            "MIC": ["XSWX", "XNAS"],
            "CFI": ["ESVUFR", "ESVUFR"],
            "Country": ["LI", None],
            "Currency": ["CHF", "USD"],
            "CcyPair": [None, "USD/CHF"],
            "Unrelated": [1, 2],
        }
    )
    registry = IsinRegistry.from_arrow_reader(golden)
    assert len(registry) == 2 and not registry.is_dirty, "a load leaves a registry clean"
    row = registry.get(APPLE)
    assert row is not None and (row["isin"], row["bloomberg"], row["miccode"], row["forexcode"]) == (
        APPLE,
        "AAPL US Equity",
        "XNAS",
        "USD/CHF",
    )
    holcim = registry.get(HOLCIM)
    assert holcim is not None and (holcim["countrycode"], holcim["currency"]) == ("LI", "CHF")
    assert registry.extend_from_arrow_reader(golden) == 2, "a known row folds again"
    with pytest.raises(ValueError, match="ric"):
        IsinRegistry.from_arrow_reader(pa.table({"ISIN": [HOLCIM], "RIC": ["A.B"], "riccode": ["A.C"]}))
    with pytest.raises(ValueError, match="isin"):
        IsinRegistry.from_arrow_reader(pa.table({"RIC": ["HOLN.S"]}))


def test_a_registry_round_trips_through_a_holder(tmp_path: pathlib.Path) -> None:
    registry = IsinRegistry()
    registry.merge({"isin": HOLCIM, "ric": "HOLN.S", "cficode": "ESVUFR", "updunix": 7})
    registry.merge({"isin": APPLE, "ric": "AAPL.OQ"})
    snapshot = registry.into_arrow_reader()
    registry.clear()
    table = snapshot.read_all()
    assert table.num_rows == 2, "the stream is a snapshot a later write does not move"
    assert table.column("isin").to_pylist() == [HOLCIM, APPLE], "in ISIN order"
    assert table.schema.names[:14] == [
        "isin",
        "updunix",
        "firstunix",
        "lastunix",
        "cficode",
        "countrycode",
        "forexcode",
        "underlyingisin",
        "eusipacode",
        "miccode",
        "ticker",
        "fisn",
        "currency",
        "origccy",
    ]
    # Forty-six: the earliest instant an event stated the ISIN joined after
    # `updunix`, and the origin currency after the trading currency.
    assert len(table.schema.names) == 46
    target = tmp_path / "instruments.arrow"
    LocalFile(target).overwrite_arrow_reader(IsinRegistry.from_arrow_reader(table).into_arrow_reader())
    loaded = IsinRegistry.from_url(target)
    assert len(loaded) == 2 and loaded.get(HOLCIM) == IsinRegistry.from_arrow_reader(table).get(HOLCIM)
    assert loaded.extend_from_handle(LocalFile(target)) == 2
    assert loaded.extend_from_handle(target) == 2
    assert len(IsinRegistry.from_url(tmp_path / "missing.arrow")) == 0, "a missing store is empty"
    with pytest.raises(TypeError, match="property"):
        IsinRegistry.from_url(target, media_type=1)


def test_a_registry_commits_to_its_store_only_where_it_moved(tmp_path: pathlib.Path) -> None:
    # A trailing separator is what makes a location that is not there yet
    # a folder rather than a leaf.
    store = str(tmp_path / "isin") + os.sep
    registry = IsinRegistry.from_url(store, max_instruments=8)
    assert len(registry) == 0 and not registry.is_dirty and registry.max_instruments == 8
    assert not (tmp_path / "isin").exists(), "nothing is laid out before a commit"
    assert registry.commit() == IOResult(0, 0), "a clean registry writes nothing"
    assert registry.merge({"isin": HOLCIM, "ric": "HOLN.S"})
    assert registry.is_dirty
    result = registry.commit()
    assert isinstance(result, IOResult) and result.written_rows == 1
    assert not registry.is_dirty
    assert (tmp_path / "isin" / "part-0.arrows").is_file()
    assert registry.commit().written_rows == 0, "a second commit writes nothing"
    reloaded = IsinRegistry.from_url(store)
    assert reloaded.get(HOLCIM) == registry.get(HOLCIM) and not reloaded.is_dirty
    # Emptied: cleared on the next commit.
    registry.clear()
    registry.commit()
    assert len(IsinRegistry.from_url(store)) == 0
    # An unbound registry has nowhere to commit.
    with pytest.raises(ValueError, match="holder"):
        IsinRegistry().commit()


def test_a_codec_shares_the_callers_registry_with_every_lifecycle_and_every_parse(codec: FixCodec) -> None:
    registry = IsinRegistry()
    shared = FixCodec(codec.registry, isin_registry=registry)
    assert shared.isin_registry == registry and codec.isin_registry is None
    assert shared.with_dedup_window_ms(None).isin_registry == registry, "a derived codec keeps it"
    list(shared.lifecycle([stated(shared)]))
    assert len(registry) == 1 and registry.get(HOLCIM) is not None
    list(codec.lifecycle([stated(codec)]))
    assert len(registry) == 1, "a codec without one learns into its own"
    # A parse through the sharing codec fills derived identifiers from the
    # table, learns nothing, and leaves the identity the bare parse gives -
    # the two read under one pinned clock, so an undated line dates alike.
    pinned = datetime.datetime(2026, 1, 2, 10, 15, 30, tzinfo=datetime.timezone.utc)
    line = b"8=FIX.4.4|35=D|11=P|55=HOLN|207=XSWX|10=0|"
    filled = FixCodec(codec.registry, isin_registry=registry, default_sending_time=pinned).parse_fix_line(line)
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
    assert not registry.learn(filled), "nothing a parse derived is learned back"


def test_the_process_registry_is_the_sealed_store_and_the_codec_the_environment_names_shares_it() -> None:
    # The suite's conftest points `YGGDRYL_ISIN_REGISTRY_URI` at a session
    # folder before anything resolves the default.
    default = IsinRegistry.from_env()
    assert default == IsinRegistry.from_env(), "resolved once"
    assert not default.is_dirty
    apple = default.get(APPLE)
    assert apple is not None and apple["ticker"] == "AAPL", "the store is laid over the seed"
    codec = FixCodec.from_env()
    assert codec.isin_registry == default
    own = IsinRegistry()
    assert FixCodec.from_env(isin_registry=own).isin_registry == own, "a stated pin stands"
    assert FixCodec(FixRegistry.from_env()).isin_registry is None, "a codec built by hand attaches none"
    with pytest.raises(ValueError, match="already resolved"):
        IsinRegistry.install_env(IsinRegistry())
    assert "YGGDRYL_ISIN_REGISTRY_URI" in os.environ, "the suite seals the default"


def test_the_seed_holds_the_common_instruments_clean_and_bound_to_nothing() -> None:
    seeded = IsinRegistry.seeded()
    assert (len(seeded), seeded.rows) == (208, 209) and not seeded.is_dirty, "HSBC lists on XHKG and XLON"
    assert [row["miccode"] for row in seeded.listings("GB0005405286")] == ["XHKG", "XLON"]
    assert len(IsinRegistry()) == 0, "a registry built by hand holds none of it"
    apple = seeded.get(APPLE)
    assert apple is not None
    assert (apple["ticker"], apple["miccode"], apple["currency"], apple["fisn"]) == (
        "AAPL",
        "XNAS",
        "USD",
        "APPLE INC/SH SH",
    )
    assert apple["cusip"] == "037833100", "the CUSIP its ISIN embeds"
    assert seeded.get_by_ticker("AAPL", "XNAS") == apple
    assert IsinRegistry.seeded() != seeded, "each call is a registry of its own"
    with pytest.raises(ValueError, match="unbound registry"):
        seeded.commit()


def test_a_store_bound_seeded_is_laid_over_the_seed(tmp_path: pathlib.Path) -> None:
    target = tmp_path / "instruments.arrows"
    store = IsinRegistry.from_url(target)
    assert store.merge({"isin": APPLE, "miccode": "XNAS", "ticker": "AAPL", "currency": "CHF"})
    bae = "GB0002634946"
    assert store.merge({"isin": bae, "miccode": "XLON"})
    store.commit()
    seed = IsinRegistry.seeded()
    assert seed.get(bae) is None
    registry = IsinRegistry.seeded_from_url(target, max_instruments=1024)
    assert len(registry) == len(seed) + 1 and not registry.is_dirty and registry.max_instruments == 1024
    apple = registry.get(APPLE)
    assert apple is not None and apple["currency"] == "CHF", "the store's value wins"
    seeded = seed.get(APPLE)
    assert seeded is not None and apple["fisn"] == seeded["fisn"] == "APPLE INC/SH SH", "the seed's fact stands"
    assert registry.get(bae) == store.get(bae), "a row only the store holds"
    assert registry.commit() == IOResult(0, 0), "clean after the load"
    assert len(IsinRegistry.from_url(target)) == 2, "unseeded: the store's rows alone"
    assert registry.merge({"isin": HOLCIM, "ric": "HOLN.S"})
    assert registry.commit().written_rows == registry.rows, "the seed's listing rows with the store's"
    assert len(IsinRegistry.from_url(target)) == len(registry)
    assert len(IsinRegistry.seeded_from_url(LocalFile(target))) == len(registry), "a handle names the store too"
    with pytest.raises(TypeError, match="properties"):
        IsinRegistry.seeded_from_url(LocalFile(target), media_type="x")
    first = IsinRegistry.seeded_from_url(str(tmp_path / "isin") + os.sep)
    assert len(first) == len(seed) and not first.is_dirty, "a first run: the seed bound to the store"
    assert first.commit() == IOResult(0, 0) and not (tmp_path / "isin").exists()


def test_the_row_declares_the_country_partition_and_the_isin_order() -> None:
    field = IsinRegistry.field()
    assert field.metadata["PARTITION:by"] == '["truncate(isin, 2)"]'
    assert field.metadata["SORT:by"] == '["isin","miccode"]', "a listing row per ISIN and market"


def test_the_short_name_is_the_column_after_the_ticker_and_merges_by_the_update_rule() -> None:
    field = IsinRegistry.field()
    assert field.index_of("fisn") == field.index_of("ticker") + 1
    registry = IsinRegistry()
    assert registry.merge({"isin": HOLCIM, "fisn": "HOLCIM LTD/SH"})
    assert registry.get(HOLCIM)["fisn"] == "HOLCIM LTD/SH"
    assert registry.merge({"isin": HOLCIM, "fisn": "HOLCIM AG/SH"}), "a stated value replaces one that differs"
    assert registry.get(HOLCIM)["fisn"] == "HOLCIM AG/SH"


def test_a_row_carries_the_defaults_its_facts_imply_where_it_states_none() -> None:
    registry = IsinRegistry()
    bae = "GB0002634946"
    assert registry.merge({"isin": bae, "miccode": "XLON"})
    row = registry.get(bae)
    assert row is not None
    assert row["sedol"] == "0263494", "the SEDOL a GB '00' ISIN embeds"
    assert row["currency"] == "GBP", "the currency of the listing market's country"
    # A stated value is never replaced by a default.
    assert registry.merge({"isin": bae, "currency": "USD"})
    assert registry.get(bae)["currency"] == "USD"


def test_a_registry_is_equal_only_to_itself_and_never_hashed_or_pickled() -> None:
    registry = IsinRegistry()
    assert registry == registry and registry != IsinRegistry()
    with pytest.raises(TypeError):
        hash(registry)
    with pytest.raises(TypeError):
        pickle.dumps(registry)
    with pytest.raises(TypeError):
        copy.deepcopy(registry)


def test_the_first_learn_sets_firstunix_and_only_an_earlier_event_moves_it(codec: FixCodec) -> None:
    utc = datetime.timezone.utc
    early, first, later = (datetime.datetime(2026, 1, 2, hour, tzinfo=utc) for hour in (8, 9, 11))

    def met(instant: datetime.datetime) -> object:
        line = b"8=FIX.4.4|35=D|11=A|22=4|48=" + HOLCIM.encode() + b"|10=0|"
        return FixCodec(codec.registry, default_sending_time=instant).parse_fix_line(line)

    registry = IsinRegistry()
    assert registry.field().index_of("firstunix") == registry.field().index_of("updunix") + 1
    assert registry.field().index_of("lastunix") == registry.field().index_of("updunix") + 2
    assert registry.learn(met(first))
    row = registry.get(HOLCIM)
    assert row is not None and (row["updunix"], row["firstunix"], row["lastunix"]) == (first, first, first)
    assert registry.learn(met(later)), "a later event moves lastunix"
    row = registry.get(HOLCIM)
    assert row is not None and (row["firstunix"], row["lastunix"]) == (first, later), "not firstunix"
    assert registry.learn(met(early)), "replayed out of order"
    row = registry.get(HOLCIM)
    assert row is not None
    assert (row["updunix"], row["firstunix"], row["lastunix"]) == (first, early, later), "updunix moves with a fact alone"
    clean = IsinRegistry.from_arrow_reader(registry.into_arrow_reader())
    assert not clean.learn(met(first)) and not clean.is_dirty, "between the two: nothing moves"
    # A golden file states it under each of its spellings, the earlier kept.
    for name in ("firstunix", "FirstSeen", "first_seen_unix"):
        golden = pa.table(
            {
                "ISIN": [HOLCIM, HOLCIM],
                name: pa.array([later, first], pa.timestamp("ns", tz="UTC")),
            }
        )
        loaded = IsinRegistry.from_arrow_reader(golden).get(HOLCIM)
        assert loaded is not None and loaded["firstunix"] == first, name


def _registry(*rows: dict[str, object]) -> IsinRegistry:
    registry = IsinRegistry()
    for row in rows:
        assert registry.merge(row)
    return registry


def _order(*codes: tuple[str, str], **facts: object) -> graph.OrderEvent:
    return graph.OrderEvent(1, securityids=[Identifier(kind, value) for kind, value in codes], **facts)


def test_the_cascade_takes_the_isin_then_a_code_then_the_ticker_on_its_market() -> None:
    registry = _registry(
        {"isin": APPLE, "miccode": "XNAS", "ticker": "AAPL"},
        {"isin": DIAGEO, "miccode": "XLON", "ticker": "DGE"},
        {"isin": SAP, "miccode": "XETR"},
    )
    # The ISIN wins over a CUSIP naming Apple.
    by_isin = registry.resolve(_order(("isin", SAP), ("cusip", "037833100")))
    assert isinstance(by_isin, Resolution) and by_isin and by_isin.matched
    assert (by_isin.tier, by_isin.kind, by_isin.derived, by_isin.listing) == ("isin", None, False, True)
    assert by_isin.entry is not None and by_isin.entry["isin"] == SAP
    assert (by_isin.unmatched, by_isin.isins, by_isin.stated, by_isin.similarity) == (None, None, None, None)
    # A CUSIP wins over a ticker naming Diageo; Apple is not listed on XLON.
    by_code = registry.resolve(_order(("cusip", "037833100"), ticker="DGE", miccode="XLON"))
    assert (by_code.tier, by_code.kind, by_code.derived, by_code.listing) == ("code", "cusip", True, False)
    assert by_code.entry is not None and by_code.entry["isin"] == APPLE
    # The ticker on its market last.
    by_ticker = registry.resolve(_order(ticker="DGE", miccode="XLON"))
    assert (by_ticker.tier, by_ticker.derived, by_ticker.listing) == ("symbology", True, True)
    assert by_ticker.entry is not None and by_ticker.entry["isin"] == DIAGEO
    # A FIX message and a `MarketData` resolve as any leaf does.
    assert registry.resolve(graph.MarketData(_order(("isin", SAP)))) == by_isin
    with pytest.raises(TypeError, match="market leaf"):
        registry.resolve("US0378331005")
    # The public lookups.
    apple = registry.get_by_code("cusip", "037833100")
    assert apple is not None and apple["isin"] == APPLE
    diageo = registry.get_by_code("SEDOL", "0237400", "XLON")
    assert diageo is not None and diageo["isin"] == DIAGEO, "the SEDOL its ISIN embeds, any spelling of its type"
    assert registry.get_by_code("cusip", "not a cusip") is None
    assert registry.get_by_code("isoccy", "USD") is None, "no lookup code"
    assert "isoccy" not in IsinRegistry.LOOKUP_CODES and IsinRegistry.LOOKUP_CODES[:3] == ("cusip", "sedol", "wkn")
    with pytest.raises(ValueError):
        registry.get_by_code("cusip", "037833100", "TOOLONG")
    # Nothing stated: no key; a ticker nobody lists: no candidate.
    nothing = registry.resolve(_order())
    assert not nothing and nothing.unmatched == "NoKey"
    assert (nothing.entry, nothing.tier, nothing.derived, nothing.listing) == (None, None, None, None)
    assert registry.resolve(_order(ticker="ZZZZ")).unmatched == "NoCandidate"


def test_an_unknown_stated_isin_ends_the_cascade_and_fills_nothing(codec: FixCodec) -> None:
    registry = _registry({"isin": APPLE, "miccode": "XNAS"})
    unknown = registry.resolve(_order(("isin", NOVARTIS), ("cusip", "037833100")))
    assert (unknown.matched, unknown.unmatched, unknown.stated) == (False, "UnknownIsin", NOVARTIS)
    assert unknown.isin is None and unknown.held is None
    message = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|22=4|48=" + NOVARTIS.encode() + b"|10=0|")
    assert registry.resolve(message) == unknown
    assert not registry.fill(message) and message.isincode == NOVARTIS and message.cficode is None


def test_a_code_two_instruments_hold_is_ambiguous_and_stops_the_cascade() -> None:
    registry = _registry(
        {"isin": APPLE, "common": "C-1"},
        {"isin": SAP, "common": "C-1"},
        {"isin": DIAGEO, "miccode": "XLON", "ticker": "DGE"},
    )
    ambiguous = registry.resolve(_order(("common", "C-1"), ticker="DGE", miccode="XLON"))
    assert (ambiguous.matched, ambiguous.unmatched, ambiguous.tier, ambiguous.kind) == (False, "Ambiguous", "code", "common")
    assert ambiguous.isins == [SAP, APPLE], "in ISIN order"
    assert registry.get_by_code("common", "C-1") is None
    # One instrument on two markets is one answer, never an ambiguity.
    hsbc = IsinRegistry.seeded().resolve(_order(("sedol", "0540528")))
    assert (hsbc.tier, hsbc.kind, hsbc.derived) == ("code", "sedol", True)
    assert hsbc.entry is not None and hsbc.entry["isin"] == HSBC


def test_the_economic_tier_matches_a_similar_short_name_in_the_same_currency() -> None:
    def named(name: str, **facts: object) -> graph.OrderEvent:
        return _order(("fisn", name), **{"currency": "USD", **facts})

    def registry_of(name: str, cfi: str) -> IsinRegistry:
        return _registry({"isin": APPLE, "miccode": "XNAS", "fisn": name, "cficode": cfi})

    similar = registry_of("APPLE INC./SH", "ESVUFR").resolve(named("APPLE INC/SH"))
    assert (similar.tier, similar.derived, similar.listing) == ("economic", True, True)
    assert similar.similarity is not None and math.isclose(similar.similarity, 12 / 13)
    assert similar.entry is not None and similar.entry["isin"] == APPLE

    plain = registry_of("APPLE INC/SH", "ESVUFR")
    below = plain.resolve(named("APPLE INC/SH USD"))
    assert (below.unmatched, below.best, below.isin) == ("BelowThreshold", 0.75, APPLE)

    conflict = registry_of("APPLE INC/SH", "DBFTFR").resolve(named("APPLE INC/SH", cficode="ESVUFR"))
    assert (conflict.unmatched, conflict.stated, conflict.held, conflict.isin) == ("CfiConflict", "E", "D", APPLE)
    assert plain.resolve(named("APPLE INC/SH", cficode="XXXXXX")).tier == "economic", "unclassified conflicts with nothing"
    assert plain.resolve(named("APPLE INC/SH", currency="EUR")).unmatched == "NoCandidate", "another currency"

    issued = _registry({"isin": APPLE, "miccode": "XNAS", "fisn": "APPLE INC/SH", "origccy": "USD"})
    origin = issued.resolve(named("APPLE INC/SH", origccy="EUR"))
    assert (origin.unmatched, origin.stated, origin.held, origin.isin) == ("CurrencyConflict", "EUR", "USD", APPLE)

    twins = registry_of("APPLE INC/SH", "ESVUFR")
    assert twins.merge({"isin": MICROSOFT, "miccode": "XNYS", "fisn": "APPLE INC/SH"})
    tied = twins.resolve(named("APPLE INC/SH"))
    assert (tied.unmatched, tied.tier, tied.similarity, tied.isins) == ("Ambiguous", "economic", 1.0, [APPLE, MICROSOFT])

    assert plain.resolve(_order(ticker="ZZZZ", currency="USD")).unmatched == "NoCandidate", "no short name"
    assert plain.resolve(named("APPLE INC/SH", currency="XXX")).unmatched == "NoCandidate", "XXX states no currency"


def test_a_fill_takes_the_economic_match_only_where_it_is_enabled(codec: FixCodec) -> None:
    registry = _registry({"isin": APPLE, "miccode": "XNAS", "fisn": "APPLE INC./SH"})
    assert not registry.is_economic_match
    assert registry.economic_threshold == IsinRegistry.DEFAULT_ECONOMIC_THRESHOLD == 0.85

    def named() -> FixMsg:
        return codec.parse_fix_line(b"8=FIX.4.4|35=D|11=A|2737=APPLE INC/SH|15=USD|10=0|")

    off = named()
    assert registry.resolve(off).tier == "economic", "resolve weighs it always"
    assert not registry.fill(off) and off.isincode is None, "a judgement no fill takes unasked"
    registry.set_economic_match(True)
    assert registry.is_economic_match
    on = named()
    assert registry.fill(on) and on.isincode == APPLE and on.securityids.is_derived("isin")
    registry.set_economic_threshold(0.95)
    assert not registry.fill(named()), "a stricter threshold refuses what it took"
    for refused in (0.0, 1.5, math.nan, -0.5):
        spelled = "NaN" if math.isnan(refused) else f"{refused:g}"
        with pytest.raises(ValueError, match=re.escape(spelled)):
            registry.set_economic_threshold(refused)
    assert registry.economic_threshold == 0.95, "a refusal moves nothing"
    registry.set_economic_threshold(1.0)
    registry.clear()
    assert (registry.economic_threshold, registry.is_economic_match) == (1.0, True), "the registry's, never the store's"


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

    # The registry's stated origin currency fills an element stating none,
    # and never one that does; nothing derives it into a column.
    registry = _registry({"isin": APPLE, "miccode": "XNAS", "origccy": "USD"}, {"isin": SAP, "miccode": "XETR"})
    assert IsinRegistry.field().index_of("origccy") == IsinRegistry.field().index_of("currency") + 1
    sap = registry.get(SAP)
    assert sap is not None and sap["currency"] == "EUR" and sap["origccy"] is None
    line = b"8=FIX.4.4|35=D|11=A|22=4|48=" + APPLE.encode() + b"|15=EUR|10=0|"
    unstated = codec.parse_fix_line(line)
    assert unstated.origccy is None and unstated.origin_currency.as_py() == "EUR"
    assert registry.fill(unstated)
    assert unstated.origccy is not None and unstated.origccy.as_py() == "USD"
    assert unstated.origin_currency.as_py() == "USD" and unstated.currency.as_py() == "EUR"
    own = codec.parse_fix_line(line)
    own.set("origccy", "CHF")
    registry.fill(own)
    assert own.origccy is not None and own.origccy.as_py() == "CHF", "a statement stands"


def test_a_resolution_is_equal_by_value_and_never_hashed() -> None:
    registry = _registry({"isin": APPLE, "miccode": "XNAS"})
    first, second = (registry.resolve(_order(("isin", APPLE))) for _ in range(2))
    assert first == second and first != registry.resolve(_order())
    assert repr(first) == "Resolution(matched=True, isin='US0378331005', tier='isin', derived=False, listing=True)"
    assert repr(registry.resolve(_order(("isin", NOVARTIS)))) == (
        "Resolution(matched=False, unmatched='UnknownIsin', stated='CH0012005267')"
    )
    with pytest.raises(TypeError):
        hash(first)
    with pytest.raises(TypeError):
        Resolution()  # type: ignore[call-arg]
