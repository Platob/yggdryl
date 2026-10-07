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

from yggdryl import Eusipa, IOResult, IsinRegistry
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
    assert row["underlyingisin"] is None
    assert registry.merge({"isin": HOLCIM, "underlyingisin": APPLE}), "an instrument fact fills"
    assert not registry.merge({"isin": HOLCIM, "underlyingisin": HOLCIM}), "the row's own ISIN states nothing"
    assert not registry.merge({"isin": HOLCIM, "underlyingisin": "US0378331006"}), "a typo is dropped"
    assert registry.merge({"isin": HOLCIM, "miccode": "XLON", "ticker": "HOLNL"}), "a listing switch"
    row = registry.get(HOLCIM)
    assert row is not None and (row["underlyingisin"], row["miccode"]) == (APPLE, "XLON"), "an instrument fact no listing switch clears"
    with pytest.raises(ValueError, match="isin"):
        registry.merge({"ric": "HOLN.S"})
    removed = registry.remove(HOLCIM)
    assert removed is not None and removed["isin"] == HOLCIM and registry.remove(HOLCIM) is None
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
    assert registry.merge({"isin": HOLCIM, "miccode": "XLON", "ticker": "HOLNL"}), "a listing switch"
    row = registry.get(HOLCIM)
    assert row is not None and (row["miccode"], row["eusipacode"]) == ("XLON", 1260), "no listing switch clears it"
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
    assert len(older.schema.names) == 42, "the forty-three columns but the product category"
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
    assert registry.commit().written_rows == 1
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
    assert table.schema.names[:11] == [
        "isin",
        "updunix",
        "cficode",
        "countrycode",
        "forexcode",
        "underlyingisin",
        "eusipacode",
        "miccode",
        "ticker",
        "fisn",
        "currency",
    ]
    assert len(table.schema.names) == 43, "the ISO 18774 short name is the forty-third column"
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
    assert len(seeded) == 208 and not seeded.is_dirty
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
