"""Tests for the yggdryl schema + temporal types (DataType, Field, Date, Time,
DateTime, Duration, Timezone).

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import pickle

import pytest

import yggdryl


# ---- DataType ----

def test_datatype_constructors_ids_and_categories():
    D = yggdryl.DataType
    assert D.int32().type_id == 4
    assert D.int32().name == "int32"
    assert D.int32().category == "primitive"
    assert D.int32().is_primitive()
    # the id, name and category line up across the registry.
    assert D.boolean().name == "bool"
    assert D.uint64().type_id == 9
    assert D.decimal(10, 2).category == "logical"
    assert D.decimal(10, 2).is_logical()
    assert D.decimal(10, 2).decimal_parts == (10, 2)
    assert D.utf8().decimal_parts is None
    assert D.struct_([]).category == "nested"
    assert D.struct_([]).is_nested()
    # the decimal scale defaults to 0.
    assert D.decimal(10) == D.decimal(10, 0)


def test_datatype_temporal_and_nested_children():
    D = yggdryl.DataType
    ts = D.timestamp("us", "UTC")
    assert ts.name == "timestamp"
    assert ts.category == "logical"
    assert D.interval("month_day_nano").name == "interval"
    with pytest.raises(ValueError):
        D.interval("nope")
    # nested types expose their child fields; scalars/logicals have none.
    s = D.struct_([
        yggdryl.Field("a", D.int32()),
        yggdryl.Field("b", D.utf8()),
    ])
    assert s.is_nested()
    assert [f.name for f in s.fields()] == ["a", "b"]
    assert D.int32().fields() == []
    assert D.list(yggdryl.Field("item", D.int32())).fields()[0].name == "item"


def test_datatype_eq_hash_and_str():
    D = yggdryl.DataType
    assert D.int64() == D.int64()
    assert D.int64() != D.int32()
    assert hash(D.int64()) == hash(D.int64())
    assert str(D.int32()) == "int32"
    assert repr(D.float64()) == "DataType.float64"
    # usable as a set/dict key.
    assert {D.int32(), D.int32(), D.utf8()} == {D.int32(), D.utf8()}


# ---- Field ----

def test_field_surface_and_in_place_mutation():
    f = yggdryl.Field("id", yggdryl.DataType.int64())
    assert f.name == "id"
    assert f.dtype == yggdryl.DataType.int64()
    assert f.metadata is None
    # name / dtype are mutable in place.
    f.name = "ident"
    f.dtype = yggdryl.DataType.int32()
    assert f.name == "ident"
    assert f.dtype == yggdryl.DataType.int32()


def test_field_reserved_metadata_accessors():
    f = yggdryl.Field("x", yggdryl.DataType.int32())
    assert f.comment is None
    assert f.index_name is None
    assert f.index_level is None
    # setters mutate the metadata map in place.
    f.comment = "a note"
    f.index_name = "idx"
    f.index_level = 7
    assert f.comment == "a note"
    assert f.index_name == "idx"
    assert f.index_level == 7
    # stored under the reserved byte keys.
    assert f.metadata[b"comment"] == b"a note"
    assert f.metadata[b"index_level"] == b"7"
    # clearing a key with None removes it, leaving the others untouched.
    f.comment = None
    f.index_level = None
    assert f.comment is None
    assert f.index_level is None
    assert f.index_name == "idx"


def test_field_metadata_replace_eq_and_hash():
    f = yggdryl.Field("id", yggdryl.DataType.int64())
    f.metadata = {b"unit": b"count"}
    assert f.metadata[b"unit"] == b"count"
    # an empty map clears back to None.
    f.metadata = {}
    assert f.metadata is None
    # equality + hashing cover name, dtype and metadata.
    a = yggdryl.Field("id", yggdryl.DataType.int64())
    b = yggdryl.Field("id", yggdryl.DataType.int64())
    assert a == b
    assert hash(a) == hash(b)
    b.comment = "x"
    assert a != b


# ---- temporal ----

def test_date():
    d = yggdryl.Date(2024, 2, 29)
    assert (d.year, d.month, d.day) == (2024, 2, 29)
    assert str(d) == "2024-02-29"
    assert d.weekday == 4  # Thursday
    assert yggdryl.Date.from_str("2024-02-29") == d
    assert d.add_days(1) == yggdryl.Date(2024, 3, 1)
    assert yggdryl.Date(2024, 1, 1) < yggdryl.Date(2024, 2, 1)
    with pytest.raises(ValueError):
        yggdryl.Date(2023, 2, 29)
    assert pickle.loads(pickle.dumps(d)) == d


def test_time():
    t = yggdryl.Time(13, 45, 30, 250_000_000)
    assert (t.hour, t.minute, t.second, t.nanosecond) == (13, 45, 30, 250_000_000)
    assert str(t) == "13:45:30.250"
    assert yggdryl.Time.from_str("13:45:30.250") == t
    assert yggdryl.Time(0, 0, 0) < yggdryl.Time(0, 0, 1)
    assert pickle.loads(pickle.dumps(t)) == t


def test_duration():
    d = yggdryl.Duration.from_str("1h30m")
    assert d.as_seconds() == 5400
    assert str(d) == "1h30m"
    assert (d + yggdryl.Duration.from_secs(30)).as_seconds() == 5430
    assert yggdryl.Duration.from_unit(500, "ms").as_nanos() == 500_000_000
    assert yggdryl.Duration.from_secs(-5).is_negative
    assert yggdryl.Duration.from_secs(2).as_millis() == 2_000
    assert yggdryl.Duration.from_micros(1_500).as_micros() == 1_500
    assert pickle.loads(pickle.dumps(d)) == d


def test_timezone():
    assert yggdryl.Timezone("UTC").is_utc
    assert yggdryl.Timezone("+05:30").offset_seconds(0) == 19800
    ny = yggdryl.Timezone("America/New_York")
    # January = EST (-5h), July = EDT (-4h).
    assert ny.offset_seconds(1_704_067_200) == -5 * 3600
    assert ny.offset_seconds(1_719_792_000) == -4 * 3600
    with pytest.raises(ValueError):
        yggdryl.Timezone("Mars/Olympus")
    assert pickle.loads(pickle.dumps(ny)) == ny


def test_temporal_math_empty_and_float():
    # Empty string decodes to the zero default.
    assert str(yggdryl.Date.from_str("")) == "1970-01-01"
    assert yggdryl.DateTime.from_str("").epoch_seconds == 0
    assert yggdryl.Duration.from_str("").as_seconds() == 0
    # Duration scale + operators.
    assert yggdryl.Duration.from_secs(5).mul(3).as_seconds() == 15
    assert (yggdryl.Duration.from_secs(5) * 4).as_seconds() == 20
    assert (yggdryl.Duration.from_secs(20) / 5).as_seconds() == 4
    assert (-yggdryl.Duration.from_secs(5)).as_seconds() == -5
    # DateTime arithmetic + diff + truncate.
    dt = yggdryl.DateTime.from_str("2024-07-01T12:00:00Z")
    later = dt + yggdryl.Duration.from_str("1h30m")
    assert str(later) == "2024-07-01T13:30:00Z"
    assert later.duration_since(dt).as_seconds() == 5_400
    assert str(dt.add(yggdryl.Duration.from_str("25m")).truncate(yggdryl.Duration.from_str("1h"))) == "2024-07-01T12:00:00Z"
    # Time wraps around midnight; Date adds whole days.
    assert str(yggdryl.Time(23, 30, 0) + yggdryl.Duration.from_str("1h")) == "00:30:00"
    assert str(yggdryl.Date(2024, 7, 1) + yggdryl.Duration.from_str("2d")) == "2024-07-03"
    # Temporal.from_datetime redirect.
    assert yggdryl.Date.from_datetime(dt) == yggdryl.Date(2024, 7, 1)
    assert yggdryl.Time.from_datetime(dt) == yggdryl.Time(12, 0, 0)


def test_temporal_conversions_and_parse():
    d = yggdryl.Date(2024, 7, 1)
    assert d.to_datetime().hour == 0
    # Date anchored to a zone, combined with a time.
    ny = d.with_timezone("America/New_York")
    assert ny.timezone.name == "America/New_York"
    assert ny.at(yggdryl.Time(8, 0, 0)).epoch_seconds == 1_719_835_200
    assert yggdryl.Time(13, 30, 0).to_datetime().hour == 13
    # from_str is the single, strict parser (raises on malformed input; no `parse`).
    assert str(yggdryl.DateTime.from_str("2024-07-01")) == "2024-07-01T00:00:00"
    assert yggdryl.DateTime.from_str("1719835200").epoch_seconds == 1_719_835_200
    with pytest.raises(ValueError):
        yggdryl.Date.from_str("not-a-date")
    # Duration ISO-8601.
    assert yggdryl.Duration.from_str("PT15M").as_seconds() == 900
    assert yggdryl.Duration.from_str("P1D").as_seconds() == 86_400
    # Date pickle keeps the timezone.
    assert pickle.loads(pickle.dumps(ny)).timezone.name == "America/New_York"


def test_datetime_dst_conversion():
    utc = yggdryl.DateTime.from_str("2024-07-01T12:00:00Z")
    assert utc.epoch_seconds == 1_719_835_200
    # Same instant displayed in other zones (DST-aware).
    ny = utc.to_timezone("America/New_York")
    assert (ny.hour, ny.minute) == (8, 0)
    assert str(ny) == "2024-07-01T08:00:00-04:00"
    tokyo = utc.to_timezone("Asia/Tokyo")
    assert tokyo.hour == 21
    assert ny.epoch_seconds == utc.epoch_seconds
    # localize a wall-clock time in a zone.
    local = yggdryl.DateTime(2024, 7, 1, 8, 0, 0, 0, "America/New_York")
    assert local.epoch_seconds == 1_719_835_200
    # naive datetime.
    naive = yggdryl.DateTime.from_str("2024-07-01T12:00:00")
    assert naive.timezone is None
    assert pickle.loads(pickle.dumps(utc)) == utc
