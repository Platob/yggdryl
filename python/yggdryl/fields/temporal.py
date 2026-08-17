"""Temporal and interval field factories."""

from __future__ import annotations

from datetime import date, datetime, time as TimeValue, timedelta
from typing import TYPE_CHECKING, Literal, TypeAlias, cast

from .._native import Field
from .._native import DataType
from ._common import MetadataInput, new_field, simple_data_type
from ._typing import TypedField

if TYPE_CHECKING:
    TimestampField: TypeAlias = TypedField[Literal["timestamp"], datetime]
    Date32Field: TypeAlias = TypedField[Literal["date32"], date]
    Date64Field: TypeAlias = TypedField[Literal["date64"], date]
    Time32Field: TypeAlias = TypedField[Literal["time32"], TimeValue]
    Time64Field: TypeAlias = TypedField[Literal["time64"], TimeValue]
    TimeField: TypeAlias = Time32Field | Time64Field
    DurationField: TypeAlias = TypedField[Literal["duration"], timedelta]
    IntervalField: TypeAlias = TypedField[Literal["interval"], object]
else:
    TimestampField = Date32Field = Date64Field = Field
    Time32Field = Time64Field = TimeField = DurationField = IntervalField = Field

_DATE32 = simple_data_type("date32")
_DATE64 = simple_data_type("date64")


def timestamp(
    name: str,
    unit: str = "microsecond",
    timezone: str | None = None,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> TimestampField:
    value = DataType._temporal("timestamp", unit, timezone)
    return new_field(TimestampField, name, value, nullable, metadata)


def date32(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Date32Field:
    return new_field(Date32Field, name, _DATE32, nullable, metadata)


def date64(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Date64Field:
    return new_field(Date64Field, name, _DATE64, nullable, metadata)


def time32(
    name: str,
    unit: str = "millisecond",
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> Time32Field:
    return new_field(
        Time32Field, name, DataType._temporal("time32", unit), nullable, metadata
    )


def time64(
    name: str,
    unit: str = "microsecond",
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> Time64Field:
    return new_field(
        Time64Field, name, DataType._temporal("time64", unit), nullable, metadata
    )


def time(
    name: str,
    unit: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> TimeField:
    """Select Time32 or Time64 through ``DataType.time``."""

    return cast(
        TimeField,
        new_field(Field, name, DataType.time(unit), nullable, metadata),
    )


def duration(
    name: str,
    unit: str = "microsecond",
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> DurationField:
    return new_field(
        DurationField, name, DataType._temporal("duration", unit), nullable, metadata
    )


def interval(
    name: str,
    unit: str = "month_day_nano",
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> IntervalField:
    return new_field(
        IntervalField, name, DataType._temporal("interval", unit), nullable, metadata
    )


__all__ = [
    "Date32Field",
    "Date64Field",
    "DurationField",
    "IntervalField",
    "Time32Field",
    "Time64Field",
    "TimeField",
    "TimestampField",
    "date32",
    "date64",
    "duration",
    "interval",
    "time",
    "time32",
    "time64",
    "timestamp",
]
