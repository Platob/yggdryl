"""Tests for the ``yggdryl.temporal`` value types (dates, times, timestamps, durations) and the
``Tz`` timezone — mirroring the Rust ``io::fixed::temporal`` suite: calendar math, DST-aware zone
conversions, unit/timezone strings, ISO parsing, value identity, and pickling."""

import copy
import pickle

import pytest

import yggdryl
from yggdryl.temporal import (
    Date32, Date64, Duration32, Duration64, Time32, Time64,
    Ts32, Ts64, Ts96, Tz,
)
from yggdryl.types import DataType


def test_module_surface():
    for cls in (Date32, Date64, Time32, Time64, Ts32, Ts64, Ts96, Duration32, Duration64, Tz):
        assert cls.__module__ == "yggdryl.temporal"
        assert hasattr(yggdryl.temporal, cls.__name__)


def test_date_calendar_and_conversions():
    d = Date32.from_ymd(2024, 2, 29)  # a leap day
    assert d.to_ymd() == (2024, 2, 29)
    assert (d.year, d.month, d.day) == (2024, 2, 29)
    assert d.weekday() == 4 and d.is_leap_year()  # 2024-02-29 is a Thursday
    assert str(d) == "2024-02-29"
    assert Date32.from_string("2024-02-29") == d
    assert Date32.from_days(0).to_ymd() == (1970, 1, 1)
    with pytest.raises(ValueError):
        Date32.from_ymd(2023, 2, 29)  # not a leap year
    # Date32 <-> Date64.
    assert d.to_date64().to_ymd() == (2024, 2, 29)
    assert Date64.from_ymd(2024, 2, 29).to_date32() == d


def test_time_components_and_units():
    t = Time32.from_hms(13, 45, 30)
    assert t.to_hms() == (13, 45, 30, 0)
    assert str(t) == "13:45:30" and t.unit == "s"
    assert t.to_unit("ms").value == (13 * 3600 + 45 * 60 + 30) * 1000
    ns = Time64.from_hms_nano(1, 2, 3, 456_000_000)
    assert str(ns) == "01:02:03.456000000"
    assert Time64.from_string("01:02:03.456").to_hms() == (1, 2, 3, 456_000_000)


def test_timezone_dst():
    paris = Tz.iana("Europe/Paris")
    assert paris.is_iana() and paris.name == "Europe/Paris"
    winter = Ts64.from_datetime(2024, 1, 15, 12, 0, 0, 0, "s", "UTC")
    summer = Ts64.from_datetime(2024, 7, 15, 12, 0, 0, 0, "s", "UTC")
    assert paris.offset_seconds_at(winter.epoch_seconds()) == 3600  # CET
    assert paris.offset_seconds_at(summer.epoch_seconds()) == 7200  # CEST
    assert Tz.parse("+02:00").offset_seconds_at(0) == 7200
    assert Tz.parse("").is_naive()
    with pytest.raises(ValueError):
        Tz.iana("Not/AZone")


def test_timestamp_wall_clock_moves_with_zone():
    # The SAME instant reads differently per zone.
    utc = Ts64.from_datetime(2024, 7, 15, 12, 0, 0, 0, "s", "UTC")
    assert utc.to_datetime() == (2024, 7, 15, 12, 0, 0, 0)
    paris = utc.with_timezone("Europe/Paris")
    assert paris.to_datetime() == (2024, 7, 15, 14, 0, 0, 0)  # +2h summer
    assert paris.epoch_value == utc.epoch_value  # same instant
    assert str(paris).endswith("+02:00")
    # Extract + unit conversion.
    assert utc.to_date().to_ymd() == (2024, 7, 15)
    assert utc.to_unit("ms").epoch_value == utc.epoch_value * 1000
    # ISO parse + width conversions.
    assert Ts64.from_string("2024-02-29T13:45:30Z").to_datetime() == (2024, 2, 29, 13, 45, 30, 0)
    far = Ts96.from_datetime(5000, 1, 1, 0, 0, 0, 0, "ns", "UTC")  # beyond i64 ns range
    assert far.year == 5000
    with pytest.raises(ValueError):
        Ts32.from_epoch(10**18, "s", "UTC")


def test_cross_type_converters():
    # Every temporal type converts to any other — date<->timestamp, time<->duration, etc.
    date = Date32.from_ymd(2024, 2, 29)
    time = Time64.from_hms_nano(13, 45, 30, 0)
    # Date <-> Timestamp (midnight, and at a wall-clock time).
    midnight = date.at_midnight("s", "UTC")
    assert midnight.to_datetime() == (2024, 2, 29, 0, 0, 0, 0)
    assert midnight.to_date() == date
    assert date.at_time(time, "s", "UTC").to_datetime() == (2024, 2, 29, 13, 45, 30, 0)
    # Date <-> Duration (days since epoch).
    assert (date.to_duration().value, date.to_duration().unit) == (date.days, "d")
    assert date.to_duration().to_date() == date
    # Time <-> Duration, and Time -> Timestamp on the epoch date.
    assert time.to_duration().to_time().to_hms() == (13, 45, 30, 0)
    assert time.to_timestamp("s", "UTC").to_datetime() == (1970, 1, 1, 13, 45, 30, 0)
    # Timestamp <-> Duration (elapsed since epoch round-trips the instant).
    assert midnight.to_duration().to_timestamp("UTC").epoch_value == midnight.epoch_value
    # Duration widths.
    assert Duration64.seconds(90).to_duration32().value == 90
    assert Duration32.seconds(90).to_duration64().value == 90


