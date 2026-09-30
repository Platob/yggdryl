"""`python/yggdryl/identifier.py` and `python/src/identifier.rs`: the identifier
and the sorted map an element states them in, keyed `src:type`, redirected to
the core."""

from __future__ import annotations

import copy
import pickle

import pytest

from yggdryl import Identifier, Identifiers


def test_an_identifier_folds_its_words_and_trims_its_value() -> None:
    held = Identifier("bic", "executing_trader", " T-1 ")
    assert (held.src, held.type, held.value) == ("bic", "executingtrader", "T-1")
    assert str(held) == "bic:executingtrader=T-1"
    assert repr(held) == "Identifier('bic', 'executingtrader', 'T-1')"
    assert held.is_of("BIC", "Executing Trader")
    assert not held.is_of("fix", "executingtrader")
    folded = Identifier("FIX", "CLORDID", "C-2")
    assert (folded.src, folded.type) == ("fix", "clordid")


def test_an_identifier_is_three_texts_and_states_no_parentage() -> None:
    with pytest.raises(TypeError):
        Identifier("fix", "clordid", "C-2", "C-1", "C-0")  # type: ignore[call-arg]
    with pytest.raises(TypeError):
        Identifier("fix", "clordid", "C-2", parent="C-1")  # type: ignore[call-arg]
    held = Identifier("fix", "clordid", "C-2")
    assert not hasattr(held, "parent") and not hasattr(held, "orig")


def test_an_identifier_names_its_unique_key_src_type() -> None:
    held = Identifier("Fix", "ClOrdID", "C-1")
    assert held.key == "fix:clordid"
    assert held.key == f"{held.src}:{held.type}"
    assert Identifier("firm.x", "house code", "hc-1").key == "firm.x:housecode"
    assert Identifier("base", "isin", "US0378331005").key == "base:isin"
    assert Identifier("fix", "orderid", "A").key == Identifier("fix", "orderid", "B").key, "a key names, never a value"
    assert Identifier("fix", "orderid", "A").key != Identifier("venue", "orderid", "A").key
    assert Identifier("fix", "orderid", "A").key != Identifier("fix", "clordid", "A").key
    read = Identifier.from_key(held.key, held.value)
    assert read == held, "the key is what from_key reads back"


def test_a_source_and_a_type_are_lower_case_words_and_a_value_is_left_as_given() -> None:
    held = Identifier("BASE", "ISIN", "us0378331005")
    assert (held.src, held.type) == ("base", "isin")
    assert held.value == "US0378331005", "an ISIN is upper-cased as its type stores it"
    assert str(held) == "base:isin=US0378331005"
    assert Identifier("Fix", "OrderID", "O-1").value == "O-1"
    assert Identifier("fix", "isinnumber", "US0378331005").type == "isin"


def test_an_identifier_refuses_what_is_no_word_and_a_value_that_states_nothing() -> None:
    with pytest.raises(ValueError, match="identifier type"):
        Identifier("fix", "café", "X")
    with pytest.raises(ValueError, match="identifier type"):
        Identifier("fix", "", "X")
    with pytest.raises(ValueError, match="identifier source"):
        Identifier("café", "orderid", "X")
    with pytest.raises(ValueError, match="identifier value"):
        Identifier("fix", "orderid", "n/a")
    with pytest.raises(ValueError, match="identifier value"):
        Identifier("fix", "orderid", "")


def test_a_security_type_checks_its_code() -> None:
    apple = Identifier("fix", "isin", " us0378331005 ")
    assert (apple.src, apple.type, apple.value) == ("fix", "isin", "US0378331005")
    with pytest.raises(ValueError):
        Identifier("fix", "isin", "US0378331006")
    pair = Identifier("base", "forex", "eur/usd")
    assert pair.value == "EUR/USD"


