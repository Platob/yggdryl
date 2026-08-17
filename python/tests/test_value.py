"""Edge cases of the one Python-object/native-value conversion pair.

Every codec entry point routes through it, so JSON is used here only as the
cheapest way to make a value cross the boundary and come back.
"""

from __future__ import annotations

import datetime as dt
import math
import zoneinfo
from decimal import Decimal
from typing import Any

import pytest

from yggdryl import json


def crosses(value: object) -> Any:
    """Send one value through the native codec and read it back.

    The decoded type is what the test is checking, so it stays dynamic here.
    """

    return json.loads(json.dumps({"value": value}))["value"]


def test_a_decimal_keeps_its_coefficient_and_its_scale() -> None:
    # The scale is data: two spellings of one number stay two spellings.
    assert str(crosses(Decimal("1.50"))) == "1.50"
    assert str(crosses(Decimal("1.5"))) == "1.5"
    # A negative scale multiplies, exactly as Arrow allows.
    assert crosses(Decimal("1.05E+5")) == Decimal("1.05E+5")
    assert str(crosses(Decimal("1.05E+5"))) == "1.05E+5"
    # The full 128-bit coefficient survives.
    widest = Decimal(2**127 - 1)
    assert crosses(widest) == widest


def test_a_decimal_that_is_not_a_number_crosses_as_the_float_that_is() -> None:
    assert math.isnan(crosses(Decimal("NaN")))
    assert crosses(Decimal("-Infinity")) == -math.inf
    # A negative zero has no coefficient to carry its sign.
    assert str(crosses(Decimal("-0.00"))) == "0.00"


def test_a_decimal_wider_than_the_native_one_is_refused_not_rounded() -> None:
    with pytest.raises(OverflowError, match="128 bits"):
        json.dumps(Decimal("1" * 40))
    with pytest.raises(OverflowError, match="no scale in -128..=127"):
        json.dumps(Decimal("1E+200"))


def test_temporals_cross_as_their_classic_iso_strings() -> None:
    # A schemaless wire spells a temporal the way every other tool does, so
    # what comes back is the string - loosely typed on purpose. The typed
    # reading comes back wherever a schema names the column's datatype.
    spelled = {
        dt.date(2026, 8, 15): "2026-08-15",
        dt.time(23, 59, 59, 999_999): "23:59:59.999999",
        dt.datetime(2026, 8, 15, 12, 3, 4, 5): "2026-08-15T12:03:04.000005",
        dt.timedelta(days=-2, seconds=3, microseconds=4): "-PT172796.999996S",
    }

    for value, expected in spelled.items():
        assert crosses(value) == expected
        # The string parses back to the same reading with the standard tools.
        if isinstance(value, dt.date) and not isinstance(value, dt.datetime):
            assert dt.date.fromisoformat(expected) == value


def test_an_aware_datetime_spells_its_offset_and_its_zone() -> None:
    paris = zoneinfo.ZoneInfo("Europe/Paris")
    value = dt.datetime(2026, 8, 15, 12, 3, 4, 5, tzinfo=paris)

    # The local reading, the offset that recovers the instant, and the zone
    # name in brackets, because the offset alone cannot say Europe/Paris.
    assert crosses(value) == "2026-08-15T12:03:04.000005+02:00[Europe/Paris]"
    # A fixed offset comes back as a fixed offset, not as a place.
    offset = dt.timezone(dt.timedelta(hours=-3, minutes=-30))
    assert crosses(dt.datetime(2026, 1, 1, tzinfo=offset)) == (
        "2026-01-01T00:00:00.000000-03:30"
    )


def test_an_ambiguous_zoned_reading_spells_the_offset_that_disambiguates_it() -> None:
    paris = zoneinfo.ZoneInfo("Europe/Paris")
    repeated = [
        dt.datetime(2026, 10, 25, 2, 30, fold=fold, tzinfo=paris) for fold in (0, 1)
    ]

    # The count is UTC, so the offset in force is what carries the answer; the
    # two readings of the repeated hour stay two distinct spellings.
    restored = [crosses(value) for value in repeated]
    assert restored == [
        "2026-10-25T02:30:00.000000+02:00[Europe/Paris]",
        "2026-10-25T02:30:00.000000+01:00[Europe/Paris]",
    ]


def test_a_naive_fold_and_a_time_of_day_zone_are_dropped() -> None:
    # Neither has anywhere to live: a naive timestamp has no offset to move by,
    # and a time of day has no zone field at all.
    assert crosses(dt.datetime(2026, 10, 25, 2, 30, fold=1)) == (
        "2026-10-25T02:30:00.000000"
    )
    assert crosses(dt.time(1, 2, tzinfo=dt.timezone.utc)) == "01:02:00.000000"


def test_a_temporal_python_cannot_hold_is_refused_not_truncated() -> None:
    nanosecond = b'{"x":{"$yggdryl":{"version":1,"type":"timestamp","value":["ns",1]}}}'
    with pytest.raises(ValueError, match="no exact microsecond count"):
        json.loads(nanosecond)

    with pytest.raises(OverflowError, match="microseconds a duration counts"):
        json.dumps(dt.timedelta.max)

    beyond_midnight = b'{"x":{"$yggdryl":{"version":1,"type":"time","value":["s",99999999]}}}'
    with pytest.raises(ValueError, match="within one day of midnight"):
        json.loads(beyond_midnight)


def test_a_zone_with_no_rules_anywhere_is_named_in_the_error() -> None:
    unknown = (
        b'{"x":{"$yggdryl":{"version":1,"type":"timestamp",'
        b'"value":["s",0,"Mars/Olympus"]}}}'
    )

    with pytest.raises(ValueError, match='"Mars/Olympus"'):
        json.loads(unknown)


def test_a_coarser_unit_is_restated_rather_than_refused() -> None:
    seconds = (
        b'{"x":{"$yggdryl":{"version":1,"type":"timestamp",'
        b'"value":["s",1700000000,"UTC"]}}}'
    )

    assert json.loads(seconds) == {
        "x": dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc)
    }


def test_none_crosses_everywhere_a_value_goes() -> None:
    # Null is a value, not a trap: alone, inside a sequence, as a mapping
    # value, and even as a mapping key, it crosses and comes back as None.
    assert crosses(None) is None
    assert crosses([None, 1]) == [None, 1]
    assert crosses({"gap": None}) == {"gap": None}
    assert json.loads(json.dumps({None: 1})) == {None: 1}


def test_a_mapping_used_as_a_key_crosses_as_the_tuple_of_its_entries() -> None:
    # JSON and YAML have no unhashable keys, so the hashable spelling is what a
    # key becomes. The record layer reads that shape back as a mapping.
    assert crosses({(1, 2): "pair"}) == {(1, 2): "pair"}
    assert crosses({frozenset({1}): "one"}) == {(1,): "one"}


def test_two_distinct_keys_that_collide_in_python_are_reported() -> None:
    with pytest.raises(ValueError, match="mapping keys collide"):
        json.loads(b'{"$yggdryl":{"version":1,"type":"mapping","value":[[1,"a"],[1.0,"b"]]}}')
