"""`python/yggdryl/identifier.py` and `python/src/identifier.rs`: a value under a
key `src:type` - the type alone for the base source - and the sorted map an
element states them in, redirected to the core."""

from __future__ import annotations

import copy
import pickle

import pytest

from yggdryl import Identifier, Identifiers, Scalar


def test_an_identifier_reads_its_key_exactly_and_trims_its_value() -> None:
    held = Identifier("bic:executing_trader", " T-1 ")
    assert (held.src, held.type, held.value) == ("bic", "executingtrader", "T-1")
    assert held.key == "bic:executingtrader"
    assert str(held) == "bic:executingtrader=T-1"
    assert repr(held) == "Identifier('bic:executingtrader', 'T-1')"
    for spelled in ("clordid", "ClOrdID", "base:clordid", "BASE:CLORDID", "fix:clordid", "FIX:ClOrdID"):
        bare = Identifier(spelled, "C-2")
        assert (bare.src, bare.type, bare.key) == ("base", "clordid", "clordid"), spelled
    assert str(Identifier("isin", "US0378331005")) == "isin=US0378331005", "a base key is its type alone"


def test_an_identifier_is_a_key_and_a_value_and_states_no_parentage() -> None:
    with pytest.raises(TypeError):
        Identifier("fix", "clordid", "C-2")  # type: ignore[call-arg]
    with pytest.raises(TypeError):
        Identifier("clordid", "C-2", parent="C-1")  # type: ignore[call-arg]
    held = Identifier("clordid", "C-2")
    assert not hasattr(held, "parent") and not hasattr(held, "orig") and not hasattr(held, "is_of")


def test_a_key_names_one_source_and_one_type() -> None:
    assert Identifier("firm.x:house code", "hc-1").key == "firm.x:housecode"
    assert Identifier("orderid", "A").key == Identifier("orderid", "B").key, "a key names, never a value"
    assert Identifier("orderid", "A").key != Identifier("venue:orderid", "A").key
    assert Identifier("marketorderid", "O-1").key == "marketorderid", "a bare word is read, never inferred"
    held = Identifier("oms:clordid", "C-1")
    assert Identifier(held.key, held.value) == held, "the key reads back as itself"
    assert Identifier.from_key(held.key, held.value) == held


def test_a_key_or_a_value_that_reads_as_nothing_is_refused() -> None:
    for key in ("fix:", ":isin", "a:b:c", "café", ""):
        with pytest.raises(ValueError, match="identifier key"):
            Identifier(key, "X")
    with pytest.raises(ValueError, match="identifier value"):
        Identifier("orderid", "n/a")
    with pytest.raises(ValueError, match="identifier value"):
        Identifier("orderid", "")


def test_a_security_type_checks_its_code() -> None:
    apple = Identifier("isin", " us0378331005 ")
    assert (apple.src, apple.type, apple.value) == ("base", "isin", "US0378331005")
    # The type checks the code's shape; a check digit is a rank, not a refusal.
    assert Identifier("isin", "US0378331006").value == "US0378331006"
    with pytest.raises(ValueError):
        Identifier("isin", "US037833100")
    assert Identifier("forex", "eur/usd").value == "EUR/USD"
    assert Identifier("isinnumber", "US0378331005").type == "isin"


def test_a_name_no_key_spells_is_read_for_the_identifier_name_it_ends_with() -> None:
    def read(key: str, value: str) -> str:
        keyed = Identifier.from_key(key, value)
        assert keyed is not None, key
        return str(keyed)

    assert read("firm.x.ParentOrderID", "P-1") == "firm.x:parentorderid=P-1"
    assert read("OMS_InstrumentID", "dbi;X") == "oms:instrumentid=dbi;X"
    assert read("marketorderid", "O-1") == "market:orderid=O-1"
    assert read("OrderID", "O-1") == "orderid=O-1"
    assert read("fix:ClOrdID", "C-1") == "clordid=C-1"
    assert read("Derived_ISIN", "US0378331005") == "isin=US0378331005", "a reserved source names no namespace"
    assert read("ISINCode", "US0378331005") == "isin=US0378331005"
    assert read("security_cusip", "037833100") == "cusip=037833100"
    assert read("firm.isin", "us0378331005") == "firm:isin=US0378331005"
    for key in ("underlyingisin", "legisin", "transversalkey", "symbol", ""):
        assert Identifier.from_key(key, "US0378331005") is None, key
    assert Identifier.from_key("ClOrdID", "null") is None


def test_identifiers_order_by_their_key_as_spelled_then_by_value() -> None:
    assert Identifier("a", "1") < Identifier("b", "1")
    cusip = "037833100"
    assert Identifier("cusip", cusip) < Identifier("derived:cusip", cusip) < Identifier("isin", "US0378331005")
    isin = "US0378331005"
    assert Identifier("isin", isin) < Identifier("oms:instrumentid", "1") < Identifier("ullink:isin", isin)
    assert Identifier("a.b:c", "1") < Identifier("a:z", "1"), "'.' sorts below ':'"
    assert Identifier("orderid", "A") < Identifier("orderid", "B")


