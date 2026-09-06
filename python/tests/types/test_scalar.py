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


def test_a_value_answers_what_shape_it_is_without_lowering_it() -> None:
    assert Scalar.from_py(None).is_null()
    assert not Scalar.from_py(0).is_null()

    assert Scalar.from_py([1, 2]).is_container()
    assert Scalar.from_py({"a": 1}).is_container()
    assert not Scalar.from_py("text").is_container()

    assert Scalar.from_py(1).is_number()
    assert Scalar.float(1.5).is_number()
    assert Scalar.decimal(150, 2).is_number()
    assert not Scalar.from_py("1").is_number()
    assert not Scalar.from_py(True).is_number()

    assert Scalar.from_py(1).is_integer()
    assert not Scalar.float(1.5).is_integer()
    assert not Scalar.decimal(150, 2).is_integer()


def test_a_value_narrows_to_the_python_number_it_is() -> None:
    assert Scalar.from_py(True).as_bool() is True
    assert Scalar.from_py(1).as_bool() is None

    # Python integers are unbounded, so nothing wraps and nothing narrows.
    assert Scalar.from_py(7).as_int() == 7
    assert Scalar.from_py(-7).as_int() == -7
    assert Scalar.from_py(2**63).as_int() == 2**63
    assert Scalar.from_py("7").as_int() is None
    assert Scalar.float(1.5).as_int() is None

    # A 32-bit float widens exactly, so both widths answer here.
    assert Scalar.float(1.5, 32).as_float() == 1.5
    assert Scalar.float(1.5, 64).as_float() == 1.5
    assert Scalar.from_py(1).as_float() is None


def test_the_payload_bytes_of_a_value_carry_no_tag_and_no_length() -> None:
    assert Scalar.from_py("AAPL").as_value_bytes() == b"AAPL"
    assert Scalar.from_py(b"\x01\x02").as_value_bytes() == b"\x01\x02"
    assert Scalar.from_py(1).as_value_bytes() == (1).to_bytes(8, "little")

    # Null and a container have no payload of their own.
    assert Scalar.from_py(None).as_value_bytes() is None
    assert Scalar.from_py([1]).as_value_bytes() is None

    # `as_bytes` is the narrower question: the bytes a byte value holds.
    assert Scalar.from_py("AAPL").as_bytes() is None


def test_a_record_says_that_its_names_are_field_names() -> None:
    record = Scalar.from_record({"b": 2, "a": 1})
    assert record.kind == "record"
    assert record.as_py() == {"a": 1, "b": 2}

    # A Python mapping is a mapping; a record is what a struct row resolves to.
    assert Scalar.from_py({"a": 1}).kind == "mapping"
    assert Scalar.from_record([("a", 1), ("b", 2)]).kind == "record"

    with pytest.raises(ValueError):
        Scalar.from_record([("a", 1), ("a", 2)])
    with pytest.raises(TypeError):
        Scalar.from_record([(1, "a")])


def test_a_default_answers_for_an_absent_name_and_for_a_stored_null() -> None:
    value = Scalar.from_py({"symbol": "AAPL", "venue": None})

    assert value.get_or("symbol", "?").as_py() == "AAPL"
    # A stored null is no answer, which is what a configuration default means.
    assert value.get_or("venue", "XNAS").as_py() == "XNAS"
    assert value.get_or("absent", "XNAS").as_py() == "XNAS"

    record = Scalar.from_record({"symbol": "MSFT"})
    assert record.get_or("symbol", "?").as_py() == "MSFT"
    assert record.get_or("absent", 0).as_py() == 0
