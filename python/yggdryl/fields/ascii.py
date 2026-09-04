"""Fixed-width ASCII field factories, by width and by registered code.

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
    Ascii16Field: TypeAlias = TypedField[Literal["ascii16"], str]
    Ascii24Field: TypeAlias = TypedField[Literal["ascii24"], str]
    Ascii32Field: TypeAlias = TypedField[Literal["ascii32"], str]
    Ascii64Field: TypeAlias = TypedField[Literal["ascii64"], str]
    Ascii96Field: TypeAlias = TypedField[Literal["ascii96"], str]
    Ascii128Field: TypeAlias = TypedField[Literal["ascii128"], str]
    CountryField: TypeAlias = TypedField[Literal["country"], str]
    CurrencyField: TypeAlias = TypedField[Literal["currency"], str]
    MicField: TypeAlias = TypedField[Literal["mic"], str]
    CfiField: TypeAlias = TypedField[Literal["cfi"], str]
    AsciiField: TypeAlias = (
        Ascii16Field
        | Ascii24Field
        | Ascii32Field
        | Ascii64Field
        | Ascii96Field
        | Ascii128Field
    )
else:
    Ascii16Field = Ascii24Field = Ascii32Field = Ascii64Field = Ascii96Field = (
        Ascii128Field
    ) = CountryField = CurrencyField = MicField = CfiField = AsciiField = Field

_ASCII16 = simple_dtype("ascii16")
_ASCII24 = simple_dtype("ascii24")
_ASCII32 = simple_dtype("ascii32")
_ASCII64 = simple_dtype("ascii64")
_ASCII96 = simple_dtype("ascii96")
_ASCII128 = simple_dtype("ascii128")
_COUNTRY = simple_dtype("country")
_CURRENCY = simple_dtype("currency")
_MIC = simple_dtype("mic")
_CFI = simple_dtype("cfi")


def ascii16(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Ascii16Field:
    return new_field(Ascii16Field, name, _ASCII16, nullable, metadata)


def ascii24(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Ascii24Field:
    return new_field(Ascii24Field, name, _ASCII24, nullable, metadata)


def ascii32(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Ascii32Field:
    return new_field(Ascii32Field, name, _ASCII32, nullable, metadata)


def ascii64(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Ascii64Field:
    return new_field(Ascii64Field, name, _ASCII64, nullable, metadata)


def ascii96(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Ascii96Field:
    return new_field(Ascii96Field, name, _ASCII96, nullable, metadata)


def ascii128(
    name: str, *, nullable: bool = True, metadata: MetadataInput = None
) -> Ascii128Field:
    return new_field(Ascii128Field, name, _ASCII128, nullable, metadata)


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


def ascii(
    name: str,
    width: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> AsciiField:
    """Select the ASCII width holding ``width`` bytes through ``DataType.ascii``."""

    return cast(
        AsciiField,
        new_field(Field, name, DataType.ascii(width), nullable, metadata),
    )


__all__ = [
    "Ascii16Field",
    "Ascii24Field",
    "Ascii32Field",
    "Ascii64Field",
    "Ascii96Field",
    "Ascii128Field",
    "AsciiField",
    "CfiField",
    "CountryField",
    "CurrencyField",
    "MicField",
    "ascii",
    "ascii16",
    "ascii24",
    "ascii32",
    "ascii64",
    "ascii96",
    "ascii128",
    "cfi",
    "country",
    "currency",
    "mic",
]