def test_identifiers_compare_hash_copy_and_pickle_as_values() -> None:
    first = Identifier("oms:clordid", "1")
    assert first == Identifier("OMS:ClOrdID", "1") and hash(first) == hash(Identifier("oms:clordid", "1"))
    assert first != Identifier("oms:clordid", "2")
    assert copy.copy(first) == first and copy.deepcopy(first) == first
    assert pickle.loads(pickle.dumps(first)) == first


def test_an_identifier_crosses_as_the_one_entry_map_of_its_key() -> None:
    assert Scalar.from_(Identifier("ullink:isin", "US0378331005")).as_py() == {"ullink:isin": "US0378331005"}
    ids = Identifiers([Identifier("ullink:isin", "US0378331005")])
    assert Scalar.from_(ids).as_py() == ids.into_dict()


def test_a_named_source_fills_the_base_key_of_its_type() -> None:
    ids = Identifiers(
        [
            Identifier("ullink:isin", "US0378331005"),
            Identifier("isin", "CH0012214059"),
            Identifier("derived:cusip", "037833100"),
            Identifier("oms:instrumentid", "dbi;X"),
        ]
    )
    assert [str(id) for id in ids] == [
        "cusip=037833100",
        "derived:cusip=037833100",
        "instrumentid=dbi;X",
        "isin=US0378331005",
        "oms:instrumentid=dbi;X",
        "ullink:isin=US0378331005",
    ]
    assert ids.get("isin") == "US0378331005", "the first statement fills; a later base key is dropped"
    assert ids.get_from("ullink:isin") == "US0378331005" and ids.get_from("oms:isin") is None
    assert ids.get_from("BASE:ISIN") == ids.get("ISIN")
    assert ids.contains_kind("cusip") and not ids.contains_kind("SEDOL")
    assert [id.key for id in ids.of_kind("isin")] == ["isin", "ullink:isin"]
    assert ids.is_derived("cusip") and not ids.is_derived("isin")
    assert not Identifiers() and len(Identifiers()) == 0


def test_a_statement_takes_back_the_derivation_of_its_type() -> None:
    ids = Identifiers([Identifier("derived:cusip", "037833100"), Identifier("ullink:cusip", "594918104")])
    assert [str(id) for id in ids] == ["cusip=594918104", "ullink:cusip=594918104"]
    assert not ids.is_derived("cusip")


def test_a_dict_reads_exactly_and_closes_by_the_base_rule() -> None:
    ids = Identifiers.from_dict({"ullink:isin": "US0378331005", "OMS:InstrumentID": "dbi;X"})
    assert ids.into_dict() == {
        "instrumentid": "dbi;X",
        "isin": "US0378331005",
        "oms:instrumentid": "dbi;X",
        "ullink:isin": "US0378331005",
    }
    assert list(ids.into_dict()) == sorted(ids.into_dict()), "a dict is in key order"
    assert Identifiers.from_dict(ids.into_dict()) == ids, "a written map comes back unchanged"
    derived = Identifiers.from_dict({"derived:cusip": "037833100"})
    assert derived.into_dict() == {"cusip": "037833100", "derived:cusip": "037833100"}
    assert derived.is_derived("cusip"), "a derivation read back stays one"
    assert Identifiers.from_dict({}) == Identifiers()


def test_a_dict_refuses_what_states_no_identifier_naming_the_key() -> None:
    with pytest.raises(ValueError, match="fix:"):
        Identifiers.from_dict({"fix:": "X"})
    with pytest.raises(ValueError, match="isin"):
        Identifiers.from_dict({"isin": "US037833100"})
    with pytest.raises(ValueError):
        Identifiers.from_dict({"isin": "US0378331005", "BASE:ISIN": "CH0012214059"})
    with pytest.raises(ValueError, match="orderid"):
        Identifiers.from_dict({"orderid": 1})


def test_a_map_compares_hashes_copies_and_pickles_as_a_value() -> None:
    ids = Identifiers([Identifier("orderid", "O-1")])
    assert ids == Identifiers([Identifier("fix:orderid", "O-1")])
    assert ids != Identifiers([Identifier("orderid", "O-2")]), "the value is part of the identity"
    assert ids != {"orderid": "O-1"}
    assert hash(ids) == hash(copy.deepcopy(ids))
    assert pickle.loads(pickle.dumps(ids)) == ids
    derived = Identifiers.from_dict({"derived:cusip": "037833100"})
    restored = pickle.loads(pickle.dumps(derived))
    assert restored == derived and restored.is_derived("cusip"), "a pickle keeps a derivation one"
    assert repr(ids) == "Identifiers([orderid=O-1])"
