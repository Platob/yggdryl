"""The registered code field factories: nine identities, one width each.

The registered codes - ``country``, ``currency``, ``mic``, ``cfi``, ``isin``,
and FIX's own ``side``, ``msgdirection``, ``state`` and ``timeinforce`` - are
datatypes of their own, each storing the width its standard fixes, so a code
factory is not a fixed-width string wearing a name: the field it builds
carries the code's identity across Arrow, answers ``is_code`` and
``fixed_byte_width``, and never ``string_parameters``. The declared
vocabularies live in :mod:`yggdryl.enums`, whose classes carry their members
onto the field they build; ``isin`` is an open identifier space closed by its
own check digit, so no class declares it.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    CountryField: TypeAlias = TypedField[Literal["country"], str]
    CurrencyField: TypeAlias = TypedField[Literal["currency"], str]
    MicField: TypeAlias = TypedField[Literal["mic"], str]
    CfiField: TypeAlias = TypedField[Literal["cfi"], str]
    IsinField: TypeAlias = TypedField[Literal["isin"], str]
    SideField: TypeAlias = TypedField[Literal["side"], str]
    MsgDirectionField: TypeAlias = TypedField[Literal["msgdirection"], str]
    StateField: TypeAlias = TypedField[Literal["state"], str]
    TimeInForceField: TypeAlias = TypedField[Literal["timeinforce"], str]
else:
    CountryField = CurrencyField = MicField = CfiField = IsinField = SideField = (
        MsgDirectionField
    ) = StateField = TimeInForceField = Field

_COUNTRY = simple_dtype("country")
_CURRENCY = simple_dtype("currency")
_MIC = simple_dtype("mic")
_CFI = simple_dtype("cfi")
_ISIN = simple_dtype("isin")
_SIDE = simple_dtype("side")
_DIRECTION = simple_dtype("msgdirection")
_STATE = simple_dtype("state")
_TIMEINFORCE = simple_dtype("timeinforce")


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


def isin(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> IsinField:
    """ISO 6166, the twelve-character securities identifier closed by its check digit."""

    return new_field(IsinField, name, _ISIN, nullable, metadata)


def side(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> SideField:
    """FIX ``Side(54)``, the wire value rather than a name for it."""

    return new_field(SideField, name, _SIDE, nullable, metadata)


def msgdirection(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> MsgDirectionField:
    """Which way a captured line moved, as the packed four bytes."""

    return new_field(MsgDirectionField, name, _DIRECTION, nullable, metadata)


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
    "CfiField",
    "CountryField",
    "CurrencyField",
    "IsinField",
    "MicField",
    "MsgDirectionField",
    "SideField",
    "StateField",
    "TimeInForceField",
    "cfi",
    "country",
    "currency",
    "isin",
    "mic",
    "msgdirection",
    "side",
    "state",
    "timeinforce",
]
