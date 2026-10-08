"""The ISO 4217 reading `yggdryl.enums.Country.currency` redirects to:
`python/src/country.rs`."""

from __future__ import annotations

import pytest

from yggdryl._native import country_currency


def test_a_country_answers_the_one_tender_list_one_gives_it() -> None:
    assert country_currency("US") == "USD"
    assert country_currency("GB") == "GBP"
    assert country_currency("EC") == "USD", "the override table settles a two-tender country"
    assert country_currency("XX") is None and country_currency("ZZ") is None


def test_a_code_of_no_country_shape_is_refused_by_the_core() -> None:
    with pytest.raises(ValueError):
        country_currency("USA")