def test_duration_arithmetic():
    total = Duration64.seconds(1) + Duration64.milliseconds(500)
    assert (total.value, total.unit) == (1500, "ms")  # aligns to the finer unit
    assert str(Duration64.seconds(90)) == "90s"
    assert Duration64.from_string("-1500ms").value == -1500
    assert Duration64.seconds(1) > Duration64.milliseconds(500)  # by elapsed span
    assert (-Duration64.seconds(5)).value == -5
    with pytest.raises(ValueError):
        Duration32.new(1, "year")  # calendar unit unsupported


def test_value_identity_and_pickle():
    for value in [
        Date32.from_ymd(2024, 2, 29),
        Time64.from_hms_nano(1, 2, 3, 4),
        Ts64.from_datetime(2024, 2, 29, 13, 45, 30, 0, "s", "Europe/Paris"),
        Duration64.milliseconds(1234),
    ]:
        assert value == copy.copy(value) == copy.deepcopy(value) == value.copy()
        assert value == type(value).deserialize_bytes(value.serialize_bytes())
        assert pickle.loads(pickle.dumps(value)) == value
        assert hash(value) == hash(copy.copy(value))
    # Usable as dict/set keys.
    assert len({Date32.from_ymd(2024, 1, 1), Date32.from_ymd(2024, 1, 1)}) == 1


def test_generic_parse_factories_and_flexible_formats():
    from yggdryl.temporal import date, time, timestamp, duration

    # Flexible date formats all reach the same date.
    for text in ["2024-02-29", "02/29/2024", "29.02.2024", "Feb 29, 2024"]:
        assert date(text).to_ymd() == (2024, 2, 29), text
    assert time("1:45 PM").to_hms() == (13, 45, 0, 0)  # 12-hour
    # Timestamp defaults unit/tz and casts while parsing.
    ts = timestamp("2024-02-29 13:45:30", unit="ms", tz="UTC")
    assert ts.to_datetime() == (2024, 2, 29, 13, 45, 30, 0) and ts.unit == "ms"
    assert timestamp("2024-07-15T12:00:00-05:00").offset_seconds() == -5 * 3600  # zone in string
    # Flexible durations: single-unit, compound, clock, and ISO-8601 — natural granularity.
    assert duration("90s").value == 90
    assert (duration("1h30m").value, duration("1h30m").unit) == (90, "min")
    assert (duration("1:30:00").value, duration("1:30:00").unit) == (90, "min")
    assert (duration("PT1H30M").value, duration("PT1H30M").unit) == (90, "min")
    assert duration("-1500ms").value == -1500
    assert (duration("1h30m", unit="s").value, duration("1h30m", unit="s").unit) == (5400, "s")


def test_native_datetime_interop():
    import datetime

    # Timestamp <-> datetime.datetime (round-trips with a fixed-offset tzinfo).
    ts = Ts64.from_datetime(2024, 2, 29, 13, 45, 30, 0, "us", "UTC")
    native = ts.to_pydatetime()
    assert isinstance(native, datetime.datetime)
    assert Ts64.from_pydatetime(native).to_datetime() == (2024, 2, 29, 13, 45, 30, 0)
    # Date <-> datetime.date, Time <-> datetime.time, Duration <-> timedelta.
    assert Date32.from_ymd(2024, 2, 29).to_pydate() == datetime.date(2024, 2, 29)
    assert Date32.from_pydate(datetime.date(2024, 2, 29)).to_ymd() == (2024, 2, 29)
    assert Time64.from_pytime(datetime.time(1, 2, 3, 456)).to_hms() == (1, 2, 3, 456_000)
    assert Duration64.milliseconds(1500).to_timedelta() == datetime.timedelta(seconds=1.5)
    assert Duration64.from_timedelta(datetime.timedelta(minutes=2)).to_nanos() == 120 * 10**9


def test_signature_repr():
    ts = Ts64.from_datetime(2024, 2, 29, 13, 45, 30, 0, "s", "UTC")
    assert repr(ts) == "ts64[s, UTC](2024-02-29T13:45:30Z)"  # inner params + ISO value
    assert repr(Date32.from_ymd(2024, 2, 29)) == "date32(2024-02-29)"
    assert repr(Duration64.milliseconds(1500)) == "duration64[ms](1500ms)"


def test_datatype_knows_temporals():
    for name, width in [("date32", 4), ("time64", 8), ("ts96", 12), ("duration64", 8)]:
        dt = DataType.by_name(name)
        assert (dt.name, dt.byte_width, dt.category) == (name, width, "temporal")
        assert dt.is_temporal() and not dt.is_numeric()
    assert DataType.ts64().is_temporal()
