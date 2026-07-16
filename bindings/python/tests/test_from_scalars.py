"""Tests for the ``from_scalars`` staticmethod on every columnar ``Serie`` wrapper, and the
native-language temporal column factories (``from_dates`` / ``from_times`` / ``from_datetimes`` /
``from_timedeltas``).

``from_scalars`` is the exact inverse of ``get_scalar`` over a whole column: it takes a list of the
type ``get_scalar`` hands back — the ``Scalar`` wrapper for the fixed / decimal / var / null
families, and the temporal **value** type (e.g. ``Ts64``) for the temporal columns — where a
``None`` item is a null element.
"""

import datetime as dt

import pytest

from yggdryl.decimal import D128Scalar, D128Serie
from yggdryl.temporal import (
    Date32,
    Date32Serie,
    Duration64,
    Duration64Serie,
    Time64,
    Time64Serie,
    Ts64,
    Ts64Serie,
)
from yggdryl.types import (
    BinaryScalar,
    BinarySerie,
    I64Scalar,
    I64Serie,
    NullScalar,
    NullSerie,
    U8Scalar,
    U8Serie,
    Utf8Scalar,
    Utf8Serie,
)


def _round_trip(cls, column):
    """``cls.from_scalars([col.get_scalar(i) …])`` reconstructs ``col`` exactly."""
    scalars = [column.get_scalar(i) for i in range(len(column))]
    return cls.from_scalars(scalars)


# ---- Deliverable 1: from_scalars round-trip, one wrapper per family ---------------------


def test_fixed_from_scalars_round_trip():
    col = U8Serie([1, None, 3, 255])
    assert _round_trip(U8Serie, col) == col

    # A None list item is a null element; explicit scalars work too.
    built = I64Serie.from_scalars([I64Scalar("10"), None, I64Scalar.null(), I64Scalar("-20")])
    assert built.to_options() == ["10", None, None, "-20"]

    # The empty list yields the empty column.
    assert U8Serie.from_scalars([]) == U8Serie()


def test_decimal_from_scalars_round_trip():
    col = D128Serie(20, 2, ["12.34", None, "6.00"])
    rebuilt = _round_trip(D128Serie, col)
    assert rebuilt == col
    # The column (precision, scale) is recovered from the first present scalar.
    assert rebuilt.precision == 20
    assert rebuilt.scale == 2

    built = D128Serie.from_scalars([D128Scalar("2.5", 10, 2), None])
    assert built.to_options() == ["2.50", None]
    assert built.precision == 10 and built.scale == 2


def test_var_from_scalars_round_trip():
    utf8 = Utf8Serie(["a", None, "cd", ""])
    assert _round_trip(Utf8Serie, utf8) == utf8

    binary = BinarySerie([b"\x00\x01", None, b"xyz"])
    assert _round_trip(BinarySerie, binary) == binary

    built = Utf8Serie.from_scalars([Utf8Scalar("hi"), None])
    assert built.to_options() == ["hi", None]


def test_null_from_scalars_round_trip():
    col = NullSerie(4)
    assert _round_trip(NullSerie, col) == col
    # A None item and a NullScalar both mean a null element.
    assert len(NullSerie.from_scalars([NullScalar(), None, NullScalar()])) == 3
    assert NullSerie.from_scalars([]) == NullSerie()


def test_temporal_from_scalars_round_trip():
    # The temporal columns take the VALUE wrapper get_scalar returns (e.g. a Ts64), with None nulls.
    col = Ts64Serie("us", "UTC", ["2021-01-01T00:00:00Z", None, "2021-01-02T00:00:00Z"])
    scalars = [col.get_scalar(i) for i in range(len(col))]
    assert scalars[1] is None  # a null element surfaces as Python None
    assert isinstance(scalars[0], Ts64)
    rebuilt = Ts64Serie.from_scalars(scalars)
    assert rebuilt == col
    assert rebuilt.unit == "us"
    assert rebuilt.timezone == "UTC"

    # A date column round-trips through its Date32 values.
    dates = Date32Serie("d", "naive", ["2020-02-29", None, "2021-07-15"])
    assert Date32Serie.from_scalars([dates.get_scalar(i) for i in range(len(dates))]) == dates


# ---- Deliverable 2: native-language temporal column factories ---------------------------


def test_from_dates():
    values = [dt.date(2021, 1, 1), None, dt.date(2021, 3, 1)]
    col = Date32Serie.from_dates(values)
    assert col.unit == "d"
    assert col.null_count == 1
    assert col.get_scalar(0).to_pydate() == dt.date(2021, 1, 1)
    assert col.get_scalar(1) is None
    assert col.get_scalar(2).to_pydate() == dt.date(2021, 3, 1)


def test_from_times():
    values = [dt.time(1, 2, 3, 456789), None, dt.time(10, 20, 30)]
    col = Time64Serie.from_times("us", values)
    assert col.unit == "us"
    assert col.null_count == 1
    assert col.get_scalar(0).to_pytime() == dt.time(1, 2, 3, 456789)
    assert col.get_scalar(1) is None
    assert col.get_scalar(2).to_pytime() == dt.time(10, 20, 30)


def test_from_datetimes():
    values = [dt.datetime(2021, 1, 1, 12, 30, 15, 500000), None, dt.datetime(2021, 6, 1, 0, 0, 0)]
    col = Ts64Serie.from_datetimes("us", "naive", values)
    assert col.unit == "us"
    assert col.null_count == 1
    assert col.get_scalar(0).to_pydatetime() == dt.datetime(2021, 1, 1, 12, 30, 15, 500000)
    assert col.get_scalar(1) is None
    assert col.get_scalar(2).to_pydatetime() == dt.datetime(2021, 6, 1, 0, 0, 0)


def test_from_timedeltas():
    values = [dt.timedelta(seconds=90), None, dt.timedelta(days=1, microseconds=250000)]
    col = Duration64Serie.from_timedeltas("us", values)
    assert col.unit == "us"
    assert col.null_count == 1
    assert col.get_scalar(0).to_timedelta() == dt.timedelta(seconds=90)
    assert col.get_scalar(1) is None
    assert col.get_scalar(2).to_timedelta() == dt.timedelta(days=1, microseconds=250000)
