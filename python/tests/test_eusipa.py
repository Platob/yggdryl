"""`python/yggdryl/eusipa.py` and `python/src/eusipa.rs`: the EUSIPA product
category of a structured product - a four-digit code held by its shape, named
by the European Derivative Map of February 2024 and by the SSPA Swiss
Derivative Map of 2023 and 2026 where each lists it - redirected to the core."""

from __future__ import annotations

import copy
import pickle

import pytest

import yggdryl
from yggdryl import Eusipa


def test_a_code_that_is_no_four_digit_investment_or_leverage_category_is_refused() -> None:
    for refused in (0, 7, 999, 3000, 3100, 9999, 10_000, 65_535):
        with pytest.raises(ValueError, match=f"EUSIPA product category.*{refused}"):
            Eusipa(refused)
    # A category is a value, no datatype: its refusal is a value's.
    with pytest.raises(ValueError) as error:
        Eusipa(3100)
    assert str(error.value) == (
        "invalid record value at $: expected a four-digit EUSIPA product category opening with 1, "
        "an investment product, or 2, a leverage product, got 3100"
    )
    for text in ("", " ", "22", "230", "23000", "02300", "+2300", "-2300", "2300.0", "23 00", "abcd", "２３００", "3100"):
        with pytest.raises(ValueError):
            Eusipa(text)
    with pytest.raises(ValueError) as error:
        Eusipa("23x0")
    assert str(error.value) == 'invalid record value at $: expected a four-digit EUSIPA product category, got "23x0"'
    # An integer no `u16` holds is an overflow; anything but an int or a str a type error.
    for wide in (-1, 65_536, 1 << 80):
        with pytest.raises(OverflowError):
            Eusipa(wide)
    for other in (True, 2300.0, b"2300", None):
        with pytest.raises(TypeError, match="code must be an int or a str"):
            Eusipa(other)  # type: ignore[arg-type]


def test_a_code_of_its_shape_is_held_whatever_either_map_lists() -> None:
    # The maps are snapshots of lists that evolve: a code of the shape no map
    # lists - a member added later, or one retired - is a code.
    for held in (1000, 1110, 1999, 2000, 2301, 2999):
        category = Eusipa(held)
        assert category.code == held
        assert not category.is_listed
        assert category.name is None and category.sspa_name is None
    constant = Eusipa(2300)
    assert (constant.code, constant.group, constant.level) == (2300, 23, 2)
    assert (Eusipa(1260).group, Eusipa(1260).level) == (12, 1)
    assert int(constant) == 2300


def test_a_code_reads_as_four_digits_trimmed_and_displays_as_them() -> None:
    assert Eusipa(" 2300 ") == Eusipa(2300)
    assert Eusipa("1100") == Eusipa(1100)
    assert str(Eusipa(2300)) == "2300" and str(Eusipa(1000)) == "1000"
    assert repr(Eusipa(2300)) == "Eusipa(2300)"
    for held in (1100, 1260, 2205, 2399):
        assert Eusipa(str(Eusipa(held))) == Eusipa(held)
    assert Eusipa(1100) < Eusipa(2300), "codes order by number"
    assert sorted([Eusipa(2300), Eusipa(1100)]) == [Eusipa(1100), Eusipa(2300)]


def test_equality_hash_copy_and_pickle() -> None:
    constant = Eusipa(2300)
    assert constant == Eusipa("2300") and constant != Eusipa(2301)
    assert constant != 2300, "a category is not its number"
    assert hash(constant) == hash(Eusipa("2300"))
    assert len({constant, Eusipa(2300), Eusipa(1260)}) == 2
    assert copy.copy(constant) == constant and copy.deepcopy(constant) == constant
    assert pickle.loads(pickle.dumps(constant)) == constant
    assert yggdryl.Eusipa is Eusipa and "Eusipa" in yggdryl.__all__


