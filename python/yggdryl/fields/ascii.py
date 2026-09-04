"""The ASCII field factories: variable, fixed-width, and by registered code.

The four registered codes - ``country``, ``currency``, ``mic``, ``cfi`` - are
datatypes of their own, each storing the width its standard fixes, so a code
factory is not a width factory wearing a name: the field it builds carries the
code's identity across Arrow. The declared vocabularies live in
:mod:`yggdryl.enums`, whose classes carry their members onto the field they
build.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias, cast

from .._native import DataType, Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    AsciiField: TypeAlias = TypedField[Literal["ascii"], str]
    FixedAsciiField: TypeAlias = TypedField[Literal["fixed_ascii"], str]
    CountryField: TypeAlias = TypedField[Literal["country"], str]
    CurrencyField: TypeAlias = TypedField[Literal["currency"], str]
    MicField: TypeAlias = TypedField[Literal["mic"], str]
    CfiField: TypeAlias = TypedField[Literal["cfi"], str]
else:
    AsciiField = FixedAsciiField = CountryField = CurrencyField = MicField = (
        CfiField
    ) = Field

_ASCII = simple_dtype("ascii")
_COUNTRY = simple_dtype("country")
_CURRENCY = simple_dtype("currency")
_MIC = simple_dtype("mic")
_CFI = simple_dtype("cfi")


def ascii(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> AsciiField:
    """Variable-width ASCII text: any length, stored as the bytes it is."""

    return new_field(AsciiField, name, _ASCII, nullable, metadata)


def fixed_ascii(
    name: str,
    width: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> FixedAsciiField:
    """ASCII text padded with trailing NUL to exactly ``width`` bytes.

    Raises:
        ValueError: when ``width`` is not at least one byte.
    """

    return cast(
        FixedAsciiField,
        new_field(Field, name, DataType.ascii(width), nullable, metadata),
    )


def country(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CountryField:
    """ISO 3166-1 alpha-2, the two-letter country code."""

    return new_field(CountryField, name, _COUNTRY, nullable, metadata)


def currency(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CurrencyField:
    """ISO 4217, the three-letter currency code."""

    return new_field(CurrencyField, name, _CURRENCY, nullable, metadata)


def mic(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> MicField:
    """ISO 10383, the four-character market identifier code."""

    return new_field(MicField, name, _MIC, nullable, metadata)


def cfi(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CfiField:
    """ISO 10962, the six-character instrument classification."""

    return new_field(CfiField, name, _CFI, nullable, metadata)


__all__ = [
    "AsciiField",
    "CfiField",
    "CountryField",
    "CurrencyField",
    "FixedAsciiField",
    "MicField",
    "ascii",
    "cfi",
    "country",
    "currency",
    "fixed_ascii",
    "mic",
]
