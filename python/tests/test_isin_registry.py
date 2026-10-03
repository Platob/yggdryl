"""`python/yggdryl/isin_registry.py` and `python/src/isin_registry.rs`: the
instrument registry, one row per ISIN of its equivalents, learned from and
filled into FIX messages and read from and written to any holder, redirected
to the core."""

from __future__ import annotations

import copy
import pathlib
import pickle

import pyarrow as pa
import pytest

from yggdryl import IsinRegistry
from yggdryl.holder import LocalFile
from yggdryl.fix import FixCodec, FixRegistry

SEED = pathlib.Path(__file__).resolve().parents[2] / "config" / "fix"
HOLCIM = "CH0012214059"
APPLE = "US0378331005"


@pytest.fixture(scope="module")
def codec() -> FixCodec:
    return FixCodec(FixRegistry.from_handle(SEED))


def stated(codec: FixCodec) -> object:
    """A message stating Holcim's ISIN, its RIC, its CFI code and its ticker."""
    return codec.parse_fix_line(
        b"8=FIX.4.4|35=D|11=A|22=4|48=" + HOLCIM.encode() + b"|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|10=0|"
    )


def test_a_registry_learns_a_message_and_fills_a_later_one_named_by_its_ric(codec: FixCodec) -> None:
    registry = IsinRegistry()
    assert not registry and len(registry) == 0
    assert registry.learn(stated(codec))
    row = registry.get(HOLCIM)
    assert row is not None
    assert {key: value for key, value in row.items() if value is not None and key != "updunix"} == {
        "cficode": "ESVUFR",
        "isin": HOLCIM,
        "ric": "HOLN.S",
        "ticker": "HOLN",
    }
    assert row["valor"] is None, "a code the message only derived is never learned"
    assert registry.get_by_ric("HOLN.S") == row and registry.get_by_ric("XXXX.S") is None
    later = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=B|22=5|48=HOLN.S|10=0|")
    assert later.isincode is None
    assert registry.fill(later)
    assert later.isincode == HOLCIM and later.securityids.is_derived("isin")
    assert later.cficode is not None and later.cficode.as_py() == "ESVUFR" and later.ticker == "HOLN"
    assert not registry.fill(later), "nothing left to fill"
    assert b"48=HOLN.S" in later.into_bytes(ord("|")) and b"461=" not in later.into_bytes(ord("|")), "never the wire"


def test_enrich_learns_then_fills(codec: FixCodec) -> None:
    registry = IsinRegistry()
    assert registry.enrich(stated(codec))
    assert len(registry) == 1


def test_a_row_merges_by_the_update_rule(codec: FixCodec) -> None:
    registry = IsinRegistry()
    assert registry.merge({"isin": HOLCIM, "ric": "HOLN.S", "updunix": 10})
    assert not registry.merge({"isin": HOLCIM, "ric": "HOLN.S"}), "a row stating nothing new moves nothing"
    assert not registry.merge({"isin": HOLCIM, "ric": "HOLN.VX", "updunix": 5}), "an older statement only fills"
    assert registry.merge({"isin": HOLCIM, "ric": "HOLN.VX", "updunix": 20}), "a newer one replaces"
    assert registry.get_by_ric("HOLN.VX") is not None and registry.get_by_ric("HOLN.S") is None
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
    assert repr(registry) == "IsinRegistry(len=1, max_instruments=1)"


def test_a_golden_file_loads_by_any_spelling_of_its_columns() -> None:
    golden = pa.table(
        {
            "ISIN": [HOLCIM, APPLE],
            "RIC": ["HOLN.S", "AAPL.OQ"],
            "BloombergSymbol": ["HOLN SW Equity", "AAPL US Equity"],
            "MIC": ["XSWX", "XNAS"],
            "CFI": ["ESVUFR", "ESVUFR"],
            "Unrelated": [1, 2],
        }
    )
    registry = IsinRegistry.from_arrow_reader(golden)
    assert len(registry) == 2
    row = registry.get_by_ric("AAPL.OQ")
    assert row is not None and (row["isin"], row["bloomberg"], row["miccode"]) == (APPLE, "AAPL US Equity", "XNAS")
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
    target = tmp_path / "instruments.arrow"
    LocalFile(target).overwrite_arrow_reader(IsinRegistry.from_arrow_reader(table).into_arrow_reader())
    loaded = IsinRegistry.from_handle(target)
    assert len(loaded) == 2 and loaded.get(HOLCIM) == IsinRegistry.from_arrow_reader(table).get(HOLCIM)
    assert loaded.extend_from_handle(LocalFile(target)) == 2
    assert len(IsinRegistry.from_handle(tmp_path / "missing.arrow")) == 0, "a missing store is empty"


def test_a_codec_shares_the_callers_registry_with_every_lifecycle(codec: FixCodec) -> None:
    registry = IsinRegistry()
    shared = FixCodec(codec.registry, isin_registry=registry)
    assert shared.isin_registry == registry and codec.isin_registry is None
    assert shared.with_dedup_window_ms(None).isin_registry == registry, "a derived codec keeps it"
    list(shared.lifecycle([stated(shared)]))
    assert len(registry) == 1 and registry.get(HOLCIM) is not None
    list(codec.lifecycle([stated(codec)]))
    assert len(registry) == 1, "a codec without one learns into its own"


def test_a_registry_is_equal_only_to_itself_and_never_hashed_or_pickled() -> None:
    registry = IsinRegistry()
    assert registry == registry and registry != IsinRegistry()
    with pytest.raises(TypeError):
        hash(registry)
    with pytest.raises(TypeError):
        pickle.dumps(registry)
    with pytest.raises(TypeError):
        copy.deepcopy(registry)
