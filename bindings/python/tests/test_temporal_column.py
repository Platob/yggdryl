"""Tests for the ``yggdryl.temporal`` columnar types — one nullable temporal column per
concept+width (``Date32Serie`` / ``Date64Serie``, ``Time32Serie`` / ``Time64Serie``, ``Ts32Serie``
/ ``Ts64Serie`` / ``Ts96Serie``, ``Duration32Serie`` / ``Duration64Serie``), over
``yggdryl_core::io::fixed``'s ``TemporalSerie``.

A column fixes one ``(unit, tz)``. Cells cross two ways: as the value type's ISO-8601 string
(``get`` / ``push`` / ``set`` / constructor) and as the raw epoch/physical count as a Python int
(``get_epoch`` / ``from_epochs``). A column is mutable, so it is unhashable. Each column also speaks
the zero-copy Arrow C Data Interface (a real pyarrow round-trip below).
"""

import copy
import datetime as dt
import pickle

import pytest

import yggdryl.temporal as temporal
from yggdryl.temporal import (
    Date32Serie,
    Date64Serie,
    Duration32Serie,
    Duration64Serie,
    Time32Serie,
    Time64Serie,
    Ts32Serie,
    Ts64Serie,
    Ts96Serie,
)

# (class, unit, tz, three values with one null in the middle, type name)
CASES = [
    (Date32Serie, "d", "naive", ["2021-01-01", None, "2021-03-01"], "date32"),
    (Date64Serie, "ms", "naive", ["2021-01-01", None, "2021-03-01"], "date64"),
    (Time32Serie, "s", "naive", ["01:02:03", None, "10:20:30"], "time32"),
    (Time64Serie, "ns", "naive", ["01:02:03.123456789", None, "10:20:30"], "time64"),
    (Ts32Serie, "s", "UTC", ["2021-01-01T00:00:00Z", None, "2021-01-02T00:00:00Z"], "ts32"),
    (Ts64Serie, "us", "UTC", ["2021-01-01T00:00:00Z", None, "2021-01-02T00:00:00Z"], "ts64"),
    (Ts96Serie, "ns", "UTC", ["2021-01-01T00:00:00Z", None, "2021-01-02T00:00:00Z"], "ts96"),
    (Duration32Serie, "s", "naive", ["90s", None, "-60s"], "duration32"),
    (Duration64Serie, "ms", "naive", ["1500ms", None, "-2s"], "duration64"),
]

ALL_CLASSES = [case[0] for case in CASES]
IDS = [case[4] for case in CASES]


def test_module_surface():
    for cls in ALL_CLASSES:
        assert cls.__module__ == "yggdryl.temporal"
        assert hasattr(temporal, cls.__name__)


@pytest.mark.parametrize("cls, unit, tz, values, name", CASES, ids=IDS)
def test_construction_and_access(cls, unit, tz, values, name):
    col = cls(unit, tz, values)
    assert len(col) == 3
    assert col.null_count == 1 and col.has_nulls and not col.is_empty()
    assert col.unit == unit
    assert col.timezone == ("" if tz == "naive" else tz)
    assert col.data_type.name == name and col.data_type.is_temporal()

    # Present cells cross as an ISO string and a raw epoch int; the null slot is None for both.
    assert isinstance(col.get(0), str) and isinstance(col.get_epoch(0), int)
    assert col.get(1) is None and col.get_epoch(1) is None and col.get_scalar(1) is None

    # get_scalar hands back the temporal *value* wrapper for this width; its str is the cell string.
    scalar = col.get_scalar(0)
    assert scalar is not None and str(scalar) == col.get(0)

    # Container protocols.
    assert list(col) == [col.get(0), None, col.get(2)]
    assert col[0] == col.get(0) and col[-1] == col.get(2) and col[1] is None
    with pytest.raises(IndexError):
        col[3]

    assert cls(unit, tz).is_empty() and len(cls(unit, tz)) == 0


@pytest.mark.parametrize("cls, unit, tz, values, name", CASES, ids=IDS)
def test_from_epochs_is_the_inverse_of_get_epoch(cls, unit, tz, values, name):
    col = cls(unit, tz, values)
    epochs = [col.get_epoch(i) for i in range(len(col))]
    assert cls.from_epochs(unit, tz, epochs) == col
    assert cls.from_epochs(unit, tz) == cls(unit, tz)  # empty


