from __future__ import annotations

import datetime as dt
import io
from decimal import Decimal

import pytest

from yggdryl import Field, json, scalar, toml, yaml


@scalar
class Trade:
    amount: Decimal
    payload: bytes
    at: dt.datetime


def amount_field() -> Field:
    return Field("amount", "decimal128(8, 2)", nullable=False)


def test_explicit_field_restores_natural_scalars() -> None:
    field = amount_field()

    assert json.loads('"12.50"', field=field) == Decimal("12.50")
    assert yaml.loads('"12.50"\n', field=field) == Decimal("12.50")
    assert list(json.loads_all('"12.50"\n"3.40"\n', field=field)) == [
        Decimal("12.50"),
        Decimal("3.40"),
    ]
    assert list(
        json.load_all(io.BytesIO(b'"12.50"\n"3.40"\n'), field=field)
    ) == [Decimal("12.50"), Decimal("3.40")]


@pytest.mark.parametrize(
    ("codec", "source"),
    [
        (
            json,
            '{"amount":"12.500000000000000000","payload":"AP8=",'
            '"at":"2026-08-15T10:30:00.000000Z"}',
        ),
        (
            yaml,
            "amount: '12.500000000000000000'\npayload: AP8=\n"
            "at: '2026-08-15T10:30:00.000000Z'\n",
        ),
        (
            toml,
            "amount = '12.500000000000000000'\npayload = 'AP8='\n"
            "at = '2026-08-15T10:30:00.000000Z'\n",
        ),
    ],
)
def test_dataclass_target_and_explicit_field_share_one_decode(codec: object, source: str) -> None:
    expected = Trade(
        Decimal("12.500000000000000000"),
        b"\x00\xff",
        dt.datetime(2026, 8, 15, 10, 30, tzinfo=dt.timezone.utc),
    )

    assert codec.loads(source, cls=Trade, field=Trade) == expected  # type: ignore[attr-defined]


def test_placeholders_resolve_before_field_interpretation() -> None:
    value = yaml.loads(
        '"{{ AMOUNT }}"\n',
        field=amount_field(),
        placeholders={"AMOUNT": "12.50"},
    )

    assert value == Decimal("12.50")


@pytest.mark.parametrize("codec", [json, yaml, toml])
def test_dump_returns_bytes_or_utf8_and_still_writes(codec: object) -> None:
    binary = codec.dump({"id": 1})  # type: ignore[attr-defined]
    text = codec.dump({"id": 1}, utf8=True)  # type: ignore[attr-defined]
    destination = io.StringIO()
    written = codec.dump({"id": 1}, destination)  # type: ignore[attr-defined]

    assert isinstance(binary, bytes)
    assert isinstance(text, str)
    assert binary.decode() == text
    assert written is None
    assert destination.getvalue() == text


def test_wrong_field_is_reported_by_the_core() -> None:
    with pytest.raises(ValueError, match="decimal"):
        json.loads('"not-a-decimal"', field=amount_field())
