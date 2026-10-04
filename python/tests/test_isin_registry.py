"""`python/yggdryl/isin_registry.py` and `python/src/isin_registry.rs`: the
instrument registry, one row per ISIN of every fact it is known by, learned
from and filled into FIX messages, bound to the store it is loaded from and
committed back to, redirected to the core."""

from __future__ import annotations

import copy
import datetime
import logging
import os
import pathlib
import pickle

import pyarrow as pa
import pytest

from yggdryl import IOResult, IsinRegistry
from yggdryl.holder import LocalFile
from yggdryl.fix import FixCodec, FixRegistry

SEED = pathlib.Path(__file__).resolve().parents[2] / "config" / "fix"
HOLCIM = "CH0012214059"
APPLE = "US0378331005"


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
    assert {key: value for key, value in row.items() if value is not None and key != "updunix"} == {
        "cficode": "ESVUFR",
        "currency": "CHF",
        "isin": HOLCIM,
        "miccode": "XSWX",
        "ric": "HOLN.S",
        "ticker": "HOLN",
    }
    assert row["valor"] is None, "a code the message only derived is never learned"
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
    # A RIC is a listing code, never a key.
    by_ric = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=C|22=5|48=HOLN.S|10=0|")
    assert not registry.fill(by_ric) and by_ric.isincode is None


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
    with pytest.raises(ValueError, match="isin"):
        registry.merge({"ric": "HOLN.S"})
    removed = registry.remove(HOLCIM)
    assert removed is not None and removed["isin"] == HOLCIM and registry.remove(HOLCIM) is None
    registry.merge({"isin": HOLCIM})
    registry.clear()
    assert len(registry) == 0


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
    assert registry.remove(novartis) is not None
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
    assert table.schema.names[:8] == ["isin", "updunix", "cficode", "countrycode", "forexcode", "miccode", "ticker", "currency"]
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
    assert registry.commit().written_rows == 1
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
    assert (filled.curruuid, filled.currhashcode, filled.into_bytes(ord("|"))) == (
        bare.curruuid,
        bare.currhashcode,
        bare.into_bytes(ord("|")),
    )
    assert filled.cficode is None, "a parse fills identifiers only"
    assert not registry.learn(filled), "nothing a parse derived is learned back"


def test_the_process_registry_is_the_sealed_store_and_the_codec_the_environment_names_shares_it() -> None:
    # The suite's conftest points `YGGDRYL_ISIN_REGISTRY_URI` at a session
    # folder before anything resolves the default.
    assert "YGGDRYL_ISIN_REGISTRY_URI" in os.environ, "the suite seals the default"
    default = IsinRegistry.from_env()
    assert default == IsinRegistry.from_env(), "resolved once"
    assert not default.is_dirty
    codec = FixCodec.from_env()
    assert codec.isin_registry == default
    own = IsinRegistry()
    assert FixCodec.from_env(isin_registry=own).isin_registry == own, "a stated pin stands"
    assert FixCodec(FixRegistry.from_env()).isin_registry is None, "a codec built by hand attaches none"
    with pytest.raises(ValueError, match="already resolved"):
        IsinRegistry.install_env(IsinRegistry())


def test_a_registry_is_equal_only_to_itself_and_never_hashed_or_pickled() -> None:
    registry = IsinRegistry()
    assert registry == registry and registry != IsinRegistry()
    with pytest.raises(TypeError):
        hash(registry)
    with pytest.raises(TypeError):
        pickle.dumps(registry)
    with pytest.raises(TypeError):
        copy.deepcopy(registry)
