"""The registered code field factories: eleven identities, one width each.

The registered codes - ``country``, ``currency``, ``mic``, ``cfi``, the four
securities identifiers ``isin``, ``cusip``, ``sedol`` and ``bloomberg``, and
FIX's own ``side``, ``state`` and ``timeinforce`` - are datatypes of their own, each
storing as the ASCII text it is and held to the width its standard fixes, so
a code factory is not a bounded string wearing a name: the field it builds
carries the code's identity across Arrow under its own extension name,
answers ``is_code`` and ``code_width``, and never ``string_parameters``. The
width bounds a value rather than laying it out, so ``fixed_byte_width`` is
``None``. The declared vocabularies live in :mod:`yggdryl.enums`, whose
classes carry their members onto the field they build; ``isin``, ``cusip``
and ``sedol`` are open identifier spaces closed by their own check digits and
``bloomberg`` is one no standard closes at all, so no class declares them.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    CountryField: TypeAlias = TypedField[Literal["country"], str]
    CurrencyField: TypeAlias = TypedField[Literal["currency"], str]
    MicCodeField: TypeAlias = TypedField[Literal["mic"], str]
    CfiCodeField: TypeAlias = TypedField[Literal["cfi"], str]
    IsinCodeField: TypeAlias = TypedField[Literal["isin"], str]
    CusipCodeField: TypeAlias = TypedField[Literal["cusip"], str]
    SedolCodeField: TypeAlias = TypedField[Literal["sedol"], str]
    BloombergCodeField: TypeAlias = TypedField[Literal["bloomberg"], str]
    SideField: TypeAlias = TypedField[Literal["side"], str]
    StateField: TypeAlias = TypedField[Literal["state"], str]
    TimeInForceField: TypeAlias = TypedField[Literal["timeinforce"], str]
else:
    CountryField = CurrencyField = MicCodeField = CfiCodeField = IsinCodeField = CusipCodeField = (
        SedolCodeField
    ) = BloombergCodeField = SideField = StateField = TimeInForceField = Field

_COUNTRY = simple_dtype("country")
_CURRENCY = simple_dtype("currency")
_MIC = simple_dtype("mic")
_CFI = simple_dtype("cfi")
_ISIN = simple_dtype("isin")
_CUSIP = simple_dtype("cusip")
_SEDOL = simple_dtype("sedol")
_BLOOMBERG = simple_dtype("bloomberg")
_SIDE = simple_dtype("side")
_STATE = simple_dtype("state")
_TIMEINFORCE = simple_dtype("timeinforce")


def country(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CountryField:
    """ISO 3166-1 alpha-2, the two-letter country code."""

    return new_field(CountryField, name, _COUNTRY, nullable, metadata)


def currency(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CurrencyField:
    """ISO 4217, the three-letter currency code."""

    return new_field(CurrencyField, name, _CURRENCY, nullable, metadata)


def mic(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> MicCodeField:
    """ISO 10383, the four-character market identifier code."""

    return new_field(MicCodeField, name, _MIC, nullable, metadata)


def cfi(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CfiCodeField:
    """ISO 10962, the six-character instrument classification."""

    return new_field(CfiCodeField, name, _CFI, nullable, metadata)


def isin(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> IsinCodeField:
    """ISO 6166, the twelve-character securities identifier closed by its check digit."""

    return new_field(IsinCodeField, name, _ISIN, nullable, metadata)


def cusip(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CusipCodeField:
    """CUSIP, the nine-character securities identifier closed by its check digit."""

    return new_field(CusipCodeField, name, _CUSIP, nullable, metadata)


def sedol(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> SedolCodeField:
    """SEDOL, the seven-character securities identifier closed by its check digit."""

    return new_field(SedolCodeField, name, _SEDOL, nullable, metadata)


def bloomberg(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BloombergCodeField:
    """A Bloomberg identifier: a ticker, a market and a yellow key, or a FIGI.

    The one code here whose width is only a bound - thirty-two bytes - because
    no standard fixes a length between those parts.
    """

    return new_field(BloombergCodeField, name, _BLOOMBERG, nullable, metadata)


def side(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> SideField:
    """FIX ``Side(54)``, the wire value rather than a name for it."""

    return new_field(SideField, name, _SIDE, nullable, metadata)


def state(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> StateField:
    """What state one thing is in, ranked so the stored bytes sort by lifecycle."""

    return new_field(StateField, name, _STATE, nullable, metadata)


def timeinforce(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> TimeInForceField:
    """FIX ``TimeInForce(59)``, the wire value rather than a name for it."""

    return new_field(TimeInForceField, name, _TIMEINFORCE, nullable, metadata)


__all__ = [
    "BloombergCodeField",
    "CfiCodeField",
    "CountryField",
    "CurrencyField",
    "CusipCodeField",
    "IsinCodeField",
    "MicCodeField",
    "SedolCodeField",
    "SideField",
    "StateField",
    "TimeInForceField",
    "bloomberg",
    "cfi",
    "country",
    "currency",
    "cusip",
    "isin",
    "mic",
    "sedol",
    "side",
    "state",
    "timeinforce",
]
