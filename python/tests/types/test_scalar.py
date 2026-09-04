"""Edges of the one Python-object/native-Scalar conversion pair."""

from __future__ import annotations

import datetime as dt
import math
import zoneinfo
from decimal import Decimal
from typing import Any

import pytest

from yggdryl import Scalar
from yggdryl.text import json


def crosses(value: object) -> Any:
    """Cross the native boundary without a lossy document intermediate."""

    return Scalar.from_py(value).as_py()


def test_a_decimal_keeps_its_width_coefficient_and_scale() -> None:
    assert str(crosses(Decimal("1.50"))) == "1.50"
    assert str(crosses(Decimal("1.5"))) == "1.5"
    assert crosses(Decimal("1.05E+5")) == Decimal("1.05E+5")
    assert str(crosses(Decimal("1.05E+5"))) == "1.05E+5"

    widest_d128 = Decimal(2**127 - 1)
    assert Scalar.from_py(widest_d128).kind == "d128"
    assert crosses(widest_d128) == widest_d128

    d256 = Decimal("1" * 40)
    assert Scalar.from_py(d256).kind == "d256"
    assert crosses(d256) == d256


def test_a_non_finite_decimal_becomes_the_float_that_can_name_it() -> None:
    assert math.isnan(crosses(Decimal("NaN")))
    assert crosses(Decimal("-Infinity")) == -math.inf
    # Decimal stores no sign bit on its zero coefficient in the native model.
    assert str(crosses(Decimal("-0.00"))) == "0.00"


def test_a_decimal_wider_than_d256_is_refused_not_rounded() -> None:
    with pytest.raises(OverflowError, match="256 bits"):
        Scalar.from_py(Decimal("1" * 80))
    with pytest.raises(OverflowError, match="no scale in -128..=127"):
        Scalar.from_py(Decimal("1E+200"))


def test_natural_json_has_no_private_value_envelopes() -> None:
    encoded = json.dumps(
        {
            "price": Decimal("1.50"),
            "at": dt.datetime(2026, 8, 15, 12, 3, 4, 5),
        }
    )
    assert json.loads(encoded) == {
        "price": "1.50",
        "at": "2026-08-15T12:03:04.000005",
    }


def test_temporals_cross_as_typed_native_scalars() -> None:
    values = [
        dt.date(2026, 8, 15),
        dt.time(23, 59, 59, 999_999),
        dt.datetime(2026, 8, 15, 12, 3, 4, 5),
        dt.timedelta(days=-2, seconds=3, microseconds=4),
    ]
    assert [crosses(value) for value in values] == values
    assert [Scalar.from_py(value).kind for value in values] == [
        "date32",
        "time64",
        "datetime64",
        "duration64",
    ]


def test_an_aware_datetime_preserves_the_instant_and_zone() -> None:
    paris = zoneinfo.ZoneInfo("Europe/Paris")
    value = dt.datetime(2026, 8, 15, 12, 3, 4, 5, tzinfo=paris)
    restored = crosses(value)
    assert restored == value
    assert restored.tzinfo.key == "Europe/Paris"

    offset = dt.timezone(dt.timedelta(hours=-3, minutes=-30))
    fixed = dt.datetime(2026, 1, 1, tzinfo=offset)
    assert crosses(fixed) == fixed


def test_an_ambiguous_zoned_reading_keeps_the_selected_instant() -> None:
    paris = zoneinfo.ZoneInfo("Europe/Paris")
    repeated = [
        dt.datetime(2026, 10, 25, 2, 30, fold=fold, tzinfo=paris)
        for fold in (0, 1)
    ]
    restored = [crosses(value) for value in repeated]
    assert [value.fold for value in restored] == [0, 1]
    assert [value.utcoffset() for value in restored] == [
        dt.timedelta(hours=2),
        dt.timedelta(hours=1),
    ]


def test_a_naive_fold_is_dropped_and_zoned_times_are_refused() -> None:
    assert crosses(dt.datetime(2026, 10, 25, 2, 30, fold=1)) == dt.datetime(
        2026, 10, 25, 2, 30
    )
    with pytest.raises(ValueError, match="timezone"):
        Scalar.from_py(dt.time(1, 2, tzinfo=dt.timezone.utc))


def test_a_temporal_python_cannot_hold_is_refused_not_truncated() -> None:
    with pytest.raises(ValueError, match="no exact microsecond count"):
        Scalar.datetime(1, "ns", "UTC").as_py()

    with pytest.raises(OverflowError, match="microseconds a duration counts"):
        Scalar.from_py(dt.timedelta.max)

    with pytest.raises(ValueError, match="within one day of midnight"):
        Scalar.time(99_999_999, "s").as_py()


def test_a_zone_with_no_rules_anywhere_is_named_in_the_error() -> None:
    with pytest.raises(ValueError, match='"Mars/Olympus"'):
        Scalar.datetime(0, "s", "Mars/Olympus").as_py()


def test_a_coarser_unit_is_restated_exactly() -> None:
    value = Scalar.datetime(1_700_000_000, "s", "UTC")
    assert value.as_py() == dt.datetime(
        2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc
    )


def test_none_crosses_everywhere_a_native_scalar_goes() -> None:
    assert crosses(None) is None
    assert crosses([None, 1]) == [None, 1]
    assert crosses({"gap": None}) == {"gap": None}
    assert crosses({None: 1}) == {None: 1}


def test_mapping_keys_cross_in_a_hashable_python_shape() -> None:
    assert crosses({(1, 2): "pair"}) == {(1, 2): "pair"}
    assert crosses({frozenset({1}): "one"}) == {(1,): "one"}


def test_equal_cross_width_numbers_share_a_hash() -> None:
    f32 = Scalar.float(1.0, 32)
    f64 = Scalar.float(1.0)
    assert f32 == f64
    assert hash(f32) == hash(f64)
