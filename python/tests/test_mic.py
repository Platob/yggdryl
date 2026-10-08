"""The ISO 10383 readings `yggdryl.enums.Mic` redirects to: `python/src/mic.rs`."""

from __future__ import annotations

import pytest

from yggdryl._native import mic_country, mic_is_segment, mic_operating


def test_a_segment_answers_its_operating_market_and_its_country() -> None:
    assert mic_operating("XNGS") == "XNAS" and mic_is_segment("XNGS")
    assert mic_operating("XNAS") == "XNAS" and not mic_is_segment("XNAS")
    assert mic_country("XNGS") == "US" and mic_country("XLON") == "GB"


def test_a_code_the_registry_never_assigned_or_placed_answers_none() -> None:
    assert mic_operating("QQQQ") is None and not mic_is_segment("QQQQ") and mic_country("QQQQ") is None
    assert mic_country("XOFF") is None and mic_country("XXXX") is None


def test_a_code_of_no_mic_shape_is_refused_by_the_core() -> None:
    with pytest.raises(ValueError):
        mic_operating("TOOLONG")