EUROPEAN = [
    (1100, "Uncapped Capital Protection"),
    (1120, "Capped Capital Protection"),
    (1130, "Capital Protection with Knock-Out"),
    (1140, "Capital Protection with Coupon"),
    (1199, "Miscellaneous Capital Protection"),
    (1200, "Discount Certificates"),
    (1210, "Barrier Discount Certificates"),
    (1220, "Reverse Convertibles"),
    (1230, "Barrier Reverse Convertibles"),
    (1240, "Capped Outperformance Certificates"),
    (1250, "Capped Bonus Certificates"),
    (1260, "Express Certificates"),
    (1299, "Miscellaneous Yield Enhancement"),
    (1300, "Tracker Certificates"),
    (1310, "Outperformance Certificates"),
    (1320, "Bonus Certificates"),
    (1330, "Outperformance Bonus Certificates"),
    (1340, "Twin-Win Certificates"),
    (1399, "Miscellaneous Participation"),
    (1440, "Credit Linked Note - Linear"),
    (1450, "Credit Linked Note - Equity Tranche"),
    (1460, "Credit Linked Note - Mezz./Senior Tranche"),
    (1499, "Miscellaneous Credit Linked Notes"),
    (2100, "Warrants"),
    (2110, "Spread Warrants"),
    (2199, "Miscellaneous"),
    (2200, "Knock-Out Warrants"),
    (2205, "Open-end Knock-Out Warrants"),
    (2210, "Mini-Futures"),
    (2230, "Double Knock-Out Warrants"),
    (2299, "Miscellaneous"),
    (2300, "Constant Leverage Certificate"),
    (2399, "Miscellaneous Constant Leverage Products"),
]

SWISS = [
    (1100, "Capital Protection Note with Participation"),
    (1130, "Capital Protection Note with Barrier"),
    (1135, "Capital Protection Note with Twin Win"),
    (1140, "Capital Protection Note with Coupon"),
    (1200, "Discount Certificate"),
    (1210, "Barrier Discount Certificate"),
    (1220, "Reverse Convertible"),
    (1230, "Barrier Reverse Convertible"),
    (1255, "Conditional Coupon Reverse Convertible"),
    (1260, "Conditional Coupon Barrier Reverse Convertible"),
    (1300, "Tracker Certificate"),
    (1310, "Outperformance Certificate"),
    (1320, "Bonus Certificate"),
    (1330, "Bonus Outperformance Certificate"),
    (1340, "Twin Win Certificate"),
    (1400, "Credit Linked Notes"),
    (1410, "Conditional Capital Protection Note with add. credit risk"),
    (1420, "Yield Enhancement Certificate with add. credit risk"),
    (1430, "Participation Certificate with add. credit risk"),
    (2100, "Warrant"),
    (2110, "Spread Warrant"),
    (2200, "Warrant with Knock-Out"),
    (2210, "Mini-Future"),
    (2300, "Constant Leverage Certificate"),
]


def test_each_map_names_every_member_it_lists_and_no_other() -> None:
    every = [Eusipa(held) for held in range(1000, 3000)]
    assert [(category.code, category.name) for category in every if category.name is not None] == EUROPEAN
    assert [(category.code, category.sspa_name) for category in every if category.sspa_name is not None] == SWISS
    listed = {held for held, _ in EUROPEAN} | {held for held, _ in SWISS}
    assert {category.code for category in every if category.is_listed} == listed


def test_the_two_maps_disagree_on_1260_and_each_lists_members_the_other_lacks() -> None:
    express = Eusipa(1260)
    assert express.name == "Express Certificates"
    assert express.sspa_name == "Conditional Coupon Barrier Reverse Convertible"
    for swiss_only in (1135, 1255, 1400, 1410, 1420, 1430):
        category = Eusipa(swiss_only)
        assert category.name is None and category.sspa_name is not None and category.is_listed
    for european_only in (1120, 1199, 1240, 1250, 1440, 1450, 1460, 2205, 2230, 2399):
        category = Eusipa(european_only)
        assert category.name is not None and category.sspa_name is None
    assert Eusipa(2300).name == Eusipa(2300).sspa_name
