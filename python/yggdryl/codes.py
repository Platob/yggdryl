"""The registered code field factories: seventeen identities, one width each.

The registered codes - ``country``, ``ccy``, ``mic``, ``cfi``, the six
securities identifiers ``isin``, ``cusip``, ``sedol``, ``bbg``, ``ric`` and
``figi``, ``unit``, the currency pair ``forex``, and the reference-data codes
``lei``, ``bic``, ``elf``, ``dti`` and ``fisn`` - are datatypes of their own,
each storing as the ASCII text it is and held to the width its standard fixes, so a code factory is not a bounded string wearing a
name: the field it builds carries the code's identity across Arrow under its
own extension name, answers ``is_code`` and ``code_width``, and never
``string_parameters``. The width bounds a value rather than laying it out, so
``fixed_byte_width`` is ``None``. The declared vocabularies live in
:mod:`yggdryl.enums`, whose classes carry their members onto the field they
build; ``isin``, ``cusip`` and ``sedol`` are open identifier spaces closed by
their own check digits, as ``lei`` and ``dti`` are, and ``bbg`` and ``ric``
are ones no standard closes at all, so no class declares them.
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
    UnitField: TypeAlias = TypedField[Literal["unit"], str]
    ForexField: TypeAlias = TypedField[Literal["forex"], str]
    LeiField: TypeAlias = TypedField[Literal["lei"], str]
    BicField: TypeAlias = TypedField[Literal["bic"], str]
    ElfField: TypeAlias = TypedField[Literal["elf"], str]
    DtiField: TypeAlias = TypedField[Literal["dti"], str]
    FisnField: TypeAlias = TypedField[Literal["fisn"], str]
else:
    CountryField = CcyField = MicField = CfiField = IsinField = CusipField = SedolField = (
        BbgField
    ) = RicField = FigiField = UnitField = ForexField = LeiField = BicField = ElfField = (
        DtiField
    ) = FisnField = Field

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
_UNIT = simple_dtype("unit")
_FOREX = simple_dtype("forex")
_LEI = simple_dtype("lei")
_BIC = simple_dtype("bic")
_ELF = simple_dtype("elf")
_DTI = simple_dtype("dti")
_FISN = simple_dtype("fisn")


def country(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CountryField:
    """ISO 3166-1 alpha-2, the two-letter country code."""

    return new_field(CountryField, name, _COUNTRY, nullable, metadata)


def ccy(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> CcyField:
    """The currency code: ISO 4217's three letters, or a digital-asset ticker, at most eight bytes."""

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


def unit(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> UnitField:
    """The unit a quantity is counted in, FIX ``UnitOfMeasure(996)``: ASCII up to 32 bytes."""

    return new_field(UnitField, name, _UNIT, nullable, metadata)


def forex(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> ForexField:
    """A currency pair, ``CCY/CCY``: two ISO 4217 codes, the base then the quote.

    Stored as its canonical text, ``EUR/USD``, whichever spelling - ``EURUSD``,
    ``eur-usd`` - a value arrives in. A digital-asset ticker is a ``ccy`` but
    no leg, so ``BTC/USDT`` is no pair.
    """

    return new_field(ForexField, name, _FOREX, nullable, metadata)


def lei(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> LeiField:
    """ISO 17442, the twenty-character legal entity identifier closed by its MOD 97-10 check digits.

    Lower case folds; a value whose digits do not close is kept at rank zero,
    never refused, so a merge replaces it by a closing one.
    """

    return new_field(LeiField, name, _LEI, nullable, metadata)


def bic(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> BicField:
    """ISO 9362, the business identifier code: eight or eleven characters, kept as stated.

    The party prefix, the country, the location and, on eleven, the branch;
    ``DEUTDEFF`` and ``DEUTDEFFXXX`` are two spellings neither folded into the
    other.
    """

    return new_field(BicField, name, _BIC, nullable, metadata)


def elf(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> ElfField:
    """ISO 20275, the four-character entity legal form code, ``2HBR`` a German GmbH."""

    return new_field(ElfField, name, _ELF, nullable, metadata)


def dti(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> DtiField:
    """ISO 24165, the nine-character digital token identifier closed by its MOD 31,30 check character."""

    return new_field(DtiField, name, _DTI, nullable, metadata)


def fisn(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> FisnField:
    """ISO 18774, the financial instrument short name: the issuer, ``/``, the description.

    At most thirty-five US-ASCII bytes, folded to upper case, split at the
    first ``/`` - ``ACME CORP/SH``; a Latin-1 letter is refused.
    """

    return new_field(FisnField, name, _FISN, nullable, metadata)


__all__ = [
    "BbgField",
    "BicField",
    "CfiField",
    "DtiField",
    "ElfField",
    "FisnField",
    "LeiField",
    "FigiField",
    "ForexField",
    "CountryField",
    "CcyField",
    "CusipField",
    "IsinField",
    "MicField",
    "RicField",
    "SedolField",
    "UnitField",
    "bbg",
    "bic",
    "cfi",
    "dti",
    "elf",
    "fisn",
    "lei",
    "country",
    "ccy",
    "figi",
    "forex",
    "cusip",
    "isin",
    "mic",
    "ric",
    "sedol",
    "unit",
]