def test_a_key_names_its_source_and_its_type() -> None:
    stated = Identifier.from_key("fix:ClOrdID", "C-1")
    assert stated is not None and (stated.src, stated.type) == ("fix", "clordid")
    spelled = Identifier.from_key("firm.x:house code", "hc-1")
    assert spelled is not None and str(spelled) == "firm.x:housecode=hc-1"
    bare = Identifier.from_key("ClOrdID", "C-1")
    assert bare is not None and (bare.src, bare.type) == ("base", "clordid")
    assert Identifier.from_key("ClOrdID", "null") is None


def test_a_key_is_read_for_the_identifier_name_it_ends_with_and_the_source_before_it() -> None:
    def read(key: str, value: str) -> str:
        keyed = Identifier.from_key(key, value)
        assert keyed is not None, key
        return str(keyed)

    assert read("firm.x.ParentOrderID", "P-1") == "firm.x:parentorderid=P-1"
    assert read("OMS_InstrumentID", "dbi;X") == "oms:instrumentid=dbi;X"
    assert read("marketorderid", "O-1") == "market:orderid=O-1"
    assert read("OMSDealerParentOrderID", "P-1") == "omsdealer:parentorderid=P-1"
    assert read("OMSUserID", "U-1") == "oms:userid=U-1"
    assert read("OrderID", "O-1") == "base:orderid=O-1"
    assert read("firm..x_OrderID", "O-1") == "firm..x:orderid=O-1", "a dot inside is kept"
    assert read(".OrderID", "O-1") == "base:orderid=O-1", "a dot at the end is trimmed"
    assert read("oms#orderid", "O-1") == "oms:orderid=O-1", "folding drops #"
    assert read("OrigClOrdID", "C-0") == "base:origclordid=C-0"
    assert read("MyOrigClOrdID", "C-0") == "my:origclordid=C-0", "the longest name the key ends with answers"
    assert read("ullink.InstrumentId", "dbi;CH0012214059_XSWX_CHF") == "ullink:instrumentid=dbi;CH0012214059_XSWX_CHF"


def test_a_whole_security_name_is_that_type_from_base() -> None:
    def read(key: str, value: str) -> str:
        keyed = Identifier.from_key(key, value)
        assert keyed is not None, key
        return str(keyed)

    assert read("ISINCode", "US0378331005") == "base:isin=US0378331005"
    assert read("security_cusip", "037833100") == "base:cusip=037833100"
    assert read("firm.isin", "us0378331005") == "firm:isin=US0378331005"


def test_a_key_that_names_no_identifier_or_another_instruments_security_is_none() -> None:
    for key, value in (
        ("underlyingisin", "US0378331005"),
        ("legisin", "US0378331005"),
        ("contra.isin", "US0378331005"),
        ("relatedcusip", "037833100"),
        ("benchmarkisin", "US0378331005"),
    ):
        assert Identifier.from_key(key, value) is None, key
    for key in ("transversalkey", "symbol", "ticker", "securityid", "id", "", "   ", "#"):
        assert Identifier.from_key(key, "X-1") is None, key
    contra = Identifier.from_key("contraorderid", "O-1")
    assert contra is not None and str(contra) == "contra:orderid=O-1", "only a security type is refused there"
    assert Identifier.from_key("ISINCode", "US0378331006") is None, "a bad check digit"
    assert Identifier.from_key("firm.x.ParentOrderID", "") is None


def test_identifiers_order_by_their_key_as_spelled_then_by_value() -> None:
    first, second = Identifier("fix", "a", "1"), Identifier("fix", "b", "1")
    assert first < second and sorted([second, first]) == [first, second]
    assert Identifier("alpha", "z", "1") < first, "a source orders before a type"
    # The order is the one of the text `src:type`, never of the (src, type)
    # pair: a '.' or a digit sorts below ':', so a longer source sorts first.
    assert Identifier("a.b", "c", "1") < Identifier("a", "z", "1"), "'a.b:c' < 'a:z'"
    assert Identifier("a1", "x", "1") < Identifier("a", "x", "1"), "'a1:x' < 'a:x'"
    assert Identifier("a", "z", "1") < Identifier("ab", "a", "1"), "':' sorts below a letter"
    assert Identifier("fix", "orderid", "A") < Identifier("fix", "orderid", "B")
    assert Identifier("fix", "orderid", "Z") < Identifier("oms", "clordid", "A"), "a key decides before a value"
    loose = [
        Identifier("a", "z", "1"),
        Identifier("a1", "x", "1"),
        Identifier("a", "x", "2"),
        Identifier("a", "x", "1"),
        Identifier("a.b", "c", "1"),
        Identifier("ab", "a", "1"),
    ]
    assert [str(held) for held in sorted(loose)] == ["a.b:c=1", "a1:x=1", "a:x=1", "a:x=2", "a:z=1", "ab:a=1"]