@pytest.mark.parametrize("cls, unit, tz, values, name", CASES, ids=IDS)
def test_field_and_codec_pickle_copy(cls, unit, tz, values, name):
    col = cls(unit, tz, values)

    field = col.to_field("event")
    assert field.name == "event" and field.type_name == name and field.is_temporal()

    assert cls.deserialize_bytes(col.serialize_bytes()) == col
    assert pickle.loads(pickle.dumps(col)) == col
    assert copy.deepcopy(col) == col

    dup = col.copy()
    assert dup == col
    dup.push(None)
    assert len(dup) == 4 and len(col) == 3  # copy is independent

    with pytest.raises(TypeError):
        hash(col)  # mutable -> unhashable


@pytest.mark.parametrize("cls, unit, tz, values, name", CASES, ids=IDS)
def test_equality_distinguishes_content_and_unit(cls, unit, tz, values, name):
    assert cls(unit, tz, values) == cls(unit, tz, values)
    assert cls(unit, tz, values) != cls(unit, tz, [values[0], values[2], None])


def test_mutation_push_set_and_guided_errors():
    col = Ts64Serie("s", "UTC", ["2021-01-01T00:00:00Z", None])
    col.push("2021-01-03T00:00:00Z")
    col.set(1, "2021-01-02T00:00:00Z")
    assert len(col) == 3 and col.null_count == 0
    assert col.get(1).startswith("2021-01-02")
    assert col.get_epoch(1) == 1609545600

    col.set(0, None)  # re-introduce a null
    assert col.get(0) is None and col.null_count == 1

    with pytest.raises(ValueError):
        col.set(99, "2021-01-01T00:00:00Z")  # out of range
    with pytest.raises(ValueError):
        col.push("not-a-timestamp")  # unparseable


def test_construction_errors_are_guided():
    with pytest.raises(ValueError):
        Ts64Serie("not-a-unit", "UTC", [])
    with pytest.raises(ValueError):
        Ts64Serie("s", "Not/AZone", [])
    with pytest.raises(ValueError):
        # An epoch far past i32 seconds cannot fit a ts32 column.
        Ts32Serie.from_epochs("s", "UTC", [10**18])


@pytest.mark.parametrize("cls, unit, tz, values, name", CASES, ids=IDS)
def test_arrow_c_data_interface_self_round_trip(cls, unit, tz, values, name):
    # Every column exports + re-imports itself through the Arrow C Data Interface with no pyarrow
    # installed — this also proves the ts96 FixedSizeBinary(12) form recovers its (unit, tz) from
    # the field metadata.
    col = cls(unit, tz, values)
    restored = cls.from_arrow(col)
    assert restored == col
    assert restored.unit == col.unit and restored.timezone == col.timezone

    cap = col.__arrow_c_schema__()
    assert type(cap).__name__ == "PyCapsule"


def test_pyarrow_timestamp_round_trip():
    pa = pytest.importorskip("pyarrow")

    # A naive timestamp materializes to naive datetimes everywhere (no tz database needed).
    naive = Ts64Serie("us", "naive", ["2021-01-01T00:00:00", None, "2021-01-02T00:00:00"])
    arr = pa.array(naive)
    assert arr.type == pa.timestamp("us")
    assert arr.to_pylist() == [dt.datetime(2021, 1, 1), None, dt.datetime(2021, 1, 2)]
    assert Ts64Serie.from_arrow(arr) == naive

    # A zoned column exports as timestamp('us', tz='UTC'); values cross via the raw i64 counts.
    utc = Ts64Serie("us", "UTC", ["2021-01-01T00:00:00Z", None, "2021-01-02T00:00:00Z"])
    uarr = pa.array(utc)
    assert uarr.type == pa.timestamp("us", tz="UTC")
    assert uarr.cast(pa.int64()).to_pylist() == [utc.get_epoch(i) for i in range(len(utc))]
    assert Ts64Serie.from_arrow(uarr) == utc


def test_pyarrow_date32_round_trip():
    pa = pytest.importorskip("pyarrow")

    col = Date32Serie("d", "naive", ["2021-01-01", None, "2021-03-01"])
    arr = pa.array(col)
    assert arr.type == pa.date32()
    assert arr.to_pylist() == [dt.date(2021, 1, 1), None, dt.date(2021, 3, 1)]
    assert Date32Serie.from_arrow(arr) == col
