"""Tests for the yggdryl schema + temporal types (DataType, Field, Date, Time,
DateTime, Duration, Timezone).

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import copy
import pickle

import pytest

import yggdryl


# ---- DataType ----

def test_datatype_parse_and_constructors():
    assert yggdryl.DataType("int64") == yggdryl.DataType.int(64)
    assert yggdryl.DataType.int(8, signed=False) == yggdryl.DataType("uint8")
    assert yggdryl.DataType("string") == yggdryl.DataType.varchar()
    assert str(yggdryl.DataType.float(64)) == "float64"
    assert str(yggdryl.DataType.decimal(10, 2)) == "decimal128[10, 2]"
    assert str(yggdryl.DataType.timestamp("us", "UTC")) == "timestamp[us, UTC]"


def test_datatype_accessors_and_categories():
    assert yggdryl.DataType.int(32).category == "primitive"
    assert yggdryl.DataType.date().category == "logical"
    assert yggdryl.DataType.struct_([]).category == "nested"
    assert yggdryl.DataType.any().category == "any"
    assert yggdryl.DataType.int(32).bit_size == 32
    assert yggdryl.DataType.boolean().bit_size == 1
    assert yggdryl.DataType.varchar().bit_size is None
    assert yggdryl.DataType.varchar(large=True).is_large
    assert yggdryl.DataType.varchar(view=True).is_view
    assert yggdryl.DataType.varchar(charset="latin1").charset == "latin1"
    assert yggdryl.DataType.timestamp("ns", "Asia/Tokyo").time_unit == "ns"
    assert yggdryl.DataType.timestamp("ns", "Asia/Tokyo").timezone.name == "Asia/Tokyo"
    assert yggdryl.DataType.decimal(10, 2).decimal_parts == (10, 2)


def test_datatype_predicates_and_children():
    assert yggdryl.DataType.int(32).is_signed_integer()
    assert yggdryl.DataType.int(32, signed=False).is_unsigned_integer()
    assert yggdryl.DataType.float(32).is_numeric()
    assert not yggdryl.DataType.decimal(1, 0).is_numeric()
    assert yggdryl.DataType.varchar().is_string()
    assert yggdryl.DataType.timestamp("s").is_temporal()
    s = yggdryl.DataType.struct_([
        yggdryl.Field("a", yggdryl.DataType.int(32)),
        yggdryl.Field("b", yggdryl.DataType.varchar()),
    ])
    assert s.is_struct()
    assert [f.name for f in s.children()] == ["a", "b"]


def test_datatype_coercion_and_merge():
    D = yggdryl.DataType
    assert D.int(8).common_type(D.int(32)) == D.int(32)
    assert D.int(32).common_type(D.float(32)) == D.float(64)
    assert D.int(32).common_type(D.varchar()) is None
    assert D.int(32).can_cast_to(D.varchar())
    assert not D.int(32).can_cast_to(D.binary())
    assert D.int(8).merge(D.int(64), "promote") == D.int(64)
    with pytest.raises(ValueError):
        D.int(8).merge(D.int(64), "strict")
    assert D.int(8).merge(D.varchar(), "permissive") == D.any()


def test_datatype_serialisation_roundtrips():
    dt = yggdryl.DataType.struct_([
        yggdryl.Field("id", yggdryl.DataType.int(64), nullable=False),
        yggdryl.Field("name", yggdryl.DataType.varchar()),
    ])
    assert yggdryl.DataType.from_json(dt.to_json()) == dt
    assert yggdryl.DataType.from_mapping(dt.to_mapping()) == dt
    assert yggdryl.DataType.from_str(str(dt)) == dt
    assert bytes(dt) == str(dt).encode()
    # pickle / copy go through __reduce__ (lossless structural JSON).
    assert pickle.loads(pickle.dumps(dt)) == dt
    assert copy.deepcopy(dt) == dt
    assert hash(dt) == hash(yggdryl.DataType.from_str(str(dt)))


# ---- Field ----

def test_field_surface_and_metadata():
    f = yggdryl.Field("id", yggdryl.DataType.int(64), nullable=False).with_comment("pk")
    assert f.name == "id"
    assert not f.nullable
    assert f.data_type == yggdryl.DataType.int(64)
    assert f.comment == "pk"
    assert str(f) == "id: int64 not null"
    m = yggdryl.Field("id", yggdryl.DataType.int(64))
    m.set_metadata("unit", "count")
    assert m.get_metadata("unit") == "count"
    assert m.metadata()["unit"] == "count"
    assert m.remove_metadata("unit") == "count"
    # mapping + json + pickle round-trips (metadata preserved).
    assert yggdryl.Field.from_mapping(f.to_mapping()) == f
    assert yggdryl.Field.from_json(f.to_json()) == f
    assert pickle.loads(pickle.dumps(f)) == f


def test_field_graph():
    schema = yggdryl.Field("rec", yggdryl.DataType.struct_([
        yggdryl.Field("Id", yggdryl.DataType.int(64), nullable=False),
        yggdryl.Field("Name", yggdryl.DataType.varchar()),
        yggdryl.Field("addr", yggdryl.DataType.struct_([
            yggdryl.Field("City", yggdryl.DataType.varchar()),
        ])),
    ]), nullable=False)
    assert schema.child_count == 3
    assert schema.child("id").name == "Id"        # case-insensitive
    assert schema.child("NAME").name == "Name"
    assert schema.child_exact("id") is None        # case-sensitive
    assert schema.child_index("name") == 1
    assert schema.child_at(2).name == "addr"
    # parent links after wiring.
    linked = schema.with_linked_children()
    addr = linked.child("addr")
    assert addr.parent.name == "rec"
    city = addr.child("city")
    assert city.parent.name == "addr"
    assert city.root().name == "rec"
    # identity ignores parent.
    assert linked == schema


def test_field_merge():
    a = yggdryl.Field("x", yggdryl.DataType.int(8), nullable=False)
    b = yggdryl.Field("x", yggdryl.DataType.int(32))
    merged = a.merge(b, "promote")
    assert merged.data_type == yggdryl.DataType.int(32)
    assert merged.nullable
    with pytest.raises(ValueError):
        a.merge(yggdryl.Field("y", yggdryl.DataType.int(8)), "promote")


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
