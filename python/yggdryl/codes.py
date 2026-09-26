"""The registered code field factories: fourteen identities, one width each.

The registered codes - ``country``, ``ccy``, ``mic``, ``cfi``, the six
securities identifiers ``isin``, ``cusip``, ``sedol``, ``bbg``, ``ric`` and
``figi``, FIX's own ``side``, ``state`` and ``timeinforce``, and ``unit`` - are
datatypes of their own, each storing as the ASCII text it is and held to the
width its standard fixes, so a code factory is not a bounded string wearing a
name: the field it builds carries the code's identity across Arrow under its
own extension name, answers ``is_code`` and ``code_width``, and never
``string_parameters``. The width bounds a value rather than laying it out, so
``fixed_byte_width`` is ``None``. The declared vocabularies live in
:mod:`yggdryl.enums`, whose classes carry their members onto the field they
build; ``isin``, ``cusip`` and ``sedol`` are open identifier spaces closed by
their own check digits and ``bbg`` and ``ric`` are ones no standard closes at
all, so no class declares them.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from ._native import Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    CountryField: TypeAlias = TypedField[Literal["country"], str]
    CcyField: TypeAlias = TypedField[Literal["ccy"], str]
    MicField: TypeAlias = TypedField[Literal["mic"], str]
    CfiField: TypeAlias = TypedField[Literal["cfi"], str]
    IsinField: TypeAlias = TypedField[Literal["isin"], str]
    CusipField: TypeAlias = TypedField[Literal["cusip"], str]
    SedolField: TypeAlias = TypedField[Literal["sedol"], str]
    BbgField: TypeAlias = TypedField[Literal["bbg"], str]
    RicField: TypeAlias = TypedField[Literal["ric"], str]
    FigiField: TypeAlias = TypedField[Literal["figi"], str]
    SideField: TypeAlias = TypedField[Literal["side"], str]
    StateField: TypeAlias = TypedField[Literal["state"], str]
    TimeInForceField: TypeAlias = TypedField[Literal["timeinforce"], str]
    UnitField: TypeAlias = TypedField[Literal["unit"], str]
else:
    CountryField = CcyField = MicField = CfiField = IsinField = CusipField = SedolField = (
        BbgField
    ) = RicField = FigiField = SideField = StateField = TimeInForceField = UnitField = Field

_COUNTRY = simple_dtype("country")
_CCY = simple_dtype("ccy")
_MIC = simple_dtype("mic")
_CFI = simple_dtype("cfi")
_ISIN = simple_dtype("isin")
_CUSIP = simple_dtype("cusip")
_SEDOL = simple_dtype("sedol")
_BBG = simple_dtype("bbg")
_RIC = simple_dtype("ric")
_FIGI = simple_dtype("figi")
_SIDE = simple_dtype("side")
_STATE = simple_dtype("state")
_TIMEINFORCE = simple_dtype("timeinforce")
_UNIT = simple_dtype("unit")


def country(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CountryField:
    """ISO 3166-1 alpha-2, the two-letter country code."""

    return new_field(CountryField, name, _COUNTRY, nullable, metadata)


def ccy(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CcyField:
    """ISO 4217, the three-letter currency code."""

    return new_field(CcyField, name, _CCY, nullable, metadata)


def mic(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> MicField:
    """ISO 10383, the four-character market identifier code."""

    return new_field(MicField, name, _MIC, nullable, metadata)


def cfi(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CfiField:
    """ISO 10962, the six-character instrument classification."""

    return new_field(CfiField, name, _CFI, nullable, metadata)


def isin(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> IsinField:
    """ISO 6166, the twelve-character securities identifier closed by its check digit."""

    return new_field(IsinField, name, _ISIN, nullable, metadata)


def cusip(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CusipField:
    """CUSIP, the nine-character securities identifier closed by its check digit."""

    return new_field(CusipField, name, _CUSIP, nullable, metadata)


def sedol(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> SedolField:
    """SEDOL, the seven-character securities identifier closed by its check digit."""

    return new_field(SedolField, name, _SEDOL, nullable, metadata)


def bbg(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> BbgField:
    """A Bloomberg identifier: a ticker, a market and a yellow key.

    Its width is only a bound - thirty-two bytes - because no standard fixes a
    length between those parts.
    """

    return new_field(BbgField, name, _BBG, nullable, metadata)


def ric(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> RicField:
    """A Refinitiv Identification Code: a ticker and an exchange mnemonic.

    Thirty-two bytes at most, one token of printable ASCII with no space, its
    case part of the code, so ``ESc1`` is not ``ESC1``.
    """

    return new_field(RicField, name, _RIC, nullable, metadata)


def figi(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> FigiField:
    """ANSI X9.145's twelve-character Financial Instrument Global Identifier."""

    return new_field(FigiField, name, _FIGI, nullable, metadata)


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


def unit(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> UnitField:
    """The unit a quantity is counted in, FIX ``UnitOfMeasure(996)``: ASCII up to 32 bytes."""

    return new_field(UnitField, name, _UNIT, nullable, metadata)


__all__ = [
    "BbgField",
    "CfiField",
    "FigiField",
    "CountryField",
    "CcyField",
    "CusipField",
    "IsinField",
    "MicField",
    "RicField",
    "SedolField",
    "SideField",
    "StateField",
    "TimeInForceField",
    "UnitField",
    "bbg",
    "cfi",
    "country",
    "ccy",
    "figi",
    "cusip",
    "isin",
    "mic",
    "ric",
    "sedol",
    "side",
    "state",
    "timeinforce",
    "unit",
]