def test_identifiers_compare_hash_copy_and_pickle_as_values() -> None:
    first = Identifier("fix", "a", "1")
    assert first == Identifier("FIX", "A", "1") and hash(first) == hash(Identifier("fix", "a", "1"))
    assert first != Identifier("fix", "a", "2")
    assert copy.copy(first) == first and copy.deepcopy(first) == first
    clordid = Identifier("fix", "clordid", "C-2")
    assert pickle.loads(pickle.dumps(clordid)) == clordid


def test_a_map_holds_one_identifier_per_key_sorted_by_that_key() -> None:
    ids = Identifiers(
        [
            Identifier("base", "isin", "US0378331005"),
            Identifier("BASE", "ISIN", "CH0012214059"),
            Identifier("derived", "cusip", "037833100"),
            Identifier("ullink", "instrumentid", "dbi;X"),
        ]
    )
    assert len(ids) == 3 and ids
    assert [str(id) for id in ids] == [
        "base:isin=US0378331005",
        "derived:cusip=037833100",
        "ullink:instrumentid=dbi;X",
    ]
    assert [id.key for id in ids] == ["base:isin", "derived:cusip", "ullink:instrumentid"], "iteration is key order"
    assert ids.get("isin") == "US0378331005"
    assert ids.get_from("ullink", "instrument_id") == "dbi;X"
    assert ids.get_from("fix", "ISIN") is None
    assert ids.contains_kind("cusip") and not ids.contains_kind("SEDOL")
    identifier = ids.get_identifier("ISIN")
    assert identifier is not None and identifier.src == "base"
    assert [id.src for id in ids.of_kind("ISIN")] == ["base"]
    assert str(ids) == "[base:isin=US0378331005, derived:cusip=037833100, ullink:instrumentid=dbi;X]"
    assert not Identifiers() and len(Identifiers()) == 0


def test_a_map_is_held_in_the_order_of_its_spelled_keys() -> None:
    ids = Identifiers(
        [
            Identifier("a", "z", "1"),
            Identifier("a1", "x", "1"),
            Identifier("a.b", "c", "1"),
            Identifier("a", "x", "1"),
            Identifier("ab", "a", "1"),
        ]
    )
    assert [id.key for id in ids] == ["a.b:c", "a1:x", "a:x", "a:z", "ab:a"]
    assert ids.get_from("a.b", "c") == "1" and ids.get_from("a1", "x") == "1"
    assert ids == Identifiers(list(reversed(list(ids)))), "the order a map was filled in never shows"


def test_a_stated_source_answers_before_a_derived_one() -> None:
    ids = Identifiers([Identifier("derived", "cusip", "037833100"), Identifier("fix", "cusip", "594918104")])
    assert ids.get("cusip") == "594918104"


def test_a_map_compares_hashes_copies_and_pickles_as_a_value() -> None:
    ids = Identifiers([Identifier("fix", "orderid", "O-1")])
    assert ids == Identifiers([Identifier("fix", "orderid", "O-1")])
    assert ids != Identifiers([Identifier("fix", "orderid", "O-2")]), "the value is part of the identity"
    assert ids != {"orderid": "O-1"}
    assert hash(ids) == hash(copy.deepcopy(ids))
    assert pickle.loads(pickle.dumps(ids)) == ids
